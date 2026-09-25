use px_nurbs_schema::ops::Sphere;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    Sphere,
    |p, _i| crate::ops::sphere::eval(p)?
}

pub fn eval(params: &params::curve::SphereParams) -> Result<px_nurbs_schema::Surface, String> {
    px_nurbs_schema::surface::sphere(params.radius, params.center, params.rings)
}
