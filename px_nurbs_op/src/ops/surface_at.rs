use px_nurbs_schema::ops::SurfaceAt;
use px_nurbs_schema::params;
use px_nurbs_schema::point::PointData;

px_graph_schema::px_body! {
    SurfaceAt,
    |p, i| crate::ops::surface_at::eval(p, i.surface.value(), i.point.value())?
}

/// **在别的节点给的参数上求值**：只读上游那个 `PointData` 的 `uv`。
pub fn eval(
    params: &params::eval::EvalParams,
    surface: &px_nurbs_schema::Surface,
    at: &PointData,
) -> Result<PointData, String> {
    let (u, v) = (at.uv[0], at.uv[1]);
    let patch = surface.patch(u, v)?;
    Ok(PointData {
        point: patch.point,
        tangent: if params.tangent { patch.du } else { [0.0; 3] },
        normal: patch.normal.unwrap_or([0.0, 1.0, 0.0]),
        uv: [u, v],
        has_tangent: params.tangent,
    })
}
