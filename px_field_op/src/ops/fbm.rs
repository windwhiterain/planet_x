use px_field_schema::field::{Field, GridField};
use px_field_schema::noise::FbmSettings;
use px_field_schema::params;
use px_graph_schema::identity::fnv1a_sources;
use px_graph_schema::{Grid, OpDescriptor, OpKind};

use crate::noise;

pub const VERSION: u32 = 4;
pub const SOURCE_HASH: u64 = fnv1a_sources(&[
    include_str!("fbm.rs"),
    include_str!("../noise.rs"),
    include_str!("../../../px_field_schema/src/field.rs"),
    include_str!("../../../px_field_schema/src/noise.rs"),
    include_str!("../../../px_field_schema/src/params.rs"),
    include_str!("../../../px_field_schema/src/payload.rs"),
]);
pub const INPUTS: &[&str] = &[];
pub const DESCRIPTOR: OpDescriptor = OpDescriptor {
    id: params::FBM,
    version: VERSION,
    source_hash: SOURCE_HASH,
    inputs: INPUTS,
    kind: OpKind::Field,
};

pub fn eval(params: &params::fbm::Params, _inputs: &[&Field], grid: Grid) -> Field {
    let settings = FbmSettings {
        frequency: params.frequency,
        octaves: params.octaves,
        lacunarity: params.lacunarity,
        gain: params.gain,
        seed: params.seed,
    };
    let mut field = grid.filled(0.0);
    for y in 0..grid.height {
        for x in 0..grid.width {
            let (u, v) = field.uv(x, y);
            let value = if params.spherical {
                noise::fbm_3(field.direction(x, y), &settings)
            } else {
                noise::fbm(u * params.aspect, v, &settings)
            };
            field.set(x, y, value);
        }
    }
    field
}
