use px_nurbs_schema::ops::CurveHodograph;
use px_nurbs_schema::params;
use px_nurbs_schema::point::PointData;

px_graph_schema::px_body! {
    CurveHodograph,
    |p, i| crate::ops::curve_hodograph::eval(p, i.curve.value())?
}

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
