//! `px_nurbs_gpu_op` 的判据：**GPU ↔ CPU 对账** + 解析靶子 + 可复现。
//!
//! ⚠ 没有可用设备时**跳过**（不是失败）：判据测的是"两侧算的是不是同一个东西"，
//!   不是"这台机器必须有卡"（与 `px_volume_gpu_op` 同一条口径）。

use super::*;
use px_nurbs_schema::curve;
use px_nurbs_schema::surface;

/// 造一份球：`rings = 2` 时 v 向正好是 北极→赤道→南极。
fn ball(radius: f64) -> Surface {
    surface::sphere(radius, [0.0, 0.0, 0.0], 2).expect("造球")
}

fn ring(radius: f64) -> Curve {
    curve::circle(radius, "xy", [0.0; 3]).expect("造圆")
}

/// **曲面格点逐点对账**：GPU 的位置/法线与 CPU 的 `Surface::patch` 必须一致。
///
/// ⚠ 容差按 `f32` 定：CPU 那一侧是 `f64`，GPU 是 `f32`，两边不是同一种算术。
///   这一条同时校验四件事：基函数、齐次混合、商法则、以及参数网格的映射。
#[test]
fn the_gpu_surface_matches_the_cpu_point_by_point() {
    let ball = ball(2.0);
    let along = 17_u32;
    let Ok((positions, normals)) = surface_grid(&ball, along, along) else {
        println!("px_nurbs_gpu_op：没有可用 GPU，跳过");
        return;
    };
    let ((u0, u1), (v0, v1)) = ball.domain();
    let mut worst_point = 0.0_f32;
    let mut worst_normal = 0.0_f32;
    for i in 0..along {
        for j in 0..along {
            let u = u0 + (u1 - u0) * f64::from(i) / f64::from(along - 1);
            let v = v0 + (v1 - v0) * f64::from(j) / f64::from(along - 1);
            let want = ball.patch(u, v).expect("CPU 求值");
            let index = (i * along + j) as usize;
            for lane in 0..3 {
                worst_point =
                    worst_point.max((positions[index * 3 + lane] - want.point[lane] as f32).abs());
                let want_normal = want.normal.unwrap_or([0.0, 1.0, 0.0]);
                worst_normal =
                    worst_normal.max((normals[index * 3 + lane] - want_normal[lane] as f32).abs());
            }
        }
    }
    println!("px_nurbs_gpu_op：曲面格点最大偏差 位置 {worst_point:.2e} / 法线 {worst_normal:.2e}");
    assert!(worst_point < 1e-4, "位置最大偏差 {worst_point}");
    assert!(worst_normal < 1e-4, "法线最大偏差 {worst_normal}");
}

/// **曲线格点逐点对账**：GPU 的点与 `Curve::point` 必须一致，而且落在圆上。
#[test]
fn the_gpu_curve_matches_the_cpu_point_by_point() {
    let ring = ring(1.5);
    let count = 33_u32;
    let Ok(points) = curve_points(&ring, count) else {
        println!("px_nurbs_gpu_op：没有可用 GPU，跳过");
        return;
    };
    let (low, high) = ring.domain();
    let mut worst = 0.0_f32;
    for index in 0..count as usize {
        let t = low + (high - low) * index as f64 / f64::from(count - 1);
        let want = ring.point(t).expect("CPU 求值");
        let got = &points[index * 3..index * 3 + 3];
        for lane in 0..3 {
            worst = worst.max((got[lane] - want[lane] as f32).abs());
        }
        let radius = (got[0] * got[0] + got[1] * got[1]).sqrt();
        assert!(
            (radius - 1.5).abs() < 1e-5,
            "第 {index} 个点离圆心 {radius}"
        );
    }
    println!("px_nurbs_gpu_op：曲线格点最大偏差 {worst:.2e}");
    assert!(worst < 1e-5, "曲线最大偏差 {worst}");
}

/// **同一次派发跑两遍逐位相同**：GPU 上的算术顺序在这一档里是确定的
/// （同一台机器、同一个驱动、同一份输入）。
#[test]
fn the_same_grid_twice_is_bit_for_bit() {
    let ball = ball(1.0);
    let (Ok(first), Ok(second)) = (surface_grid(&ball, 9, 9), surface_grid(&ball, 9, 9)) else {
        println!("px_nurbs_gpu_op：没有可用 GPU，跳过");
        return;
    };
    assert_eq!(first.0, second.0, "位置不可复现");
    assert_eq!(first.1, second.1, "法线不可复现");
}

