use px_nurbs_schema::ops::SurfaceInsert;
use px_nurbs_schema::params;
use px_nurbs_schema::surface::Along;

px_graph_schema::px_body! {
    SurfaceInsert,
    |p, i| crate::ops::surface_insert::eval(p, i.surface.value())?
}

pub fn eval(
    params: &params::insert::InsertParams,
    source: &px_nurbs_schema::Surface,
) -> Result<px_nurbs_schema::Surface, String> {
    let along = match params.along.as_str() {
        "u" => Along::U,
        "v" => Along::V,
        other => return Err(format!("曲面插入的方向只能是 `u` 或 `v`，给的是 `{other}`")),
    };
    px_nurbs_schema::surface::insert_knot(source, along, params.t, params.times as usize)
}
