use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::noise::fnv1a;
use crate::FieldOp;

pub struct Mix;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub bias: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self { bias: 0.0 }
    }
}

impl FieldOp for Mix {
    type Params = Params;
    const ID: &'static str = "field.mix";
    const VERSION: u32 = 1;
    const SOURCE_HASH: u64 = fnv1a(include_str!("mix.rs"));
    const INPUTS: &'static [&'static str] = &["a", "b", "mask"];

    fn eval(params: &Params, inputs: &[&Field], size: (u32, u32)) -> Field {
        let (a, b, mask) = (inputs[0], inputs[1], inputs[2]);
        let mut field = Field::filled(size.0, size.1, 0.0);
        for y in 0..size.1 {
            for x in 0..size.0 {
                let weight = (mask.at(x, y) + params.bias).clamp(0.0, 1.0);
                let value = a.at(x, y) * (1.0 - weight) + b.at(x, y) * weight;
                field.set(x, y, value);
            }
        }
        field
    }
}
