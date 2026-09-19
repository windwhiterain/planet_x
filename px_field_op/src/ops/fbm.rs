use px_field_schema::field::{Field, GridField};
use px_field_schema::noise::FbmSettings;
use px_field_schema::params;
use px_graph_schema::Grid;

use crate::noise;

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
