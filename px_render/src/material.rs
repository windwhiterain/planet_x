use std::collections::HashMap;
use std::sync::Arc;

use px_protocol::material::{MATERIAL_BIND_GROUP, PARAMS_BINDING, TEXTURE_SLOTS, TextureDimension};
use px_protocol::scene::{AlphaMode, CullMode, Sampler};
use wgpu::util::{DeviceExt, TextureDataOrder};

use crate::art::{BoundTexture, LoadedObject, LoadedTexture};
use crate::shot::FORMAT;

pub const FRAGMENT_ENTRY: &str = "fragment";

pub const VERTEX_ENTRY: &str = "vertex";

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

pub const FALLBACK_PIXEL: [u8; 4] = [255, 255, 255, 255];

pub const FALLBACK_SIZE: u32 = 1;

pub const FALLBACK_CUBE_LAYERS: u32 = 6;

pub fn version_of(key: &str) -> Result<u64, String> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "shader 成员的内容键应当是 64 位十六进制，实际是 '{key}'"
        ));
    }
    u64::from_str_radix(&key[..16], 16).map_err(|err| format!("内容键 '{key}' 解不开：{err}"))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipelineKey {
    pub shader: u64,
    pub cull: u8,
    pub alpha: u8,
}

impl PipelineKey {
    pub fn new(shader: u64, cull: CullMode, alpha: AlphaMode) -> PipelineKey {
        PipelineKey {
            shader,
            cull: cull_code(cull),
            alpha: alpha_code(alpha),
        }
    }

    pub fn label(self) -> String {
        format!(
            "material {:016x} cull={} alpha={}",
            self.shader, self.cull, self.alpha
        )
    }

    pub fn cull_face(self) -> Option<wgpu::Face> {
        match self.cull {
            0 => Some(wgpu::Face::Back),
            1 => Some(wgpu::Face::Front),
            _ => None,
        }
    }

    pub fn blend(self) -> Option<wgpu::BlendState> {
        match self.alpha {
            2 => Some(wgpu::BlendState::ALPHA_BLENDING),
            1 | 3 => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            _ => None,
        }
    }

    pub fn depth_write(self) -> bool {
        self.alpha == 0
    }
}

pub fn cull_code(cull: CullMode) -> u8 {
    match cull {
        CullMode::Back => 0,
        CullMode::Front => 1,
        CullMode::None => 2,
    }
}

pub fn alpha_code(alpha: AlphaMode) -> u8 {
    match alpha {
        AlphaMode::Opaque => 0,
        AlphaMode::Premultiplied => 1,
        AlphaMode::Blend => 2,
        AlphaMode::Add => 3,
    }
}

pub fn key_of(object: &LoadedObject) -> Result<PipelineKey, String> {
    let version = version_of(&object.shader.member.key)
        .map_err(|err| format!("物体 '{}' 的 shader 成员：{err}", object.id))?;
    Ok(PipelineKey::new(version, object.cull, object.alpha))
}

pub fn view_dimension(dimension: TextureDimension) -> wgpu::TextureViewDimension {
    match dimension {
        TextureDimension::D2 => wgpu::TextureViewDimension::D2,
        TextureDimension::Cube => wgpu::TextureViewDimension::Cube,
        TextureDimension::D2Array => wgpu::TextureViewDimension::D2Array,
    }
}

pub fn layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: PARAMS_BINDING,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }];
    for (binding, dimension) in TEXTURE_SLOTS {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: view_dimension(dimension),
                multisampled: false,
            },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: binding + 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
    }
    entries
}

