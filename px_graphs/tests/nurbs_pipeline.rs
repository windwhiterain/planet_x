//! NURBS 域的**端到端判据**：从"装载门"那一侧真叫起算子，一路到网格。
//!
//! ⚠ 算子走的是**声明那条真路**（`PxOp::render` → 运行时装载 `px_nurbs_op` → 调它的符号）：
//!   所以"声明 ↔ 实现"这条线在这里也有实证，而图程序不必链接实现库
//!   （那是 `tests/crate_graph.rs` 那道门看着的）。
//!
//! 靶子是**解析**的（不依赖 CAS 里有没有烘过什么）：圆上的点到圆心距离恒为半径、
//! 球细分成网格之后每个顶点仍在球面上、闭合网格没有开口边、同一份参数跑两次逐位一样。

use px_field_schema::field::Projection;
use px_graph_schema::{Cooked, Grid, PxOp};
use px_nurbs_schema::ops as nurbs;
use px_nurbs_schema::params as nurbs_params;
use px_nurbs_schema::{Curve, PointData, Surface};

/// 画布：签名要一个 `Grid`（NURBS 那一档与画布无关，见 `RESOLUTION_IS_CANVAS`）。
fn grid() -> Grid {
    Grid {
        width: 4,
        height: 4,
        projection: Projection::CubeMap,
    }
}

/// 一条假键：这里的输入不是缓存里的节点，只是把值包成"已经拿到手的节点"那个形状。
fn cooked<P>(value: P) -> Cooked<P> {
    Cooked::new([0; 32], value, false, 0, 0)
}

fn circle(radius: f64) -> Curve {
    nurbs::Circle
        .render(
            &nurbs_params::curve::CircleParams {
                radius,
                plane: "xy".to_string(),
                center: [0.0; 3],
            },
            &(),
            grid(),
        )
        .expect("造圆失败")
}

fn sphere(radius: f64) -> Surface {
    nurbs::Sphere
        .render(
            &nurbs_params::curve::SphereParams {
                radius,
                center: [0.0; 3],
                rings: 2,
            },
            &(),
            grid(),
        )
        .expect("造球失败")
}

/// **一条最小图**：`nurbs.sphere → nurbs.surface.tessellate`，量三件事：
/// 顶点还在球面上、网格闭合、顶点数不为零。
#[test]
fn a_sphere_reaches_a_watertight_mesh_through_the_loading_gate() {
    let ball = sphere(3.0);
    let mesh = nurbs::SurfaceTessellate
        .render(
            &nurbs_params::tessellate::TessellateParams {
                tolerance: 1e-2,
                depth: 3,
                segments: 2,
            },
            &nurbs::SurfaceInput {
                surface: cooked(ball),
            },
            grid(),
        )
        .expect("细分失败");

    assert!(mesh.vertices() > 32, "网格太寒酸了，这个判据没在测东西");
    for vertex in 0..mesh.vertices() {
        let point = &mesh.positions[vertex * 3..vertex * 3 + 3];
        let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
        assert!(
            (radius - 3.0).abs() < 1e-4,
            "第 {vertex} 个顶点离球心 {radius}（应当是 3）"
        );
    }
    // 闭合：每条边恰好被两个三角形用到（欧拉数也顺带是 2）。
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
    println!(
        "球网格：{} 顶点 / {} 三角形（{} 条边）｜开口 {open}、非流形 {nonmanifold}",
        mesh.vertices(),
        mesh.triangles(),
        edges.len(),
    );
    assert_eq!(open, 0, "有 {open} 条开口边");
    assert_eq!(nonmanifold, 0, "有 {nonmanifold} 条非流形边");
    assert_eq!(
        mesh.vertices() as i64 - edges.len() as i64 + mesh.triangles() as i64,
        2,
        "欧拉数不是 2 ⇒ 这不是一张闭合的球面网格"
    );
}

/// **另一条最小图**：`nurbs.circle → nurbs.curve.eval` 与 `nurbs.circle → nurbs.curve.tessellate`。
#[test]
fn a_circle_evaluates_and_tessellates_through_the_loading_gate() {
    let circle = circle(2.0);
    let point: PointData = nurbs::CurveEval
        .render(
            &nurbs_params::eval::EvalParams {
                u: 0.25,
                v: 0.0,
                tangent: true,
            },
            &nurbs::CurveInput {
                curve: cooked(circle.clone()),
            },
            grid(),
        )
        .expect("求值失败");
    // 四分之一圈处就是 (0, R)。
    assert!(
        point.point[0].abs() < 1e-12 && (point.point[1] - 2.0).abs() < 1e-12,
        "t=0.25 处应当是 (0, 2, 0)，给的是 {:?}",
        point.point
    );
    assert!(point.has_tangent, "要了切向量却没交出来");
    let dot = point.normal[0] * point.tangent[0] + point.normal[1] * point.tangent[1];
    assert!(dot.abs() < 1e-12, "切向量与平面法线的点积是 {dot}");

    let mesh = nurbs::CurveTessellate
        .render(
            &nurbs_params::tessellate::TessellateParams {
                tolerance: 1e-3,
                depth: 6,
                segments: 4,
            },
            &nurbs::CurveInput {
                curve: cooked(circle),
            },
            grid(),
        )
        .expect("细分失败");
    for vertex in 0..mesh.vertices() {
        let point = &mesh.positions[vertex * 3..vertex * 3 + 3];
        let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
        assert!(
            (radius - 2.0).abs() < 1e-5,
            "第 {vertex} 个顶点离圆心 {radius}（应当是 2）"
        );
    }
}

/// **可复现**：同一份参数跑两次逐位一样（缓存是「键 = 内容」，位置与法线都不许有随机性）。
#[test]
fn the_tessellation_is_reproducible() {
    let params = nurbs_params::tessellate::TessellateParams {
        tolerance: 1e-2,
        depth: 3,
        segments: 2,
    };
    let first = nurbs::SurfaceTessellate
        .render(
            &params,
            &nurbs::SurfaceInput {
                surface: cooked(sphere(1.0)),
            },
            grid(),
        )
        .expect("细分失败");
    let second = nurbs::SurfaceTessellate
        .render(
            &params,
            &nurbs::SurfaceInput {
                surface: cooked(sphere(1.0)),
            },
            grid(),
        )
        .expect("细分失败");
    assert_eq!(first.positions, second.positions, "顶点位置不可复现");
    assert_eq!(first.normals, second.normals, "法线不可复现");
    assert_eq!(first.uvs, second.uvs);
    assert_eq!(first.indices, second.indices);
}
