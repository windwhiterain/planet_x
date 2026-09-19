//! **场域**的数据那一半：`Field` 与它的投影、`Scalar` 数值抽象、七个场算子的参数，
//! 以及场载荷的序列化。
//!
//! 算法不在这里（`px_field_op`），驱动不在这里（`px_graph`）——这一层只回答
//! 「场是什么、参数长什么样」。

pub mod field;
pub mod noise;
pub mod params;
pub mod payload;

pub use field::{Field, GridField, Projection, ProjectionKind, Stats, cube_map_extent};
pub use noise::{FbmSettings, Scalar};
