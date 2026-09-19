//! 图与算子之间的**协议数据**：画布、图规格、清单条目。

use serde::{Deserialize, Serialize};

use px_protocol::art::{Camera, Domain};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSpec {
    pub name: String,
    /// **图的身份**，不是键的一部分。
    ///
    /// ⚠ 它曾经进节点键 —— 那是错的：改图脚本里别处一行代码，不该让某个节点的产物作废。
    ///   它现在的用途是"这张图的这一版"这个说法本身（清单里记着它）。
    pub version: u32,
    /// 图脚本源码的哈希。用途同上：图的身份，**不进键**。
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
