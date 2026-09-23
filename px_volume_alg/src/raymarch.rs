//! `sky.nebula`：**沿视线积分**（发射-吸收模型），出一条通道的天空。
//!
//! ```text
//! L(ray) = Σ_k  T_k · j(x_k) · ds  +  T_end · (星点 + 背景)
//! T_k    = exp( −Σ_{j<k} σ_a(x_j) · ds )
//! ```
//!
//! 相机在**壳的中心**（球心），所以每条视线的弦长都不同、穿过的气也不同
//! ⇒ **纵深是免费的**：亮不是因为"这一格的密度高"，而是因为"沿这条视线积得长"。
//!
//! ⚠ **一次只出一条通道**（`channel` 0/1/2）。理由不是偏好，是依赖方向：
//!   `px_volume_alg` 在 `px_graph` **下面**（图驱动依赖算法），所以这一档
//!   **不可能**返回 `px_graph::generate::TextureData`。⇒ 它的出路是"一条通道一张
//!   `CubeMap` 场"，三张由图脚本拼成贴图（`px_graphs` 那一层两边都够得着）。
//!
//! ⚠ 这一档**不造形状、不算光照**：形状在上游的 `field.fbm3` / `field.warp3` 里，
//!   光照在 `cloud.emission` 里（按体素算过一遍）。这里只有积分。

use px_field_schema::field::{CUBE_FACES, Field, Projection, art_direction_at, cube_face_of};
use px_sparse::StarField;
use px_volume_schema::params::sky::SkyParams;
use px_volume_schema::{TextureData, TextureFormat, VolumeData};

/// 格子的确定性抖动：同一格永远同一个偏移。
///
/// ⚠ **必须逐格逐步都不同**（`texel` 与 `step` 一起进哈希）：第一版写成
///   `jitter_at(texel * 1024 + step)`，而 `texel = (y * face + x) ^ (face << 20)`
///   —— 同一个 `y` 上 `texel * 1024` 的低位被 `+ step` 主导，于是**整行拿到同一个偏移**
///   ⇒ 画面上一道道**横向条纹**（实测：六面平铺图上明显的等距横线）。
///   抖动的本意是"打散层状条纹"，写错反而造出另一种条纹。
///
/// ⚠ **不许**用时间或真随机：那会让同一份参数烘出两张不同的图，而缓存键是"键 = 内容"
///   ——"同参数同产物"是全仓的地基。
fn jitter_at(texel: u32, step: u32, seed: u32) -> f32 {
    let mut hash = texel
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(step.wrapping_mul(0x85eb_ca6b))
        .wrapping_add(seed.wrapping_mul(0x27d4_eb2d));
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x2c1b_3c6d);
    hash ^= hash >> 12;
    (hash & 0xffff) as f32 / 65535.0
}

/// 一条视线的**星点候选**：查一次 R3 星场就够（与"步进到哪一步"无关）。
///
/// ⚠ 半径要**留着**：每一颗星用**它自己那一步**的透过率（近处的星不被整层气遮住、
///   远处的被前面的气吃掉）—— 那正是"星嵌在星云里"这件事。
#[derive(Clone, Copy)]
struct StarHit {
    /// 星到相机（原点）的距离。
    radius: f32,
    /// 视线与星方向的夹角正弦（`PSF` 用它，见 [`star_power`]）。
    sine: f32,
    /// 星自己的光（亮度 × 色）。
    power: [f32; 3],
}

/// 一颗星的**角向轮廓**（核 + 晕），自变量是 `sin θ`。
///
/// ⚠⚠ 用 `sin θ` 而不是 `θ`：`θ` 要么走 `acos`（贵的），要么在近轴处丢掉精度；
///   而支持域本来就只有几十毫弧度 ⇒ `sin θ` 与 `θ` 的差在 `1e-5` 量级（可以忽略），
///   换成它之后两侧都只剩两次平方根、零次反三角。
///
/// ⚠⚠ 这是**世界空间**的量（弧度），与"这颗星落在哪个纹素、哪个面"完全无关 ——
///   旧版把核宽写成"几个纹素"（`π / width`），于是世界空间里一个点的大小被**存储网格
///   的斜度**决定（实测面心 σ 径向/切向 1.02、面角 1.57，一个圆被存成椭圆）。
///   现在这颗星的形状只由 `star_core` / `star_halo` 两个弧度数决定，**放在哪里都一样**。
pub fn star_power(sine: f32, params: &SkyParams) -> f32 {
    let core = (params.star_core.max(1e-6)).max(1e-6);
    let halo = params.star_halo.max(1e-6);
    let core_term = (-(sine / core).powi(2)).exp();
    let halo_term = (-(sine / halo).powi(2)).exp();
    core_term + params.star_halo_gain * halo_term
}

/// `PSF` 的**支持域**（角度正弦）：再远的地方 `exp(−9) ≈ 1.2e-4`，乘上增益已经读不出来。
pub fn star_support(params: &SkyParams) -> f32 {
    3.0 * params.star_core.max(params.star_halo).max(1e-6)
}

/// **判据与探针用**：一条视线上"每层收下几颗星"（GPU 那份定长队列该开多大，靠这个数说话）。
///
/// ⚠ 它回的是**逐层的候选数**，不是总数：GPU 的待消费队列是逐层收、按半径消费的
///   （队列里最多同时待着一层多一点的星），所以上限由"单层最多几颗"决定，而不是总数。
pub fn slab_candidate_counts(
    field: &StarField,
    direction: [f32; 3],
    enter: f32,
    exit: f32,
    support: f32,
) -> Vec<usize> {
    let cell = field.grid.meta.cell;
    let mut out = Vec::new();
    let mut t0 = (enter - cell).max(0.0);
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    while t0 <= exit {
        let t1 = t0 + cell;
        let half = (t1 * support + cell).max(cell);
        let centre = [direction[0] * t1, direction[1] * t1, direction[2] * t1];
        let low = [centre[0] - half, centre[1] - half, centre[2] - half];
        let high = [centre[0] + half, centre[1] + half, centre[2] + half];
        ranges.clear();
        field.for_each_cell_in(low, high, |_, range| ranges.push(range));
        let mut count = 0;
        for range in &ranges {
            for index in range.clone() {
                let star = field.star(index);
                let radius = (star.position[0] * star.position[0]
                    + star.position[1] * star.position[1]
                    + star.position[2] * star.position[2])
                    .sqrt();
                if radius < t0 || radius >= t1 || radius <= f32::EPSILON {
                    continue;
                }
                let cosine = (star.position[0] * direction[0]
                    + star.position[1] * direction[1]
                    + star.position[2] * direction[2])
                    / radius;
                if (1.0 - cosine * cosine).max(0.0).sqrt() <= support {
                    count += 1;
                }
            }
        }
        out.push(count);
        t0 = t1;
    }
    out
}

