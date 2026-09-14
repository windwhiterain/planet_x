use std::ops::{Add, Div, Mul, Sub};

pub use num_dual::{Dual64, DualNum, first_derivative};

use crate::noise::Scalar;

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub struct Dual(pub Dual64);

impl Add for Dual {
    type Output = Dual;
    fn add(self, other: Dual) -> Dual {
        Dual(self.0 + other.0)
    }
}

impl Sub for Dual {
    type Output = Dual;
    fn sub(self, other: Dual) -> Dual {
        Dual(self.0 - other.0)
    }
}

impl Mul for Dual {
    type Output = Dual;
    fn mul(self, other: Dual) -> Dual {
        Dual(self.0 * other.0)
    }
}

impl Div for Dual {
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
        if self.0.re <= 0.0 {
            Dual(Dual64::from_re(0.0))
        } else if self.0.re >= 1.0 {
            Dual(Dual64::from_re(1.0))
        } else {
            self
        }
    }
    fn sqrt(self) -> Self {
        Dual(self.0.sqrt())
    }
    fn min_with(self, other: Self) -> Self {
        if self.0.re <= other.0.re { self } else { other }
    }
    fn max_with(self, other: Self) -> Self {
        if self.0.re >= other.0.re { self } else { other }
    }
}

impl Dual {
    pub fn constant(value: f64) -> Self {
        Dual(Dual64::from_re(value))
    }
    pub fn value(self) -> f64 {
        self.0.re
    }
}
