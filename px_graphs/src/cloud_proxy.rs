//! 云硬表面的**代理 mesh**：驱动这一侧的标量场。
//!
//! `field = "coarse"`（默认）：细节场在 shader 里是乘上去的（`shape_of(cover, altitude, billows)`），
//! 而 `billows ∈ [0,1]` ⇒ `final ≤ coarse`。所以烘**粗场** `shape_of(cover, altitude, 1.0)` 的
//! 等值面就天然包住最终等值面，不需要任何经验外扩；代价是落点离真表面 40+ 步。
//! `field = "final"`：烘 shader `cloud_field` 那个量 ⇒ 落点贴近真表面，但不再包住（判据换成像素）。
//!
//! 这里住两件事：
//! * `CoarseVolume`：`px_ops::VolumeOp` 的实现 —— 立方球参数空间撒点、算场、存成 3D 网格。
//!   场的参照实现来自 `px_verify::cloud_field`（它依赖 `px_ops`，所以只能由驱动这边接上）。
//! * 判据仪器：射线求交（参照场的交点半径 vs 代理 mesh 的径向范围）与梯度上界 `L` 的量法。

use px_ops::field::Field;
use px_ops::noise::fnv1a;
use px_ops::{PATCHES, VolumeData, VolumeOp, point_of};
use px_verify::cloud_field::CloudFieldParams;
use serde::{Deserialize, Serialize};

pub struct CoarseVolume;

/// 烘进体积网格的**标量场**。
///
/// `Coarse` 是 `shape_of(cover, altitude, 1.0)`（噪声取上界），它在结构上包住真场
/// （`billows ≤ 1`），代价是落点离真表面很远 —— 步进起点因此离命中 40+ 步。
/// `Final` 就是 shader `cloud_field` 返回的那个量，落点贴着真表面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    #[default]
    Coarse,
    Final,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// 烘哪个场（见 `FieldKind`）。默认 `coarse` ⇒ 老配方逐位不变。
    pub field: FieldKind,
    /// 每个面 s/t 两个轴的采样点数（`res × res` 条射线）。
    pub res: u32,
    /// 径向层数。
    pub layers: u32,
    /// 壳的内外半径（世界点 = 方向 × 半径）。
    pub inner: f32,
    pub outer: f32,
    /// 硬表面的阈值 τ（就是 shader 的 `surface_level`）。
    pub tau: f32,
    /// 归一化用的梯度上界 `L`：存进体积的是 `(场 - τ) / L`。
    /// 自适应八叉树那类提取器的剪枝测试是 `|f| < size·√3`，只在 `|∇f| ≤ 1` 时不漏，
    /// 所以这个数只许大不许小（大了只费时间，小了网格出洞）。
    /// 现在的提取器是稠密 MC（插值对尺度不变）⇒ `L` 不改变 mesh，只影响存的量级。
    pub scale: f32,
    /// 保守化的邻域半径，单位是**格**（0 = 不保守，只做对照）。
    ///
    /// 点采样出来的场会漏掉节点之间的尖峰：覆盖度的细节波长只有约 2.7 格，65² 的网格
    /// 上峰值能被低估一半（实测漏掉 2/212 条方向上的整块云）。取邻域最大值 = 把覆盖度换成
    /// 它自己的膨胀 ⇒ 等值面只往外走，代价是最多外扩一格 —— 方向是安全的那一边。
    /// ⚠ 缺几何是硬失败（fragment 来自 mesh，没 mesh 就没 fragment，往回步进救不回来）。
    pub reach: u32,
    /// 云的那一档形状参数：必须与场景里 `clouds` 那一档的数一致。
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub erode: f32,
    pub orientation: [f32; 4],
}

