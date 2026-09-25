use px_nurbs_schema::MeshData;
use px_nurbs_schema::ops::SurfaceTessellateGpu;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    SurfaceTessellateGpu,
    |p, i| crate::ops::surface_tessellate_gpu::eval(p, i.surface.value())?
}

pub fn eval(
    params: &params::tessellate::TessellateParams,
    source: &px_nurbs_schema::Surface,
) -> Result<MeshData, String> {
    crate::tessellate_surface(source, params.tolerance, params.depth, params.segments)
}
