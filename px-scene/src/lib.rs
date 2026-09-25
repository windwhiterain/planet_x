//! See docs/art.md

pub mod baked;
pub mod builder;
pub mod cameras;
pub mod contract;
pub mod frame;
pub mod math;
pub mod members;
pub mod recipe;
pub mod stage;
pub mod vocab;
pub mod vshadow;

pub use baked::Baked;
pub use builder::{MaterialBuilder, SceneBuilder};
pub use contract::{merge_named, schema_of, shader_parts_of};
pub use frame::{DEFAULT_FRAME, FrameFile, Sources};
pub use recipe::{SceneFile, compile, load, material_params, recipe_path};
pub use stage::{
    Atmosphere, CheckStages, Clouds, Content, Given, Opaque, PointShadow, Prepass, Registration,
    Ring, Sky, Skybox, Stage, StageParams, Surface, Transparent,
};
pub use vocab::{
    CLOUD_BASE, CLOUD_SHADOW_GAIN, CLOUD_SHADOW_HEIGHT, CLOUD_TOP, SKYBOX_BRIGHTNESS,
    SUN_RANGE_FACTOR, SYSTEM_TILT,
};
