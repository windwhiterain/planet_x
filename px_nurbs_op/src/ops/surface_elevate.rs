use px_nurbs_schema::ops::SurfaceElevate;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    SurfaceElevate,
    |p, i| crate::ops::surface_elevate::eval(p, i.surface.value())?
}

/// 升阶（曲面）：两向各按曲线那套升一次（张量积 ⇒ 两次一维升阶就够）。
///
/// ⚠ 顺序（先 u 后 v）**不影响结果**：两个方向的升阶各自只动自己那一维的基，
///   所以它们可交换 —— 这里不必挑。
pub fn eval(
    params: &params::elevate::ElevateParams,
    source: &px_nurbs_schema::Surface,
) -> Result<px_nurbs_schema::Surface, String> {
    let target = params.degree as usize;
    let along_u =
        px_nurbs_schema::surface::elevate(source, px_nurbs_schema::surface::Along::U, target)?;
    px_nurbs_schema::surface::elevate(&along_u, px_nurbs_schema::surface::Along::V, target)
}
