//! `cloud.emission`：**密度体积 → 逐体素的发射与消光**（体渲染的"材质"）。
//!
//! 输出体积的四个通道（**交错**）：
//!
//! ```text
//! R = 发射强度（标量）
//! G = 逐通道消光 σ_R
//! B = 逐通道消光 σ_G
//! A = 逐通道消光 σ_B
//! ```
//!
//! ⚠ **消光必须逐通道存**（这是第一版写错的地方）：原来把三通道消光取**平均**塞进一个 A，
//!   于是三条通道积出来**逐字相同** —— 图像是灰的，"尘埃把光染红"整个没有发生
//!   （实测 `sky_0/1/2` 的最大值 0.4537 / 0.4536 / 0.4528，只差在第四位）。
//!   逐通道消光是"星云照片为什么偏红"的**唯一**来路，不能压成一个数。
//!
//! ⚠ 发射只存**一个标量**（三格不各存一份）：这一档不建模"不同波长发不同光"，
//!   颜色由**消光**造成（蓝先被吃掉 ⇒ 透出来的偏红）。要多色发射是另一档事。
//!
//! ⚠ **为什么光照要在这里算完**（而不是留给步进）：阴影是"从这一点朝光源看，中间有多少气"
//!   —— 它**只与这一点有关**，与看它的视线无关。按体素算一遍是 `体素数 × 阴影步数`；
//!   塞进步进就是 `射线数 × 步数 × 阴影步数`，那正是"渲染期步进"做不到的原因。
//!   烘图时算得完，是因为体素数（几十万）比"射线数 × 步数"（上亿）小三个数量级。

use px_field_schema::field::{CUBE_FACES, cube_direction};
use px_volume_schema::VolumeData;
use px_volume_schema::params::density::DensityParams;
use px_volume_schema::params::emission::EmissionParams;

use crate::density::{bake_density, sample_world};

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

