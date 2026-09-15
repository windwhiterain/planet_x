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
    pub bump: f32,
    pub seed: u32,
    pub ablate: u32,
    pub slope_scale: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    /// 硬表面/法线那两条路判「碰到云了」的阈值。以前写死在 WGSL 里（0.20）。
    pub surface_level: f32,
    /// 保守上界早退的开关（0 = 关）。开着时 `cloud_field` 先拿
    /// `shape_of(cover, altitude, 1.0)` 这个精确上界比阈值，够不着就直接 0.0。
    pub bound: u32,
    /// 硬表面算不算解析梯度（0 = 直接用常量法线）。找面靠步进、梯度只用来算法线，
    /// 所以关掉它只该改变明暗，不该改变命中位置。
    pub gradient: u32,
    /// 细节风：两层各一份**幅度**（方向单位）。**缺省 0 = 不动** ⇒ 老场景与
    /// "同一场景两次出图逐字节相同"那条判据都不受影响（§61）。
    pub wind: f32,
    /// 细的那一层（skin）的幅度。两层的时间尺度不同（写在 WGSL 里）⇒ 细节互相搓动。
    pub wind_skin: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ablate {
    None,
    Sun,
    Noise,
    Fetch,
    Detail,
    Surface,
    Normals,
}

impl Ablate {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "none" => Ok(Self::None),
            "sun" => Ok(Self::Sun),
            "noise" => Ok(Self::Noise),
            "fetch" => Ok(Self::Fetch),
            "detail" => Ok(Self::Detail),
            "surface" => Ok(Self::Surface),
            "normals" => Ok(Self::Normals),
            other => Err(format!(
                "消融档只认 none / sun / noise / fetch / detail / surface / normals，不认 '{other}'"
            )),
        }
    }

    pub fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Sun => 1,
            Self::Noise => 2,
            Self::Fetch => 3,
            Self::Detail => 4,
            Self::Surface => 5,
            Self::Normals => 6,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sun => "sun",
            Self::Noise => "noise",
            Self::Fetch => "fetch",
            Self::Detail => "detail",
            Self::Surface => "surface",
            Self::Normals => "normals",
        }
    }
}

pub fn describe(ablate: Ablate) -> &'static str {
    match ablate {
        Ablate::None => "体积云",
        Ablate::Sun => "体积云（关自阴影）",
        Ablate::Noise => "体积云（关噪声）",
        Ablate::Fetch => "体积云（关覆盖度采样）",
        Ablate::Detail => "体积云（关细节法线）",
        Ablate::Surface => "硬表面",
        Ablate::Normals => "法线",
    }
}

/// 云的那一整档形状参数。场景产物按名字给的就是这一档（壳与朝向不在里面：
/// 壳随行星半径走、朝向随自转走）。`Default` 是加场景路之前写死在 `CloudParams::new`
/// 里的那些值 —— 老路与产物路必须共用同一组数，否则两份「默认」迟早会漂开。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudShape {
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub detail_scale: f32,
    pub detail_strength: f32,
    pub erode: f32,
    pub phase: f32,
    pub shadow: f32,
    pub steps: u32,
    pub bump: f32,
    pub seed: u32,
    pub slope_scale: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub surface_level: f32,
    /// 保守上界早退的开关。默认 0 = 关 ⇒ 老路与现有场景的像素一个都不许变。
    pub bound: u32,
    /// 硬表面算不算解析梯度。默认 1 = 算（就是老路写死的行为）。
    pub gradient: u32,
    /// 细节风（§61）：缺省 0 = 不动 —— 与加这个特性之前逐字节相同。
    /// 两层给不同速度才有"搓动"的观感（同速就是整块平移）。
    pub wind: f32,
    pub wind_skin: f32,
}

impl Default for CloudShape {
    fn default() -> Self {
        Self {
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            detail_scale: 16.0,
            detail_strength: 0.55,
            erode: 0.0,
            phase: 0.62,
            shadow: 1.0,
            steps: 56,
            bump: 0.85,
            seed: 7,
            slope_scale: 0.12,
            taper: 0.45,
            coverage_gain: 2.6,
            surface_level: 0.20,
            bound: 0,
            gradient: 1,
            wind: 0.0,
            wind_skin: 0.0,
        }
    }
}

