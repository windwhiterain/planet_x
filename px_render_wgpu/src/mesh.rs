//! 网格产物 → 可以上传的顶点 / 索引。这是渲染器里**唯一**碰几何的地方。
//!
//! 从 `px_render::mesh` 搬来（§102：耦合度 6，全在 Bevy 的 `Mesh` 上）。它不认识行星、
//! 不认识云：给一份 `kind = Mesh` 的产物，还一份顶点/索引。位移、球壳镶嵌这些
//! "形状从哪来"的事全在烘图侧（§65）。
//!
//! ⚠ 两次"搬运"的算式一个括号都没动 —— 它们**改的是像素**：
//!
//! - [`weld_normals`]：按位置量化（×65536 取整）分组、组内求和的**顺序**决定法线；
//! - [`outward_winding`]：等值面算子出来的代理缠绕朝里（实测有向体积 −0.47），
//!   不翻的话被剔除的是**近**面，云会整片消失（§102 记的那条）。
//!
//! 审计文本也逐字照抄：它进的是报告与服务日志，是 §51 那套历史读数的一部分。

use std::collections::HashMap;

use px_protocol::art::{AssetKind, MeshData};
use px_protocol::stream::{self, Frame};

use crate::vec::Vec3;

/// 平面表示的网格。没有 Bevy 的 `Mesh`、没有顶点布局的泛型 —— 布局是**运行期的一张表**
/// （`wgpu::VertexBufferLayout`），不必为每种顶点生成一份代码。
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// 闭合网格按**有向体积**判缠绕朝向：朝里就翻成朝外。返回翻之前的体积（`None` = 没动）。
///
/// 为什么渲染侧要管这件事：材质那条管线默认把背面剔掉（`cull = Back`），
/// 而等值面算子出来的代理缠绕朝里 ⇒ 被剔掉的是**近**面。顺着缠绕翻，法线属性得跟着翻，
/// 否则顶点法与三角形绕向说的不是一件事。
pub fn outward_winding(mesh: &mut Mesh) -> Option<f64> {
    if mesh.positions.is_empty() || mesh.indices.is_empty() {
        return None;
    }
    let at = |index: u32| Vec3::from_array(mesh.positions[index as usize]);
    let mut volume = 0.0_f64;
    for triangle in mesh.indices.chunks_exact(3) {
        let (a, b, c) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
        volume += f64::from(a.dot(b.cross(c)));
    }
    if volume >= 0.0 {
        return None;
    }

    for triangle in mesh.indices.chunks_exact_mut(3) {
        triangle.swap(1, 2);
    }
    for normal in mesh.normals.iter_mut() {
        for value in normal.iter_mut() {
            *value = -*value;
        }
    }
    Some(volume / 6.0)
}

/// 同一个位置上的顶点把法线焊成一条（平均后归一）。
///
/// ⚠ 分组键是**量化过的位置**（×65536 四舍五入成 i32），顺序是位置下标的升序 ——
/// 求和次序换了，法线最后一位就换，逐字节判据当场红。
pub fn weld_normals(mesh: &mut Mesh) {
    if mesh.positions.is_empty() || mesh.normals.len() != mesh.positions.len() {
        return;
    }

    let mut groups: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    for (index, position) in mesh.positions.iter().enumerate() {
        let key = [
            (position[0] * 65_536.0).round() as i32,
            (position[1] * 65_536.0).round() as i32,
            (position[2] * 65_536.0).round() as i32,
        ];
        groups.entry(key).or_default().push(index);
    }

    let mut welded = mesh.normals.clone();
    for indices in groups.values() {
        if indices.len() < 2 {
            continue;
        }
        let mut sum = Vec3::ZERO;
        for &index in indices {
            sum += Vec3::from_array(mesh.normals[index]);
        }
        let average = sum.normalize_or_zero().to_array();
        for &index in indices {
            welded[index] = average;
        }
    }
    mesh.normals = welded;
}

/// 网格产物 → 网格。返回的审计文本是**值**，要被缓存原样重放。
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

    let mut mesh = Mesh {
        positions: data
            .positions
            .chunks_exact(3)
            .map(|chunk| [chunk[0], chunk[1], chunk[2]])
            .collect(),
        normals: data
            .normals
            .chunks_exact(3)
            .map(|chunk| [chunk[0], chunk[1], chunk[2]])
            .collect(),
        uvs: data
            .uvs
            .chunks_exact(2)
            .map(|chunk| [chunk[0], chunk[1]])
            .collect(),
        indices: data.indices.clone(),
    };

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

    weld_normals(&mut mesh);

    match outward_winding(&mut mesh) {
        Some(volume) => audit.push_str(&format!(
            "缠绕审计：有向体积 {volume:.4}（朝里）⇒ 已按朝外翻面\n"
        )),
        None => audit.push_str("缠绕审计：有向体积为正（朝外），不动\n"),
    }

    if mesh.normals.len() == mesh.positions.len() {
        let mut worst = 1.0_f32;
        for (normal, position) in mesh.normals.iter().zip(mesh.positions.iter()) {
            let radial = Vec3::new(position[0], position[1], position[2]).normalize_or_zero();
            worst = worst.min(Vec3::from_array(*normal).dot(radial));
        }
        audit.push_str(&format!("载入网格法线审计：最小点积 {worst:.3}\n"));
    }
    Ok((mesh, audit))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 朝里的缠绕必须被翻过来，而且**法线跟着翻**（顶点法与绕向说的得是一件事）。
    #[test]
    fn an_inward_winding_is_flipped_with_its_normals() {
        let mut mesh = Mesh {
            // 一个朝里的四面体（每个面的绕向都反着）。
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0]; 4],
            // 一个有向体积为负的四面体（绕向整体反着来 ⇒ 法线朝里）。
            indices: vec![1, 2, 0, 3, 1, 0, 2, 3, 0, 3, 2, 1],
        };
        let before = mesh.indices.clone();
        let volume = outward_winding(&mut mesh).expect("朝里就该翻");
        assert!(volume < 0.0, "返回的是翻之前的体积：{volume}");
        assert_ne!(mesh.indices, before, "索引换了");
        assert_eq!(mesh.normals[0], [0.0, 0.0, -1.0], "法线也跟着翻了");
        // 翻完再判一次：这次是朝外，不许再动。
        assert!(outward_winding(&mut mesh).is_none());
    }

    /// 同一位置的顶点法线焊成一条：两个相反的法线应当互相抵消 ⇒ 归一化兜底成零。
    #[test]
    fn welding_averages_the_normals_of_one_position() {
        let mut mesh = Mesh {
            positions: vec![[0.5, 0.0, 0.0], [0.5, 0.0, 0.0]],
            normals: vec![[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]],
            uvs: vec![[0.0, 0.0]; 2],
            indices: vec![0, 1, 0],
        };
        weld_normals(&mut mesh);
        assert_eq!(mesh.normals[0], [0.0, 0.0, 0.0]);
        assert_eq!(mesh.normals[1], [0.0, 0.0, 0.0]);
    }
}
