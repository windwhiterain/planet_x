use px_nurbs_schema::ops::CurveInsert;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    CurveInsert,
    |p, i| crate::ops::curve_insert::eval(p, i.curve.value())?
}

pub fn eval(
    params: &params::insert::InsertParams,
    source: &px_nurbs_schema::Curve,
) -> Result<px_nurbs_schema::Curve, String> {
    px_nurbs_schema::curve::insert_knot(source, params.t, params.times as usize)
}
