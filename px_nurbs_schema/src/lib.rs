//! See docs/nurbs.md

pub const SOURCE_HASH: &str = env!("PX_SOURCE_HASH");

pub mod curve;
pub mod knot;
pub mod mesh;
pub mod ops;
pub mod params;
pub mod payload;
pub mod point;
pub mod surface;

pub use curve::{Curve, Sample};
pub use point::PointData;
pub use surface::{Along, Patch, Surface};

pub use px_protocol::art::{MeshData, PolylineData};
