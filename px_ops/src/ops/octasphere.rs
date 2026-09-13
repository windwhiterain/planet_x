use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::noise::fnv1a;
use crate::{Grid, MeshOp};
use px_protocol::art::{MeshData, octahedral_uv_y_up};

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

impl MeshOp for Octasphere {
    type Params = Params;
    const ID: &'static str = "mesh.octasphere";
    const VERSION: u32 = 1;
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
        let mut normals: Vec<f32> = Vec::new();
        let mut uvs: Vec<f32> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();

        for first in [0_usize, 1] {
            for second in [2_usize, 3] {
                for third in [4_usize, 5] {
                    let base = (positions.len() / 3) as u32;
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
                    if normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2] < 0.0 {
                        std::mem::swap(&mut b, &mut c);
                    }
                    let rows = n + 1;

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
                            let height = ((field.sample_direction(direction) - stats.min) / span).clamp(0.0, 1.0);
                            let shaped = if params.flat_sea && height < params.sea_level {
                                params.sea_level
                            } else {
                                height
                            };
                            let lift = params.radius * (1.0 + params.displace * (shaped - params.sea_level));
                            positions.extend_from_slice(&[
                                direction[0] * lift,
                                direction[1] * lift,
                                direction[2] * lift,
                            ]);
                            let uv = octahedral_uv_y_up(direction);
                            uvs.extend_from_slice(&uv);
                            normals.extend_from_slice(&direction);
                        }
                    }

                    let offset = |i: u32, j: u32| -> u32 {
                        let row_start: u32 = (0..i).map(|row| rows - row).sum();
                        base + row_start + j
                    };

                    for i in 0..n {
                        for j in 0..(n - i) {
                            let p0 = offset(i, j);
                            let p1 = offset(i + 1, j);
                            let p2 = offset(i, j + 1);
                            indices.extend_from_slice(&[p0, p1, p2]);
                            if j + 1 < n - i {
                                let p3 = offset(i + 1, j + 1);
                                indices.extend_from_slice(&[p2, p1, p3]);
                            }
                        }
                    }
                }
            }
        }

        let vertex_at = |index: usize| -> [f32; 3] {
            [
                positions[index * 3],
                positions[index * 3 + 1],
                positions[index * 3 + 2],
            ]
        };

        let mut rebuilt: Vec<f32> = Vec::with_capacity(normals.len());
        let rows = (n + 1) as usize;
        let patch_vertices = ((n + 1) * (n + 2) / 2) as usize;
        let ring: [(i64, i64); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
        let mut patches: Vec<usize> = Vec::new();
        let mut cursor = 0_usize;
        while cursor < positions.len() / 3 {
            patches.push(cursor);
            cursor += patch_vertices;
        }
        for start in patches {
            let neighbour = |i: i64, j: i64| -> Option<usize> {
                if i < 0 || j < 0 || i + j > n as i64 {
                    return None;
                }
                let row_start: usize = (0..i as usize).map(|row| rows - row).sum();
                Some(start + row_start + j as usize)
            };
            for i in 0..=n as i64 {
                for j in 0..=(n as i64 - i) {
                    let here = vertex_at(neighbour(i, j).expect("自身一定在网格内"));
                    let mut sum = [0.0_f32; 3];
                    for step in 0..6 {
                        let (ai, aj) = ring[step];
                        let (bi, bj) = ring[(step + 1) % 6];
                        let (Some(first), Some(second)) =
                            (neighbour(i + ai, j + aj), neighbour(i + bi, j + bj))
                        else {
                            continue;
                        };
                        let one = vertex_at(first);
                        let two = vertex_at(second);
                        let edge_one = [one[0] - here[0], one[1] - here[1], one[2] - here[2]];
                        let edge_two = [two[0] - here[0], two[1] - here[1], two[2] - here[2]];
                        sum[0] += edge_one[1] * edge_two[2] - edge_one[2] * edge_two[1];
                        sum[1] += edge_one[2] * edge_two[0] - edge_one[0] * edge_two[2];
                        sum[2] += edge_one[0] * edge_two[1] - edge_one[1] * edge_two[0];
                    }
                    let radial = normalize(here);
                    let mut normal = normalize(sum);
                    if (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt() <= f32::EPSILON {
                        normal = radial;
                    }
                    if normal[0] * radial[0] + normal[1] * radial[1] + normal[2] * radial[2] < 0.0 {
                        normal = [-normal[0], -normal[1], -normal[2]];
                    }
                    rebuilt.extend_from_slice(&normal);
                }
            }
        }
        if rebuilt.len() == normals.len() {
            normals = rebuilt;
        }
        let mut inward = 0_usize;
        let mut zero = 0_usize;
        let mut worst = 1.0_f32;
        for vertex in 0..(positions.len() / 3) {
            let n = [
                normals[vertex * 3],
                normals[vertex * 3 + 1],
                normals[vertex * 3 + 2],
            ];
            let r = normalize([
                positions[vertex * 3],
                positions[vertex * 3 + 1],
                positions[vertex * 3 + 2],
            ]);
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if length < 0.5 {
                zero += 1;
            }
            let dot = n[0] * r[0] + n[1] * r[1] + n[2] * r[2];
            if dot < 0.0 {
                inward += 1;
            }
            worst = worst.min(dot);
        }
        println!(
            "网格审计：{} 顶点，{inward} 个法线朝内，{zero} 个零长，最小点积 {worst:.3}",
            positions.len() / 3
        );

        MeshData {
            positions,
            normals,
            uvs,
            indices,
        }
    }
}



