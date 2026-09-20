//! **网格域**的算子**实现**：立方球位移网格与等值面代理，编成一份 dylib。
//!
//! `mesh.proxy` 是**叶子**：它只认识体积的参数空间约定（`px_volume_schema`）与算法库
//! （`isosurface`）—— 不认识云、不认识驱动 ⇒ 换算法库不动图脚本。
//!
//! ⚠ 这一份是**实现**：声明在 `px_mesh_schema::ops` 里（图侧编译的是那一份）。
//!   每个算法文件顶上那一行 `px_body!` 就是"声明 ↔ 实现"的那根线。
//!
//! ⚠ 它**不是**任何人的 cargo 依赖：图程序按身份在运行期装载它。

pub mod cubesphere;
pub mod proxy;

// 这个库的身份：`…__source_hash`（进键的"实现是哪一份"）与 `…__contract_hash`
// （装载时与图程序对账"我们是不是同一份契约编出来的"）。
px_graph_schema::px_impl_lib!();
