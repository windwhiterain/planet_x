use bevy::material::AlphaMode;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct AtmosphereParams {
    pub inner: f32,
    pub outer: f32,
    pub density: f32,
    pub softness: f32,
    pub camera: Vec3,
    pub padding: f32,
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

#[derive(Resource)]
pub struct ShaderLibrary(pub Handle<Shader>);

fn sync_camera(
    camera: Query<&GlobalTransform, With<crate::OrbitCamera>>,
    mut materials: ResMut<Assets<AtmosphereMaterial>>,
) {
    let Ok(transform) = camera.single() else {
        return;
    };
    let position = transform.translation();
    for (_, material) in materials.iter_mut() {
        material.params.camera = position;
    }
}

fn load_library(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(ShaderLibrary(
        assets.load("shaders/common.wgsl"),
    ));
}

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<AtmosphereMaterial>::default())
            .add_systems(Startup, load_library)
            .add_systems(Update, sync_camera);
    }
}





