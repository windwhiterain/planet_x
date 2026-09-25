use px_nurbs_schema::ops::CurveAt;
use px_nurbs_schema::params;
use px_nurbs_schema::point::PointData;

px_graph_schema::px_body! {
    CurveAt,
    |p, i| crate::ops::curve_at::eval(p, i.curve.value(), i.point.value())?
}

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
