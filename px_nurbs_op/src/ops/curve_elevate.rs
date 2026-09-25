use px_nurbs_schema::curve;
use px_nurbs_schema::ops::CurveElevate;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    CurveElevate,
    |p, i| crate::ops::curve_elevate::eval(p, i.curve.value())?
}

pub fn eval(
    params: &params::elevate::ElevateParams,
    source: &px_nurbs_schema::Curve,
) -> Result<px_nurbs_schema::Curve, String> {
    curve::elevate_degree(source, params.degree as usize)
}