/// **逐体素的发射与消光**。
///
/// 每一格的算法：
/// 1. 读这一格的密度 `d`；
/// 2. 从这一点**朝光源**步进 `shadow_steps` 步，累加光深 `τ`；
/// 3. 受光 `lit = exp(-τ × shadow_gain)`；
/// 4. 发射 = `d^emission_power × emission_gain × lit`；
/// 5. 消光 = `d^extinction_power × extinction`，再按"尘埃档"加一笔（`dust_bias`）。
///
/// ⚠ 光深要用**同一个采样函数**（三线性）算，不能拿体素格点凑：格点上的 τ 会有台阶，
///   而台阶在画面上就是一圈圈等值线。
pub fn bake_emission(density: &VolumeData, params: &EmissionParams) -> VolumeData {
    let res = density.res.max(2);
    let layers = density.layers.max(2);
    let span = density.outer - density.inner;
    let light_direction = normalize(params.light);
    let light_radius = density.inner + span * params.light_radius.clamp(0.0, 1.0);

    // 朝光源的步长：整段壳分成 `shadow_steps` 段，逐段累加光深。
    let steps = params.shadow_steps.max(1);
    let total = (density.outer - light_radius).max(1e-4);
    let step = total / steps as f32;

    // ⚠ **四通道**（RGB 发射 + A 消光）⇒ 分配要乘 4。少乘就是"写到下一格"的越界。
    // ⚠ **六通道**：`[发射 R, G, B, σ_R, σ_G, σ_B]` ⇒ 分配要乘 6。
    //
    // ⚠⚠ 发射为什么必须**逐通道**（而不是一个标量）：
    //   单标量时 `每通道辐射 = 同一个 emit × 各通道透过率`，而透过率只随**总消光**走
    //   ⇒ **浓处与薄处的色相必然相同**（只是亮度不同）。
    //   于是"核白蓝 + 边玫红"（参考图的定义性特征）**根本给不出来** ——
    //   把 `σ_G` 调大能让薄处玫红、浓处也跟着变紫（前几轮"要么全灰、要么全紫"的根因）。
    //   逐通道之后，**底光**可以带自己的色相（`glow_tint`）⇒ 核里那层蓝白就有了。
    // ⚠⚠ **按行带并行**（`px_field_schema::parallel`，与场算子同一套口径，逐位相同）：
    //   一行 = 一个 `(面, 层, t)` 上的全部 `s` 与六条通道（`res × 6` 个 f32），
    //   行号 = `(face × layers + layer) × res + t` —— 与 [`VolumeData`] 的摊平顺序逐字相同。
    //   每格只写自己那一段 ⇒ 结果与串行**完全一致**（缓存键不受影响）。
    //   ⚠ 这一档是体积链上最贵的一处（每格一次 `shadow_steps` 步的阴影行进）：
    //     实测 shape 64 单线程 **56.8 秒** —— 占整轮"改发射模型 → 看图"迭代的大头。
    let width = res as usize * 6;
    let height = (CUBE_FACES * layers * res) as usize;
    let data = px_field_schema::parallel::rows(width, height, |first, count, out| {
        for row in 0..count {
            let row_index = (first + row) as u32;
            let face = row_index / (layers * res);
            let layer = (row_index % (layers * res)) / res;
            let t = row_index % res;
            let altitude = layer as f32 / (layers - 1) as f32;
            let radius = density.inner + span * altitude;
            let s_t = (t as f32 + 0.5) / res as f32;
            for s in 0..res {
                let s_s = (s as f32 + 0.5) / res as f32;
                let direction = cube_direction(face, s_s, s_t);
                let position = [
                    direction[0] * radius,
                    direction[1] * radius,
                    direction[2] * radius,
                ];

                let d = sample_world(density, position).max(0.0);

                // ---- 朝光源的遮挡 ----
                let mut optical_depth = 0.0_f32;
                for step_index in 1..=steps {
                    let distance = step_index as f32 * step;
                    let probe = [
                        position[0] + light_direction[0] * distance,
                        position[1] + light_direction[1] * distance,
                        position[2] + light_direction[2] * distance,
                    ];
                    optical_depth += sample_world(density, probe) * step;
                }
                let lit = (-optical_depth * params.shadow_gain).exp();

                // ---- 中心星团：逐星阴影行进 + 1/r² + 色温 ----
                //
                // ⚠ 与方向光**并联**（不是替换）：方向光给"整体一侧亮"，星团给
                //   "内缘朝心那一圈亮、背面暗" —— 后者才是目标点名的四样（朝光亮缘、
                //   背光暗面、参差剪影、前景挡后景）的主要来源。
                // ⚠ 星位走 **Fibonacci 球**（确定性的）：烘图必须可复现，
                //   随机星位会让同一份配方每次烘出不同的字节。
                let mut cluster_lit = 0.0_f32;
                if params.cluster_count > 0 {
                    let stars = params.cluster_count.min(8);
                    let golden = 2.399_963_2_f32;
                    for star in 0..stars {
                        let z = 1.0 - 2.0 * (star as f32 + 0.5) / stars as f32;
                        let ring = (1.0 - z * z).max(0.0).sqrt();
                        let phi = golden * star as f32;
                        let spread = params.cluster_spread * radius;
                        let star_position = [
                            ring * phi.cos() * spread,
                            ring * phi.sin() * spread,
                            z * spread,
                        ];
                        let to_star = [
                            star_position[0] - position[0],
                            star_position[1] - position[1],
                            star_position[2] - position[2],
                        ];
                        let distance = (to_star[0] * to_star[0]
                            + to_star[1] * to_star[1]
                            + to_star[2] * to_star[2])
                            .sqrt()
                            .max(1e-4);
                        let away = [
                            to_star[0] / distance,
                            to_star[1] / distance,
                            to_star[2] / distance,
                        ];
                        let count = params.cluster_steps.max(1);
                        let through = distance / count as f32;
                        let mut tau = 0.0_f32;
                        for step_index in 1..=count {
                            let far = step_index as f32 * through;
                            let probe = [
                                position[0] + away[0] * far,
                                position[1] + away[1] * far,
                                position[2] + away[2] * far,
                            ];
                            tau += sample_world(density, probe) * through;
                        }
                        // `1/r²` 以壳内半径为单位归一化（否则换 inner 就换亮度）。
                        let falloff = (density.inner * density.inner) / (distance * distance);
                        cluster_lit += (-tau * params.shadow_gain).exp() * falloff;
                    }
                    cluster_lit /= stars as f32;
                }

                // ---- 两份发射，逐通道 ----
                // 主项（高幂 ⇒ 只有浓的地方亮）是**中性**的，它的颜色由消光给
                // （薄处自然被染成玫红）；底光（低幂 ⇒ 浓处相对更强）带自己的色相。
                let main = d.powf(params.emission_power) * params.emission_gain * lit;
                // ⚠ **门控**（不是曲线）：门限以下**正好是 0** ⇒ 暗部完全交回
                //   "主发射 + 逐通道消光"（玫红）。见 glow_threshold 的文档。
                let above = (d - params.glow_threshold).max(0.0);
                let glow = above.powf(params.glow_power) * params.glow_gain * lit;

                // ---- 消光：逐通道 + 尘埃那一笔 ----
                let base = d.powf(params.extinction_power);
                let dust = ((d - params.dust_threshold).max(0.0)) * params.dust_bias;

                // 星团那一笔：与主发射同形状（只在有气的地方亮），颜色走色温。
                let cluster = d.powf(params.emission_power) * params.cluster_gain * cluster_lit;

                let at = row * width + s as usize * 6;
                for channel in 0..3 {
                    out[at + channel] =
                        main + glow * params.glow_tint[channel] + cluster * params.cluster_tint[channel];
                    out[at + 3 + channel] = base * params.extinction[channel] + dust;
                }
            }
        }
    });

    VolumeData {
        res,
        layers,
        inner: density.inner,
        outer: density.outer,
        data,
    }
}

