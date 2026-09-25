use serde::Serialize;
use serde_json::Value;

pub type Key = [u8; 32];

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

#[derive(Debug, Clone, Copy)]
pub struct OpId<'a> {
    pub id: &'a str,
    pub interface: u64,
    pub source_hash: &'a str,
}

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
