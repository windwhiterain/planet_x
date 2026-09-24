use px_nurbs_schema::ops::CurveAt;
use px_nurbs_schema::params;
use px_nurbs_schema::point::PointData;

px_graph_schema::px_body! {
    CurveAt,
    |p, i, _g| crate::ops::curve_at::eval(p, i.curve.value(), i.point.value())?
}

/// **在别的节点给的参数上求值**（"在哪求值"因此可以由别的节点算出来）。
///
/// ⚠ 只读上游那个 `PointData` 的 `uv[0]`：它是"参数"这一档的载体，别的栏不管
///   （所以"沿曲线撒样点"只要接一条参数序列进来）。
pub fn eval(
    params: &params::eval::EvalParams,
    curve: &px_nurbs_schema::Curve,
    at: &PointData,
) -> Result<PointData, String> {
    let t = at.uv[0];
    let sample = curve.sample(t)?;
    Ok(PointData {
        point: sample.point,
        tangent: if params.tangent {
            sample.tangent
        } else {
            [0.0; 3]
        },
        normal: px_nurbs_schema::curve::plane_normal(curve),
        uv: [t, 0.0],
        has_tangent: params.tangent,
    })
}
