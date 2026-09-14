use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::Grid;
use crate::noise::fnv1a;
use crate::FieldOp;

pub struct Remap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub in_min: f32,
    pub in_max: f32,
    pub out_min: f32,
    pub out_max: f32,
    pub smooth: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            in_min: 0.0,
            in_max: 1.0,
            out_min: 0.0,
            out_max: 1.0,
            smooth: true,
        }
    }
}

impl FieldOp for Remap {
    type Params = Params;
    const ID: &'static str = "field.remap";
    const VERSION: u32 = 1;
    const SOURCE_HASH: u64 = fnv1a(include_str!("remap.rs"));
    const INPUTS: &'static [&'static str] = &["input"];

    fn eval(params: &Params, inputs: &[&Field], grid: Grid) -> Field {
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
}


