//! 细分的两条算法：曲线按弦误差摊成折线、曲面按弦误差摊成三角网格。
//!
//! ⚠ 判据是**弦误差**（真实曲面与那张近似网格的偏差），不是"切多细"：
//!   `tolerance` 是世界单位的上界 ⇒ 调它就是在"顶点数"与"看起来是圆的"之间选。
//!   `depth` 是**保底**（尖点处不允许无限递归）。
//!
//! ⚠ 相邻格的级别最多差一级（四叉树配平）。不平配也能出图，但两条边一个分两半、
//!   一个不分，接缝上就会多出"只在一边存在"的顶点 —— 那正是网格裂缝。

use std::collections::HashMap;

use px_nurbs_schema::curve::Curve;
use px_nurbs_schema::surface::Surface;
use px_protocol::art::MeshData;

/// **曲线 → 折线网格**：逐段对分，直到中点到弦的距离小于 `tolerance`。
///
/// ⚠ 输出仍是 `MeshData`（这是"曲线也能进渲染管线"的那条路）：三角形是**退化**的
///   （退化成那条折线本身），于是这套形状与曲面那一档逐条同构 —— 法线是曲线所在平面的
///   法线，uv 的 `u` 是参数。闭合曲线（首尾同一个点）会把首尾并成一个顶点。
pub fn curve(curve: &Curve, tolerance: f64, depth: u32, segments: u32) -> Result<MeshData, String> {
    let (low, high) = curve.domain();
    let steps = segments.max(1) as usize;
    let mut values: Vec<f64> = (0..=steps)
        .map(|step| low + (high - low) * step as f64 / steps as f64)
        .collect();
    let mut points: Vec<[f64; 3]> = values
        .iter()
        .map(|value| curve.point(*value))
        .collect::<Result<_, _>>()?;

    for _ in 0..depth {
        let mut next_values = Vec::with_capacity(values.len() * 2 - 1);
        let mut next_points: Vec<[f64; 3]> = Vec::with_capacity(points.len() * 2 - 1);
        let mut split = false;
        for index in 0..values.len() - 1 {
            let a = points[index];
            let b = points[index + 1];
            let middle = (values[index] + values[index + 1]) * 0.5;
            let point = curve.point(middle)?;
            let chord = [
                (a[0] + b[0]) * 0.5,
                (a[1] + b[1]) * 0.5,
                (a[2] + b[2]) * 0.5,
            ];
            next_values.push(values[index]);
            next_points.push(a);
            if distance(point, chord) > tolerance {
                split = true;
                next_values.push(middle);
                next_points.push(point);
            }
        }
        next_values.push(values[values.len() - 1]);
        next_points.push(points[points.len() - 1]);
        values = next_values;
        points = next_points;
        if !split {
            break;
        }
    }

    let closed = distance(points[0], points[points.len() - 1]) <= 1e-12;
    let keep = if closed {
        points.len() - 1
    } else {
        points.len()
    };
    let normal = px_nurbs_schema::curve::plane_normal(curve);
    let mut positions = Vec::with_capacity(keep * 3);
    let mut normals = Vec::with_capacity(keep * 3);
    let mut uvs = Vec::with_capacity(keep * 2);
    for index in 0..keep {
        positions.extend_from_slice(&points[index]);
        normals.extend_from_slice(&normal);
        uvs.extend_from_slice(&[values[index], 0.0]);
    }
    // 退化三角形：`(a, b, b)` 的面积为 0 ⇒ 渲染出来就是那条折线（与"线条"同一套网格）。
    let mut indices = Vec::with_capacity(keep * 6);
    let span = if closed { keep } else { keep - 1 };
    for index in 0..span {
        let a = index as u32;
        let b = ((index + 1) % keep) as u32;
        indices.extend_from_slice(&[a, b, b, b, b, b]);
    }

    Ok(mesh(positions, normals, uvs, indices))
}

