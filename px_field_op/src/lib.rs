//! **场域**的算子：七个场算子 + 一个 dylib 入口。
//!
//! ⚠ 它不认识驱动、不认识 CAS、不认识 `art/`：进来的是**序列化的载荷**，
//! 出去的是同一个形状。这就是「算子之间只通过 schema 说话」。

use px_field_schema::field::Field;
use px_field_schema::params;
use px_field_schema::payload as field_payload;
use px_graph_schema::{OpCall, OpDescriptor, OpTable, ParamsCanonical, PayloadBundle};

pub mod noise;
pub mod ops;
pub mod typed;

use px_graph_schema::Grid;

static OPS: &[OpDescriptor] = &[
    ops::constant::DESCRIPTOR,
    ops::fbm::DESCRIPTOR,
    ops::gradient::DESCRIPTOR,
    ops::mix::DESCRIPTOR,
    ops::remap::DESCRIPTOR,
    ops::ridged::DESCRIPTOR,
    ops::warp::DESCRIPTOR,
];

#[unsafe(no_mangle)]
pub extern "Rust" fn px_field_op_table() -> &'static OpTable {
    static TABLE: OpTable = OpTable {
        ops: OPS,
        canonical_params: params::canonical as ParamsCanonical,
        call: call as OpCall,
    };
    &TABLE
}

fn params_of<P: serde::de::DeserializeOwned>(text: &str) -> Result<P, String> {
    serde_json::from_str(text).map_err(|err| format!("参数 JSON 解不开：{err}"))
}

extern "Rust" fn call(
    op_id: &str,
    params_json: &str,
    grid: Grid,
    inputs: &[&[u8]],
) -> Result<Vec<u8>, String> {
    let fields: Vec<Field> = inputs
        .iter()
        .map(|bytes| field_payload::decode(bytes, grid.projection))
        .collect::<Result<Vec<_>, String>>()?;
    let borrowed: Vec<&Field> = fields.iter().collect();

    let field = match op_id {
        params::CONSTANT => ops::constant::eval(&params_of(params_json)?, &borrowed, grid),
        params::FBM => ops::fbm::eval(&params_of(params_json)?, &borrowed, grid),
        params::GRADIENT => ops::gradient::eval(&params_of(params_json)?, &borrowed, grid),
        params::MIX => ops::mix::eval(&params_of(params_json)?, &borrowed, grid),
        params::REMAP => ops::remap::eval(&params_of(params_json)?, &borrowed, grid),
        params::RIDGED => ops::ridged::eval(&params_of(params_json)?, &borrowed, grid),
        params::WARP => ops::warp::eval(&params_of(params_json)?, &borrowed, grid),
        other => return Err(format!("px_field_op 不认识算子 {other}")),
    };
    let bundle: PayloadBundle = field_payload::encode(&field);
    bundle.placeholder()
}
