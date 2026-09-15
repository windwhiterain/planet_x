use bevy::image::Image;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};

/// 行星表面的自写材质：`art/shaders/surface.wgsl` 的绑定那一半。
/// 口径、判据与踩过的坑都在 `06-clouds.md` §59。
///
/// 行星表面材质的整档参数。**直接光与环境光的强度不在这里**：它们从光源 uniform
/// （`lights`）里读，只有一份来源（相机的 `AmbientLight` 与场景里的 `DirectionalLight`）。
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct SurfaceParams {
    /// 行星的世界朝向（`SYSTEM_TILT × spin`）。云覆盖度立方图烘在未倾斜的局部系里，
    /// 查它之前要把方向转回去 —— 与云材质拿的是同一个四元数，同一份来源。
    pub orientation: Vec4,
    /// glow（emissive）倍率。没有 glow 贴图时是 0：兜底那张白图乘 0 就是不发光。
    pub emissive: Vec4,
    /// 云壳的绝对内/外半径、覆盖度阈值 —— 三个都与云材质同一口径，
    /// 否则"云在哪里"会有两份答案。
    pub inner: f32,
    pub outer: f32,
    pub coverage: f32,
    /// 云影强度（0 = 关）与「指定高度」h（0 = 云底、1 = 云顶）。
    pub shadow: f32,
    pub height: f32,
    /// 覆盖度 → 光学深度的增益。
    pub gain: f32,
}

/// 云影默认的"有多不透明"：覆盖度 1 的那一处大约压掉 86% 的直接光。
pub const CLOUD_SHADOW_GAIN: f32 = 2.0;
/// 「指定高度」的缺省值：云带中点。取中点是因为那一层最接近"太阳光线真正穿过的那一层"。
pub const CLOUD_SHADOW_HEIGHT: f32 = 0.5;

impl SurfaceParams {
    pub fn new(inner: f32, outer: f32, coverage: f32) -> Self {
        Self {
            orientation: Vec4::new(0.0, 0.0, 0.0, 1.0),
            emissive: Vec4::ZERO,
            inner,
            outer,
            coverage,
            shadow: 0.0,
            height: CLOUD_SHADOW_HEIGHT,
            gain: CLOUD_SHADOW_GAIN,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
#[bind_group_data(SurfaceShaderKey)]
pub struct SurfaceMaterial {
    #[uniform(0)]
    pub params: SurfaceParams,
    #[texture(1)]
    #[sampler(2)]
    #[dependency]
    pub albedo: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    #[dependency]
    pub glow: Option<Handle<Image>>,
    /// 云的覆盖度立方图（`.r` 是 mask，`.gba` 是它的三轴梯度）。没有云时留 `None` +
    /// `params.shadow = 0`：云影那条路连一次采样都不会发。
    #[texture(5, dimension = "cube")]
    #[sampler(6)]
    #[dependency]
    pub coverage: Option<Handle<Image>>,
    /// 这份材质钉的 WGSL **内容版本**（0 = 还没有真本 ⇒ 画占位）。与云、大气同一条规矩：
    /// 它不进 bind group，只当管线特化的键 —— 两版 shader 各留一套管线，来回切是缓存命中。
    pub shader: u64,
}

/// 管线特化的键。**必须**带上 `shader`：同一份材质类型、两版 WGSL 就是两条管线。
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct SurfaceShaderKey(pub u64);

impl From<&SurfaceMaterial> for SurfaceShaderKey {
    fn from(material: &SurfaceMaterial) -> Self {
        Self(material.shader)
    }
}

impl Material for SurfaceMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        bevy::shader::ShaderRef::Path(crate::slots::placeholder_path(crate::slots::SURFACE))
    }

    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        // 与云材质同一条规矩：prepass / 阴影那几条由 `PrepassPipeline` 建，label 固定是
        // "prepass_pipeline"，那里留着占位（写不写颜色都不影响它们写深度）；只换主 pass。
        if descriptor.label.as_deref() == Some("prepass_pipeline") {
            return Ok(());
        }
        if let Some(handle) =
            crate::slots::version_handle(crate::slots::SURFACE, key.bind_group_data.0)
        {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader = handle;
            }
        }
        Ok(())
    }
}

pub struct SurfacePlugin;

impl Plugin for SurfacePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<SurfaceMaterial>::default());
    }
}
