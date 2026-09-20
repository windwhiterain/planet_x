//! pcg 的图程序（`src/bin/*.rs`）与它们共用的**判据仪器**。
//!
//! ⚠ **算子不在本 crate，也不在本 crate 的依赖里**：声明住在各域 schema 的 `ops.rs`，
//! 实现在 `px_field_op` / `px_volume_op` / `px_mesh_op`（dylib，运行时按**身份**装载）。
//! 图脚本只依赖**契约**（`px_graph_schema`）、**门面**（`px_cook`）与**数据**（`px_*_schema`）：
//!
//! ```text
//! let graph = begin(GraphSpec { … });                        ← 一棵图的句柄
//! let continents = cook::<field::Fbm>(&graph, "continents", ())?;   ← 类型在编译期，实现不在
//! graph.finish();
//! ```
//!
//! ⚠ **场景语义与帧图编译器也不在这里**：它们在 `px-scene`（§px-scene 那一轮）。
//! 依赖方向是 **`px_graphs` → `px_graph` / `px-scene`** —— 图程序像用寻常引擎那样调用那些层。
//!
//! 留在本 crate 的是**图与判据**：
//! · [`cloud_proxy`]：体积代理那一条的**仪器**（射线求交、梯度上界），`--bin clouds` 用；
//! · `art/<图名>/<节点>.toml` 那些参数所对应的图程序。
//!
//! ⚠ **图侧也可以现写算子**（`px_graph_schema::px_local_op!`）：那种算子的身份就是
//! **本 crate** 这一份源码（[`SOURCE_HASH`]），实现在图脚本里、泛型参数在图侧实例化。
//! 真样本在 `tests/local_op.rs`。
pub mod cloud_proxy;
/// **泛型实例**的声明（`px_inst`）与 **stage 1 的 build graph**（`insts::build`）——
/// bin `px`（`list` / `build` / `run`）与 `tests/inst_gate.rs` 都看它。
pub mod insts;
// ⚠ 按文本找 `px_inst` 调用的那个扫描器住在 **`px_cook::inst_scan`**（生成与执行都在那一层，
//   `20-build-graph.md` §187 B2 后半）：要用它就走那条全路径，**这里不留 re-export**
//   —— 搬了家就搬干净，留一行旧路径就是下一个过期物。

/// **本图程序**这一份源码的指纹（`build.rs` 算的十六进制）。
///
/// 它是「图侧现写的算子」的身份 —— 与实现库那份身份（运行期从 dylib 里读）是**两回事**：
/// 前者跟着图脚本走，后者跟着 `px_*_op` 走。
pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");
