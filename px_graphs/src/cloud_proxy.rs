//! 云硬表面代理的**判据仪器**：射线求交（参照场的交点半径 vs 代理 mesh 的径向范围）
//! 与梯度上界 `L` 的量法。
//!
//! ⚠ 算子（`CoarseVolume`）**不在这里**了：它搬去了 `px_volume_op`（dylib）。
//! 这份仪器留在图脚本这一侧，是因为它要**静态**用（判据不能依赖动态装载的成败），
//! 而它用的参照场住在 `px_verify::proxy`（算子用的也是同一份 ⇒ 两边判的是同一个东西）。

use px_field_schema::field::Field;
use px_mesh_schema::MeshData;
use px_volume_schema::{PATCHES, params::Params, point_of};
use px_verify::cloud_field::CloudFieldParams;
use px_verify::proxy;

fn sub(one: [f32; 3], two: [f32; 3]) -> [f32; 3] {
    [one[0] - two[0], one[1] - two[1], one[2] - two[2]]
}

fn cross(one: [f32; 3], two: [f32; 3]) -> [f32; 3] {
    [
        one[1] * two[2] - one[2] * two[1],
        one[2] * two[0] - one[0] * two[2],
        one[0] * two[1] - one[1] * two[0],
    ]
}

fn dot(one: [f32; 3], two: [f32; 3]) -> f32 {
    one[0] * two[0] + one[1] * two[1] + one[2] * two[2]
}

fn length(vector: [f32; 3]) -> f32 {
    dot(vector, vector).sqrt()
}

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let size = length(vector);
    if size <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [vector[0] / size, vector[1] / size, vector[2] / size]
}

/// 一串确定性的伪随机方向（不依赖 rand，跨进程一致）。
pub fn scatter_directions(seed: u64, count: usize) -> Vec<[f32; 3]> {
    let mut state = seed | 1;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f32 / (1u64 << 53) as f32
    };
    (0..count)
        .map(|_| {
            let z = next() * 2.0 - 1.0;
            let phi = next() * std::f32::consts::TAU;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            normalize([ring * phi.cos(), z, ring * phi.sin()])
        })
        .collect()
}

/// 参照场 `= τ` 在这条射线上的全部交点半径（暴力扫 + 二分）。
pub fn coarse_roots(
    cloud: &CloudFieldParams,
    coverage: &Field,
    params: &Params,
    direction: [f32; 3],
    samples: usize,
) -> Vec<f32> {
    let cover = proxy::cover_at(cloud, coverage, direction);
    if cover <= 0.0 {
        return Vec::new();
    }
    let samples = samples.max(2);
    let value_at =
        |altitude: f32| proxy::field_at(cloud, params, cover, direction, altitude) - params.tau;
    let mut roots = Vec::new();
    let mut previous_altitude = 0.0_f32;
    let mut previous = value_at(0.0);
    for step in 1..=samples {
        let altitude = step as f32 / samples as f32;
        let value = value_at(altitude);
        if previous == 0.0 {
            roots.push(params.inner + previous_altitude * params.span());
        }
        if previous * value < 0.0 {
            let (mut low, mut high) = (previous_altitude, altitude);
            let mut low_value = previous;
            for _ in 0..40 {
                let middle = 0.5 * (low + high);
                let middle_value = value_at(middle);
                if low_value * middle_value <= 0.0 {
                    high = middle;
                } else {
                    low = middle;
                    low_value = middle_value;
                }
            }
            roots.push(params.inner + 0.5 * (low + high) * params.span());
        }
        previous_altitude = altitude;
        previous = value;
    }
    roots.sort_by(|one, two| one.partial_cmp(two).unwrap_or(std::cmp::Ordering::Equal));
    roots
}

/// 代理 mesh 在这条射线上的交点半径。
pub fn mesh_roots(mesh: &MeshData, direction: [f32; 3]) -> Vec<f32> {
    let vertex = |index: u32| -> [f32; 3] {
        let slot = index as usize * 3;
        [
            mesh.positions[slot],
            mesh.positions[slot + 1],
            mesh.positions[slot + 2],
        ]
    };
    let mut roots = Vec::new();
    for triangle in mesh.indices.chunks_exact(3) {
        let a = vertex(triangle[0]);
        let b = vertex(triangle[1]);
        let c = vertex(triangle[2]);
        let edge_one = sub(b, a);
        let edge_two = sub(c, a);
        let pvec = cross(direction, edge_two);
        let det = dot(edge_one, pvec);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let tvec = sub([0.0, 0.0, 0.0], a);
        let u = dot(tvec, pvec) * inv;
        if !(-1e-6..=1.0 + 1e-6).contains(&u) {
            continue;
        }
        let qvec = cross(tvec, edge_one);
        let v = dot(direction, qvec) * inv;
        if v < -1e-6 || u + v > 1.0 + 1e-6 {
            continue;
        }
        let t = dot(edge_two, qvec) * inv;
        if t > 0.0 {
            roots.push(t);
        }
    }
    roots.sort_by(|one, two| one.partial_cmp(two).unwrap_or(std::cmp::Ordering::Equal));
    roots
}

