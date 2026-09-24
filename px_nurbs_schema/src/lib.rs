//! **NURBS 域**的数据那一半：曲线 / 曲面 / 求值读数，以及八个 NURBS 算子的参数与
//! **声明**（`ops.rs`：`px_op!` 那几行）。
//!
//! 算法不在这里（`px_nurbs_op`，dylib）、驱动不在这里（`px_graph`）——这一层回答
//! 「NURBS 是什么、参数长什么样、算子怎么接」。
//!
//! ⚠ 全库按 **f64** 算：`f32` 连单位圆都表示不准（控制点里那个 `√2/2` 一舍入，
//!   圆就不再是圆），而"精确圆"正是 NURBS 存在的理由。要进网格时才降到 `f32`
//!   （细分那一步）。

/// **这个 crate 编译进去的全部源码**的指纹（`build.rs` 给的十六进制）。
/// 声明里那句 `const DECL_HASH: &'static str = env!("PX_SOURCE_HASH")` 用的就是它 ——
/// 泛型实例的键必须覆盖"声明所在的这一份源码"（见 `.agents/notes/art/19-generic-inst.md` §177）。
pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

pub mod curve;
pub mod knot;
pub mod ops;
pub mod params;
pub mod payload;
pub mod point;
pub mod surface;

pub use curve::{Curve, Sample};
pub use point::PointData;
pub use surface::{Along, Patch, Surface};
