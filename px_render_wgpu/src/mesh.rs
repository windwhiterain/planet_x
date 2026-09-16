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

    /// 顶点属性表：`(shader_location, 格式, 从哪个属性来)`。
    ///
    /// ⚠ **真本是产物自己带的那四个属性**（[`px_protocol::art::MESH_ATTRIBUTES`]）：
    /// 位置 / 法线 / uv 三样在网格里就是三块 `f32` 数组，所以**位置天然就是**
    /// location 0 的 `Float32x3`、法线 1、uv 2 —— 这个次序不是我挑的，是产物的形状定的
    /// （与 Bevy 那份 `Mesh` 的 `POSITION` / `NORMAL` / `UV_0` 同一个次序，oracle 也是它）。
    /// 在宿主里另写一张"我猜的布局"就是 §66.1：漂开的那天顶点属性会静默错位。
    ///
    /// ⚠ 交错只发生在**上传**这一步：产物是**非交错**的三块数组（每块自己一段），
    /// 而 `px_pass` 的一笔 draw 只接受一个顶点缓冲 ⇒ 上传前必须拼成一条 `stride = 32` 的流。
    /// 拼的次序就是这张表（0/1/2 依次排下去）。
    pub fn attributes() -> Vec<(u32, wgpu::VertexFormat, &'static str)> {
        vec![
            (0, wgpu::VertexFormat::Float32x3, "positions"),
            (1, wgpu::VertexFormat::Float32x3, "normals"),
            (2, wgpu::VertexFormat::Float32x2, "uvs"),
        ]
    }

    /// 交错后的顶点流（`positions → normals → uvs`，每条 `stride` 字节）。
    ///
    /// ⚠ 三块属性长度不一致就**当场拒**，不许按下标硬取：那会在上传时 panic 在
    /// 一个看不出所以然的位置，而真正的原因是产物本身不完整。
    pub fn interleaved(&self) -> Result<Vec<u8>, String> {
        let count = self.positions.len();
        if self.normals.len() != count || self.uvs.len() != count {
            return Err(format!(
                "网格的三块属性对不上：位置 {count} 条 / 法线 {} 条 / uv {} 条 ⇒ 拼不出交错顶点流",
                self.normals.len(),
                self.uvs.len()
            ));
        }
        let stride = Self::stride() as usize;
        let mut bytes = Vec::with_capacity(count * stride);
        for index in 0..count {
            for value in self.positions[index] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in self.normals[index] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            for value in self.uvs[index] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        Ok(bytes)
    }

    /// 一条顶点的字节数 = 三个属性的宽度之和（12 + 12 + 8 = 32）。
    pub fn stride() -> u32 {
        Self::attributes()
            .iter()
            .map(|(_, format, _)| format.size() as u32)
            .sum()
    }

    /// `stride` / 每一格的偏移与格式。偏移是**表里累加出来的**（不是手抄的 0/12/24）。
    pub fn vertex_attributes() -> Vec<wgpu::VertexAttribute> {
        let mut offset = 0_u64;
        let mut attributes = Vec::new();
        for (shader_location, format, _) in Self::attributes() {
            attributes.push(wgpu::VertexAttribute {
                format,
                offset,
                shader_location,
            });
            offset += format.size();
        }
        attributes
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
