//! **生成的单态化实例**：一个算子（`volume.cloud.coarse` 的闭式覆盖度那一档），
//! 编成 cdylib，被图程序**动态装载**。
//!
//! 为什么非要这个 crate 不可：`bake<F: FieldFn>` 的实例必须与类型参数 `F` 在**同一个
//! 编译单元**里生成。场函数是图侧的东西，而图程序是 `bin` ⇒ 实例只可能落在一个
//! **图侧自建的 dylib** 里。这就是「生成 + 动态链接」那一级的全部理由。
//!
//! ⚠ 本文件是**生成物**（`tools/mono-gen.ps1` 从 `tools/mono-template/` 复制到
//! `target/mono/<mono_key>/`）。编辑面是 `fields.rs`；这个文件是接线，别手改。

use px_cook::field_fn::{ClosedForm, CoverCloud};
use px_graph_schema::identity::fnv1a_sources;
use px_graph_schema::{
    Grid, OpCall, OpDescriptor, OpKind, OpTable, ParamsCanonical, PayloadBundle,
};
use px_volume_schema::{VolumeData, params};

#[path = "fields.rs"]
mod fields;

const VERSION: u32 = 1;

/// ⚠ **`lib.rs` 自己也在清单里**：接线改了（比如换了参数字段）也必须让身份变。
/// 而 `fields.rs` 在里面 ⇒ **改一行场函数就让 `SOURCE_HASH` 变 ⇒ 缓存键变 ⇒ 必然重算**，
/// 不必记得升 `VERSION`（§159.4 那条「源码变了而版本没升」的告警在这一支上不可能触发）。
const SOURCE_HASH: u64 = fnv1a_sources(&[
    include_str!("lib.rs"),
    include_str!("fields.rs"),
    include_str!("../../../../px_cook/src/field_fn.rs"),
    include_str!("../../../../px_volume_schema/src/volume.rs"),
    include_str!("../../../../px_volume_schema/src/params.rs"),
    include_str!("../../../../px_volume_schema/src/payload.rs"),
    include_str!("../../../../px_field_schema/src/field.rs"),
    include_str!("../../../../px_verify/src/cloud_field.rs"),
    include_str!("../../../../px_verify/src/noise.rs"),
    include_str!("../../../../px_verify/src/dual.rs"),
    include_str!("../../../../px_verify/src/proxy.rs"),
    include_str!("../../../../px_volume_op/src/lib.rs"),
]);

/// ⚠ **自己的 id，不复用 `params::CLOUD_COARSE`**。
///
/// 理由：`Context::find` 是「在已装载的库里取**第一个**命中的 op_id」（`driver.rs:107-124`），
/// 而装载顺序是路径字典序 ⇒ 同 id 时 mono 实例会**静默抢掉** `px_volume_op` 的那一个，
/// `--bin clouds` 的老路径就变质了。两档并存就必须两个身份。
const MONO_ID: &str = "volume.cloud.coarse.closed";

const DESCRIPTOR: OpDescriptor = OpDescriptor {
    id: MONO_ID,
    version: VERSION,
    source_hash: SOURCE_HASH,
    inputs: &["coverage"],
    kind: OpKind::Volume,
};

#[unsafe(no_mangle)]
pub extern "Rust" fn px_mono_clouds_op_table() -> &'static OpTable {
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
        other => Err(format!("px_mono_clouds 不认识算子 {other}")),
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
            let coverage = px_field_schema::payload::decode(inputs[0], grid.projection)?;
            let cloud = px_verify::proxy::from_volume(&params);
            let _ = (&cloud, &coverage);
            // ⚠ 覆盖度**不来自上游那张场**：这里拿到的 `coverage` 只用于满足接口，
            //    真正的覆盖度是图侧现算的闭式场（`fields::closed_cover`）。
            //    构造型闭包是零尺寸类型 ⇒ 实例里没有虚调用。
            let closed = ClosedForm {
                f: |cloud: &CoverCloud, direction: [f32; 3]| fields::closed_cover(cloud, direction),
            };
            let volume: VolumeData = px_volume_op::eval_closed(&params, &closed);
            let bundle: PayloadBundle = px_volume_schema::payload::encode(&volume);
            bundle.placeholder()
        }
        other => Err(format!("px_mono_clouds 不认识算子 {other}")),
    }
}
