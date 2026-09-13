use serde::{Deserialize, Serialize};

use crate::field::Field;
use crate::noise::fnv1a;
use crate::FieldOp;

pub struct Constant;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    pub value: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self { value: 0.5 }
    }
}

impl FieldOp for Constant {
    type Params = Params;
    const ID: &'static str = "field.constant";
    const VERSION: u32 = 1;
    const SOURCE_HASH: u64 = fnv1a(include_str!("constant.rs"));
    const INPUTS: &'static [&'static str] = &[];

    fn eval(params: &Params, _inputs: &[&Field], size: (u32, u32)) -> Field {
        Field::filled(size.0, size.1, params.value)
    }
}
