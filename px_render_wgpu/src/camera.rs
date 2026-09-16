//! 探针相机：**逐位**复刻 Bevy 那条路造出来的 `world_from_view` / `clip_from_world`。
//!
//! 语义照抄，不做"改进"：
//!
//! - `looking_at(target, up)` = `look_to(target - translation, up)`（`bevy_transform-0.19.1`
//!   `src/components/transform.rs:462`）。
//! - `look_to(direction, up)`（同文件 `:475`）：
//!   `back = -normalize(direction)`；`right = normalize(up.cross(back))`；
//!   `up2 = back.cross(right)`；`rotation = Quat::from_mat3(&Mat3::from_cols(right, up2, back))`。
//! - `world_from_view = Mat4::from_scale_rotation_translation(scale, rotation, translation)`
//!   （glam 在 x86_64 走 SSE2：`src/f32/sse2/mat4.rs:226`，等价于 `Affine3A` 那条）。
//! - `clip_from_view = Mat4::perspective_infinite_reverse_rh(PI/4, aspect, 0.1)`。
//! - `view_from_world = world_from_view.inverse()`；
//!   `clip_from_world = clip_from_view.mul_mat4(&view_from_world)`。
//!
//! ⚠ `Mat3 → Quat → Mat3` **不是**逐位恒等（4 个相机位姿实测过），所以两个方向都必须是
//! 忠实转写；这里 `from_mat3` 抄的是 Shepperd 分支，`from_quat` 抄的是 `quat_to_axes`。

#![allow(dead_code)]

use crate::mat4::{Mat3, Mat4, Quat, Vec3};

pub struct Camera {
    pub position: Vec3,
    pub world_from_view: Mat4,
    pub view_from_world: Mat4,
    pub clip_from_view: Mat4,
    pub clip_from_world: Mat4,
}

/// `None` = 命令行没给 `--cam`（固定的探针机位）；`Some([yaw, pitch, distance])` 是那条路。
pub fn probe_camera(cam: Option<[f32; 3]>, aspect: f32) -> Camera {
    let (translation, rotation) = match cam {
        None => {
            let translation = Vec3::new(0.0, 0.55, 3.15);
            let rotation = looking_at(translation, Vec3::ZERO, Vec3::Y);
            (translation, rotation)
        }
        Some([yaw, pitch, distance]) => {
            let yaw = yaw.to_radians();
            let pitch = pitch.clamp(-89.5, 89.5).to_radians();
            let direction = Vec3::new(
                pitch.cos() * yaw.sin(),
                pitch.sin(),
                pitch.cos() * yaw.cos(),
            );
            let translation = direction * distance;
            let rotation = looking_at(translation, Vec3::ZERO, Vec3::Y);
            (translation, rotation)
        }
    };

    let world_from_view =
        Mat4::from_scale_rotation_translation(Vec3::splat(1.0), rotation, translation);
    let clip_from_view =
        Mat4::perspective_infinite_reverse_rh(core::f32::consts::PI / 4.0, aspect, 0.1);
    let view_from_world = world_from_view.inverse();
    let clip_from_world = clip_from_view.mul_mat4(&view_from_world);

    Camera {
        position: translation,
        world_from_view,
        view_from_world,
        clip_from_view,
        clip_from_world,
    }
}

/// Bevy `Transform::look_at` -> `look_to`。
fn looking_at(translation: Vec3, target: Vec3, up: Vec3) -> Quat {
    look_to(target - translation, up)
}

/// `bevy_transform-0.19.1` `src/components/transform.rs:475-484`。
///
/// ⚠ `direction.try_into()` / `up.try_into()` 走的是 `Dir3::new` = `value / length`
/// （`bevy_math-0.19.1/src/direction.rs:587-594`，**除法**），而**不是** glam 的
/// `try_normalize`（乘 `1/length`）—— 这两条差 1 个 ulp，`right` 用的是后者。
fn look_to(direction: Vec3, up: Vec3) -> Quat {
    let back = -dir3(direction, Vec3::Z);
    let up = dir3(up, Vec3::Y);
    let right = up.cross(back).try_normalize().unwrap_or(Vec3::X);
    let up = back.cross(right);
    Quat::from_mat3(&Mat3::from_cols(right, up, back))
}

/// `bevy_math-0.19.1/src/direction.rs:563-594`：`value / length`；失败时退到 `fallback`。
fn dir3(value: Vec3, fallback: Vec3) -> Vec3 {
    let length = value.length();
    if length.is_finite() && length > 0.0 {
        Vec3::new(value.x / length, value.y / length, value.z / length)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::probe_camera;
    use crate::mat4::Mat4;

    fn bits(m: &Mat4) -> [u32; 16] {
        let c = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
        let mut out = [0u32; 16];
        for i in 0..4 {
            out[i * 4] = c[i].x.to_bits();
            out[i * 4 + 1] = c[i].y.to_bits();
            out[i * 4 + 2] = c[i].z.to_bits();
            out[i * 4 + 3] = c[i].w.to_bits();
        }
        out
    }

    const WORLD: [u32; 16] = [
        0x3F80_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000,
        0x0000_0000, 0x3F7C_2F4D, 0xBE30_2108, 0x0000_0000,
        0x0000_0000, 0x3E30_2108, 0x3F7C_2F4D, 0x0000_0000,
        0x0000_0000, 0x3F0C_CCCD, 0x4049_999A, 0x3F80_0000,
    ];

    const CLIP: [u32; 16] = [
        0x3FCE_034C, 0x0000_0000, 0x0000_0000, 0x0000_0000,
        0x0000_0000, 0x401A_8279, 0x0000_0000, 0x0000_0000,
        0x0000_0000, 0x0000_0000, 0x0000_0000, 0xBF80_0000,
        0x0000_0000, 0x0000_0000, 0x3DCC_CCCD, 0x0000_0000,
    ];

    #[test]
    fn the_probe_camera_matches_bevy_bit_for_bit() {
        let aspect = 960.0f32 / 640.0f32;
        assert_eq!(aspect.to_bits(), 0x3FC0_0000, "aspect bits");

        let cam = probe_camera(None, aspect);

        assert_eq!(cam.position.x.to_bits(), 0x0000_0000, "position.x");
        assert_eq!(cam.position.y.to_bits(), 0x3F0C_CCCD, "position.y");
        assert_eq!(cam.position.z.to_bits(), 0x4049_999A, "position.z");

        let got = bits(&cam.world_from_view);
        for i in 0..16 {
            assert_eq!(
                got[i], WORLD[i],
                "world_from_view[{i}] (col {} .{}) got {:08X} want {:08X}",
                i / 4,
                ["x", "y", "z", "w"][i % 4],
                got[i],
                WORLD[i]
            );
        }

        let got = bits(&cam.clip_from_view);
        for i in 0..16 {
            assert_eq!(
                got[i], CLIP[i],
                "clip_from_view[{i}] (col {} .{}) got {:08X} want {:08X}",
                i / 4,
                ["x", "y", "z", "w"][i % 4],
                got[i],
                CLIP[i]
            );
        }
    }

    #[test]
    fn a_command_line_pose_is_finite() {
        let cam = probe_camera(Some([35.0, 20.0, 8.0]), 1.5);
        assert!(cam.position.x.is_finite());
        assert!(cam.position.y.is_finite());
        assert!(cam.position.z.is_finite());
        assert!(cam.clip_from_world.w_axis.w.is_finite());
        // yaw=35°, pitch=20°, distance=8 → z = cos(20°)cos(35°)*8 > 0
        assert!(cam.position.z > 0.0);
    }
}
