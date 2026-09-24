//! 缓存键：**键 = 内容**（§17.1）。全部是纯函数，不含路径 / 时间 / pid / 主机名。

use serde::Serialize;
use serde_json::Value;

pub type Key = [u8; 32];

/// 参数规范化：serde → 键序固定的 JSON。改注释、调格式**不会**让缓存失效（§17.2）。
pub fn canonical_params<T: Serialize>(params: &T) -> String {
    let value = serde_json::to_value(params).expect("参数无法序列化");
    serde_json::to_string(&sorted(value)).expect("参数无法规范化")
}

fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, sorted(v))).collect()),
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// **一个算子的身份**：它是谁、接口什么形状、是哪一份源码。
#[derive(Debug, Clone, Copy)]
pub struct OpId<'a> {
    /// 人读的那一半（`field.fbm`）；它也进键。
    pub id: &'a str,
    /// **接口形状哈希** —— 从 `Params` / `Inputs` / `Payload` 三个类型名推（`px_cook`）。
    pub interface: u64,
    /// **实现**的源码指纹。⚠ 它随实现过来（实现库的运行期身份），不是这张图的属性。
    pub source_hash: &'a str,
}

/// **一个节点的键 = 产出这个节点的那些东西**，也只有那些：
///
/// ```text
/// op_id ‖ 接口形状哈希 ‖ 实现的源码指纹 ‖ 规范参数 ‖ 上游的键
/// ```
///
/// ⚠ 这里**没有**两样从前有、现在删掉的东西，理由同一个 —— 它们不是"这个节点是什么"：
///
/// * **`graph_version`**：改图脚本里别处一行代码，不该让这个节点的产物作废。
///   它是**图的属性**，进键等于把"节点身份"与"这张图今天长什么样"绑在一起。
/// * **画布**（`GraphSpec.width/height/projection` 那一套）：它跟"这个节点算什么"无关，
///   而且**已经不存在了** —— 尺寸与投影是**参数**（`field.*` 那几个算子的 `Shape`），
///   于是"改尺寸要不要重算"由**参数表**回答，不用谁来手写一条声明：
///   参数里有尺寸的算子 ⇒ 改尺寸就换键；没有的（体积/网格/贴图/NURBS）⇒ 不换。
///
/// ⇒ 同样的算子、同样的参数、同样的上游 ⇒ **同一个产物**，不管图脚本长什么样。
///
/// ⚠ `b"px_cook/v1"` 是键的**域分隔符**，不是一个可以顺手改的字符串：改它 = 全仓换键。
pub fn node_key(op: &OpId<'_>, params_json: &str, inputs: impl FnOnce(&mut blake3::Hasher)) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_cook/v1");
    hasher.update(op.id.as_bytes());
    hasher.update(&op.interface.to_le_bytes());
    hasher.update(op.source_hash.as_bytes());
    hasher.update(params_json.as_bytes());
    inputs(&mut hasher);
    *hasher.finalize().as_bytes()
}

/// 载荷内容的 FNV-1a —— 实现在 `px_protocol::payload`（载荷类型住那边）。
pub use px_protocol::payload::payload_fingerprint;

pub fn hex(key: &Key) -> String {
    let mut out = String::with_capacity(64);
    for byte in key {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn hex_short(key: &Key) -> String {
    hex(key)[..12].to_string()
}
