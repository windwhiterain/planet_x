use px_nurbs_schema::MeshData;
use px_nurbs_schema::ops::SurfaceTessellateGpu;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    SurfaceTessellateGpu,
    |p, i, _g| crate::ops::surface_tessellate_gpu::eval(p, i.surface.value())?
}

/// **曲面细分（GPU）**：逐格求值在 WGSL，编排与装配在 `crate`。
///
/// ⚠ 拿不到设备时**当场 Err**（硬失败，不回退 CPU）：选了这个节点就等于声明有卡。
pub fn eval(
    params: &params::tessellate::TessellateParams,
    source: &px_nurbs_schema::Surface,
) -> Result<MeshData, String> {
    crate::tessellate_surface(source, params.tolerance, params.depth, params.segments)
}
