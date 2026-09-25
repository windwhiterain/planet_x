use px_field_schema::field::Field;
use px_graph_schema::{Cooked, px_op};
use px_protocol::art::TextureData;
use px_sparse::StarField;

use crate::VolumeData;
use crate::params;

#[derive(px_derive::PxInputs)]
pub struct CloudCoarseInput {
    pub coverage: Cooked<Field>,
}

#[derive(px_derive::PxInputs)]
pub struct DensityInput {
    pub density: Cooked<Field>,
}

#[derive(px_derive::PxInputs)]
pub struct EmissionInput {
    pub volume: Cooked<VolumeData>,
    pub stars: Cooked<StarField>,
}

#[derive(px_derive::PxInputs)]
pub struct StarsInput {
    pub volume: Cooked<VolumeData>,
}

#[derive(px_derive::PxInputs)]
pub struct SkyInput {
    pub volume: Cooked<VolumeData>,
    pub stars: Cooked<StarField>,
}

pub type VolumeOut = Cooked<VolumeData>;

pub type StarOut = Cooked<StarField>;

px_op! {
    Stars, "sky.stars", "px_volume_op", params::stars::StarsParams, StarsInput, StarField
}

px_op! {
    CloudCoarse, "cloud.coarse", "px_volume_op", params::Params, CloudCoarseInput, VolumeData
}

px_op! {
    Density, "cloud.density", "px_volume_op", params::density::DensityParams, DensityInput, VolumeData
}

px_op! {
    Emission, "cloud.emission", "px_volume_op", params::emission::EmissionParams, EmissionInput, VolumeData
}

px_op! {
    SkyNebula, "sky.nebula", "px_volume_op", params::sky::SkyParams, SkyInput, TextureData
}