/// **沿视线查 R3 星场**：按径向一层层扫"胖射线"（横向半径 = `t · support` 的锥）覆盖的细格，
/// 收下支持域内的星，**按半径升序**交出来。
///
/// ⚠ 这一份是**判据用的朴素版**（逐层的 AABB 里逐格下探）：它慢，但**明显正确**，
///   而且与 GPU 那份**找到的星完全一样** —— 因为"收哪些星"是一条**谓词**
///   （半径落在这一段 + 角偏移在支持域内），与遍历怎么走无关。GPU 那份走的是
///   沿视线的 DDA + 横向格点（快得多），两份的**累积次序**都由"按半径升序"钉住
///   ⇒ 逐位对账才成立。
fn gather_stars(
    field: &StarField,
    direction: [f32; 3],
    enter: f32,
    exit: f32,
    support: f32,
) -> Vec<StarHit> {
    let mut hits: Vec<StarHit> = Vec::new();
    if field.count() == 0 || support <= 0.0 || exit <= enter {
        return hits;
    }
    let cell = field.grid.meta.cell;
    // 径向逐层：层的厚度 = 一个细格（**半径归属**就是按层判的 ⇒ 一颗星只会被收一次）。
    let mut t0 = (enter - cell).max(0.0);
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    while t0 <= exit {
        let t1 = t0 + cell;
        // 这一层的横向半径：层里**最远**那个半径上的支持域，再放一格（格是方的、锥是圆的）。
        let half = (t1 * support + cell).max(cell);
        let centre = [direction[0] * t1, direction[1] * t1, direction[2] * t1];
        let low = [centre[0] - half, centre[1] - half, centre[2] - half];
        let high = [centre[0] + half, centre[1] + half, centre[2] + half];
        ranges.clear();
        field.for_each_cell_in(low, high, |_, range| ranges.push(range));
        for range in &ranges {
            for index in range.clone() {
                let star = field.star(index);
                let radius = (star.position[0] * star.position[0]
                    + star.position[1] * star.position[1]
                    + star.position[2] * star.position[2])
                    .sqrt();
                if radius < t0 || radius >= t1 || radius <= f32::EPSILON {
                    continue;
                }
                // `sin θ`：`cos θ = (p · d) / |p|` ⇒ `sin² = 1 − cos²`（比 `acos` 稳且快）。
                let cosine = (star.position[0] * direction[0]
                    + star.position[1] * direction[1]
                    + star.position[2] * direction[2])
                    / radius;
                let sine = (1.0 - cosine * cosine).max(0.0).sqrt();
                if sine > support {
                    continue;
                }
                hits.push(StarHit {
                    radius,
                    sine,
                    power: [
                        star.brightness * star.tint[0],
                        star.brightness * star.tint[1],
                        star.brightness * star.tint[2],
                    ],
                });
            }
        }
        t0 = t1;
    }
    // ⚠ 升序排一次（半径并列时按 `sin`）：步进是**按半径消费**候选的，
    //   次序两侧一致，逐位对账才有意义。
    hits.sort_by(|a, b| {
        a.radius
            .partial_cmp(&b.radius)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.sine
                    .partial_cmp(&b.sine)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    hits
}

/// **一次采样的几何**：世界点 → 体网格的八个角与三个权重。
///
/// ⚠⚠ **它存在的唯一理由是性能，而代价是三个数量级**：`sample_lane` 原来把这一整条链
///   （开方 → 层号 → 方向 → `cube_face_of`）在每个**角**上走一遍 —— 八个角、而发射与消光
///   各要一次 ⇒ **每个采样点 16 遍**。实测 `--face 128` 因此烘不完（10 分钟超时）。
///   这条链与"读哪一条通道"无关 ⇒ 算一遍、两条通道各 gather 一次。
#[derive(Clone, Copy)]
struct Sample {
    /// 八个角在 `data` 里的**下标**（已含 `* 6`，等通道再加 `lane`）。
    corners: [usize; 8],
    weights: [f32; 8],
    inside: bool,
}

impl Sample {
    /// 世界点落在壳外时 `inside = false`（读出来就是 0，方向那一套不必算）。
    fn below_shell(volume: &VolumeData, point: [f32; 3]) -> Self {
        let _ = (volume, point);
        Self {
            corners: [0; 8],
            weights: [0.0; 8],
            inside: false,
        }
    }

    #[inline]
    fn gather(&self, data: &[f32], lane: usize) -> f32 {
        if !self.inside {
            return 0.0;
        }
        let mut total = 0.0_f32;
        for index in 0..8 {
            total += self.weights[index] * data[self.corners[index] + lane];
        }
        total
    }
}

#[inline]
fn snap(fraction: f32) -> f32 {
    if fraction < 1e-4 {
        0.0
    } else if fraction > 1.0 - 1e-4 {
        1.0
    } else {
        fraction
    }
}

/// 世界点 → **一次采样的几何**（见 [`Sample`] 那条性能说明）。
/// **判据用**：按世界点采一条通道的值 —— 就是 sample_at 那一份语义的公开口。
///
/// ⚠ 它的存在只为"GPU 侧要和 CPU 逐点对账"这件事：sample_at 本身是私有实现细节，
///   而 GPU 那份 WGSL 是同一套语义的**第二份实现** ⇒ 必须有一条能直接对照的口。
/// ---- 跨面混合重建：尝试过一次，**回退了**（第 72 轮）----
///
/// 目标（用户判据："真实统一的世界场肯定不会这样"）：存储保持六面立方球（刻意：面内密、
/// 径向疏），但**三线性重建按面各做一份** ⇒ 跨棱格架换朝向 ⇒ 导数跳变 ⇒ 一道通高细折痕
/// （实测：行平均后是邻列的 3.27x，参考图自身 1.29x）。
///
/// 做法（两侧同改）：把三线性抽成 `sample_at_face(face, s, t)`，再在**面棱一个半格内**
/// 找"最贴近的另一面"（用**未夹**的面投影），按 `smoothstep(band, 0, outside)` 混合
/// （棱上各半、离棱降到零）。
///
/// **失败点（下一个我照这个查）**：发射那条（带宽 0，等价于原路径）**对上了**；
/// 天空那条两侧差 **0.182 @ texel 286** ⇒ 镜像实现里有一处细节不一致。最可能的三个：
/// 1. `outside` 的算法在"点同时落在多面边界上"时的取法（我用 `max(-s, s-1, -t, t-1)`）；
/// 2. 伙伴面的 `s`/`t` 要不要夹（我**不夹**，但 `cube_direction` 收到越界值时的行为要两边一致）；
/// 3. 带宽里那个 `res`：CPU 用 `volume.res`，WGSL 用 `volume.shape.x`。
/// ⇒ 定位法：写一条**单点探针**，同一个世界点分别打印两侧的 `(face, s, t, partner, outside, weight)`，
///   一眼看出在哪一步分叉 —— 比读代码快得多。
pub fn sample_volume(volume: &VolumeData, point: [f32; 3], lane: usize) -> f32 {
    sample_at(volume, point).gather(&volume.data, lane)
}

fn sample_at(volume: &VolumeData, point: [f32; 3]) -> Sample {
    let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
    let span = volume.outer - volume.inner;
    // ⚠ 边界**闭 + 相对容差**（与 `density::sample_world` 同一个口径）：体网格的第一层与
    //   最后一层落在 `inner` / `outer` 上，而 `f32` 上那个和会偏 1e-7 ⇒ 严格比较会把
    //   壳的两壁读成 0。
    let tolerance = span.abs().max(1.0) * 1e-5;
    if radius < volume.inner - tolerance
        || radius > volume.outer + tolerance
        || span.abs() <= f32::EPSILON
    {
        return Sample::below_shell(volume, point);
    }
    let direction = [point[0] / radius, point[1] / radius, point[2] / radius];
    let (face, s, t) = cube_face_of(direction);
    let res = volume.res.max(2);
    let layers = volume.layers.max(2);
    let last_layer = layers - 1;
    let altitude =
        px_volume_schema::volume::Shell::new(volume.inner, volume.outer).altitude_of(radius);
    let sz = altitude * last_layer as f32;
    let nearest = sz.round();
    let layer0 = if (sz - nearest).abs() < 1e-3 {
        nearest
    } else {
        sz.floor()
    };
    let tz = snap(sz - layer0);
    let sx = s * res as f32 - 0.5;
    let sy = t * res as f32 - 0.5;
    let x0 = sx.floor();
    let y0 = sy.floor();
    let tx = snap(sx - x0);
    let ty = snap(sy - y0);
    let layer_at = |step: f32| (layer0 + step).clamp(0.0, last_layer as f32) as u32;
    let (la, lb) = (layer_at(0.0), layer_at(1.0));

    // ⚠⚠ **八个角要跨面取**（round 30 修 —— 那道竖缝的真凶）：`s` / `t` 走出本面时，落点是
    //   **相邻面**的格子。从前是 `wrap_cell(s)` + `clamp_cell(t)`、而 `slot` 的面号**写死**
    //   ⇒ 八个角全在本面里 ⇒ 视线跨过面棱时采样值跳一下 ⇒ 贴图上那道通高的竖缝。
    //   实测（`PX_DEBUG_SEAM`）：面 0 的 `s=0` 棱与面 4 的 `s=1` 棱**本该是棱上的同一点**
    //   （`cube_direction` 两面同值、只差一个纹素），亮度却是 0.0015 对 0.0295（20 倍）；
    //   画面正中那一对（面 1 `s=0` / 面 5 `s=1`）差 12 倍。
    //   ⚠ 这与 `density::sample_world` 是**同一类 bug 的第二份实现** —— 两处都得按方向反查。
    let corner_slot = |cell_s: f32, cell_t: f32, layer: u32| -> usize {
        let s = (cell_s + 0.5) / res as f32;
        let t = (cell_t + 0.5) / res as f32;
        let (nf, ns, nt) = cube_face_of(px_field_schema::field::cube_direction(face, s, t));
        let cs = ((ns * res as f32) as u32).min(res - 1);
        let ct = ((nt * res as f32) as u32).min(res - 1);
        ((((nf * layers + layer.min(last_layer)) * res + ct) * res + cs) * 6) as usize
    };
    let corners = [
        corner_slot(x0, y0, la),
        corner_slot(x0 + 1.0, y0, la),
        corner_slot(x0, y0 + 1.0, la),
        corner_slot(x0 + 1.0, y0 + 1.0, la),
        corner_slot(x0, y0, lb),
        corner_slot(x0 + 1.0, y0, lb),
        corner_slot(x0, y0 + 1.0, lb),
        corner_slot(x0 + 1.0, y0 + 1.0, lb),
    ];
    let (wx, wy) = (1.0 - tx, 1.0 - ty);
    let weights = [
        wx * wy * (1.0 - tz),
        tx * wy * (1.0 - tz),
        wx * ty * (1.0 - tz),
        tx * ty * (1.0 - tz),
        wx * wy * tz,
        tx * wy * tz,
        wx * ty * tz,
        tx * ty * tz,
    ];
    Sample {
        corners,
        weights,
        inside: true,
    }
}

/// **点源的辐照律**：相机（在壳心）看一颗距离 `r` 的星，像素值 ∝ `1/r²`。
///
/// ⚠⚠ 2026-09-25 用户定的：`1/r²` 是辐照度、必须成立。此前直接看见那一档**漏了它**
///   （`B·PSF` 与距离无关），而"照亮气体"那一档早就是 `B/(d²+soft²)` ——
///   **同一个 `B` 在两档里含义不同**，那才是真正的不一致。
///
/// ⚠ 增益**锚在内壁上**（`(inner/r)²`）：`star_gain` 的含义因此是"内壁上一颗星的
///   表观亮度"⇒ 近处的星亮度与从前一样、远处的按 `1/r²` 变暗（`r = outer` 处是 1/9）。
///   不锚的话整套增益要重标一遍，而那会把"哪一档变了"搅在一起。
///
/// ⚠ 这里**不含任何介质**（用户："先不管介质"）：均匀稀薄介质要么进 RTE、要么进
///   假想的星等，两样都不是这一档该干的事。气自己的消光仍然照旧（那是星云本身）。
pub fn star_falloff(radius: f32, inner: f32) -> f32 {
    let reference = inner.max(1e-4);
    let distance = radius.max(1e-4);
    (reference / distance) * (reference / distance)
}

/// 一条视线的积分（出一条通道）。
///
/// ⚠ 步长是**弦长除以步数**：弧长参数化下每步的 `ds` 相同 ⇒ 透过率可以逐步累乘，
///   不必重算前缀和。
fn march_channel(
    emission: &VolumeData,
    stars: Option<&StarField>,
    params: &SkyParams,
    channel: usize,
    direction: [f32; 3],
    texel: u32,
) -> f32 {
    let enter = emission.inner;
    let exit = emission.outer;
    let steps = params.steps.max(1);
    // ⚠⚠ **步长在参数空间里取固定**（用户 2026-09-25 的口径）：`u` 均匀 ⇒ 落到世界里是
    //   等比步长（∝ r），与角向格子 `r·Δθ` 配成各向同性。世界长度仍然要算出来
    //   （光学深度是世界的量）：第 `i` 步的世界长度 = 那一格 `[i·Δu, (i+1)·Δu]` 的
    //   `R` 之差 —— 它与抖动无关，所以期望值不变。
    let shell = px_volume_schema::volume::Shell::new(enter, exit);
    let du = 1.0 / steps as f32;
    let seed = 0x51ed_270b_u32.wrapping_add(channel as u32);
    // ⚠ 这一通道自己的消光（`1 + channel`）⇒ 蓝被吃得比红多 ⇒ 尘埃染红。
    let sigma_lane = 3 + channel.min(2);
    let lane = channel.min(2);

    // ⚠ **壳外的那一段不必采样**：相机在壳心 ⇒ 每条视线的入射半径就是 `inner`，
    //   所以从相机出发**整段都在壳内**，解析的入射/出射点没有意义。
    //   但 `inner` 是"近处留的空"：`inner > 0` 时最里面那一小段也是空的。
    //   真正的省法在别处（见 `Sample` 那条：几何只算一遍）。

    // ⚠ 星点候选**每条视线查一次**（与步长无关）：每颗星按**它自己的半径**用那一步的
    //   透过率 —— 近处的星不被整层气遮住、远处的被前面的气吃掉。
    let hits = match stars {
        Some(field) if params.star_gain > 0.0 => {
            gather_stars(field, direction, enter, exit, star_support(params))
        }
        _ => Vec::new(),
    };
    let mut next_hit = 0_usize;

    let mut transmittance = 1.0_f32;
    let mut radiance = 0.0_f32;
    for index in 0..steps {
        // ⚠ 抖动只挪**格内**的采样点（不跨格）⇒ 期望值不变，而层状条纹被打散。
        let offset = if params.jitter > 0.0 {
            jitter_at(texel, index, seed) * params.jitter
        } else {
            0.5
        };
        let distance = shell.radius_of((index as f32 + offset) * du);
        // 这一格的世界长度（`u` 均匀、世界等比 ⇒ 每步都不一样）。
        let step = shell.radius_of((index + 1) as f32 * du) - shell.radius_of(index as f32 * du);
        let point = [
            direction[0] * distance,
            direction[1] * distance,
            direction[2] * distance,
        ];
        // ⚠ 几何**算一遍**，发射与消光各 gather 一次（见 [`Sample`]）。
        let sample = sample_at(emission, point);
        // ⚠ **逐通道的发射**：lane c 是这一条通道自己的发射（见 cloud.emission 的六通道布局）。
        let emit = sample.gather(&emission.data, lane);
        let sigma = sample.gather(&emission.data, sigma_lane);
        if emit > 0.0 {
            radiance += transmittance * emit * step;
        }
        if sigma > 0.0 {
            transmittance *= (-sigma * step).exp();
        }
        // ⚠ **这一步负责的星**（半径落在这一步里）：用当前的透过率加上去 —— 于是
        //   "每步查一次星"在物理上就是"这一步之前的吸收算完了，星的光从那里过来"。
        //   ⚠ 摆在透过率**更新之后**：星在这一步里，它前面那些气也该算上。
        while next_hit < hits.len() && hits[next_hit].radius <= distance {
            let hit = &hits[next_hit];
            radiance += transmittance
                * hit.power[lane]
                * star_power(hit.sine, params)
                * params.star_gain
                * star_falloff(hit.radius, enter);
            next_hit += 1;
        }
        if transmittance < 1e-4 {
            // ⚠ 剩下的星还在后面（更远）⇒ 它们的贡献是 `T × …`，`T < 1e-4` 已经读不出来。
            //   但**必须**把候选游标走完：不然下一层的判据会看到"星少了几颗"。
            next_hit = hits.len();
            break;
        }
    }

    // 远处的星（半径超过最后一个采样点、或提前 break 掉的那一批）用最后的透过率兜底：
    // 它们的贡献是 `T_end × …`，与"星在天穹上"那一档逐字一致。
    while next_hit < hits.len() {
        let hit = &hits[next_hit];
        radiance += transmittance
            * hit.power[lane]
            * star_power(hit.sine, params)
            * params.star_gain
            * star_falloff(hit.radius, enter);
        next_hit += 1;
    }

    radiance + transmittance * params.background[lane]
}

/// **把整条天空积出来**（一条通道）。
///
/// 输出是一张 `CubeMap` 场：`face × (face × 6)`，值就是这条通道的辐射亮度。
/// ⚠ 出的是**线性**值（不钳到 `[0,1]`）：曝光是渲染期的事，烘图时钳掉就把高光砍了。
pub fn raymarch_channel(
    emission: &VolumeData,
    stars: Option<&StarField>,
    params: &SkyParams,
    channel: usize,
) -> Field {
    let face = params.face.max(1);
    let height = face * CUBE_FACES;
    // ⚠ **按行带并行**（`px_field_schema::parallel`）：逐格结果与串行逐位相同。
    //   这一档是天空烘焙里最贵的一处：六个面 × face² 格 × 每格一次 256 步的步进，
    //   而 `raymarch_sky` 还要把三条通道各走一遍。
    let data =
        px_field_schema::parallel::rows(face as usize, height as usize, |first, count, out| {
            for row in 0..count {
                let y = (first + row) as u32;
                let base = row * face as usize;
                for x in 0..face {
                    let direction = art_direction_at(Projection::CubeMap, face, height, x, y);
                    // ⚠⚠ 抖动种子**按方向取**（round 30 修）：从前是
                    //   `(y * face + x) ^ (face_index << 20)` —— 贴图坐标在**面边界上跳变**
                    //   （面号那一项整块换掉）⇒ 棱两侧的抖动模式互不相关 ⇒ 抖动打散出来的
                    //   噪声在棱上断层，画面上就是那条通高的**竖缝**（列跳变 0.0179，
                    //   而全图中位只有 0.0024、参考图全图最大 0.0115）。
                    //   方向是**连续**的（`cube_direction` 在公共棱上同值）⇒ 按它取种子，
                    //   棱两侧就落在同一套模式里；逐格仍然唯一（每格的方向都不同）。
                    let texel = direction[0].to_bits()
                        ^ direction[1].to_bits().rotate_left(11)
                        ^ direction[2].to_bits().rotate_left(22);
                    out[base + x as usize] =
                        march_channel(emission, stars, params, channel, direction, texel);
                }
            }
        });
    Field::with_projection(face, height, data, Projection::CubeMap)
}

/// 图侧最顺手的入口：**发射体积 + 星图 → 一张天空立方贴图**。
///
/// ⚠ 三条通道在**这一档内部**各积一遍：逐通道消光意味着它们本来就不同
///   （`exp(-σ_B·ds) / exp(-σ_R·ds)` 那个比值就是"尘埃染红"），一次交出一张
///   `Rgba16Float` 立方贴图才是这个算子该有的形状。
///
/// ⚠ 它**只收发射体积**（不是密度场）：搬密度、算光照是上游那两档的事，而它们与
///   "壳多细、半径多大"有关、与"积多细"无关 ⇒ 那些参数不该抄到这一档来。
///
/// ## 分级（按亮度走色相斜坡）
///
/// ⚠⚠ **全部量自参考图的区域均值**（线性空间、归一到 R=1），不是拧出来的：
///
/// | 档 | 参考图的区域 | 该区 luma | 色相 R:G:B |
/// |---|---|---|---|
/// | 暗 | 左缘红尘 | 0.016 | **1.00 : 0.24 : 0.41** |
/// | 中 | 河畔暖沙 | 0.055 | **1.00 : 0.38 : 0.45** |
/// | 亮 | **蓝气河** | 0.12 | **1.00 : 1.14 : 2.17** |
/// | 极亮 | 星核 | 0.35+ | **1.00 : 0.95 : 1.05** |
///
/// ⚠⚠ 之前用"亮 15% 的均值"当目标（1.00 : 0.81 : 1.10）——
///   **那把白色星点平均进了蓝气**，于是"蓝"被稀释成中性灰（画面一片灰紫）。
///   参考图里**星是白的、气是深青蓝的**（星核 1:0.92:1.07 对蓝河 1:1.14:2.17）——
///   两者亮度相近、颜色完全不同 ⇒ **目标必须按区域取，不能按亮度分位取**。
///   这一版的四档是按亮度**分段**走的（对数域插值），但端点来自区域。
///
/// ⚠ 亮度逐格守恒（目标先缩到这一格的 luma）⇒ 亮度那一维仍只由体渲染的参数管。
/// ⚠⚠ **档位按"区域层级"重标定**（round 26）：参考图的蓝河**内部各亮度级都是蓝的**
///   （河盒 0.03~0.25 全蓝，盒均值 0.107 只是量级标签）；暖沙在**河缘**
///   （沙盒 0.061 是空间上的边缘带，不是"亮度到 0.06 就变沙"）。
///   ⇒ 旧档位（蓝从 0.09 起）把湖体 0.05~0.07 的区域全判成暖沙/过渡
///   （亮档 G/R 只有 0.47）—— 蓝档的起点要落在**湖的区域均值**（0.058）。
///   档位（输出域 luma）：尘埃 ≤0.028 → 河缘沙 0.058 起蓝 → **蓝河** → 白星 0.35。
///
/// ⚠⚠ 尘埃/沙的边界从 0.014 收到 **0.028**（round 28）：尘埃区的区域亮度被
///   星晕/辉光垫到 0.02~0.03，落在旧的"沙档"里 ⇒ 暗像素继承暖档 ⇒
///   暗带 G/R 0.41（参考 0.24）。边界收到**尘埃区域的实测层级**（0.028）后，
///   暗像素的键落回尘埃档（红）。
pub const RAMP_LUMA: [f32; 4] = [0.028, 0.034, 0.058, 0.35];
pub const RAMP_HUE: [[f32; 3]; 4] = [
    // ⚠ 暗档的 B 从 **0.41 抬到 0.55**：左缘红尘实测 B/R 0.41、下缘 0.69，
    //   而**暗带均值要 0.61** —— 端点取两者之间（0.41 会把整条暗带压得太暖，
    //   实测 B/R 0.45 对参考 0.61）。
    [1.00, 0.25, 0.55], // 暗：红尘
    [1.00, 0.42, 0.58], // 中：暖褐（实测 (1,0.51,0.55) 与暖沙 (1,0.34,0.40) 之间）
    // ⚠ 亮档 B 抬到 2.30、且 luma 端点从 0.12 收到 0.09：
    //   亮带的 B/R 实测 0.92 对参考 1.09 —— 我的亮带像素有相当一部分
    //   落在 0.05~0.12 的**暖过渡**里，蓝档要**更早**接管。
    [1.00, 1.14, 2.30], // 亮：蓝气河
    [1.00, 0.95, 1.05], // 极亮：星核（白）
];
/// 朝目标色相走多满（`1` = 完全替换；`< 1` 保留一点原色变化）。
pub const GRADE_STRENGTH: f32 = 0.95;

/// **亮度响应**（S 形对比曲线）—— 分级里的第二件事（第一件是色相斜坡）。
///
/// ⚠⚠ 全部从**参考图的分位实测**拟合：把我当前的 p10/p50/p85/p99 映到参考的 ——
///
///   | 锚点（输入 luma → 输出 luma） | 段的 log-log 斜率 |
///   |---|---|
///   | 0.0061 → 0.0051（p10） | —— |
///   | 0.0299 → 0.0171（p50） | **0.76**（暗部几乎不动） |
///   | 0.0636 → 0.0746（p85） | **1.96**（中调大压） |
///   | 0.1357 → 0.2489（p99） | **1.59**（亮端大拉） |
///
///   ⇒ 是**S 形对比**，不是幂律：纯 gamma 1.7 能映中后三点，但 p10 被压掉 5 倍
///   （推演排除）。⚠ 锚点**跟着实测走**：输入分布变了（改密度/发射）要重拟 ——
///   像白平衡一样，是"对参考的响应"，不是普适常数。
/// ⚠ p99 之上走**软肩**（C¹ 连续、渐近 [`TONE_CEIL`]）：星核该白但**不许撞顶**
///   （参考图削顶 0.000%、线性最高 0.9868）。
/// ⚠ `tone(0) = 0`：纯黑原样（"深黑太空"靠它）。
pub const TONE_IN: [f32; 4] = [0.002672, 0.014921, 0.041914, 0.110530];
pub const TONE_OUT: [f32; 4] = [0.0051, 0.0171, 0.0746, 0.2489];
const TONE_SHOULDER: f32 = 0.72;
const TONE_CEIL: f32 = 0.95;

/// **判据用**：响应曲线（分级的前一半）的公开口；GPU 那份走同一套锚点。
pub fn tone_at(l: f32) -> f32 {
    tone(l)
}

/// 响应曲线的软肩起点与上限（GPU 侧从 uniform 读同一对值）。
pub const TONE_LIMITS: [f32; 2] = [TONE_SHOULDER, TONE_CEIL];

fn tone(l: f32) -> f32 {
    if l <= 0.0 {
        return 0.0;
    }
    // log-log 分段线性（锚点间插值；两端按最外一段的实测斜率幂外推）。
    let x = l.ln();
    let y = if l <= TONE_IN[0] {
        let slope = (TONE_OUT[1] / TONE_OUT[0]).ln() / (TONE_IN[1] / TONE_IN[0]).ln();
        (TONE_OUT[0].ln() + (x - TONE_IN[0].ln()) * slope).exp()
    } else if l >= TONE_IN[3] {
        let slope = (TONE_OUT[3] / TONE_OUT[2]).ln() / (TONE_IN[3] / TONE_IN[2]).ln();
        (TONE_OUT[3].ln() + (x - TONE_IN[3].ln()) * slope).exp()
    } else {
        let mut value = TONE_OUT[0];
        for stop in 0..3 {
            if l < TONE_IN[stop + 1] {
                let w = (x - TONE_IN[stop].ln()) / (TONE_IN[stop + 1].ln() - TONE_IN[stop].ln());
                value =
                    (TONE_OUT[stop].ln() + w * (TONE_OUT[stop + 1] / TONE_OUT[stop]).ln()).exp();
                break;
            }
        }
        value
    };
    // 软肩：指数收敛到 `TONE_CEIL`（起点处斜率恰为 1 ⇒ C¹ 连续）。
    if y <= TONE_SHOULDER {
        y
    } else {
        let span = TONE_CEIL - TONE_SHOULDER;
        TONE_SHOULDER + span * (1.0 - (-(y - TONE_SHOULDER) / span).exp())
    }
}

/// 斜坡上的目标色相（按**档位键**的亮度取段）。
/// **判据用**：色相档位插值的公开口（GPU 那份是同一套语义的第二份实现）。
pub fn ramp_hue_at(key: f32) -> [f32; 3] {
    ramp_hue(key)
}

fn ramp_hue(key: f32) -> [f32; 3] {
    if key >= RAMP_LUMA[3] {
        return RAMP_HUE[3];
    }
    for stop in 0..3 {
        let (lo, hi) = (RAMP_LUMA[stop], RAMP_LUMA[stop + 1]);
        if key < hi {
            // ⚠⚠ 段间过渡**收窄**（压到段间的 20%）：线性混合会让大量像素停在混色带上
            //   （暖沙→蓝河的中点 = **薰衣草**；盲看："蓝色是灰蓝/薰衣草"）。
            //   参考图的过渡是**空间性**的（暖沙只占河缘的窄带）⇒ 键在段间快速换档，
            //   混色只剩一条窄带。
            let raw = ((key / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0);
            let shaped = ((raw - 0.4) / 0.2).clamp(0.0, 1.0);
            let w = shaped * shaped * (3.0 - 2.0 * shaped);
            let mut target = [0.0_f32; 3];
            for channel in 0..3 {
                target[channel] = RAMP_HUE[stop][channel]
                    + (RAMP_HUE[stop + 1][channel] - RAMP_HUE[stop][channel]) * w;
            }
            return target;
        }
    }
    RAMP_HUE[3]
}

/// **分级**（一格）：色相按**这一格自己的亮度**取档，亮度逐格守恒。
///
/// ⚠⚠ 这里曾经有一层"区域亮度"（单面盒式模糊后再取档）—— 那是**实时渲染的取巧**：
///   烘焙是离线的，判据应当**直接来自采样纹理本身**，采样不够就**去提采样**
///   （`--shape` / `--layers`），而不是对成品图做邻域平均。那层模糊还有两个后患：
///   * 面内模糊在立方体面的接缝两侧各用半窗、平均的是**不同内容** ⇒ 区域值在接缝处
///     阶跃 ⇒ **接缝可见**（cam 135° 那台画面正中恰好在一条面棱上）；
///   * 它把"区域"这件事**建立在别人的亮度上**，掩盖了采样不足。
///   删掉之后，色相由每格自己的亮度决定 —— 采样够细时画面自然成区。
fn grade_pixel(rgb: [f32; 3]) -> [f32; 3] {
    let luma = |color: &[f32; 3]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
    let measured = luma(&rgb);
    // ⚠ 纯黑**原样出去**：目标色会把零格染上一点暗档色相 —— 数值虽小，
    //   但"深黑太空"就靠这些格是**正好 0**。
    if measured <= 1e-6 {
        return rgb;
    }
    let target = ramp_hue(measured);
    // ⚠ 目标先缩到**这一格的 luma** 再插值 ⇒ 输出 luma 与输入逐格相同
    //   （判据 `grading_keeps_the_luma_and_heads_for_the_band_hue` 钉着）。
    let scale = measured / luma(&target);
    let mut out = rgb;
    for channel in 0..3 {
        out[channel] += (target[channel] * scale - rgb[channel]) * GRADE_STRENGTH;
        out[channel] = out[channel].max(0.0);
    }
    out
}

pub fn raymarch_sky(
    emission: &VolumeData,
    stars: &StarField,
    sky_params: &SkyParams,
) -> Result<TextureData, String> {
    let mut planes = Vec::with_capacity(3);
    for channel in 0..3 {
        let field = raymarch_channel(emission, Some(stars), sky_params, channel);
        planes.push(field.data);
    }

    // ⚠ **逐格色相分级**（bake 期，最后一步）—— 档位键就是这一格自己的亮度。
    //   动机与常数见文件头，逐格那点事在 [`grade_pixel`]
    //   （判据 `grading_keeps_the_luma` 钉着它的亮度守恒）。
    //   ⚠⚠ 这里**不再有"区域亮度"那一层模糊**（理由见 [`grade_pixel`]）：
    //   采样不足要靠**提高采样**解决，不是拿邻域平均去盖。
    let line = |value: f32| -> f32 { value.max(0.0) };
    let texels = planes[0].len();
    let mut graded = vec![0.0_f32; texels * 3];
    for index in 0..texels {
        let rgb = [
            planes[0][index].max(0.0),
            planes[1][index].max(0.0),
            planes[2][index].max(0.0),
        ];
        // ⚠ **先亮度响应、后色相斜坡**：色相的档位常数（`RAMP_LUMA`）是**输出域**
        //   量出来的（参考图的分位）⇒ 必须对齐到响应**之后**的亮度，顺序不能反。
        let l = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        let response = if l > 1e-9 { tone(l) / l } else { 0.0 };
        let rgb = [rgb[0] * response, rgb[1] * response, rgb[2] * response];
        let out = grade_pixel(rgb);
        graded[index * 3] = line(out[0]);
        graded[index * 3 + 1] = line(out[1]);
        graded[index * 3 + 2] = line(out[2]);
    }

    // ⚠ **半精度、线性、不钳**：亮核可以超过 1，8 位会把高光砍在 1.0
    //   —— 而"亮核"正是靠超过 1 的那一段。转换走 `crate::half`（本 crate 自己的那一份：
    //   `px_graph` 的同类函数在依赖树的上面，够不到）。
    let face = sky_params.face.max(1);
    let mut bytes = Vec::with_capacity(texels * 8);
    for index in 0..texels {
        for channel in 0..3 {
            bytes.extend_from_slice(
                &crate::half::half_from_f32(graded[index * 3 + channel]).to_le_bytes(),
            );
        }
        bytes.extend_from_slice(&crate::half::half_from_f32(1.0).to_le_bytes());
    }

    // ⚠ 构造走 `TextureData::new`（唯一的构造口）：它自检"字节数 = 形状算出来的 mip 链长度"。
    //   一份字节数不对的贴图进了 CAS 之后，症状是渲染器采样越界 —— 归因极远。
    Ok(TextureData::new(
        face,
        face,
        CUBE_FACES,
        1,
        TextureFormat::Rgba16Float,
        bytes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::Field;

    /// 一份均匀的发射体积：发射 `emit`、三通道消光都是 `alpha`。
    ///
    /// ⚠ 布局与 `cloud.emission` **逐字一致**（`[发射, σ_R, σ_G, σ_B]`）：判据用的假数据
    ///   一旦与真布局不同，测的就是另一个算子。
    fn uniform(res: u32, layers: u32, emit: f32, alpha: f32) -> VolumeData {
        uniform_channels(res, layers, emit, [alpha, alpha, alpha])
    }

    /// 同上，但三通道消光可以不同（用来判"尘埃染红"）。
    fn uniform_channels(res: u32, layers: u32, emit: f32, alpha: [f32; 3]) -> VolumeData {
        // ⚠ 布局：`[发射 R, G, B, σ_R, σ_G, σ_B]`（六通道）。
        //   发射给成三条通道**相同**（中性）⇒ 色相全由消光决定，
        //   于是这些"消光越大越暗"的判据仍然只测消光那一件事。
        let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res * 6) as usize];
        for chunk in data.chunks_mut(6) {
            chunk[0] = emit;
            chunk[1] = emit;
            chunk[2] = emit;
            chunk[3] = alpha[0];
            chunk[4] = alpha[1];
            chunk[5] = alpha[2];
        }
        VolumeData {
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data,
        }
    }

    /// ⚠⚠ **接缝的判据**：跨过立方体面棱采样时，读到的必须是**连续**的值。
    ///
    /// 面 0（`+x`）的 `s = 0` 棱与面 4（`+z`）的 `s = 1` 棱是**同一条线**
    /// （`cube_direction(0, 0, t)` 与 `cube_direction(4, 1, t)` 给同一个方向）⇒
    /// 棱两侧各半个纹素处的两个点，采样值应当几乎相同。
    ///
    /// ⚠ 从前 `sample_at` 的八个角**全在本面里**（`wrap_cell` + `clamp_cell`、`slot` 的
    ///   面号写死）⇒ 棱两侧差到 **12~20 倍**，天空贴图上就是那道通高的竖缝。
    ///   这条判据钉住它（`density::sample_world` 是同类的第二份实现，两边都已修）。
    #[test]
    fn sampling_stays_continuous_across_a_face_edge() {
        let (res, layers) = (16_u32, 8_u32);
        // 逐格按**方向**填一个光滑但非平凡的值：光滑 ⇒ 本来就该连续；非平凡 ⇒ 不平庸地过。
        let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res * 6) as usize];
        let mut slot = 0_usize;
        for face in 0..CUBE_FACES {
            for layer in 0..layers {
                for t in 0..res {
                    for s in 0..res {
                        let direction = px_volume_schema::direction_of(
                            face,
                            s as f32 / (res - 1) as f32,
                            t as f32 / (res - 1) as f32,
                        );
                        let value = 0.2
                            + 0.15
                                * (7.0 * direction[0]).sin()
                                * (7.0 * direction[1]).sin()
                                * (7.0 * direction[2]).sin();
                        for lane in 0..6 {
                            data[slot + lane] = value;
                        }
                        slot += 6;
                    }
                }
            }
        }
        let volume = VolumeData {
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data,
        };
        // 棱两侧各离棱半个纹素，取同一个 `t`、同一个半径。
        let t = 0.5_f32;
        let half = 0.5 / res as f32;
        let radius = 1.5_f32;
        let point = |direction: [f32; 3]| {
            [
                direction[0] * radius,
                direction[1] * radius,
                direction[2] * radius,
            ]
        };
        let left = point(px_field_schema::field::cube_direction(0, half, t));
        let right = point(px_field_schema::field::cube_direction(4, 1.0 - half, t));
        let a = sample_at(&volume, left).gather(&volume.data, 0);
        let b = sample_at(&volume, right).gather(&volume.data, 0);
        assert!(
            (a - b).abs() < 0.05,
            "棱两侧的采样值差了 {:.4}（{a:.4} 对 {b:.4}）—— 场在棱上是断的",
            (a - b).abs()
        );
    }

    fn params() -> SkyParams {
        SkyParams {
            face: 8,
            steps: 96,
            jitter: 1.0,
            background: [0.0, 0.0, 0.0],
            ..Default::default()
        }
    }

    /// **零消光 ⇒ 亮度 = 发射 × 弦长**（有解析解，对得上才算积分是对的）。
    ///
    /// ⚠ 这是整条积分的地基：光学薄时 `L = ∫ j ds = j × (outer - inner)`。
    ///   步长、抖动、边界任何一处错了，这个数都不会对。
    #[test]
    fn an_optically_thin_shell_gives_emission_times_the_chord() {
        let volume = uniform(8, 6, 0.5, 0.0);
        let field = raymarch_channel(&volume, None, &params(), 0);
        let expected = 0.5 * (volume.outer - volume.inner);
        let mut worst = 0.0_f32;
        for value in &field.data {
            worst = worst.max((value - expected).abs());
        }
        assert!(
            worst < 1e-3,
            "光学薄时应当是 {expected}（发射 × 弦长），最大偏差 {worst}"
        );
    }

    /// **消光越强 ⇒ 越暗**（而且不会变成负的）。
    #[test]
    fn more_extinction_gives_a_dimmer_result() {
        let thin = raymarch_channel(&uniform(8, 6, 0.5, 0.0), None, &params(), 0);
        let thick = raymarch_channel(&uniform(8, 6, 0.5, 4.0), None, &params(), 0);
        let mean = |field: &Field| -> f64 {
            field.data.iter().map(|v| *v as f64).sum::<f64>() / field.data.len() as f64
        };
        let (thin_mean, thick_mean) = (mean(&thin), mean(&thick));
        assert!(
            thick_mean < thin_mean,
            "消光变大必须变暗：{thick_mean:.4} vs {thin_mean:.4}"
        );
        assert!(thick_mean > 0.0, "再浓也该有一点光透出来");
    }

    /// **空体积 ⇒ 只剩背景**。
    #[test]
    fn an_empty_volume_shows_only_the_background() {
        let volume = uniform(8, 4, 0.0, 0.0);
        let sky = SkyParams {
            background: [0.02, 0.03, 0.04],
            ..params()
        };
        let field = raymarch_channel(&volume, None, &sky, 0);
        let mut worst = 0.0_f32;
        for value in &field.data {
            worst = worst.max((value - 0.02).abs());
        }
        assert!(worst < 1e-4, "空体积应当只剩背景 0.02，最大偏差 {worst}");
    }

    /// 一片**够密**的星：方向按 Fibonacci 球均匀撒满，半径都一样（球形壳）。
    ///
    /// ⚠ 判据只关心"星的贡献怎么被气吃掉"与"轮廓是不是圆的"，所以星位要**均匀**
    ///   （否则量到的差异可能来自"这一条视线附近恰好没星"）。
    fn star_shell(count: usize, radius: f32, brightness: f32) -> StarField {
        // 方向按 Fibonacci 球均匀（壳上每球面度一样密）+ 统一稀疏格。
        let block = px_sparse::grid::CHUNK_CELLS;
        let cell = 0.25_f32;
        let half = radius + 2.0 * cell;
        let dims = (((2.0 * half) / cell).ceil() as u32).div_ceil(block) * block;
        let meta = px_sparse::GridMeta {
            cell,
            origin: [-(dims as f32) * cell * 0.5; 3],
            dims: [dims, dims, dims],
        };
        let golden = 2.399_963_2_f32;
        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(count);
        for index in 0..count {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = golden * index as f32;
            positions.push([
                ring * phi.cos() * radius,
                ring * phi.sin() * radius,
                z * radius,
            ]);
        }
        let values = vec![brightness; count];
        let tints = vec![[1.0, 1.0, 1.0]; count];
        StarField::build(meta, &positions, &values, &tints).expect("造星壳")
    }

    /// **星的亮度乘透射率**：前面挡一层浓气，星就暗下去。
    #[test]
    fn a_star_behind_extinction_is_dimmer() {
        let stars = star_shell(512, 2.0, 1.0);
        let clear = raymarch_channel(&uniform(8, 4, 0.0, 0.0), Some(&stars), &params(), 0);
        let dusty = raymarch_channel(&uniform(8, 4, 0.0, 3.0), Some(&stars), &params(), 0);
        let mean = |field: &Field| -> f64 {
            field.data.iter().map(|v| *v as f64).sum::<f64>() / field.data.len() as f64
        };
        assert!(
            mean(&dusty) < mean(&clear) * 0.5,
            "尘埃必须把星压暗：{} vs {}",
            mean(&dusty),
            mean(&clear)
        );
    }

    /// ⚠⚠ **这一档换掉旧星图的全部理由**：星的密度与强度必须**与位置无关**。
    ///
    /// 旧版把星画进立方图，于是同一片天在**面心**与**面角**上表现不同，实测：
    ///   * 位置分布（立方格投到球面）：面心 11.2e3 星/球面度 → 面角 15.9e3（1.43x）；
    ///   * 成品天空的亮面积占球面度之比：面心 2.75% → 面角 4.33%（**1.58x**）。
    ///   用户的原话是"星星在 cubemap 棱附近好像被压缩了"。
    ///
    /// 现在星是 R3 里的点、轮廓是世界空间的角函数、每条视线按**方向**解析求值
    /// ⇒ 每球面度看到的星数必须一致。这条判据直接量那 1.58x 有没有回来：
    /// 在真空里烘一张天空，按**半径分箱**数亮面积，面角那一箱不许比面心高一截。
    #[test]
    fn the_star_light_is_uniform_across_a_face() {
        let params = SkyParams {
            face: 128,
            steps: 4,
            jitter: 0.0,
            star_gain: 1.0,
            star_halo_gain: 0.0,
            ..Default::default()
        };
        let vacuum = uniform(8, 4, 0.0, 0.0);
        // 方向均匀撒满（Fibonacci 球）⇒ 每球面度的星数一样。
        let stars = star_shell(20000, 2.0, 8.0);
        let sky = raymarch_channel(&vacuum, Some(&stars), &params, 0);
        let face = params.face;
        // 分箱：格心到面心（a=b=0）的距离，取三个环带。
        let mut lit = [0.0_f64; 3];
        let mut area = [0.0_f64; 3];
        let mut all = 0.0_f64;
        for face_index in 0..CUBE_FACES {
            for y in 0..face {
                for x in 0..face {
                    let s = (x as f32 + 0.5) / face as f32;
                    let t = (y as f32 + 0.5) / face as f32;
                    let a = s * 2.0 - 1.0;
                    let b = t * 2.0 - 1.0;
                    let r = (a * a + b * b).sqrt();
                    let band = if r < 0.5 {
                        0
                    } else if r < 0.9 {
                        1
                    } else {
                        2
                    };
                    let value = sky.at(x, face_index * face + y) as f64;
                    area[band] += 1.0;
                    if value > 0.05 {
                        lit[band] += 1.0;
                    }
                    all += value;
                }
            }
        }
        assert!(all > 0.0, "一颗星都没画出来");
        let centre = lit[0] / area[0];
        let corner = lit[2] / area[2];
        assert!(
            centre > 0.0 && corner > 0.0,
            "有的环带一颗星都没有（面心 {centre:.5} / 面角 {corner:.5}）"
        );
        let ratio = corner / centre;
        assert!(
            (0.9..1.1).contains(&ratio),
            "面角/面心的星点密度比是 {ratio:.3}（旧版实测 1.58）—— 星又被位置影响了"
        );
    }

    /// **确定性**：同样的参数烘两遍逐字节相同（缓存键靠它）。
    #[test]
    fn the_same_parameters_give_the_same_sky() {
        let volume = uniform(8, 5, 0.4, 1.1);
        let one = raymarch_channel(&volume, None, &params(), 0);
        let two = raymarch_channel(&volume, None, &params(), 0);
        assert_eq!(one.data, two.data);
    }

    /// **逐通道消光真的分得开**：蓝吃得比红多 ⇒ 同一份体积积出来的蓝通道必须比红通道暗。
    ///
    /// ⚠ 这条钉的是"消光通道写在哪个 lane"。第一版把三通道消光取平均塞进一个 A，
    ///   于是三条通道**逐字相同**（实测最大值只差第四位）—— 图像是灰的，
    ///   而"尘埃染红"这件事在画面上完全没发生。
    #[test]
    fn per_channel_extinction_makes_blue_darker_than_red() {
        // 红光几乎不吃、蓝光吃得多（与 `EmissionParams::default` 同一个方向）。
        let volume = uniform_channels(8, 5, 0.5, [0.2, 1.2, 2.6]);
        let red = raymarch_channel(&volume, None, &params(), 0);
        let green = raymarch_channel(&volume, None, &params(), 1);
        let blue = raymarch_channel(&volume, None, &params(), 2);
        let mean = |field: &Field| -> f64 {
            field.data.iter().map(|v| *v as f64).sum::<f64>() / field.data.len() as f64
        };
        let (r, g, b) = (mean(&red), mean(&green), mean(&blue));
        assert!(
            r > g && g > b,
            "消光越大该越暗，实际 R {r:.4} / G {g:.4} / B {b:.4}（分不开说明读错了通道）"
        );
    }

    /// **消光与发射读的是同一个位置**：给一份"发射只在半边、消光只在另半边"的体积，
    /// 两者的空间分布必须各自正确（这一条钉的是 `sample_lane` 的那一份实现）。
    #[test]
    fn emission_and_extinction_are_sampled_at_the_same_place() {
        let res = 8;
        let layers = 6;
        let mut volume = uniform(res, layers, 0.0, 0.0);
        // 按半径把体积分成内外两半：内半有发射，外半有消光（三通道都给）。
        for face in 0..CUBE_FACES {
            for layer in 0..layers {
                for t in 0..res {
                    for s in 0..res {
                        let slot = (((face * layers + layer) * res + t) * res + s) as usize;
                        // ⚠ 布局是**六通道** `[发射 R, G, B, σ_R, σ_G, σ_B]`。
                        //
                        // ⚠⚠ 这几行曾是 `slot * 4`（四通道时代的步长）—— 六通道改造时
                        //   没跟上（改的是"四行连写"的另一处，这处分写在 if/else 里、没匹配上）。
                        //   于是写出去的 0.3 **散布到别的格与别的通道上**，而判据的断言很宽
                        //   （结果落在 `(0, 0.3 × 弦长)` 之间）⇒ **照样通过**。
                        //   ⇒ "布局变了而判据没变"最坏的样子：测的是别的东西、且显示绿色。
                        if layer < layers / 2 {
                            volume.data[slot * 6] = 0.3;
                        } else {
                            volume.data[slot * 6 + 3] = 0.3;
                            volume.data[slot * 6 + 4] = 0.3;
                            volume.data[slot * 6 + 5] = 0.3;
                        }
                    }
                }
            }
        }
        let field = raymarch_channel(&volume, None, &params(), 0);
        // 有发射又有消光 ⇒ 结果必须落在 (0, 0.3 × 弦长) 之间（被外层吃掉一部分）。
        let chord = volume.outer - volume.inner;
        let mut worst_high = 0.0_f32;
        let mut saw_dimmer = false;
        for value in &field.data {
            worst_high = worst_high.max(*value - 0.3 * chord);
            if *value < 0.3 * chord * 0.99 {
                saw_dimmer = true;
            }
        }
        assert!(worst_high < 1e-3, "不该超过光学薄上界，超出 {worst_high}");
        assert!(saw_dimmer, "外层有消光 ⇒ 必须有一部分被吃掉");
    }

    /// **分级：亮度守恒、逐格色相分档、纯黑原样**。
    ///
    /// ⚠ 亮度守恒是那条硬性质（色相与亮度互不干扰 ⇒ 亮度那一维仍只由体渲染的参数管）。
    ///   浮点求和顺序变了 ⇒ 不是逐位恒等，用紧容差。
    #[test]
    fn grading_keeps_the_luma_and_heads_for_the_band_hue() {
        let luma = |color: &[f32; 3]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        // ⚠ 纯黑必须**正好**是纯黑（判据用逐位：这里恒等成立）。
        assert_eq!(grade_pixel([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        for rgb in [
            [0.004, 0.002, 0.003],
            [0.01, 0.01, 0.01],
            [0.05, 0.03, 0.06],
            [0.2, 0.16, 0.22],
            [1.5, 0.9, 1.2],
        ] {
            let out = grade_pixel(rgb);
            let drift = (luma(&out) - luma(&rgb)).abs();
            assert!(
                drift <= 1e-6 + luma(&rgb) * 1e-4,
                "{rgb:?} 分级之后亮度漂了 {drift}"
            );
        }
        // 暗的往暗档色相走（目标 G/R = 0.24、B/R = 0.61；起点是中性的 1/1）。
        let dark = grade_pixel([0.006, 0.006, 0.006]);
        assert!(
            dark[1] / dark[0] < 0.4,
            "暗档的绿没压下去（{:.3}）",
            dark[1] / dark[0]
        );
        assert!(
            dark[2] / dark[0] < 0.75,
            "暗档的蓝没压过绿（{:.3}）",
            dark[2] / dark[0]
        );
        // 亮的往亮档色相走（目标 G/R = 0.81、B/R = 1.10）。
        let bright = grade_pixel([0.2, 0.2, 0.2]);
        assert!(
            bright[2] / bright[0] > 1.0,
            "亮档的蓝没高过红（{:.3}）",
            bright[2] / bright[0]
        );
        assert!(
            bright[1] / bright[0] > 0.7,
            "亮档的绿没抬起来（{:.3}）",
            bright[1] / bright[0]
        );
        // ⚠ **键就是这一格自己的亮度**（区域那一层已删，见 `grade_pixel` 的说明）：
        //   两格**等亮度**时档位相同 ⇒ 目标色同向（差别只来自那 5% 的原色残留）。
        //   ⚠ 第二格要按亮度**配平**（`0.0796/0.0318/0.1432` 的亮度也是 0.05）——
        //   随手取一组通道值并不等亮度，那测的就是"键漂了"，不是这条性质。
        let a = grade_pixel([0.05, 0.05, 0.05]);
        let b = grade_pixel([0.0796, 0.0318, 0.1432]);
        assert!(
            (luma(&a) - luma(&b)).abs() < 1e-4,
            "配平写错了：两格亮度不同（{:.5} vs {:.5}）",
            luma(&a),
            luma(&b)
        );
        assert!(
            (a[2] / a[0] - b[2] / b[0]).abs() < 0.1,
            "等亮度的两格档位不一致（{:.2} vs {:.2}）—— 键漂了",
            a[2] / a[0],
            b[2] / b[0]
        );

        // ⚠ **亮度响应**（`tone`）的三条硬性质：单调、纯黑原样、**永不撞顶**
        //   （削顶必须是 0 —— 参考图削顶 0.000%，白核靠软肩不靠钳位）。
        assert_eq!(tone(0.0), 0.0);
        let mut previous = 0.0_f32;
        for step in 0..200_usize {
            let l = step as f32 * 0.01;
            let y = tone(l);
            assert!(y >= previous, "tone 不单调（{l} 处 {y} < {previous}）");
            // ⚠ `<=` 而不是 `<`：f32 里软肩会**等于**渐近值（`e^(−84)` 直接下溢为 0），
            //   而 0.95 线性 = sRGB 250 —— 本就不算削顶，判据要的是"不许**超过**"。
            assert!(y <= TONE_CEIL, "tone 撞顶（{l} 处 {y}）");
            previous = y;
        }
    }
}
