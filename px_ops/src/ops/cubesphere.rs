use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::noise::fnv1a;
use crate::{Grid, MeshOp};
use px_protocol::art::{
    CUBE_FACES, CUBE_GUTTER, MeshData, cube_atlas_uv, cube_cell_size, cube_direction,
    cube_face_size,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Params {
    pub subdivisions: u32,
    pub radius: f32,
    pub displace: f32,
    pub sea_level: f32,
    pub flat_sea: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            subdivisions: 128,
            radius: 1.0,
            displace: 0.075,
            sea_level: 0.52,
            flat_sea: false,
        }
    }
}

pub struct CubeSphere;

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

fn quantize(direction: [f32; 3]) -> [i64; 3] {
    [
        (direction[0] as f64 * 262_144.0).round() as i64,
        (direction[1] as f64 * 262_144.0).round() as i64,
        (direction[2] as f64 * 262_144.0).round() as i64,
    ]
}

fn vertex_of(positions: &[f32], index: u32) -> [f32; 3] {
    let slot = index as usize * 3;
    [positions[slot], positions[slot + 1], positions[slot + 2]]
}

impl MeshOp for CubeSphere {
    type Params = Params;
    const ID: &'static str = "mesh.cubesphere";
    const VERSION: u32 = 1;
    const SOURCE_HASH: u64 = fnv1a(include_str!("cubesphere.rs"));
    const INPUTS: &'static [&'static str] = &["height"];

    fn eval(params: &Params, inputs: &[&Field], grid: Grid) -> MeshData {
        let field = inputs[0];
        let stats = field.stats();
        let span = if (stats.max - stats.min).abs() <= f32::EPSILON {
            1.0
        } else {
            stats.max - stats.min
        };
        let face_size = cube_face_size(grid.width).max(2);
        let cell = cube_cell_size(grid.width);
        let n = params.subdivisions.clamp(2, 512);

        let mut positions: Vec<f32> = Vec::new();
        let mut uvs: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut welded: HashMap<(u32, [i64; 3]), u32> = HashMap::new();

        for face in 0..CUBE_FACES {
            let mut patch: Vec<u32> = Vec::with_capacity(((n + 1) * (n + 1)) as usize);
            for j in 0..=n {
                for i in 0..=n {
                    let s = i as f32 / n as f32;
                    let t = j as f32 / n as f32;
                    let direction = cube_direction(face, s, t);
                    let key = (face, quantize(direction));
                    let index = match welded.get(&key) {
                        Some(existing) => *existing,
                        None => {
                            let height =
                                ((field.sample_direction(direction) - stats.min) / span).clamp(0.0, 1.0);
                            let shaped = if params.flat_sea && height < params.sea_level {
                                params.sea_level
                            } else {
                                height
                            };
                            let lift =
                                params.radius * (1.0 + params.displace * (shaped - params.sea_level));
                            let created = (positions.len() / 3) as u32;
                            positions.extend_from_slice(&[
                                direction[0] * lift,
                                direction[1] * lift,
                                direction[2] * lift,
                            ]);
                            let uv = cube_atlas_uv(face, s, t, face_size, CUBE_GUTTER);
                            uvs.extend_from_slice(&uv);
                            welded.insert(key, created);
                            created
                        }
                    };
                    patch.push(index);
                }
            }

            let rows = n + 1;
            let at = |i: u32, j: u32| -> u32 { patch[(j * rows + i) as usize] };
            let corner = |i: u32, j: u32| vertex_of(&positions, at(i, j));
            let first = (corner(0, 0), corner(1, 0), corner(0, 1));
            let edge_one = [
                first.1[0] - first.0[0],
                first.1[1] - first.0[1],
                first.1[2] - first.0[2],
            ];
            let edge_two = [
                first.2[0] - first.0[0],
                first.2[1] - first.0[1],
                first.2[2] - first.0[2],
            ];
            let normal = [
                edge_one[1] * edge_two[2] - edge_one[2] * edge_two[1],
                edge_one[2] * edge_two[0] - edge_one[0] * edge_two[2],
                edge_one[0] * edge_two[1] - edge_one[1] * edge_two[0],
            ];
            let centre = cube_direction(face, 0.5, 0.5);
            let outward =
                normal[0] * centre[0] + normal[1] * centre[1] + normal[2] * centre[2] > 0.0;

            for j in 0..n {
                for i in 0..n {
                    let p00 = at(i, j);
                    let p10 = at(i + 1, j);
                    let p01 = at(i, j + 1);
                    let p11 = at(i + 1, j + 1);
                    if outward {
                        indices.extend_from_slice(&[p00, p10, p01, p01, p10, p11]);
                    } else {
                        indices.extend_from_slice(&[p00, p01, p10, p01, p11, p10]);
                    }
                }
            }
        }

        let vertices = positions.len() / 3;
        let mut accumulated = vec![0.0_f32; vertices * 3];
        for triangle in indices.chunks_exact(3) {
            let a = vertex_of(&positions, triangle[0]);
            let b = vertex_of(&positions, triangle[1]);
            let c = vertex_of(&positions, triangle[2]);
            let edge_one = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let edge_two = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                edge_one[1] * edge_two[2] - edge_one[2] * edge_two[1],
                edge_one[2] * edge_two[0] - edge_one[0] * edge_two[2],
                edge_one[0] * edge_two[1] - edge_one[1] * edge_two[0],
            ];
            for index in triangle {
                let slot = *index as usize * 3;
                accumulated[slot] += cross[0];
                accumulated[slot + 1] += cross[1];
                accumulated[slot + 2] += cross[2];
            }
        }

        let mut normals = vec![0.0_f32; vertices * 3];
        let mut zero = 0_usize;
        for vertex in 0..vertices {
            let slot = vertex * 3;
            let sum = [
                accumulated[slot],
                accumulated[slot + 1],
                accumulated[slot + 2],
            ];
            let length = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
            let unit = if length <= f32::EPSILON {
                zero += 1;
                normalize(vertex_of(&positions, vertex as u32))
            } else {
                normalize(sum)
            };
            normals[slot..slot + 3].copy_from_slice(&unit);
        }

        let mut by_position: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
        for vertex in 0..vertices {
            let direction = normalize(vertex_of(&positions, vertex as u32));
            by_position
                .entry(quantize(direction))
                .or_default()
                .push(vertex);
        }
        let mut welds = 0_usize;
        for group in by_position.values() {
            if group.len() < 2 {
                continue;
            }
            welds += 1;
            let mut sum = [0.0_f32; 3];
            for vertex in group {
                let slot = vertex * 3;
                sum[0] += normals[slot];
                sum[1] += normals[slot + 1];
                sum[2] += normals[slot + 2];
            }
            let unit = normalize(sum);
            for vertex in group {
                let slot = vertex * 3;
                normals[slot..slot + 3].copy_from_slice(&unit);
            }
        }

        let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
        for triangle in indices.chunks_exact(3) {
            for pair in 0..3 {
                let (a, b) = (triangle[pair], triangle[(pair + 1) % 3]);
                let key = if a < b { (a, b) } else { (b, a) };
                *edges.entry(key).or_insert(0) += 1;
            }
        }
        let open = edges.values().filter(|count| **count == 1).count();
        let mut inward = 0_usize;
        let mut worst = 1.0_f32;
        for vertex in 0..vertices {
            let slot = vertex * 3;
            let normal = [normals[slot], normals[slot + 1], normals[slot + 2]];
            let radial = normalize(vertex_of(&positions, vertex as u32));
            let dot = normal[0] * radial[0] + normal[1] * radial[1] + normal[2] * radial[2];
            if dot < 0.0 {
                inward += 1;
            }
            worst = worst.min(dot);
        }
        println!(
            "网格审计：{vertices} 顶点 / {} 三角形，面 {face_size}²、格子 {cell}、法线焊接 {welds} 组、开口边 {open}，法线朝内 {inward}、零长 {zero}、最小点积 {worst:.3}",
            indices.len() / 3,
        );

        MeshData {
            positions,
            normals,
            uvs,
            indices,
        }
    }
}


