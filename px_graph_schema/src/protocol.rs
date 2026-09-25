use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSpec {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub node: String,
    pub op: String,
    pub op_version: u64,
    pub key: String,
    pub hit: bool,
    pub millis: u64,
    pub bytes: u64,
    pub detail: String,
}
