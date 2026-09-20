//! **格点哈希与值噪声** —— 泛型实例**也**要用的那一档（2026-09-20 从 `px_field_op` 搬下来）。
//!
//! ⚠ 为什么搬家：这一档从前住在 `px_field_op`（预置算子的 dylib）里，而**泛型实例库只链
//!   [`px_field_alg`]**（见 `lib.rs` 的文件头）—— 于是实例里想"按格抽一个随机数"（放涡旋、
//!   打散周期性）就只能自己再写一份哈希。两份哈希迟早会分叉，而它们本来该是同一个东西：
//!   "同一个格 → 同一个数"是**全仓共用的一条约定**，不是某个算子的私事。
//!   搬下来之后 `px_field_op` 把它 re-export 出去（老调用点一字不改）。
//!
//! ⚠ 这一份**是算法**：进了场域那名册 ⇒ 改它要重登记（与 `remap.rs` 同一条规矩）。

/// 三维格点的 32 位哈希（`cell_hash` 的内核；梯度噪声也用同一个）。
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

/// **三维值噪声**（格点哈希 + 三线性插值）—— 给"要一条**不重复**的低频曲线"的场合。
///
/// ⚠ 它与正弦的区别正是它存在的理由：`sin(a·纬度 + b·上游)` 这类解析摆**在球面上会周期性重复**
///   （绕经度一圈回来是同一个样子，只是被"看不见的接缝"藏住了），而哈希值噪声不会 ——
///   同一个方向永远同一个值，但**没有任何周期**。
/// ⚠ 值噪声的导数不连续（格点边界上有折线感）；要平滑的梯度噪声用 `px_field_op` 那一档
///   （它不能进实例库：那边还带着梯度表与 12 个方向的常量）。
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
