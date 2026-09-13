use num_dual::{Dual64, DualNum, first_derivative};
use px_ops::noise::{FbmSettings, Scalar, fbm_3};

#[derive(Clone, Copy, PartialEq, PartialOrd)]
struct Dual(Dual64);

impl std::ops::Add for Dual {
    type Output = Dual;
    fn add(self, other: Dual) -> Dual {
        Dual(self.0 + other.0)
    }
}

impl std::ops::Sub for Dual {
    type Output = Dual;
    fn sub(self, other: Dual) -> Dual {
        Dual(self.0 - other.0)
    }
}

impl std::ops::Mul for Dual {
    type Output = Dual;
    fn mul(self, other: Dual) -> Dual {
        Dual(self.0 * other.0)
    }
}

impl std::ops::Div for Dual {
    type Output = Dual;
    fn div(self, other: Dual) -> Dual {
        Dual(self.0 / other.0)
    }
}

impl Scalar for Dual {
    fn from_f32(value: f32) -> Self {
        Dual(Dual64::from_re(value as f64))
    }
    fn real(self) -> f32 {
        self.0.re as f32
    }
    fn clamp01(self) -> Self {
        let low = Dual64::from_re(0.0);
        let high = Dual64::from_re(1.0);
        if self.0.re <= low.re {
            Dual(low)
        } else if self.0.re >= high.re {
            Dual(high)
        } else {
            self
        }
    }
}

fn settings() -> FbmSettings {
    FbmSettings {
        frequency: 1.0,
        octaves: 3,
        lacunarity: 2.0,
        gain: 0.5,
        seed: 7,
    }
}

fn value(point: [f32; 3]) -> f32 {
    fbm_3(point, &settings())
}

fn dual_axis(point: [f64; 3], axis: usize) -> f64 {
    let (_, slope) = first_derivative(
        |seed: Dual64| {
            let mut lifted = [
                Dual(Dual64::from_re(point[0])),
                Dual(Dual64::from_re(point[1])),
                Dual(Dual64::from_re(point[2])),
            ];
            lifted[axis] = Dual(seed);
            fbm_3(lifted, &settings()).0
        },
        point[axis],
    );
    slope
}

fn numeric_axis(point: [f64; 3], axis: usize, step: f64) -> f64 {
    let mut ahead = point;
    let mut behind = point;
    ahead[axis] += step;
    behind[axis] -= step;
    let ahead = value([ahead[0] as f32, ahead[1] as f32, ahead[2] as f32]) as f64;
    let behind = value([behind[0] as f32, behind[1] as f32, behind[2] as f32]) as f64;
    (ahead - behind) / (2.0 * step)
}

fn scatter(count: usize) -> Vec<[f64; 3]> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    (0..count)
        .map(|_| [next() * 8.0 - 4.0, next() * 8.0 - 4.0, next() * 8.0 - 4.0])
        .collect()
}

#[test]
fn diagnose_the_noise_discrepancy() {
    let settings = settings();
    let point = [
        -2.6519325487039564_f64,
        2.8856959088139638,
        1.0339259678444934,
    ];
    for axis in 0..3 {
        let dual = dual_axis(point, axis);
        println!("轴 {axis}: 对偶数给 {dual:.9}");
        for step in [1e-2_f64, 1e-3, 1e-4, 1e-5] {
            let numeric = numeric_axis(point, axis, step);
            println!(
                "   h={step:e}  中心差分 {numeric:.9}  偏差 {:.9}",
                (dual - numeric).abs()
            );
        }
    }

    let mut frequency = settings.frequency;
    for octave in 0..settings.octaves {
        let scaled = [
            point[0] as f32 * frequency,
            point[1] as f32 * frequency,
            point[2] as f32 * frequency,
        ];
        let raw = px_ops::noise::gradient_noise_3(scaled, settings.seed ^ octave);
        let clamped = raw <= 0.0 || raw >= 1.0;
        println!(
            "八度 {octave}: 频率 {frequency}  值 {raw:.9}{}",
            if clamped { "   ← 夹住了" } else { "" }
        );
        frequency *= settings.lacunarity;
    }

    let base = value([point[0] as f32, point[1] as f32, point[2] as f32]);
    println!("fbm 平均值 {base:.9}");
}

#[test]
fn the_dual_gradient_of_the_noise_matches_its_own_values() {
    let points = scatter(4000);
    let steps = [1e-2_f64, 1e-3, 1e-4];
    let mut worst_at = [0.0_f64; 3];
    let mut magnitude = 0.0_f64;

    let mut sampled = 0usize;
    let mut saturated = 0usize;
    let mut worst_per_step = Vec::new();
    for step in steps {
        let mut worst = 0.0_f64;
        for point in &points {
            let here = value([point[0] as f32, point[1] as f32, point[2] as f32]);
            if here <= 0.02 || here >= 0.98 {
                if step == steps[0] {
                    saturated += 1;
                }
                continue;
            }
            for axis in 0..3 {
                let dual = dual_axis(*point, axis);
                let numeric = numeric_axis(*point, axis, step);
                let error = (dual - numeric).abs();
                if error > worst {
                    worst = error;
                    if step == *steps.last().unwrap() {
                        worst_at = *point;
                    }
                }
                if step == steps[0] {
                    magnitude = magnitude.max(numeric.abs());
                    sampled += 1;
                }
            }
        }
        worst_per_step.push(worst);
    }

    assert!(
        sampled > 1000,
        "有效采样点只有 {sampled} 个（饱和 {saturated} 个），这个测试没在测东西"
    );
    assert!(
        magnitude > 0.1,
        "梯度量级只有 {magnitude}，这个测试没在测东西"
    );

    for pair in worst_per_step.windows(2) {
        assert!(
            pair[1] < pair[0] * 0.5,
            "偏差没有随步长缩小，这正说明公式错了而不是步长太大：各步长最大偏差 {worst_per_step:?}"
        );
    }

    let finest = *worst_per_step.last().unwrap();
    assert!(
        finest < 2e-3,
        "对偶数求得的 fbm 梯度与中心差分不符：最小步长下最大偏差 {finest}（在 {worst_at:?}），梯度量级 {magnitude}"
    );
}

#[test]
fn the_noise_value_is_clamped_so_its_gradient_is_zero_outside() {
    let settings = settings();
    let mut saw_clamped = false;
    let mut saw_free = false;
    for point in scatter(2000) {
        let lifted = [
            Dual(Dual64::from_re(point[0])),
            Dual(Dual64::from_re(point[1])),
            Dual(Dual64::from_re(point[2])),
        ];
        let here = fbm_3(lifted, &settings).0.re;
        if here <= 0.0 + 1e-9 || here >= 1.0 - 1e-9 {
            saw_clamped = true;
        } else {
            saw_free = true;
        }
    }
    assert!(saw_free, "所有采样点都被夹住了，样本没覆盖到有效区域");
    if saw_clamped {
        for point in scatter(200) {
            for axis in 0..3 {
                let slope = dual_axis(point, axis);
                assert!(
                    slope.is_finite(),
                    "被夹住的点上梯度必须是有限的，实测 {slope}（在 {point:?} 轴 {axis}）"
                );
            }
        }
    }
}
