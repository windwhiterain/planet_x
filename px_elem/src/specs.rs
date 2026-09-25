use crate::constant::ConstantParams;
use crate::fuse::{FuseInput, FuseParams};
use crate::mix::{MixInput, MixParams};
use crate::remap::{RemapInput, RemapParams};

crate::px_elem_specs! {
    Constant, ConstantParams, (), "field.constant",
        source: "px_elem/body/constant.rs", roots: &[],
        shape: |params: &ConstantParams, _inputs: &()| params.shape;

    Mix, MixParams, MixInput, "field.mix",
        source: "px_elem/body/mix.rs", roots: &[],
        shape: |_params: &MixParams, inputs: &MixInput| crate::like(inputs.a.value());

    Remap, RemapParams, RemapInput, "field.remap",
        source: "px_elem/body/remap.rs", roots: &["px_field_alg"],
        shape: |_params: &RemapParams, inputs: &RemapInput| crate::like(inputs.field.value());

    Fuse, FuseParams, FuseInput, "field.fuse",
        source: "px_elem/body/fuse.rs", roots: &["px_field_alg"],
        shape: |_params: &FuseParams, inputs: &FuseInput| crate::like(inputs.a.value());
}
