use px_nurbs_schema::ops::SurfaceElevate;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    SurfaceElevate,
    |p, i| crate::ops::surface_elevate::eval(p, i.surface.value())?
}

pub fn eval(
    params: &params::elevate::ElevateParams,
    source: &px_nurbs_schema::Surface,
) -> Result<px_nurbs_schema::Surface, String> {
    let target = params.degree as usize;
    let along_u =
        px_nurbs_schema::surface::elevate(source, px_nurbs_schema::surface::Along::U, target)?;
    px_nurbs_schema::surface::elevate(&along_u, px_nurbs_schema::surface::Along::V, target)
}
