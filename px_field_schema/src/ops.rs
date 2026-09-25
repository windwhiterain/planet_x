use px_graph_schema::{Cooked, px_op};

use crate::field::Field;
use crate::params;

#[derive(px_derive::PxInputs)]
pub struct FieldInput {
    pub field: Cooked<Field>,
}

#[derive(px_derive::PxInputs)]
pub struct FieldPairInput {
    pub field: Cooked<Field>,
    pub offset: Cooked<Field>,
}

#[derive(px_derive::PxInputs)]
pub struct FieldRemapInput {
    pub input: Cooked<Field>,
}

#[derive(px_derive::PxInputs)]
pub struct CratersInput {
    pub base: Cooked<Field>,
}

#[derive(px_derive::PxInputs)]
pub struct Warp3Input {
    pub field: Cooked<Field>,
    pub offset_a: Cooked<Field>,
    pub offset_b: Cooked<Field>,
    pub offset_c: Cooked<Field>,
}

px_op! {
    Fbm, "field.fbm", "px_field_op", params::fbm::Params, (), Field
}

px_op! {
    Ridged, "field.ridged", "px_field_op", params::ridged::Params, (), Field
}

px_op! {
    Gradient, "field.gradient", "px_field_op", params::gradient::Params, FieldInput, Field
}

px_op! {
    Warp, "field.warp", "px_field_op", params::warp::Params, FieldPairInput, Field
}

px_op! {
FieldRemap, "field.remap/inst", "px_field_op", params::RemapParams, FieldRemapInput, Field
}

px_op! {
    Craters, "field.craters", "px_field_op", params::CratersParams, CratersInput, Field
}

px_op! {
    Stamps, "field.stamps", "px_field_op", params::StampsParams, CratersInput, Field
}

px_op! {
    Fbm3, "field.fbm3", "px_field_op", params::Fbm3Params, (), Field
}

px_op! {
    Ridged3, "field.ridged3", "px_field_op", params::Ridged3Params, (), Field
}

px_op! {
    Warp3, "field.warp3", "px_field_op", params::Warp3Params, Warp3Input, Field
}
