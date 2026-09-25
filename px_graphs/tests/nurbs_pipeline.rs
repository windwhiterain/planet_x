use px_graph_schema::{Cooked, PxOp};
use px_nurbs_schema::ops as nurbs;
use px_nurbs_schema::params as nurbs_params;
use px_nurbs_schema::{Curve, PointData, Surface};

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
        )
        .expect("造球失败")
}

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
        )
        .expect("求值失败");
    assert!(
        point.point[0].abs() < 1e-12 && (point.point[1] - 2.0).abs() < 1e-12,
        "t=0.25 处应当是 (0, 2, 0)，给的是 {:?}",
        point.point
    );
    assert!(point.has_tangent, "要了切向量却没交出来");
    let dot = point.normal[0] * point.tangent[0] + point.normal[1] * point.tangent[1];
    assert!(dot.abs() < 1e-12, "切向量与平面法线的点积是 {dot}");

    let line = nurbs::CurveTessellate
        .render(
            &nurbs_params::tessellate::TessellateParams {
                tolerance: 1e-3,
                depth: 6,
                segments: 4,
            },
            &nurbs::CurveInput {
                curve: cooked(circle),
            },
        )
        .expect("细分失败");
    for vertex in 0..line.vertices() {
        let point = &line.positions[vertex * 3..vertex * 3 + 3];
        let radius = (point[0] * point[0] + point[1] * point[1]).sqrt();
        assert!(
            (radius - 2.0).abs() < 1e-5,
            "第 {vertex} 个顶点离圆心 {radius}（应当是 2）"
        );
    }
    assert_eq!(line.segments(), line.vertices(), "闭合折线的段数 = 顶点数");
    assert_eq!(
        &line.indices[line.indices.len() - 2..],
        &[line.vertices() as u32 - 1, 0],
        "最后一段没有接回第 0 个顶点"
    );
}

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
        )
        .expect("细分失败");
    let second = nurbs::SurfaceTessellate
        .render(
            &params,
            &nurbs::SurfaceInput {
                surface: cooked(sphere(1.0)),
            },
        )
        .expect("细分失败");
    assert_eq!(first.positions, second.positions, "顶点位置不可复现");
    assert_eq!(first.normals, second.normals, "法线不可复现");
    assert_eq!(first.uvs, second.uvs);
    assert_eq!(first.indices, second.indices);
}

#[test]
fn the_gpu_tessellation_reaches_a_watertight_mesh_through_the_loading_gate() {
    let ball = sphere(3.0);
    let rendered = nurbs::SurfaceTessellateGpu.render(
        &nurbs_params::tessellate::TessellateParams {
            tolerance: 1e-2,
            depth: 3,
            segments: 2,
        },
        &nurbs::SurfaceInput {
            surface: cooked(ball),
        },
    );
    // A device is required, not optional: `px_graphs` is a default member, so returning here on a
    // machine without one would report the fast chain green while this comparison never ran
    // (docs/invariants.md, "A skipped check is not a passing check"). `px_nurbs_op` reports the
    // stack's message through `-> Result<_, String>`, which is all this side can see — the
    // implementation library is loaded at runtime, so the graph side cannot link the GPU crate.
    let mesh = match rendered {
        Ok(mesh) => mesh,
        Err(err) => panic!("this check requires a working GPU: {err}"),
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
    println!(
        "GPU 球网格：{} 顶点 / {} 三角形（{} 条边）｜开口 {open}、非流形 {nonmanifold}",
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
