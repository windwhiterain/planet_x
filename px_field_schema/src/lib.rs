//! **场域**的数据那一半：`Field` 与它的投影、`Scalar` 数值抽象、七个场算子的参数与**声明**，
//! 以及场载荷的序列化。
//!
//! 算法不在这里（`px_field_op`，dylib）、驱动不在这里（`px_graph`）——这一层回答
//! 「场是什么、参数长什么样、算子怎么接」。

/// **这个 crate 编译进去的全部源码**的指纹（`build.rs` 给的十六进制）。
/// 声明里那句 `const DECL_HASH: &'static str = env!("PX_SOURCE_HASH")` 用的就是它 ——
/// 泛型实例的键必须覆盖"声明所在的这一份源码"（见 `.agents/notes/art/19-generic-inst.md` §177）。
pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

pub mod field;
pub mod noise;
pub mod ops;
pub mod parallel;
pub mod params;
pub mod payload;
pub mod volume;

pub use field::{Field, GridField, Projection, ProjectionKind, Stats, cube_map_extent};
pub use noise::{FbmSettings, Scalar};
pub use volume::VolumeShape;
