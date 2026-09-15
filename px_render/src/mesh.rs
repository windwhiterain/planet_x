//! 网格产物 → 渲染就绪的网格。这是渲染器里**唯一**碰几何的地方。
//!
//! 它不认识行星、不认识云：给一份 `kind = Mesh` 的产物，还一个 `Handle<Mesh>`。
//! 顶点位置的位移、球壳镶嵌这些"形状从哪来"的事全在烘图侧（§65）。

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use px_protocol::art::{AssetKind, MeshData};
use px_protocol::stream::{self, Frame};

use crate::art_cache::ReadyMesh;

/// 闭合网格按**有向体积**判缠绕朝向：朝里就翻成朝外。返回翻之前的体积（`None` = 没动）。
///
/// 为什么渲染侧要管这件事：材质走 Bevy 的通用 `Material` 管线，而那条管线默认把背面剔掉
/// （`cull_mode = Some(Face::Back)`，`bevy_pbr/src/render/mesh.rs`—— 除非产物在材质里
/// 声明了别的剔除档）。等值面算子出来的代理缠绕朝里（实测有向体积 −0.47）⇒ 被剔掉的是**近**面，
/// 云会整片消失。顺着缠绕翻，法线属性得跟着翻，否则顶点法与三角形绕向说的不是一件事。
fn outward_winding(mesh: &mut Mesh) -> Option<f64> {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).cloned()
    else {
        return None;
    };
    let Some(Indices::U32(indices)) = mesh.indices().cloned() else {
        return None;
    };
    let at = |index: u32| Vec3::from(positions[index as usize]);
    let mut volume = 0.0_f64;
    for triangle in indices.chunks_exact(3) {
        let (a, b, c) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
        volume += f64::from(a.dot(b.cross(c)));
    }
    if volume >= 0.0 {
        return None;
    }

    let mut flipped = indices;
    for triangle in flipped.chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
    mesh.insert_indices(Indices::U32(flipped));
    if let Some(VertexAttributeValues::Float32x3(normals)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_NORMAL)
    {
        for normal in normals.iter_mut() {
            for value in normal.iter_mut() {
                *value = -*value;
            }
        }
    }
    Some(volume / 6.0)
}

fn weld_normals(mesh: &mut Mesh) {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).cloned()
    else {
        return;
    };
    let Some(VertexAttributeValues::Float32x3(normals)) =
        mesh.attribute(Mesh::ATTRIBUTE_NORMAL).cloned()
    else {
        return;
    };

    let mut groups: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    for (index, position) in positions.iter().enumerate() {
        let key = [
            (position[0] * 65_536.0).round() as i32,
            (position[1] * 65_536.0).round() as i32,
            (position[2] * 65_536.0).round() as i32,
        ];
        groups.entry(key).or_default().push(index);
    }

    let mut welded = normals.clone();
    for indices in groups.values() {
        if indices.len() < 2 {
            continue;
        }
        let mut sum = Vec3::ZERO;
        for &index in indices {
            sum += Vec3::from(normals[index]);
        }
        let average = sum.normalize_or_zero().to_array();
        for &index in indices {
            welded[index] = average;
        }
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, welded);
}

