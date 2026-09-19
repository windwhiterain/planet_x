//! **网格域**的算子库：立方球位移网格与等值面代理 + dylib 入口。
//!
//! `mesh.proxy` 是**叶子**：它只认识体积的参数空间约定（`px_volume_schema`）与算法库
//! （`isosurface`）—— 不认识云、不认识驱动 ⇒ 换算法库不动图脚本。
//!
//! ⚠ 底下三行宏就是 dylib 那一侧的全部管道，见 `px_field_op/src/lib.rs` 的同一段注释。

pub mod typed;

pub mod cubesphere;
pub mod proxy;

