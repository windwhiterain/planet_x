use std::collections::HashMap;

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BufferBinding, BufferBindingType,
    BufferUsages, ColorTargetState, ColorWrites, CommandEncoder, Device,
    Extent3d, FragmentState, LoadOp, MultisampleState, Operations, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PrimitiveState, RenderPassColorAttachment, RenderPassDescriptor,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, Texture, TextureDescriptor,
    TextureDimension as GpuDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
    TextureViewDescriptor, TextureViewDimension, VertexState,
};

pub const FRAGMENT_ENTRY: &str = "fs_main";
pub const VERTEX_ENTRY: &str = "px_fullscreen_vertex";

const FULLSCREEN_VERTEX: &str = r#"
struct PxFullscreenOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn px_fullscreen_vertex(@builtin(vertex_index) index: u32) -> PxFullscreenOut {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: PxFullscreenOut;
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    return out;
}
"#;

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Rgba8UnormSrgb,
    Rgba16Float,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Rgba8UnormSrgb => "rgba8unorm-srgb",
            Format::Rgba16Float => "rgba16float",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "rgba8unorm-srgb" => Ok(Format::Rgba8UnormSrgb),
            "rgba16float" => Ok(Format::Rgba16Float),
            other => Err(format!(
                "不认识的资源格式 '{other}'：这一版认 'rgba8unorm-srgb' 与 'rgba16float'"
            )),
        }
    }

    pub fn to_wgpu(self) -> TextureFormat {
        match self {
            Format::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
            Format::Rgba16Float => TextureFormat::Rgba16Float,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeRule {
    View,
    Half,
    Fixed(u32, u32),
}

impl SizeRule {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "view" => Ok(SizeRule::View),
            "half" => Ok(SizeRule::Half),
            other => {
                let (width, height) = other.split_once('x').ok_or_else(|| {
                    format!("不认识的尺寸规则 '{other}'：认 'view' / 'half' / '<宽>x<高>'")
                })?;
                let width = width
                    .trim()
                    .parse::<u32>()
                    .map_err(|err| format!("尺寸规则 '{other}' 的宽读不出来：{err}"))?;
                let height = height
                    .trim()
                    .parse::<u32>()
                    .map_err(|err| format!("尺寸规则 '{other}' 的高读不出来：{err}"))?;
                if width == 0 || height == 0 {
                    return Err(format!("尺寸规则 '{other}' 里有 0"));
                }
                Ok(SizeRule::Fixed(width, height))
            }
        }
    }

    pub fn resolve(self, width: u32, height: u32) -> (u32, u32) {
        match self {
            SizeRule::View => (width.max(1), height.max(1)),
            SizeRule::Half => (width.div_ceil(2).max(1), height.div_ceil(2).max(1)),
            SizeRule::Fixed(width, height) => (width, height),
        }
    }

    pub fn name(self) -> String {
        match self {
            SizeRule::View => "view".to_string(),
            SizeRule::Half => "half".to_string(),
            SizeRule::Fixed(width, height) => format!("{width}x{height}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Use {
    RenderAttachment,
    TextureBinding,
}

impl Use {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "render_attachment" => Ok(Use::RenderAttachment),
            "texture_binding" => Ok(Use::TextureBinding),
            other => Err(format!(
                "不认识的用途 '{other}'：这一版认 'render_attachment' 与 'texture_binding'\
                 （storage_texture / storage_buffer 是 compute 那一档，还没接）"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Use::RenderAttachment => "render_attachment",
            Use::TextureBinding => "texture_binding",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSpec {
    pub name: String,
    pub format: Format,
    pub size: SizeRule,
    pub usage: Vec<Use>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    Fullscreen,
    Compute,
}

// ---------------------------------------------------------------------------
// 绑定布局是**宿主给的**（§79 的 C 案）
//
// 执行器不认识任何内建名字：它不知道"材质"、不知道 `view`，也不知道哪一格是参数块 ——
// 这些都由宿主从**契约**（`px_protocol::material`）读出来之后填进来。
// 于是 pass 与材质共用同一张表，而这张表在本 crate 里一个字都没有抄。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    D2,
    Cube,
}

impl Dimension {
    pub fn name(self) -> &'static str {
        match self {
            Dimension::D2 => "texture_2d",
            Dimension::Cube => "texture_cube",
        }
    }

    fn view_dimension(self) -> TextureViewDimension {
        match self {
            Dimension::D2 => TextureViewDimension::D2,
            Dimension::Cube => TextureViewDimension::Cube,
        }
    }

    fn layers(self) -> u32 {
        match self {
            Dimension::D2 => 1,
            Dimension::Cube => 6,
        }
    }
}

/// 一格贴图：绑定下标 + 维度。采样器永远在 `binding + 1`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot {
    pub binding: u32,
    pub dimension: Dimension,
}

/// 执行器的绑定组形状。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Layout {
    /// 绑定组在第几组（材质的契约里是 `MATERIAL_BIND_GROUP`）。
    pub group: u32,
    /// 参数块占这一组的第几格（材质的契约里是 0）。
    pub params_binding: u32,
    /// 参数块的字节数按它对齐（材质的契约里是 16）。
    pub params_align: u32,
    /// 贴图能落在哪几格。
    pub slots: Vec<Slot>,
}

impl Layout {
    pub fn slot(&self, binding: u32) -> Option<&Slot> {
        self.slots.iter().find(|slot| slot.binding == binding)
    }

    fn key(&self) -> String {
        let slots = self
            .slots
            .iter()
            .map(|slot| format!("{}:{}", slot.binding, slot.dimension.name()))
            .collect::<Vec<_>>()
            .join(",");
        format!("{}|{}|{slots}", self.group, self.params_binding)
    }
}

#[derive(Debug, Clone)]
pub struct PassPlan {
    pub kind: PassKind,
    pub label: String,
    pub shader: String,
    pub entry: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    /// 参数块的字节：宿主按**这份 shader 自己声明的结构体**打好了（与材质同一条路）。
    pub params: Vec<u8>,
    /// `reads[k]` 落在哪一格（`layout.slots` 里的 `binding`）。
    pub slots: Vec<u32>,
}

impl PassPlan {
    pub fn target(&self) -> Option<&str> {
        self.writes.first().map(String::as_str)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// 全部 pass 共用一份布局（固定超集：空着的格绑兜底贴图）。
    pub layout: Layout,
    pub resources: Vec<ResourceSpec>,
    pub passes: Vec<PassPlan>,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            group: 0,
            params_binding: 0,
            params_align: 16,
            slots: Vec::new(),
        }
    }
}

impl Plan {
    pub fn resource(&self, name: &str) -> Option<&ResourceSpec> {
        self.resources.iter().find(|resource| resource.name == name)
    }

    pub fn is_empty(&self) -> bool {
        self.passes.is_empty()
    }

    fn name_list(&self) -> String {
        if self.resources.is_empty() {
            "（一个都没声明）".to_string()
        } else {
            self.resources
                .iter()
                .map(|resource| resource.name.clone())
                .collect::<Vec<_>>()
                .join(" / ")
        }
    }

    fn layout_faults(&self) -> Result<(), String> {
        let layout = &self.layout;
        if layout.params_align == 0 {
            return Err("布局的参数块对齐是 0".to_string());
        }
        let mut seen: Vec<u32> = Vec::new();
        for slot in &layout.slots {
            if seen.contains(&slot.binding) {
                return Err(format!("布局里第 {} 格出现了两次", slot.binding));
            }
            if slot.binding == layout.params_binding {
                return Err(format!(
                    "布局第 {} 格既是参数块又是贴图",
                    slot.binding
                ));
            }
            seen.push(slot.binding);
            seen.push(slot.binding + 1);
        }
        Ok(())
    }

    pub fn check(&self) -> Result<(), String> {
        self.layout_faults()?;
        let align = self.layout.params_align as usize;

        let mut names: Vec<&str> = Vec::new();
        for resource in &self.resources {
            if resource.name.is_empty() {
                return Err("有个资源没给名字".to_string());
            }
            if names.contains(&resource.name.as_str()) {
                return Err(format!("资源名重了：'{}'", resource.name));
            }
            if resource.usage.is_empty() {
                return Err(format!(
                    "资源 '{}' 的 usage 是空的：读它还是写它，得说出来",
                    resource.name
                ));
            }
            names.push(&resource.name);
        }

        for (index, pass) in self.passes.iter().enumerate() {
            let at = format!("第 {index} 条 pass '{}'", pass.label);
            if pass.kind == PassKind::Compute {
                return Err(format!(
                    "{at} 的 kind 是 compute：这一版执行器只有 fullscreen。\
                     声明了执行器不兑现的东西就当场拒 —— 静默跳过正是要避免的那种故障"
                ));
            }
            if pass.shader.trim().is_empty() {
                return Err(format!("{at} 的 shader 是空的"));
            }
            if pass.entry.is_empty() {
                return Err(format!("{at} 没给入口点名字"));
            }
            if pass.params.is_empty() || pass.params.len() % align != 0 {
                return Err(format!(
                    "{at} 的参数块是 {} 字节：布局要求它是 {align} 的正数倍",
                    pass.params.len()
                ));
            }
            if pass.slots.len() != pass.reads.len() {
                return Err(format!(
                    "{at} 给了 {} 个格位、{} 个 reads：一条 read 一个格，不能多也不能少",
                    pass.slots.len(),
                    pass.reads.len()
                ));
            }
            for binding in &pass.slots {
                if self.layout.slot(*binding).is_none() {
                    return Err(format!(
                        "{at} 的某一格是 {binding}，而布局里的贴图格只有：[{}]",
                        self.layout
                            .slots
                            .iter()
                            .map(|slot| slot.binding.to_string())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    ));
                }
            }
            let target = pass.target().ok_or_else(|| {
                format!("{at} 没有 writes：它不写任何东西，画了也没人看得见")
            })?;
            if pass.writes.len() > 1 {
                return Err(format!(
                    "{at} 写了 {} 个目标：这一版一条 pass 只画一个颜色附件",
                    pass.writes.len()
                ));
            }
            if let Some(resource) = self.resource(target) {
                if !resource.usage.contains(&Use::RenderAttachment) {
                    return Err(format!(
                        "{at} 写到 '{}'，而那个资源的 usage 里没有 render_attachment",
                        resource.name
                    ));
                }
            }
            if self.resource(target).is_some() && pass.reads.iter().any(|read| read == target) {
                return Err(format!(
                    "{at} 同时读和写 '{target}'：文档把它声明成了一个资源（执行器只给它一张纹理）"
                ));
            }
            for read in &pass.reads {
                if let Some(resource) = self.resource(read) {
                    if !resource.usage.contains(&Use::TextureBinding) {
                        return Err(format!(
                            "{at} 读 '{}'，而那个资源的 usage 里没有 texture_binding",
                            resource.name
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Read,
    Write,
}

pub struct External<'a> {
    pub name: &'a str,
    pub role: Role,
    pub view: &'a TextureView,
    pub format: TextureFormat,
}

pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub sets: &'a [Vec<External<'a>>],
}

struct Pooled {
    width: u32,
    height: u32,
    format: TextureFormat,
    view: TextureView,
}

#[derive(Default)]
pub struct Executor {
    pipelines: HashMap<String, RenderPipeline>,
    layouts: HashMap<Layout, BindGroupLayout>,
    sampler: Option<Sampler>,
    pool: HashMap<String, Pooled>,
    /// 没被 reads 占到的格一律绑它：布局是固定超集，shader 里声明了就一定绑得上。
    fallback: HashMap<Dimension, TextureView>,
}

impl Executor {
    pub fn new() -> Self {
        Self::default()
    }

    fn sampler(&mut self, device: &Device) -> Sampler {
        match &self.sampler {
            Some(sampler) => sampler.clone(),
            None => {
                let sampler = device.create_sampler(&SamplerDescriptor {
                    label: Some("px_pass_sampler"),
                    ..Default::default()
                });
                self.sampler = Some(sampler.clone());
                sampler
            }
        }
    }

    fn resource_view(
        &mut self,
        device: &Device,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
    ) -> TextureView {
        let format = resource.format.to_wgpu();
        if let Some(pooled) = self.pool.get(&resource.name) {
            if pooled.width == width && pooled.height == height && pooled.format == format {
                return pooled.view.clone();
            }
        }
        let usage = TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT;
        let texture: Texture = device.create_texture(&TextureDescriptor {
            label: Some(resource.name.as_str()),
            size: Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        self.pool.insert(
            resource.name.clone(),
            Pooled {
                width,
                height,
                format,
                view: view.clone(),
            },
        );
        view
    }

    /// 兜底贴图：1×1 白（cube 是 1×1×6）。
    ///
    /// 用**编码器**清成白色而不是建完就算：新纹理按规范是清零的，而"没给这一格 ⇒ 采到纯白"
    /// 才是与材质那一侧一致的语义（Bevy 的 `FallbackImage` 也是白的）。执行器拿不到 `Queue`，
    /// 所以白是拿一个清屏 pass 写进去的 —— 就在同一个编码器里，顺序天然正确。
    fn fallback(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        dimension: Dimension,
    ) -> TextureView {
        if let Some(view) = self.fallback.get(&dimension) {
            return view.clone();
        }
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("px_pass_fallback"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: dimension.layers(),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor {
            label: Some("px_pass_fallback_view"),
            dimension: Some(dimension.view_dimension()),
            ..Default::default()
        });
        for layer in 0..dimension.layers() {
            let layer_view = texture.create_view(&TextureViewDescriptor {
                label: Some("px_pass_fallback_layer"),
                dimension: Some(TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let descriptor = RenderPassDescriptor {
                label: Some("px_pass_fallback_clear"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &layer_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::WHITE),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            };
            let _ = encoder.begin_render_pass(&descriptor);
        }
        self.fallback.insert(dimension, view.clone());
        view
    }

    fn layout(&mut self, device: &Device, layout: &Layout) -> BindGroupLayout {
        if let Some(cached) = self.layouts.get(layout) {
            return cached.clone();
        }
        let mut entries: Vec<BindGroupLayoutEntry> = vec![BindGroupLayoutEntry {
            binding: layout.params_binding,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                // 每个 shader 的结构体大小不同，而布局只有一份：真实大小由各自的缓冲决定。
                min_binding_size: None,
            },
            count: None,
        }];
        for slot in &layout.slots {
            entries.push(BindGroupLayoutEntry {
                binding: slot.binding,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: slot.dimension.view_dimension(),
                    multisampled: false,
                },
                count: None,
            });
            entries.push(BindGroupLayoutEntry {
                binding: slot.binding + 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            });
        }
        let built = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass_bind_group_layout"),
            entries: &entries,
        });
        self.layouts.insert(layout.clone(), built.clone());
        built
    }

    #[allow(clippy::too_many_arguments)]
    fn bind_group(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        layout: &Layout,
        sampler: &Sampler,
        params: &[u8],
        bound: &[(u32, TextureView)],
    ) -> BindGroup {
        let group_layout = self.layout(device, layout);
        let params_buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass_params"),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            contents: params,
        });
        // 视图先全收进一个表里：`entries` 借的是它们，而它们必须活到 `create_bind_group` 之后。
        let mut views: Vec<TextureView> = Vec::with_capacity(layout.slots.len());
        for slot in &layout.slots {
            let view = bound
                .iter()
                .find(|(binding, _)| *binding == slot.binding)
                .map(|(_, view)| view.clone())
                .unwrap_or_else(|| self.fallback(device, encoder, slot.dimension));
            views.push(view);
        }
        let mut entries: Vec<BindGroupEntry> = vec![BindGroupEntry {
            binding: layout.params_binding,
            resource: BindingResource::Buffer(BufferBinding {
                buffer: &params_buffer,
                offset: 0,
                size: None,
            }),
        }];
        for (slot, view) in layout.slots.iter().zip(views.iter()) {
            entries.push(BindGroupEntry {
                binding: slot.binding,
                resource: BindingResource::TextureView(view),
            });
            entries.push(BindGroupEntry {
                binding: slot.binding + 1,
                resource: BindingResource::Sampler(sampler),
            });
        }
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("px_pass_bind_group"),
            layout: &group_layout,
            entries: &entries,
        })
    }

    fn pipeline(
        &mut self,
        device: &Device,
        layout: &Layout,
        pass: &PassPlan,
        format: TextureFormat,
    ) -> RenderPipeline {
        let key = format!(
            "{:016x}|{format:?}|{}|{}|{}",
            fnv1a(pass.shader.as_bytes()),
            pass.entry,
            pass.reads.join(","),
            layout.key()
        );
        if let Some(pipeline) = self.pipelines.get(&key) {
            return pipeline.clone();
        }
        let label = format!("px_pass {}", pass.label);
        let vertex = device.create_shader_module(ShaderModuleDescriptor {
            label: Some("px_pass_fullscreen_vertex"),
            source: ShaderSource::Wgsl(FULLSCREEN_VERTEX.into()),
        });
        let fragment = device.create_shader_module(ShaderModuleDescriptor {
            label: Some(label.as_str()),
            source: ShaderSource::Wgsl(pass.shader.as_str().into()),
        });
        let group_layout = self.layout(device, layout);
        let mut groups: Vec<Option<&BindGroupLayout>> = vec![None; layout.group as usize + 1];
        groups[layout.group as usize] = Some(&group_layout);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("px_pass_pipeline_layout"),
            bind_group_layouts: &groups,
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label.as_str()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &vertex,
                entry_point: Some(VERTEX_ENTRY),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &fragment,
                entry_point: Some(pass.entry.as_str()),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline.clone());
        pipeline
    }

    pub fn execute(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        plan: &Plan,
        frame: &Frame<'_>,
    ) -> Result<String, String> {
        plan.check()?;
        if plan.is_empty() {
            return Ok("pass 表是空的：这一帧没有任何 pass".to_string());
        }
        let layout = plan.layout.clone();
        let sampler = self.sampler(device);
        let mut audit: Vec<String> = Vec::new();

        for (index, pass) in plan.passes.iter().enumerate() {
            let target = pass
                .target()
                .ok_or_else(|| format!("第 {index} 条 pass '{}' 没有 writes", pass.label))?
                .to_string();
            let (destination, format) =
                self.resolve(device, plan, frame, index, &pass.label, &target, Role::Write)?;

            let mut bound: Vec<(u32, TextureView)> = Vec::new();
            for (read, binding) in pass.reads.iter().zip(pass.slots.iter()) {
                let (view, _) =
                    self.resolve(device, plan, frame, index, &pass.label, read, Role::Read)?;
                bound.push((*binding, view));
            }
            if bound
                .iter()
                .any(|(_, view)| view == &destination)
            {
                return Err(format!(
                    "pass '{}' 的读目标与写目标是**同一个视图**：宿主给这一条 pass 的这两样\
                     指到了同一张纹理（要读写同一张就得让宿主分开给，例如 ping-pong 的两张）",
                    pass.label
                ));
            }

            let pipeline = self.pipeline(device, &layout, pass, format);
            let bind_group = self.bind_group(
                device,
                encoder,
                &layout,
                &sampler,
                &pass.params,
                &bound,
            );
            let pass_label = format!("px_pass {}", pass.label);
            let descriptor = RenderPassDescriptor {
                label: Some(pass_label.as_str()),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &destination,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            };
            let mut render_pass = encoder.begin_render_pass(&descriptor);
            render_pass.set_pipeline(&pipeline);
            render_pass.set_bind_group(layout.group, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
            drop(render_pass);

            audit.push(format!(
                "pass {index} '{}' 读 [{}] 写 '{target}'（{format:?}）｜参数 {} 字节｜格 {}",
                pass.label,
                pass.reads.join(" / "),
                pass.params.len(),
                pass.slots
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        Ok(audit.join("\n"))
    }

    fn resolve(
        &mut self,
        device: &Device,
        plan: &Plan,
        frame: &Frame<'_>,
        index: usize,
        label: &str,
        name: &str,
        role: Role,
    ) -> Result<(TextureView, TextureFormat), String> {
        let role_name = match role {
            Role::Read => "读",
            Role::Write => "写",
        };
        let resource = plan.resource(name);
        let external = frame.sets.get(index).and_then(|set| {
            set.iter()
                .find(|external| external.name == name && external.role == role)
        });
        match (resource, external) {
            (Some(_), Some(_)) => Err(format!(
                "pass '{label}' {role_name} '{name}'：文档声明了这个资源，宿主又给了同名的外部目标\
                 —— 说不清该用哪一个"
            )),
            (Some(resource), None) => {
                let (width, height) = resource.size.resolve(frame.width, frame.height);
                let format = resource.format.to_wgpu();
                Ok((self.resource_view(device, resource, width, height), format))
            }
            (None, Some(external)) => Ok((external.view.clone(), external.format)),
            (None, None) => Err(format!(
                "pass '{index}'（'{label}'）{role_name} '{name}'：文档没声明这个资源（声明了的：{}），\
                 宿主这一帧也没给这个名字的{role_name}目标（这一条 pass 宿主给了：{}）",
                plan.name_list(),
                match frame.sets.get(index) {
                    Some(set) if !set.is_empty() => set
                        .iter()
                        .map(|external| format!(
                            "{}{}",
                            external.name,
                            match external.role {
                                Role::Read => "（读）",
                                Role::Write => "（写）",
                            }
                        ))
                        .collect::<Vec<_>>()
                        .join(" / "),
                    _ => "（一个都没有）".to_string(),
                }
            )),
        }
    }
}
