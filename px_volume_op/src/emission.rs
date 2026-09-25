use px_volume_schema::ops::Emission;

fn px_volume_op_gpu_emission(
    density: &px_volume_schema::VolumeData,
    stars: &px_sparse::StarField,
    params: &px_volume_schema::params::emission::EmissionParams,
) -> Result<px_volume_schema::VolumeData, String> {
    px_volume_gpu_op::bake_emission(density, stars, params)
}

px_graph_schema::px_body! {
    Emission,
    |p, i|
            px_volume_op_gpu_emission(i.volume.value().expect_single(), i.stars.value(), p)?
}
