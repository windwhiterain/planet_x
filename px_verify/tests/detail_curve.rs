//! 细节噪声那次凹重映射（`sqrt`）的两笔账 —— 都不依赖 GPU 探针：
//!
//! 1. **链式法则**：重映射写在 `billows` 里，`shape_of` 的噪声偏导就得跟着乘
//!    `detail_curve_slope(b)`。这里把参考场的解析梯度（`Dual`）与中心差商对齐，
//!    再把 `detail_curve_slope` 自己与 `detail_curve` 的差商对齐 —— 因子抄错就会露馅。
//! 2. **`b` 的正下界**：shader 的解析梯度用的也是同一个因子，若表面上 `b → 0` 那么
//!    `1/(2√b)` 会把法线打炸。表面上场 > τ ⇒ `√b > τ / coverage_gain` ⇒ `b > (τ/gain)²`；
//!    这里按 shader 的栅格（壳入射点 + i/steps，绝对锚）实测那个最小值。
//!
//! ⚠ 这两条管的是**公式**。WGSL 文本里有没有真的把因子乘上去，由
//! `px_render/tests/cloud_field.rs` 的文本门管（那份是组装后的 shader 原文）。

use px_verify::cloud_field::{CloudFieldParams, detail_curve, detail_curve_slope};

/// 场景里那一档云（`art/scene/orbit-surface.toml` 的 clouds part）。
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

/// 与 `px_graphs::cloud_proxy::scatter_directions` 同一套 xorshift（不引 rand）。
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

/// 壳里某个高度上的世界点。
fn point_at(direction: [f32; 3], altitude: f32, cloud: &CloudFieldParams) -> [f32; 3] {
    let radius = cloud.inner + altitude * cloud.span();
    [
        direction[0] * radius,
        direction[1] * radius,
        direction[2] * radius,
    ]
}

/// 场值（= shader `cloud_field` 里那一串）与重映射前的 `b`：`b = noise²`，
/// 因为 `noise = √b`。
fn field_and_b(cloud: &CloudFieldParams, direction: [f32; 3], altitude: f32, cover: f32) -> (f32, f32) {
    let point = point_at(direction, altitude, cloud);
    let medium = cloud.medium_of(point);
    let noise = cloud.billows(medium.direction, medium.altitude);
    let value = cloud.shape(cover, medium.altitude, noise);
    (value, noise * noise)
}

#[test]
fn the_slope_helper_is_the_derivative_of_the_remap() {
    // shader 的解析梯度乘的就是这个数：它必须等于 d√b/db，差一个因子就是错的法线。
    let mut worst = 0.0_f32;
    for b in [0.006, 0.02, 0.1, 0.35, 0.62, 0.9, 1.0] {
        let h = 1e-4_f32;
        let numeric = (detail_curve(b + h) - detail_curve(b - h)) / (2.0 * h);
        let analytic = detail_curve_slope(b);
        worst = worst.max((numeric - analytic).abs() / analytic.max(1e-6));
    }
    assert!(worst < 1e-2, "detail_curve_slope 与 d√b/db 差得太远：{worst:e}");
}

#[test]
fn the_reference_gradient_follows_the_remap_chain_rule() {
    // 参考场加了凹重映射之后，`Dual` 这条路必须自动带上链式法则（`sqrt` 的导数）。
    // 抽查壳里贴着阈值的那一层（shader 真正会去做梯度的位置）：重映射漏在值路径上、
    // 或者差了个因子，都会在这里露馅。
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
            let size = (analytic[0] * analytic[0] + analytic[1] * analytic[1] + analytic[2] * analytic[2])
                .sqrt()
                .max(1.0) as f32;
            // 差商步长取几档取最小的那个：h 大了会跨过 clamp 的折点，小了被 f32 的舍入盖住。
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
    assert!(sampled > 20, "贴着 τ 的采样点太少（{sampled} 个），这条判据没量到东西");
    assert!(
        worst < 5e-2,
        "解析梯度（Dual，含 sqrt 链式法则）与中心差商不一致：最差相对差 {worst:e}"
    );
}

#[test]
fn the_remapped_noise_keeps_a_positive_lower_bound_on_the_surface() {
    // 表面上场 > τ ⇒ b 有正下界 ⇒ 解析梯度里的 1/(2√b) 有上界。
    // 栅格与 shader 同锚（相机在壳外 ⇒ 入射点高度 0，采样点在 i/steps）。
    let cloud = scene_cloud();
    let mut worst = f32::INFINITY;
    let mut worst_at = (0.0_f32, 0.0_f32);
    let mut found = 0_usize;
    for direction in scatter(512) {
        // 覆盖度取几档扫一遍：footprint ≤ cover，所以 cover = 1 是最坏那一档。
        for cover in [0.35_f32, 0.5, 0.7, 0.9, 1.0] {
            for step in 1..=STEPS {
                let altitude = step as f32 / STEPS as f32;
                let (value, b) = field_and_b(&cloud, direction, altitude, cover);
                // 「命中」= 第一个超过阈值的采样点，正是 shader 做梯度的那个点。
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

    // 理论上界：√b > τ / coverage_gain ⇒ b > (τ/gain)²。
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
