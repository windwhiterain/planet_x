//! **相机 oracle**：把 Bevy 那条路算出来的视图矩阵原样落盘，给 `px_render_wgpu` 的移植当判据。
//!
//! 为什么要它：`px_render_wgpu` 不许依赖 bevy / glam，于是"相机 → 矩阵"这条链要自己写。
//! 而这条链上有三处**读代码定不了、只能实测**的地方（§110.1 / §109.2 记的都是这类）：
//!
//! 1. `view_from_world = world_from_view.inverse()` —— glam 走的是**通用余子式求逆**，
//!    而"刚体变换的解析逆"（`[Rᵀ | −Rᵀt]`）在数学上等价、在最后一位上不一定等价。
//!    这一条决定了移植件要不要把 glam 那 40 行余子式**连括号一起抄**。
//! 2. `f32::exp2(-9.7f32) / 1.2f32` 到底是不是 f64 那个值取整（`view.exposure`）。
//! 3. `clip_from_world = clip_from_view * view_from_world` 的乘法次序与结合方式。
//!
//! ⚠ 默认 `#[ignore]`：它是一次性导出，不是门。
//! 用法：`cargo test -p px_render --test view_oracle -- --ignored --nocapture`

use bevy::prelude::*;

/// 位模式 —— 比较两个 f32 是不是**同一个数**，这样写没有"看着差不多"的余地。
fn bits(value: f32) -> String {
    format!("{:08X}", value.to_bits())
}

fn dump_matrix(tag: &str, matrix: &Mat4) {
    println!("{tag}");
    for (index, column) in [matrix.x_axis, matrix.y_axis, matrix.z_axis, matrix.w_axis]
        .iter()
        .enumerate()
    {
        println!(
            "  c{index}  {:>14.9} {:>14.9} {:>14.9} {:>14.9}   |  {} {} {} {}",
            column.x,
            column.y,
            column.z,
            column.w,
            bits(column.x),
            bits(column.y),
            bits(column.z),
            bits(column.w)
        );
    }
}

fn all_equal(a: &Mat4, b: &Mat4) -> bool {
    let left = [a.x_axis, a.y_axis, a.z_axis, a.w_axis];
    let right = [b.x_axis, b.y_axis, b.z_axis, b.w_axis];
    left.iter().zip(right.iter()).all(|(l, r)| {
        l.x.to_bits() == r.x.to_bits()
            && l.y.to_bits() == r.y.to_bits()
            && l.z.to_bits() == r.z.to_bits()
            && l.w.to_bits() == r.w.to_bits()
    })
}

/// 刚体变换的**解析逆**：`[Rᵀ | −Rᵀt]`。这是"省掉 glam 那 40 行"的那条路。
fn analytic_rigid_inverse(rotation: Quat, translation: Vec3) -> Mat4 {
    let transpose = Mat3::from_quat(rotation).transpose();
    let shifted = transpose * -translation;
    Mat4::from_cols(
        transpose.x_axis.extend(0.0),
        transpose.y_axis.extend(0.0),
        transpose.z_axis.extend(0.0),
        shifted.extend(1.0),
    )
}

