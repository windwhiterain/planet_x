use px_field_schema::field::{Field, GridField};
use px_field_schema::params;
use px_graph_schema::identity::fnv1a_sources;
use px_graph_schema::{Grid, OpDescriptor, OpKind};

pub const VERSION: u32 = 1;
pub const SOURCE_HASH: u64 = fnv1a_sources(&[
    include_str!("remap.rs"),
    include_str!("../noise.rs"),
    include_str!("../../../px_field_schema/src/field.rs"),
    include_str!("../../../px_field_schema/src/noise.rs"),
    include_str!("../../../px_field_schema/src/params.rs"),
    include_str!("../../../px_field_schema/src/payload.rs"),
]);
pub const INPUTS: &[&str] = &["input"];
pub const DESCRIPTOR: OpDescriptor = OpDescriptor {
    id: params::REMAP,
    version: VERSION,
    source_hash: SOURCE_HASH,
    inputs: INPUTS,
    kind: OpKind::Field,
};

pub fn eval(params: &params::remap::Params, inputs: &[&Field], grid: Grid) -> Field {
    let input = inputs[0];
    let span = params.in_max - params.in_min;
    let inv_span = if span.abs() < f32::EPSILON {
        0.0
    } else {
        1.0 / span
    };
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let mut t = ((input.at(x, y) - params.in_min) * inv_span).clamp(0.0, 1.0);
            if params.smooth {
                t = t * t * (3.0 - 2.0 * t);
            }
            field.set(x, y, params.out_min + t * (params.out_max - params.out_min));
        }
    }
    field
}
