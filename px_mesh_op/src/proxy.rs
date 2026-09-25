use std::collections::HashMap;

use isosurface::MarchingCubes;
use isosurface::distance::Signed;
use isosurface::extractor::IndexedVertices;
use isosurface::math::vector::Vec3;
use isosurface::sampler::Sampler;
use isosurface::source::ScalarSource;

use px_mesh_schema::ops::Proxy;
use px_mesh_schema::params;
use px_protocol::art::MeshData;
use px_volume_schema::{PATCHES, VolumeSampler};

px_graph_schema::px_body! {
    Proxy,
    |p, i| {
        let grid = px_volume_schema::VolumeGrid::new(i.volume.value());
        crate::proxy::surface(p, &grid)?
    }
}

struct Face<'a> {
    field: &'a dyn VolumeSampler,
    face: u32,
    level: f32,
}

impl ScalarSource for Face<'_> {
    fn sample_scalar(&self, p: Vec3) -> Signed {
        Signed(self.field.sample(self.face, [p.x, p.y, p.z]) - self.level)
    }
}

fn vertex_of(positions: &[f32], index: u32) -> [f32; 3] {
    let slot = index as usize * 3;
    [positions[slot], positions[slot + 1], positions[slot + 2]]
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

fn radial_gradient(field: &dyn VolumeSampler, point: [f32; 3]) -> [f32; 3] {
    let (face, at) = field.parameters(point);
    let step = 1.0 / 256.0;
    let mut gradient = [0.0_f32; 3];
    for axis in 0..3 {
        let mut offset = at;
        offset[axis] += step;
        let ahead = field.sample(face, offset);
        offset[axis] -= 2.0 * step;
        let behind = field.sample(face, offset);
        gradient[axis] = (ahead - behind) / (2.0 * step);
    }
    gradient
}

fn distance(one: [f32; 3], two: [f32; 3]) -> f32 {
    let delta = [one[0] - two[0], one[1] - two[1], one[2] - two[2]];
    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
}

fn world_gradient(field: &dyn VolumeSampler, point: [f32; 3]) -> [f32; 3] {
    let (face, at) = field.parameters(point);
    let step = 1.0 / 256.0;
    let mut gradient = [0.0_f32; 3];
    for axis in 0..3 {
        let mut ahead = at;
        ahead[axis] += step;
        let mut behind = at;
        behind[axis] -= step;
        let far = distance(field.point(face, ahead), field.point(face, behind));
        if far > f32::EPSILON {
            gradient[axis] = (field.sample(face, ahead) - field.sample(face, behind)) / far;
        }
    }
    gradient
}

fn push_vertex(
    cells: &mut HashMap<(i64, i64, i64), Vec<u32>>,
    positions: &mut Vec<f32>,
    uvs: &mut Vec<f32>,
    point: [f32; 3],
    uv: [f32; 2],
    weld: f32,
) -> (u32, bool) {
    let key = (
        (point[0] / weld).floor() as i64,
        (point[1] / weld).floor() as i64,
        (point[2] / weld).floor() as i64,
    );
    let mut best: Option<(u32, f32)> = None;
    for dx in -1..=1 {
        for dy in -1..=1 {
            for dz in -1..=1 {
                let Some(list) = cells.get(&(key.0 + dx, key.1 + dy, key.2 + dz)) else {
                    continue;
                };
                for index in list {
                    let slot = *index as usize * 3;
                    let far = (positions[slot] - point[0])
                        .abs()
                        .max((positions[slot + 1] - point[1]).abs())
                        .max((positions[slot + 2] - point[2]).abs());
                    if far <= weld && best.is_none_or(|(_, previous)| far < previous) {
                        best = Some((*index, far));
                    }
                }
            }
        }
    }
    if let Some((index, _)) = best {
        return (index, true);
    }
    let index = (positions.len() / 3) as u32;
    positions.extend_from_slice(&point);
    uvs.extend_from_slice(&uv);
    cells.entry(key).or_default().push(index);
    (index, false)
}

pub fn surface(
    params: &params::proxy::Params,
    field: &dyn VolumeSampler,
) -> Result<MeshData, String> {
    let depth = params.depth.clamp(1, 8);
    let points = (1_usize << depth) + 1;
    let weld = if params.weld > 0.0 { params.weld } else { 1e-4 };

    let mut positions: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut cells: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    let mut welds = 0_usize;

    for face in 0..PATCHES {
        let source = Face {
            field,
            face,
            level: params.level,
        };
        let sampler = Sampler::new(&source);
        let mut mesher = MarchingCubes::<Signed>::new(points);
        let mut local: Vec<f32> = Vec::new();
        let mut local_indices: Vec<u32> = Vec::new();
        {
            let mut extractor = IndexedVertices::new(&mut local, &mut local_indices);
            mesher.extract(&sampler, &mut extractor);
        }

        let mut remap: Vec<u32> = Vec::with_capacity(local.len() / 3);
        for index in 0..local.len() / 3 {
            let u = local[index * 3];
            let v = local[index * 3 + 1];
            let at = [u, v, local[index * 3 + 2]];
            let point = field.point(face, at);
            let uv = [u, (face as f32 + v) / PATCHES as f32];
            let (welded, merged) =
                push_vertex(&mut cells, &mut positions, &mut uvs, point, uv, weld);
            welds += usize::from(merged);
            remap.push(welded);
        }

        for triangle in local_indices.chunks(3) {
            let (a, b, c) = (
                remap[triangle[0] as usize],
                remap[triangle[1] as usize],
                remap[triangle[2] as usize],
            );
            if a == b || b == c || a == c {
                continue;
            }
            indices.extend_from_slice(&[a, b, c]);
        }
    }

    let vertices = positions.len() / 3;
    if vertices == 0 {
        return Err("体积里没有这个等值面 ⇒ 代理是空的（检查 tau 与 level）".to_string());
    }

    let mut accumulated = vec![0.0_f32; vertices * 3];
    let mut bend = vec![0.0_f32; vertices];
    let mut volume = 0.0_f32;
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
        volume += a[0] * (b[1] * c[2] - b[2] * c[1])
            + a[1] * (b[2] * c[0] - b[0] * c[2])
            + a[2] * (b[0] * c[1] - b[1] * c[0]);
        for index in triangle {
            let slot = *index as usize * 3;
            accumulated[slot] += cross[0];
            accumulated[slot + 1] += cross[1];
            accumulated[slot + 2] += cross[2];
            bend[*index as usize] +=
                (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt();
        }
    }
    volume /= 6.0;
    let mut normals = vec![0.0_f32; vertices * 3];
    let mut zero = 0_usize;
    for vertex in 0..vertices {
        let slot = vertex * 3;
        let sum = [
            accumulated[slot],
            accumulated[slot + 1],
            accumulated[slot + 2],
        ];
        let unit = if sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2] > f32::EPSILON {
            normalize(sum)
        } else {
            zero += 1;
            let point = vertex_of(&positions, vertex as u32);
            let gradient = radial_gradient(field, point);
            if gradient[0] * gradient[0] + gradient[1] * gradient[1] + gradient[2] * gradient[2]
                > f32::EPSILON
            {
                normalize([-gradient[0], -gradient[1], -gradient[2]])
            } else {
                normalize(point)
            }
        };
        normals[slot..slot + 3].copy_from_slice(&unit);
    }

    let mut fallback = 0_usize;
    if params.offset != 0.0 {
        let winding = if volume < 0.0 { -1.0 } else { 1.0 };
        let mut moved = [0.0_f32; 3];
        for vertex in 0..vertices {
            let slot = vertex * 3;
            let point = vertex_of(&positions, vertex as u32);
            let sum = [
                accumulated[slot],
                accumulated[slot + 1],
                accumulated[slot + 2],
            ];
            let size = sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2];
            let direction = if size > 0.0 && size * 256.0 >= bend[vertex] * bend[vertex] {
                let unit = normalize(sum);
                [unit[0] * winding, unit[1] * winding, unit[2] * winding]
            } else {
                fallback += 1;
                let gradient = world_gradient(field, point);
                if gradient[0] * gradient[0] + gradient[1] * gradient[1] + gradient[2] * gradient[2]
                    > f32::EPSILON
                {
                    normalize([-gradient[0], -gradient[1], -gradient[2]])
                } else {
                    [normals[slot], normals[slot + 1], normals[slot + 2]]
                }
            };
            for axis in 0..3 {
                moved[axis] = point[axis] + direction[axis] * params.offset;
            }
            positions[slot..slot + 3].copy_from_slice(&moved);
        }
        println!(
            "代理外扩：沿外法线推 {:+.6} 世界单位（{vertices} 个顶点；有向体积 {:+.4} ⇒ 缠绕{}；顶点星不成形改走场梯度的 {} 个）",
            params.offset,
            volume,
            if volume < 0.0 { "朝里" } else { "朝外" },
            fallback,
        );
    }

    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
    for triangle in indices.chunks_exact(3) {
        for pair in 0..3 {
            let (one, two) = (triangle[pair], triangle[(pair + 1) % 3]);
            let key = if one < two { (one, two) } else { (two, one) };
            *edges.entry(key).or_insert(0) += 1;
            *directed.entry((one, two)).or_insert(0) += 1;
        }
    }
    let open = edges.values().filter(|count| **count == 1).count();
    let nonmanifold = edges.values().filter(|count| **count > 2).count();
    let flipped = directed.values().filter(|count| **count > 1).count();
    println!(
        "代理审计：{PATCHES} 面 × {points}³ 点（depth {depth}）→ {vertices} 顶点 / {} 三角形｜开口边 {open}、非流形边 {nonmanifold}、翻面边 {flipped}、焊接 {welds} 个重复顶点、梯度兜底法线 {zero}",
        indices.len() / 3,
    );
    if open > 0 {
        println!("⚠ 代理有 {open} 条开口边 ⇒ 相邻两面的接缝没焊严，硬表面会从缝里漏出去");
    }

    Ok(MeshData {
        positions,
        normals,
        uvs,
        indices,
    })
}
