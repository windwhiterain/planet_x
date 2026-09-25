#![allow(dead_code)]

use crate::mat4::{Mat3, Mat4, Quat, Vec3, Vec4};

pub struct Camera {
    pub position: Vec3,
    pub world_from_view: Mat4,
    pub view_from_world: Mat4,
    pub clip_from_view: Mat4,
    pub view_from_clip: Mat4,
    pub clip_from_world: Mat4,
}

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
    from_pose(translation, rotation, aspect)
}

pub fn review_camera(camera: &px_protocol::art::Camera, aspect: f32) -> Camera {
    let direction = Vec3::from_array(camera.direction);
    let direction = if direction.dot(direction) > 1e-12 {
        direction.normalize()
    } else {
        Vec3::Z
    };
    let translation = direction * camera.distance.max(1e-3);
    let rotation = looking_at(translation, Vec3::ZERO, Vec3::Y);
    from_pose(translation, rotation, aspect)
}

fn from_pose(translation: Vec3, rotation: Quat, aspect: f32) -> Camera {
    let world_from_view =
        Mat4::from_scale_rotation_translation(Vec3::splat(1.0), rotation, translation);
    let clip_from_view =
        Mat4::perspective_infinite_reverse_rh(core::f32::consts::PI / 4.0, aspect, 0.1);
    let view_from_world = world_from_view.inverse();
    let view_from_clip = clip_from_view.inverse();
    let clip_from_world = clip_from_view.mul_mat4(&view_from_world);

    Camera {
        position: translation,
        world_from_view,
        view_from_world,
        clip_from_view,
        view_from_clip,
        clip_from_world,
    }
}

fn looking_at(translation: Vec3, target: Vec3, up: Vec3) -> Quat {
    look_to(target - translation, up)
}

pub const CUBE_MAP_FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::NEG_Z, Vec3::Y),
    (Vec3::Z, Vec3::Y),
];

pub fn face_view(light_position: Vec3, face: u32, near_z: f32) -> Camera {
    let (target, up) = CUBE_MAP_FACES[face as usize];
    let rotation = looking_at(Vec3::ZERO, target, up);
    let left = Mat3::IDENTITY;
    let right = Mat3::from_quat(rotation);
    let matrix3 = left.mul_mat3(&right);
    let translation = left.mul_vec3(Vec3::ZERO).add(light_position);
    let world_from_view = Mat4::from_cols(
        Vec4::new(matrix3.x_axis.x, matrix3.x_axis.y, matrix3.x_axis.z, 0.0),
        Vec4::new(matrix3.y_axis.x, matrix3.y_axis.y, matrix3.y_axis.z, 0.0),
        Vec4::new(matrix3.z_axis.x, matrix3.z_axis.y, matrix3.z_axis.z, 0.0),
        Vec4::new(translation.x, translation.y, translation.z, 1.0),
    );
    let clip_from_view =
        Mat4::perspective_infinite_reverse_rh(core::f32::consts::FRAC_PI_2, 1.0, near_z);
    let view_from_world = world_from_view.inverse();
    let view_from_clip = clip_from_view.inverse();
    let clip_from_world = clip_from_view.mul_mat4(&view_from_world);
    Camera {
        position: light_position,
        world_from_view,
        view_from_world,
        clip_from_view,
        view_from_clip,
        clip_from_world,
    }
}

