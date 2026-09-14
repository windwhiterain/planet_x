use crate::probe;

use px_render::clouds::{CLOUD_BASE, CLOUD_TOP, CloudParams};
use px_verify::cloud_field::CloudFieldParams;
use px_verify::noise::{FbmSettings, fbm_3, gradient_noise_3};
use crate::probe::{MASK_GRADIENT, Mask, POINTS, STEPS, quantised};

const SWEEP: [f32; STEPS] = [8e-5, 4e-5, 2e-5, 1e-5, 5e-6];
const MASK: f32 = 153.0 / 255.0;
const LIVE: f32 = 1e-3;
const FLOOR: f64 = 1e-3;

fn production_params() -> CloudParams {
    CloudParams::new(CLOUD_BASE, CLOUD_TOP, 900.0)
}

fn reference(params: &CloudParams) -> CloudFieldParams {
    CloudFieldParams {
        orientation: params.orientation.to_array(),
        inner: params.inner,
        outer: params.outer,
        coverage: params.coverage,
        base: params.base,
        top: params.top,
        detail_scale: params.detail_scale,
        detail_strength: params.detail_strength,
        erode: params.erode,
        taper: params.taper,
        coverage_gain: params.coverage_gain,
        seed: params.seed,
    }
}

fn lifted(point: [f32; 3]) -> [f64; 3] {
    [point[0] as f64, point[1] as f64, point[2] as f64]
}

fn shell_points() -> Vec<[f32; 3]> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f32 / (1u64 << 53) as f32
    };
    (0..POINTS)
        .map(|_| {
            let z = next() * 2.0 - 1.0;
            let phi = next() * std::f32::consts::TAU;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let radius = CLOUD_BASE + (CLOUD_TOP - CLOUD_BASE) * next();
            [
                ring * phi.cos() * radius,
                z * radius,
                ring * phi.sin() * radius,
            ]
        })
        .collect()
}

fn distance(one: [f32; 3], two: [f64; 3]) -> f64 {
    let mut worst = 0.0_f64;
    for axis in 0..3 {
        worst = worst.max((one[axis] as f64 - two[axis]).abs());
    }
    worst
}

fn magnitude(values: [f64; 3]) -> f64 {
    values[0].abs().max(values[1].abs()).max(values[2].abs())
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|one, two| one.partial_cmp(two).expect("误差里出现了 NaN"));
    values[values.len() / 2]
}

fn the_reference_ports_the_shader_parameters_exactly() {
    let params = production_params();
    let field = reference(&params);
    let points = shell_points();
    let rows = probe::run(&points, &params, SWEEP, Mask::Constant(MASK));
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let cover = field.cover_from_mask(MASK) as f64;
    let mut altitude_worst = 0.0_f64;
    let mut radius_worst = 0.0_f64;
    let mut direction_worst = 0.0_f64;
    let mut cover_worst = 0.0_f64;
    for (row, point) in rows.iter().zip(&points) {
        let medium = field.medium_of(*point);
        altitude_worst = altitude_worst.max((row.altitude as f64 - medium.altitude as f64).abs());
        radius_worst = radius_worst.max((row.radius as f64 - medium.radius as f64).abs());
        direction_worst = direction_worst.max(distance(
            row.direction,
            [
                medium.direction[0] as f64,
                medium.direction[1] as f64,
                medium.direction[2] as f64,
            ],
        ));
        cover_worst = cover_worst.max((row.cover as f64 - cover).abs());
    }
    println!(
        "参考实现与 shader 的中介量最大出入：altitude {altitude_worst:e}，radius {radius_worst:e}，\
         direction {direction_worst:e}，cover {cover_worst:e}"
    );
    assert!(
        altitude_worst < 1e-5,
        "altitude 对不上（{altitude_worst:e}）⇒ 参考实现没有照抄 shader 的 medium_of"
    );
    assert!(
        radius_worst < 1e-5,
        "radius 对不上（{radius_worst:e}）⇒ 参考实现没有照抄 shader 的 medium_of"
    );
    assert!(
        direction_worst < 1e-6,
        "direction 对不上（{direction_worst:e}）⇒ 参考实现没有照抄 shader 的 medium_of"
    );
    assert!(
        cover_worst < 1e-6,
        "cover 对不上（{cover_worst:e}）⇒ 参考实现的覆盖度重映射和 shader 不一样"
    );
}

