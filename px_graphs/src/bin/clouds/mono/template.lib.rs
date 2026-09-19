// ⚠ 这一份是**生成的**（`px_graphs/src/bin/mono-gen.rs` 复制到
// `target/mono/<库名>/crate/`）。别手改这里 —— 编辑面是
// `src/bin/clouds/mono/fields.rs`（stage 1）与 `src/bin/clouds/mono.rs`（声明）。
//
// ⚠ 本文件被 `identity.rs` 与 `fields.rs` 一起 `include!`/`#[path]` 进来，
// 所以这里**不能有** `//!` 文档注释（外层文档只能出现在 crate 根的顶部），
// 也不能 `#![allow(...)]`（内层属性在 include! 展开后不是 crate 根的位置）。
//
// 为什么必须生成这么一层：`bake<F: FieldFn>` 的实例要与类型参数 `F` 在**同一个编译单元**
// 里生成，而图程序是 `bin` ⇒ 实例只可能落在**图侧自建的 dylib** 里。
// 这就是「生成 + 动态链接」那一级的全部理由。

// ⚠⚠ 为什么这里**还要手写** `PxOp` 与那三段 `extern "Rust"`（而算子库那边只要三行宏）：
//   因为 `render(&self, params, inputs, grid)` **拿不到那个场函数** ——
//   它是"图上现写的"（`ClosedForm { f: |cloud, dir| fields::closed_cover(...) }`），
//   既不是参数、也不是上游输入，而是**这一份实例自己的类型**。
//   要让宏收得下它，`PxOp` 的签名就得变成 `render<F: FieldFn>` 之类 ——
//   为一个特例把通用契约改成泛型的，不划算。**所以这一档是有意破例的，别去"统一"它。**

use px_cook::field_fn::{ClosedForm, CoverCloud};
use px_graph_schema::{
    Grid, OpCall, OpDescriptor, OpKind, OpTable, ParamsCanonical, PayloadBundle,
};
use px_volume_schema::{VolumeData, params};

/// stage 1 就在旁边（生成器把 `mono/fields.rs` 复制到同目录再引）。
#[path = "fields.rs"]
mod fields;

// 这一份实例的身份（`id` / `version` / `source_hash`）来自 `src/bin/clouds/mono.rs`，
// 生成器把它具体化进 `identity.rs`。
include!("identity.rs");

// ⚠ 导出符号名由**库名**派生：驱动按 `<库名>_op_table` 找（`px_graph_schema::op::table_symbol`）。
// 所以这里用 `#[export_name]` 把库名拼进去 —— 模板本身不带具体库名，生成器填 `@LIB@`。
#[unsafe(export_name = concat!("@LIB@", "_op_table"))]
pub extern "Rust" fn table() -> &'static OpTable {
    // ⚠ 描述符要运行期建（`interface()` 不是 const）。见 `px_cook::px_op_table!` 的同一段理由。
    static TABLE: std::sync::OnceLock<OpTable> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| OpTable {
        ops: Box::leak(
            vec![OpDescriptor {
                id: MONO_ID,
                interface: interface(),
                source_hash: SOURCE_HASH,
                // ⚠ 输入名不在这里了（描述符只剩"我是谁"）。这一份实例的输入形状
                //   由本文件的 `call` 自己解 —— 它知道"一个上游，覆盖度场"。
                kind: OpKind::Volume,
            }]
            .into_boxed_slice(),
        ),
        canonical_params: canonical as ParamsCanonical,
        call: call as OpCall,
    })
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
