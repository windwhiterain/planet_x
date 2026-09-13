use bevy::asset::RenderAssetUsages;
use bevy::image::{
    Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor,
};
use bevy::material::AlphaMode;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat, TextureViewDescriptor,
    TextureViewDimension,
};

use crate::planet::{Field, PlanetSpec};
use px_protocol::art::{CUBE_FACES, Domain};

pub const CLOUD_BASE: f32 = 1.01;
pub const CLOUD_TOP: f32 = 1.06;

#[derive(Clone, Copy, Debug, ShaderType)]
pub struct CloudParams {
    pub orientation: Vec4,
    pub tint: Vec4,
    pub inner: f32,
    pub outer: f32,
    pub density: f32,
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub detail_scale: f32,
    pub detail_strength: f32,
    pub erode: f32,
    pub phase: f32,
    pub shadow: f32,
    pub steps: u32,
    pub sun_steps: u32,
    pub seed: u32,
    pub ablate: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ablate {
    None,
    Sun,
    Noise,
    Fetch,
}

impl Ablate {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "none" => Ok(Self::None),
            "sun" => Ok(Self::Sun),
            "noise" => Ok(Self::Noise),
            "fetch" => Ok(Self::Fetch),
            other => Err(format!(
                "--cloud-ablate 只认 none / sun / noise / fetch，不认 {other}"
            )),
        }
    }

    pub fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Sun => 1,
            Self::Noise => 2,
            Self::Fetch => 3,
        }
    }
}

impl CloudParams {
    pub fn new(inner: f32, outer: f32, density: f32) -> Self {
        Self {
            orientation: Vec4::new(0.0, 0.0, 0.0, 1.0),
            tint: Vec4::new(1.0, 0.99, 0.97, 1.0),
            inner,
            outer,
            density,
            coverage: 0.0,
            base: 0.06,
            top: 0.62,
            detail_scale: 16.0,
            detail_strength: 0.55,
            erode: 0.0,
            phase: 0.62,
            shadow: 0.85,
            steps: 56,
            sun_steps: 4,
            seed: 7,
            ablate: Ablate::None.code(),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct CloudsMaterial {
    #[uniform(0)]
    pub params: CloudParams,
    #[texture(1, dimension = "cube")]
    #[sampler(2)]
    #[dependency]
    pub coverage: Option<Handle<Image>>,
}

impl Material for CloudsMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        "shaders/clouds.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        -1.0
    }
}

#[derive(Component)]
pub struct PlanetCloud;

fn half_from_f32(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    if exponent == 0xff {
        let payload = if mantissa == 0 { 0 } else { 0x0200 };
        return sign | 0x7c00 | payload;
    }

    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00;
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let mut half = (mantissa >> shift) as u16;
        if (mantissa >> (shift - 1)) & 1 == 1 {
            half += 1;
        }
        return sign | half;
    }

    let mut half = ((unbiased as u32) << 10) as u16 | (mantissa >> 13) as u16;
    if mantissa & 0x1000 != 0 {
        half += 1;
    }
    sign | half
}

pub fn coverage_image(field: &Field) -> Result<Image, String> {
    if field.projection != Domain::CubeMap {
        return Err(format!(
            "云覆盖度需要 CubeMap 产物，这份是 {:?}",
            field.projection
        ));
    }
    let face = field.width.max(1);
    if field.height != face * CUBE_FACES {
        return Err(format!(
            "云覆盖度的行数应当是 {face} × {CUBE_FACES} = {}，实际 {}",
            face * CUBE_FACES,
            field.height
        ));
    }

    let mut bytes = Vec::with_capacity(field.data.len() * 2);
    for value in &field.data {
        bytes.extend_from_slice(&half_from_f32(value.clamp(0.0, 1.0)).to_le_bytes());
    }

    let mut image = Image::new(
        Extent3d {
            width: face,
            height: face,
            depth_or_array_layers: CUBE_FACES,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::R16Float,
        RenderAssetUsages::default(),
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        address_mode_w: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..default()
    });
    Ok(image)
}

pub fn spawn_clouds(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<CloudsMaterial>,
    images: &mut Assets<Image>,
    parent: Entity,
    orientation: Quat,
    spec: &PlanetSpec,
    density: f32,
) -> Result<String, String> {
    let coverage = crate::planet::load_field(
        spec.clouds
            .as_deref()
            .ok_or_else(|| "没有给云覆盖度".to_string())?,
    )?;
    let image = coverage_image(&coverage)?;
    let face = coverage.width;
    let handle = images.add(image);

    let inner = spec.radius * CLOUD_BASE;
    let outer = spec.radius * CLOUD_TOP;
    let Ok(sphere) = Sphere::new(outer).mesh().ico(64) else {
        return Err("云壳网格造不出来".to_string());
    };

    let mut params = CloudParams::new(inner, outer, density);
    params.orientation = Vec4::new(orientation.x, orientation.y, orientation.z, orientation.w);

    commands.entity(parent).with_children(|parent| {
        parent.spawn((
            crate::ScenePart,
            PlanetCloud,
            Mesh3d(meshes.add(sphere)),
            MeshMaterial3d(materials.add(CloudsMaterial {
                params,
                coverage: Some(handle),
            })),
            Transform::from_rotation(Quat::from_rotation_y(spec.spin)),
        ));
    });

    Ok(format!(
        "云层：{:.3}..{:.3}｜覆盖度 {}²×{}｜消光 {:.1}",
        inner, outer, face, CUBE_FACES, density,
    ))
}

pub fn warm_clouds(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<CloudsMaterial>) {
    let Ok(sphere) = Sphere::new(1.0).mesh().ico(4) else {
        return;
    };
    let mut params = CloudParams::new(CLOUD_BASE, CLOUD_TOP, 1.0);
    params.steps = 2;
    commands.spawn((
        crate::ScenePart,
        PlanetCloud,
        Mesh3d(meshes.add(sphere)),
        MeshMaterial3d(materials.add(CloudsMaterial {
            params,
            coverage: None,
        })),
        Transform::default(),
    ));
}

fn sync_cloud_shells(
    shells: Query<(&GlobalTransform, &MeshMaterial3d<CloudsMaterial>), With<PlanetCloud>>,
    mut materials: ResMut<Assets<CloudsMaterial>>,
) {
    for (transform, handle) in shells.iter() {
        let rotation = transform.rotation();
        let wanted = Vec4::new(rotation.x, rotation.y, rotation.z, rotation.w);
        let stale = materials
            .get(&handle.0)
            .map(|material| material.params.orientation != wanted)
            .unwrap_or(false);
        if !stale {
            continue;
        }
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.params.orientation = wanted;
        }
    }
}

pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CloudsMaterial>::default())
            .add_systems(Update, sync_cloud_shells);
    }
}
