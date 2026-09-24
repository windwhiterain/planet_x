use px_nurbs_schema::ops::CurveHodograph;
use px_nurbs_schema::params;
use px_nurbs_schema::point::PointData;

px_graph_schema::px_body! {
    CurveHodograph,
    |p, i, _g| crate::ops::curve_hodograph::eval(p, i.curve.value())?
}

/// **一阶导（hodograph）**：`t → 切向量`。
///
/// ⚠ 切向量放在 `tangent` 那一栏、`point` 仍放曲线上的点：读的人要的是"在哪、朝哪"，
///   而 `tangent` 是这套 `PointData` 里语义最准的一格（`has_tangent` 跟着置位）。
pub fn eval(
    params: &params::eval::EvalParams,
    curve: &px_nurbs_schema::Curve,
) -> Result<PointData, String> {
    let sample = curve.sample(params.u)?;
    Ok(PointData {
        point: sample.point,
        tangent: sample.tangent,
        normal: px_nurbs_schema::curve::plane_normal(curve),
        uv: [params.u, 0.0],
        has_tangent: true,
    })
}
