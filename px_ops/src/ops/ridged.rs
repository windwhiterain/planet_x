use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::noise::{self, FbmSettings, fnv1a};
use crate::FieldOp;

pub struct Ridged;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub frequency: f32,
    pub octaves: u32,
    pub lacunarity: f32,
    pub gain: f32,
    pub seed: u32,
    pub aspect: f32,
    pub sharpness: f32,
    pub spherical: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            frequency: 9.0,
            octaves: 5,
            lacunarity: 2.0,
            gain: 0.5,
            seed: 21,
            aspect: 2.0,
            sharpness: 1.6,
            spherical: true,
        }
    }
}

impl FieldOp for Ridged {
    type Params = Params;
    const ID: &'static str = "field.ridged";
    const VERSION: u32 = 2;
    const SOURCE_HASH: u64 = fnv1a(include_str!("ridged.rs"));
    const INPUTS: &'static [&'static str] = &[];

    fn eval(params: &Params, _inputs: &[&Field], size: (u32, u32)) -> Field {
        let settings = FbmSettings {
            frequency: params.frequency,
            octaves: params.octaves,
            lacunarity: params.lacunarity,
            gain: params.gain,
            seed: params.seed,
        };
        let mut field = Field::filled(size.0, size.1, 0.0);
        for y in 0..size.1 {
            for x in 0..size.0 {
                let (u, v) = field.uv(x, y);
                let value = if params.spherical {
                    noise::ridged_3(noise::direction(u, v), &settings, params.sharpness)
                } else {
                    noise::ridged(u * params.aspect, v, &settings, params.sharpness)
                };
                field.set(x, y, value);
            }
        }
        field
    }
}