pub struct Fallbacks {
    pub pixel: [u8; 4],
    pub texture_2d: wgpu::Texture,
    pub view_2d: wgpu::TextureView,
    pub texture_cube: wgpu::Texture,
    pub view_cube: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl Fallbacks {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Fallbacks {
        let descriptor = |layers: u32| wgpu::TextureDescriptor {
            label: Some("material fallback"),
            size: wgpu::Extent3d {
                width: FALLBACK_SIZE,
                height: FALLBACK_SIZE,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let pixels: Vec<u8> = FALLBACK_PIXEL
            .iter()
            .copied()
            .cycle()
            .take(FALLBACK_PIXEL.len() * FALLBACK_CUBE_LAYERS as usize)
            .collect();
        let texture_2d = device.create_texture_with_data(
            queue,
            &descriptor(1),
            TextureDataOrder::LayerMajor,
            &FALLBACK_PIXEL,
        );
        let texture_cube = device.create_texture_with_data(
            queue,
            &descriptor(FALLBACK_CUBE_LAYERS),
            TextureDataOrder::LayerMajor,
            &pixels,
        );
        let view_2d = texture_2d.create_view(&wgpu::TextureViewDescriptor::default());
        let view_cube = texture_cube.create_view(&wgpu::TextureViewDescriptor {
            label: Some("material fallback cube"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("material fallback sampler"),
            ..Default::default()
        });
        Fallbacks {
            pixel: FALLBACK_PIXEL,
            texture_2d,
            view_2d,
            texture_cube,
            view_cube,
            sampler,
        }
    }

    pub fn view(&self, dimension: TextureDimension) -> &wgpu::TextureView {
        match dimension {
            TextureDimension::D2 => &self.view_2d,
            TextureDimension::Cube => &self.view_cube,
            TextureDimension::D2Array => &self.view_2d,
        }
    }

    pub fn texture(&self, dimension: TextureDimension) -> &wgpu::Texture {
        match dimension {
            TextureDimension::D2 => &self.texture_2d,
            TextureDimension::Cube => &self.texture_cube,
            TextureDimension::D2Array => &self.texture_2d,
        }
    }
}

pub fn address_mode(mode: px_protocol::scene::Address) -> wgpu::AddressMode {
    match mode {
        px_protocol::scene::Address::Repeat => wgpu::AddressMode::Repeat,
        px_protocol::scene::Address::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        px_protocol::scene::Address::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}

pub fn filter_mode(mode: px_protocol::scene::Filter) -> wgpu::FilterMode {
    match mode {
        px_protocol::scene::Filter::Linear => wgpu::FilterMode::Linear,
        px_protocol::scene::Filter::Nearest => wgpu::FilterMode::Nearest,
    }
}

pub fn sampler_of(device: &wgpu::Device, sampler: &Sampler) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("material sampler"),
        address_mode_u: address_mode(sampler.address_u),
        address_mode_v: address_mode(sampler.address_v),
        address_mode_w: address_mode(sampler.address_v),
        mag_filter: filter_mode(sampler.filter),
        min_filter: filter_mode(sampler.filter),
        mipmap_filter: match sampler.filter {
            px_protocol::scene::Filter::Linear => wgpu::MipmapFilterMode::Linear,
            px_protocol::scene::Filter::Nearest => wgpu::MipmapFilterMode::Nearest,
        },
        lod_min_clamp: 0.0,
        lod_max_clamp: 32.0,
        compare: None,
        anisotropy_clamp: sampler.anisotropy.clamp(1, 16) as u16,
        border_color: None,
    })
}

pub fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &LoadedTexture,
) -> (wgpu::Texture, wgpu::TextureView) {
    let format = match texture.shape.format {
        px_protocol::art::TextureFormat::Rgba8Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        px_protocol::art::TextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
    };
    let descriptor = wgpu::TextureDescriptor {
        label: Some("material texture"),
        size: wgpu::Extent3d {
            width: texture.shape.width,
            height: texture.shape.height,
            depth_or_array_layers: texture.shape.layers,
        },
        mip_level_count: texture.shape.levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    };
    let gpu = device.create_texture_with_data(
        queue,
        &descriptor,
        TextureDataOrder::LayerMajor,
        &texture.bytes,
    );
    let view = gpu.create_view(&wgpu::TextureViewDescriptor {
        label: Some("material texture view"),
        dimension: Some(view_dimension(if texture.shape.layers == 6 {
            TextureDimension::Cube
        } else {
            TextureDimension::D2
        })),
        ..Default::default()
    });
    (gpu, view)
}

pub fn pipeline_layout(
    device: &wgpu::Device,
    group_zero: &wgpu::BindGroupLayout,
    material: &wgpu::BindGroupLayout,
    stage: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    let mut groups: Vec<Option<&wgpu::BindGroupLayout>> =
        vec![None; MATERIAL_BIND_GROUP as usize + 1];
    groups[0] = Some(group_zero);
    groups[1] = Some(stage);
    groups[MATERIAL_BIND_GROUP as usize] = Some(material);
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("material pipeline layout"),
        bind_group_layouts: &groups,
        immediate_size: 0,
    })
}

pub struct MaterialBinding {
    pub key: PipelineKey,
    pub params: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub bound_slots: Vec<u32>,
    pub fallback_slots: Vec<u32>,
}

pub struct BindingRequest<'a> {
    pub key: PipelineKey,
    pub label: &'a str,
    pub params: &'a [u8],
    pub textures: &'a [BoundTexture],
}