#[test]
#[ignore = "oracle 导出：跑一次就够，不进日常门"]
fn dump_the_default_camera_matrices() {
    // 出图时不给 `--cam` ⇒ `probe_camera(None)`（`px_render/src/scene.rs:518-530`）。
    let transform = Transform::from_xyz(0.0, 0.55, 3.15).looking_at(Vec3::ZERO, Vec3::Y);
    let world_from_view = Mat4::from_scale_rotation_translation(
        transform.scale,
        transform.rotation,
        transform.translation,
    );
    dump_matrix("world_from_view（相机位姿，不是 view 矩阵）", &world_from_view);

    let general = world_from_view.inverse();
    dump_matrix("view_from_world = world_from_view.inverse()（glam 通用求逆）", &general);

    let analytic = analytic_rigid_inverse(transform.rotation, transform.translation);
    dump_matrix("解析刚体逆 [Rᵀ | −Rᵀt]", &analytic);

    println!();
    println!(
        "== 通用逆与解析逆逐位相同吗：**{}**",
        if all_equal(&general, &analytic) { "相同" } else { "不同" }
    );

    // 投影：`PerspectiveProjection::default()` = fov π/4、near 0.1、far 1000（far 不进矩阵）；
    // aspect 由视口给，960×640 ⇒ 1.5。
    let clip_from_view = Mat4::perspective_infinite_reverse_rh(
        core::f32::consts::PI / 4.0,
        960.0 / 640.0,
        0.1,
    );
    dump_matrix("clip_from_view（无限 reverse-Z 右手）", &clip_from_view);
    dump_matrix(
        "clip_from_world = clip_from_view * view_from_world",
        &(clip_from_view * general),
    );

    println!();
    println!("world_position = {:?}", transform.translation.to_array());
    let exposure = f32::exp2(-9.7_f32) / 1.2_f32;
    println!(
        "exposure = exp2(-9.7)/1.2 = {exposure:.12e}（{}）｜f64 版 {:.15e}",
        bits(exposure),
        f64::exp2(-9.7_f64) / 1.2_f64
    );
    println!(
        "viewport = ({}, {}, {}, {})｜aspect = {}",
        0,
        0,
        960,
        640,
        bits(960.0 / 640.0)
    );
}

/// 一批 **(输入 → glam 通用逆)** 的位模式向量，落盘给移植件当测试集。
/// 为什么要一批而不是一个：一个向量可能被碰巧对上，而"解析刚体逆"那条路在这里
/// 只差 1–2 ulp —— 用一批不同姿态/缩放的矩阵才拦得住那种"差一点点"。
#[test]
#[ignore = "oracle 导出：跑一次就够，不进日常门"]
fn dump_inverse_vectors() {
    let mut lines = vec![
        "# glam 0.32.1 Mat4::inverse（x86_64 走 SSE2 那条）的测试向量".to_string(),
        "# 每行：输入 16 个 f32 的位模式 → 期望 16 个 f32 的位模式（列主序，x/y/z/w 依次）".to_string(),
        "# 输入矩阵不一定是刚体：这里混了带缩放与非正交的，免得移植件只对刚体成立".to_string(),
    ];

    let mut cases: Vec<Mat4> = Vec::new();
    // 1) 出图那个默认相机（`probe_camera(None)`）。
    let camera = Transform::from_xyz(0.0, 0.55, 3.15).looking_at(Vec3::ZERO, Vec3::Y);
    cases.push(Mat4::from_scale_rotation_translation(
        camera.scale,
        camera.rotation,
        camera.translation,
    ));
    // 2) 几组别的相机姿态（`--cam yaw,pitch,dist` 那条路）。
    for cam in [[0.0_f32, 0.0, 3.5], [35.0, 20.0, 2.4], [-120.0, -55.0, 6.0]] {
        let yaw = cam[0].to_radians();
        let pitch = cam[1].clamp(-89.5, 89.5).to_radians();
        let direction = Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos());
        let transform = Transform::from_translation(direction * cam[2]).looking_at(Vec3::ZERO, Vec3::Y);
        cases.push(Mat4::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        ));
    }
    // 3) 带缩放与非正交的：刚体专用的写法在这些上会**明显**不对（粗网）。
    cases.push(Mat4::from_scale_rotation_translation(
        Vec3::new(1.5, 0.5, 2.25),
        Quat::from_rotation_y(0.7),
        Vec3::new(-1.0, 2.0, 0.25),
    ));
    cases.push(Mat4::from_cols(
        Vec4::new(1.0, 2.0, 3.0, 4.0),
        Vec4::new(0.5, -1.0, 0.25, 2.0),
        Vec4::new(-2.0, 0.75, 1.5, -0.5),
        Vec4::new(3.0, 0.125, -1.0, 1.0),
    ));

    for (index, input) in cases.iter().enumerate() {
        let expected = input.inverse();
        let flat = |matrix: &Mat4| -> String {
            [matrix.x_axis, matrix.y_axis, matrix.z_axis, matrix.w_axis]
                .iter()
                .flat_map(|column| [column.x, column.y, column.z, column.w])
                .map(bits)
                .collect::<Vec<_>>()
                .join(" ")
        };
        lines.push(format!("case {index}"));
        lines.push(format!("  in  {}", flat(input)));
        lines.push(format!("  out {}", flat(&expected)));
    }

    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_render 必须住在 workspace 下");
    let directory = workspace.join("target").join("oracle");
    std::fs::create_dir_all(&directory).expect("建不了 target/oracle");
    let path = directory.join("bevy-view-vectors.txt");
    std::fs::write(&path, lines.join("\n") + "\n").expect("写不了向量文件");
    println!("写了 {} 组向量：{}", cases.len(), path.display());
}

