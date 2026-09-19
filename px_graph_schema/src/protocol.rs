//! 图与算子之间的**协议数据**：画布、图规格、清单条目。

use serde::{Deserialize, Serialize};

use px_protocol::art::{Camera, Domain};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSpec {
    pub name: String,
    pub version: u32,
    pub source_hash: u64,
    pub width: u32,
    pub height: u32,
    pub projection: Domain,
    /// 评审相机表：写进每一个产物（局部方向 + 距离，见 `px_protocol::art::Camera`）。
    /// 它属于「怎么看」不属于「是什么」，但住在产物里 ⇒ 必须进缓存键。
    pub cameras: Vec<Camera>,
}

/// 交给算子的画布：`width × height` + 投影。场的 `filled` / `direction` 那些helper
/// 住在 `px_field_schema`（域自己的事），这里只留数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub width: u32,
    pub height: u32,
    pub projection: Domain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub op_id: String,
    /// 接口哈希的低 32 位（只用于显示/对账；真正进键的是 64 位接口哈希的文本）。
    pub op_version: u32,
    pub graph_version: u32,
    /// 算子库的**源码指纹**（blake3 十六进制，`build.rs` 算的）。
    pub source_hash: String,
    pub graph_source_hash: u64,
    pub node: String,
    pub millis: u64,
    pub bytes: u64,
    /// 这个节点是哪个 dll 算出来的（dll 文件字节的 blake3 前 8 字节）。
    ///
    /// ⚠ **它不进键**（用户裁决）：dll 是一个字节序列，整包进键就是 §19 当年否掉的
    /// 「一动全废」（改任一算子 ⇒ 所有图的全部节点换键）。它进这里干两件事：
    /// 「这个键是哪个二进制算的」可查，以及换 dll 而版本没升时能当场喊（§19.1 同一套）。
    #[serde(default)]
    pub dll: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub node: String,
    pub op: String,
    pub op_version: u32,
    pub key: String,
    pub hit: bool,
    pub millis: u64,
    pub bytes: u64,
    pub min: f32,
    pub max: f32,
    pub mean: f32,
}