/// **曲面 → 三角网格**：四叉树细分（2:1 配平），直到格中点偏离双线性近似小于 `tolerance`。
/// **曲面 → 三角网格**：**统一**细分，直到每一格的中心偏离四个角平均（弦误差的估计）
/// 都不超过 `tolerance`，或者到达深度上限。
///
/// ⚠ 为什么是**统一**细分而不是自适应四叉树：自适应会留下 **T 形接点**（粗格那条边上
///   有细格分出来的顶点），于是"每条边恰好被两个三角形用到"不成立 —— 网格看着没缝
///   （顶点就在那条直线上），但判据上是**开口**的。要真正水密就得再写一套"挂点扇形"
///   把粗格那条边按邻居的顶点切开；这一档选了简单且**真水密**的那个。
///   代价是平坦处也多切了几刀，而 `tolerance` 仍然是弦误差上界（判据的那一条不变）。
pub fn surface(
    surface: &Surface,
    tolerance: f64,
    depth: u32,
    segments: u32,
) -> Result<MeshData, String> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    if !(u1 > u0) || !(v1 > v0) {
        return Err("细分一份参数域为空的曲面".to_string());
    }
    let steps = u64::from(segments.clamp(1, 64));
    let depth = depth.min(16);
    let mut level = 0_u32;
    loop {
        let unit = steps << level;
        let mut worst = 0.0_f64;
        for i in 0..unit {
            for j in 0..unit {
                let error = cell_error(surface, u0, u1, v0, v1, unit, i, j)?;
                if error > worst {
                    worst = error;
                }
            }
        }
        if worst <= tolerance || level >= depth {
            let mut cache = Sample::new(surface, u0, u1, v0, v1, unit);
            let mut indices = Vec::with_capacity((unit * unit * 6) as usize);
            for i in 0..unit {
                for j in 0..unit {
                    let a = cache.vertex(i, j)?;
                    let b = cache.vertex(i + 1, j)?;
                    let c = cache.vertex(i, j + 1)?;
                    let d = cache.vertex(i + 1, j + 1)?;
                    indices.extend_from_slice(&[a, b, c, c, b, d]);
                }
            }
            return Ok(weld(
                std::mem::take(&mut cache.positions),
                std::mem::take(&mut cache.normals),
                std::mem::take(&mut cache.uvs),
                indices,
            ));
        }
        level += 1;
    }
}

/// 一格的中心偏离"四个角的平均"多远（弦误差的估计）。
///
/// ⚠ 这里**不进**下面那个顶点缓存：误差探针要的中心点不是一个网格顶点，
///   塞进缓存会留下一堆没人用的孤立顶点。
fn cell_error(
    surface: &Surface,
    u0: f64,
    u1: f64,
    v0: f64,
    v1: f64,
    unit: u64,
    i: u64,
    j: u64,
) -> Result<f64, String> {
    let parameter = |index: u64| -> (f64, f64) {
        (
            u0 + (u1 - u0) * index as f64 / unit as f64,
            v0 + (v1 - v0) * index as f64 / unit as f64,
        )
    };
    let (u, v) = parameter(i);
    let (next_u, next_v) = parameter(i + 1);
    let (left, bottom) = parameter(j);
    let (right, top) = parameter(j + 1);
    let _ = (v, next_v, left, right);
    let corners = [
        surface.point(u, bottom)?,
        surface.point(next_u, bottom)?,
        surface.point(u, top)?,
        surface.point(next_u, top)?,
    ];
    let middle = surface.point((u + next_u) * 0.5, (bottom + top) * 0.5)?;
    Ok(distance(middle, average4(corners)))
}

