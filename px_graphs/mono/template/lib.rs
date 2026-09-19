// ⚠ 这一份是**生成物**的接线（`px_graphs/src/bin/mono-gen.rs` 复制到 `target/mono/crate/`）。
// 别手改这里 —— 编辑面是 `px_graphs/mono/fields.rs`（stage 1）与
// `px_graphs/src/mono.rs`（身份清单）。
//
// ⚠ 本文件被 `identity.rs` 与 `fields.rs` 一起 `include!`/`#[path]` 进来，
// 所以这里**不能有** `//!` 文档注释（外层文档只能出现在 crate 根的顶部）。
//
// 为什么必须生成这么一层：`bake<F: FieldFn>` 的实例要与类型参数 `F` 在**同一个编译单元**
// 里生成，而图程序是 `bin` ⇒ 实例只可能落在**图侧自建的 dylib** 里。
// 这就是「生成 + 动态链接」那一级的全部理由。

use px_cook::field_fn::{ClosedForm, CoverCloud};
use px_graph_schema::{
    Grid, OpCall, OpDescriptor, OpKind, OpTable, ParamsCanonical, PayloadBundle,
};
use px_volume_schema::{VolumeData, params};

#[path = "fields.rs"]
mod fields;

// 这一份实例的身份（`id` / `version` / `source_hash`）由 `px_graphs::mono` 给 ——
// 生成器把它具体化进 `identity.rs`。
include!("identity.rs");

#[unsafe(no_mangle)]
pub extern "Rust" fn px_mono_op_table() -> &'static OpTable {
    static TABLE: OpTable = OpTable {
        ops: &[DESCRIPTOR],
        canonical_params: canonical as ParamsCanonical,
        call: call as OpCall,
    };
    &TABLE
}

extern "Rust" fn canonical(op_id: &str, toml_text: Option<&str>) -> Result<String, String> {
    match op_id {
        MONO_ID => {
            let params = params::parse(toml_text)?;
            Ok(px_graph_schema::canonical_params(&params))
        }
        other => Err(format!("{MONO_ID} 之外的算子不认识：{other}")),
    }
}

extern "Rust" fn call(
    op_id: &str,
    params_json: &str,
    grid: Grid,
    inputs: &[&[u8]],
) -> Result<Vec<u8>, String> {
    match op_id {
        MONO_ID => {
            let params: params::Params = serde_json::from_str(params_json)
                .map_err(|err| format!("参数 JSON 解不开：{err}"))?;
            // 描述符声明了 `inputs = ["coverage"]`（老路径的接口形状），这一格必须给；
            // 但**闭式那一档不看它** —— 覆盖度是现算的。解一下只为走同一条边界。
            let _coverage = px_field_schema::payload::decode(inputs[0], grid.projection)?;
            // 构造型闭包是零尺寸类型 ⇒ 单态化出来的实例里没有虚调用。
            let closed = ClosedForm {
                f: |cloud: &CoverCloud, direction: [f32; 3]| fields::closed_cover(cloud, direction),
            };
            let volume: VolumeData = px_volume_op::eval_closed(&params, &closed);
            let bundle: PayloadBundle = px_volume_schema::payload::encode(&volume);
            bundle.placeholder()
        }
        other => Err(format!("{MONO_ID} 之外的算子不认识：{other}")),
    }
}
