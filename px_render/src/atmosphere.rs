use bevy::material::AlphaMode;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct AtmosphereParams {
    pub power: f32,
    pub intensity: f32,
    pub padding: Vec2,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct AtmosphereMaterial {
    #[uniform(0)]
    pub params: AtmosphereParams,
    #[uniform(1)]
    pub tint: LinearRgba,
}

impl Material for AtmosphereMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/atmosphere.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }
}

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<AtmosphereMaterial>::default());
    }
}

