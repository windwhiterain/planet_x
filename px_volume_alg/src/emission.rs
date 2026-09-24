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
use px_sparse::StarField;
use px_volume_schema::VolumeData;
use px_volume_schema::params::density::DensityParams;
use px_volume_schema::params::emission::EmissionParams;

use crate::density::{bake_density, sample_world};
use crate::stars::brightest_near;

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
/// 光深要用**同一个采样函数**（三线性）算，不能拿体素格点凑：格点上的 τ 会有台阶，
///   而台阶在画面上就是一圈圈等值线。
///
/// ⚠⚠ **星光照气体**（`starlight_*`，2026-09-25）：逐体素查 R3 星场里附近的星，
///   算 `亮度 / (d² + soft²) × 朝它走的遮挡`，再乘 `density^power` 折进发射。
///   这就是"气被星照亮"那一条 —— 参考图里星周围那圈晕**长在气上**：
///   星在浓气里晕小而实、在空处几乎没有晕，而且它是世界坐标里的量（没有贴图分辨率、
///   没有面棱、没有椭圆）。
///
/// ⚠ 它与旧的 `cluster_*`（4 颗只知道方向的程序化假光源）是**替换关系**：星簇现在是
///   星表里真实的一组星，既直射进画面、也照亮周围的气 ⇒ 两套光照不会打架。
///
/// ⚠ 逐星遮挡是**逐体素**的量（与视线无关），与方向光那一条同一条理由：算一遍是
///   `体素数 × 候选星数 × 星影步数`，塞进天空那一步进就是乘上"步数"。
pub fn bake_emission(
    density: &VolumeData,
    stars: &StarField,
    params: &EmissionParams,
) -> VolumeData {
    let res = density.res.max(2);
    let layers = density.layers.max(2);
    // ⚠ 径向律是**参数空间线性、世界等比**（与 `px_volume_schema::volume::Shell` 同一条）：
    //   层心的高度就是参数 `u`，世界半径要过 `Shell`；方向光的半径同理（那个参数也是一个高度）。
    let shell = px_volume_schema::volume::Shell::new(density.inner, density.outer);
    let light_direction = normalize(params.light);
    let light_radius = shell.radius_of(params.light_radius.clamp(0.0, 1.0));
    // ⚠ 光是一个**点**（`light` 方向上、半径 `light_radius` 处）：方向与步长都随体素变。
    let light_position = [
        light_direction[0] * light_radius,
        light_direction[1] * light_radius,
        light_direction[2] * light_radius,
    ];

    // 朝光源的步数（步长由**到光点的真实距离**除以它，逐体素不同）。
    let steps = params.shadow_steps.max(1);

    // 星光那一档的常数：查多远（`starlight_radius`，**物理**旋钮）、多软、逐星走几步、
    // 最多吃几颗。⚠ 它与 `stars.cell`（**存储**细格）是两个旋钮，这里只读前者。
    let star_reach = params.starlight_radius.max(1e-4);
    let star_soft2 = (params.starlight_soft.max(1e-4)).powi(2);
    let star_steps = params.starlight_steps.max(1);
    let star_keep = params.starlight_max as usize;

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
            let radius = shell.radius_of(altitude);
            let s_t = (t as f32 + 0.5) / res as f32;
            // ⚠ 候选表**一行一份**（复用，不逐体素分配）：`brightest_near` 只清空它。
            let mut candidates: Vec<px_sparse::Star> = Vec::new();
            for s in 0..res {
                let s_s = (s as f32 + 0.5) / res as f32;
                let direction = cube_direction(face, s_s, s_t);
                let position = [
                    direction[0] * radius,
                    direction[1] * radius,
                    direction[2] * radius,
                ];

                let d = sample_world(density, position).max(0.0);

                // ---- 朝**点光源**的遮挡 + 1/d² 辐照 ----
                //
                // ⚠⚠ 这里从前是**平行光 + 只算遮挡**（`probe = position + light_direction × step`）
                //   —— 没有任何距离衰减 ⇒ 整团气被均匀照亮 ⇒ 无论怎么调，画面都"像自发光"。
                //   与星那一档缺 `1/r²` 是**同一类遗漏**（用户 2026-09-25 两次都是先看出画不对）。
                //   现在：朝光**点**步进（方向随体素变）、长度是真实距离、再乘 `1/d²`。
                let to_light = [
                    light_position[0] - position[0],
                    light_position[1] - position[1],
                    light_position[2] - position[2],
                ];
                let light_distance = (to_light[0] * to_light[0]
                    + to_light[1] * to_light[1]
                    + to_light[2] * to_light[2])
                    .sqrt()
                    .max(1e-4);
                let through = light_distance / steps as f32;
                let mut optical_depth = 0.0_f32;
                for step_index in 1..=steps {
                    let far = step_index as f32 * through;
                    let probe = [
                        position[0] + to_light[0] / light_distance * far,
                        position[1] + to_light[1] / light_distance * far,
                        position[2] + to_light[2] / light_distance * far,
                    ];
                    optical_depth += sample_world(density, probe) * through;
                }
                // 辐照锚在**光自己的半径**上（那一圈是满强度）：`(light_radius / d)²`。
                // ⚠ 这就是用户说的"遮挡必须和搜索范围比配"：`density × 距离 × shadow_gain`
                //   要落在 O(1) 上，遮挡才读得出来（实测 0.06 × 0.4 × 3 = 0.07 ⇒ 只压 7%。
                //   现在距离由光的位置定、`shadow_gain` 的量纲也跟着变）。
                let reach = (light_radius / light_distance).clamp(0.0, 1.0);
                let lit = (-optical_depth * params.shadow_gain).exp() * reach * reach;

                // ---- 星光照气体（= 星光被气**散射**）：附近最亮的几颗 + 逐星遮挡 ----
                //
                // ⚠ 与方向光**并联**（不是替换）：方向光给"整体一侧亮"，星给
                //   "内缘朝心那一圈亮、背面暗" —— 后者才是目标点名的四样（朝光亮缘、
                //   背光暗面、参差剪影、前景挡后景）的主要来源。
                // ⚠ 辐照取 `亮度 / (d² + soft²)`：`soft` 是软化半径（`d → 0` 时不发散）。
                //   星是**幂律**亮的（少数亮星 + 大量暗星）⇒ 只吃前 `starlight_max` 颗。
                //
                // ⚠⚠ **无色**（2026-09-25，用户口径：星云不许自发光，亮度只能来自星光的散射）：
                //   逐通道加的是**同一个** `visible`，**不带** `star.tint` —— 这一笔的语义是
                //   "星光被气散射出来的光"，它的通道配比由 `glow_tint`（红）给；
                //   星自己的色温只走**直射**那一档（`raymatch` 里 `亮度 × star_tint`，蓝）。
                //   带上 `star.tint` 的话蓝星散射出来的就是蓝光，分色诊断整个失效。
                let mut star_lit = [0.0_f32; 3];
                if params.starlight_gain > 0.0 {
                    brightest_near(stars, position, star_reach, star_keep, &mut candidates);
                    for star in &candidates {
                        let to_star = [
                            star.position[0] - position[0],
                            star.position[1] - position[1],
                            star.position[2] - position[2],
                        ];
                        let distance2 = to_star[0] * to_star[0]
                            + to_star[1] * to_star[1]
                            + to_star[2] * to_star[2];
                        let distance = distance2.sqrt().max(1e-4);
                        let away = [
                            to_star[0] / distance,
                            to_star[1] / distance,
                            to_star[2] / distance,
                        ];
                        let through = distance / star_steps as f32;
                        let mut tau = 0.0_f32;
                        for step_index in 1..=star_steps {
                            let far = step_index as f32 * through;
                            let probe = [
                                position[0] + away[0] * far,
                                position[1] + away[1] * far,
                                position[2] + away[2] * far,
                            ];
                            tau += sample_world(density, probe) * through;
                        }
                        let falloff = star.brightness / (distance2 + star_soft2);
                        let visible = (-tau * params.shadow_gain).exp() * falloff;
                        for channel in 0..3 {
                            star_lit[channel] += visible;
                        }
                    }
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

                // 星光被气**散射**那一笔：与主发射**同形状**（只在有气的地方亮），
                // 而 `star_lit` 只是一份**无色的形状**（哪里的气被照得多亮）—— 颜色由
                // 下面的 `glow_tint` 给（用户 2026-09-25：散射走红、星自己的直射光走蓝）。
                let star_emit = d.powf(params.emission_power) * params.starlight_gain;

                let at = row * width + s as usize * 6;
                for channel in 0..3 {
                    // ⚠ `scatter_tint` 只乘**发射**通道（消光通道不动）：尘埃染色是物理，
                    //   而分色是诊断（用户 2026-09-25：散射走红、星光走蓝）。
                    // ⚠⚠ `glow_tint` 现在**也乘 `main`** ⇒ 它的含义升级为"星云散射出来的光的
                    //   通道配比"（用户 2026-09-25："散射定为红色"）。从前它只管 `glow` 那一档
                    //   （而那一档关着 ⇒ 等于没用）。这样"散射走红"不需要任何新 uniform。
                    // ⚠⚠ **散射项现在也吃 `glow_tint`**（`star_emit × star_lit[channel]` 那一笔）：
                    //   星云不许自发光（`emission_gain` / `glow_gain` 都关着）⇒ 画面上亮的那
                    //   一片就是星光的散射，而它必须是**红色** ⇒ 整条 `(main + glow + 散射)`
                    //   乘同一份配比。`star_lit` 无色 ⇒ 出来的是纯红，正是分色诊断要的。
                    out[at + channel] = (main + glow + star_emit * star_lit[channel])
                        * params.glow_tint[channel]
                        * params.scatter_tint[channel];
                    out[at + 3 + channel] = base * params.extinction[channel] + dust;
                }
            }
        }
    });

    VolumeData {
        lanes: 6,
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
    stars: &StarField,
    density_field: &px_field_schema::field::Field,
) -> Result<VolumeData, String> {
    let density = bake_density(density_params, density_field)?;
    Ok(bake_emission(&density, stars, emission_params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_field_schema::field::{Field, Projection};
    use px_field_schema::volume::VolumeShape;

    fn flat_density(res: u32, layers: u32, value: f32) -> VolumeData {
        VolumeData {
            lanes: 1,
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data: vec![value; (CUBE_FACES * layers * res * res) as usize],
        }
    }

    /// 一份**空**星场（`starlight_gain > 0` 时它必须不产生任何光）。
    fn no_stars() -> StarField {
        StarField::build(empty_meta(), &[], &[], &[]).expect("造空星场")
    }

    /// 一颗亮星（放在 +X 轴上半径 1.5 处）。
    fn one_star(brightness: f32) -> StarField {
        StarField::build(
            empty_meta(),
            &[[1.5, 0.0, 0.0]],
            &[brightness],
            &[[1.0, 1.0, 1.0]],
        )
        .expect("造一颗星")
    }

    /// 一份"只有一颗星那么大的盒"的格参数（够装下 +X 上半径 1.5 那颗）。
    fn empty_meta() -> px_sparse::GridMeta {
        let block = px_sparse::grid::CHUNK_CELLS;
        let dims = 3 * block;
        px_sparse::GridMeta {
            cell: 0.5,
            origin: [-3.0; 3],
            dims: [dims, dims, dims],
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
        let shell = px_volume_schema::volume::Shell::new(density.inner, density.outer);
        let mut worst = 0.0_f32;
        for face in 0..CUBE_FACES {
            for layer in 0..density.layers {
                let altitude = layer as f32 / (density.layers - 1) as f32;
                let radius = shell.radius_of(altitude);
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
        let emission = bake_emission(&density, &no_stars(), &params);
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
            &no_stars(),
            &EmissionParams {
                shadow_gain: 0.0,
                ..Default::default()
            },
        );
        let thick = bake_emission(
            &flat_density(8, 6, 0.85),
            &no_stars(),
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
        let emission = bake_emission(&density, &no_stars(), &params);
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
                &no_stars(),
                &field
            )
            .is_err(),
            "形状对不上的场必须被拒"
        );
    }

    /// **星光照的是气**：同一颗星、同一份参数，只有"把气放进去"才出光 ——
    /// 而且**空星场**必须与"没有星"逐字相同（关掉星光那一档的等价性）。
    ///
    /// ⚠ 这一条钉两件事：`starlight_gain` 真的接到了输出上；以及"没有星 ⇒ 没有这一笔"
    ///   （空星场读出 0 颗 ⇒ 逐字不变，这正是 `starlight_gain = 0` 与其他档共存的理由）。
    #[test]
    fn starlight_needs_both_a_star_and_gas() {
        let params = EmissionParams {
            starlight_gain: 4.0,
            starlight_radius: 0.5,
            starlight_steps: 4,
            shadow_gain: 0.0,
            ..Default::default()
        };
        let emission_of = |brightness: f32| -> [f64; 3] {
            let volume = bake_emission(&flat_density(8, 4, 0.6), &one_star(brightness), &params);
            let sum = |lane: usize| -> f64 {
                volume
                    .data
                    .chunks(6)
                    .map(|chunk| chunk[lane] as f64)
                    .sum::<f64>()
            };
            [sum(0), sum(1), sum(2)]
        };
        let off = emission_of(0.0);
        let on = emission_of(8.0);
        assert!(
            on[0] > off[0] * 1.5,
            "点亮一颗星应当让气体明显更亮：{:.3} vs {:.3}",
            on[0],
            off[0]
        );
        // 亮度是线性进光照的（`亮度 / (d² + soft²)`）。
        let brighter = emission_of(16.0);
        assert!(
            (brighter[0] - off[0]) > 1.8 * (on[0] - off[0]),
            "星光那一笔必须跟着星的亮度线性涨"
        );
    }

    /// **没有气就没有星光照**：把密度放到 0，星光那一笔必然消失
    /// （散射发生在气上，不是空间里发光）。
    #[test]
    fn starlight_vanishes_without_gas() {
        let params = EmissionParams {
            starlight_gain: 4.0,
            starlight_radius: 0.5,
            starlight_steps: 4,
            shadow_gain: 0.0,
            ..Default::default()
        };
        let vacuum = bake_emission(&flat_density(8, 4, 0.0), &one_star(32.0), &params);
        for value in &vacuum.data {
            assert_eq!(*value, 0.0, "真空里不该有星光照出来的光");
        }
    }
}
