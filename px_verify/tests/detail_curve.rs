use px_verify::cloud_field::{CloudFieldParams, detail_curve, detail_curve_slope};

fn scene_cloud() -> CloudFieldParams {
    CloudFieldParams {
        orientation: [0.0, 0.0, 0.0, 1.0],
        inner: 1.01,
        outer: 1.06,
        coverage: 0.35,
        base: 0.06,
        top: 0.62,
        detail_scale: 16.0,
        detail_strength: 0.55,
        erode: 0.0,
        taper: 0.45,
        coverage_gain: 2.6,
        seed: 7,
    }
}

const TAU: f32 = 0.20;
const STEPS: u32 = 56;

fn scatter(count: usize) -> Vec<[f32; 3]> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64 | 1;
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
            let raw = [ring * phi.cos(), z, ring * phi.sin()];
            let size = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2]).sqrt();
            [raw[0] / size, raw[1] / size, raw[2] / size]
        })
        .collect()
}

fn point_at(direction: [f32; 3], altitude: f32, cloud: &CloudFieldParams) -> [f32; 3] {
    let radius = cloud.inner + altitude * cloud.span();
    [
        direction[0] * radius,
        direction[1] * radius,
        direction[2] * radius,
    ]
}

fn field_and_b(
    cloud: &CloudFieldParams,
    direction: [f32; 3],
    altitude: f32,
    cover: f32,
) -> (f32, f32) {
    let point = point_at(direction, altitude, cloud);
    let medium = cloud.medium_of(point);
    let noise = cloud.billows(medium.direction, medium.altitude);
    let value = cloud.shape(cover, medium.altitude, noise);
    (value, noise * noise)
}

#[test]
fn the_slope_helper_is_the_derivative_of_the_remap() {
    let mut worst = 0.0_f32;
    for b in [0.006, 0.02, 0.1, 0.35, 0.62, 0.9, 1.0] {
        let h = 1e-4_f32;
        let numeric = (detail_curve(b + h) - detail_curve(b - h)) / (2.0 * h);
        let analytic = detail_curve_slope(b);
        worst = worst.max((numeric - analytic).abs() / analytic.max(1e-6));
    }
    assert!(
        worst < 1e-2,
        "detail_curve_slope 与 d√b/db 差得太远：{worst:e}"
    );
}

#[test]
fn the_reference_gradient_follows_the_remap_chain_rule() {
    let cloud = scene_cloud();
    let cover = 0.62_f32;
    let mut worst = 0.0_f32;
    let mut sampled = 0;
    for direction in scatter(64) {
        for step in 1..=STEPS {
            let altitude = step as f32 / STEPS as f32;
            let point = point_at(direction, altitude, &cloud);
            let (value, _) = field_and_b(&cloud, direction, altitude, cover);
            if !(value > TAU && value < TAU + 0.05) {
                continue;
            }
            sampled += 1;
            let analytic = cloud.gradient(
                [point[0] as f64, point[1] as f64, point[2] as f64],
                cover as f64,
            );
            let size =
                (analytic[0] * analytic[0] + analytic[1] * analytic[1] + analytic[2] * analytic[2])
                    .sqrt()
                    .max(1.0) as f32;
            let mut gap = f32::INFINITY;
            for h in [1e-5_f32, 3e-5, 1e-4] {
                let mut numeric = [0.0_f32; 3];
                for axis in 0..3 {
                    let mut low = point;
                    let mut high = point;
                    low[axis] -= h;
                    high[axis] += h;
                    let low_medium = cloud.medium_of(low);
                    let high_medium = cloud.medium_of(high);
                    let low_field = cloud.shape(
                        cover,
                        low_medium.altitude,
                        cloud.billows(low_medium.direction, low_medium.altitude),
                    );
                    let high_field = cloud.shape(
                        cover,
                        high_medium.altitude,
                        cloud.billows(high_medium.direction, high_medium.altitude),
                    );
                    numeric[axis] = (high_field - low_field) / (2.0 * h);
                }
                let here = ((analytic[0] as f32 - numeric[0]).powi(2)
                    + (analytic[1] as f32 - numeric[1]).powi(2)
                    + (analytic[2] as f32 - numeric[2]).powi(2))
                .sqrt()
                    / size;
                gap = gap.min(here);
            }
            worst = worst.max(gap);
        }
    }
    assert!(
        sampled > 20,
        "贴着 τ 的采样点太少（{sampled} 个），这条判据没量到东西"
    );
    assert!(
        worst < 5e-2,
        "解析梯度（Dual，含 sqrt 链式法则）与中心差商不一致：最差相对差 {worst:e}"
    );
}

#[test]
fn the_remapped_noise_keeps_a_positive_lower_bound_on_the_surface() {
    let cloud = scene_cloud();
    let mut worst = f32::INFINITY;
    let mut worst_at = (0.0_f32, 0.0_f32);
    let mut found = 0_usize;
    for direction in scatter(512) {
        for cover in [0.35_f32, 0.5, 0.7, 0.9, 1.0] {
            for step in 1..=STEPS {
                let altitude = step as f32 / STEPS as f32;
                let (value, b) = field_and_b(&cloud, direction, altitude, cover);
                if value > TAU {
                    found += 1;
                    if b < worst {
                        worst = b;
                        worst_at = (cover, altitude);
                    }
                    break;
                }
            }
        }
    }
    assert!(found > 100, "命中点太少（{found}），这条判据没量到东西");

    let floor = (TAU / cloud.coverage_gain).powi(2);
    let slope = detail_curve_slope(worst);
    println!(
        "表面上的 b 下界：min b = {worst:.6}（cover {:.2}、高度 {:.4}）；理论下界 (τ/gain)² = {floor:.6}；\
         链式因子上界 1/(2√b) = {slope:.3}",
        worst_at.0, worst_at.1,
    );
    assert!(
        worst >= floor,
        "表面上出现了 b = {worst:e} < (τ/gain)² = {floor:e} ⇒ 链式因子会炸",
    );
    assert!(
        worst > 0.0 && slope <= cloud.coverage_gain / (2.0 * TAU) + 1e-3,
        "链式因子 {slope} 超过了 gain/(2τ) = {} 这条账",
        cloud.coverage_gain / (2.0 * TAU),
    );
}
