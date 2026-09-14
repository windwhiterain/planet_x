use std::ops::{Add, Div, Mul, Sub};

pub trait Scalar:
    Copy + Add<Output = Self> + Sub<Output = Self> + Mul<Output = Self> + Div<Output = Self> + PartialOrd
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
        if self.real() <= other.real() { self } else { other }
    }
    fn max_with(self, other: Self) -> Self {
        if self.real() >= other.real() { self } else { other }
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

pub const fn fnv1a(bytes: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    let raw = bytes.as_bytes();
    while index < raw.len() {
        hash ^= raw[index] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        index += 1;
    }
    hash
}

fn lattice(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 0x00ff_ffff as f32
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

pub fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let tx = smooth(x - x0);
    let ty = smooth(y - y0);
    let (ix, iy) = (x0 as i32, y0 as i32);

    let c00 = lattice(ix, iy, seed);
    let c10 = lattice(ix + 1, iy, seed);
    let c01 = lattice(ix, iy + 1, seed);
    let c11 = lattice(ix + 1, iy + 1, seed);

    let top = c00 + (c10 - c00) * tx;
    let bottom = c01 + (c11 - c01) * tx;
    top + (bottom - top) * ty
}

pub struct FbmSettings {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
}

pub fn fbm(x: f32, y: f32, settings: &FbmSettings) -> f32 {
    let mut total = 0.0_f32;
    let mut amplitude = 1.0_f32;
    let mut normalization = 0.0_f32;
    let mut frequency = settings.frequency;
    for octave in 0..settings.octaves {
        total += amplitude * value_noise(x * frequency, y * frequency, settings.seed ^ octave);
        normalization += amplitude;
        amplitude *= settings.gain;
        frequency *= settings.lacunarity;
    }
    if normalization > 0.0 {
        total / normalization
    } else {
        0.0
    }
}

pub fn ridged(x: f32, y: f32, settings: &FbmSettings, sharpness: f32) -> f32 {
    let mut total = 0.0_f32;
    let mut amplitude = 1.0_f32;
    let mut normalization = 0.0_f32;
    let mut frequency = settings.frequency;
    for octave in 0..settings.octaves {
        let sample = value_noise(x * frequency, y * frequency, settings.seed ^ octave);
        let ridge = (1.0 - (sample * 2.0 - 1.0).abs()).powf(sharpness);
        total += amplitude * ridge;
        normalization += amplitude;
        amplitude *= settings.gain;
        frequency *= settings.lacunarity;
    }
    if normalization > 0.0 {
        total / normalization
    } else {
        0.0
    }
}

fn lattice3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d)
        ^ (y as u32).wrapping_mul(0x1656_67b1)
        ^ (z as u32).wrapping_mul(0x9e37_79b9)
        ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

const GRADIENTS: [[f32; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

fn smooth_scalar<S: Scalar>(t: S) -> S {
    t * t * (S::from_f32(3.0) - t - t)
}

pub fn faded_gradient_noise_3<S: Scalar>(point: [S; 3], seed: u32) -> S {
    let x0 = point[0].real().floor();
    let y0 = point[1].real().floor();
    let z0 = point[2].real().floor();
    let (ix, iy, iz) = (x0 as i32, y0 as i32, z0 as i32);
    let tx = smooth_scalar(point[0] - S::from_f32(x0));
    let ty = smooth_scalar(point[1] - S::from_f32(y0));
    let tz = smooth_scalar(point[2] - S::from_f32(z0));

    let mut total = S::zero();
    for corner in 0..8 {
        let step_x = (corner & 1) as i32;
        let step_y = (corner >> 1) as i32 & 1;
        let step_z = (corner >> 2) as i32 & 1;
        let offset = [step_x as f32, step_y as f32, step_z as f32];
        let gradient = GRADIENTS[(lattice3(ix + step_x, iy + step_y, iz + step_z, seed) % 12) as usize];
        let dot = S::from_f32(gradient[0]) * (tx - S::from_f32(offset[0]))
            + S::from_f32(gradient[1]) * (ty - S::from_f32(offset[1]))
            + S::from_f32(gradient[2]) * (tz - S::from_f32(offset[2]));
        let weight = if step_x == 1 { tx } else { S::from_f32(1.0) - tx }
            * if step_y == 1 { ty } else { S::from_f32(1.0) - ty }
            * if step_z == 1 { tz } else { S::from_f32(1.0) - tz };
        total = total + dot * weight;
    }
    (total * S::from_f32(0.9) + S::from_f32(0.5)).clamp01()
}

pub fn direction(u: f32, v: f32) -> [f32; 3] {
    let theta = v.clamp(0.0, 1.0) * std::f32::consts::PI;
    let phi = u * std::f32::consts::TAU;
    let ring = theta.sin();
    [ring * phi.cos(), theta.cos(), ring * phi.sin()]
}

pub fn fbm_3<S: Scalar>(point: [S; 3], settings: &FbmSettings) -> S {
    let mut total = S::zero();
    let mut amplitude = S::from_f32(1.0);
    let mut normalization = S::zero();
    let mut frequency = settings.frequency;
    for octave in 0..settings.octaves {
        total = total
            + amplitude
                * faded_gradient_noise_3(
                    [
                        point[0] * S::from_f32(frequency),
                        point[1] * S::from_f32(frequency),
                        point[2] * S::from_f32(frequency),
                    ],
                    settings.seed ^ octave,
                );
        normalization = normalization + amplitude;
        amplitude = amplitude * S::from_f32(settings.gain);
        frequency *= settings.lacunarity;
    }
    if normalization > S::zero() {
        total / normalization
    } else {
        S::zero()
    }
}

pub fn ridged_3(point: [f32; 3], settings: &FbmSettings, sharpness: f32) -> f32 {
    let mut total = 0.0_f32;
    let mut amplitude = 1.0_f32;
    let mut normalization = 0.0_f32;
    let mut frequency = settings.frequency;
    for octave in 0..settings.octaves {
        let sample = faded_gradient_noise_3(
            [
                point[0] * frequency,
                point[1] * frequency,
                point[2] * frequency,
            ],
            settings.seed ^ octave,
        );
        let ridge = (1.0 - (sample * 2.0 - 1.0).abs()).powf(sharpness);
        total += amplitude * ridge;
        normalization += amplitude;
        amplitude *= settings.gain;
        frequency *= settings.lacunarity;
    }
    if normalization > 0.0 {
        total / normalization
    } else {
        0.0
    }
}
