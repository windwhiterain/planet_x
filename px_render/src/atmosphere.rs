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
    pub camera_x: f32,
    pub camera_y: f32,
    pub camera_z: f32,
    pub reserved: f32,
}

impl AtmosphereParams {
    pub fn new(inner: f32, outer: f32, density: f32, softness: f32) -> Self {
        Self {
            inner,
            outer,
            density,
            softness,
            camera_x: 0.0,
            camera_y: 0.0,
            camera_z: 0.0,
            reserved: 0.0,
        }
    }
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

#[derive(Component)]
pub struct RequestCamera;

#[derive(Resource)]
pub struct ShaderLibrary(pub Handle<Shader>);

fn load_library(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(ShaderLibrary(assets.load("shaders/common.wgsl")));
}

fn sync_cameras(
    cameras: Query<
        (
            &GlobalTransform,
            &Projection,
            Option<&crate::OrbitCamera>,
        ),
        With<Camera3d>,
    >,
    canvases: Query<&crate::Canvas>,
    mut materials: ResMut<Assets<AtmosphereMaterial>>,
) {
    let mut chosen = None;
    for (transform, projection, orbit) in cameras.iter() {
        if orbit.is_some() {
            chosen = Some((transform, projection));
        }
    }
    let Some((transform, projection)) = chosen else {
        return;
    };
    let size = canvases
        .iter()
        .next()
        .map(|canvas| Vec2::new(canvas.size.0 as f32, canvas.size.1 as f32))
        .unwrap_or(Vec2::new(960.0, 640.0));
    let position = transform.translation();

    for (_, material) in materials.iter_mut() {
        material.params.camera_x = position.x;
        material.params.camera_y = position.y;
        material.params.camera_z = position.z;

    }
}

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<AtmosphereMaterial>::default())
            .add_systems(Startup, load_library)
            .add_systems(Update, sync_cameras);
    }
}



