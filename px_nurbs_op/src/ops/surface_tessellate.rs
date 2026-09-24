use px_nurbs_schema::ops::SurfaceTessellate;
use px_nurbs_schema::params;
use px_protocol::art::MeshData;

px_graph_schema::px_body! {
    SurfaceTessellate,
    |p, i, _g| crate::ops::surface_tessellate::eval(p, i.surface.value())?
}

/// **曲面细分**：按弦误差摊成三角网格（算法在 `crate::tessellate`）。
pub fn eval(
    params: &params::tessellate::TessellateParams,
    source: &px_nurbs_schema::Surface,
) -> Result<MeshData, String> {
    crate::tessellate::surface(source, params.tolerance, params.depth, params.segments)
}
