use px_verify::cloud_field::CloudFieldParams;

const INNER: f32 = 1.01;
const OUTER: f32 = 1.06;
const MASK: f32 = 153.0 / 255.0;
const LIVE: f64 = 1e-3;

fn params() -> CloudFieldParams {
    CloudFieldParams {
        orientation: [0.0, 0.0, 0.0, 1.0],
        inner: INNER,
        outer: OUTER,
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

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn scatter(seed: u64, count: usize, inner: f64, outer: f64) -> Vec<[f32; 3]> {
    let mut rng = Rng(seed);
    (0..count)
        .map(|_| {
            let z = rng.next() * 2.0 - 1.0;
            let phi = rng.next() * std::f64::consts::TAU;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let radius = inner + (outer - inner) * rng.next();
            [
                (ring * phi.cos() * radius) as f32,
                (z * radius) as f32,
                (ring * phi.sin() * radius) as f32,
            ]
        })
        .collect()
}

fn lifted(point: [f32; 3]) -> [f64; 3] {
    [point[0] as f64, point[1] as f64, point[2] as f64]
}

fn numeric(field: &CloudFieldParams, point: [f32; 3], cover: f32, axis: usize, step: f64) -> f64 {
    let mut ahead = lifted(point);
    let mut behind = ahead;
    ahead[axis] += step;
    behind[axis] -= step;
    let plus = field.density(
        [ahead[0] as f32, ahead[1] as f32, ahead[2] as f32],
        cover,
    ) as f64;
    let minus = field.density(
        [behind[0] as f32, behind[1] as f32, behind[2] as f32],
        cover,
    ) as f64;
    (plus - minus) / (2.0 * step)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|one, two| one.partial_cmp(two).expect("误差里出现了 NaN"));
    values[values.len() / 2]
}

#[test]
fn the_dual_gradient_of_the_field_matches_the_fields_own_values() {
    let field = params();
    let cover = field.cover_from_mask(MASK);
    let steps = [2e-3_f64, 2e-4, 2e-5];

    let live: Vec<[f32; 3]> = scatter(0x9e37_79b9_7f4a_7c15, 4000, INNER as f64, OUTER as f64)
        .into_iter()
        .filter(|point| field.density(*point, cover) as f64 > LIVE)
        .collect();
    assert!(
        live.len() > 200,
        "落在云里的采样点只有 {} 个，这个测试没在测东西",
        live.len()
    );

    let mut medians = Vec::new();
    let mut magnitude = 0.0_f64;
    let mut worst = 0.0_f64;
    let mut worst_at = [0.0_f32; 3];
    let finest = steps[steps.len() - 1];

    for (slot, step) in steps.iter().enumerate() {
        let mut errors: Vec<f64> = Vec::new();
        for point in &live {
            let exact = field.gradient(lifted(*point), cover as f64);
            let mut error = 0.0_f64;
            let mut size = 0.0_f64;
            for axis in 0..3 {
                error = error.max((exact[axis] - numeric(&field, *point, cover, axis, *step)).abs());
                size = size.max(exact[axis].abs());
            }
            if slot == 0 {
                magnitude = magnitude.max(size);
            }
            if slot == steps.len() - 1 && error > worst {
                worst = error;
                worst_at = *point;
            }
            if size > 1e-9 {
                errors.push(error / size);
            }
        }
        assert!(
            errors.len() > 200,
            "步长 {step:e} 下可用于比对的点只有 {} 个",
            errors.len()
        );
        medians.push(median(&mut errors));
    }

    println!(
        "壳内点 {} 个，梯度量级 {magnitude:e}；中位相对偏差随步长：{medians:?}（最细步长 {finest:e}）",
        live.len()
    );
    assert!(
        magnitude > 1.0,
        "梯度量级只有 {magnitude}，这个测试没在测东西"
    );

    let best = medians.iter().fold(f64::MAX, |lowest, value| lowest.min(*value));
    assert!(
        medians[1] < medians[0] * 0.1,
        "偏差没有随步长缩小，这正说明公式错了而不是步长太大：各步长中位相对偏差 {medians:?}"
    );
    assert!(
        best < 1e-3,
        "最好的步长上中位相对偏差也只有 {best:e}（最坏点在 {worst_at:?}，绝对偏差 {worst:e}）"
    );
    assert!(
        medians[medians.len() - 1] < 1e-2,
        "最细步长 {:e} 上偏差反弹到 {:e}，比 h={:e} 差了 {:.1} 倍 ⇒ 差商已经落进 f32 的量化噪声，不是公式错",
        steps[steps.len() - 1],
        medians[medians.len() - 1],
        steps[1],
        medians[medians.len() - 1] / medians[1],
    );
}

#[test]
fn the_field_gradient_is_flat_outside_the_shell() {
    let field = params();
    let cover = field.cover_from_mask(MASK);
    let mut outside = 0_usize;
    let mut inside = 0_usize;

    for point in scatter(0x2545_f491_4f6c_dd1d, 2000, 0.5, 1.7) {
        let value = field.density(point, cover) as f64;
        let gradient = field.gradient(lifted(point), cover as f64);
        let size = gradient[0].abs().max(gradient[1].abs()).max(gradient[2].abs());
        if value <= 0.0 {
            assert_eq!(
                size, 0.0,
                "场值是 0 的点上梯度却不是 0：{gradient:?}（在 {point:?}）"
            );
            outside += 1;
        } else {
            inside += 1;
        }
    }

    println!("壳外 {outside} 个点梯度恰好为零；壳内 {inside} 个");
    assert!(outside > 100, "壳外的点只有 {outside} 个");
    assert!(inside > 50, "壳内的点只有 {inside} 个");
}
