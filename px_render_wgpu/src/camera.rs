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
//!
//! ⚠ 两条**逆矩阵**（`view_from_world` 与 `view_from_clip`）都走 `mat4::inverse` 那个
//! **逐位移植的通用逆**（§110.1.1：解析逆差 1–2 ulp）。这不是"省事的写法"能换的：
//! 刚体明明有 `[Rᵀ | −Rᵀt]` 这条更省事的解析逆，实测与 glam 的余子式逆**不是同一个数**。

#![allow(dead_code)]

use crate::mat4::{Mat3, Mat4, Quat, Vec3, Vec4};

pub struct Camera {
    pub position: Vec3,
    pub world_from_view: Mat4,
    pub view_from_world: Mat4,
    pub clip_from_view: Mat4,
    /// `clip_from_view` 的**通用逆**（天空盒的片元阶段用它把片元坐标还原成视线方向）。
    ///
    /// ⚠ 它是**这一帧唯一的**求逆点：宿主算一次、进 `view` uniform，shader 那边只做乘法 ——
    /// 在 shader 里求逆既慢又是另一条算术路径（§110.1.1，逐位判据下"等价"不算等价）。
    pub view_from_clip: Mat4,
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
    // 天空盒要的那条逆：Bevy 那边是 `let view_from_clip = clip_from_view.inverse();`
    // （`bevy_render-0.19.1/src/view/mod.rs:1048`）—— 同一个函数、同一个顺序，
    // 所以这里也是宿主算、shader 只乘。
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

/// Bevy `Transform::look_at` -> `look_to`。
fn looking_at(translation: Vec3, target: Vec3, up: Vec3) -> Quat {
    look_to(target - translation, up)
}

/// 点光 cube 的**六面朝向**：`bevy_camera-0.19.1/src/primitives.rs:347-378` 的
/// `CUBE_MAP_FACES`，次序 `+X −X +Y −Y +Z −Z`。
///
/// ⚠ 次序**不是**随便排的：它同时定了两件事 —— 烘图侧那个 `face` 数（`px_graphs::frame::
/// FACE_NAMES` 与它同序）与这里算矩阵用的 target/up。两处不一致 ⇒ 影子贴到别的面上，
/// 而"贴错面"在画面上常常看着像"影子有点歪"。宿主拿到的 `face` 与 `layer` 是一起进来的
/// （`PassSpec::cube_face`），对账那条算式（`layer == light×6 + face`）就是这条一致性的门。
///
/// ⚠ `+Z` / `−Z` 两面的 target 是**反**的（`+Z` 用 `NEG_Z`、`−Z` 用 `+Z`）：cube 的坐标系是
/// **左手 y-up**，而 Bevy 的世界是右手 y-up（`primitives.rs:341-346` 的注释），
/// 所以"哪一面"要在两套约定之间翻一次。照抄，不要"理顺"。
pub const CUBE_MAP_FACES: [(Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::NEG_Z, Vec3::Y),
    (Vec3::Z, Vec3::Y),
];

/// 点光 cube 的**某一面**当相机：`light.rs:2061-2123` 那条路。
///
/// ⚠⚠ 两处**不许化简**（§109.2）：
///
/// 1. `world_from_view = GlobalTransform::from_translation(灯位) × Transform::looking_at(...)`
///    —— 那是**两个仿射矩阵相乘**（`Affine3A * Affine3A` ⇒ `matrix3.mul_mat3` 与
///    `matrix3.mul_vec3(t) + t`），**不是**"拿四元数直接拼一个 `[R|t]`"。
///    两种写法数学等价、浮点不等价（±0.0 与 1 ulp 那一族），而这里差一个末位就会
///    让影子边缘的 8 个比较采样里有一个翻符号。所以照抄乘法链。
/// 2. 投影是 `perspective_infinite_reverse_rh(FRAC_PI_2, 1.0, near_z)` —— **90°、宽高比 1**、
///    近平面来自 `PointLight::shadow_map_near_z`（0.1）。那是 §109 里"最大的绝对轴就是
///    世界深度"那条推导的前提（45° 的视锥面）。
///
/// `near_z` 由调用方给（它住在 group 0 那一份策略常量里）。
pub fn face_view(light_position: Vec3, face: u32, near_z: f32) -> Camera {
    let (target, up) = CUBE_MAP_FACES[face as usize];
    // `Transform::IDENTITY.looking_at(target, up)`（`light.rs:1095-1098` 预先把六面算好）。
    let rotation = looking_at(Vec3::ZERO, target, up);
    // ---- `GlobalTransform::from_translation(灯位) * Transform::from_rotation(...)` ----
    //
    // 左边：单位矩阵 + 平移；右边：`Mat3::from_quat` + 零平移。
    // `Affine3A * Affine3A` = `matrix3: a.m3 * b.m3`、`translation: a.m3 * b.t + a.t`。
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
    let clip_from_view = Mat4::perspective_infinite_reverse_rh(
        core::f32::consts::FRAC_PI_2,
        1.0,
        near_z,
    );
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

    /// `world_from_view.inverse()` —— **不是**解析逆（§110.1.1）。
    ///
    /// 摘自 `target/oracle/bevy-view-vectors.txt` 的 `case 0` `out`：那 6 组向量的第一组
    /// 就是默认相机（`case 0` 的 `in` 正是上面那个 `WORLD`），而它期望的 `out` 是
    /// **glam 的通用余子式逆**。⚠ 解析刚体逆 `[Rᵀ | −Rᵀt]` 在这里给的是
    /// `c1.y = 3F7C2F4D`（与位姿同值）、`c3.z = C04CA662` —— 差 1–2 ulp，**不是**这个常量。
    const VIEW_FROM_WORLD: [u32; 16] = [
        0x3F80_0000, 0x8000_0000, 0x0000_0000, 0x8000_0000,
        0x8000_0000, 0x3F7C_2F4F, 0x3E30_2109, 0x0000_0000,
        0x0000_0000, 0xBE30_2109, 0x3F7C_2F4F, 0x8000_0000,
        0x0000_0000, 0xB380_0001, 0xC04C_A664, 0x3F80_0000,
    ];

    /// `clip_from_view.inverse()`（天空盒的片元阶段用它还原视线方向）。
    ///
    /// 摘自 `px_render/tests/view_oracle.rs::dump_the_default_camera_matrices` 这一次的
    /// 实测读数（`bevy_render-0.19.1/src/view/mod.rs:1048` 那一行就是它的出处）。
    /// ⚠ 这一格**盖不到**上面那 6 组向量：那批的输入是位姿/一般矩阵，而投影矩阵不是刚体 ——
    /// `[Rᵀ | −Rᵀt]` 这条解析路在这儿连形式都不成立。所以它只能对着 Bevy 的读数钉。
    const VIEW_FROM_CLIP: [u32; 16] = [
        0x3F1F_0EDA, 0x8000_0000, 0x0000_0000, 0x8000_0000,
        0x8000_0000, 0x3ED4_13CD, 0x8000_0000, 0x0000_0000,
        0x0000_0000, 0x8000_0000, 0x0000_0000, 0x4120_0000,
        0x8000_0000, 0x0000_0000, 0xBF7F_FFFF, 0x0000_0000,
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

        // 两条逆矩阵（§135）：一条给内容 shader 的透明排序/裁剪坐标那条路，
        // 一条给天空盒重建视线方向。⚠ 都由**通用逆**产出 —— 解析逆在这两格上都不是这个数。
        for (what, matrix, expected) in [
            ("view_from_world", &cam.view_from_world, VIEW_FROM_WORLD),
            ("view_from_clip", &cam.view_from_clip, VIEW_FROM_CLIP),
        ] {
            let got = bits(matrix);
            for i in 0..16 {
                assert_eq!(
                    got[i], expected[i],
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
        // yaw=35°, pitch=20°, distance=8 → z = cos(20°)cos(35°)*8 > 0
        assert!(cam.position.z > 0.0);
    }
}
