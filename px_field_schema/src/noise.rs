use std::ops::{Add, Div, Mul, Sub};

pub trait Scalar:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + PartialOrd
{
    fn from_f32(value: f32) -> Self;
    fn real(self) -> f32;
    fn clamp01(self) -> Self;
    fn sqrt(self) -> Self;
    fn zero() -> Self {
        Self::from_f32(0.0)
    }
    fn one() -> Self {
        Self::from_f32(1.0)
    }
    fn min_with(self, other: Self) -> Self {
        if self.real() <= other.real() {
            self
        } else {
            other
        }
    }
    fn max_with(self, other: Self) -> Self {
        if self.real() >= other.real() {
            self
        } else {
            other
        }
    }
}

impl Scalar for f32 {
    fn from_f32(value: f32) -> Self {
        value
    }
    fn real(self) -> f32 {
        self
    }
    fn clamp01(self) -> Self {
        self.clamp(0.0, 1.0)
    }
    fn sqrt(self) -> Self {
        f32::sqrt(self)
    }
    fn min_with(self, other: Self) -> Self {
        f32::min(self, other)
    }
    fn max_with(self, other: Self) -> Self {
        f32::max(self, other)
    }
}

pub struct FbmSettings {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
}
