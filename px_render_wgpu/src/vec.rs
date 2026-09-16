//! 一点点向量数学。**故意不用 `glam`。**
//!
//! 理由只有一条，但是实测过的：`glam` 是单态化大户，§92 的冷编账里**它一个人就值 11.5 s**
//! （那是剥离 bevy 要省的那笔账里最大的单项之一），而本渲染器真正用到的只有
//! `dot` / `cross` / `normalize` 这几个算符。用户口径（§100）："能动态的就动态、
//! 减少类型检查与单态化的时间"。
//!
//! ⚠ 但**算式必须与 glam 逐位一致**，否则 `weld_normals` 焊出来的法线会在最后一位上
//! 与 Bevy 分岔，而判据是逐字节的。下面每一个函数都是从 `glam 0.32.1` 的
//! `src/f32/vec3.rs` **逐字抄**下来的（连括号的结合顺序都照抄）：
//!
//! - `dot`  = `(x*x') + (y*y') + (z*z')`（左结合，不是先加后两项）
//! - `length` = `sqrt(dot(self, self))`
//! - `length_recip` = `1.0 / length()`（`length().recip()`）
//! - `normalize_or_zero` = `rcp = length_recip(); if rcp.is_finite() && rcp > 0.0 { self * rcp } else { ZERO }`
//!
//! 少抄一个括号就是另一条法线 —— 这正是 §104 第 1 条那种"看着一样、哈希不一样"的来源。

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const X: Vec3 = Vec3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    pub const Y: Vec3 = Vec3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    pub const Z: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    pub const fn new(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3 { x, y, z }
    }

    pub const fn splat(value: f32) -> Vec3 {
        Vec3 {
            x: value,
            y: value,
            z: value,
        }
    }

    pub const fn from_array(value: [f32; 3]) -> Vec3 {
        Vec3 {
            x: value[0],
            y: value[1],
            z: value[2],
        }
    }

    pub const fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    /// 逐字抄 glam：`(x*x') + (y*y') + (z*z')`，**左结合**。
    pub fn dot(self, rhs: Vec3) -> f32 {
        (self.x * rhs.x) + (self.y * rhs.y) + (self.z * rhs.z)
    }

    /// 逐字抄 glam：分量相减的次序也一样（`self.y * rhs.z - rhs.y * self.z`）。
    pub fn cross(self, rhs: Vec3) -> Vec3 {
        Vec3 {
            x: self.y * rhs.z - rhs.y * self.z,
            y: self.z * rhs.x - rhs.z * self.x,
            z: self.x * rhs.y - rhs.x * self.y,
        }
    }

    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn length_recip(self) -> f32 {
        self.length().recip()
    }

    /// glam 的 `normalize()`：**不判零**，直接乘 `length_recip`。
    pub fn normalize(self) -> Vec3 {
        self * self.length_recip()
    }

    /// glam 的 `normalize_or_zero()` ⇒ `normalize_or(ZERO)`，判定条件是
    /// `rcp.is_finite() && rcp > 0.0`（注意不是拿长度判，是拿**倒数**判）。
    pub fn normalize_or_zero(self) -> Vec3 {
        let rcp = self.length_recip();
        if rcp.is_finite() && rcp > 0.0 {
            self * rcp
        } else {
            Vec3::ZERO
        }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, rhs: Vec3) -> Vec3 {
        Vec3 {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
            z: self.z + rhs.z,
        }
    }
}

impl AddAssign for Vec3 {
    fn add_assign(&mut self, rhs: Vec3) {
        *self = *self + rhs;
    }
}

impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, rhs: Vec3) -> Vec3 {
        Vec3 {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
            z: self.z - rhs.z,
        }
    }
}

impl Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, rhs: f32) -> Vec3 {
        Vec3 {
            x: self.x * rhs,
            y: self.y * rhs,
            z: self.z * rhs,
        }
    }
}

impl Div<f32> for Vec3 {
    type Output = Vec3;
    fn div(self, rhs: f32) -> Vec3 {
        Vec3 {
            x: self.x / rhs,
            y: self.y / rhs,
            z: self.z / rhs,
        }
    }
}

impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3 {
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这几条钉的是"与 glam 同一位"：数值取自 glam 自己那套算式手算的结果，
    /// 任何一处括号/次序改动都会让它们变。
    #[test]
    fn the_arithmetic_matches_glam_bit_for_bit() {
        let a = Vec3::new(1.5, -2.25, 0.125);
        let b = Vec3::new(-0.75, 3.5, 2.0);
        assert_eq!(a.dot(b), (1.5 * -0.75) + (-2.25 * 3.5) + (0.125 * 2.0));
        assert_eq!(a.cross(b).x, a.y * b.z - b.y * a.z);
        assert_eq!(a.cross(b).y, a.z * b.x - b.z * a.x);
        assert_eq!(a.cross(b).z, a.x * b.y - b.x * a.y);
        assert_eq!(a.length(), a.dot(a).sqrt());
        assert_eq!(a.length_recip(), a.length().recip());
        assert_eq!(a.normalize(), a * a.length().recip());
        assert_eq!(Vec3::ZERO.normalize_or_zero(), Vec3::ZERO);
        assert_eq!(
            Vec3::new(f32::NAN, 0.0, 0.0).normalize_or_zero(),
            Vec3::ZERO,
            "非有限 ⇒ 兜底零（判的是倒数，不是长度）"
        );
    }

    #[test]
    fn add_and_scale_are_component_wise() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(a + a, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(a * 2.0, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(-a, Vec3::new(-1.0, -2.0, -3.0));
    }
}
