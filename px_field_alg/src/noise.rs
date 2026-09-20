//! **格点哈希与值噪声** —— 泛型实例**也**要用的那一档（2026-09-20 从 `px_field_op` 搬下来）。
//!
//! ⚠ 为什么搬家：这一档从前住在 `px_field_op`（预置算子的 dylib）里，而**泛型实例库只链
//!   [`px_field_alg`]**（见 `lib.rs` 的文件头）—— 于是实例里想"按格抽一个随机数"（放涡旋、
//!   打散周期性）就只能自己再写一份哈希。两份哈希迟早会分叉，而它们本来该是同一个东西：
//!   "同一个格 → 同一个数"是**全仓共用的一条约定**，不是某个算子的私事。
//!   搬下来之后 `px_field_op` 把它 re-export 出去（老调用点一字不改）。
//!
//! ⚠ 这一份**是算法**：改了它，吃场算子的节点键就换（名册吃这些源文件）。

/// 三维格点的 32 位哈希（`cell_hash` 的内核；梯度噪声也用同一个）。
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

/// **一个格子的 32 位哈希**（对外的那一扇门）—— 给"每格要抽好几个互相独立的随机数"的算子用。
///
/// ⚠ 第一个消费者是 `field.stamps`（盖章式打坑）：一个格子里要同时抽**位置**（三个十位段）、
///   **半径**、**年龄**、**遮罩硬币**四样东西，而它们必须互不相关（同一个位段既当年龄又当硬币，
///   画面里就会出现"年轻的更容易被盖上"这种系统性偏差）。⇒ 想要几样就调几次，`seed` 各不相同
///   （比如 `seed` 与 `seed ^ 0x51ed270b`）。
/// ⚠ 它是**纯函数**（同一格永远同一个数）：图重算两次逐字节相同、跨平台一致，也不需要存表。
pub fn cell_hash(seed: u32, cell: [i32; 3]) -> u32 {
    lattice3(cell[0], cell[1], cell[2], seed)
}

/// 把哈希的**第 `channel` 个 8 位段**映到 `[0, 1)`（`channel` 取 0..=3）。
///
/// ⚠ 一个 32 位哈希有四个独立的 8 位段：要几样互不相关的随机数就取几段（不要用同一个段变着法子
///   缩放 —— 那样抽出来的几样是**共线**的，画面里就是"大的坑总是深的"这一类看不见的相关性）。
pub fn unit(hash: u32, channel: u32) -> f32 {
    let shift = (channel % 4) * 8;
    ((hash >> shift) & 0xff) as f32 / 255.0
}

/// 梯度噪声用的 12 个方向（与 Ken Perlin 那套同形）。
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

/// **三维梯度噪声**（`[-1,1]` 的梯度点积 + 平滑权重，映到 `[0,1]`）。
///
/// ⚠ 与 [`value_noise3`] 的区别是**量化**的，不是"折线 vs 平滑"（2026-09-20 实测）：
///   两者都用同一个 `3t²−2t³` 平滑权重 ⇒ **都在格面上变平**。同一条扫描线上量到的斜率：
///   值噪声格面 `0.001` / 格中 `0.851`（压掉约 900 倍 ⇒ 看起来像一块块平台），
///   梯度噪声格面 `0.004` / 格中 `0.358`（压掉约 100 倍）。
///   ⇒ 想要"没有平台感"的起伏用这一档；只想要"便宜的低频扰动"用值噪声。
///   ⚠ 我原先在注释里写"值噪声在格面导数不连续"——**那是错的**（那是**线性**插值那版的性质），
///     已按实测改正。
/// ⚠ 泛型 `S: Scalar`：`px_verify` 的 `Dual` 也实现它 ⇒ 解析梯度那条路复用的就是这一份。
///   2026-09-20 从 `px_field_op` 搬下来（实例库只链 `px_field_alg`，要平滑噪声就得有这一档）。
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

/// **三维值噪声**（格点哈希 + 三线性插值）—— 给"要一条**不重复**的低频曲线"的场合。
///
/// ⚠ 它与正弦的区别正是它存在的理由：`sin(a·纬度 + b·上游)` 这类解析摆**在球面上会周期性重复**
///   （绕经度一圈回来是同一个样子，只是被"看不见的接缝"藏住了），而哈希值噪声不会 ——
///   同一个方向永远同一个值，但**没有任何周期**。
/// ⚠ 它在**格面上会变平**（平滑权重的必然结果，实测见 [`faded_gradient_noise_3`] 的注释）——
///   低频扰动看不出来，要"平台感更弱"的起伏就用梯度噪声那一档。
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
    // 平滑步（与梯度噪声同一档）：`3t² − 2t³`。
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

/// **一个格点周围的 27 个格子**（含自己）—— 三维足迹筛选用。
///
/// ⚠ 为什么要有这一份：`field.stamps`（盖章式打坑）与实例里的"放涡旋"都要"看一圈邻居"，
///   而这段循环从前是算子私有的 ⇒ 实例又要自己写一遍。**枚举邻域**与**取哈希**是同一族的词汇，
///   都该放在这一档里（这一档的意义就是"实例也能用的那份算法"）。
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

/// **平面档的 9 个邻居**（`z` 固定为 0）—— 平面档没有第三维，27 格里多出来的那 18 格够不着。
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

/// 按档位取邻域表（球面 27 / 平面 9）—— 调用点不必自己分支。
pub fn neighbours(spherical: bool) -> &'static [[i32; 3]] {
    if spherical {
        &NEIGHBOURS_3
    } else {
        &NEIGHBOURS_2
    }
}

/// **点 → 它落在哪个格**（`floor`）。⚠ 返回的是**格号**（`i32`），不是坐标。
pub fn cell_of(point: [f32; 3]) -> [i32; 3] {
    [
        point[0].floor() as i32,
        point[1].floor() as i32,
        point[2].floor() as i32,
    ]
}

/// **格号 → 格心**（`+0.5`）。⚠ 只对"点已经被某个频率缩放"的那套坐标有意义。
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

    /// **纯函数**：同一格两次同一个数；不同格/不同种子要不一样（不然"随机"是假的）。
    #[test]
    fn the_hash_is_a_pure_function_of_the_cell() {
        assert_eq!(cell_hash(7, [1, -2, 3]), cell_hash(7, [1, -2, 3]));
        assert_ne!(cell_hash(7, [1, -2, 3]), cell_hash(7, [1, -2, 4]));
        assert_ne!(cell_hash(7, [1, -2, 3]), cell_hash(8, [1, -2, 3]));
    }

    /// 四个 8 位段是**互相独立**的（"要几样抽几段"这条约定靠它）：
    /// 两个不同的 channel 不能高度相关 —— 这里用"同号的占比"当粗判据（独立时约一半）。
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

    /// 邻域表：27 / 9 格、**互不重复**、都含自己 —— 少一格就漏掉一个够得着的特征。
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

    /// `cell_of` / `cell_centre` 互逆（在格的内部）：格心一定落回同一个格号。
    #[test]
    fn a_cell_centre_falls_back_into_its_own_cell() {
        for cell in [[0, 0, 0], [-3, 7, -1], [12, -5, 9]] {
            assert_eq!(cell_of(cell_centre(cell)), cell);
        }
        assert_eq!(cell_of([0.2, -0.2, 1.9]), [0, -1, 1]);
    }

    /// **值噪声在 [0,1]、连续、无周期**：绕一圈（大范围）不会回到同一个值。
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
        // 相邻步长 0.01 上的跳变应当很小（连续），而一整圈之后不该复原。
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