/// 网格产物 → 网格。返回的审计文本同上：它是**值**，要被缓存原样重放。
pub fn load_mesh(path: &str) -> Result<(Mesh, String), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let frames = stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    let kind = frames.iter().find_map(|frame| match frame {
        Frame::Art(bundle) => bundle.assets.first().map(|asset| asset.kind),
        _ => None,
    });
    if kind != Some(AssetKind::Mesh) {
        return Err(format!("{path} 不是 Mesh 产物：{kind:?}"));
    }
    let blobs: Vec<&px_protocol::wire::Blob> = frames
        .iter()
        .filter_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .collect();
    let data = MeshData::from_blobs(&blobs).map_err(|err| err.to_string())?;

    let positions: Vec<[f32; 3]> = data
        .positions
        .chunks_exact(3)
        .map(|chunk| [chunk[0], chunk[1], chunk[2]])
        .collect();
    let normals: Vec<[f32; 3]> = data
        .normals
        .chunks_exact(3)
        .map(|chunk| [chunk[0], chunk[1], chunk[2]])
        .collect();
    let uvs: Vec<[f32; 2]> = data
        .uvs
        .chunks_exact(2)
        .map(|chunk| [chunk[0], chunk[1]])
        .collect();

    let mut audit = String::new();
    {
        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for triangle in data.indices.chunks_exact(3) {
            for pair in 0..3 {
                let (a, b) = (triangle[pair], triangle[(pair + 1) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                *edges.entry(key).or_insert(0) += 1;
            }
        }
        let open = edges.values().filter(|count| **count == 1).count();
        let odd = edges.values().filter(|count| **count > 2).count();
        audit.push_str(&format!(
            "网格缝合审计：{} 条边，其中 {open} 条只属于一个三角形（开口），{odd} 条属于两个以上\n",
            edges.len()
        ));
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(data.indices));
    weld_normals(&mut mesh);

    match outward_winding(&mut mesh) {
        Some(volume) => audit.push_str(&format!(
            "缠绕审计：有向体积 {volume:.4}（朝里）⇒ 已按朝外翻面\n"
        )),
        None => audit.push_str("缠绕审计：有向体积为正（朝外），不动\n"),
    }

    if let (
        Some(VertexAttributeValues::Float32x3(normals)),
        Some(VertexAttributeValues::Float32x3(positions)),
    ) = (
        mesh.attribute(Mesh::ATTRIBUTE_NORMAL),
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
    ) {
        let mut worst = 1.0_f32;
        for (normal, position) in normals.iter().zip(positions.iter()) {
            let radial = Vec3::new(position[0], position[1], position[2]).normalize_or_zero();
            worst = worst.min(Vec3::from(*normal).dot(radial));
        }
        audit.push_str(&format!("载入网格法线审计：最小点积 {worst:.3}\n"));
    }
    Ok((mesh, audit))
}

/// 网格产物 → 渲染就绪的网格资产：载入、焊法线、审计。
pub fn build_artifact_mesh(path: &str, meshes: &mut Assets<Mesh>) -> Result<ReadyMesh, String> {
    let (mesh, audit) = load_mesh(path)?;
    Ok(ReadyMesh {
        handle: meshes.add(mesh),
        audit,
    })
}

/// 顶点数 / 三角形数（从资产里数，不走缓存也便宜）。
pub fn mesh_counts(
    meshes: &Assets<Mesh>,
    handle: &Handle<Mesh>,
) -> Result<(usize, usize), String> {
    let mesh = meshes
        .get(handle)
        .ok_or_else(|| "拿不到刚插进去的网格资产".to_string())?;
    Ok((
        mesh.count_vertices(),
        mesh.indices().map(|indices| indices.len() / 3).unwrap_or(0),
    ))
}

/// 内建图元。球 / 细分球是任何渲染器都有的东西，不是"行星知识"：
/// 场景产物可以只要一个球壳（消融档 `orbit-soft-shell` 就是 `icosphere` + 半径），
/// 不必每次烘一份一模一样的网格产物。
pub fn primitive(name: &str, params: &std::collections::BTreeMap<String, px_protocol::Value>) -> Result<Mesh, String> {
    use px_protocol::Value;
    let number = |key: &str| -> Result<f32, String> {
        match params.get(key) {
            Some(Value::Num(value)) => Ok(*value as f32),
            Some(other) => Err(format!("图元 '{name}' 的参数 '{key}' 要一个数，实际是 {other:?}")),
            None => Err(format!(
                "图元 '{name}' 缺参数 '{key}'；它有的参数：{}",
                if params.is_empty() {
                    "（空）".to_string()
                } else {
                    params.keys().cloned().collect::<Vec<_>>().join(" / ")
                }
            )),
        }
    };
    match name {
        "icosphere" => {
            let radius = number("radius")?;
            let subdivisions = number("subdivisions")?;
            if !(subdivisions.fract() == 0.0 && (1.0..=64.0).contains(&subdivisions)) {
                return Err(format!(
                    "图元 'icosphere' 的 'subdivisions' 是 {subdivisions}：要 1..=64 的整数"
                ));
            }
            Sphere::new(radius)
                .mesh()
                .ico(subdivisions as u32)
                .map_err(|err| format!("细分球造不出来：{err}"))
        }
        "uv_sphere" => {
            let radius = number("radius")?;
            let sectors = number("sectors")? as u32;
            let stacks = number("stacks")? as u32;
            Ok(Sphere::new(radius).mesh().uv(sectors, stacks))
        }
        other => Err(format!(
            "不认识的图元 '{other}'；这份渲染器认：icosphere / uv_sphere"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icosphere_is_the_same_shape_the_cloud_shell_used() {
        let params = std::collections::BTreeMap::from([
            ("radius".to_string(), px_protocol::Value::Num(1.06)),
            ("subdivisions".to_string(), px_protocol::Value::Num(64.0)),
        ]);
        let now = primitive("icosphere", &params).expect("造不出来");
        let before = Sphere::new(1.06).mesh().ico(64).expect("Bevy 也造不出来");
        assert_eq!(now.count_vertices(), before.count_vertices());
        assert_eq!(now.indices().map(|i| i.len()), before.indices().map(|i| i.len()));
    }

    #[test]
    fn an_unknown_primitive_or_a_bad_parameter_is_an_error() {
        let empty = std::collections::BTreeMap::new();
        assert!(primitive("torus", &empty).unwrap_err().contains("torus"));
        let params = std::collections::BTreeMap::from([
            ("radius".to_string(), px_protocol::Value::Num(1.0)),
            ("subdivisions".to_string(), px_protocol::Value::Num(2.5)),
        ]);
        assert!(primitive("icosphere", &params).is_err());
    }
}
