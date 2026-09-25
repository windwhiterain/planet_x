use px_volume_schema::ops::SkyNebula;

fn px_volume_op_gpu(
    emission: &px_volume_schema::VolumeData,
    stars: &px_sparse::StarField,
    params: &px_volume_schema::params::sky::SkyParams,
) -> Result<px_volume_schema::TextureData, String> {
    px_volume_gpu_op::raymarch_sky(emission, stars, params)
}

px_graph_schema::px_body! {
    SkyNebula,
    |p, i| px_volume_op_gpu(i.volume.value(), i.stars.value(), p)?
}
