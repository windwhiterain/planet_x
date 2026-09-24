//! 评审视角表：**场景数据**里那一张"该从哪几个角度看"。
//!
//! ⚠ 它住**场景**这一侧（2026-09-27 从 `px_graph` 搬过来，用户裁定："camera 也不属于
//!   graph 啊，camera 就是 scene scripts 中的一种普通数据"）：相机不进图、不进产物、
//!   不进缓存键 —— 它管的是「怎么看」，不属于「这个节点算什么」。
//!   场景配方（`art/scene/*.toml` 的 `cameras` 一栏）点名用哪一张：`"review"`（默认）
//!   就是这里的 [`review`]。
//!
//! 这一组角度原来散在 `tools/probe.ps1`：`$views` 里手写 yaw/pitch，再靠
//! `AimLocal` 套上 `$tilt = 0.34`。现在它们是**局部方向**（行星还没被倾斜的
//! 那个系），倾斜由渲染器自己套（`px_render::planet::camera_for`）——
//! 所以这个文件里**一个渲染器常数都没有**，`0.34` 也不会再有一份副本。

use px_protocol::art::Camera;

/// 全局视角的距离（行星半径的倍数）。原来是 `probe.ps1` 的 `-Distance 3.15`。
pub const GLOBAL_DISTANCE: f32 = 3.15;
/// 立方体域特写的距离。原来是 `probe.ps1` 的 `Distance = 1.40`。
pub const CLOSE_DISTANCE: f32 = 1.40;

/// 12 个评审视角：赤道四向 / ±45° / 两极 / 立方体域的棱角·棱中·面心。
///
/// 覆盖的失效模式各不同：赤道看经度环绕，两极看域退化，棱与角看接缝与镜像采样。
pub fn review() -> Vec<Camera> {
    let mut out = Vec::with_capacity(12);

    let elevation = 8.0_f32.to_radians();
    for (yaw_degrees, tag) in [
        (0.0_f32, "equator-000"),
        (90.0, "equator-090"),
        (180.0, "equator-180"),
        (270.0, "equator-270"),
    ] {
        let yaw = yaw_degrees.to_radians();
        out.push(Camera::new(
            [
                elevation.cos() * yaw.sin(),
                elevation.sin(),
                elevation.cos() * yaw.cos(),
            ],
            GLOBAL_DISTANCE,
            tag,
        ));
    }

    let half = 45.0_f32.to_radians();
    out.push(Camera::new(
        [0.0, half.sin(), half.cos()],
        GLOBAL_DISTANCE,
        "north-045",
    ));
    out.push(Camera::new(
        [0.0, -half.sin(), half.cos()],
        GLOBAL_DISTANCE,
        "south-045",
    ));

    // 凑到 82° 而不是 90°：正落在极轴上时 up 向量退化成与视线共线
    let pole = 82.0_f32.to_radians();
    out.push(Camera::new(
        [0.0, pole.sin(), pole.cos()],
        GLOBAL_DISTANCE,
        "north-pole",
    ));
    out.push(Camera::new(
        [0.0, -pole.sin(), pole.cos()],
        GLOBAL_DISTANCE,
        "south-pole",
    ));

    for (direction, tag) in [
        ([1.0, 1.0, 1.0], "cube-corner-top"),
        ([1.0, -1.0, 1.0], "cube-corner-bottom"),
        ([1.0, 1.0, 0.0], "cube-edge-middle"),
        ([0.0, 0.0, 1.0], "cube-face-centre"),
    ] {
        out.push(Camera::new(direction, CLOSE_DISTANCE, tag));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_review_set_is_twelve_tagged_normalized_cameras() {
        let cameras = review();
        assert_eq!(cameras.len(), 12);
        let mut tags: Vec<&str> = cameras.iter().map(|camera| camera.tag.as_str()).collect();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), 12, "tag 必须互不相同，整理对照图时要靠它定位");
        for camera in &cameras {
            let length = (camera.direction[0] * camera.direction[0]
                + camera.direction[1] * camera.direction[1]
                + camera.direction[2] * camera.direction[2])
                .sqrt();
            assert!(
                (length - 1.0).abs() < 1e-5,
                "{} 的方向没归一化：{length}",
                camera.tag
            );
            assert!(
                camera.distance > 1.0,
                "{} 的相机在球里：{}",
                camera.tag,
                camera.distance
            );
        }
    }
}
