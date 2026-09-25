use px_field_schema::field::Field;
use px_graph_schema::{Cooked, px_op};
use px_volume_schema::VolumeData;

use crate::MeshData;
use crate::params;

#[derive(px_derive::PxInputs)]
pub struct CubeSphereInput {
    pub height: Cooked<Field>,
}

#[derive(px_derive::PxInputs)]
pub struct ProxyInput {
    pub volume: Cooked<VolumeData>,
}

px_op! {
    CubeSphere, "mesh.cubesphere", "px_mesh_op", params::cubesphere::Params, CubeSphereInput, MeshData
}

px_op! {
    Proxy, "mesh.proxy", "px_mesh_op", params::proxy::Params, ProxyInput, MeshData
}