/// 一个方向上一格的世界尺寸（三轴合成对角线）——判据里「小于一个单元对角线」用的就是它。
pub fn cell_diagonal(params: &Params, direction: [f32; 3], radius: f32) -> f32 {
    let res = params.res.max(2);
    let layers = params.layers.max(2);
    let (face, u, v) = px_protocol::art::cube_face_of(direction);
    let altitude = ((radius - params.inner) / params.span()).clamp(0.0, 1.0);
    let point = point_of(face, [u, v, altitude], params.inner, params.outer);
    let step_s = point_of(
        face,
        [(u + 1.0 / (res - 1) as f32).min(1.0), v, altitude],
        params.inner,
        params.outer,
    );
    let step_t = point_of(
        face,
        [u, (v + 1.0 / (res - 1) as f32).min(1.0), altitude],
        params.inner,
        params.outer,
    );
    let step_r = params.span() / (layers - 1) as f32;
    let one = length(sub(step_s, point));
    let two = length(sub(step_t, point));
    (one * one + two * two + step_r * step_r).sqrt()
}

#[derive(Debug, Clone)]
pub struct Containment {
    pub rays: usize,
    pub rays_with_surface: usize,
    /// 内边界余量：粗场最里面的交点半径 − 代理最里面的交点半径（正 = 代理更靠里 = 包住）。
    pub worst_inner: f32,
    /// 外边界余量：代理最外面的交点半径 − 粗场最外面的交点半径（正 = 包住）。
    pub worst_outer: f32,
    /// 两者取小，就是这条判据的余量（正 = 包住）。
    pub worst_slack: f32,
    pub worst_direction: [f32; 3],
    pub worst_cell: f32,
    /// 粗场在这条方向上有交点、代理 mesh 却一个交点都没有的方向数（硬失败）。
    pub missing: usize,
    /// 粗场的交点区间没被代理的任何区间包含的方向数（比径向范围更严的一档，仅报告）。
    pub intervals_outside: usize,
}

/// 判据 2：沿 `rays` 个随机方向暴力求交，断言代理的径向范围包住粗场的全部交点半径。
pub fn containment(
    mesh: &MeshData,
    cloud: &CloudFieldParams,
    coverage: &Field,
    params: &Params,
    rays: usize,
    samples: usize,
) -> Containment {
    let mut report = Containment {
        rays,
        rays_with_surface: 0,
        worst_inner: f32::INFINITY,
        worst_outer: f32::INFINITY,
        worst_slack: f32::INFINITY,
        worst_direction: [0.0, 1.0, 0.0],
        worst_cell: 0.0,
        missing: 0,
        intervals_outside: 0,
    };
    for direction in scatter_directions(0x9e37_79b9_7f4a_7c15, rays) {
        let roots = coarse_roots(cloud, coverage, params, direction, samples);
        if roots.is_empty() {
            continue;
        }
        report.rays_with_surface += 1;
        let hits = mesh_roots(mesh, direction);
        if hits.is_empty() {
            report.missing += 1;
            continue;
        }
        let inner = roots[0] - hits[0];
        let outer = hits[hits.len() - 1] - roots[roots.len() - 1];
        report.worst_inner = report.worst_inner.min(inner);
        report.worst_outer = report.worst_outer.min(outer);
        let slack = inner.min(outer);
        if slack < report.worst_slack {
            report.worst_slack = slack;
            report.worst_direction = direction;
            report.worst_cell = cell_diagonal(
                params,
                direction,
                0.5 * (roots[0] + roots[1.min(roots.len() - 1)]),
            );
        }
        // 区间级：粗场的每一段 [低, 高] 都要落在代理的某一段里
        let mut outside = false;
        for pair in roots.chunks(2) {
            let (low, high) = (pair[0], *pair.last().expect("非空"));
            let covered = hits
                .chunks(2)
                .any(|span| span[0] <= low && *span.last().expect("非空") >= high);
            if !covered {
                outside = true;
            }
        }
        if outside {
            report.intervals_outside += 1;
        }
    }
    if report.rays_with_surface == 0 {
        report.worst_slack = f32::NAN;
        report.worst_inner = f32::NAN;
        report.worst_outer = f32::NAN;
    }
    report
}