impl Default for Params {
    fn default() -> Self {
        Self {
            field: FieldKind::Coarse,
            res: 65,
            layers: 65,
            inner: 1.01,
            outer: 1.06,
            tau: 0.20,
            scale: 240.0,
            reach: 1,
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            taper: 0.45,
            coverage_gain: 2.6,
            erode: 0.0,
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl Params {
    pub fn span(&self) -> f32 {
        self.outer - self.inner
    }

    /// 粗场的参照实现要的那一档参数。噪声三件套（`detail_scale`/`seed`）在粗场里
    /// 用不上：噪声取常数 1.0 ⇒ `mix(1-detail_strength, 1, 1) = 1`，`ceiling` 只剩 `top`。
    pub fn cloud(&self) -> CloudFieldParams {
        CloudFieldParams {
            orientation: self.orientation,
            inner: self.inner,
            outer: self.outer,
            coverage: self.coverage,
            base: self.base,
            top: self.top,
            detail_scale: 16.0,
            detail_strength: 0.55,
            erode: self.erode,
            taper: self.taper,
            coverage_gain: self.coverage_gain,
            seed: 7,
        }
    }
}

/// 某个世界方向上的覆盖度（shader 的 `coverage_of`：先转进局部系、采覆盖图、再重映射）。
/// 局部系的旋转放在这里，采样和判据就都走同一条路。
pub fn cover_at(cloud: &CloudFieldParams, coverage: &Field, direction: [f32; 3]) -> f32 {
    let local = cloud.to_local(direction);
    cloud.cover_from_mask(coverage.sample_direction(local))
}

/// 粗场：噪声取它的上界 1.0。壳外一律 0（与 shader 的早退逐位一致）。
pub fn coarse_at(cloud: &CloudFieldParams, cover: f32, altitude: f32) -> f32 {
    if altitude < 0.0 || altitude > 1.0 || cover <= 0.0 {
        return 0.0;
    }
    cloud.shape(cover, altitude, 1.0)
}

/// 真场：噪声取 shader 的 `billows`。`direction` 是世界方向，这里自己转局部 ——
/// shader 的 `medium.direction` 也是局部方向，两者必须同一个系，否则噪声不是那一份。
pub fn final_at(cloud: &CloudFieldParams, cover: f32, direction: [f32; 3], altitude: f32) -> f32 {
    if altitude < 0.0 || altitude > 1.0 || cover <= 0.0 {
        return 0.0;
    }
    let local = cloud.to_local(direction);
    cloud.shape(cover, altitude, cloud.billows(local, altitude))
}

/// 按 `params.field` 取场值：烘、判据仪器、量 `L` 都走这一条，不各抄一份。
pub fn field_at(
    cloud: &CloudFieldParams,
    params: &Params,
    cover: f32,
    direction: [f32; 3],
    altitude: f32,
) -> f32 {
    match params.field {
        FieldKind::Coarse => coarse_at(cloud, cover, altitude),
        FieldKind::Final => final_at(cloud, cover, direction, altitude),
    }
}

impl VolumeOp for CoarseVolume {
    type Params = Params;
    const ID: &'static str = "cloud.coarse";
    const VERSION: u32 = 1;
    const SOURCE_HASH: u64 = fnv1a(include_str!("cloud_proxy.rs"));
    const INPUTS: &'static [&'static str] = &["coverage"];

    fn bake(params: &Params, inputs: &[&Field]) -> VolumeData {
        let coverage = inputs[0];
        let cloud = params.cloud();
        let res = params.res.max(2);
        let layers = params.layers.max(2);
        let inv_scale = 1.0 / if params.scale > 0.0 { params.scale } else { 1.0 };

        // 每个节点的覆盖度取**切向邻域**上的最大值（保守化）。邻域撒在**方向**上：
        // 面内参数是各面自己的，两个面在接缝上的同一个节点用参数撒邻域会撒出两组不同的
        // 方向 ⇒ 两边的最大值不一样 ⇒ 接缝又焊不上了（实测 2973 条开口边）。按方向撒就没有
        // 这个问题：同一个方向算出来的是同一组邻居、同一个最大值。
        let steps = 4 * params.reach as i64;
        let radius = params.reach as f32 * tangential_cell(params);
        let mut node_cover = vec![0.0_f32; (PATCHES * res * res) as usize];
        for face in 0..PATCHES {
            for t in 0..res {
                for s in 0..res {
                    let direction = px_ops::direction_of(
                        face,
                        s as f32 / (res - 1) as f32,
                        t as f32 / (res - 1) as f32,
                    );
                    let mut best = cover_at(&cloud, coverage, direction);
                    if radius > 0.0 {
                        let (east, north) = px_ops::field::tangent_frame(direction);
                        for far in -steps..=steps {
                            for side in -steps..=steps {
                                if side * side + far * far > steps * steps {
                                    continue;
                                }
                                let across = radius * side as f32 / steps as f32;
                                let along = radius * far as f32 / steps as f32;
                                let neighbour = normalize([
                                    direction[0] + east[0] * across + north[0] * along,
                                    direction[1] + east[1] * across + north[1] * along,
                                    direction[2] + east[2] * across + north[2] * along,
                                ]);
                                best = best.max(cover_at(&cloud, coverage, neighbour));
                            }
                        }
                    }
                    node_cover[((face * res + t) * res + s) as usize] = best;
                }
            }
        }

        let mut data = vec![0.0_f32; (PATCHES * layers * res * res) as usize];
        for face in 0..PATCHES {
            for layer in 0..layers {
                let altitude = layer as f32 / (layers - 1) as f32;
                for t in 0..res {
                    for s in 0..res {
                        let cover = node_cover[((face * res + t) * res + s) as usize];
                        // 真场要在节点自己的方向上取噪声；粗场不需要 ⇒ 老路径一位没动。
                        let value = match params.field {
                            FieldKind::Coarse => coarse_at(&cloud, cover, altitude),
                            FieldKind::Final => {
                                let direction = px_ops::direction_of(
                                    face,
                                    s as f32 / (res - 1) as f32,
                                    t as f32 / (res - 1) as f32,
                                );
                                final_at(&cloud, cover, direction, altitude)
                            }
                        };
                        let slot = (((face * layers + layer) * res + t) * res + s) as usize;
                        data[slot] = (value - params.tau) * inv_scale;
                    }
                }
            }
            // 径向也做一次最大值滤波：`shape` 在壳的上下之间不是单调的，
            // 逐层取最大才是膨胀（也才会把内表面往里推、外表面往外推）。
            //
            // ⚠ 但**两头那两层不许动**：壳的上下壁上场是 0（负），一动就变正，
            // 等值面就会切在域壁上 ⇒ 代理开口（实测 3027 条开口边）。所以滤波只覆盖内层，
            // 壁上仍是原值，等值面必然留在域内。
            let reach = params.reach as usize;
            if reach > 0 && layers as usize > 2 * reach + 1 {
                let mut filtered = vec![0.0_f32; (layers * res * res) as usize];
                for layer in 0..layers as usize {
                    let low = layer.saturating_sub(reach);
                    let high = (layer + reach).min(layers as usize - 1);
                    for t in 0..res as usize {
                        for s in 0..res as usize {
                            let mut best = f32::MIN;
                            for other in low..=high {
                                let slot = (((face * layers + other as u32) * res
                                    + t as u32)
                                    * res
                                    + s as u32) as usize;
                                best = best.max(data[slot]);
                            }
                            filtered[(layer * res as usize + t) * res as usize + s] = best;
                        }
                    }
                }
                for layer in 1..layers as usize - 1 {
                    for t in 0..res as usize {
                        for s in 0..res as usize {
                            let slot = (((face * layers + layer as u32) * res + t as u32) * res
                                + s as u32) as usize;
                            data[slot] = filtered[(layer * res as usize + t) * res as usize + s];
                        }
                    }
                }
            }
        }
        VolumeData {
            res,
            layers,
            inner: params.inner,
            outer: params.outer,
            data,
        }
    }
}

/// 最粗的那一格的切向宽度（世界单位）。立方球的参数化在面心拉伸、在角上压缩，
/// 取面内各处的最大值 ⇒ 保守。
pub fn tangential_cell(params: &Params) -> f32 {
    let res = params.res.max(2);
    let step = 1.0 / (res - 1) as f32;
    let mut worst = 0.0_f32;
    for face in 0..PATCHES {
        for t in 0..=16 {
            for s in 0..=16 {
                let u = s as f32 / 16.0;
                let v = t as f32 / 16.0;
                let here = px_ops::direction_of(face, u, v);
                for (du, dv) in [(step, 0.0), (0.0, step)] {
                    let there = px_ops::direction_of(face, u + du, v + dv);
                    let far = ((there[0] - here[0]).powi(2)
                        + (there[1] - here[1]).powi(2)
                        + (there[2] - here[2]).powi(2))
                    .sqrt()
                        * params.outer;
                    worst = worst.max(far);
                }
            }
        }
    }
    worst
}

// ---------------------------------------------------------------------------
// 判据仪器
// ---------------------------------------------------------------------------

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
    let cover = cover_at(cloud, coverage, direction);
    if cover <= 0.0 {
        return Vec::new();
    }
    let samples = samples.max(2);
    let value_at =
        |altitude: f32| field_at(cloud, params, cover, direction, altitude) - params.tau;
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
pub fn mesh_roots(mesh: &px_ops::MeshData, direction: [f32; 3]) -> Vec<f32> {
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
    mesh: &px_ops::MeshData,
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
        let direction = px_ops::direction_of(face, u, v);
        let cover = cover_at(cloud, coverage, direction);
        field_at(cloud, params, cover, direction, altitude)
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
                            direction: px_ops::direction_of(face, u, v),
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