fn the_reference_and_the_shader_agree_on_the_field_value() {
    let params = production_params();
    let field = reference(&params);
    let points = shell_points();
    let rows = probe::run(&points, &params, SWEEP, Mask::Constant(MASK));
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let names = [
        "footprint",
        "lobed",
        "floor_here",
        "ceiling",
        "under_top",
        "shape",
    ];
    let mut stage_worst = [0.0_f64; 6];
    let mut stage_at = [[0.0_f32; 3]; 6];
    let mut noise_worst = 0.0_f64;
    let mut cover_worst = 0.0_f64;
    let mut value_worst = 0.0_f64;
    let mut value_at = [0.0_f32; 3];
    let mut tower_worst = 0.0_f64;
    let mut skin_worst = 0.0_f64;
    let mut tower_single_worst = 0.0_f64;
    let mut skin_single_worst = 0.0_f64;
    let mut live = 0_usize;

    let fixed = [1.3_f32, 2.7, -0.4];
    let settings = FbmSettings {
        frequency: 1.0,
        octaves: 3,
        lacunarity: 2.0,
        gain: 0.5,
        seed: 7,
    };
    let fbm_here = fbm_3(fixed, &settings);
    let gradient_here = gradient_noise_3(fixed, 7);
    println!(
        "固定点 {fixed:?}：fbm_3（3 八度）参考 {fbm_here} 对 shader {}；\
         gradient_noise_3 参考 {gradient_here} 对 shader {}",
        rows[0].fbm_fixed, rows[0].gradient_fixed,
    );

    for (row, point) in rows.iter().zip(&points) {
        if row.field <= LIVE {
            continue;
        }
        live += 1;
        let medium = field.medium_of(*point);
        tower_worst = tower_worst.max(
            (row.tower
                - field.sampled_noise(
                    medium.direction,
                    medium.altitude,
                    field.detail_scale * 0.35,
                    3,
                    field.seed,
                )) as f64,
        );
        skin_worst = skin_worst.max(
            (row.skin
                - field.sampled_noise(
                    medium.direction,
                    medium.altitude,
                    field.detail_scale * 1.70,
                    2,
                    field.seed ^ 31,
                )) as f64,
        );
        tower_single_worst = tower_single_worst.max(
            (row.tower_single
                - field.sampled_noise(
                    medium.direction,
                    medium.altitude,
                    field.detail_scale * 0.35,
                    1,
                    field.seed,
                )) as f64,
        );
        skin_single_worst = skin_single_worst.max(
            (row.skin_single
                - field.sampled_noise(
                    medium.direction,
                    medium.altitude,
                    field.detail_scale * 1.70,
                    1,
                    field.seed ^ 31,
                )) as f64,
        );
        let noise = field.billows(medium.direction, medium.altitude);
        let value = field.density(*point, row.cover);
        let gap = (row.field as f64 - value as f64).abs();
        if gap > value_worst {
            value_worst = gap;
            value_at = *point;
        }
        noise_worst = noise_worst.max((row.noise as f64 - noise as f64).abs());
        cover_worst = cover_worst.max((row.analytic_cover as f64 - row.cover as f64).abs());

        let stages = field.stages(row.cover, medium.altitude, noise);
        let shader = [
            row.footprint,
            row.lobed,
            row.floor_here,
            row.ceiling,
            row.under_top,
            row.shape,
        ];
        for index in 0..6 {
            let gap = (shader[index] as f64 - stages[index] as f64).abs();
            if gap > stage_worst[index] {
                stage_worst[index] = gap;
                stage_at[index] = *point;
            }
        }
    }

    assert!(live > 40, "可用点只有 {live} 个，这个测试没在测东西");
    println!("可用点 {live} 个；shader 对参考实现逐级最大出入：");
    for (index, name) in names.iter().enumerate() {
        println!(
            "  {name:<11} {:e}（在 {:?}）",
            stage_worst[index], stage_at[index]
        );
    }
    println!("  {:<11} {noise_worst:e}", "noise");
    println!("  {:<11} {cover_worst:e}", "两条覆盖度路径");
    println!("  {:<11} {value_worst:e}（在 {value_at:?}）", "场值");
    println!("  {:<11} {tower_single_worst:e} / {skin_single_worst:e}", "单八度");
    println!("  {:<11} {tower_worst:e} / {skin_worst:e}", "多八度");

    assert!(
        (rows[0].fbm_fixed as f64 - fbm_here as f64).abs() < 1e-6,
        "固定点上 fbm_3 就已经不一样：参考 {fbm_here} 对 shader {} ⇒ 两个 fbm_3 实现不同",
        rows[0].fbm_fixed,
    );
    assert!(
        (rows[0].gradient_fixed as f64 - gradient_here as f64).abs() < 1e-6,
        "固定点上 gradient_noise_3 就已经不一样：参考 {gradient_here} 对 shader {}",
        rows[0].gradient_fixed,
    );
    assert!(
        tower_single_worst < 1e-4,
        "单八度 tower 对不上（{tower_single_worst:e}）⇒ 差在采样坐标而不是八度累加"
    );
    assert!(
        skin_single_worst < 1e-4,
        "单八度 skin 对不上（{skin_single_worst:e}）⇒ 差在采样坐标而不是八度累加"
    );
    assert!(
        tower_worst < 1e-4,
        "多八度 tower 对不上（{tower_worst:e}）"
    );
    assert!(skin_worst < 1e-4, "多八度 skin 对不上（{skin_worst:e}）");

    assert!(
        cover_worst < 1e-6,
        "coverage_of 和 coverage_gradient_of 的 r 通道不一致（{cover_worst:e}）⇒ 解析路径量的是另一个覆盖度"
    );
    assert!(
        noise_worst < 1e-4,
        "billows 对不上（{noise_worst:e}）⇒ 参考实现的噪声混合和 shader 不一样"
    );
    for (index, name) in names.iter().enumerate() {
        assert!(
            stage_worst[index] < 1e-4,
            "{name} 对不上（{:e}，在 {:?}）",
            stage_worst[index],
            stage_at[index]
        );
    }
    assert!(
        value_worst < 1e-4,
        "场值对不上：{value_worst:e}（在 {value_at:?}）"
    );
}

