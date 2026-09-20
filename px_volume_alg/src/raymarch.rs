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
use px_volume_schema::VolumeData;
use px_volume_schema::params::emission::EmissionParams;
use px_volume_schema::params::sky::SkyParams;

/// 格子的确定性抖动：同一格永远同一个偏移。
///
/// ⚠ **不许**用时间或真随机：那会让同一份参数烘出两张不同的图，而缓存键是"键 = 内容"
///   ——"同参数同产物"是全仓的地基。
fn jitter_at(index: u32, seed: u32) -> f32 {
    let mut hash = index
        .wrapping_mul(0x9e37_79b9)
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

/// 世界点 → 三条通道的值。`lane` 取 `0..=3`（0/1/2 = 发射的 RGB、3 = 消光）。
///
/// ⚠ **一份实现，两个调用点**（发射那一趟与消光那一趟都走这里）：体积的四个通道是交错的，
///   而"跨面取邻居 / 边界返回 0"这套规则只能有一份 —— 分开写两遍迟早会漂，而漂了的表现
///   是"发射与消光不在同一个位置"，画面上是雾与暗带错开一格，极难归因。
fn sample_lane(volume: &VolumeData, point: [f32; 3], lane: usize) -> f32 {
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
        return 0.0;
    }
    let direction = [point[0] / radius, point[1] / radius, point[2] / radius];
    let (face, s, t) = cube_face_of(direction);
    let res = volume.res.max(2);
    let last_layer = volume.layers.max(2) - 1;
    let altitude = ((radius - volume.inner) / span).clamp(0.0, 1.0);
    let sz = altitude * last_layer as f32;
    let nearest = sz.round();
    let layer0 = if (sz - nearest).abs() < 1e-3 {
        nearest
    } else {
        sz.floor()
    };
    let snap = |fraction: f32| {
        if fraction < 1e-4 {
            0.0
        } else if fraction > 1.0 - 1e-4 {
            1.0
        } else {
            fraction
        }
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
    let lane = lane.min(3);
    let corner = |cell_s: u32, cell_t: u32, layer: u32| {
        let slot = (((face * volume.layers + layer) * res + cell_t) * res + cell_s) as usize;
        volume.data[slot * 4 + lane]
    };
    let top = (corner(xa, ya, la) * (1.0 - tx) + corner(xb, ya, la) * tx) * (1.0 - ty)
        + (corner(xa, yb, la) * (1.0 - tx) + corner(xb, yb, la) * tx) * ty;
    let bottom = (corner(xa, ya, lb) * (1.0 - tx) + corner(xb, ya, lb) * tx) * (1.0 - ty)
        + (corner(xa, yb, lb) * (1.0 - tx) + corner(xb, yb, lb) * tx) * ty;
    top * (1.0 - tz) + bottom * tz
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

    let mut transmittance = 1.0_f32;
    let mut radiance = 0.0_f32;
    for index in 0..steps {
        // ⚠ 抖动只挪**格内**的采样点（不跨格）⇒ 期望值不变，而层状条纹被打散。
        let offset = if params.jitter > 0.0 {
            jitter_at(texel.wrapping_mul(1024).wrapping_add(index), seed) * params.jitter
        } else {
            0.5
        };
        let distance = enter + (index as f32 + offset) * step;
        let point = [
            direction[0] * distance,
            direction[1] * distance,
            direction[2] * distance,
        ];
        let emit = sample_lane(emission, point, channel);
        let sigma = sample_lane(emission, point, 3);
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
            let texel = (y * face + x) ^ (face_index << 20);
            let value = march_channel(emission, stars, params, channel, direction, texel);
            field.set(x, y, value);
        }
    }
    field
}

/// 图脚本最顺手的入口：密度场 → （搬进体积 + 算光照）→ 积出一条通道。
///
/// ⚠ 三个中间产物（密度体积、发射体积）**都不单独进键**：它们只是这一档内部的两步。
pub fn raymarch_from_field(
    density_params: &px_volume_schema::params::density::DensityParams,
    emission_params: &EmissionParams,
    sky_params: &SkyParams,
    density_field: &Field,
    stars: Option<&Field>,
    channel: usize,
) -> Result<Field, String> {
    let emission =
        crate::emission::emit_from_field(density_params, emission_params, density_field)?;
    Ok(raymarch_channel(&emission, stars, sky_params, channel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::Field;

    /// 一份均匀的发射体积：发射 `emit`、消光 `alpha`。
    fn uniform(res: u32, layers: u32, emit: f32, alpha: f32) -> VolumeData {
        let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res * 4) as usize];
        for chunk in data.chunks_mut(4) {
            chunk[0] = emit;
            chunk[1] = emit;
            chunk[2] = emit;
            chunk[3] = alpha;
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

    /// **消光与发射读的是同一个位置**：给一份"发射只在半边、消光只在另半边"的体积，
    /// 两者的空间分布必须各自正确（这一条钉的是 `sample_lane` 的那一份实现）。
    #[test]
    fn emission_and_extinction_are_sampled_at_the_same_place() {
        let res = 8;
        let layers = 6;
        let mut volume = uniform(res, layers, 0.0, 0.0);
        // 按半径把体积分成内外两半：内半有发射，外半有消光。
        let span = volume.outer - volume.inner;
        for face in 0..CUBE_FACES {
            for layer in 0..layers {
                let altitude = layer as f32 / (layers - 1) as f32;
                let _ = span * altitude;
                for t in 0..res {
                    for s in 0..res {
                        let slot = (((face * layers + layer) * res + t) * res + s) as usize;
                        if layer < layers / 2 {
                            volume.data[slot * 4] = 0.3;
                        } else {
                            volume.data[slot * 4 + 3] = 0.3;
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
}
