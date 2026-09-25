pub fn half_from_f32(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0 {
        return sign;
    }
    let half_exponent = exponent - 127 + 15;
    if half_exponent >= 0x1f {
        return sign | 0x7c00;
    }
    if half_exponent <= 0 {
        if half_exponent < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - half_exponent) as u32;
        let half_mantissa = mantissa >> shift;
        let round_bit = 1_u32 << (shift - 1);
        let rounded = half_mantissa + u32::from(mantissa & round_bit != 0);
        return sign | rounded as u16;
    }
    let half_mantissa = mantissa >> 13;
    let round_bit = 1_u32 << 12;
    let mut result = (half_exponent as u16) << 10 | half_mantissa as u16;
    if mantissa & round_bit != 0 {
        result = result.wrapping_add(1);
    }
    sign | result
}

pub fn f32_from_half(half: u16) -> f32 {
    let sign = if half & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
    let exponent = (half >> 10) & 0x1f;
    let mantissa = (half & 0x03ff) as f32;
    let magnitude = match exponent {
        0 => f32::from_bits(0x3380_0000) * mantissa / 1024.0,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn back(half: u16) -> f32 {
        let sign = if half & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = (half >> 10) & 0x1f;
        let mantissa = (half & 0x03ff) as f32;
        let magnitude = match exponent {
            0 => f32::from_bits(0x3380_0000) * mantissa / 1024.0,
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

    #[test]
    fn extremes_saturate_the_way_half_does() {
        assert!(back(half_from_f32(1.0e30)).is_infinite());
        assert!(back(half_from_f32(-1.0e30)).is_infinite());
        assert_eq!(back(half_from_f32(1.0e-30)), 0.0);
        assert_eq!(back(half_from_f32(0.0)), 0.0);
    }
}
