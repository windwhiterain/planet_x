pub use px_ops::noise::Scalar;

pub const GRADIENTS: [[f32; 3]; 12] = [
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

pub struct FbmSettings {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
}

fn easier<S: Scalar>(value: S) -> S {
    value * value * (S::from_f32(3.0) - value - value)
}

pub fn lattice_3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut hash = (x as u32).wrapping_mul(0x27d4_eb2d)
        ^ (y as u32).wrapping_mul(0x1656_67b1)
        ^ (z as u32).wrapping_mul(0x9e37_79b9)
        ^ seed;
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x2c1b_3c6d);
    hash ^= hash >> 12;
    hash = hash.wrapping_mul(0x297a_2d39);
    hash ^= hash >> 15;
    hash
}

pub fn gradient_noise_3<S: Scalar>(point: [S; 3], seed: u32) -> S {
    let base = [
        point[0].real().floor(),
        point[1].real().floor(),
        point[2].real().floor(),
    ];
    let cell = [base[0] as i32, base[1] as i32, base[2] as i32];
    let local = [
        point[0] - S::from_f32(base[0]),
        point[1] - S::from_f32(base[1]),
        point[2] - S::from_f32(base[2]),
    ];
    let weight = [easier(local[0]), easier(local[1]), easier(local[2])];

    let mut total = S::zero();
    for corner in 0..8_u32 {
        let step = [
            (corner & 1) as i32,
            ((corner >> 1) & 1) as i32,
            ((corner >> 2) & 1) as i32,
        ];
        let index = lattice_3(
            cell[0] + step[0],
            cell[1] + step[1],
            cell[2] + step[2],
            seed,
        ) % 12;
        let gradient = GRADIENTS[index as usize];
        let dot = S::from_f32(gradient[0]) * (local[0] - S::from_f32(step[0] as f32))
            + S::from_f32(gradient[1]) * (local[1] - S::from_f32(step[1] as f32))
            + S::from_f32(gradient[2]) * (local[2] - S::from_f32(step[2] as f32));
        let blend = [
            if step[0] == 1 {
                weight[0]
            } else {
                S::one() - weight[0]
            },
            if step[1] == 1 {
                weight[1]
            } else {
                S::one() - weight[1]
            },
            if step[2] == 1 {
                weight[2]
            } else {
                S::one() - weight[2]
            },
        ];
        total = total + dot * blend[0] * blend[1] * blend[2];
    }
    (total * S::from_f32(0.9) + S::from_f32(0.5)).clamp01()
}

pub fn fbm_3<S: Scalar>(point: [S; 3], settings: &FbmSettings) -> S {
    let mut total = S::zero();
    let mut amplitude = S::one();
    let mut normalization = S::zero();
    let mut frequency = settings.frequency;
    for octave in 0..settings.octaves {
        total = total
            + amplitude
                * gradient_noise_3(
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
