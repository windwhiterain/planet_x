//! See docs/mesh.md

pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

pub mod ops;
pub mod params;

pub use px_protocol::art::MeshData;
