//! 立方球代理 mesh：拿一张 3D 标量网格出等值面。
//!
//! 这是**叶子** crate：只依赖 `px_ops`（协议）与 `isosurface`（算法库），
//! 不认识云、不认识驱动、不认识渲染器 ⇒ 换算法库不动 `px_graphs`。
//!
//! 六个面各自在自己的 `[0,1]³`（`u, v, 径向高度`）里跑一次等值面，然后把落在一起的
//! 顶点焊起来。焊得动靠的是参数空间的性质：相邻两面在共用的那条棱上给出**逐位相同**的
//! 方向（`cube_direction` 的公式在棱上重合）⇒ 两个面在棱上的采样点、场值、交点位置
//! 都一样，只需按位置合并。剩下的边界就只有壳的上下两面 —— 而那里场是常数（负），
//! 等值面碰不到 ⇒ 焊完没有开口边。
//!
//! 用的是同一个 crate 里的稠密 `MarchingCubes` 而不是 `LinearHashedMarchingCubes`：
//! 后者是自适应哈希八叉树，实测（`target/isosurface-probe`）两件事过不去 ——
//! ① 它的输出顶点位置依赖 `HashMap` 的遍历序（同一进程里跑两次，depth 7 有 1251/14721
//! 个顶点位置不同），而缓存要求「键 = 内容」；② 它在域壁上把交点放在离壁半格的位置，
//! 于是相邻两面的切割折线错开一格，焊不严。稠密 MC 逐位可复现，且域壁上的切割折线正是
//! 两边共享的那条二维等值线，能逐点焊上。

use std::collections::HashMap;

use isosurface::distance::Signed;
use isosurface::extractor::IndexedVertices;
use isosurface::math::vector::Vec3;
use isosurface::sampler::Sampler;
use isosurface::source::ScalarSource;
use isosurface::MarchingCubes;
use serde::{Deserialize, Serialize};

use px_ops::noise::fnv1a;
use px_ops::{IsosurfaceOp, MeshData, PATCHES, VolumeSampler};

pub struct ProxySurface;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// 等值面高度。体积里存的是归一化后的场 `(粗场 - τ) / L`，所以默认 0.0。
    pub level: f32,
    /// 每个面的分辨率：每轴 `2^depth + 1` 个采样点。代理要粗，6~7 就够。
    pub depth: u32,
    /// 焊缝焊接容差（世界单位）。远小于一个格子，只吃掉两块之间 1 ULP 级的偏差。
    pub weld: f32,
    /// 几何外扩：每个顶点沿外法线往外推这么远（世界单位）。
    ///
    /// 为什么只能靠几何补：稠密 MC 的等值面比解析等值面**浅**（实测外边界余量 −0.0017），
    /// 云的轮廓上因此丢一圈；而体积里存的是 `(场−τ)/L`，`level` 往外偏的下限就是壁上那层
    /// `−τ/L`（再低提取器直接报"体积里没有这个等值面"）⇒ 偏 `level` 补不回来。
    ///
    /// 0 不进键（`skip_serializing_if`）：不写这个字段的老档输出逐位不变，键也不该变。
    #[serde(skip_serializing_if = "is_zero")]
    pub offset: f32,
}

/// `skip_serializing_if` 要的那条：0 等价于"没写这个字段"。
fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

impl Default for Params {
    fn default() -> Self {
        Self {
            level: 0.0,
            depth: 6,
            weld: 1e-4,
            offset: 0.0,
        }
    }
}

/// 把「本面参数」接到「参数空间」上：面号单独传，面内坐标就是 marching cubes 的域。
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

/// 世界点处的场梯度（中心差商）：先反查它落在哪个面的哪个参数上，再回采样器。
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

/// 两点的距离。
fn distance(one: [f32; 3], two: [f32; 3]) -> f32 {
    let delta = [one[0] - two[0], one[1] - two[1], one[2] - two[2]];
    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
}

/// 世界空间的场梯度（中心差商）。
///
/// ⚠ 不能像 `radial_gradient` 那样直接拿参数差商当世界方向：参数空间三轴的**世界长度**
/// 差 23 倍（面内一格 ≈1.15、径向一格 0.05）⇒ 那样算出来的"法线"几乎全是切向的
/// （径向分量被压掉一个数量级，而云的轮廓恰恰靠径向）。所以每个轴都除以它自己的世界长度。
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

/// 落一个顶点，顺手焊掉已经落过的那个（返回已有顶点、或者新顶点）。
///
/// 查 27 个量子格而不是只查一个：两块在接缝上的插值可能差 1 ULP，正好跨格的那几个
/// 只按格查会漏，漏了就留开口边。
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

impl IsosurfaceOp for ProxySurface {
    type Params = Params;
    const ID: &'static str = "mesh.proxy";
    const VERSION: u32 = 1;
    const SOURCE_HASH: u64 = fnv1a(include_str!("lib.rs"));
    const INPUTS: &'static [&'static str] = &["volume"];

    fn surface(params: &Params, field: &dyn VolumeSampler) -> Result<MeshData, String> {
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
                // uv 按立方球图谱的排法：面号决定行，面内 v 在行里走
                let uv = [u, (face as f32 + v) / PATCHES as f32];
                let (welded, merged) = push_vertex(&mut cells, &mut positions, &mut uvs, point, uv, weld);
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

        // 法线：按三角形面积加权累加。零长的（碎片三角形凑不出面积）改用场的梯度 ——
        // 梯度指向场增大的那一侧，取反就是代理的外法线，内壳外壳都对；再兜不住才退回径向。
        //
        // 顺手再累两个量，只给下面的几何外扩用（`offset = 0` 时一点不影响老路径）：
        // `bend` = Σ|叉积| 是「顶点星成形程度」的尺子，`volume` 是有向体积（定缠绕里外）。
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
            volume += a[0] * (b[1] * c[2] - b[2] * c[1]) + a[1] * (b[2] * c[0] - b[0] * c[2])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
            for index in triangle {
                let slot = *index as usize * 3;
                accumulated[slot] += cross[0];
                accumulated[slot + 1] += cross[1];
                accumulated[slot + 2] += cross[2];
                bend[*index as usize] += (cross[0] * cross[0]
                    + cross[1] * cross[1]
                    + cross[2] * cross[2])
                    .sqrt();
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

        // 几何外扩：沿外法线把每个顶点往外推 `offset`（世界单位）。
        //
        // 方向这里自己算一份，不读属性里那份法线：属性不能动（`offset = 0` 要与老档逐位
        // 相同），而且它 77% 的顶点走了 `> f32::EPSILON` 那个兜底（门槛对 1e-4 量级的三角形
        // 面积太严，弯曲面上 Σ 叉积本来就只有 Σ|叉积| 的一半）⇒ 拿它定方向会把外扩推歪。
        // 三步：① 顶点星成形（|Σ叉积| ≥ Σ|叉积|/16）就用它 —— 那才是这张网格的几何法向，
        // 它跟着缠绕走，所以要按有向体积翻一次；② 否则用世界空间的场梯度取反（梯度指向
        // 云里）—— 这条天然朝外，与缠绕无关，不许再翻；③ 再退化才沿用属性里那份。
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
                    if gradient[0] * gradient[0]
                        + gradient[1] * gradient[1]
                        + gradient[2] * gradient[2]
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

        // 审计：闭合判据就是「没有开口边」。顺带数一下缠绕方向 —— 每条有向边只该出现一次，
        // 出现两次就说明有三角形翻面了（光栅化要背面剔除的话会在意）。
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
}
