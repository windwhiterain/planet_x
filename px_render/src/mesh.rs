use std::collections::HashMap;

use px_protocol::art::MeshData;
use px_protocol::stream::{self, Frame};

use crate::vec::Vec3;

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

    pub fn attributes() -> Vec<(u32, wgpu::VertexFormat, &'static str)> {
        vec![
            (0, wgpu::VertexFormat::Float32x3, "positions"),
            (1, wgpu::VertexFormat::Float32x3, "normals"),
            (2, wgpu::VertexFormat::Float32x2, "uvs"),
        ]
    }

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

    pub fn stride() -> u32 {
        Self::attributes()
            .iter()
            .map(|(_, format, _)| format.size() as u32)
            .sum()
    }

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

pub fn load_mesh(path: &str) -> Result<(Mesh, String), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let frames = stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
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

    #[test]
    fn an_inward_winding_is_flipped_with_its_normals() {
        let mut mesh = Mesh {
            positions: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0]; 4],
            indices: vec![1, 2, 0, 3, 1, 0, 2, 3, 0, 3, 2, 1],
        };
        let before = mesh.indices.clone();
        let volume = outward_winding(&mut mesh).expect("朝里就该翻");
        assert!(volume < 0.0, "返回的是翻之前的体积：{volume}");
        assert_ne!(mesh.indices, before, "索引换了");
        assert_eq!(mesh.normals[0], [0.0, 0.0, -1.0], "法线也跟着翻了");
        assert!(outward_winding(&mut mesh).is_none());
    }

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
