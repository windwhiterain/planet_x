//! 烘图侧要自己算的那几个量：四元数、向量长度、场景整体的朝向。
//!
//! ⚠ 这里的**逐项次序照抄 glam**，不是"数学上等价"的另一种写法 —— 两者在 f32 下会差
//! 最后一位，而这一位会顺着 `looking_at` / `range_attenuation` 放大成亚像素抖动（实测：
//! 个别像素差 1~22 / 几十个像素差 1）。所以改这个文件之前先读这两处的注释。

/// 绕 X 轴的四元数。
pub fn quat_x(angle: f32) -> [f32; 4] {
    [(angle * 0.5).sin(), 0.0, 0.0, (angle * 0.5).cos()]
}

/// 绕 Y 轴的四元数。
pub fn quat_y(angle: f32) -> [f32; 4] {
    [0.0, (angle * 0.5).sin(), 0.0, (angle * 0.5).cos()]
}

/// 汉密尔顿积。**逐项次序照抄 glam 的 `Quat::mul`**：数学上等价的另一种写法
/// （把同一批项按别的顺序相加）在 f32 下会差最后一位 —— 差别会被 `looking_at` 放大成
/// 亚像素抖动，量出来就是"个别像素差 1~22"（迁移那一轮真踩过：1112 个像素）。
pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by + ay * bw + az * bx - ax * bz,
        aw * bz + az * bw + ax * by - ay * bx,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// 用四元数转一个向量。同样是 **glam `Quat::mul_vec3` 的逐项次序**
/// （`v(w²−b·b) + 2b(v·b) + 2w(b×v)`），不是常见的 `v + 2w(q×v) + 2q×(q×v)`。
pub fn rotate(quaternion: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let (x, y, z, w) = (quaternion[0], quaternion[1], quaternion[2], quaternion[3]);
    let (vx, vy, vz) = (vector[0], vector[1], vector[2]);
    let axis = [x, y, z];
    let square = x * x + y * y + z * z;
    let dot = vx * x + vy * y + vz * z;
    let cross = [y * vz - z * vy, z * vx - x * vz, x * vy - y * vx];
    let scale = w * w - square;
    let twice = dot * 2.0;
    let spin = w * 2.0;
    [
        vx * scale + axis[0] * twice + cross[0] * spin,
        vy * scale + axis[1] * twice + cross[1] * spin,
        vz * scale + axis[2] * twice + cross[2] * spin,
    ]
}

/// 场景整体的朝向：`rot_x(TILT) × rot_y(spin)` —— 与迁移前渲染器里那个父实体 +
/// 自转合成的一致（**只在这里出现一次**，渲染器里没有这个常数了）。
pub fn orientation(spin: f32, tilt: f32) -> [f32; 4] {
    quat_mul(quat_x(tilt), quat_y(spin))
}

/// 向量长度。**逐项次序照抄 glam 的 `Vec3::length`**：SSE2 那条路的横向加法是
/// `(x² + z²) + y²`，不是手写的 `(x² + y²) + z²` —— 两者在 f32 下会差最后一位，
/// 而这一位会顺着 `range_attenuation` 漏进光照，表现为几十个像素差 1。
pub fn length(vector: [f32; 3]) -> f32 {
    let (x, y, z) = (vector[0], vector[1], vector[2]);
    ((x * x + z * z) + y * y).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 朝向的合成顺序：`rot_x(TILT) × rot_y(spin)` —— 与迁移前渲染器那个
    /// 「父实体带倾斜 + 子实体带自转」合成出来的世界旋转是同一个四元数。
    #[test]
    fn the_world_orientation_is_tilt_then_spin() {
        let half = std::f32::consts::FRAC_PI_2;
        let composed = quat_mul(quat_x(half), quat_y(half));
        let rotated = rotate(composed, [0.0, 0.0, 1.0]);
        // 先绕 Y 转 90°（Z → X），再绕 X 转 90°（X 不动）⇒ 结果是 X 轴。
        assert!(
            (rotated[0] - 1.0).abs() < 1e-5
                && rotated[1].abs() < 1e-5
                && rotated[2].abs() < 1e-5,
            "合成朝向不对：{rotated:?}"
        );
        assert!((rotate(quat_x(0.0), [0.0, 1.0, 0.0])[1] - 1.0).abs() < 1e-6);
    }
}
