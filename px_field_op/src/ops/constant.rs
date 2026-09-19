use px_field_schema::field::{Field, GridField};
use px_field_schema::params;
use px_graph_schema::identity::fnv1a_sources;
use px_graph_schema::{Grid, OpDescriptor, OpKind};

pub const VERSION: u32 = 1;
pub const SOURCE_HASH: u64 = fnv1a_sources(&[
    include_str!("constant.rs"),
    include_str!("../noise.rs"),
    include_str!("../../../px_field_schema/src/field.rs"),
    include_str!("../../../px_field_schema/src/noise.rs"),
    include_str!("../../../px_field_schema/src/params.rs"),
    include_str!("../../../px_field_schema/src/payload.rs"),
]);
pub const INPUTS: &[&str] = &[];
pub const DESCRIPTOR: OpDescriptor = OpDescriptor {
    id: params::CONSTANT,
    version: VERSION,
    source_hash: SOURCE_HASH,
    inputs: INPUTS,
    kind: OpKind::Field,
};

pub fn eval(params: &params::constant::Params, _inputs: &[&Field], grid: Grid) -> Field {
    grid.filled(params.value)
}
