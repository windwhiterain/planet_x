//! **体积域**的数据那一半：立方球参数空间的约定、体积网格，以及烘培参数。

pub mod params;
pub mod payload;
pub mod volume;

pub use params::{FieldKind, Params};
pub use volume::{PATCHES, VolumeGrid, VolumeSampler, direction_of, point_of};
pub use px_protocol::art::VolumeData;
