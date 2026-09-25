use px_volume_schema::ops::CloudCoarse;

px_graph_schema::px_body! {
    CloudCoarse,
    |p, i| px_volume_alg::eval_sampled(p, i.coverage.value())
}
