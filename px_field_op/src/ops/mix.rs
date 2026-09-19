use px_field_schema::field::{Field, GridField};
use px_field_schema::params;
use px_graph_schema::identity::fnv1a_sources;
use px_graph_schema::{Grid, OpDescriptor, OpKind};

pub const VERSION: u32 = 1;
pub const SOURCE_HASH: u64 = fnv1a_sources(&[
    include_str!("mix.rs"),
    include_str!("../noise.rs"),
    include_str!("../../../px_field_schema/src/field.rs"),
    include_str!("../../../px_field_schema/src/noise.rs"),
    include_str!("../../../px_field_schema/src/params.rs"),
    include_str!("../../../px_field_schema/src/payload.rs"),
]);
pub const INPUTS: &[&str] = &["a", "b", "mask"];
pub const DESCRIPTOR: OpDescriptor = OpDescriptor {
    id: params::MIX,
    version: VERSION,
    source_hash: SOURCE_HASH,
    inputs: INPUTS,
    kind: OpKind::Field,
};

pub fn eval(params: &params::mix::Params, inputs: &[&Field], grid: Grid) -> Field {
    let (a, b, mask) = (inputs[0], inputs[1], inputs[2]);
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let weight = (mask.at(x, y) + params.bias).clamp(0.0, 1.0);
            let value = a.at(x, y) * (1.0 - weight) + b.at(x, y) * weight;
            field.set(x, y, value);
        }
    }
    field
}
