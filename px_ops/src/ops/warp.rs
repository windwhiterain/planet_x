use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::noise::fnv1a;
use crate::FieldOp;

pub struct Warp;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub strength: f32,
    pub lateral: f32,
    pub spherical: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            strength: 0.40,
            lateral: 0.50,
            spherical: true,
        }
    }
}

impl FieldOp for Warp {
    type Params = Params;
    const ID: &'static str = "field.warp";
    const VERSION: u32 = 2;
    const SOURCE_HASH: u64 = fnv1a(include_str!("warp.rs"));
    const INPUTS: &'static [&'static str] = &["input", "warp"];

    fn eval(params: &Params, inputs: &[&Field], size: (u32, u32)) -> Field {
        let (input, warp) = (inputs[0], inputs[1]);
        let mut field = Field::filled(size.0, size.1, 0.0);
        let half_x = size.0 / 3;
        let half_y = (size.1 / 3).max(1);

        for y in 0..size.1 {
            for x in 0..size.0 {
                let first = warp.at(x, y) - 0.5;
                let second = warp.at((x + half_x) % size.0, (y + half_y) % size.1) - 0.5;

                let (offset_x, offset_y) = if params.spherical {
                    let (u, v) = field.uv(x, y);
                    let sine = (v * std::f32::consts::PI).sin();
                    let ring = (sine * sine + 0.16).sqrt();
                    let angle = first * params.strength;
                    (
                        (u + angle / (std::f32::consts::TAU * ring)) * size.0 as f32,
                        (v + second * params.lateral * params.strength / std::f32::consts::PI)
                            * (size.1 as f32 - 1.0),
                    )
                } else {
                    let scale = params.strength * size.0 as f32;
                    (
                        x as f32 + first * scale,
                        y as f32 + second * params.lateral * scale * size.0 as f32
                            / size.1 as f32,
                    )
                };

                field.set(x, y, input.sample_bilinear(offset_x, offset_y));
            }
        }
        field
    }
}