/// 上游：一张密度场（体网格场）→ 一份密度体积 → 一份发射体积。
///
/// ⚠ 这是 `cloud.emission` 算子的入口：它先请 [`bake_density`] 把场搬成体积，
///   再算光照。两步都住在这一份里，是为了"密度体积"这个中间物不必单独进键
///   （它只是这一档的内部步骤）。
pub fn emit_from_field(
    density_params: &DensityParams,
    emission_params: &EmissionParams,
    canvas_width: u32,
    density_field: &px_field_schema::field::Field,
) -> Result<VolumeData, String> {
    let density = bake_density(density_params, canvas_width, density_field)?;
    Ok(bake_emission(&density, emission_params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::{Field, Projection};
    use px_field_schema::volume::VolumeShape;

    fn flat_density(res: u32, layers: u32, value: f32) -> VolumeData {
        VolumeData {
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data: vec![value; (CUBE_FACES * layers * res * res) as usize],
        }
    }

    /// **逐体素的位置都落在壳里**：格心算出来的半径必须在 `(inner, outer)` 之内，
    /// 而且从那里读到的密度就是常数。
    ///
    /// ⚠ 这条把"位置算错"与"光照算错"分开：整幅全黑时，先怀疑的是位置（半径算到壳外
    ///   就处处读 0），而不是曝光。
    #[test]
    fn every_voxel_centre_lands_inside_the_shell() {
        let density = flat_density(8, 4, 0.5);
        let span = density.outer - density.inner;
        let mut worst = 0.0_f32;
        for face in 0..CUBE_FACES {
            for layer in 0..density.layers {
                let altitude = layer as f32 / (density.layers - 1) as f32;
                let radius = density.inner + span * altitude;
                assert!(
                    (density.inner..=density.outer).contains(&radius),
                    "层 {layer} 的半径 {radius} 跑到壳外了（壳是 {}..{}）",
                    density.inner,
                    density.outer
                );
                for t in 0..density.res {
                    for s in 0..density.res {
                        let direction = cube_direction(
                            face,
                            (s as f32 + 0.5) / density.res as f32,
                            (t as f32 + 0.5) / density.res as f32,
                        );
                        let point = [
                            direction[0] * radius,
                            direction[1] * radius,
                            direction[2] * radius,
                        ];
                        let read = sample_world(&density, point);
                        worst = worst.max((read - 0.5).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-4, "格心读到的密度应当是 0.5，最大偏差 {worst}");
    }

    /// **正对光源 vs 背对光源：亮度差一个数量级**（这就是"亮脊 + 暗柱"的来源）。
    ///
    /// ⚠ 判据要拿**正对/正背**两处比，不能拿"朝光那半球 vs 背光那半球"比：均匀密度下
    ///   朝光的半球里有四个面（0/2/3/4）几乎没有遮挡，只有**正背**的面 1 与斜背的面 5 被挡
    ///   ⇒ 半球平均把对比冲淡到 1.12 倍，而逐面的真实对比是 5 倍（实测）。判据写错会让
    ///   "机制没生效"与"判据太粗"看起来一模一样。
    #[test]
    fn the_face_pointing_at_the_light_is_much_brighter_than_the_one_pointing_away() {
        let density = flat_density(16, 8, 0.6);
        let params = EmissionParams {
            light: [1.0, 0.0, 0.0],
            light_radius: 0.1,
            shadow_steps: 32,
            shadow_gain: 3.0,
            ..Default::default()
        };
        let emission = bake_emission(&density, &params);
        let emit_of = |face: u32| -> f32 {
            let layer = density.layers - 1;
            let mid = density.res / 2;
            // ⚠ 布局是六通道 ⇒ 发射在 lane 0..3（这里是 R）。
            emission.data[((((face * density.layers + layer) * density.res + mid) * density.res
                + mid)
                * 6) as usize]
        };
        let facing = emit_of(0); // 面 0 的法线朝 +X，光源就在 +X
        let away = emit_of(1); // 面 1 朝 −X
        assert!(
            facing > away * 3.0,
            "正对光源 {facing:.5} 应当远亮于背对 {away:.5}（差不到 3 倍说明遮挡没生效）"
        );
        assert!(away > 0.0, "背光那一侧还应当有自发光的底（不是全黑）");
    }

    /// **消光随密度单调**，而且尘埃那一笔让高密度区更不透明。
    #[test]
    fn extinction_grows_with_density() {
        let thin = bake_emission(
            &flat_density(8, 6, 0.15),
            &EmissionParams {
                shadow_gain: 0.0,
                ..Default::default()
            },
        );
        let thick = bake_emission(
            &flat_density(8, 6, 0.85),
            &EmissionParams {
                shadow_gain: 0.0,
                ..Default::default()
            },
        );
        let mean_alpha = |volume: &VolumeData| -> f64 {
            volume.data.chunks(4).map(|c| c[3] as f64).sum::<f64>()
                / volume.data.len().max(4) as f64
                * 6.0
        };
        assert!(
            mean_alpha(&thick) > mean_alpha(&thin),
            "浓的地方消光必须更大：{} vs {}",
            mean_alpha(&thick),
            mean_alpha(&thin)
        );
    }

    /// **逐通道消光不同 ⇒ 尘埃会把光染红**：A 通道的平均消光与三档系数的比例一致。
    #[test]
    fn the_extinction_channels_differ() {
        let params = EmissionParams {
            extinction: [1.0, 2.0, 3.0],
            dust_bias: 0.0,
            shadow_gain: 0.0,
            ..Default::default()
        };
        let density = flat_density(8, 4, 0.5);
        let emission = bake_emission(&density, &params);
        // 先确认密度真的读到了（阳性对照）：读不到的话下面那条测的是"0 的平均"。
        assert!(
            emission.data[3] > 1e-6,
            "格心该读到密度 0.5，σ_R 却是 {}（密度没读到）",
            emission.data[3]
        );
        // ⚠ 布局是**六通道** `[发射 R, G, B, σ_R, σ_G, σ_B]` ⇒ `data[5]` 才是 σ_B。
        //   0.5 密度 × 3.0 = 1.5。
        let alpha = emission.data[5];
        assert!(
            (alpha - 1.5).abs() < 1e-3,
            "0.5 密度 × B 通道系数 3 应当是 1.5，实际 {alpha}"
        );
        // 顺带钉住三条消光通道确实是**三个不同的数**（取平均会让它们逐字相同）。
        assert!(
            emission.data[3] < emission.data[4] && emission.data[4] < emission.data[5],
            "三个消光通道必须逐格不同：{:?}",
            &emission.data[3..6]
        );
    }

    /// **搬运那一侧的形状对不上就当场拒**。
    #[test]
    fn a_field_of_the_wrong_shape_is_rejected() {
        let field = Field::filled_with(4, 4, 0.0, Projection::Volume);
        let shape = VolumeShape { res: 4, layers: 3 };
        assert_ne!(field.height, shape.height());
        assert!(
            emit_from_field(
                &DensityParams::default(),
                &EmissionParams::default(),
                shape.res,
                &field
            )
            .is_err(),
            "形状对不上的场必须被拒"
        );
    }
}
