//! 图与算子之间的**协议数据**：图规格、清单条目。

use serde::{Deserialize, Serialize};

/// 一张图：**就一个名字**。
///
/// ⚠ **没有尺寸、没有投影**：它们从前住在这里，作为"图交给算子的那张画布"；现在
///   它们是**参数**（`field.*` 那几个算子的 `Shape`）—— 于是"这个节点产出多大"这句话
///   只由**参数**回答，不再有一个藏在驱动里的第二份真相。
///   渲染那一侧的帧尺寸本来就来自请求（`px_render::serve`），不从这里取。
///
/// ⚠ **也没有相机表**：相机是**场景脚本**里的普通数据（`px-scene` 的 recipe 里那张表），
///   不属于图、不属于产物、不进缓存键 —— 它管的是「怎么看」。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSpec {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub node: String,
    pub op: String,
    /// 接口哈希的低 32 位（只用于显示 / 对账）。
    pub op_version: u32,
    pub key: String,
    pub hit: bool,
    pub millis: u64,
    pub bytes: u64,
    /// **人读的一行**：这份产物长什么样。
    ///
    /// ⚠ 从前这里是 `min` / `max` / `mean` 三格，被四个消费者塞了**四种含义**
    /// （shader 的 `mean` 是文本长度、scene 的 `mean` 是物体数、generated 的 `mean` 是载荷字节）。
    /// 那不是读数，是字段复用 —— 读清单的人得先记住"这一格这次是什么意思"。
    /// 一个字符串把这件事说清楚，而**说这句话的人是域自己**（类型化的值在它手里）。
    pub detail: String,
}
