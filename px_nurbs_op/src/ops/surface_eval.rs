use px_nurbs_schema::ops::SurfaceEval;
use px_nurbs_schema::params;
use px_nurbs_schema::point::PointData;

px_graph_schema::px_body! {
    SurfaceEval,
    |p, i| crate::ops::surface_eval::eval(p, i.surface.value())?
}

pub fn eval(
    params: &params::eval::EvalParams,
    surface: &px_nurbs_schema::Surface,
) -> Result<PointData, String> {
    let patch = surface.patch(params.u, params.v)?;
    Ok(PointData {
        point: patch.point,
        tangent: if params.tangent { patch.du } else { [0.0; 3] },
        normal: patch.normal.unwrap_or([0.0, 1.0, 0.0]),
        uv: [params.u, params.v],
        has_tangent: params.tangent,
    })
}
