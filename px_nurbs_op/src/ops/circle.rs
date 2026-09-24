use px_nurbs_schema::ops::Circle;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    Circle,
    |p, _i, _g| crate::ops::circle::eval(p)?
}

/// **有理二次整圆**（精确圆，不是拟合）：构造住在 schema 里（`curve::circle`），
/// 这一档只把参数接上去。
pub fn eval(params: &params::curve::CircleParams) -> Result<px_nurbs_schema::Curve, String> {
    px_nurbs_schema::curve::circle(params.radius, &params.plane, params.center)
}
