use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::noise::fnv1a;
use crate::{Grid, MeshOp};
use px_protocol::art::MeshData;

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
            subdivisions: 160,
            radius: 1.0,
            displace: 0.075,
            sea_level: 0.52,
            flat_sea: false,
        }
    }
}

pub struct Octasphere;

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

fn quantize(direction: [f32; 3]) -> [i64; 3] {
    [
        (direction[0] as f64 * 1_048_576.0).round() as i64,
        (direction[1] as f64 * 1_048_576.0).round() as i64,
        (direction[2] as f64 * 1_048_576.0).round() as i64,
    ]
}

fn vertex_of(positions: &[f32], index: u32) -> [f32; 3] {
    let slot = index as usize * 3;
    [positions[slot], positions[slot + 1], positions[slot + 2]]
}

impl MeshOp for Octasphere {
    type Params = Params;
    const ID: &'static str = "mesh.octasphere";
    const VERSION: u32 = 2;
    const SOURCE_HASH: u64 = fnv1a(include_str!("octasphere.rs"));
    const INPUTS: &'static [&'static str] = &["height"];

    fn eval(params: &Params, inputs: &[&Field], _grid: Grid) -> MeshData {
        let field = inputs[0];
        let stats = field.stats();
        let span = if (stats.max - stats.min).abs() <= f32::EPSILON {
            1.0
        } else {
            stats.max - stats.min
        };
        let n = params.subdivisions.clamp(2, 512);
        let corners = [
            [1.0_f32, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];

        let mut positions: Vec<f32> = Vec::new();
        let mut uvs: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut welded: HashMap<[i64; 3], u32> = HashMap::new();

        for first in [0_usize, 1] {
            for second in [2_usize, 3] {
                for third in [4_usize, 5] {
                    let mut a = corners[first];
                    let mut b = corners[second];
                    let mut c = corners[third];
                    let outward = [a[0] + b[0] + c[0], a[1] + b[1] + c[1], a[2] + b[2] + c[2]];
                    let edge_one = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                    let edge_two = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                    let normal = [
                        edge_one[1] * edge_two[2] - edge_one[2] * edge_two[1],
                        edge_one[2] * edge_two[0] - edge_one[0] * edge_two[2],
                        edge_one[0] * edge_two[1] - edge_one[1] * edge_two[0],
                    ];
                    if normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2] < 0.0
                    {
                        std::mem::swap(&mut b, &mut c);
                    }
                    let rows = n + 1;

                    let mut patch: Vec<u32> = Vec::with_capacity(((n + 1) * (n + 2) / 2) as usize);
                    for i in 0..=n {
                        for j in 0..=(n - i) {
                            let wa = (n - i - j) as f32 / n as f32;
                            let wb = i as f32 / n as f32;
                            let wc = j as f32 / n as f32;
                            let direction = normalize([
                                a[0] * wa + b[0] * wb + c[0] * wc,
                                a[1] * wa + b[1] * wb + c[1] * wc,
                                a[2] * wa + b[2] * wb + c[2] * wc,
                            ]);
                            let key = quantize(direction);
                            let index = match welded.get(&key) {
                                Some(existing) => *existing,
                                None => {
                                    let height = ((field.sample_direction(direction) - stats.min)
                                        / span)
                                        .clamp(0.0, 1.0);
                                    let shaped = if params.flat_sea && height < params.sea_level {
                                        params.sea_level
                                    } else {
                                        height
                                    };
                                    let lift = params.radius
                                        * (1.0 + params.displace * (shaped - params.sea_level));
                                    let created = (positions.len() / 3) as u32;
                                    positions.extend_from_slice(&[
                                        direction[0] * lift,
                                        direction[1] * lift,
                                        direction[2] * lift,
                                    ]);
                                    let uv = px_protocol::art::octahedral_uv_y_up(direction);
                                    uvs.extend_from_slice(&uv);
                                    welded.insert(key, created);
                                    created
                                }
                            };
                            patch.push(index);
                        }
                    }

                    let at = |i: u32, j: u32| -> u32 {
                        let row_start: u32 = (0..i).map(|row| rows - row).sum();
                        patch[(row_start + j) as usize]
                    };
                    for i in 0..n {
                        for j in 0..(n - i) {
                            let p0 = at(i, j);
                            let p1 = at(i + 1, j);
                            let p2 = at(i, j + 1);
                            indices.extend_from_slice(&[p0, p1, p2]);
                            if j + 1 < n - i {
                                let p3 = at(i + 1, j + 1);
                                indices.extend_from_slice(&[p2, p1, p3]);
                            }
                        }
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
            "网格审计：{vertices} 顶点 / {} 三角形，开口边 {open}，法线朝内 {inward}，零长 {zero}，最小点积 {worst:.3}",
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
#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::art::{octahedral_direction_y_up, octahedral_uv_y_up};

    #[test]
    fn every_vertex_uv_points_back_at_its_direction() {
        let samples = [(1.0_f32, 1.0, 1.0), (4.0, 1.0, 1.0), (1.0, 4.0, 1.0), (1.0, 1.0, 4.0), (1.0, 1.0, 0.0)];
        let corners = [
            [1.0_f32, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        let mut worst = 0.0_f32;
        for first in [0_usize, 1] {
            for second in [2_usize, 3] {
                for third in [4_usize, 5] {
                    let (a, b, c) = (corners[first], corners[second], corners[third]);
                    for (sa, sb, sc) in samples {
                        let total = sa + sb + sc;
                        if total <= 0.0 {
                            continue;
                        }
                        let (wa, wb, wc) = (sa / total, sb / total, sc / total);
                        let direction = normalize([
                            a[0] * wa + b[0] * wb + c[0] * wc,
                            a[1] * wa + b[1] * wb + c[1] * wc,
                            a[2] * wa + b[2] * wb + c[2] * wc,
                        ]);
                        let uv = octahedral_uv_y_up(direction);
                        let back = octahedral_direction_y_up(uv[0], uv[1]);
                        let error = ((direction[0] - back[0]).powi(2)
                            + (direction[1] - back[1]).powi(2)
                            + (direction[2] - back[2]).powi(2))
                        .sqrt();
                        worst = worst.max(error);
                    }
                }
            }
        }
        assert!(worst < 0.01, "顶点 UV 与方向不符，最大误差 {worst}");
    }
}
