use px_nurbs_schema::ops::CurveEval;
use px_nurbs_schema::params;
use px_nurbs_schema::point::PointData;

px_graph_schema::px_body! {
    CurveEval,
    |p, i| crate::ops::curve_eval::eval(p, i.curve.value())?
}

/// 在一个固定参数上求值（参数来自参数文件的 `u`）。
pub fn eval(
    params: &params::eval::EvalParams,
    curve: &px_nurbs_schema::Curve,
) -> Result<PointData, String> {
    let sample = curve.sample(params.u)?;
    let normal = px_nurbs_schema::curve::plane_normal(curve);
    Ok(PointData {
        point: sample.point,
        tangent: if params.tangent {
            sample.tangent
        } else {
            [0.0; 3]
        },
        normal,
        uv: [params.u, 0.0],
        has_tangent: params.tangent,
    })
}