pub fn frame_key(version: u64) -> PipelineKey {
    PipelineKey::new(version, CullMode::None, AlphaMode::Opaque)
}

pub struct PipelineRequest<'a> {
    pub key: PipelineKey,
    pub vertex: &'a wgpu::ShaderModule,
    pub fragment: &'a wgpu::ShaderModule,
    pub pipeline_layout: &'a wgpu::PipelineLayout,
    pub vertex_buffers: &'a [wgpu::VertexBufferLayout<'a>],
}

pub struct Materials {
    layout: wgpu::BindGroupLayout,
    fallbacks: Fallbacks,
    samplers: HashMap<Sampler, wgpu::Sampler>,
    pipelines: HashMap<PipelineKey, Arc<wgpu::RenderPipeline>>,
}

impl Materials {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Materials {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material bind group layout"),
            entries: &layout_entries(),
        });
        Materials {
            layout,
            fallbacks: Fallbacks::new(device, queue),
            samplers: HashMap::new(),
            pipelines: HashMap::new(),
        }
    }

    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn fallbacks(&self) -> &Fallbacks {
        &self.fallbacks
    }

    pub fn sampler(&mut self, device: &wgpu::Device, sampler: &Sampler) -> wgpu::Sampler {
        if let Some(cached) = self.samplers.get(sampler) {
            return cached.clone();
        }
        let built = sampler_of(device, sampler);
        self.samplers.insert(*sampler, built.clone());
        built
    }

    pub fn bind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        object: &LoadedObject,
    ) -> Result<MaterialBinding, String> {
        let request = BindingRequest {
            key: key_of(object)?,
            label: object.id.as_str(),
            params: object.params.as_slice(),
            textures: object.textures.as_slice(),
        };
        self.bind_request(device, queue, &request)
    }

    pub fn bind_request(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        request: &BindingRequest<'_>,
    ) -> Result<MaterialBinding, String> {
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material params"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            contents: request.params,
        });

        let mut views: Vec<wgpu::TextureView> = Vec::with_capacity(TEXTURE_SLOTS.len());
        let mut samplers: Vec<wgpu::Sampler> = Vec::with_capacity(TEXTURE_SLOTS.len());
        let mut bound_slots = Vec::new();
        let mut fallback_slots = Vec::new();
        for (binding, dimension) in TEXTURE_SLOTS {
            match request
                .textures
                .iter()
                .find(|bound| bound.binding == binding)
            {
                Some(bound) => {
                    let (_, view) = upload_texture(device, queue, &bound.texture);
                    views.push(view);
                    samplers.push(self.sampler(device, &bound.sampler));
                    bound_slots.push(binding);
                }
                None => {
                    views.push(self.fallbacks.view(dimension).clone());
                    samplers.push(self.fallbacks.sampler.clone());
                    fallback_slots.push(binding);
                }
            }
        }

        let mut entries: Vec<wgpu::BindGroupEntry> = vec![wgpu::BindGroupEntry {
            binding: PARAMS_BINDING,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &params_buffer,
                offset: 0,
                size: None,
            }),
        }];
        for (index, (binding, _)) in TEXTURE_SLOTS.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: *binding,
                resource: wgpu::BindingResource::TextureView(&views[index]),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: binding + 1,
                resource: wgpu::BindingResource::Sampler(&samplers[index]),
            });
        }

        let label = format!("material bind group（{}）", request.label);
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label.as_str()),
            layout: &self.layout,
            entries: &entries,
        });
        Ok(MaterialBinding {
            key: request.key,
            params: params_buffer,
            bind_group,
            bound_slots,
            fallback_slots,
        })
    }

    pub fn pipeline(
        &mut self,
        device: &wgpu::Device,
        request: &PipelineRequest<'_>,
    ) -> Arc<wgpu::RenderPipeline> {
        if let Some(cached) = self.pipelines.get(&request.key) {
            return cached.clone();
        }
        let key = request.key;
        let label = key.label();
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label.as_str()),
            layout: Some(request.pipeline_layout),
            vertex: wgpu::VertexState {
                module: request.vertex,
                entry_point: Some(VERTEX_ENTRY),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: request.vertex_buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: key.cull_face(),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(key.depth_write()),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: request.fragment,
                entry_point: Some(FRAGMENT_ENTRY),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: key.blend(),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let pipeline = Arc::new(pipeline);
        self.pipelines.insert(key, pipeline.clone());
        pipeline
    }

    pub fn cached(&self, key: PipelineKey) -> Option<Arc<wgpu::RenderPipeline>> {
        self.pipelines.get(&key).cloned()
    }

    pub fn len(&self) -> usize {
        self.pipelines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pipelines.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use px_protocol::art::TextureFormat;
    use px_protocol::art::TextureShape;
    use px_protocol::scene::Address;
    use px_protocol::scene::Filter;
    use std::path::PathBuf;

    const SCENE_LIST: &str = "target/oracle/orbit-bare-nolight.txt";

    fn scene_path() -> Option<PathBuf> {
        let list = crate::shader::workspace().join(SCENE_LIST);
        if !list.exists() {
            println!("⚠ 跳过：{} 不在（target/ 不入 git）", list.display());
            return None;
        }
        let text = std::fs::read_to_string(&list)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", list.display()));
        let path = PathBuf::from(text.trim());
        if !path.exists() {
            println!(
                "⚠ 跳过：场景产物 {} 不在（target/ 不入 git）",
                path.display()
            );
            return None;
        }
        Some(path)
    }

    #[test]
    fn the_states_follow_the_document_and_the_contract_table() {
        for alpha in [AlphaMode::Add, AlphaMode::Premultiplied] {
            let key = PipelineKey::new(7, CullMode::Back, alpha);
            assert_eq!(
                key.blend(),
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                "{alpha:?} 不是加法混合：Bevy 0.19 里它与 Premultiplied 同档"
            );
            let blend = key.blend().expect("有混合");
            assert_eq!(blend.color.src_factor, wgpu::BlendFactor::One);
            assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::OneMinusSrcAlpha);
            assert!(!key.depth_write(), "透明档不写深度");
        }
        let blend = PipelineKey::new(7, CullMode::Back, AlphaMode::Blend);
        assert_eq!(blend.blend(), Some(wgpu::BlendState::ALPHA_BLENDING));
        assert_eq!(
            blend.blend().expect("有混合").color.src_factor,
            wgpu::BlendFactor::SrcAlpha
        );
        assert!(!blend.depth_write());

        let opaque = PipelineKey::new(7, CullMode::Back, AlphaMode::Opaque);
        assert_eq!(opaque.blend(), None, "不透明档不混合");
        assert!(opaque.depth_write(), "不透明档写深度");

        assert_eq!(
            PipelineKey::new(7, CullMode::Back, AlphaMode::Opaque).cull_face(),
            Some(wgpu::Face::Back)
        );
        assert_eq!(
            PipelineKey::new(7, CullMode::Front, AlphaMode::Opaque).cull_face(),
            Some(wgpu::Face::Front)
        );
        assert_eq!(
            PipelineKey::new(7, CullMode::None, AlphaMode::Opaque).cull_face(),
            None
        );

        let base = PipelineKey::new(7, CullMode::Back, AlphaMode::Opaque);
        assert_ne!(base, PipelineKey::new(8, CullMode::Back, AlphaMode::Opaque));
        assert_ne!(base, PipelineKey::new(7, CullMode::None, AlphaMode::Opaque));
        assert_ne!(base, PipelineKey::new(7, CullMode::Back, AlphaMode::Add));

        assert_eq!(
            version_of("f679cdf810156a75ee3dcd477c0d9c9871c34574eacbd20398407996d3e6136c").unwrap(),
            0xf679_cdf8_1015_6a75
        );
        assert!(version_of("f679cdf8").is_err());
        assert!(version_of(&"z".repeat(64)).is_err());

        let entries = layout_entries();
        assert_eq!(entries.len(), 1 + TEXTURE_SLOTS.len() * 2);
        assert_eq!(entries[0].binding, PARAMS_BINDING);
        assert!(matches!(
            entries[0].ty,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            }
        ));
        assert_eq!(MATERIAL_BIND_GROUP, 3, "运行期是 Bevy 说了算");
        for (index, (binding, dimension)) in TEXTURE_SLOTS.iter().enumerate() {
            let texture = &entries[index * 2 + 1];
            let sampler = &entries[index * 2 + 2];
            assert_eq!(texture.binding, *binding);
            assert_eq!(sampler.binding, binding + 1, "采样器永远在贴图 + 1");
            match texture.ty {
                wgpu::BindingType::Texture {
                    view_dimension: found,
                    multisampled: false,
                    ..
                } => assert_eq!(found, view_dimension(*dimension)),
                other => panic!("第 {binding} 格不是贴图：{other:?}"),
            }
            assert!(matches!(
                sampler.ty,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
            ));
        }
        assert_eq!(FALLBACK_PIXEL, [255, 255, 255, 255], "兜底图是白的");
    }

    #[test]
    fn the_sampler_settings_are_named_the_way_the_document_names_them() {
        assert_eq!(address_mode(Address::Repeat), wgpu::AddressMode::Repeat);
        assert_eq!(
            address_mode(Address::ClampToEdge),
            wgpu::AddressMode::ClampToEdge
        );
        assert_eq!(
            address_mode(Address::MirrorRepeat),
            wgpu::AddressMode::MirrorRepeat
        );
        assert_eq!(filter_mode(Filter::Linear), wgpu::FilterMode::Linear);
        assert_eq!(filter_mode(Filter::Nearest), wgpu::FilterMode::Nearest);
        assert_ne!(
            address_mode(Address::Repeat),
            address_mode(Address::ClampToEdge)
        );
        assert_ne!(
            address_mode(Address::ClampToEdge),
            address_mode(Address::MirrorRepeat)
        );
        assert_eq!(0_u32.clamp(1, 16), 1, "产物写 0 = 不设 ⇒ 落回 1");
        assert_eq!(64_u32.clamp(1, 16), 16, "wgpu 的上限是 16");

        assert_ne!(Sampler::clamped(), Sampler::default());
        assert_eq!(Sampler::clamped().address_u, Address::ClampToEdge);
        assert_eq!(Sampler::default().address_u, Address::Repeat);
        assert_eq!(
            Sampler::clamped().address_v,
            Sampler::default().address_v,
            "两者 v 轴都是 ClampToEdge —— 差别**只在** u 轴"
        );
        for sampler in [Sampler::clamped(), Sampler::default()] {
            assert_eq!(
                sampler.filter,
                Filter::Linear,
                "本工程的缺省过滤是 Linear（Bevy 的 ImageSamplerDescriptor::default() 是 Nearest）"
            );
        }
    }

    #[test]
    fn the_fallback_shapes_are_the_ones_the_contract_needs() {
        assert_eq!(FALLBACK_SIZE, 1);
        assert_eq!(FALLBACK_CUBE_LAYERS, 6, "契约里 cube 那一档就是 6 层");
        assert_eq!(
            px_protocol::material::TextureDimension::Cube.layers(),
            FALLBACK_CUBE_LAYERS
        );
        assert_eq!(FALLBACK_PIXEL, [255, 255, 255, 255]);
        assert_eq!(
            DEPTH_FORMAT,
            wgpu::TextureFormat::Depth32Float,
            "无限 reverse-Z"
        );
        assert_eq!(FORMAT, wgpu::TextureFormat::Rgba8UnormSrgb);
    }

    const VERTEX_PROBE: &str = "struct VertexOutput {\n\
        \x20   @builtin(position) position: vec4<f32>,\n\
        \x20   @location(0) world_position: vec4<f32>,\n\
        \x20   @location(1) world_normal: vec3<f32>,\n\
        \x20   @location(2) uv: vec2<f32>,\n\
        };\n\
        @vertex\n\
        fn vertex(\n\
        \x20   @location(0) position: vec3<f32>,\n\
        \x20   @location(1) normal: vec3<f32>,\n\
        \x20   @location(2) uv: vec2<f32>,\n\
        ) -> VertexOutput {\n\
        \x20   var out: VertexOutput;\n\
        \x20   out.position = vec4<f32>(position, 1.0);\n\
        \x20   out.world_position = vec4<f32>(position, 1.0);\n\
        \x20   out.world_normal = normal;\n\
        \x20   out.uv = uv;\n\
        \x20   return out;\n\
        }\n";

    const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2];

    fn group_zero_layout(
        device: &wgpu::Device,
        module: &naga::Module,
    ) -> (wgpu::BindGroupLayout, Vec<(u32, u32, String)>) {
        let mut entries = Vec::new();
        let mut rows = Vec::new();
        for (_, global) in module.global_variables.iter() {
            let Some(binding) = global.binding else {
                continue;
            };
            if binding.group != 0 {
                continue;
            }
            let name = global.name.clone().unwrap_or_else(|| "?".to_string());
            let entry = match (&global.space, &module.types[global.ty].inner) {
                (naga::AddressSpace::Uniform, _) => wgpu::BindGroupLayoutEntry {
                    binding: binding.binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                (naga::AddressSpace::Storage { .. }, _) => wgpu::BindGroupLayoutEntry {
                    binding: binding.binding,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                (
                    naga::AddressSpace::Handle,
                    naga::TypeInner::Image {
                        dim,
                        arrayed,
                        class,
                    },
                ) => {
                    let view_dimension = match (dim, arrayed) {
                        (naga::ImageDimension::D2, false) => wgpu::TextureViewDimension::D2,
                        (naga::ImageDimension::D2, true) => wgpu::TextureViewDimension::D2Array,
                        (naga::ImageDimension::Cube, false) => wgpu::TextureViewDimension::Cube,
                        (naga::ImageDimension::Cube, true) => wgpu::TextureViewDimension::CubeArray,
                        (other, arrayed) => {
                            panic!("组 0 的贴图维度不认识：{other:?}（数组 {arrayed}）")
                        }
                    };
                    let sample_type = match class {
                        naga::ImageClass::Depth { .. } => wgpu::TextureSampleType::Depth,
                        _ => wgpu::TextureSampleType::Float { filterable: true },
                    };
                    wgpu::BindGroupLayoutEntry {
                        binding: binding.binding,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type,
                            view_dimension,
                            multisampled: false,
                        },
                        count: None,
                    }
                }
                (naga::AddressSpace::Handle, naga::TypeInner::Sampler { comparison: true }) => {
                    wgpu::BindGroupLayoutEntry {
                        binding: binding.binding,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    }
                }
                (space, inner) => panic!("组 0 多了个不认识的全局变量 {name}：{space:?} {inner:?}"),
            };
            entries.push(entry);
            rows.push((binding.group, binding.binding, name));
        }
        rows.sort();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("group 0 probe layout"),
            entries: &entries,
        });
        (layout, rows)
    }

    fn declared_material_slots(module: &naga::Module) -> Vec<(u32, wgpu::TextureViewDimension)> {
        let mut slots = Vec::new();
        for (_, global) in module.global_variables.iter() {
            let Some(binding) = global.binding else {
                continue;
            };
            if binding.group != MATERIAL_BIND_GROUP {
                continue;
            }
            if let naga::TypeInner::Image { dim, .. } = &module.types[global.ty].inner {
                let dimension = match dim {
                    naga::ImageDimension::D2 => wgpu::TextureViewDimension::D2,
                    naga::ImageDimension::Cube => wgpu::TextureViewDimension::Cube,
                    other => panic!("材质贴图维度不认识：{other:?}"),
                };
                slots.push((binding.binding, dimension));
            }
        }
        slots.sort_by_key(|slot| slot.0);
        slots
    }

    fn read_pixel(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> [u8; 4] {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fallback readback"),
            size: wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fallback readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(60)),
            })
            .expect("等回读超时");
        receiver.recv().expect("映射没有回调").expect("映射失败");
        let data = slice.get_mapped_range();
        let pixel = [data[0], data[1], data[2], data[3]];
        drop(data);
        buffer.unmap();
        pixel
    }

    #[test]
    fn the_material_bind_group_and_pipeline_build_for_both_objects() {
        let Some(path) = scene_path() else {
            println!("⚠ 这一档判据没跑（见上面那行）：不是通过，是没测");
            return;
        };
        let root = crate::art::default_pcg_root();
        if !root.exists() {
            println!("⚠ 跳过：CAS 根 {} 不在（target/ 不入 git）", root.display());
            return;
        }
        let scene = crate::art::load_scene(&path, &root)
            .unwrap_or_else(|err| panic!("载入 {} 失败：{err}", path.display()));

        let gpu = crate::gpu::connect();
        let mut materials = Materials::new(&gpu.device, &gpu.queue);

        assert_eq!(
            materials.fallbacks().pixel,
            [255, 255, 255, 255],
            "上传前的那四个字节"
        );
        assert_eq!(
            read_pixel(
                &gpu.device,
                &gpu.queue,
                materials.fallbacks().texture(TextureDimension::D2)
            ),
            [255, 255, 255, 255],
            "兜底 2D 图上真的那一个像素"
        );
        assert_eq!(
            read_pixel(
                &gpu.device,
                &gpu.queue,
                materials.fallbacks().texture(TextureDimension::Cube)
            ),
            [255, 255, 255, 255],
            "兜底 cube 的第一层"
        );

        let vertex = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("material test vertex"),
                source: wgpu::ShaderSource::Wgsl(VERTEX_PROBE.into()),
            });
        let vertex_buffers = [wgpu::VertexBufferLayout {
            array_stride: 32,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &VERTEX_ATTRIBUTES,
        }];

        let mut keys = Vec::new();
        for id in ["planet", "atmosphere"] {
            let object = scene.object(id).expect("物体在");
            let module =
                crate::shader::validate(&format!("{id}（组装后）"), &object.shader.assembled)
                    .unwrap_or_else(|err| panic!("{id} 的组装文本过不了 naga：{err}"));

            let declared = declared_material_slots(&module);
            let entries = layout_entries();
            let mut bound = Vec::new();
            for (binding, dimension) in &declared {
                let texture = entries
                    .iter()
                    .find(|entry| entry.binding == *binding)
                    .unwrap_or_else(|| panic!("{id} 声明了第 {binding} 格，而布局里没有"));
                match texture.ty {
                    wgpu::BindingType::Texture {
                        view_dimension: found,
                        ..
                    } => assert_eq!(found, *dimension, "{id} 第 {binding} 格的维度与布局不一致"),
                    ref other => panic!("{id} 第 {binding} 格在布局里不是贴图：{other:?}"),
                }
                assert!(
                    entries.iter().any(|entry| entry.binding == binding + 1
                        && matches!(
                            entry.ty,
                            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)
                        )),
                    "{id} 第 {binding} 格的采样器不在布局里"
                );
                bound.push(*binding);
            }

            let material = materials
                .bind(&gpu.device, &gpu.queue, object)
                .unwrap_or_else(|err| panic!("{id} 的绑定组建不出来：{err}"));
            assert_eq!(
                material.bound_slots,
                object
                    .textures
                    .iter()
                    .map(|bound| bound.binding)
                    .collect::<Vec<_>>(),
                "{id}：绑定组里「产物真给了」的那几格"
            );
            for slot in &declared {
                if !material.bound_slots.contains(&slot.0) {
                    assert!(
                        material.fallback_slots.contains(&slot.0),
                        "{id}：第 {} 格 shader 声明了、产物没给 ⇒ 必须是兜底",
                        slot.0
                    );
                }
            }
            for slot in &material.fallback_slots {
                assert!(
                    !material.bound_slots.contains(slot),
                    "{id}：第 {slot} 格不可能既是兜底又是绑上的"
                );
            }
            assert_eq!(
                material.bound_slots.len() + material.fallback_slots.len(),
                TEXTURE_SLOTS.len(),
                "{id}：每一格都必须有归属（绑上的或兜底的）"
            );

            let fragment = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(id),
                    source: wgpu::ShaderSource::Wgsl(object.shader.assembled.as_str().into()),
                });
            let (group_zero, rows) = group_zero_layout(&gpu.device, &module);
            let known = [
                (0, 0, "view".to_string()),
                (0, 1, "lights".to_string()),
                (0, 2, "point_shadow_textures".to_string()),
                (0, 3, "point_shadow_textures_comparison_sampler".to_string()),
                (0, 8, "clustered_lights".to_string()),
                (0, 11, "globals".to_string()),
                (0, 20, "depth_prepass_texture".to_string()),
            ];
            for row in &rows {
                assert!(
                    known.contains(row),
                    "{id} 的组 0 冒出一个契约外的绑定：{row:?}（契约见 group0.rs）"
                );
            }
            let expected: Vec<&str> = match id {
                "planet" => vec![
                    "view",
                    "lights",
                    "point_shadow_textures",
                    "point_shadow_textures_comparison_sampler",
                    "clustered_lights",
                ],
                _ => vec!["view", "clustered_lights", "depth_prepass_texture"],
            };
            let names: Vec<&str> = rows.iter().map(|row| row.2.as_str()).collect();
            assert_eq!(
                names, expected,
                "{id} 的组 0 声明变了：替身布局与管线布局都要跟着改"
            );
            let counts: Vec<u32> = rows.iter().map(|row| row.1).collect();
            assert_eq!(counts, {
                let mut sorted = counts.clone();
                sorted.sort();
                sorted
            });
            let material_layout = materials.bind_group_layout().clone();
            let stage_layout = group_zero.clone();
            let pipeline_layout =
                pipeline_layout(&gpu.device, &group_zero, &material_layout, &stage_layout);
            let pipeline = materials.pipeline(
                &gpu.device,
                &PipelineRequest {
                    key: material.key,
                    vertex: &vertex,
                    fragment: &fragment,
                    pipeline_layout: &pipeline_layout,
                    vertex_buffers: &vertex_buffers,
                },
            );
            assert_eq!(
                materials
                    .cached(material.key)
                    .map(|found| Arc::as_ptr(&found)),
                Some(Arc::as_ptr(&pipeline)),
                "{id}：同一个键必须拿回同一个 Arc（缓存命中，不重编）"
            );
            println!(
                "{id}｜键 shader={:016x} cull={} alpha={}｜声明 {:?}｜绑上 {:?}｜兜底 {} 格",
                material.key.shader,
                material.key.cull,
                material.key.alpha,
                declared,
                material.bound_slots,
                material.fallback_slots.len()
            );
            keys.push(material.key);
        }

        assert_ne!(keys[0], keys[1], "planet 与 atmosphere 的状态不同");
        assert_eq!(keys[0].cull, keys[1].cull, "剔除档都是 back");
        assert_ne!(keys[0].alpha, keys[1].alpha, "混合档一个 opaque 一个 add");
        assert_ne!(
            keys[0].shader, keys[1].shader,
            "两份 shader 的内容版本必须不同（否则它们会共用一条管线）"
        );
        assert_eq!(materials.len(), 2, "两条管线，不多不少");

        let planet = scene.object("planet").expect("planet 在");
        let module = crate::shader::validate("planet（组装后）", &planet.shader.assembled)
            .expect("组装文本过不了 naga");
        let fragment = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("planet"),
                source: wgpu::ShaderSource::Wgsl(planet.shader.assembled.as_str().into()),
            });
        let (group_zero, _) = group_zero_layout(&gpu.device, &module);
        let material_layout = materials.bind_group_layout().clone();
        let stage_layout = group_zero.clone();
        let pipeline_layout =
            pipeline_layout(&gpu.device, &group_zero, &material_layout, &stage_layout);
        let again = materials.pipeline(
            &gpu.device,
            &PipelineRequest {
                key: keys[0],
                vertex: &vertex,
                fragment: &fragment,
                pipeline_layout: &pipeline_layout,
                vertex_buffers: &vertex_buffers,
            },
        );
        assert_eq!(materials.len(), 2, "同键不该长出第三条管线");
        assert_eq!(
            materials.cached(keys[0]).map(|found| Arc::as_ptr(&found)),
            Some(Arc::as_ptr(&again))
        );
        assert!(!materials.is_empty());
    }

    #[test]
    fn the_texture_upload_follows_the_artifact_shape() {
        let list = crate::shader::workspace().join(SCENE_LIST);
        if !list.exists() {
            println!("⚠ 跳过：{} 不在（target/ 不入 git）", list.display());
            return;
        }
        let Some(path) = scene_path() else {
            return;
        };
        let root = crate::art::default_pcg_root();
        if !root.exists() {
            println!("⚠ 跳过：CAS 根 {} 不在（target/ 不入 git）", root.display());
            return;
        }
        let scene = crate::art::load_scene(&path, &root).expect("载入场景");
        let gpu = crate::gpu::connect();

        let planet = scene.object("planet").expect("planet 在");
        let albedo = &planet.texture(1).expect("第 1 格").texture;
        let (texture, _view) = upload_texture(&gpu.device, &gpu.queue, albedo);
        assert_eq!(texture.width(), 780);
        assert_eq!(texture.height(), 520);
        assert_eq!(texture.mip_level_count(), 10);
        assert_eq!(texture.format(), wgpu::TextureFormat::Rgba8UnormSrgb);

        let stars = &scene.skybox.as_ref().expect("天空盒").texture;
        let (cube, _cube_view) = upload_texture(&gpu.device, &gpu.queue, stars);
        assert_eq!(cube.depth_or_array_layers(), 6);
        assert_eq!(cube.mip_level_count(), 1);

        let sampler = planet.texture(1).expect("第 1 格").sampler;
        assert_eq!(address_mode(sampler.address_u), wgpu::AddressMode::Repeat);
        assert_eq!(
            address_mode(sampler.address_v),
            wgpu::AddressMode::ClampToEdge,
            "这一档的 v 轴是夹边（w 轴也用它）"
        );
        assert_eq!(filter_mode(sampler.filter), wgpu::FilterMode::Linear);
        assert_eq!(sampler.anisotropy.clamp(1, 16), 8);

        let tiny = LoadedTexture {
            member: px_protocol::scene::Member::new("test", "half", &"0".repeat(64)),
            shape: TextureShape {
                width: 1,
                height: 1,
                layers: 1,
                levels: 1,
                format: TextureFormat::Rgba16Float,
            },
            bytes: vec![0_u8; 8],
            image: image::RgbaImage::new(1, 1),
        };
        let (half, _) = upload_texture(&gpu.device, &gpu.queue, &tiny);
        assert_eq!(half.format(), wgpu::TextureFormat::Rgba16Float);
        assert_eq!(half.width(), 1);
    }
}
