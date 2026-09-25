use px_field_schema::noise::Scalar;

pub fn lattice3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
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

pub fn cell_hash(seed: u32, cell: [i32; 3]) -> u32 {
    lattice3(cell[0], cell[1], cell[2], seed)
}

pub fn unit(hash: u32, channel: u32) -> f32 {
    let shift = (channel % 4) * 8;
    ((hash >> shift) & 0xff) as f32 / 255.0
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
        let gradient =
            GRADIENTS[(lattice3(ix + step_x, iy + step_y, iz + step_z, seed) % 12) as usize];
        let dot = S::from_f32(gradient[0]) * (tx - S::from_f32(offset[0]))
            + S::from_f32(gradient[1]) * (ty - S::from_f32(offset[1]))
            + S::from_f32(gradient[2]) * (tz - S::from_f32(offset[2]));
        let weight = if step_x == 1 {
            tx
        } else {
            S::from_f32(1.0) - tx
        } * if step_y == 1 {
            ty
        } else {
            S::from_f32(1.0) - ty
        } * if step_z == 1 {
            tz
        } else {
            S::from_f32(1.0) - tz
        };
        total = total + dot * weight;
    }
    (total * S::from_f32(0.9) + S::from_f32(0.5)).clamp01()
}

pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() <= f32::EPSILON {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn value_noise3(point: [f32; 3], seed: u32) -> f32 {
    let base = [
        point[0].floor() as i32,
        point[1].floor() as i32,
        point[2].floor() as i32,
    ];
    let frac = [
        point[0] - base[0] as f32,
        point[1] - base[1] as f32,
        point[2] - base[2] as f32,
    ];
    let w = [
        frac[0] * frac[0] * (3.0 - 2.0 * frac[0]),
        frac[1] * frac[1] * (3.0 - 2.0 * frac[1]),
        frac[2] * frac[2] * (3.0 - 2.0 * frac[2]),
    ];
    let corner = |dx: i32, dy: i32, dz: i32| -> f32 {
        unit(
            cell_hash(seed, [base[0] + dx, base[1] + dy, base[2] + dz]),
            0,
        )
    };
    let mut total = 0.0;
    for dz in 0..2 {
        for dy in 0..2 {
            for dx in 0..2 {
                let weight = (if dx == 1 { w[0] } else { 1.0 - w[0] })
                    * (if dy == 1 { w[1] } else { 1.0 - w[1] })
                    * (if dz == 1 { w[2] } else { 1.0 - w[2] });
                total += weight * corner(dx, dy, dz);
            }
        }
    }
    total
}

pub const NEIGHBOURS_3: [[i32; 3]; 27] = {
    let mut table = [[0_i32; 3]; 27];
    let mut i = 0;
    let mut dz = -1;
    while dz <= 1 {
        let mut dy = -1;
        while dy <= 1 {
            let mut dx = -1;
            while dx <= 1 {
                table[i] = [dx, dy, dz];
                i += 1;
                dx += 1;
            }
            dy += 1;
        }
        dz += 1;
    }
    table
};

pub const NEIGHBOURS_2: [[i32; 3]; 9] = {
    let mut table = [[0_i32; 3]; 9];
    let mut i = 0;
    let mut dy = -1;
    while dy <= 1 {
        let mut dx = -1;
        while dx <= 1 {
            table[i] = [dx, dy, 0];
            i += 1;
            dx += 1;
        }
        dy += 1;
    }
    table
};

pub fn neighbours(spherical: bool) -> &'static [[i32; 3]] {
    if spherical {
        &NEIGHBOURS_3
    } else {
        &NEIGHBOURS_2
    }
}

pub fn cell_of(point: [f32; 3]) -> [i32; 3] {
    [
        point[0].floor() as i32,
        point[1].floor() as i32,
        point[2].floor() as i32,
    ]
}

pub fn cell_centre(cell: [i32; 3]) -> [f32; 3] {
    [
        cell[0] as f32 + 0.5,
        cell[1] as f32 + 0.5,
        cell[2] as f32 + 0.5,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_a_pure_function_of_the_cell() {
        assert_eq!(cell_hash(7, [1, -2, 3]), cell_hash(7, [1, -2, 3]));
        assert_ne!(cell_hash(7, [1, -2, 3]), cell_hash(7, [1, -2, 4]));
        assert_ne!(cell_hash(7, [1, -2, 3]), cell_hash(8, [1, -2, 3]));
    }

    #[test]
    fn the_channels_of_the_hash_are_not_collinear() {
        let mut same_sign = 0;
        let mut total = 0;
        for x in 0..32 {
            for y in 0..32 {
                let hash = cell_hash(3, [x, y, 0]);
                let a = unit(hash, 0) - 0.5;
                let b = unit(hash, 1) - 0.5;
                if (a > 0.0) == (b > 0.0) {
                    same_sign += 1;
                }
                total += 1;
            }
        }
        let share = f64::from(same_sign) / f64::from(total);
        assert!(
            (0.35..0.65).contains(&share),
            "两个通道同号的占比 {share:.3} 不在「互相独立」的范围内"
        );
    }

    #[test]
    fn the_neighbourhood_tables_are_complete_and_distinct() {
        use std::collections::BTreeSet;
        let three: BTreeSet<[i32; 3]> = NEIGHBOURS_3.iter().copied().collect();
        assert_eq!(three.len(), 27, "27 邻域里有重复格");
        assert!(three.contains(&[0, 0, 0]), "27 邻域要含自己");
        let two: BTreeSet<[i32; 3]> = NEIGHBOURS_2.iter().copied().collect();
        assert_eq!(two.len(), 9, "9 邻域里有重复格");
        assert!(two.iter().all(|cell| cell[2] == 0), "平面档的 z 必须固定");
        assert_eq!(neighbours(true).len(), 27);
        assert_eq!(neighbours(false).len(), 9);
    }

    #[test]
    fn a_cell_centre_falls_back_into_its_own_cell() {
        for cell in [[0, 0, 0], [-3, 7, -1], [12, -5, 9]] {
            assert_eq!(cell_of(cell_centre(cell)), cell);
        }
        assert_eq!(cell_of([0.2, -0.2, 1.9]), [0, -1, 1]);
    }

    #[test]
    fn the_value_noise_is_bounded_continuous_and_aperiodic() {
        let mut worst_step = 0.0_f32;
        let mut previous = value_noise3([0.0, 0.0, 0.0], 5);
        for step in 1..2000 {
            let point = [step as f32 * 0.01, 1.7, -0.3];
            let value = value_noise3(point, 5);
            assert!((0.0..=1.0).contains(&value), "值噪声跑出 [0,1]：{value}");
            worst_step = worst_step.max((value - previous).abs());
            previous = value;
        }
        assert!(
            worst_step < 0.08,
            "步长 0.01 上的最大跳变 {worst_step} 太大（不连续？）"
        );
        let start = value_noise3([0.0, 1.7, -0.3], 5);
        let far = value_noise3([10.0, 1.7, -0.3], 5);
        assert!(
            (start - far).abs() > 1e-4,
            "大范围移动后回到同一个值（有周期？）"
        );
    }
}
