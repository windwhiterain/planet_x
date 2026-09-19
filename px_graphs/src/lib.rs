//! pcg 的图程序（`src/bin/*.rs`）与它们共用的**判据仪器**。
//!
//! ⚠ **算子不在这里**：场算子住 `px_field_op`、体积算子住 `px_volume_op`、
//! 网格算子住 `px_mesh_op`（都是 dylib，运行时按描述符表装载）。
//! 图脚本只依赖**数据**（`px_*_schema`）与**驱动**（`px_graph`）：
//!
//! ```text
//! begin(GraphSpec { … })
//! node("field.fbm", "continents", &[])      ← 按算子 id 取，不按类型取
//! finish()
//! ```
//!
//! ⚠ **场景语义与帧图编译器也不在这里**：它们在 `px-scene`（§px-scene 那一轮）。
//! 依赖方向是 **`px_graphs` → `px_graph` / `px-scene`** —— 图程序像用寻常引擎那样调用那些层。
//!
//! 留在本 crate 的是**图与判据**：
//! · [`cloud_proxy`]：体积代理那一条的**仪器**（射线求交、梯度上界），`--bin clouds` 用；
//! · `art/<图名>/<节点>.toml` 那些参数所对应的图程序。
pub mod cloud_proxy;
