//! 缓存键：**键 = 内容**（§17.1）。全部是纯函数，不含路径 / 时间 / pid / 主机名。

use serde::Serialize;
use serde_json::Value;

use px_protocol::art::Camera;
use px_protocol::wire::Blob;

use crate::identity::fnv1a;

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

/// **一个节点的键 = 产出这个节点的那些东西。**
///
/// ```text
/// op_id + 接口哈希 + 规范参数 + 上游的键
/// ```
///
/// ⚠ 这里**没有**两样从前有、现在删掉的东西，理由同一个 —— 它们不是"这个节点是什么"：
///
/// * **`graph_version`**：改图脚本里别处一行代码，不该让这个节点的产物作废。
///   它是**图的属性**，进键等于把"节点身份"与"这张图今天长什么样"绑在一起。
/// * **画布尺寸**：它对**场**是真的（场的分辨率就是画布），但对体积/网格无关
///   —— 所以它由**域自己**声明要不要（`Payload::RESOLUTION_IS_CANVAS`），
///   不在这里一刀切。见 `px_cook::cook`。
///
/// ⇒ 同样的算子、同样的参数、同样的上游（+ 该域的画布）⇒ **同一个产物**，
///   不管图脚本长什么样。
///
/// ⚠ 投影不在这里：它只影响**编码/解码的字节布局**，那种差异该由 `Payload` 的
///   `decode`/`encode` 承担，而不是靠往节点键里掺一个"当时用什么投影编的"。
pub fn node_key(
    op_id: &str,
    op_interface: &str,
    params_json: &str,
    input_keys: &[Key],
) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_pcg/v2");
    hasher.update(op_id.as_bytes());
    hasher.update(op_interface.as_bytes());
    hasher.update(params_json.as_bytes());
    for key in input_keys {
        hasher.update(key);
    }
    *hasher.finalize().as_bytes()
}

/// 相机表住在产物里 ⇒ 它变了产物内容就变了 ⇒ 必须进键。
/// 否则 CAS 会出现「同一个键、不同内容」（§17.1 那条「键 = 内容」）。
///
/// ⚠ 体积那一档**不进**相机：相机是「怎么看」，而体积没人看（渲染器只读 mesh）。
pub fn key_with_cameras(key: Key, cameras: &[Camera]) -> Key {
    if cameras.is_empty() {
        return key;
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(&key);
    for camera in cameras {
        for value in camera.direction {
            hasher.update(&value.to_le_bytes());
        }
        hasher.update(&camera.distance.to_le_bytes());
        hasher.update(camera.tag.as_bytes());
    }
    *hasher.finalize().as_bytes()
}

/// 载荷内容的 FNV-1a。节点名也混进去，免得两个载荷相同的节点指纹撞上。
pub fn payload_fingerprint(id: &str, blobs: &[Blob]) -> u64 {
    let mut hash = fnv1a(id);
    for blob in blobs {
        for byte in &blob.bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(crate::identity::FNV_PRIME);
        }
    }
    hash
}

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
