//! pcg 的图程序（`src/bin/*.rs`）与它们共用的东西。
//!
//! ⚠ **场景语义与帧图编译器不在这里了**：它们搬进了 `px-scene`（§px-scene 那一轮）。
//! 依赖方向是 **`px_graphs` → `px_scene`** —— 图程序像用寻常引擎那样调用那一层。
//!
//! 留在本 crate 的是**算子与图本身**：
//! · [`cloud_proxy`]：体积代理那一条（`VolumeOp` 实现，`--bin clouds` 用）；
//! · `art/<图名>/<节点>.toml` 那些参数所对应的图程序。
pub mod cloud_proxy;

