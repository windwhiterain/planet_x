pub fn quat_x(angle: f32) -> [f32; 4] {
    [(angle * 0.5).sin(), 0.0, 0.0, (angle * 0.5).cos()]
}

pub fn quat_y(angle: f32) -> [f32; 4] {
    [0.0, (angle * 0.5).sin(), 0.0, (angle * 0.5).cos()]
}

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

pub fn orientation(spin: f32, tilt: f32) -> [f32; 4] {
    quat_mul(quat_x(tilt), quat_y(spin))
}

pub fn length(vector: [f32; 3]) -> f32 {
    let (x, y, z) = (vector[0], vector[1], vector[2]);
    ((x * x + z * z) + y * y).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_orientation_is_tilt_then_spin() {
        let half = std::f32::consts::FRAC_PI_2;
        let composed = quat_mul(quat_x(half), quat_y(half));
        let rotated = rotate(composed, [0.0, 0.0, 1.0]);
        assert!(
            (rotated[0] - 1.0).abs() < 1e-5 && rotated[1].abs() < 1e-5 && rotated[2].abs() < 1e-5,
            "合成朝向不对：{rotated:?}"
        );
        assert!((rotate(quat_x(0.0), [0.0, 1.0, 0.0])[1] - 1.0).abs() < 1e-6);
    }
}