#[derive(Debug, Clone, Copy)]
pub struct Bound {
    pub bound: f32,
    /// 三个轴上各自最大的单边差商，用来看出 L 是被哪一维顶起来的。
    pub axes: [f32; 3],
    pub face: u32,
    /// 面内参数 `(u, v, 径向高度)`。
    pub at: [f32; 3],
    pub direction: [f32; 3],
    pub value: f32,
}

/// `L` 的量法：在参数空间 `(s, t, 高度)` 上对粗场做单边和中心差商，取欧氏范数的最大值。
///
/// 剪枝测试用的是各向同性的域 `[0,1]³`，所以量的是三轴合成的范数。粗场里有 `clamp`
/// 造成的折点，折点处中心差商会低估一侧的斜率 ⇒ 单边差商也一并量，取三者的最大。
pub fn measure_gradient_bound(
    cloud: &CloudFieldParams,
    coverage: &Field,
    params: &Params,
    faces: u32,
    res: u32,
    layers: u32,
) -> Bound {
    let res = res.max(2);
    let layers = layers.max(2);
    let step = 1.0 / 1024.0;
    let value = |face: u32, u: f32, v: f32, altitude: f32| -> f32 {
        let direction = px_volume_schema::direction_of(face, u, v);
        let cover = proxy::cover_at(cloud, coverage, direction);
        proxy::field_at(cloud, params, cover, direction, altitude)
    };
    let mut bound = Bound {
        bound: 0.0,
        axes: [0.0; 3],
        face: 0,
        at: [0.0; 3],
        direction: [0.0, 1.0, 0.0],
        value: 0.0,
    };
    for face in 0..faces.min(PATCHES) {
        for t in 0..res {
            let v = t as f32 / (res - 1) as f32;
            for s in 0..res {
                let u = s as f32 / (res - 1) as f32;
                for layer in 0..layers {
                    let altitude = layer as f32 / (layers - 1) as f32;
                    let here = value(face, u, v, altitude);
                    if here <= 0.0 {
                        continue;
                    }
                    let mut worst = [0.0_f32; 3];
                    for axis in 0..3 {
                        let point = [u, v, altitude];
                        // 单边差商：域边界上只量往里那一侧（另一侧的间距是 0）。
                        // 两边的斜率都能量时取绝对值大的那个 —— 粗场里有 `clamp` 的折点，
                        // 折点处中心差商会低估斜率。
                        for sign in [-1.0_f32, 1.0] {
                            let mut other = point;
                            other[axis] = point[axis] + sign * step;
                            if other[axis] < 0.0 || other[axis] > 1.0 {
                                continue;
                            }
                            let delta = (other[axis] - point[axis]).abs();
                            if delta <= 0.0 {
                                continue;
                            }
                            let slope =
                                (value(face, other[0], other[1], other[2]) - here) / delta;
                            if slope.abs() > worst[axis].abs() {
                                worst[axis] = slope;
                            }
                        }
                    }
                    let norm =
                        (worst[0] * worst[0] + worst[1] * worst[1] + worst[2] * worst[2]).sqrt();
                    if norm > bound.bound {
                        bound = Bound {
                            bound: norm,
                            axes: worst,
                            face,
                            at: [u, v, altitude],
                            direction: px_volume_schema::direction_of(face, u, v),
                            value: here,
                        };
                    }
                }
            }
        }
    }
    bound
}

/// 一行给报告用的打印。
pub fn print_containment(report: &Containment, params: &Params) {
    println!(
        "包住判据：{} 条方向里 {} 条粗场有交点；{} 条代理找不到交点；{} 条的区间没被包住",
        report.rays, report.rays_with_surface, report.missing, report.intervals_outside,
    );
    println!(
        "  最差方向 {:?}：内边界余量 {:+.6}、外边界余量 {:+.6}（世界单位，正 = 包住）｜该方向一格的单元对角线 {:.6}｜余量/对角线 {:+.3}",
        report.worst_direction.map(|value| (value * 1000.0).round() / 1000.0),
        report.worst_inner,
        report.worst_outer,
        report.worst_cell,
        report.worst_slack / report.worst_cell,
    );
    println!(
        "  壳厚 {}（inner {} outer {}）⇒ 一格的单元对角线占壳厚的 {:.1}%",
        params.span(),
        params.inner,
        params.outer,
        100.0 * report.worst_cell / params.span(),
    );
}
