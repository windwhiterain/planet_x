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
    pub screen_x: f32,
    pub screen_y: f32,
    pub padding_a: f32,
    pub padding_b: f32,
    pub padding_c: f32,
    pub forward: Vec4,
    pub right: Vec4,
    pub up: Vec4,
    pub reserved: Vec4,
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
            screen_x: 960.0,
            screen_y: 640.0,
            padding_a: 0.0,
            padding_b: 0.0,
            padding_c: 0.0,
            forward: Vec4::Z,
            right: Vec4::X,
            up: Vec4::Y,
            reserved: Vec4::W,
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
    let Projection::Perspective(perspective) = projection else {
        return;
    };
    let position = transform.translation();
    let tangent = (perspective.fov * 0.5).tan();
    let aspect = size.x.max(1.0) / size.y.max(1.0);
    let forward = transform.forward().as_vec3();
    let right = transform.right().as_vec3() * tangent * aspect;
    let up = transform.up().as_vec3() * tangent;

    for (_, material) in materials.iter_mut() {
        material.params.camera_x = position.x;
        material.params.camera_y = position.y;
        material.params.camera_z = position.z;
        material.params.screen_x = material.params.screen_x.max(size.x);
        material.params.screen_y = material.params.screen_y.max(size.y);
        material.params.forward = forward.extend(0.0);
        material.params.right = right.extend(0.0);
        material.params.up = up.extend(0.0);
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


