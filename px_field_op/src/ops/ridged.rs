use px_field_schema::field::{Field, GridField};
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Ridged;
use px_field_schema::params;
use px_graph_schema::Grid;

use crate::noise;

px_graph_schema::px_body! { Ridged, |p, _i, g| crate::ops::ridged::eval(p, &[], g) }

pub fn eval(params: &params::ridged::Params, _inputs: &[&Field], grid: Grid) -> Field {
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
                noise::ridged_3(field.direction(x, y), &settings, params.sharpness)
            } else {
                noise::ridged(u * params.aspect, v, &settings, params.sharpness)
            };
            field.set(x, y, value);
        }
    }
    field
}
