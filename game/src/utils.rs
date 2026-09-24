#[cfg(test)]
mod tests;

/// 对数尺度的数值边界，**不是策略边界**。
///
/// 取 `ln(f32::MAX) / 2 ≈ 44.36` 的一半，理由只有一条：`share × depth` 这类
/// **两项相乘**的结果必须还留在 f32 里。它说的是"这台机器能表示到哪"，
/// 不是"报价最多几倍"或"楔子最多偏离多少"——后者是模型假设，必须另有理由。
pub const LOG_LIMIT: f32 = 44.0;

pub fn conditional_swap<T>(a: T, b: T, condition: bool) -> (T, T) {
    if condition { (b, a) } else { (a, b) }
}

pub fn similarity(a: f32, b: f32) -> f32 {
    2.0 * a * b / (a * a + b * b)
}

pub fn conditional_signature(condition: bool) -> f32 {
    if condition { 1.0 } else { -1.0 }
}

pub fn geometric_average(a: f32, b: f32) -> f32 {
    f32::sqrt((a * b).abs())
}

pub fn normal_cdf(z: f32) -> f32 {
    if z.is_nan() {
        return 0.0;
    }
    if z >= 8.0 {
        return 1.0;
    }
    if z <= -8.0 {
        return 0.0;
    }
    let x = z * std::f32::consts::FRAC_1_SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
    let polynomial = ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t
        + 0.254829592)
        * t;
    let erf = if x < 0.0 {
        polynomial * (-x * x).exp() - 1.0
    } else {
        1.0 - polynomial * (-x * x).exp()
    };
    (0.5 * (1.0 + erf)).clamp(0.0, 1.0)
}
