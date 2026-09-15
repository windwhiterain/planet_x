use bevy::image::Image;
use bevy::material::AlphaMode;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AsBindGroup, AsBindGroupError, BindGroupLayoutEntry, BindingResources, BindingType,
    BufferBindingType, BufferInitDescriptor, BufferUsages, OwnedBindingResource, SamplerBindingType,
    ShaderStages, SpecializedMeshPipelineError, TextureSampleType, TextureViewDimension,
    UnpreparedBindGroup,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::shader::ShaderRef;

use px_protocol::scene::CullMode;

use crate::reflect::{PARAMS_BINDING, TEXTURE_SLOTS, TextureDimension};

/// 材质里绑到某一格的那张贴图。产物说绑哪一格，渲染器就绑哪一格。
#[derive(Debug, Clone)]
pub struct BoundTexture {
    pub binding: u32,
    pub image: Handle<Image>,
}

/// 管线特化的键：**哪一版 WGSL + 哪一档剔除**。
///
/// ⚠ 必须带上 `shader`：同一份材质类型、两版 WGSL 就是两条管线（两版各自留管线，
/// 来回切是缓存命中，§52.3）。剔除也必须在键里：`Material::specialize` 是个**静态**函数，
/// 拿不到 `&self`（Bevy 的材质钩子全静态），所以 per-材质 的管线状态只能从键里读
/// —— `StandardMaterial` 也是这么做的（`pbr_material.rs` 的 `bind_group_data`）。
#[derive(Clone, Copy, Hash, PartialEq, Eq, Debug)]
pub struct DocShaderKey {
    pub shader: u64,
    pub cull: u8,
}

fn cull_code(cull: CullMode) -> u8 {
    match cull {
        CullMode::Back => 0,
        CullMode::Front => 1,
        CullMode::None => 2,
    }
}

fn cull_of(code: u8) -> Option<bevy::render::render_resource::Face> {
    match code {
        0 => Some(bevy::render::render_resource::Face::Back),
        1 => Some(bevy::render::render_resource::Face::Front),
        _ => None,
    }
}

/// **通用材质**：渲染器里唯一的一种材质。
///
/// 它不认识云、不认识地表、不认识大气 —— 它只知道三件事：
/// 一块按 shader 自己声明的结构体打好的参数（`params`）、按契约那几格绑上的贴图（`textures`）、
/// 两条渲染状态（混合档、剔除档）。参数怎么排是**反射**出来的（`crate::reflect`），
/// 不是由渲染器里第二张表说了算。
///
/// 绑定布局是**固定超集**（见 `crate::reflect` 的表）：Bevy 的 `MaterialPlugin` 每种材质类型
/// 只建一份布局，所以空着的格一律填兜底贴图/采样器，shader 里声明了就一定绑得上。
#[derive(Asset, TypePath, Clone, Debug)]
pub struct DocMaterial {
    /// 打包好的 uniform 字节（长度 = 反射出来的 `params_bytes`）。
    pub params: Vec<u8>,
    pub textures: Vec<BoundTexture>,
    pub alpha: AlphaMode,
    pub cull: CullMode,
    /// 深度偏置。云壳压 −1：它整颗球都盖在行星上，不偏一点就会跟地表抢深度。
    pub depth_bias: f32,
    /// 这份材质钉的 WGSL **内容版本**（0 = 还没有真本 ⇒ 画占位）。不进 bind group。
    pub shader: u64,
}

impl DocMaterial {
    pub fn key(&self) -> DocShaderKey {
        DocShaderKey {
            shader: self.shader,
            cull: cull_code(self.cull),
        }
    }
}

impl AsBindGroup for DocMaterial {
    type Data = DocShaderKey;
    type Param = (
        bevy::ecs::system::lifetimeless::SRes<RenderAssets<GpuImage>>,
        bevy::ecs::system::lifetimeless::SRes<FallbackImage>,
    );

