use px_nurbs_schema::ops::CurveTessellate;
use px_nurbs_schema::params;
use px_protocol::art::PolylineData;

px_graph_schema::px_body! {
    CurveTessellate,
    |p, i| crate::ops::curve_tessellate::eval(p, i.curve.value())?
}

/// **曲线细分**：按弦误差摊成折线（算法在 `crate::tessellate`）。
pub fn eval(
    params: &params::tessellate::TessellateParams,
    source: &px_nurbs_schema::Curve,
) -> Result<PolylineData, String> {
    crate::tessellate::curve(source, params.tolerance, params.depth, params.segments)
}
