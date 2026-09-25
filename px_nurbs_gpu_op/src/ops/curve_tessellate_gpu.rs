use px_nurbs_schema::PolylineData;
use px_nurbs_schema::ops::CurveTessellateGpu;
use px_nurbs_schema::params;

px_graph_schema::px_body! {
    CurveTessellateGpu,
    |p, i| crate::ops::curve_tessellate_gpu::eval(p, i.curve.value())?
}

pub fn eval(
    params: &params::tessellate::TessellateParams,
    source: &px_nurbs_schema::Curve,
) -> Result<PolylineData, String> {
    crate::tessellate_curve(source, params.tolerance, params.depth, params.segments)
}
