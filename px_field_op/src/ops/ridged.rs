use px_field_schema::field::Field;
use px_field_schema::noise::FbmSettings;
use px_field_schema::ops::Ridged;
use px_field_schema::params;

use crate::noise;

px_graph_schema::px_body! { Ridged, |p, _i| crate::ops::ridged::eval(p, &[]) }

pub fn eval(params: &params::ridged::Params, _inputs: &[&Field]) -> Field {
    let settings = FbmSettings {
        frequency: params.frequency,
        octaves: params.octaves,
        lacunarity: params.lacunarity,
        gain: params.gain,
        seed: params.seed,
    };
    let mut field = params.shape.filled(0.0);
    for y in 0..params.shape.height {
        for x in 0..params.shape.width {
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
