//! GPU 那两个算子的**实现**：一个算子一个文件。
//!
//! ⚠ **一个 `px_body!` 一个文件**：那个宏导出的是 `__px_body`（Rust 标识符），
//!   同一个模块里两次就重名。
//!
//! ⚠ 这一层薄到几乎没有：求值在 WGSL（`../surface.wgsl` / `../curve.wgsl`）、
//!   编排在 `crate`（细分到哪一级、参数怎么分、装配）—— 这里只是把声明接到编排上。

pub mod curve_tessellate_gpu;
pub mod surface_tessellate_gpu;
