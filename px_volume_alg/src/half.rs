//! **f32 → 半精度（binary16）**的位级转换。
//!
//! ⚠ **为什么这里自己写一份**（而不是用 `px_graph::generate::shade::half_from_f32`）：
//!   依赖方向不允许 —— `px_volume_alg` 在 `px_graph` **下面**（图驱动依赖算法）。
//!   而天空贴图是"算法直接交出成品贴图"的第一档，转换必须住在算法这一侧。
//!
//! ⚠ 这份**不追求与那一份逐位相同**（那份带它自己的舍入/夹取口径）：它只要满足
//!   "写进去再读出来是同一个数"到半精度的极限即可。真要对齐两个口径，
//!   唯一可靠的办法是拿同一批值逐个比对 —— 今天没有这个需求（贴图只有这一档用）。

/// f32 → binary16 的位模式。
///
/// 规则：符号 + 指数（偏置 15）+ 10 位尾数；溢出到 ±inf；太小落到 0（舍入到最近的偶数）。
pub fn half_from_f32(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    // 指数为 0 ⇒ 零或非规格化 f32：数量级比半精度能表示的最小值还小得多 ⇒ 归零。
    if exponent == 0 {
        return sign;
    }
    // 指数的偏置从 127 换到 15。
    let half_exponent = exponent - 127 + 15;
    // 溢出（半精度的最大指数是 30）⇒ ±inf。
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    // 下溢到半精度的非规格化区（或 0）。
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        // 把隐含的那个 1 补上再右移，并对齐到非规格化的 10 位。
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - half_exponent) as u32;
        let half_mantissa = mantissa >> shift;
        // 舍入到最近（看被移掉的那一位）。
        let round_bit = 1_u32 << (shift - 1);
        let rounded = half_mantissa + u32::from(mantissa & round_bit != 0);
        return sign | rounded as u16;
    }
    // 常规：10 位尾数 + 舍入到最近。
    let half_mantissa = mantissa >> 13;
    let round_bit = 1_u32 << 12;
    let mut result = (half_exponent as u16) << 10 | half_mantissa as u16;
    if mantissa & round_bit != 0 {
        result = result.wrapping_add(1);
    }
    sign | result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 半精度位模式 → f32。**独立写一遍**（不借被测那个方向的代码）：
    /// 用同一个函数来回转换会让"两个方向一起错"看起来像通过。
    ///
    /// ⚠ 不用标准库的 `f16`（今天还是 unstable）。
    fn back(half: u16) -> f32 {
        let sign = if half & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = (half >> 10) & 0x1f;
        let mantissa = (half & 0x03ff) as f32;
        let magnitude = match exponent {
            0 => f32::from_bits(0x3380_0000) * mantissa / 1024.0, // 2^-24 × m/1024
            0x1f => {
                if mantissa == 0.0 {
                    f32::INFINITY
                } else {
                    f32::NAN
                }
            }
            _ => {
                let scale = f32::from_bits(((exponent as u32) + 112) << 23);
                scale * (1.0 + mantissa / 1024.0)
            }
        };
        sign * magnitude
    }

    /// **常见值往返**：贴图里出现的那一档（0 附近与 1 附近）必须准。
    #[test]
    fn ordinary_values_survive_the_round_trip() {
        for value in [
            0.0_f32, 1.0, 0.5, 2.0, 0.25, 0.125, -1.0, -0.5, 10.0, 0.03125,
        ] {
            let got = back(half_from_f32(value));
            assert!(
                (got - value).abs() <= value.abs() * 1e-3 + 1e-6,
                "{value} 转成半精度再读回来是 {got}"
            );
        }
    }

    /// **几个精确值**：半精度能**逐位精确**表示的数必须逐位相等（不是"差不多"）。
    ///
    /// ⚠ 这条比上一条强：上一条容 1e-3，任何"指数偏一档但数量级对"的实现都能混过去；
    ///   这一条只放行真正的位级正确。
    #[test]
    fn exactly_representable_values_come_back_bit_for_bit() {
        for (value, expected) in [
            (1.0_f32, 0x3c00_u16),
            (0.5, 0x3800),
            (2.0, 0x4000),
            (-1.0, 0xbc00),
            (0.0, 0x0000),
        ] {
            let bits = half_from_f32(value);
            assert_eq!(
                bits, expected,
                "{value} 的半精度位模式应当是 {expected:#06x}，实际 {bits:#06x}"
            );
            assert_eq!(back(bits), value, "{value} 读回来应当逐位相等");
        }
    }

    /// **量级单调**：从 0 到 20 一路上去，读回来必须单调不减。
    #[test]
    fn the_conversion_is_monotonic() {
        let mut previous = f32::NEG_INFINITY;
        let mut value = 0.0_f32;
        while value < 20.0 {
            let got = back(half_from_f32(value));
            assert!(
                got >= previous,
                "{value} 读回来是 {got}，比上一个 {previous} 还小（不单调）"
            );
            previous = got;
            value += 0.017;
        }
    }

    /// **溢出给 inf、极小给 0**（不静默变成别的数）。
    #[test]
    fn extremes_saturate_the_way_half_does() {
        assert!(back(half_from_f32(1.0e30)).is_infinite());
        assert!(back(half_from_f32(-1.0e30)).is_infinite());
        assert_eq!(back(half_from_f32(1.0e-30)), 0.0);
        assert_eq!(back(half_from_f32(0.0)), 0.0);
    }
}
