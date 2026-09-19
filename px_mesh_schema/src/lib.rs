//! **网格域**的数据那一半：网格载荷，以及两个网格算子的参数。

pub mod params;
pub mod payload;

pub use params::{CUBESPHERE, PROXY};
pub use px_protocol::art::MeshData;
