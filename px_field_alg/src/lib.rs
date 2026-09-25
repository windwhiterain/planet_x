//! See docs/field.md

#[allow(dead_code)]
pub mod field_fn;
pub mod noise;
pub mod remap;

pub use field_fn::{FieldFn, Sampled, Upstream};
pub use remap::{Cell, Scale, identity, map_grid, remap_with};
