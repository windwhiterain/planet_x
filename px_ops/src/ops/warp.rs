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
}

impl Default for Params {
    fn default() -> Self {
        Self {
            strength: 0.10,
            lateral: 0.35,
        }
    }
}

impl FieldOp for Warp {
    type Params = Params;
    const ID: &'static str = "field.warp";
    const VERSION: u32 = 1;
    const SOURCE_HASH: u64 = fnv1a(include_str!("warp.rs"));
    const INPUTS: &'static [&'static str] = &["input", "warp"];

    fn eval(params: &Params, inputs: &[&Field], size: (u32, u32)) -> Field {
        let (input, warp) = (inputs[0], inputs[1]);
        let mut field = Field::filled(size.0, size.1, 0.0);
        let scale = params.strength * size.0 as f32;
        let vertical = params.lateral * size.0 as f32 / size.1 as f32;
        let half_x = size.0 / 3;
        let half_y = (size.1 / 3).max(1);

        for y in 0..size.1 {
            for x in 0..size.0 {
                let first = warp.at(x, y) - 0.5;
                let second = warp.at((x + half_x) % size.0, (y + half_y) % size.1) - 0.5;
                let offset_x = first * scale;
                let offset_y = second * vertical * scale;
                field.set(
                    x,
                    y,
                    input.sample_bilinear(x as f32 + offset_x, y as f32 + offset_y),
                );
            }
        }
        field
    }
}