fn look_to(direction: Vec3, up: Vec3) -> Quat {
    let back = -dir3(direction, Vec3::Z);
    let up = dir3(up, Vec3::Y);
    let right = up.cross(back).try_normalize().unwrap_or(Vec3::X);
    let up = back.cross(right);
    Quat::from_mat3(&Mat3::from_cols(right, up, back))
}

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
    use super::{CUBE_MAP_FACES, probe_camera, review_camera};
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
        0x3F80_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0x3F7C_2F4D,
        0xBE30_2108,
        0x0000_0000,
        0x0000_0000,
        0x3E30_2108,
        0x3F7C_2F4D,
        0x0000_0000,
        0x0000_0000,
        0x3F0C_CCCD,
        0x4049_999A,
        0x3F80_0000,
    ];

    const CLIP: [u32; 16] = [
        0x3FCE_034C,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0x401A_8279,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0xBF80_0000,
        0x0000_0000,
        0x0000_0000,
        0x3DCC_CCCD,
        0x0000_0000,
    ];

    const VIEW_FROM_WORLD: [u32; 16] = [
        0x3F80_0000,
        0x8000_0000,
        0x0000_0000,
        0x8000_0000,
        0x8000_0000,
        0x3F7C_2F4F,
        0x3E30_2109,
        0x0000_0000,
        0x0000_0000,
        0xBE30_2109,
        0x3F7C_2F4F,
        0x8000_0000,
        0x0000_0000,
        0xB380_0001,
        0xC04C_A664,
        0x3F80_0000,
    ];

    const VIEW_FROM_CLIP: [u32; 16] = [
        0x3F1F_0EDA,
        0x8000_0000,
        0x0000_0000,
        0x8000_0000,
        0x8000_0000,
        0x3ED4_13CD,
        0x8000_0000,
        0x0000_0000,
        0x0000_0000,
        0x8000_0000,
        0x0000_0000,
        0x4120_0000,
        0x8000_0000,
        0x0000_0000,
        0xBF7F_FFFF,
        0x0000_0000,
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
                got[i],
                WORLD[i],
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
                got[i],
                CLIP[i],
                "clip_from_view[{i}] (col {} .{}) got {:08X} want {:08X}",
                i / 4,
                ["x", "y", "z", "w"][i % 4],
                got[i],
                CLIP[i]
            );
        }

        for (what, matrix, expected) in [
            ("view_from_world", &cam.view_from_world, VIEW_FROM_WORLD),
            ("view_from_clip", &cam.view_from_clip, VIEW_FROM_CLIP),
        ] {
            let got = bits(matrix);
            for i in 0..16 {
                assert_eq!(
                    got[i],
                    expected[i],
                    "{what}[{i}] (col {} .{}) got {:08X} want {:08X}",
                    i / 4,
                    ["x", "y", "z", "w"][i % 4],
                    got[i],
                    expected[i]
                );
            }
        }
    }

    #[test]
    fn a_command_line_pose_is_finite() {
        let cam = probe_camera(Some([35.0, 20.0, 8.0]), 1.5);
        assert!(cam.position.x.is_finite());
        assert!(cam.position.y.is_finite());
        assert!(cam.position.z.is_finite());
        assert!(cam.clip_from_world.w_axis.w.is_finite());
        assert!(cam.position.z > 0.0);
    }

    #[test]
    fn the_review_camera_at_the_probe_pose_is_bit_identical_to_the_probe_camera() {
        let aspect = 960.0f32 / 640.0f32;
        let review = review_camera(
            &px_protocol::art::Camera::raw([0.0, 0.0, 1.0], 3.15, "probe"),
            aspect,
        );
        let probe = probe_camera(Some([0.0, 0.0, 3.15]), aspect);
        assert_eq!(
            review.position.to_array(),
            probe.position.to_array(),
            "位置要么逐位相同，要么这条实验不成立"
        );
        for (what, a, b) in [
            (
                "world_from_view",
                &review.world_from_view,
                &probe.world_from_view,
            ),
            (
                "view_from_world",
                &review.view_from_world,
                &probe.view_from_world,
            ),
            (
                "clip_from_view",
                &review.clip_from_view,
                &probe.clip_from_view,
            ),
            (
                "view_from_clip",
                &review.view_from_clip,
                &probe.view_from_clip,
            ),
            (
                "clip_from_world",
                &review.clip_from_world,
                &probe.clip_from_world,
            ),
        ] {
            assert_eq!(bits(a), bits(b), "{what} 不逐位相同");
        }
    }

    #[test]
    fn a_degenerate_direction_falls_back_to_z_and_the_distance_has_a_floor() {
        let degenerate = review_camera(
            &px_protocol::art::Camera::raw([0.0, 0.0, 0.0], 0.0, "degenerate"),
            1.0,
        );
        assert_eq!(degenerate.position.to_array(), [0.0, 0.0, 1e-3]);
        let tiny = review_camera(
            &px_protocol::art::Camera::raw([1e-7, 0.0, 0.0], 4.0, "tiny"),
            1.0,
        );
        assert_eq!(
            tiny.position.to_array(),
            [0.0, 0.0, 4.0],
            "平方 1e-14 不够 1e-12"
        );
    }
    #[test]
    fn the_protocols_six_face_basis_is_the_one_these_cameras_use() {
        for (index, (target, up)) in CUBE_MAP_FACES.iter().enumerate() {
            let back = target.mul(-1.0);
            let right = up.cross(back).normalize();
            let up2 = back.cross(right);
            let expected = [right, up2, *target];
            let got = px_protocol::scene::SHADOW_FACE_BASIS[index];
            for (slot, (want, have)) in expected.iter().zip(got.iter()).enumerate() {
                assert_eq!(
                    want.to_array(),
                    *have,
                    "第 {index} 面（{}）的第 {slot} 条基向量：协议那张表是 {have:?}，\
                     而这六台相机用的是 {want:?} —— 两处不一致 ⇒ 烘图侧会把页分到\"不是\
                     渲染器画的那一面\"上，而着色器照着协议查页 ⇒ 影子贴错面",
                    ["+x", "-x", "+y", "-y", "-z", "+z"][index]
                );
            }
        }
    }
}