/// **焊顶点 + 扔退化三角形**：把重合的顶点并成一个。
///
/// ⚠ 这一步是"闭合曲面水密"的落点：参数域的两条缝（u 的 0 与 1、以及两极那种整行
///   退化成同一个点的纬线）在**网格下标**上是不同的顶点，只有把重合的焊起来，
///   "每条边恰好被两个三角形用到"才成立。焊完再扔掉 `a == b` 那种退化三角形
///   （极点那一格的半个是退化的）。
///
/// ⚠ 判"重合"用的是**与包围盒成正比**的容差（`对角线 · 1e-6`），不是逐位相等：
///   缝上那两个点在数学上是同一点，但各自那条路上的 `f64` 舍入不同 ——
///   实测哪怕只差 `1e-16`，转成 `f32` 就是两个不同的位型，逐位比会**焊不上**
///   （症状：缝上留一圈开口边）。容差按尺度走，大模型小模型都不会误焊。
fn weld(positions: Vec<f64>, normals: Vec<f64>, uvs: Vec<f64>, indices: Vec<u32>) -> MeshData {
    let positions: Vec<f32> = positions.iter().map(|value| *value as f32).collect();
    let normals: Vec<f32> = normals.iter().map(|value| *value as f32).collect();
    let uvs: Vec<f32> = uvs.iter().map(|value| *value as f32).collect();
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for vertex in 0..positions.len() / 3 {
        for lane in 0..3 {
            let value = f64::from(positions[vertex * 3 + lane]);
            low[lane] = low[lane].min(value);
            high[lane] = high[lane].max(value);
        }
    }
    let diagonal = {
        let delta = [high[0] - low[0], high[1] - low[1], high[2] - low[2]];
        (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
    };
    let step = (diagonal * 1e-6).max(f64::MIN_POSITIVE);
    let key = |vertex: usize| -> [i64; 3] {
        let mut out = [0_i64; 3];
        for lane in 0..3 {
            out[lane] = (f64::from(positions[vertex * 3 + lane]) / step).round() as i64;
        }
        out
    };

    let mut canonical: Vec<u32> = Vec::with_capacity(positions.len() / 3);
    let mut lookup: HashMap<[i64; 3], u32> = HashMap::new();
    let mut kept_positions: Vec<f32> = Vec::new();
    let mut kept_normals: Vec<f32> = Vec::new();
    let mut kept_uvs: Vec<f32> = Vec::new();
    for vertex in 0..positions.len() / 3 {
        let index = match lookup.get(&key(vertex)) {
            Some(found) => *found,
            None => {
                let index = (kept_positions.len() / 3) as u32;
                kept_positions.extend_from_slice(&positions[vertex * 3..vertex * 3 + 3]);
                kept_normals.extend_from_slice(&normals[vertex * 3..vertex * 3 + 3]);
                kept_uvs.extend_from_slice(&uvs[vertex * 2..vertex * 2 + 2]);
                lookup.insert(key(vertex), index);
                index
            }
        };
        canonical.push(index);
    }
    let mut kept_indices = Vec::with_capacity(indices.len());
    for triangle in indices.chunks_exact(3) {
        let a = canonical[triangle[0] as usize];
        let b = canonical[triangle[1] as usize];
        let c = canonical[triangle[2] as usize];
        if a != b && b != c && a != c {
            kept_indices.extend_from_slice(&[a, b, c]);
        }
    }
    MeshData {
        positions: kept_positions,
        normals: kept_normals,
        uvs: kept_uvs,
        indices: kept_indices,
    }
}

/// 采样缓存：格点按索引存，**相邻格共用**。
///
/// ⚠ 共用是判据的一部分：两格共用一条边时，那条边上的顶点必须是**同一个** `MeshData`
///   顶点（下标相同），否则网格在缝上裂开 ——「开口边」那道判据量到的就是它。
struct Sample<'a> {
    surface: &'a Surface,
    u0: f64,
    u1: f64,
    v0: f64,
    v1: f64,
    unit: u64,
    points: HashMap<(u64, u64), u32>,
    positions: Vec<f64>,
    normals: Vec<f64>,
    uvs: Vec<f64>,
}

impl<'a> Sample<'a> {
    fn new(surface: &'a Surface, u0: f64, u1: f64, v0: f64, v1: f64, unit: u64) -> Self {
        Self {
            surface,
            u0,
            u1,
            v0,
            v1,
            unit,
            points: HashMap::new(),
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
        }
    }

    fn vertex(&mut self, i: u64, j: u64) -> Result<u32, String> {
        if let Some(found) = self.points.get(&(i, j)) {
            return Ok(*found);
        }
        let u = self.u0 + (self.u1 - self.u0) * i as f64 / self.unit as f64;
        let v = self.v0 + (self.v1 - self.v0) * j as f64 / self.unit as f64;
        let patch = self.surface.patch(u, v)?;
        let index = (self.positions.len() / 3) as u32;
        self.positions.extend_from_slice(&patch.point);
        let normal = patch.normal.unwrap_or([0.0, 1.0, 0.0]);
        self.normals.extend_from_slice(&normal);
        self.uvs.extend_from_slice(&[u, v]);
        self.points.insert((i, j), index);
        Ok(index)
    }
}

