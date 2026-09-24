//! **体积域**的数据那一半：立方球参数空间的约定、体积网格、烘培参数与**声明**
//! （`ops.rs`：`px_op!` 那几行）。
//!
//! 算法不在这里（`px_volume_op`，dylib）、驱动不在这里（`px_graph`）——这一层回答
//! 「体积是什么、参数长什么样、算子怎么接」。

/// **这个 crate 编译进去的全部源码**的指纹（`build.rs` 给的十六进制）。
/// 声明里那句 `const DECL_HASH: &'static str = env!("PX_SOURCE_HASH")` 用的就是它 ——
/// 泛型实例的键必须覆盖"声明所在的这一份源码"（见 `docs/system/generic-instances.md` §177）。
pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

pub mod ops;
pub mod params;
pub mod volume;

pub use params::{FieldKind, Params};
pub use px_protocol::art::{TextureData, TextureFormat, VolumeData};
pub use volume::{PATCHES, VolumeGrid, VolumeSampler, direction_of, point_of};
