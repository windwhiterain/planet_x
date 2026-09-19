//! 缓存键：**键 = 内容**（§17.1）。全部是纯函数，不含路径 / 时间 / pid / 主机名。

use serde::Serialize;
use serde_json::Value;

use px_protocol::art::{Camera, Domain};
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

pub fn node_key(
    op_id: &str,
    op_interface: &str,
    graph_version: u32,
    canvas: (u32, u32),
    projection: Domain,
    params_json: &str,
    input_keys: &[Key],
) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_pcg/v1");
    hasher.update(op_id.as_bytes());
    hasher.update(op_interface.as_bytes());
    hasher.update(&graph_version.to_le_bytes());
    hasher.update(&canvas.0.to_le_bytes());
    hasher.update(&canvas.1.to_le_bytes());
    hasher.update(projection.name().as_bytes());
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