fn the_shader_coverage_term_matches_the_exact_band_gradient() {
    let params = production_params();
    let field = reference(&params);
    let points = shell_points();
    let rows = probe::run(&points, &params, SWEEP, Mask::Varying);
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let slope = [
        quantised(MASK_GRADIENT[0]) as f64,
        quantised(MASK_GRADIENT[1]) as f64,
        quantised(MASK_GRADIENT[2]) as f64,
    ];
    let mut fixed = Vec::new();
    let mut broken = Vec::new();
    let mut baked_worst = 0.0_f64;
    let mut skipped = 0_usize;

    for (row, point) in rows.iter().zip(&points) {
        if row.field <= LIVE {
            continue;
        }
        baked_worst = baked_worst.max((row.baked_g as f64 - slope[0]).abs());
        let direction = [
            row.direction[0] as f64,
            row.direction[1] as f64,
            row.direction[2] as f64,
        ];
        let cover_slope = [
            (row.chain * quantised(MASK_GRADIENT[0])) as f64,
            (row.chain * quantised(MASK_GRADIENT[1])) as f64,
            (row.chain * quantised(MASK_GRADIENT[2])) as f64,
        ];
        let base = row.cover as f64
            - (cover_slope[0] * direction[0]
                + cover_slope[1] * direction[1]
                + cover_slope[2] * direction[2]);
        let exact = field.band_gradient(lifted(*point), base, cover_slope);
        let scale = magnitude(exact);
        let axis = [row.new_axis[0], row.new_axis[1], row.new_axis[2]];
        if scale < FLOOR || magnitude([axis[0] as f64, axis[1] as f64, axis[2] as f64]) < 1e-4 {
            skipped += 1;
            continue;
        }
        let as_before = [
            row.analytic[0] - row.new_axis[0] + row.old_axis[0],
            row.analytic[1] - row.new_axis[1] + row.old_axis[1],
            row.analytic[2] - row.new_axis[2] + row.old_axis[2],
        ];
        fixed.push(distance(row.analytic, exact) / scale);
        broken.push(distance(as_before, exact) / scale);
    }

    assert!(
        fixed.len() > 20,
        "覆盖度轴非零的可用点只有 {} 个（跳过 {skipped} 个），这个测试没在测东西",
        fixed.len()
    );
    let fixed_median = median(&mut fixed);
    let broken_median = median(&mut broken);
    println!(
        "变覆盖度：可用点 {} 个；探针读到的 baked.g 对烘焙值最大出入 {baked_worst:e}",
        broken.len()
    );
    println!("  修复后的覆盖度项：中位相对偏差 {fixed_median:e}（最大 {:e}）", fixed.iter().fold(0.0_f64, |worst, value| worst.max(*value)));
    println!("  修复前的覆盖度项：中位相对偏差 {broken_median:e}");

    assert!(
        baked_worst < 1e-6,
        "探针读到的 baked.g 与烘焙值不符（{baked_worst:e}）⇒ 参考实现用的梯度分量不是 shader 用的那个"
    );
    assert!(
        fixed_median < 1e-4,
        "修复后的覆盖度项对精确梯度仍有 {fixed_median:e} 的偏差 ⇒ 覆盖度那条链还没对"
    );
    assert!(
        broken_median > 1e-2,
        "修复前的覆盖度项只偏了 {broken_median:e} ⇒ 这个 arbiter 分不出对错，正向对照失效"
    );
}

