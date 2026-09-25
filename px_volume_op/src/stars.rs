use px_volume_schema::ops::Stars;

px_graph_schema::px_body! {
    Stars,
    |p, i|
        px_volume_alg::bake_stars(p, Some(i.volume.value().expect_single()))?
}
