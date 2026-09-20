//! 几何：环网格（`ring_mesh`）。
//!
//! 边界：这一档只出顶点/法线/UV/索引，**不碰字节**——顶点数据的展平在 `super::texture`
//! 的 `flatten2`（与二维 UV 共用），落盘在 `super::store`。
//! 现在的 `flatten3` 只有这里用，所以留在本模块私有。

use px_protocol::art::MeshData;

use super::texture::flatten2;

// ---------------------------------------------------------------------------
// 环：网格
// ---------------------------------------------------------------------------

/// 环网格（从 `ring_mesh` 逐字搬）。
pub fn ring_mesh(inner: f32, outer: f32, segments: u32) -> MeshData {
    let mut positions = Vec::with_capacity((segments as usize + 1) * 2);
    let mut normals = Vec::with_capacity((segments as usize + 1) * 2);
    let mut uvs = Vec::with_capacity((segments as usize + 1) * 2);
    let mut indices = Vec::with_capacity(segments as usize * 6);

    for index in 0..=segments {
        let angle = index as f32 / segments as f32 * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        let v = index as f32 / segments as f32;
        positions.push([cos * inner, 0.0, sin * inner]);
        positions.push([cos * outer, 0.0, sin * outer]);
        normals.push([0.0, 1.0, 0.0]);
        normals.push([0.0, 1.0, 0.0]);
        uvs.push([0.0, v]);
        uvs.push([1.0, v]);
    }
    for index in 0..segments {
        let base = index * 2;
        indices.extend_from_slice(&[base, base + 2, base + 1, base + 1, base + 2, base + 3]);
    }

    MeshData {
        positions: flatten3(&positions),
        normals: flatten3(&normals),
        uvs: flatten2(&uvs),
        indices,
    }
}

fn flatten3(values: &[[f32; 3]]) -> Vec<f32> {
    let mut out = Vec::with_capacity(values.len() * 3);
    for value in values {
        out.extend_from_slice(value);
    }
    out
}
