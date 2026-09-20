//! 场域算子的噪声算法（自写的值噪声 + 3D 梯度噪声）。
//!
//! `Scalar` / `FbmSettings` 住在 `px_field_schema`（约定），这里只放算法。

use px_field_schema::noise::{FbmSettings, Scalar};

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

// ⚠ 2026-09-20：格点哈希与值噪声**搬到了 `px_field_alg`**（实例库只链那一个 crate，
//   而"同一个格 → 同一个数"是全仓共用的一条约定）。这里 re-export 出去，调用点一字不改
//   （`crate::noise::cell_hash`）。`lattice3` 也一并转出去（本文件里的梯度噪声要用它）。
pub use px_field_alg::noise::{
    NEIGHBOURS_2, NEIGHBOURS_3, cell_centre, cell_hash, cell_of, faded_gradient_noise_3, lattice3,
    neighbours, unit, value_noise3,
};

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

// ---------------------------------------------------------------------------
// 元胞噪声（Worley / F1）—— `field.craters` 用的那一档
//
// ⚠ 与上面那两族（值噪声 / 梯度噪声）不是一回事：这里要的是"**到最近特征点的距离**"，
//   而特征点是**每个格子里一个、位置由哈希决定**的。它是陨坑、裂纹、鳞片这类
//   "以格点为骨架"的图形的底座。
//
// ⚠ 搜索范围是**27 邻域**（3D）/ **9 邻域**（2D）：特征点被 `jitter` 限制在自己的格子里
//   （`jitter ≤ 1`）时，最近点必定落在相邻格里 —— 所以调用方**先钳 jitter**（`craters` 那句
//   `jitter.clamp(0.0, 1.0)`）。`jitter > 1` 会让"本格的坑被邻居抢走"，图形上表现为
//   偶尔多出/少掉一个坑，而**不是**崩溃。
// ---------------------------------------------------------------------------

/// 一个格子的**特征点**（3D）：格子中心 + 由哈希决定的抖动偏移。
///
/// 三个偏移取自**同一个哈希的三个十位段**（`lattice3` 是 32 位混合）：于是同一个格子只算一次
/// 哈希，而三个轴互不相关。
fn feature_point_3(cell: [i32; 3], seed: u32, jitter: f32) -> [f32; 3] {
    let hash = lattice3(cell[0], cell[1], cell[2], seed);
    let offset = |shift: u32| ((hash >> shift) & 0x3ff) as f32 / 1024.0;
    [
        cell[0] as f32 + 0.5 + (offset(0) - 0.5) * jitter,
        cell[1] as f32 + 0.5 + (offset(10) - 0.5) * jitter,
        cell[2] as f32 + 0.5 + (offset(20) - 0.5) * jitter,
    ]
}

/// **3D 元胞距离（F1）**：到最近特征点的距离，单位是**格**（`1.0` = 一个格子）。
pub fn worley_3(point: [f32; 3], seed: u32, jitter: f32) -> f32 {
    let base = [
        point[0].floor() as i32,
        point[1].floor() as i32,
        point[2].floor() as i32,
    ];
    let mut best = f32::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let feature =
                    feature_point_3([base[0] + dx, base[1] + dy, base[2] + dz], seed, jitter);
                let distance = ((point[0] - feature[0]).powi(2)
                    + (point[1] - feature[1]).powi(2)
                    + (point[2] - feature[2]).powi(2))
                .sqrt();
                if distance < best {
                    best = distance;
                }
            }
        }
    }
    best
}

/// 一个格子的**特征点**（2D，平面档用）。
fn feature_point_2(cell: [i32; 2], seed: u32, jitter: f32) -> [f32; 2] {
    let x = lattice(cell[0], cell[1], seed);
    let y = lattice(cell[0], cell[1], seed ^ 0x9e37_79b9);
    [
        cell[0] as f32 + 0.5 + (x - 0.5) * jitter,
        cell[1] as f32 + 0.5 + (y - 0.5) * jitter,
    ]
}

/// **2D 元胞距离（F1）**：到最近特征点的距离，单位是**格**。
pub fn worley_2(point: [f32; 2], seed: u32, jitter: f32) -> f32 {
    let base = [point[0].floor() as i32, point[1].floor() as i32];
    let mut best = f32::MAX;
    for dy in -1..=1 {
        for dx in -1..=1 {
            let feature = feature_point_2([base[0] + dx, base[1] + dy], seed, jitter);
            let distance =
                ((point[0] - feature[0]).powi(2) + (point[1] - feature[1]).powi(2)).sqrt();
            if distance < best {
                best = distance;
            }
        }
    }
    best
}
