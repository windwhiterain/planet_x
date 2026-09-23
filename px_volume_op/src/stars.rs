//! `sky.stars`：**世界坐标里的一批点光源 → 一个 R3 星场**（`StarField` 载荷）。
//!
//! ⚠ 这一档**不是场算子**（尽管它的前身 `field.stars` 是）：输出是 `StarField`
//!   （星表 + 稀疏均匀三维格），不是一张 `width × height` 的画布。见
//!   `px_volume_alg::stars` 的文件头 —— 那里写着"为什么不能再画进一张立方图"。
//!
//! ⚠⚠ 它现在吃**一个输入**：`cloud.density`（体积图里的那份密度）。
//!   用户 2026-09-25 要"星星和星云在大尺度上分布近似"，而**只对低频噪声**
//!   量出来相关只有 +0.022 —— 气的分布是整条链（阈值/mix）定的 ⇒ 必须吃真密度场。
//!   ⚠ 这一档因此**只在体积图里**解析（天空图那份多余的节点删掉了：天空节点直接
//!   拿体积图的星场句柄，与 `volume` 完全同一条路）。
//!   ⚠ 画布（`grid`）仍然**不参与**（`RESOLUTION_IS_CANVAS = false`）：星的位置由参数
//!   与**上游密度**决定 —— 密度进键，画布不进。
//!   ⚠ 没有密度输入时（判据/探针的老用法）退化成从前那一档（`gas_biased` 那一栏空转）。

use px_volume_schema::ops::Stars;

px_graph_schema::px_body! {
    Stars,
    |p, i, _g| px_volume_alg::bake_stars(p, Some(i.volume.value()))?
}