impl CloudShape {
    /// 盖到 `CloudParams` 上。`ablate` 是仪器不是内容，留在这里不动。
    pub fn overlay(self, params: &mut CloudParams) {
        params.coverage = self.coverage;
        params.base = self.base;
        params.top = self.top;
        params.detail_scale = self.detail_scale;
        params.detail_strength = self.detail_strength;
        params.erode = self.erode;
        params.phase = self.phase;
        params.shadow = self.shadow;
        params.steps = self.steps;
        params.bump = self.bump;
        params.seed = self.seed;
        params.slope_scale = self.slope_scale;
        params.taper = self.taper;
        params.coverage_gain = self.coverage_gain;
        params.surface_level = self.surface_level;
        params.bound = self.bound;
        params.gradient = self.gradient;
        params.wind = self.wind;
        params.wind_skin = self.wind_skin;
    }
}

impl CloudParams {
    pub fn new(inner: f32, outer: f32, density: f32) -> Self {
        let mut params = Self {
            orientation: Vec4::new(0.0, 0.0, 0.0, 1.0),
            tint: Vec4::new(1.0, 0.99, 0.97, 1.0),
            inner,
            outer,
            density,
            coverage: 0.0,
            base: 0.0,
            top: 0.0,
            detail_scale: 0.0,
            detail_strength: 0.0,
            erode: 0.0,
            phase: 0.0,
            shadow: 0.0,
            steps: 0,
            bump: 0.0,
            seed: 0,
            ablate: Ablate::None.code(),
            slope_scale: 0.0,
            taper: 0.0,
            coverage_gain: 0.0,
            surface_level: 0.0,
            bound: 0,
            gradient: 0,
            wind: 0.0,
            wind_skin: 0.0,
        };
        CloudShape::default().overlay(&mut params);
        params
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
#[bind_group_data(CloudShaderKey)]
pub struct CloudsMaterial {
    #[uniform(0)]
    pub params: CloudParams,
    #[texture(1, dimension = "cube")]
    #[sampler(2)]
    #[dependency]
    pub coverage: Option<Handle<Image>>,
    /// 这份材质钉的 WGSL **内容版本**（0 = 还没有真本 ⇒ 画占位）。
    /// 它不进 bind group（WGSL 的绑定布局一个字节都没动）—— 它只当**管线特化的键**：
    /// Bevy 按 `AsBindGroup::Data` 分别缓存特化结果与管线，所以两版 shader 各自留一套，
    /// 来回切是缓存命中，不重编。
    pub shader: u64,
}

/// 管线特化的键。**必须**带上 `shader`：同一份材质类型、两版 WGSL 就是两条管线。
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct CloudShaderKey(pub u64);

impl From<&CloudsMaterial> for CloudShaderKey {
    fn from(material: &CloudsMaterial) -> Self {
        Self(material.shader)
    }
}

impl Material for CloudsMaterial {
    /// 声明的是**占位**槽：它同时是"还没装真本时画什么"（洋红，一眼看得出不是成品），
    /// 也是 `specialize` 认"现在这条是主 pass"的凭据（prepass/阴影的 fragment 不是它）。
    fn fragment_shader() -> bevy::shader::ShaderRef {
        bevy::shader::ShaderRef::Path(crate::slots::placeholder_path(crate::slots::CLOUDS))
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn depth_bias(&self) -> f32 {
        -1.0
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // 只动**主 pass**。认法看 descriptor 的 label，不看句柄：
        //   · prepass / 阴影那几条由 `PrepassPipeline` 建，label 是固定的 "prepass_pipeline"；
        //   · 主 pass 由 `MeshPipeline` 建，label 按 alpha 模式取 "…_mesh_pipeline" 之一
        //     （云是 "premultiplied_alpha_mesh_pipeline"）。
        // ⚠ **不能**用"和占位句柄的 AssetId 比"来认主 pass：主世界与渲染世界各自
        // `asset_server.load(同一个路径)` 会拿到**不同的 AssetId**（实测 110/111 vs 158/159）
        // ⇒ 那条路把真 shader 全挡在门外，画出来整块洋红（本轮真踩过）。
        if descriptor.label.as_deref() == Some("prepass_pipeline") {
            return Ok(());
        }
        // 这一版不在活版本表里就**什么都不做**：继续用占位，绝不猜一份别的内容。
        if let Some(handle) =
            crate::slots::version_handle(crate::slots::CLOUDS, key.bind_group_data.0)
        {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader = handle;
            }
        }
        Ok(())
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

pub fn coverage_image(field: &Field, slopes: &[&Field; 3]) -> Result<Image, String> {
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
    for slope in slopes {
        if slope.projection != Domain::CubeMap
            || slope.width != face
            || slope.height != field.height
        {
            return Err(format!(
                "云的梯度场必须和覆盖度同形（{face}×{}），这份是 {:?} {}×{}",
                field.height,
                slope.projection,
                slope.width,
                slope.height
            ));
        }
    }

    let mut bytes = Vec::with_capacity(field.data.len() * 8);
    for index in 0..field.data.len() {
        for value in [
            field.data[index],
            slopes[0].data[index],
            slopes[1].data[index],
            slopes[2].data[index],
        ] {
            bytes.extend_from_slice(&half_from_f32(value).to_le_bytes());
        }
    }

    let mut image = Image::new(
        Extent3d {
            width: face,
            height: face,
            depth_or_array_layers: CUBE_FACES,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba16Float,
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
            // 0 = 还没有真本 ⇒ 画占位。启动时槽里就是占位（`SlotsPlugin::seeded`）。
            shader: 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 「旧路行为不许变」的那一半：这十四个数是加场景路之前写在 `CloudParams::new` 里的。
    /// 它们进 `CloudShape::default` 之后仍然得是同一组 —— 谁改都得先改这一条测试。
    #[test]
    fn the_default_shape_is_the_number_set_cloud_params_new_used_to_hard_code() {
        let params = CloudParams::new(1.01, 1.06, 900.0);
        let shape = CloudShape::default();
        assert_eq!(shape.coverage, 0.35);
        assert_eq!(shape.base, 0.06);
        assert_eq!(shape.top, 0.62);
        assert_eq!(shape.detail_scale, 16.0);
        assert_eq!(shape.detail_strength, 0.55);
        assert_eq!(shape.erode, 0.0);
        assert_eq!(shape.phase, 0.62);
        assert_eq!(shape.shadow, 1.0);
        assert_eq!(shape.steps, 56);
        assert_eq!(shape.bump, 0.85);
        assert_eq!(shape.seed, 7);
        assert_eq!(shape.slope_scale, 0.12);
        assert_eq!(shape.taper, 0.45);
        assert_eq!(shape.coverage_gain, 2.6);
        assert_eq!(shape.surface_level, 0.20);
        // 上界早退默认关着：老路与现有场景的像素一个都不许变。
        assert_eq!(shape.bound, 0);
        // 解析梯度默认算（老路写死的行为）。
        assert_eq!(shape.gradient, 1);

        assert_eq!(params.coverage, shape.coverage);
        assert_eq!(params.base, shape.base);
        assert_eq!(params.top, shape.top);
        assert_eq!(params.detail_scale, shape.detail_scale);
        assert_eq!(params.detail_strength, shape.detail_strength);
        assert_eq!(params.erode, shape.erode);
        assert_eq!(params.phase, shape.phase);
        assert_eq!(params.shadow, shape.shadow);
        assert_eq!(params.steps, shape.steps);
        assert_eq!(params.bump, shape.bump);
        assert_eq!(params.seed, shape.seed);
        assert_eq!(params.slope_scale, shape.slope_scale);
        assert_eq!(params.taper, shape.taper);
        assert_eq!(params.coverage_gain, shape.coverage_gain);
        assert_eq!(params.surface_level, shape.surface_level);
        assert_eq!(params.bound, shape.bound);
        assert_eq!(params.gradient, shape.gradient);

        // 壳与朝向不在「形状」里：前者随行星半径走，后者随自转走。
        assert_eq!(params.inner, 1.01);
        assert_eq!(params.outer, 1.06);
        assert_eq!(params.density, 900.0);
        assert_eq!(params.ablate, Ablate::None.code());
    }

    #[test]
    fn the_shape_overlays_only_the_fields_it_owns() {
        let mut params = CloudParams::new(1.0, 1.14, 900.0);
        CloudShape {
            coverage: 0.9,
            steps: 8,
            ..CloudShape::default()
        }
        .overlay(&mut params);
        assert_eq!(params.coverage, 0.9);
        assert_eq!(params.steps, 8);
        assert_eq!(params.inner, 1.0, "形状不该动壳");
        assert_eq!(params.outer, 1.14);
        assert_eq!(params.density, 900.0);
    }
}