fn distance(one: [f64; 3], two: [f64; 3]) -> f64 {
    let delta = [one[0] - two[0], one[1] - two[1], one[2] - two[2]];
    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
}

fn average4(points: [[f64; 3]; 4]) -> [f64; 3] {
    let mut out = [0.0_f64; 3];
    for point in points {
        for lane in 0..3 {
            out[lane] += point[lane] * 0.25;
        }
    }
    out
}

fn mesh(positions: Vec<f64>, normals: Vec<f64>, uvs: Vec<f64>, indices: Vec<u32>) -> MeshData {
    MeshData {
        positions: positions.iter().map(|value| *value as f32).collect(),
        normals: normals.iter().map(|value| *value as f32).collect(),
        uvs: uvs.iter().map(|value| *value as f32).collect(),
        indices,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **细分出来的每个顶点都在球面上**（半径精确是有理二次表示那条性质，
    /// 细分只是采样它，不该把它做丢）。
    ///
    /// ⚠ 容差是 `f32` 那一档的：网格顶点按 `f32` 出厂（`MeshData` 的线格式就是 `f32`），
    ///   所以这里量的是"细分 + 降精度"之后还在不在球面上。
    #[test]
    fn the_tessellated_sphere_stays_on_the_radius() {
        let ball = px_nurbs_schema::surface::sphere(2.0, [0.0; 3], 2).expect("造球");
        let mesh = surface(&ball, 1e-3, 4, 2).expect("细分");
        assert!(mesh.vertices() > 0 && mesh.triangles() > 0);
        for vertex in 0..mesh.vertices() {
            let point = &mesh.positions[vertex * 3..vertex * 3 + 3];
            let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
            assert!(
                (radius - 2.0).abs() < 1e-5,
                "第 {vertex} 个顶点离球心 {radius}（应当是 2）"
            );
        }
    }

    /// **闭合网格没有开口边**：每条边恰好被两个三角形用到（极点那几格靠"零面积三角形
    /// 不进网格"这一条收口）。
    #[test]
    fn the_tessellated_sphere_is_watertight() {
        let ball = px_nurbs_schema::surface::sphere(1.0, [0.0; 3], 2).expect("造球");
        let mesh = surface(&ball, 1e-2, 3, 2).expect("细分");
        let mut edges: std::collections::HashMap<(u32, u32), usize> =
            std::collections::HashMap::new();
        for triangle in mesh.indices.chunks_exact(3) {
            for pair in 0..3 {
                let one = triangle[pair];
                let two = triangle[(pair + 1) % 3];
                let key = if one < two { (one, two) } else { (two, one) };
                *edges.entry(key).or_default() += 1;
            }
        }
        let open: Vec<_> = edges.iter().filter(|(_, count)| **count != 2).collect();
        assert!(
            open.is_empty(),
            "有 {} 条开口边（共 {} 条边）：{:?}",
            open.len(),
            edges.len(),
            &open[..open.len().min(5)]
        );
    }

    /// **折线落在圆上**，并且**首尾并成一个顶点**（闭合曲线不该在接缝处裂开）。
    #[test]
    fn the_tessellated_circle_closes_on_itself() {
        let circle = px_nurbs_schema::curve::circle(1.0, "xy", [0.0; 3]).expect("造圆");
        let mesh = curve(&circle, 1e-3, 6, 4).expect("细分");
        assert!(mesh.vertices() >= 8);
        for vertex in 0..mesh.vertices() {
            let point = &mesh.positions[vertex * 3..vertex * 3 + 3];
            let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
            assert!(
                (radius - 1.0).abs() < 1e-6,
                "第 {vertex} 个顶点离圆心 {radius}（应当是 1）"
            );
            assert!(point[2].abs() < 1e-6, "第 {vertex} 个顶点跑出了 xy 平面");
        }
        // 闭合：最后一段把最后一个顶点接回第 0 个（接缝处只留一个顶点）。
        let first = mesh.indices[0];
        let seam = mesh.indices[mesh.indices.len() - 3];
        assert_eq!(
            mesh.positions[seam as usize * 3..seam as usize * 3 + 2],
            mesh.positions[first as usize * 3..first as usize * 3 + 2],
            "最后一段没有接回第 0 个顶点"
        );
    }
}
