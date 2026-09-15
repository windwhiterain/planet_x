use bevy::material::AlphaMode;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};

/// 大气壳的内容：壳厚、密度、软度、色调。老路由色板默认值构造出同样的一份。
#[derive(Clone, Copy, Debug)]
pub struct AtmosphereSpec {
    /// 壳的外半径倍数（× 行星半径）。
    pub outer: f32,
    pub density: f32,
    pub softness: f32,
    pub tint: [f32; 3],
}

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
#[bind_group_data(AtmosphereShaderKey)]
pub struct AtmosphereMaterial {
    #[uniform(0)]
    pub params: AtmosphereParams,
    #[uniform(1)]
    pub tint: LinearRgba,
    /// 这份材质钉的 WGSL **内容版本**（0 = 还没有真本 ⇒ 画占位）。不进 bind group，
    /// 只当管线特化的键（道理同 `CloudsMaterial::shader`）。
    pub shader: u64,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct AtmosphereShaderKey(pub u64);

impl From<&AtmosphereMaterial> for AtmosphereShaderKey {
    fn from(material: &AtmosphereMaterial) -> Self {
        Self(material.shader)
    }
}

impl Material for AtmosphereMaterial {
    /// 同云：声明占位槽，真本由 `specialize` 按版本切。
    fn fragment_shader() -> bevy::shader::ShaderRef {
        bevy::shader::ShaderRef::Path(crate::slots::placeholder_path(crate::slots::ATMOSPHERE))
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // 只动主 pass —— 判据同云：prepass / 阴影的 label 是 "prepass_pipeline"，跳过。
        // ⚠ 不许用"和占位句柄的 AssetId 比"（跨世界 load 同一路径会拿到不同的 id，实测过）。
        if descriptor.label.as_deref() == Some("prepass_pipeline") {
            return Ok(());
        }
        if let Some(handle) =
            crate::slots::version_handle(crate::slots::ATMOSPHERE, key.bind_group_data.0)
        {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader = handle;
            }
        }
        Ok(())
    }
}

#[derive(Component)]
pub struct RequestCamera;

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
            .add_systems(Update, sync_cameras);
    }
}



