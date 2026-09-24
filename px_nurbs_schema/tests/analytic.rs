//! **NURBS 的解析对照与不变量**：这些判据量的是"库算得对不对"，不是"接口接得通"。
//!
//! 三条量法（都是**解析**靶子，不依赖任何外部产物）：
//!
//! 1. **精确圆**：有理二次整圆的每个参数点都必须落在半径上（`|P(t)| = R`）；
//! 2. **不变量**：插入节点 / 升阶**不换几何**（同一个参数上的点逐位相同）；
//! 3. **导数**：圆的切向量与半径垂直（商法则那条路的落点）。
//!
//! ⚠ 全库按 `f64` 算，判据的容差因此能开到 `1e-10` 量级 —— 而 `f32` 存不下
//!   `√2/2` 那个权（圆会瘪），这正是"为什么不是 f32"那条决定的靶子。

use px_nurbs_schema::curve::{self, Curve};
use px_nurbs_schema::surface::{self, Surface};

fn circle() -> Curve {
    curve::circle(1.0, "xy", [0.0; 3]).expect("造一条单位圆")
}

/// **精确圆**：`|P(t)| = R` 在整条参数上成立。
///
/// ⚠ 它同时钉住三件事：权重是 `√2/2`、节点重数是 `{¼,¼,½,½,¾,¾}`（少一重圆就瘪）、
///   求值走的是**有理** de Boor（不是把权当摆设）。
#[test]
fn the_rational_circle_stays_on_the_radius() {
    let circle = circle();
    for step in 0..=200 {
        let t = f64::from(step) / 200.0;
        let point = circle.point(t).expect("求值");
        let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
        assert!((radius - 1.0).abs() < 1e-12, "t={t} 处的半径是 {radius}");
        assert!(point[2].abs() < 1e-15, "t={t} 处跑出了 xy 平面：{point:?}");
    }
}

/// **一阶导与半径垂直**（圆的法线就是半径方向）。
#[test]
fn the_hodograph_of_the_circle_is_perpendicular_to_the_radius() {
    let circle = circle();
    for step in 1..100 {
        let t = f64::from(step) / 100.0;
        let sample = circle.sample(t).expect("求值");
        let dot = sample.point[0] * sample.tangent[0] + sample.point[1] * sample.tangent[1];
        assert!(dot.abs() < 1e-12, "t={t} 处点积是 {dot}");
        let speed =
            (sample.tangent[0] * sample.tangent[0] + sample.tangent[1] * sample.tangent[1]).sqrt();
        assert!(speed > 0.1, "t={t} 处切向量退化了：{speed}");
    }
}

/// **插入节点不换几何**：控制点变多、次数不变、每个参数点上的值不变（Böhm 的性质）。
#[test]
fn inserting_a_knot_leaves_the_curve_where_it_was() {
    let circle = circle();
    let fine = curve::insert_knot(&circle, 0.125, 1).expect("插一个节点");
    assert_eq!(fine.degree, circle.degree);
    assert_eq!(fine.count(), circle.count() + 1);
    assert_eq!(fine.weights.len(), circle.weights.len() + 1);
    for step in 0..=100 {
        let t = f64::from(step) / 100.0;
        let before = circle.point(t).expect("原曲线");
        let after = fine.point(t).expect("插过的曲线");
        for lane in 0..3 {
            assert!(
                (before[lane] - after[lane]).abs() < 1e-12,
                "t={t} 第 {lane} 栏动了：{before:?} → {after:?}"
            );
        }
    }
}

/// **升阶不换几何**（有理那一档必须**在齐次坐标上**升，否则权会漂）。
#[test]
fn elevating_the_degree_leaves_the_curve_where_it_was() {
    let circle = circle();
    let higher = curve::elevate_degree(&circle, 4).expect("升到 4 次");
    assert_eq!(higher.degree, 4);
    // 四段 90°（每段是一条二次 Bézier），升到 4 次之后每段 5 个端点、段间共享一个
    // ⇒ `4 · 4 + 1 = 17`。
    assert_eq!(higher.count(), 17);
    for step in 0..=100 {
        let t = f64::from(step) / 100.0;
        let before = circle.point(t).expect("原曲线");
        let after = higher.point(t).expect("升过阶的曲线");
        for lane in 0..3 {
            assert!(
                (before[lane] - after[lane]).abs() < 1e-10,
                "t={t} 第 {lane} 栏动了：{before:?} → {after:?}"
            );
        }
    }
    // 升过阶之后它仍然是那个圆。
    for step in 0..=50 {
        let t = f64::from(step) / 50.0;
        let point = higher.point(t).expect("求值");
        let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
        assert!((radius - 1.0).abs() < 1e-10, "t={t} 处半径 {radius}");
    }
}

/// **平面法线是那张平面的法线**（圆在 xy 平面 ⇒ `[0, 0, 1]`）。
#[test]
fn the_plane_normal_of_a_planar_curve_is_its_plane() {
    let xy = curve::plane_normal(&circle());
    assert!(
        (xy[0]).abs() < 1e-12 && (xy[1]).abs() < 1e-12 && (xy[2] - 1.0).abs() < 1e-12,
        "{xy:?}"
    );
    let xz = curve::plane_normal(&curve::circle(1.0, "xz", [0.0; 3]).expect("xz 圆"));
    assert!(
        (xz[0]).abs() < 1e-12 && (xz[1] + 1.0).abs() < 1e-12 && xz[2].abs() < 1e-12,
        "{xz:?}"
    );
}

/// **圆载荷编解码逐位相同**（内容寻址的地基），且形状（点数 / 节点数）跟着走。
#[test]
fn the_curve_payload_round_trips_bit_for_bit() {
    let circle = circle();
    let bundle = <Curve as px_graph_schema::Build>::encode(&circle).expect("编码");
    assert_eq!(bundle.params["count"] as usize, circle.count());
    assert_eq!(bundle.params["knots"] as usize, circle.knots.len());
    assert_eq!(bundle.params["rational"], 1.0);
    let back = <Curve as px_graph_schema::Build>::decode(&bundle, "circle").expect("解码");
    assert_eq!(back.control, circle.control);
    assert_eq!(back.weights, circle.weights);
    assert_eq!(back.knots, circle.knots);
}

/// 一份**球面**：细分成网格之后每个顶点都落在半径上（球是有理二次精确表示的）。
#[test]
fn a_rational_sphere_tessellates_onto_the_radius() {
    // 判据在 schema 这一侧只量"曲面求值是不是精确的"：细分在实现库那一档。
    let surface: Surface = surface::sphere(1.0, [0.0; 3], 2).expect("造球");
    for i in 0..=8 {
        for j in 0..=8 {
            let u = f64::from(i) / 8.0;
            let v = f64::from(j) / 8.0;
            let point = surface.point(u, v).expect("求值");
            let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
            assert!(
                (radius - 1.0).abs() < 1e-12,
                "(u={u}, v={v}) 处的半径是 {radius}"
            );
        }
    }
}
