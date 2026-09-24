use px_nurbs_schema::PolylineData;
use px_nurbs_schema::ops::CurveTessellateGpu;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    CurveTessellateGpu,
    |p, i, _g| crate::ops::curve_tessellate_gpu::eval(p, i.curve.value())?
}

/// **曲线细分（GPU）**：逐点求值在 WGSL，编排与装配在 `crate`。
///
/// ⚠ 拿不到设备时**当场 Err**（硬失败，不回退 CPU）：选了这个节点就等于声明有卡。
pub fn eval(
    params: &params::tessellate::TessellateParams,
    source: &px_nurbs_schema::Curve,
) -> Result<PolylineData, String> {
    crate::tessellate_curve(source, params.tolerance, params.depth, params.segments)
}