fn the_shader_analytic_gradient_matches_the_exact_field_gradient() {
    let params = production_params();
    let field = reference(&params);
    let points = shell_points();
    let rows = probe::run(&points, &params, SWEEP, Mask::Constant(MASK));
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let mut analytic_relative = Vec::new();
    let mut fd_relative = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut analytic_worst = 0.0_f64;
    let mut analytic_worst_at = [0.0_f32; 3];
    let mut live = 0_usize;
    let mut skipped = 0_usize;

    for (row, point) in rows.iter().zip(&points) {
        if row.field <= LIVE {
            continue;
        }
        let exact = field.gradient(lifted(*point), row.cover as f64);
        let scale = magnitude(exact);
        if scale < FLOOR {
            skipped += 1;
            continue;
        }
        live += 1;

        let analytic = distance(row.analytic, exact);
        analytic_relative.push(analytic / scale);
        if analytic > analytic_worst {
            analytic_worst = analytic;
            analytic_worst_at = *point;
        }
        for (slot, bucket) in fd_relative.iter_mut().enumerate() {
            bucket.push(distance(row.step[slot], exact) / scale);
        }
    }

    assert!(
        live > 40,
        "可用于比对的点只有 {live} 个（跳过 {skipped} 个，壳内 {})，这个测试没在测东西",
        rows.iter().filter(|row| row.field > LIVE).count()
    );

    let analytic_median = median(&mut analytic_relative);
    let analytic_max = analytic_relative
        .iter()
        .fold(0.0_f64, |worst, value| worst.max(*value));
    println!(
        "可用点 {live} 个；解析梯度对精确梯度的相对偏差：中位 {analytic_median:e}，最大 {analytic_max:e}；\
         绝对最大偏差 {analytic_worst:e}（最坏点 {analytic_worst_at:?}）"
    );

    let mut fd_medians = Vec::new();
    for (slot, bucket) in fd_relative.iter_mut().enumerate() {
        let value = median(bucket);
        fd_medians.push(value);
        println!(
            "  差商 h={:e}：中位相对偏差 {value:e}，最大 {:e}",
            SWEEP[slot],
            bucket.iter().fold(0.0_f64, |worst, entry| worst.max(*entry)),
        );
    }
    let fd_best = fd_medians.iter().fold(f64::MAX, |best, value| best.min(*value));
    println!("解析式 {analytic_median:e} 对最好的差商 {fd_best:e}（h={:e}）", SWEEP[0]);

    assert!(
        analytic_median < 1e-4,
        "解析梯度对精确场梯度的中位相对偏差 {analytic_median:e} 太大 ⇒ 手写的链式法则和场不一致"
    );
    assert!(
        analytic_max < 1e-2,
        "解析梯度最坏点上相对偏差 {analytic_max:e}（在 {analytic_worst_at:?}，绝对 {analytic_worst:e}）⇒ 手写的链式法则和场不一致"
    );
    assert!(
        analytic_median < fd_best,
        "解析梯度没有比最好的差商更准（{analytic_median:e} 对 {fd_best:e}）⇒ 差商的截断误差没被解析式甩开，\
         说明差商的误差已经不是瓶颈"
    );
}

/// §46.3 的 arbiter：4 条腿的场级对拍。两条腿用同一组参数，靠进程内缓存复用
/// （原来是两个 `#[test]`，各建一次设备）。
pub fn checks() -> Vec<(&'static str, fn())> {
    vec![
        (
            "the_reference_ports_the_shader_parameters_exactly",
            the_reference_ports_the_shader_parameters_exactly,
        ),
        (
            "the_reference_and_the_shader_agree_on_the_field_value",
            the_reference_and_the_shader_agree_on_the_field_value,
        ),
        (
            "the_shader_coverage_term_matches_the_exact_band_gradient",
            the_shader_coverage_term_matches_the_exact_band_gradient,
        ),
        (
            "the_shader_analytic_gradient_matches_the_exact_field_gradient",
            the_shader_analytic_gradient_matches_the_exact_field_gradient,
        ),
    ]
}