/// **细分出来的网格**：顶点在球面上、每条边恰好被两个三角形用到、欧拉数 = 2
/// —— 与 CPU 那一侧**同一套判据**（装配也是同一个函数）。
#[test]
fn the_gpu_tessellation_is_watertight_and_on_the_radius() {
    let ball = ball(3.0);
    let Ok(mesh) = tessellate_surface(&ball, 1e-2, 3, 2) else {
        println!("px_nurbs_gpu_op：没有可用 GPU，跳过");
        return;
    };
    assert!(mesh.vertices() > 32, "网格太寒酸了，这个判据没在测东西");
    for vertex in 0..mesh.vertices() {
        let point = &mesh.positions[vertex * 3..vertex * 3 + 3];
        let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
        assert!(
            (radius - 3.0).abs() < 1e-3,
            "第 {vertex} 个顶点离球心 {radius}（应当是 3）"
        );
    }
    let mut edges: std::collections::HashMap<(u32, u32), usize> = std::collections::HashMap::new();
    for triangle in mesh.indices.chunks_exact(3) {
        for pair in 0..3 {
            let one = triangle[pair];
            let two = triangle[(pair + 1) % 3];
            let key = if one < two { (one, two) } else { (two, one) };
            *edges.entry(key).or_default() += 1;
        }
    }
    let open = edges.values().filter(|count| **count == 1).count();
    let nonmanifold = edges.values().filter(|count| **count > 2).count();
    assert_eq!(open, 0, "有 {open} 条开口边");
    assert_eq!(nonmanifold, 0, "有 {nonmanifold} 条非流形边");
    assert_eq!(
        mesh.vertices() as i64 - edges.len() as i64 + mesh.triangles() as i64,
        2,
        "欧拉数不是 2 ⇒ 这不是一张闭合的球面网格"
    );
    println!(
        "px_nurbs_gpu_op：球网格 {} 顶点 / {} 三角形（{} 条边）",
        mesh.vertices(),
        mesh.triangles(),
        edges.len()
    );
}

/// **曲线细分**：折线落在圆上、段数 = 顶点数（闭合）、最后一段接回第 0 个。
#[test]
fn the_gpu_curve_tessellation_closes_on_itself() {
    let ring = ring(1.0);
    let Ok(line) = tessellate_curve(&ring, 1e-3, 6, 4) else {
        println!("px_nurbs_gpu_op：没有可用 GPU，跳过");
        return;
    };
    assert!(line.vertices() >= 8);
    assert_eq!(line.segments(), line.vertices(), "闭合折线的段数 = 顶点数");
    for vertex in 0..line.vertices() {
        let point = &line.positions[vertex * 3..vertex * 3 + 3];
        let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
        assert!(
            (radius - 1.0).abs() < 1e-5,
            "第 {vertex} 个顶点离圆心 {radius}（应当是 1）"
        );
    }
    assert_eq!(
        &line.indices[line.indices.len() - 2..],
        &[line.vertices() as u32 - 1, 0],
        "最后一段没有接回第 0 个顶点"
    );
}

/// **两个基数的曲面**也对得上（次数不是写死的 2）：升阶到 4 次之后 GPU 与 CPU 仍一致。
#[test]
fn the_gpu_follows_the_curve_degree() {
    let circle = curve::circle(1.0, "xy", [0.0; 3]).expect("造圆");
    let higher = curve::elevate_degree(&circle, 4).expect("升阶");
    let count = 25_u32;
    let Ok(points) = curve_points(&higher, count) else {
        println!("px_nurbs_gpu_op：没有可用 GPU，跳过");
        return;
    };
    let (low, high) = higher.domain();
    let mut worst = 0.0_f32;
    for index in 0..count as usize {
        let t = low + (high - low) * index as f64 / f64::from(count - 1);
        let want = higher.point(t).expect("CPU 求值");
        let got = &points[index * 3..index * 3 + 3];
        for lane in 0..3 {
            worst = worst.max((got[lane] - want[lane] as f32).abs());
        }
    }
    assert!(worst < 1e-5, "4 次曲线最大偏差 {worst}");
}
