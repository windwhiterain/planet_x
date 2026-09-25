use px_nurbs_schema::ops::Circle;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    Circle,
    |p, _i| crate::ops::circle::eval(p)?
}

pub fn eval(params: &params::curve::CircleParams) -> Result<px_nurbs_schema::Curve, String> {
    px_nurbs_schema::curve::circle(params.radius, &params.plane, params.center)
}
