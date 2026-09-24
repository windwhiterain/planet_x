use px_nurbs_schema::ops::Sphere;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    Sphere,
    |p, _i, _g| crate::ops::sphere::eval(p)?
}

/// **有理二次球面**（精确球）：构造住在 schema 里（`surface::sphere`）。
pub fn eval(params: &params::curve::SphereParams) -> Result<px_nurbs_schema::Surface, String> {
    px_nurbs_schema::surface::sphere(params.radius, params.center, params.rings)
}