    fn label() -> &'static str {
        "DocMaterial"
    }

    fn bind_group_layout_entries(
        _render_device: &RenderDevice,
        _force_no_bindless: bool,
    ) -> Vec<BindGroupLayoutEntry> {
        // 参数块：布局上**不写死大小**（min_binding_size = None）—— 每个 shader 的结构体
        // 大小不同，而布局只有一份。真实大小由各自的缓冲决定，wgpu 按 shader 声明的那一份校验。
        let mut entries = vec![BindGroupLayoutEntry {
            binding: PARAMS_BINDING,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }];
        for (binding, dimension) in TEXTURE_SLOTS {
            entries.push(BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: view_of(dimension),
                    multisampled: false,
                },
                count: None,
            });
            entries.push(BindGroupLayoutEntry {
                binding: binding + 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            });
        }
        entries
    }

    fn unprepared_bind_group(
        &self,
        _layout: &bevy::render::render_resource::BindGroupLayout,
        render_device: &RenderDevice,
        param: &mut bevy::ecs::system::SystemParamItem<'_, '_, Self::Param>,
        _force_no_bindless: bool,
    ) -> Result<UnpreparedBindGroup, AsBindGroupError> {
        let (images, fallback_image) = param;
        let mut bindings: Vec<(u32, OwnedBindingResource)> = Vec::with_capacity(1 + TEXTURE_SLOTS.len() * 2);
        bindings.push((
            PARAMS_BINDING,
            OwnedBindingResource::Buffer(render_device.create_buffer_with_data(
                &BufferInitDescriptor {
                    label: Some("通用材质参数"),
                    usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                    contents: &self.params,
                },
            )),
        ));

        for (binding, dimension) in TEXTURE_SLOTS {
            let view = view_of(dimension);
            let bound = self
                .textures
                .iter()
                .find(|texture| texture.binding == binding);
            match bound {
                Some(texture) => {
                    let image = images
                        .get(&texture.image)
                        .ok_or(AsBindGroupError::RetryNextUpdate)?;
                    bindings.push((binding, OwnedBindingResource::TextureView(view, image.texture_view.clone())));
                    bindings.push((
                        binding + 1,
                        OwnedBindingResource::Sampler(
                            SamplerBindingType::Filtering,
                            image.sampler.clone(),
                        ),
                    ));
                }
                None => {
                    // 这一格产物没给 ⇒ 兜底：布局与 shader 一个字都不用改。
                    let fallback = match dimension {
                        TextureDimension::D2 => &fallback_image.d2,
                        TextureDimension::Cube => &fallback_image.cube,
                    };
                    bindings.push((
                        binding,
                        OwnedBindingResource::TextureView(view, fallback.texture_view.clone()),
                    ));
                    bindings.push((
                        binding + 1,
                        OwnedBindingResource::Sampler(
                            SamplerBindingType::Filtering,
                            fallback.sampler.clone(),
                        ),
                    ));
                }
            }
        }

        Ok(UnpreparedBindGroup {
            bindings: BindingResources(bindings),
        })
    }

    fn bind_group_data(&self) -> Self::Data {
        self.key()
    }
}

fn view_of(dimension: TextureDimension) -> TextureViewDimension {
    match dimension {
        TextureDimension::D2 => TextureViewDimension::D2,
        TextureDimension::Cube => TextureViewDimension::Cube,
    }
}

impl Material for DocMaterial {
    fn fragment_shader() -> ShaderRef {
        // 真本由场景产物在运行时装进槽里（`crate::slots`），`specialize` 再把管线指过去。
        // 这一刻还没有真本时画的是占位（洋红 + 相同的绑定）：一眼看得出不是成品。
        ShaderRef::Path(crate::slots::placeholder_path(crate::slots::MATERIAL))
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }

    fn depth_bias(&self) -> f32 {
        self.depth_bias
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = cull_of(key.bind_group_data.cull);
        // prepass / 阴影那几条管线由 `PrepassPipeline` 建，label 固定是 "prepass_pipeline"：
        // 那里留着占位（写不写颜色都不影响它们写深度）；只换主 pass。
        if descriptor.label.as_deref() == Some("prepass_pipeline") {
            return Ok(());
        }
        if let Some(handle) =
            crate::slots::version_handle(crate::slots::MATERIAL, key.bind_group_data.shader)
        {
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader = handle;
            }
        }
        Ok(())
    }
}

pub struct DocMaterialPlugin;

impl Plugin for DocMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<DocMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pipeline_key_carries_shader_and_cull_but_not_the_params() {
        let material = DocMaterial {
            params: vec![0; 16],
            textures: Vec::new(),
            alpha: AlphaMode::Opaque,
            cull: CullMode::Back,
            depth_bias: 0.0,
            shader: 7,
        };
        assert_eq!(material.key().shader, 7);
        assert_eq!(material.key().cull, 0);
        let blended = DocMaterial {
            cull: CullMode::None,
            alpha: AlphaMode::Blend,
            ..material.clone()
        };
        assert_eq!(blended.key().cull, 2);
        assert_ne!(material.key(), blended.key(), "剔除档不同就是两条管线");
        assert_eq!(blended.alpha_mode(), AlphaMode::Blend);
    }

    #[test]
    fn the_layout_is_the_fixed_superset_and_never_depends_on_the_instance() {
        // 布局是**静态**的（Bevy 每种材质类型只建一份）⇒ 它必须覆盖契约里全部格，
        // 而且一个字节都不许随实例变。这里只钉住形状：参数块 + 每张贴图各带一个采样器。
        assert_eq!(crate::reflect::MATERIAL_BIND_GROUP, 2);
        let bindings: Vec<u32> = TEXTURE_SLOTS
            .iter()
            .flat_map(|(binding, _)| [*binding, binding + 1])
            .collect();
        assert_eq!(bindings, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
