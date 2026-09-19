//! pcg 的图程序（`src/bin/*.rs`）与它们共用的东西。
//!
//! ⚠ **场景语义与帧图编译器不在这里了**：它们搬进了 `px-scene`（§px-scene 那一轮）。
//! 依赖方向是 **`px_graphs` → `px_scene`** —— 图程序像用寻常引擎那样调用那一层。
//!
//! 留在本 crate 的是**算子与图本身**：
//! · [`cloud_proxy`]：体积代理那一条（`VolumeOp` 实现，`--bin clouds` 用）；
//! · `art/<图名>/<节点>.toml` 那些参数所对应的图程序。
pub mod cloud_proxy;

/// ⚠ 兼容别名：`px_graphs::params` 原来的两个函数搬进了 `px_scene::contract`。
///
/// 留着它是因为**两条烘图路**（`--bin scene` 与 `--bin passes`）与它们的历史注释都按
/// 这个名字说话；真正的实现只有一份（`px_scene::contract`），这里只是转出去 ——
/// 抄第二份就是第二个会漂开的真相。
pub use px_scene::contract as params;