/// **判定性实验**：Bevy 建相机位姿走的是 `look_to` → `Quat::from_mat3` →
/// `Affine3A::from_rotation_translation`（内部再 `Mat3A::from_quat`）。
/// 也就是"三列 → 四元数 → 三列"绕了一圈。
///
/// 如果这一圈是**逐位恒等**的，移植件就**不用**移植 `Quat::from_mat3` / `Mat3::from_quat`
/// —— 直接拿 `right/up/back` 拼矩阵即可，省掉一整块容易写错的 Shepperd 代码。
/// 如果**不是**恒等，就必须照抄那两段。
///
/// 注意这里比的是 **`Affine3A`**（`Mat3A`，SIMD 版）那条真实路径，不是标量 `Mat3`。
#[test]
#[ignore = "oracle 导出：跑一次就够，不进日常门"]
fn is_the_quaternion_round_trip_the_identity() {
    fn same_mat4(a: &Mat4, b: &Mat4) -> bool {
        [a.x_axis, a.y_axis, a.z_axis, a.w_axis]
            .iter()
            .zip([b.x_axis, b.y_axis, b.z_axis, b.w_axis].iter())
            .all(|(l, r)| {
                l.x.to_bits() == r.x.to_bits()
                    && l.y.to_bits() == r.y.to_bits()
                    && l.z.to_bits() == r.z.to_bits()
                    && l.w.to_bits() == r.w.to_bits()
            })
    }

    for cam in [
        None,
        Some([0.0_f32, 0.0, 3.5]),
        Some([35.0, 20.0, 2.4]),
        Some([-120.0, -55.0, 6.0]),
    ] {
        let transform = match cam {
            None => Transform::from_xyz(0.0, 0.55, 3.15).looking_at(Vec3::ZERO, Vec3::Y),
            Some([yaw, pitch, distance]) => {
                let yaw = yaw.to_radians();
                let pitch = pitch.clamp(-89.5, 89.5).to_radians();
                let direction =
                    Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos());
                Transform::from_translation(direction * distance).looking_at(Vec3::ZERO, Vec3::Y)
            }
        };

        // Bevy 的真实路径。
        let bevy_way = Mat4::from_scale_rotation_translation(
            transform.scale,
            transform.rotation,
            transform.translation,
        );

        // 解析路径：`look_to` 的三列直接拼。
        let back = (-(Vec3::ZERO - transform.translation)).normalize();
        let right = Vec3::Y.cross(back).normalize();
        let up = back.cross(right);
        let direct = Mat4::from_cols(
            right.extend(0.0),
            up.extend(0.0),
            back.extend(0.0),
            transform.translation.extend(1.0),
        );

        println!(
            "cam {cam:?}｜三列直接拼 == Bevy 那条绕四元数的路：**{}**",
            if same_mat4(&bevy_way, &direct) { "相同" } else { "不同" }
        );
    }
}
