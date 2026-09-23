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

/// 星点的亮度（给一个方向，回它压到多少）。星图是 `CubeMap` 场。
fn star_level(stars: &Field, direction: [f32; 3], params: &SkyParams) -> f32 {
    if stars.projection != Projection::CubeMap {
        return 0.0;
    }
    let face_size = stars.width.max(1);
    let (face, s, t) = cube_face_of(direction);
    let x = ((s * face_size as f32) as u32).min(face_size - 1);
    let y = (face * face_size + (t * face_size as f32) as u32).min(stars.height - 1);
    let raw = stars.at(x, y);
    if raw <= params.star_floor {
        return 0.0;
    }
    (raw - params.star_floor) / (1.0 - params.star_floor).max(1e-4)
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
    let altitude = ((radius - volume.inner) / span).clamp(0.0, 1.0);
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
    let clamp_cell = |value: f32| value.clamp(0.0, (res - 1) as f32) as u32;
    let wrap_cell = |value: f32| (value.rem_euclid(res as f32)) as u32;
    let (xa, xb) = (wrap_cell(x0), wrap_cell(x0 + 1.0));
    let (ya, yb) = (clamp_cell(y0), clamp_cell(y0 + 1.0));
    let layer_at = |step: f32| (layer0 + step).clamp(0.0, last_layer as f32) as u32;
    let (la, lb) = (layer_at(0.0), layer_at(1.0));

    let slot = |cell_s: u32, cell_t: u32, layer: u32| -> usize {
        ((((face * layers + layer) * res + cell_t) * res + cell_s) * 6) as usize
    };
    let corners = [
        slot(xa, ya, la),
        slot(xb, ya, la),
        slot(xa, yb, la),
        slot(xb, yb, la),
        slot(xa, ya, lb),
        slot(xb, ya, lb),
        slot(xa, yb, lb),
        slot(xb, yb, lb),
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

/// 一条视线的积分（出一条通道）。
///
/// ⚠ 步长是**弦长除以步数**：弧长参数化下每步的 `ds` 相同 ⇒ 透过率可以逐步累乘，
///   不必重算前缀和。
fn march_channel(
    emission: &VolumeData,
    stars: Option<&Field>,
    params: &SkyParams,
    channel: usize,
    direction: [f32; 3],
    texel: u32,
) -> f32 {
    let enter = emission.inner;
    let exit = emission.outer;
    let steps = params.steps.max(1);
    let step = (exit - enter) / steps as f32;
    let seed = 0x51ed_270b_u32.wrapping_add(channel as u32);
    // ⚠ 这一通道自己的消光（`1 + channel`）⇒ 蓝被吃得比红多 ⇒ 尘埃染红。
    let sigma_lane = 3 + channel.min(2);

    // ⚠ **壳外的那一段不必采样**：相机在壳心 ⇒ 每条视线的入射半径就是 `inner`，
    //   所以从相机出发**整段都在壳内**，解析的入射/出射点没有意义。
    //   但 `inner` 是"近处留的空"：`inner > 0` 时最里面那一小段也是空的。
    //   真正的省法在别处（见 `Sample` 那条：几何只算一遍）。

    let mut transmittance = 1.0_f32;
    let mut radiance = 0.0_f32;
    for index in 0..steps {
        // ⚠ 抖动只挪**格内**的采样点（不跨格）⇒ 期望值不变，而层状条纹被打散。
        let offset = if params.jitter > 0.0 {
            jitter_at(texel, index, seed) * params.jitter
        } else {
            0.5
        };
        let distance = enter + (index as f32 + offset) * step;
        let point = [
            direction[0] * distance,
            direction[1] * distance,
            direction[2] * distance,
        ];
        // ⚠ 几何**算一遍**，发射与消光各 gather 一次（见 [`Sample`]）。
        let sample = sample_at(emission, point);
        // ⚠ **逐通道的发射**：lane c 是这一条通道自己的发射（见 cloud.emission 的六通道布局）。
        let emit = sample.gather(&emission.data, channel.min(2));
        let sigma = sample.gather(&emission.data, sigma_lane);
        if emit > 0.0 {
            radiance += transmittance * emit * step;
        }
        if sigma > 0.0 {
            transmittance *= (-sigma * step).exp();
            if transmittance < 1e-4 {
                break;
            }
        }
    }

    // 星点与背景：**乘透射率** ⇒ 被前面的气遮住、被尘埃染红。
    let mut result = radiance;
    if let Some(stars) = stars {
        result += transmittance * star_level(stars, direction, params) * params.star_gain;
    }
    result + transmittance * params.background[channel.min(2)]
}

/// **把整条天空积出来**（一条通道）。
///
/// 输出是一张 `CubeMap` 场：`face × (face × 6)`，值就是这条通道的辐射亮度。
/// ⚠ 出的是**线性**值（不钳到 `[0,1]`）：曝光是渲染期的事，烘图时钳掉就把高光砍了。
pub fn raymarch_channel(
    emission: &VolumeData,
    stars: Option<&Field>,
    params: &SkyParams,
    channel: usize,
) -> Field {
    let face = params.face.max(1);
    let mut field = Field::filled_with(face, face * CUBE_FACES, 0.0, Projection::CubeMap);
    for y in 0..field.height {
        let face_index = (y / face).min(CUBE_FACES - 1);
        for x in 0..field.width {
            let direction = art_direction_at(Projection::CubeMap, face, face * CUBE_FACES, x, y);
            // ⚠ 逐格唯一：`y * face + x` **必须**带上 x（行号 y 一样的格会拿到同一个抖动
            //   ⇒ 横向条纹）。`face_index << 20` 只是把六个面分开。
            let texel = (y * face + x) ^ (face_index << 20);
            let value = march_channel(emission, stars, params, channel, direction, texel);
            field.set(x, y, value);
        }
    }
    field
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
pub const TONE_IN: [f32; 4] = [0.0070, 0.0344, 0.0731, 0.156];
pub const TONE_OUT: [f32; 4] = [0.0051, 0.0171, 0.0746, 0.2489];
const TONE_SHOULDER: f32 = 0.72;
const TONE_CEIL: f32 = 0.95;

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

/// **区域亮度**（低频）：单面内两趟盒式模糊（行 + 列，前缀和）。
///
/// ⚠⚠ 治的是**粉彩糊**（第三版分级的全部要点）：
///   档位若按**逐像素**亮度取，每朵云内部从亮到暗就走完红→沙→蓝全程
///   ⇒ 每朵云都是红蓝渐变 ⇒ 整图粉彩。参考图的色相是**区域性**的 ——
///   整条气河是饱和蓝青、整片尘埃是深绯红（HII 区 vs 尘埃带是**空间**分的，
///   不是明暗分的）。⇒ 档位按**区域亮度**取：同区域共享色相家族，跨区域才换色。
///
/// ⚠ 半径取 `face/8`（1024 面上 128 纹素）——**"区域"必须是河/尘埃的分界尺度**：
///   取 `face/32`（32 纹素）实测**只平均到云内** ⇒ 蓝河的亮部被拖到区域均值的暖档
///   （亮档 G/R 掉到 0.61）。再小退化成逐像素（粉彩糊），再大则两类区域和成一锅。
///   嵌在蓝河里的**暗尘球**与星由 `grade_pixel` 的**两头逃逸**兜住，不靠半径。
/// ⚠ 模糊**限制在单面内**（接缝不做立方体邻接的跨界平均）：跨界要按面邻接走、
///   代价大，而接缝处只影响模糊半径宽的边缘（1024 面上 12%）。
/// ⚠ 前缀和按 f64、固定顺序累加 ⇒ 逐位确定（键要能命中缓存）。
fn region_luma(luma: &[f32], face: usize) -> Vec<f32> {
    let radius = (face / 8).max(4);
    let mut out = vec![0.0_f32; luma.len()];
    let mut rows = vec![0.0_f32; face * face];
    let mut prefix = vec![0.0_f64; face + 1];
    for f in 0..6_usize {
        let base = f * face * face;
        // 第一趟：沿行（t 固定，s 走）。
        for t in 0..face {
            let row_in = &luma[base + t * face..base + (t + 1) * face];
            prefix[0] = 0.0;
            for s in 0..face {
                prefix[s + 1] = prefix[s] + row_in[s] as f64;
            }
            let row_out = &mut rows[t * face..(t + 1) * face];
            for s in 0..face {
                let lo = s.saturating_sub(radius);
                let hi = (s + radius + 1).min(face);
                row_out[s] = ((prefix[hi] - prefix[lo]) / (hi - lo) as f64) as f32;
            }
        }
        // 第二趟：沿列（s 固定，t 走）。
        for s in 0..face {
            prefix[0] = 0.0;
            for t in 0..face {
                prefix[t + 1] = prefix[t] + rows[t * face + s] as f64;
            }
            for t in 0..face {
                let lo = t.saturating_sub(radius);
                let hi = (t + radius + 1).min(face);
                out[base + t * face + s] = ((prefix[hi] - prefix[lo]) / (hi - lo) as f64) as f32;
            }
        }
    }
    out
}

/// **分级**（一格）：色相按**区域亮度**取档（`region`），亮度逐格守恒。
fn grade_pixel(rgb: [f32; 3], region: f32) -> [f32; 3] {
    let luma = |color: &[f32; 3]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
    let measured = luma(&rgb);
    // ⚠ 纯黑**原样出去**：目标色会把零格染上一点暗档色相 —— 数值虽小，
    //   但"深黑太空"就靠这些格是**正好 0**。
    if measured <= 1e-6 {
        return rgb;
    }
    // ⚠ **两头逃逸**（否则区域档会伤到两类东西）：
    //   * 上逃：**星核/亮点**远亮于所在区域 ⇒ 按自身亮度取档
    //   （否则暗区里的星会变红 —— 参考图的星是白的）；
    //   * 下逃：**暗尘带/空洞**远暗于所在区域 ⇒ 也按自身
    //   （否则嵌在蓝河里的暗尘球会被区域染蓝 —— 参考图的暗尘球是暗红的）。
    //   中间的普通云纹（0.5× ~ 2× 区域）⇒ `t = 0` ⇒ 同区域同族。
    // ⚠⚠ 档位键 = **区域亮度**，孤立亮点的越档**封顶 20%**（`min(自身, 区域×1.2)`）：
    //   * **星晕**（ratio ~3 的孤立亮点）只许越档 20% ⇒ 在尘埃里仍是粉红
    //     （round 24 的 `max` 无封顶 ⇒ 星晕进亮档 ⇒ 薰衣草、暗档 G/R 0.41）；
    //   * **湖心**（ratio ~1.3 的区域成员）本来就贴着区域 ⇒ 封顶不影响 ⇒ 蓝；
    //   * **星核**（ratio 数十倍）与**暗尘带**（ratio < 0.2）两头逃逸按自身取；
    //   * 纯区域键（无封顶、无逃逸）也试过：区域均值把湖拖暖（亮档 G/R 0.48）——
    //     封顶 20% 是"区域管族、亮部有限越档"的正解。
    let ratio = measured / region.max(1e-6);
    let smooth = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let up = smooth((ratio - 8.0) / 12.0);
    let down = smooth((0.2 - ratio) / 0.15);
    let t = up.max(down);
    let base = region.max(measured.min(region * 1.2));
    let key = base.powf(1.0 - t) * measured.powf(t);
    let target = ramp_hue(key);
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
    stars: &Field,
    sky_params: &SkyParams,
) -> Result<TextureData, String> {
    let mut planes = Vec::with_capacity(3);
    for channel in 0..3 {
        let field = raymarch_channel(emission, Some(stars), sky_params, channel);
        planes.push(field.data);
    }

    // ⚠ **按区域分档的色相分级**（bake 期，最后一步）—— 动机与常数见文件头，
    //   逐格的那点事在 [`grade_pixel`]（判据 `grading_keeps_the_luma` 钉着它的亮度守恒）。
    //
    //   ⚠⚠ 分级走到第三版才对：一版按通道补丁（各推一把、两档都差）、
    //   二版按**逐像素亮度**分档（两档对了、但整图粉彩糊 —— 每朵云内部都在走色相斜坡）、
    //   三版按**区域亮度**分档（这一版）：同区域同族、跨区域换色。
    let line = |value: f32| -> f32 { value.max(0.0) };
    let texels = planes[0].len();
    let face_size = sky_params.face.max(1) as usize;
    // 区域亮度 = 色相的档位键（见 [`region_luma`] —— 治粉彩糊的那一步）。
    let mut luma_of = vec![0.0_f32; texels];
    for index in 0..texels {
        luma_of[index] = planes[0][index].max(0.0) * 0.2126
            + planes[1][index].max(0.0) * 0.7152
            + planes[2][index].max(0.0) * 0.0722;
    }
    let region = region_luma(&luma_of, face_size);
    let mut graded = vec![0.0_f32; texels * 3];
    for index in 0..texels {
        let rgb = [
            planes[0][index].max(0.0),
            planes[1][index].max(0.0),
            planes[2][index].max(0.0),
        ];
        // ⚠ **先亮度响应、后色相斜坡**：色相的档位常数（`RAMP_LUMA`）是**输出域**
        //   量出来的（参考图的分位）⇒ 必须对齐到响应**之后**的亮度，顺序不能反。
        let l = luma_of[index];
        let response = if l > 1e-9 { tone(l) / l } else { 0.0 };
        let rgb = [rgb[0] * response, rgb[1] * response, rgb[2] * response];
        let out = grade_pixel(rgb, tone(region[index].max(0.0)));
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

    /// **星的亮度乘透射率**：前面挡一层浓气，星就暗下去。
    #[test]
    fn a_star_behind_extinction_is_dimmer() {
        let stars = Field::filled_with(8, 8 * CUBE_FACES, 1.0, Projection::CubeMap);
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

    /// **分级：亮度守恒、同区域同族、星与暗带两头逃逸、纯黑原样**。
    ///
    /// ⚠ 亮度守恒是那条硬性质（色相与亮度互不干扰 ⇒ 亮度那一维仍只由体渲染的参数管）。
    ///   浮点求和顺序变了 ⇒ 不是逐位恒等，用紧容差。
    #[test]
    fn grading_keeps_the_luma_and_heads_for_the_band_hue() {
        let luma = |color: &[f32; 3]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        // ⚠ 纯黑必须**正好**是纯黑（判据用逐位：这里恒等成立）。
        assert_eq!(grade_pixel([0.0, 0.0, 0.0], 0.01), [0.0, 0.0, 0.0]);
        for rgb in [
            [0.004, 0.002, 0.003],
            [0.01, 0.01, 0.01],
            [0.05, 0.03, 0.06],
            [0.2, 0.16, 0.22],
            [1.5, 0.9, 1.2],
        ] {
            // 均匀区域（`region` = 自身）⇒ 走的就是"逐像素档"那条老路径。
            let out = grade_pixel(rgb, luma(&rgb));
            let drift = (luma(&out) - luma(&rgb)).abs();
            assert!(
                drift <= 1e-6 + luma(&rgb) * 1e-4,
                "{rgb:?} 分级之后亮度漂了 {drift}"
            );
        }
        // 暗的往暗档色相走（目标 G/R = 0.24、B/R = 0.61；起点是中性的 1/1）。
        let dark = grade_pixel([0.006, 0.006, 0.006], 0.006);
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
        let bright = grade_pixel([0.2, 0.2, 0.2], 0.2);
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

        // ⚠⚠ **区域是唯一的档位键**（星核/暗尘带两头逃逸除外）：
        //   * **星晕**（亮于区域的孤立亮点）跟**区域**走 ⇒ 在尘埃里是粉红
        //     （round 24 让它逃进亮档 ⇒ 薰衣草、暗档 G/R 抬到 0.41）；
        //   * **湖缘**（暗于湖心的区域成员）也跟区域走 ⇒ 湖内同族不掉档。
        let region = 0.02;
        let dim_in = grade_pixel([0.01, 0.01, 0.01], region);
        assert!(
            dim_in[1] / dim_in[0] < 0.5,
            "暗部该跟区域档走（G/R {:.2}）",
            dim_in[1] / dim_in[0]
        );
        let halo = grade_pixel([0.08, 0.08, 0.08], region);
        assert!(
            halo[1] / halo[0] < 0.5,
            "星晕逃进了亮档（G/R {:.2}）—— 薰衣草回来了",
            halo[1] / halo[0]
        );
        let lake_edge = grade_pixel([0.05, 0.05, 0.05], 0.15);
        assert!(
            lake_edge[2] / lake_edge[0] > 1.2,
            "湖缘掉了档（B/R {:.2}）—— 湖内要同族",
            lake_edge[2] / lake_edge[0]
        );
        // 星要**逃出**区域档：远亮于区域的格子按自身取 ⇒ 白（参考图的星是白的）。
        let star = grade_pixel([0.5, 0.5, 0.5], region);
        assert!(
            star[2] / star[0] > 0.95,
            "星没逃出区域档（B/R {:.2}）",
            star[2] / star[0]
        );
        // 暗尘带也要**逃出**：远暗于区域的格子按自身取 ⇒ 暗红
        // （否则嵌在蓝河里的暗尘球会被区域染蓝 —— 参考图的暗尘球是暗红的）。
        let lane = grade_pixel([0.003, 0.003, 0.003], 0.08);
        assert!(
            lane[1] / lane[0] < 0.45,
            "暗尘带没逃出区域档（G/R {:.2}）",
            lane[1] / lane[0]
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
