//! See docs/renderer.md
use std::collections::HashMap;

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferBinding,
    BufferBindingType, BufferUsages, ColorTargetState, ColorWrites, CommandEncoder, Device,
    Extent3d, FragmentState, IndexFormat, LoadOp, MultisampleState, Operations, Origin3d,
    PipelineCompilationOptions, PipelineLayoutDescriptor, PrimitiveState,
    RenderPassColorAttachment, RenderPassDepthStencilAttachment, RenderPassDescriptor,
    RenderPipeline, RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, StoreOp, TexelCopyTextureInfo, Texture,
    TextureAspect, TextureDescriptor, TextureDimension as GpuDimension, TextureFormat,
    TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension,
    VertexBufferLayout, VertexState,
};

pub const FRAGMENT_ENTRY: &str = "fs_main";
pub const VERTEX_ENTRY: &str = "px_fullscreen_vertex";

fn module_of_wgsl(device: &Device, label: &str, source: &str) -> wgpu::ShaderModule {
    let descriptor = ShaderModuleDescriptor {
        label: Some(label),
        source: ShaderSource::Wgsl(source.into()),
    };
    unsafe {
        device.create_shader_module_trusted(descriptor, wgpu::ShaderRuntimeChecks::unchecked())
    }
}

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

fn entry_points_of<'a>(
    at: &str,
    source: &str,
    stage: wgpu::naga::ShaderStage,
) -> Result<Vec<String>, String> {
    let module = wgpu::naga::front::wgsl::parse_str(source)
        .map_err(|err| format!("{at} 的 WGSL 解析不过：{}", err.emit_to_string(source)))?;
    Ok(module
        .entry_points
        .iter()
        .filter(|point| point.stage == stage)
        .map(|point| point.name.clone())
        .collect())
}

fn require_entry(
    at: &str,
    what: &str,
    entry: &str,
    entries: &[String],
    stage: &str,
) -> Result<(), String> {
    if entries.iter().any(|name| name == entry) {
        return Ok(());
    }
    let stage_name = match stage {
        "fragment" => "片段",
        other => other,
    };
    Err(format!(
        "{at} 的 {what} 里没有 @{stage} 入口 '{entry}'；它有的{stage_name}入口：{}",
        if entries.is_empty() {
            "（一个都没有）".to_string()
        } else {
            entries.join(" / ")
        }
    ))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn name_list<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    if names.is_empty() {
        "（一个都没有）".to_string()
    } else {
        names.join(" / ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Rgba8UnormSrgb,
    Rgba16Float,
    Depth32Float,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Rgba8UnormSrgb => "rgba8unorm-srgb",
            Format::Rgba16Float => "rgba16float",
            Format::Depth32Float => "depth32float",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "rgba8unorm-srgb" => Ok(Format::Rgba8UnormSrgb),
            "rgba16float" => Ok(Format::Rgba16Float),
            "depth32float" => Ok(Format::Depth32Float),
            other => Err(format!(
                "不认识的资源格式 '{other}'：这一版认 'rgba8unorm-srgb' 与 'rgba16float'\
                 与 'depth32float'"
            )),
        }
    }

    pub fn to_wgpu(self) -> TextureFormat {
        match self {
            Format::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
            Format::Rgba16Float => TextureFormat::Rgba16Float,
            Format::Depth32Float => TextureFormat::Depth32Float,
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
    CopySrc,
    CopyDst,
}

impl Use {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "render_attachment" => Ok(Use::RenderAttachment),
            "texture_binding" => Ok(Use::TextureBinding),
            "copy_src" => Ok(Use::CopySrc),
            "copy_dst" => Ok(Use::CopyDst),
            other => Err(format!(
                "不认识的用途 '{other}'：这一版认 'render_attachment' 与 'texture_binding'\
                 与 'copy_src' 与 'copy_dst'\
                 （storage_texture / storage_buffer 是 compute 那一档，还没接）"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Use::RenderAttachment => "render_attachment",
            Use::TextureBinding => "texture_binding",
            Use::CopySrc => "copy_src",
            Use::CopyDst => "copy_dst",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceSpec {
    pub name: String,
    pub format: Format,
    pub size: SizeRule,
    pub layers: u32,
    pub usage: Vec<Use>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassKind {
    #[default]
    Fullscreen,
    Geometry,
    Copy,
    Compute,
}

impl PassKind {
    pub fn name(self) -> &'static str {
        match self {
            PassKind::Fullscreen => "fullscreen",
            PassKind::Geometry => "geometry",
            PassKind::Copy => "copy",
            PassKind::Compute => "compute",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "fullscreen" => Ok(PassKind::Fullscreen),
            "geometry" => Ok(PassKind::Geometry),
            "copy" => Ok(PassKind::Copy),
            "compute" => Ok(PassKind::Compute),
            other => Err(format!(
                "不认识的 pass 类型 '{other}'：这一版认 'fullscreen' / 'geometry' / 'copy' / 'compute'"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Color {
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    pub fn name(self) -> String {
        format!("{},{},{},{}", self.r, self.g, self.b, self.a)
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        if parts.len() != 4 {
            return Err(format!(
                "颜色要四个数（r,g,b,a），实际给了 {} 个：'{text}'",
                parts.len()
            ));
        }
        let mut channels = [0.0_f64; 4];
        for (index, part) in parts.iter().enumerate() {
            channels[index] = part.parse::<f64>().map_err(|_| {
                format!(
                    "颜色的第 {} 个通道不是数：'{part}'（整串 '{text}'）",
                    index + 1
                )
            })?;
        }
        Ok(Color::new(
            channels[0],
            channels[1],
            channels[2],
            channels[3],
        ))
    }

    pub fn to_wgpu(self) -> wgpu::Color {
        wgpu::Color {
            r: self.r,
            g: self.g,
            b: self.b,
            a: self.a,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Compare {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    #[default]
    GreaterEqual,
    Always,
}

impl Compare {
    pub fn name(self) -> &'static str {
        match self {
            Compare::Never => "never",
            Compare::Less => "less",
            Compare::Equal => "equal",
            Compare::LessEqual => "less_equal",
            Compare::Greater => "greater",
            Compare::NotEqual => "not_equal",
            Compare::GreaterEqual => "greater_equal",
            Compare::Always => "always",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "never" => Ok(Compare::Never),
            "less" => Ok(Compare::Less),
            "equal" => Ok(Compare::Equal),
            "less_equal" => Ok(Compare::LessEqual),
            "greater" => Ok(Compare::Greater),
            "not_equal" => Ok(Compare::NotEqual),
            "greater_equal" => Ok(Compare::GreaterEqual),
            "always" => Ok(Compare::Always),
            other => Err(format!(
                "不认识的深度比较 '{other}'：这一版认 never / less / equal / less_equal / \
                 greater / not_equal / greater_equal / always"
            )),
        }
    }

    pub fn to_wgpu(self) -> wgpu::CompareFunction {
        match self {
            Compare::Never => wgpu::CompareFunction::Never,
            Compare::Less => wgpu::CompareFunction::Less,
            Compare::Equal => wgpu::CompareFunction::Equal,
            Compare::LessEqual => wgpu::CompareFunction::LessEqual,
            Compare::Greater => wgpu::CompareFunction::Greater,
            Compare::NotEqual => wgpu::CompareFunction::NotEqual,
            Compare::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
            Compare::Always => wgpu::CompareFunction::Always,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Cull {
    #[default]
    None,
    Front,
    Back,
}

impl Cull {
    pub fn name(self) -> &'static str {
        match self {
            Cull::None => "none",
            Cull::Front => "front",
            Cull::Back => "back",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "none" => Ok(Cull::None),
            "front" => Ok(Cull::Front),
            "back" => Ok(Cull::Back),
            other => Err(format!(
                "不认识的剔除档 '{other}'：这一版认 'none' / 'front' / 'back'"
            )),
        }
    }

    pub fn to_wgpu(self) -> Option<wgpu::Face> {
        match self {
            Cull::None => None,
            Cull::Front => Some(wgpu::Face::Front),
            Cull::Back => Some(wgpu::Face::Back),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Winding {
    #[default]
    Ccw,
    Cw,
}

impl Winding {
    pub fn name(self) -> &'static str {
        match self {
            Winding::Ccw => "ccw",
            Winding::Cw => "cw",
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "ccw" => Ok(Winding::Ccw),
            "cw" => Ok(Winding::Cw),
            other => Err(format!(
                "不认识的正面朝向 '{other}'：这一版认 'ccw' 与 'cw'"
            )),
        }
    }

    pub fn to_wgpu(self) -> wgpu::FrontFace {
        match self {
            Winding::Ccw => wgpu::FrontFace::Ccw,
            Winding::Cw => wgpu::FrontFace::Cw,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Attachment<T> {
    None,
    Clear(T),
    Load,
}

impl Attachment<Color> {
    pub fn name(&self) -> String {
        match self {
            Attachment::None => "none".to_string(),
            Attachment::Load => "load".to_string(),
            Attachment::Clear(color) => format!("clear({})", color.name()),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "none" => Ok(Attachment::None),
            "load" => Ok(Attachment::Load),
            other => match other
                .strip_prefix("clear(")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                Some(inner) => Ok(Attachment::Clear(Color::parse(inner)?)),
                None => Err(format!(
                    "认不出颜色附件 '{other}'：这一版认 'none' / 'load' / 'clear(r,g,b,a)'"
                )),
            },
        }
    }
}

impl Attachment<f32> {
    pub fn name(&self) -> String {
        match self {
            Attachment::None => "none".to_string(),
            Attachment::Load => "load".to_string(),
            Attachment::Clear(value) => format!("clear({value})"),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "none" => Ok(Attachment::None),
            "load" => Ok(Attachment::Load),
            other => match other
                .strip_prefix("clear(")
                .and_then(|rest| rest.strip_suffix(')'))
            {
                Some(inner) => {
                    Ok(Attachment::Clear(inner.trim().parse::<f32>().map_err(
                        |_| format!("深度值不是数：'{inner}'（整串 '{other}'）"),
                    )?))
                }
                None => Err(format!(
                    "认不出深度附件 '{other}'：这一版认 'none' / 'load' / 'clear(0.0)'"
                )),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderState {
    pub color: Attachment<Color>,
    pub depth: Attachment<f32>,
    pub depth_write: bool,
    pub frag_depth: bool,
    pub compare: Compare,
    pub winding: Winding,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            color: Attachment::Clear(Color::TRANSPARENT),
            depth: Attachment::None,
            depth_write: true,
            frag_depth: false,
            compare: Compare::GreaterEqual,
            winding: Winding::Ccw,
        }
    }
}

const STATE_KEYS: [&str; 5] = ["color", "depth", "depth_write", "compare", "winding"];
const OPTIONAL_KEYS: [&str; 1] = ["frag_depth"];

impl RenderState {
    pub fn name(&self) -> String {
        format!(
            "color={}|depth={}|depth_write={}|{}compare={}|winding={}",
            self.color.name(),
            self.depth.name(),
            self.depth_write,
            if self.frag_depth {
                "frag_depth=true|"
            } else {
                ""
            },
            self.compare.name(),
            self.winding.name()
        )
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let mut state = RenderState::default();
        let mut seen: Vec<&str> = Vec::new();
        for part in text.split('|') {
            let (key, value) = part.split_once('=').ok_or_else(|| {
                format!("状态里的 '{part}' 不是 key=value 的形状（整串 '{text}'）")
            })?;
            let key = key.trim();
            let value = value.trim();
            if !STATE_KEYS.contains(&key) && !OPTIONAL_KEYS.contains(&key) {
                return Err(format!(
                    "认不出的状态键 '{key}'：这一版认 {}（整串 '{text}'）",
                    STATE_KEYS.join(" / ")
                ));
            }
            if seen.contains(&key) {
                return Err(format!("状态键 '{key}' 出现了两次（整串 '{text}'）"));
            }
            seen.push(key);
            match key {
                "color" => state.color = Attachment::<Color>::parse(value)?,
                "depth" => state.depth = Attachment::<f32>::parse(value)?,
                "frag_depth" => {
                    state.frag_depth = match value {
                        "true" => true,
                        "false" => false,
                        other => {
                            return Err(format!(
                                "frag_depth 只认 'true' 与 'false'，实际是 '{other}'"
                            ));
                        }
                    };
                }
                "depth_write" => {
                    state.depth_write = match value {
                        "true" => true,
                        "false" => false,
                        other => {
                            return Err(format!(
                                "depth_write 只认 'true' 与 'false'，实际是 '{other}'"
                            ));
                        }
                    }
                }
                "compare" => state.compare = Compare::parse(value)?,
                "winding" => state.winding = Winding::parse(value)?,
                _ => unreachable!("STATE_KEYS 与上面这几支必须一一对应"),
            }
        }
        for key in STATE_KEYS {
            if !seen.contains(&key) {
                return Err(format!(
                    "状态里缺 '{key}'（整串 '{text}'）：六格都要写出来，\
                     缺一格就是'没说'，而'没说'与默认值长得一样"
                ));
            }
        }
        Ok(state)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    D2,
    Cube,
    D2Array,
}

impl Dimension {
    pub fn name(self) -> &'static str {
        match self {
            Dimension::D2 => "texture_2d",
            Dimension::Cube => "texture_cube",
            Dimension::D2Array => "texture_depth_2d_array",
        }
    }

    fn view_dimension(self) -> TextureViewDimension {
        match self {
            Dimension::D2 => TextureViewDimension::D2,
            Dimension::Cube => TextureViewDimension::Cube,
            Dimension::D2Array => TextureViewDimension::D2Array,
        }
    }

    fn layers(self) -> u32 {
        match self {
            Dimension::D2 => 1,
            Dimension::Cube => 6,
            Dimension::D2Array => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slot {
    pub binding: u32,
    pub dimension: Dimension,
    pub depth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Layout {
    pub group: u32,
    pub params_binding: u32,
    pub params_align: u32,
    pub slots: Vec<Slot>,
    pub geometry_group: u32,
    pub geometry_params_binding: u32,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draw {
    pub geometry: String,
    pub material: String,
}

#[derive(Debug, Clone, Default)]
pub struct PassPlan {
    pub kind: PassKind,
    pub label: String,
    pub shader: String,
    pub entry: String,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub texture_slots: Option<Vec<Slot>>,
    pub params: Vec<u8>,
    pub slots: Vec<u32>,
    pub render: RenderState,
    pub draws: Vec<Draw>,
    pub depth_target: Option<String>,
    pub layer: Option<u32>,
    pub vertex_shader: String,
    pub vertex_entry: String,
    pub viewport: Option<[f32; 4]>,
    pub params_offset: u32,
}

impl PassPlan {
    pub fn target(&self) -> Option<&str> {
        self.writes.first().map(String::as_str)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Plan {
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
            geometry_group: 0,
            geometry_params_binding: 0,
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
                return Err(format!("布局第 {} 格既是参数块又是贴图", slot.binding));
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
            if resource.layers == 0 {
                return Err(format!(
                    "资源 '{}' 的层数是 0：一份资源至少一层",
                    resource.name
                ));
            }
            names.push(&resource.name);
        }

        for (index, pass) in self.passes.iter().enumerate() {
            let at = format!("第 {index} 条 pass '{}'", pass.label);
            if pass.kind == PassKind::Copy {
                if pass.render.color != Attachment::None || pass.render.depth != Attachment::None {
                    return Err(format!(
                        "{at} 的 kind 是 copy，却挂了附件（color={} / depth={}）：\
                         拷贝不画附件，状态那一栏要写成『没有附件』那一种\
                         （`color=none|depth=none|…`）。往拷贝上挂附件是当场拒，不是被忽略的字段",
                        pass.render.color.name(),
                        pass.render.depth.name()
                    ));
                }
                if pass.reads.len() != 1 || pass.writes.len() != 1 {
                    return Err(format!(
                        "{at} 读 {} 张、写 {} 张：一条 copy 就是**一次搬运**，\
                         恰好一读一写（读了 [{}]／写了 [{}]）",
                        pass.reads.len(),
                        pass.writes.len(),
                        pass.reads.join(" / "),
                        pass.writes.join(" / ")
                    ));
                }
                if pass.reads[0] == pass.writes[0] {
                    return Err(format!(
                        "{at} 读写的都是 '{}'：一次搬运的两端不能是同一张图",
                        pass.reads[0]
                    ));
                }
                if !pass.draws.is_empty() {
                    return Err(format!(
                        "{at} 的 kind 是 copy，却声明了 {} 笔 draws：搬运不画东西，\
                         要画几何就该是一条 geometry pass",
                        pass.draws.len()
                    ));
                }
                if !pass.shader.trim().is_empty() || !pass.entry.trim().is_empty() {
                    return Err(format!(
                        "{at} 的 kind 是 copy，却给了 shader/entry：拷贝不建管线。\
                         这两栏是 fullscreen 的（片元阶段），而几何 pass 的片元阶段属于**材质**"
                    ));
                }
                if !pass.vertex_shader.trim().is_empty() || !pass.vertex_entry.trim().is_empty() {
                    return Err(format!(
                        "{at} 的 kind 是 copy，却给了 vertex_shader/vertex_entry：\
                         拷贝没有顶点阶段（它一个三角形都不画）"
                    ));
                }
                if !pass.params.is_empty() || !pass.slots.is_empty() {
                    return Err(format!(
                        "{at} 的 kind 是 copy，却给了参数块（{} 字节）/ 格位（{} 个）：\
                         拷贝没有绑定组（参数与格位是全屏 pass 那两栏）",
                        pass.params.len(),
                        pass.slots.len()
                    ));
                }
                let source_name = pass.reads[0].as_str();
                let target_name = pass.writes[0].as_str();
                let source = self.resource(source_name);
                let target = self.resource(target_name);
                if let Some(source) = source {
                    if !source.usage.contains(&Use::CopySrc) {
                        return Err(format!(
                            "{at} 读 '{source_name}'，而它的 usage 里没有 'copy_src'：\
                             拷贝的两端都要在**文档的 `resources` 那一栏**声明用途\
                             （源要 copy_src、目标要 copy_dst）。少了它这一条 pass 会一路\
                             过到执行期才失败，而那时的报错指向纹理创建、不指向这里"
                        ));
                    }
                    if !source.usage.contains(&Use::TextureBinding) {}
                }
                if let Some(target) = target {
                    if !target.usage.contains(&Use::CopyDst) {
                        return Err(format!(
                            "{at} 写 '{target_name}'，而它的 usage 里没有 'copy_dst'：\
                             拷贝的两端都要在**文档的 `resources` 那一栏**声明用途\
                             （源要 copy_src、目标要 copy_dst）"
                        ));
                    }
                }
                if let (Some(source), Some(target)) = (source, target) {
                    if source.format != target.format {
                        return Err(format!(
                            "{at} 把 '{source_name}'（{}）搬到 '{target_name}'（{}）：\
                             格式不同的两张图不能直接拷（这一版不做格式转换）",
                            source.format.name(),
                            target.format.name()
                        ));
                    }
                    if source.size.name() != target.size.name() {
                        return Err(format!(
                            "{at} 把 '{source_name}'（尺寸规则 {}）搬到 '{target_name}'（尺寸规则 {}）：\
                             规则不同的两张在别的帧尺寸下就可能不一样大，而尺寸不同的一对\
                             要按 min 裁着拷 —— 那是另一件事。这一版只搬**同规格**的两张\
                             （层数与 mip 都是 1，由池子的建法保证）",
                            source.size.name(),
                            target.size.name()
                        ));
                    }
                }
                continue;
            }
            if pass.kind == PassKind::Compute {
                if pass.render.color != Attachment::None {
                    return Err(format!(
                        "{at} 的 kind 是 compute，却挂了颜色附件：compute 不画附件，\
                         颜色/深度都不该挂给它"
                    ));
                }
                return Err(format!(
                    "{at} 的 kind 是 compute：这一版执行器只有 fullscreen 与 geometry。\
                     声明了执行器不兑现的东西就当场拒 —— 静默跳过正是要避免的那种故障"
                ));
            }
            if pass.render.color == Attachment::None && pass.render.depth == Attachment::None {
                return Err(format!(
                    "{at} 既不挂颜色也不挂深度：它画到哪儿去？一条 pass 至少要有一个附件"
                ));
            }
            if pass.kind == PassKind::Fullscreen
                && pass.render.color == Attachment::None
                && !pass.render.frag_depth
            {
                return Err(format!(
                    "{at} 的 kind 是 fullscreen，却没挂颜色附件：全屏三角只会写颜色，\
                     不挂颜色就等于什么都没做（要只写深度就该是一条 geometry pass）"
                ));
            }
            match (&pass.render.depth, &pass.depth_target) {
                (Attachment::None, Some(name)) => {
                    return Err(format!(
                        "{at} 没挂深度附件，却给了 depth_target '{name}'：那张图没人用"
                    ));
                }
                (Attachment::None, None) => {}
                (_, None) => {
                    return Err(format!(
                        "{at} 挂了深度附件，却没给 depth_target：深度图从哪来？\
                         （要么是 resources 里一份 depth32float，要么是宿主这一帧给的 Role::Depth）"
                    ));
                }
                (_, Some(name)) => {
                    if let Some(resource) = self.resource(name) {
                        if resource.format != Format::Depth32Float {
                            return Err(format!(
                                "{at} 的深度目标 '{name}' 格式是 {}：深度附件只能是 \
                                 depth32float（这个渲染器只有一种深度约定）",
                                resource.format.name()
                            ));
                        }
                        if !resource.usage.contains(&Use::RenderAttachment) {
                            return Err(format!(
                                "{at} 的深度目标 '{name}' 的 usage 里没有 render_attachment"
                            ));
                        }
                        match (pass.layer, resource.layers) {
                            (None, layers) if layers > 1 => {
                                return Err(format!(
                                    "{at} 的深度目标 '{name}' 有 {layers} 层，而这条 pass \
                                     没说写第几层：多层图上不分层就是一句说不清的话\
                                     （要整份一起写就该显式说清那是哪一种 pass）"
                                ));
                            }
                            (Some(layer), layers) if layer >= layers => {
                                return Err(format!(
                                    "{at} 要写 '{name}' 的第 {layer} 层，而那份资源只有 \
                                     {layers} 层"
                                ));
                            }
                            _ => {}
                        }
                    } else if pass.layer.is_some() {
                        return Err(format!(
                            "{at} 指定了写第 {} 层，而深度目标 '{name}' 不是 \
                             `resources` 里的资源（是宿主给的外部目标）：它有几层、\
                             分层视图怎么建都没有依据",
                            pass.layer.expect("上面判过是 Some")
                        ));
                    }
                }
            }
            if let Some(rect) = pass.viewport {
                if rect[2] <= 0.0 || rect[3] <= 0.0 {
                    return Err(format!(
                        "{at} 的 viewport 是 ({}, {}, {}, {})：宽高必须是正数\
                         （一个零宽的格子画不出东西，而画不出来与画错了在画面上分不开）",
                        rect[0], rect[1], rect[2], rect[3]
                    ));
                }
                if !rect.iter().all(|value| value.is_finite()) {
                    return Err(format!("{at} 的 viewport 里有不是有限数的值：{rect:?}"));
                }
                if pass.render.color == Attachment::None && pass.render.depth == Attachment::None {
                    return Err(format!(
                        "{at} 给了 viewport 却一个附件都没挂：那一块落在哪张图上？"
                    ));
                }
            }
            match pass.kind {
                PassKind::Compute => unreachable!("上面已经 return 了"),
                PassKind::Copy => unreachable!("copy 在上面那一支就 continue 了"),
                PassKind::Fullscreen => {
                    if !pass.draws.is_empty() {
                        return Err(format!(
                            "{at} 是 fullscreen，却声明了 {} 笔 draws：全屏三角的顶点由执行器\
                             自备，这两样说不清谁说了算（要画几何就该是一条 geometry pass）",
                            pass.draws.len()
                        ));
                    }
                    if pass.shader.trim().is_empty() {
                        return Err(format!("{at} 的 shader 是空的"));
                    }
                    if pass.entry.is_empty() {
                        return Err(format!("{at} 没给入口点名字"));
                    }
                    let entries = entry_points_of(
                        at.as_str(),
                        &pass.shader,
                        wgpu::naga::ShaderStage::Fragment,
                    )?;
                    require_entry(at.as_str(), "shader", &pass.entry, &entries, "fragment")?;
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
                }
                PassKind::Geometry => {
                    if pass.vertex_shader.trim().is_empty() {
                        return Err(format!(
                            "{at} 是 geometry，却没给 vertex_shader：几何 pass 的顶点阶段\
                             必须由宿主给（内容 shader 是纯片元的，没有 `@vertex`）"
                        ));
                    }
                    if pass.vertex_entry.is_empty() {
                        return Err(format!("{at} 没给 vertex_entry"));
                    }
                    let entries = entry_points_of(
                        at.as_str(),
                        &pass.vertex_shader,
                        wgpu::naga::ShaderStage::Vertex,
                    )?;
                    require_entry(
                        at.as_str(),
                        "顶点阶段",
                        &pass.vertex_entry,
                        &entries,
                        "vertex",
                    )?;
                    if !pass.shader.trim().is_empty() || !pass.entry.trim().is_empty() {
                        return Err(format!(
                            "{at} 是 geometry，却给了 shader/entry：片元阶段属于**材质**                             （每个物体一支），几何 pass 的这一栏没人用 ——                              把 shader 挪到材质的 `fragment_shader` 那一格去"
                        ));
                    }
                    if !pass.reads.is_empty() || !pass.slots.is_empty() {
                        return Err(format!(
                            "{at} 是 geometry，却给了 reads（{} 个）/ slots（{} 个）：\
                             几何 pass 的纹理绑定由宿主解析（材质名 → 组），执行器不自己造，\
                             这两栏没人用。⚠ `params` 不在这一条里 —— 几何 pass 的**参数块**\
                             由执行器造（虚拟影图的每一页要把自己的视图当参数传下去）",
                            pass.reads.len(),
                            pass.slots.len()
                        ));
                    }
                    let align = self.layout.params_align as usize;
                    if !pass.params.is_empty() && pass.params.len() % align != 0 {
                        return Err(format!(
                            "{at} 是 geometry，参数块是 {} 字节：布局要求它是 {align} 的正数倍",
                            pass.params.len()
                        ));
                    }
                }
            }
            for (order, draw) in pass.draws.iter().enumerate() {
                if draw.geometry.trim().is_empty() {
                    return Err(format!(
                        "{at} 的第 {} 笔没说用哪份几何（geometry 是空的）",
                        order + 1
                    ));
                }
            }
            if pass.writes.len() > 1 {
                return Err(format!(
                    "{at} 写了 {} 个目标：这一版一条 pass 只画一个颜色附件",
                    pass.writes.len()
                ));
            }
            if pass.render.color == Attachment::None {
                if !pass.writes.is_empty() {
                    return Err(format!(
                        "{at} 没挂颜色附件，却声明了写目标 [{}]：写目标就是颜色附件",
                        pass.writes.join(" / ")
                    ));
                }
                continue;
            }
            let target = pass
                .target()
                .ok_or_else(|| format!("{at} 没有 writes：它不写任何东西，画了也没人看得见"))?;
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
    Depth,
}

pub struct External<'a> {
    pub name: &'a str,
    pub role: Role,
    pub view: &'a TextureView,
    pub format: TextureFormat,
}

pub struct ResolvedGeometry<'a> {
    pub name: &'a str,
    pub vertices: Option<(&'a Buffer, VertexBufferLayout<'a>)>,
    pub indices: Option<(&'a Buffer, IndexFormat, u32)>,
    pub vertex_count: u32,
    pub instances: std::ops::Range<u32>,
}

#[derive(Clone)]
pub struct ResolvedGroup<'a> {
    pub group: u32,
    pub bind_group: &'a BindGroup,
    pub layout: BindGroupLayout,
    pub layout_id: u64,
    pub dynamic: bool,
    pub dynamic_offset: u32,
}

#[derive(Clone)]
pub struct ResolvedMaterial<'a> {
    pub name: &'a str,
    pub groups: Vec<ResolvedGroup<'a>>,
    pub blend: Option<BlendState>,
    pub cull: Cull,
    pub fragment_shader: &'a str,
    pub fragment_entry: &'a str,
}

pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub viewport: Option<[f32; 4]>,
    pub sets: &'a [Vec<External<'a>>],
    pub geometries: &'a [ResolvedGeometry<'a>],
    pub materials: &'a [ResolvedMaterial<'a>],
    pub zero_dummy: Option<(&'a BindGroupLayout, &'a BindGroup)>,
}

pub const TIMESTAMP_SLOTS_PER_PASS: u32 = 4;

pub const TIMESTAMP_FRAME_SLOTS: u32 = 2;

pub fn pass_slot_count(kind: PassKind) -> u32 {
    match kind {
        PassKind::Geometry | PassKind::Fullscreen => TIMESTAMP_SLOTS_PER_PASS,
        PassKind::Copy | PassKind::Compute => 2,
    }
}

pub fn frame_slots(kinds: &[PassKind]) -> (Vec<PassSlots>, u32) {
    let mut out = Vec::with_capacity(kinds.len());
    let mut cursor = 0_u32;
    for kind in kinds {
        let slots = match kind {
            PassKind::Geometry | PassKind::Fullscreen => PassSlots {
                envelope: (cursor, cursor + 3),
                inside: Some((cursor + 1, cursor + 2)),
            },
            PassKind::Copy | PassKind::Compute => PassSlots {
                envelope: (cursor, cursor + 1),
                inside: None,
            },
        };
        cursor += pass_slot_count(*kind);
        out.push(slots);
    }
    (out, cursor)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassSlots {
    pub envelope: (u32, u32),
    pub inside: Option<(u32, u32)>,
}

pub struct PassTimestamps<'a> {
    query_set: &'a wgpu::QuerySet,
    slots: &'a [PassSlots],
}

impl<'a> PassTimestamps<'a> {
    pub fn new(query_set: &'a wgpu::QuerySet, slots: &'a [PassSlots]) -> Self {
        PassTimestamps { query_set, slots }
    }

    pub fn slots(&self, index: usize) -> Option<PassSlots> {
        self.slots.get(index).copied()
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    fn pass_writes(
        &self,
        slots: PassSlots,
        calls: &mut u32,
    ) -> wgpu::RenderPassTimestampWrites<'a> {
        *calls += 2;
        wgpu::RenderPassTimestampWrites {
            query_set: self.query_set,
            beginning_of_pass_write_index: Some(slots.envelope.0),
            end_of_pass_write_index: Some(slots.envelope.1),
        }
    }
}

fn write_stamp(
    stamps: Option<&PassTimestamps<'_>>,
    encoder: &mut CommandEncoder,
    index: u32,
    calls: &mut u32,
) {
    if let Some(stamps) = stamps {
        encoder.write_timestamp(stamps.query_set, index);
        *calls += 1;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CellSpace {
    Whole,
    Viewport([f32; 4]),
    Scissor([f32; 4]),
}

fn cell_space_for(
    pass: &PassPlan,
    index: usize,
    plan: &Plan,
    cell: [f32; 4],
    external_write: bool,
) -> Result<CellSpace, String> {
    if external_write {
        if pass.kind != PassKind::Fullscreen {
            return Err(format!(
                "第 {index} 条 pass '{}' 是 {}，它写的是宿主这一帧的外部目标，而这一帧带**格子**\
                 （`Frame::viewport` = {:?}）：格子里的几何必须靠 viewport 落位，而写外部目标的那一条\
                 按 oracle 的形状只设 scissor（不设 viewport）⇒ 这条路走不通",
                pass.label,
                pass.kind.name(),
                cell
            ));
        }
        return Ok(CellSpace::Scissor(cell));
    }
    let view_sized = pass
        .target()
        .and_then(|name| plan.resource(name))
        .is_some_and(|resource| resource.size == SizeRule::View)
        || pass
            .depth_target
            .as_deref()
            .and_then(|name| plan.resource(name))
            .is_some_and(|resource| resource.size == SizeRule::View);
    Ok(if view_sized {
        CellSpace::Viewport(cell)
    } else {
        CellSpace::Whole
    })
}

fn cell_space(
    pass: &PassPlan,
    index: usize,
    plan: &Plan,
    frame: &Frame<'_>,
) -> Result<CellSpace, String> {
    let Some(cell) = frame.viewport else {
        return Ok(CellSpace::Whole);
    };
    let external_write = pass.target().is_some_and(|name| {
        frame.sets.get(index).is_some_and(|set| {
            set.iter()
                .any(|external| external.name == name && external.role == Role::Write)
        })
    });
    cell_space_for(pass, index, plan, cell, external_write)
}

struct Pooled {
    width: u32,
    height: u32,
    layers: u32,
    format: TextureFormat,
    usage: TextureUsages,
    view: TextureView,
    texture: Texture,
}

fn copy_aspect(format: TextureFormat) -> TextureAspect {
    if format.has_depth_aspect() {
        TextureAspect::DepthOnly
    } else {
        TextureAspect::All
    }
}

pub fn texture_usage(resource: &ResourceSpec) -> TextureUsages {
    let mut usage = TextureUsages::empty();
    for declared in &resource.usage {
        usage |= match declared {
            Use::RenderAttachment => TextureUsages::RENDER_ATTACHMENT,
            Use::TextureBinding => TextureUsages::TEXTURE_BINDING,
            Use::CopySrc => TextureUsages::COPY_SRC,
            Use::CopyDst => TextureUsages::COPY_DST,
        };
    }
    usage
}

#[derive(Default)]
pub struct Executor {
    pipelines: HashMap<String, RenderPipeline>,
    layouts: HashMap<Layout, BindGroupLayout>,
    sampler: Option<Sampler>,
    comparison_sampler: Option<Sampler>,
    pool: HashMap<String, Pooled>,
    fallback: HashMap<(Dimension, bool), TextureView>,
    seeded: Vec<String>,
}

impl Executor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn seed(
        &mut self,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
        texture: Texture,
    ) -> Result<(), String> {
        let format = resource.format.to_wgpu();
        let declared = texture_usage(resource);
        let mut problems: Vec<String> = Vec::new();
        if texture.format() != format {
            problems.push(format!(
                "格式是 {:?}，文档声明的是 {:?}",
                texture.format(),
                format
            ));
        }
        if texture.width() != width || texture.height() != height {
            problems.push(format!(
                "尺寸是 {}×{}，文档那一条尺寸规则（{}）在这一帧是 {}×{}",
                texture.width(),
                texture.height(),
                resource.size.name(),
                width,
                height
            ));
        }
        if texture.depth_or_array_layers() != resource.layers.max(1)
            || texture.mip_level_count() != 1
        {
            problems.push(format!(
                "有 {} 层 / {} 级 mip，而文档声明的是 {} 层 1 级",
                texture.depth_or_array_layers(),
                texture.mip_level_count(),
                resource.layers.max(1)
            ));
        }
        let missing = declared - texture.usage();
        if !missing.is_empty() {
            problems.push(format!(
                "用途少了 {:?}：文档给 '{}' 声明的是 [{}]，宿主建这张纹理时给的用法必须**涵盖**它们\
                 （少了 copy_src / copy_dst 那种，是校验全过、拷贝那一刻才炸）",
                missing,
                resource.name,
                resource
                    .usage
                    .iter()
                    .map(|use_| use_.name())
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        }
        if !problems.is_empty() {
            return Err(format!(
                "宿主 seed 的 '{}' 与文档声明对不上：{}",
                resource.name,
                problems.join("；")
            ));
        }
        let view = texture.create_view(&TextureViewDescriptor::default());
        self.pool.insert(
            resource.name.clone(),
            Pooled {
                width,
                height,
                layers: texture.depth_or_array_layers(),
                format,
                usage: declared,
                view,
                texture,
            },
        );
        if !self.seeded.contains(&resource.name) {
            self.seeded.push(resource.name.clone());
        }
        Ok(())
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

    fn comparison_sampler(&mut self, device: &Device) -> Sampler {
        match &self.comparison_sampler {
            Some(sampler) => sampler.clone(),
            None => {
                let sampler = device.create_sampler(&SamplerDescriptor {
                    label: Some("px_pass_comparison_sampler"),
                    compare: Some(wgpu::CompareFunction::GreaterEqual),
                    ..Default::default()
                });
                self.comparison_sampler = Some(sampler.clone());
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
        self.pooled(device, resource, width, height).view.clone()
    }

    fn layer_view(
        &mut self,
        device: &Device,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
        layer: u32,
    ) -> Result<TextureView, String> {
        let pooled = self.pooled(device, resource, width, height);
        if layer >= pooled.layers {
            let (name, layers) = (resource.name.clone(), pooled.layers);
            return Err(format!(
                "资源 '{name}' 只有 {layers} 层，而这条 pass 要写第 {layer} 层"
            ));
        }
        Ok(pooled.texture.create_view(&TextureViewDescriptor {
            label: Some("px_pass_layer_view"),
            format: None,
            dimension: Some(TextureViewDimension::D2),
            usage: None,
            aspect: TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: layer,
            array_layer_count: Some(1),
        }))
    }

    fn resource_texture(
        &mut self,
        device: &Device,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
    ) -> Texture {
        self.pooled(device, resource, width, height).texture.clone()
    }

    fn pooled(
        &mut self,
        device: &Device,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
    ) -> &Pooled {
        let format = resource.format.to_wgpu();
        let usage = texture_usage(resource);
        let layers = resource.layers.max(1);
        let stale = match self.pool.get(&resource.name) {
            Some(pooled) => {
                let reason = if pooled.width != width {
                    Some(format!("宽 {} → {width}", pooled.width))
                } else if pooled.height != height {
                    Some(format!("高 {} → {height}", pooled.height))
                } else if pooled.layers != layers {
                    Some(format!("层 {} → {layers}", pooled.layers))
                } else if pooled.format != format {
                    Some(format!("格式 {:?} → {format:?}", pooled.format))
                } else if pooled.usage != usage {
                    Some(format!("用途 {:?} → {usage:?}", pooled.usage))
                } else {
                    None
                };
                if let Some(reason) = &reason {
                    if std::env::var_os("PX_POOL_DEBUG").is_some() {
                        eprintln!("池子重建 '{}'：{reason}", resource.name);
                    }
                }
                reason.is_some()
            }
            None => true,
        };
        if stale {
            let texture: Texture = device.create_texture(&TextureDescriptor {
                label: Some(resource.name.as_str()),
                size: Extent3d {
                    width,
                    height,
                    depth_or_array_layers: layers,
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
                    layers,
                    format,
                    usage,
                    view,
                    texture,
                },
            );
        }
        self.pool.get(&resource.name).expect("刚插进去的")
    }

    fn fallback(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        dimension: Dimension,
        depth: bool,
    ) -> TextureView {
        if let Some(view) = self.fallback.get(&(dimension, depth)) {
            return view.clone();
        }
        let format = if depth {
            TextureFormat::Depth32Float
        } else {
            TextureFormat::Rgba8UnormSrgb
        };
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
            format,
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
            let descriptor = if depth {
                RenderPassDescriptor {
                    label: Some("px_pass_fallback_clear"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &layer_view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(0.0),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                }
            } else {
                RenderPassDescriptor {
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
                }
            };
            let _ = encoder.begin_render_pass(&descriptor);
        }
        self.fallback.insert((dimension, depth), view.clone());
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
                min_binding_size: None,
            },
            count: None,
        }];
        for slot in &layout.slots {
            let sample_type = if slot.depth {
                TextureSampleType::Depth
            } else {
                TextureSampleType::Float { filterable: true }
            };
            entries.push(BindGroupLayoutEntry {
                binding: slot.binding,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type,
                    view_dimension: slot.dimension.view_dimension(),
                    multisampled: false,
                },
                count: None,
            });
            entries.push(BindGroupLayoutEntry {
                binding: slot.binding + 1,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(if slot.depth {
                    SamplerBindingType::Comparison
                } else {
                    SamplerBindingType::Filtering
                }),
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
    fn geometry_params_group(
        &self,
        device: &Device,
        _group: u32,
        binding: u32,
        params: &[u8],
    ) -> BindGroup {
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass_geometry_params_layout"),
            entries: &[BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass_geometry_params"),
            usage: BufferUsages::UNIFORM,
            contents: params,
        });
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("px_pass_geometry_params"),
            layout: &layout,
            entries: &[BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

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
        let mut views: Vec<TextureView> = Vec::with_capacity(layout.slots.len());
        for slot in &layout.slots {
            let view = bound
                .iter()
                .find(|(binding, _)| *binding == slot.binding)
                .map(|(_, view)| view.clone())
                .unwrap_or_else(|| self.fallback(device, encoder, slot.dimension, slot.depth));
            views.push(view);
        }
        let mut samplers: Vec<Sampler> = Vec::with_capacity(layout.slots.len());
        for slot in &layout.slots {
            samplers.push(if slot.depth {
                self.comparison_sampler(device)
            } else {
                sampler.clone()
            });
        }
        let mut entries: Vec<BindGroupEntry> = vec![BindGroupEntry {
            binding: layout.params_binding,
            resource: BindingResource::Buffer(BufferBinding {
                buffer: &params_buffer,
                offset: 0,
                size: None,
            }),
        }];
        for ((slot, view), slot_sampler) in
            layout.slots.iter().zip(views.iter()).zip(samplers.iter())
        {
            entries.push(BindGroupEntry {
                binding: slot.binding,
                resource: BindingResource::TextureView(view),
            });
            entries.push(BindGroupEntry {
                binding: slot.binding + 1,
                resource: BindingResource::Sampler(slot_sampler),
            });
        }
        device.create_bind_group(&BindGroupDescriptor {
            label: Some("px_pass_bind_group"),
            layout: &group_layout,
            entries: &entries,
        })
    }

    fn states_key(render: &RenderState, blend: Option<BlendState>, cull: Cull) -> String {
        format!(
            "cull={:?}|winding={:?}|dw={}|compare={:?}|depth={}|blend={:?}",
            cull,
            render.winding,
            render.depth_write,
            render.compare,
            render.depth != Attachment::None,
            blend
        )
    }

    fn vertex_layout_key(layout: &VertexBufferLayout<'_>) -> String {
        let attributes = layout
            .attributes
            .iter()
            .map(|attribute| {
                format!(
                    "{}:{:?}:{}",
                    attribute.shader_location, attribute.format, attribute.offset
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "stride={}|step={:?}|[{attributes}]",
            layout.array_stride, layout.step_mode
        )
    }

    fn pipeline_fullscreen(
        &mut self,
        device: &Device,
        layout: &Layout,
        pass: &PassPlan,
        format: Option<TextureFormat>,
        zero_layout: Option<&BindGroupLayout>,
    ) -> RenderPipeline {
        let key = format!(
            "fullscreen|{:016x}|{format:?}|{}|{}|{}|{}",
            fnv1a(pass.shader.as_bytes()),
            pass.entry,
            pass.reads.join(","),
            layout.key(),
            Self::states_key(&pass.render, None, Cull::None)
        );
        if let Some(pipeline) = self.pipelines.get(&key) {
            return pipeline.clone();
        }
        let label = format!("px_pass {}", pass.label);
        let vertex = module_of_wgsl(device, "px_pass_fullscreen_vertex", FULLSCREEN_VERTEX);
        let fragment = module_of_wgsl(device, label.as_str(), pass.shader.as_str());
        let group_layout = self.layout(device, layout);
        let mut groups: Vec<Option<&BindGroupLayout>> = vec![None; layout.group as usize + 1];
        if let Some(zero) = zero_layout {
            groups.resize(groups.len().max(1), None);
            groups[0] = Some(zero);
        }
        groups[layout.group as usize] = Some(&group_layout);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("px_pass_pipeline_layout"),
            bind_group_layouts: &groups,
            immediate_size: 0,
        });
        let color_targets = [format.map(|format| ColorTargetState {
            format,
            blend: None,
            write_mask: ColorWrites::ALL,
        })];
        let targets: &[Option<ColorTargetState>] = if pass.render.frag_depth {
            &[]
        } else {
            &color_targets
        };
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label.as_str()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &vertex,
                entry_point: Some(VERTEX_ENTRY),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: PrimitiveState {
                cull_mode: None,
                front_face: pass.render.winding.to_wgpu(),
                ..Default::default()
            },
            depth_stencil: pass.render.frag_depth.then(|| wgpu::DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(pass.render.depth_write),
                depth_compare: Some(pass.render.compare.to_wgpu()),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: MultisampleState::default(),
            fragment: Some(FragmentState {
                module: &fragment,
                entry_point: Some(pass.entry.as_str()),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline.clone());
        pipeline
    }

    fn pipeline_geometry(
        &mut self,
        device: &Device,
        pass: &PassPlan,
        color_format: Option<TextureFormat>,
        geometry: &ResolvedGeometry<'_>,
        material: Option<&ResolvedMaterial<'_>>,
    ) -> Result<RenderPipeline, String> {
        let vertex_layout = geometry.vertices.as_ref().map(|(_, layout)| layout);
        let vertex_key = match vertex_layout {
            Some(layout) => Self::vertex_layout_key(layout),
            None => "procedural".to_string(),
        };
        let groups: Vec<(u32, BindGroupLayout)> = match material {
            Some(material) => material
                .groups
                .iter()
                .map(|group| (group.group, group.layout.clone()))
                .collect(),
            None => Vec::new(),
        };
        let blend = material.and_then(|material| material.blend);
        let cull = material.map(|material| material.cull).unwrap_or(Cull::None);
        let fragment = material
            .map(|material| (material.fragment_shader, material.fragment_entry))
            .filter(|(shader, _)| !shader.trim().is_empty())
            .filter(|_| color_format.is_some());
        let mut identities: Vec<String> = match material {
            Some(material) => material
                .groups
                .iter()
                .map(|group| format!("{}:{:016x}", group.group, group.layout_id))
                .collect(),
            None => Vec::new(),
        };
        identities.sort();
        let key = format!(
            "geometry|{:016x}|{:016x}|{}|{}|{vertex_key}|{color_format:?}|{}|{}",
            fnv1a(pass.vertex_shader.as_bytes()),
            fnv1a(fragment.map(|(shader, _)| shader).unwrap_or("").as_bytes()),
            pass.vertex_entry,
            fragment.map(|(_, entry)| entry).unwrap_or(""),
            Self::states_key(&pass.render, blend, cull),
            identities.join(",")
        );
        if let Some(pipeline) = self.pipelines.get(&key) {
            return Ok(pipeline.clone());
        }
        let label = format!("px_pass {}", pass.label);
        let vertex = module_of_wgsl(device, label.as_str(), pass.vertex_shader.as_str());
        if color_format.is_some() && fragment.is_none() {
            return Err(format!(
                "pass '{}' 挂了颜色附件，而这一笔的材质{}没有片元阶段：颜色没人写。                 （深度-only 的那一笔不该挂颜色附件；要颜色就得给它一份有片元 shader 的材质）",
                pass.label,
                match material {
                    Some(material) => format!(" '{}'", material.name),
                    None => "（这一笔没给材质）".to_string(),
                }
            ));
        }
        let fragment_module =
            fragment.map(|(shader, _)| module_of_wgsl(device, label.as_str(), shader));
        let widest = groups.iter().map(|(group, _)| *group).max().unwrap_or(0);
        let mut group_layouts: Vec<Option<&BindGroupLayout>> = vec![None; widest as usize + 1];
        for (group, layout) in &groups {
            group_layouts[*group as usize] = Some(layout);
        }
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("px_pass_geometry_pipeline_layout"),
            bind_group_layouts: &group_layouts,
            immediate_size: 0,
        });
        let targets: Vec<Option<ColorTargetState>> = match color_format {
            Some(format) => vec![Some(ColorTargetState {
                format,
                blend,
                write_mask: ColorWrites::ALL,
            })],
            None => Vec::new(),
        };
        let depth_stencil = match pass.render.depth {
            Attachment::None => None,
            _ => Some(wgpu::DepthStencilState {
                format: TextureFormat::Depth32Float,
                depth_write_enabled: Some(pass.render.depth_write),
                depth_compare: Some(pass.render.compare.to_wgpu()),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
        };
        let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
            label: Some(label.as_str()),
            layout: Some(&pipeline_layout),
            vertex: VertexState {
                module: &vertex,
                entry_point: Some(pass.vertex_entry.as_str()),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: match vertex_layout {
                    Some(layout) => std::slice::from_ref(layout),
                    None => &[],
                },
            },
            primitive: PrimitiveState {
                cull_mode: cull.to_wgpu(),
                front_face: pass.render.winding.to_wgpu(),
                ..Default::default()
            },
            depth_stencil,
            multisample: MultisampleState::default(),
            fragment: fragment_module.as_ref().map(|module| FragmentState {
                module,
                entry_point: fragment.map(|(_, entry)| entry),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipelines.insert(key, pipeline.clone());
        Ok(pipeline)
    }

    pub fn execute(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        plan: &Plan,
        frame: &Frame<'_>,
    ) -> Result<String, String> {
        self.execute_inner(device, encoder, plan, frame, None)
            .map(|(audit, _)| audit)
    }

    pub fn execute_timed(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        plan: &Plan,
        frame: &Frame<'_>,
        stamps: PassTimestamps<'_>,
    ) -> Result<(String, u32), String> {
        self.execute_inner(device, encoder, plan, frame, Some(stamps))
    }

    fn execute_inner(
        &mut self,
        device: &Device,
        encoder: &mut CommandEncoder,
        plan: &Plan,
        frame: &Frame<'_>,
        stamps: Option<PassTimestamps<'_>>,
    ) -> Result<(String, u32), String> {
        let wants_timestamps = stamps.is_some();
        let mut calls = 0_u32;
        plan.check()?;
        if plan.is_empty() {
            return Ok(("pass 表是空的：这一帧没有任何 pass".to_string(), calls));
        }
        let used: Vec<&str> = plan
            .passes
            .iter()
            .flat_map(|pass| {
                pass.reads
                    .iter()
                    .map(String::as_str)
                    .chain(pass.writes.iter().map(String::as_str))
                    .chain(pass.depth_target.as_deref())
            })
            .collect();
        for name in &self.seeded {
            if !used.contains(&name.as_str()) {
                return Err(format!(
                    "宿主 seed 过 '{name}'，而这份计划里没有任何一条 pass 用到它\
                     （计划声明过的资源：{}）—— seed 了却没人用，意味着宿主与文档对\
                     『存在什么』意见不一致，而那种不一致不许是静默的",
                    plan.name_list()
                ));
            }
        }
        let layout = plan.layout.clone();
        let sampler = self.sampler(device);
        let mut audit: Vec<String> = Vec::new();

        for (index, pass) in plan.passes.iter().enumerate() {
            let slots: Option<PassSlots> = match stamps.as_ref() {
                Some(stamps) => match stamps.slots(index) {
                    Some(slots) => Some(slots),
                    None => {
                        return Err(format!(
                            "时间戳排布里只有 {} 条 pass，而这一条是第 {index} 条（'{}'）：\
                             计划与排布不是同一次算出来的 —— 照着读会读到别人的格",
                            stamps.len(),
                            pass.label
                        ));
                    }
                },
                None => None,
            };
            if pass.kind == PassKind::Copy {
                let source_name = pass.reads.first().ok_or_else(|| {
                    format!("第 {index} 条 pass '{}' 是 copy，却没给 reads", pass.label)
                })?;
                let target_name = pass.writes.first().ok_or_else(|| {
                    format!("第 {index} 条 pass '{}' 是 copy，却没给 writes", pass.label)
                })?;
                let (source, target) = self.copy_pair(
                    device,
                    plan,
                    frame,
                    index,
                    &pass.label,
                    source_name,
                    target_name,
                )?;
                let extent = Extent3d {
                    width: source.width(),
                    height: source.height(),
                    depth_or_array_layers: source.depth_or_array_layers(),
                };
                if let Some(slots) = slots {
                    write_stamp(stamps.as_ref(), encoder, slots.envelope.0, &mut calls);
                }
                encoder.copy_texture_to_texture(
                    TexelCopyTextureInfo {
                        texture: &source,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect: copy_aspect(source.format()),
                    },
                    TexelCopyTextureInfo {
                        texture: &target,
                        mip_level: 0,
                        origin: Origin3d::ZERO,
                        aspect: copy_aspect(target.format()),
                    },
                    extent,
                );
                if let Some(slots) = slots {
                    write_stamp(stamps.as_ref(), encoder, slots.envelope.1, &mut calls);
                }
                audit.push(format!(
                    "pass {index} '{}'（copy）搬运 '{source_name}' → '{target_name}'｜{}×{} {:?}\
                     ｜不建管线、不开 render pass",
                    pass.label,
                    extent.width,
                    extent.height,
                    source.format()
                ));
                continue;
            }

            let color = match pass.render.color {
                Attachment::None => None,
                load => {
                    let target = pass.target().ok_or_else(|| {
                        format!(
                            "第 {index} 条 pass '{}' 挂了颜色附件却没有 writes",
                            pass.label
                        )
                    })?;
                    let (view, format) =
                        self.resolve(device, plan, frame, index, &pass.label, target, Role::Write)?;
                    Some((view, format, load))
                }
            };

            let depth = match pass.render.depth {
                Attachment::None => None,
                load => {
                    let name = pass.depth_target.as_deref().ok_or_else(|| {
                        format!(
                            "第 {index} 条 pass '{}' 挂了深度附件，却没给 depth_target",
                            pass.label
                        )
                    })?;
                    let (view, format) = match (pass.layer, plan.resource(name)) {
                        (Some(layer), Some(resource)) => {
                            let (width, height) = resource.size.resolve(frame.width, frame.height);
                            (
                                self.layer_view(device, resource, width, height, layer)?,
                                resource.format.to_wgpu(),
                            )
                        }
                        _ => self.resolve(
                            device,
                            plan,
                            frame,
                            index,
                            &pass.label,
                            name,
                            Role::Depth,
                        )?,
                    };
                    if format != TextureFormat::Depth32Float {
                        return Err(format!(
                            "第 {index} 条 pass '{}' 的深度目标 '{name}' 是 {format:?}：\
                             深度附件只能是 depth32float（这个渲染器只有一种深度约定）",
                            pass.label
                        ));
                    }
                    Some((view, load))
                }
            };

            let fullscreen = pass.kind == PassKind::Fullscreen;
            let mut bound: Vec<(u32, TextureView)> = Vec::new();
            if fullscreen {
                for (read, binding) in pass.reads.iter().zip(pass.slots.iter()) {
                    let (view, _) =
                        self.resolve(device, plan, frame, index, &pass.label, read, Role::Read)?;
                    bound.push((*binding, view));
                }
                if let Some((destination, _, _)) = &color {
                    if bound.iter().any(|(_, view)| view == destination) {
                        return Err(format!(
                            "pass '{}' 的读目标与写目标是**同一个视图**：宿主给这一条 pass 的这两样\
                             指到了同一张纹理（要读写同一张就得让宿主分开给，例如 ping-pong 的两张）",
                            pass.label
                        ));
                    }
                }
            }

            let mut draws: Vec<(
                RenderPipeline,
                &ResolvedGeometry<'_>,
                Option<&ResolvedMaterial<'_>>,
            )> = Vec::new();
            if !fullscreen {
                let color_format = color.as_ref().map(|(_, format, _)| *format);
                let mut shapes: Option<(String, String)> = None;
                for draw in &pass.draws {
                    let geometry = frame
                        .geometries
                        .iter()
                        .find(|geometry| geometry.name == draw.geometry)
                        .ok_or_else(|| {
                            format!(
                                "pass '{}' 要画几何 '{}'，而宿主这一帧没给这个名字（给了：{}）",
                                pass.label,
                                draw.geometry,
                                name_list(frame.geometries.iter().map(|g| g.name))
                            )
                        })?;
                    let material = if draw.material.is_empty() {
                        None
                    } else {
                        Some(
                            frame
                                .materials
                                .iter()
                                .find(|material| material.name == draw.material)
                                .ok_or_else(|| {
                                    format!(
                                        "pass '{}' 要材质 '{}'，而宿主这一帧没给这个名字（给了：{}）",
                                        pass.label,
                                        draw.material,
                                        name_list(frame.materials.iter().map(|m| m.name))
                                    )
                                })?,
                        )
                    };
                    let shape = match &geometry.vertices {
                        Some((_, layout)) => Self::vertex_layout_key(layout),
                        None => "procedural（没有顶点缓冲）".to_string(),
                    };
                    if geometry.instances.is_empty() {
                        return Err(format!(
                            "pass '{}' 的几何 '{}' 给的实例区间是 {}..{}（空的）：\
                             一笔 draw 至少要画一个实例；宿主不该给出这一笔",
                            pass.label,
                            draw.geometry,
                            geometry.instances.start,
                            geometry.instances.end
                        ));
                    }
                    match &shapes {
                        Some((first, first_shape)) if *first_shape != shape => {
                            return Err(format!(
                                "pass '{}' 里两笔 draw 的顶点布局不同：'{}' 是 {first_shape}，                                 '{}' 是 {shape} —— 顶点阶段挂在 pass 上，套到另一套布局上就是错的。                                 一条 pass 只能画同一种布局的几何（§129）",
                                pass.label, first, draw.geometry
                            ));
                        }
                        Some(_) => {}
                        None => shapes = Some((draw.geometry.clone(), shape)),
                    }
                    let pipeline =
                        self.pipeline_geometry(device, pass, color_format, geometry, material)?;
                    draws.push((pipeline, geometry, material));
                }
            }

            let mut fullscreen_draw = None;
            let mut geometry_params: Option<BindGroup> = None;
            if !fullscreen && !pass.params.is_empty() {
                geometry_params = Some(self.geometry_params_group(
                    device,
                    layout.geometry_group,
                    layout.geometry_params_binding,
                    &pass.params,
                ));
            }
            if fullscreen {
                let pass_layout = match &pass.texture_slots {
                    Some(slots) => Layout {
                        slots: slots.clone(),
                        ..layout.clone()
                    },
                    None => layout.clone(),
                };
                let pipeline = self.pipeline_fullscreen(
                    device,
                    &pass_layout,
                    pass,
                    color.as_ref().map(|(_, format, _)| *format),
                    frame.zero_dummy.map(|(zero_layout, _)| zero_layout),
                );
                let bind_group = self.bind_group(
                    device,
                    encoder,
                    &pass_layout,
                    &sampler,
                    &pass.params,
                    &bound,
                );
                fullscreen_draw = Some((pipeline, bind_group));
            }

            let space = cell_space(pass, index, plan, frame)?;
            let pass_label = format!("px_pass {}", pass.label);
            let color_attachments: Vec<Option<RenderPassColorAttachment>> = match &color {
                Some((view, _, load)) => vec![Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: match load {
                            Attachment::Clear(color) => LoadOp::Clear(color.to_wgpu()),
                            Attachment::Load => LoadOp::Load,
                            Attachment::None => unreachable!("颜色附件是 None 的走不到这里"),
                        },
                        store: StoreOp::Store,
                    },
                })],
                None => Vec::new(),
            };
            let depth_attachment = match (&depth, pass.render.depth) {
                (Some((view, load)), _) => Some(RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: Some(Operations {
                        load: match load {
                            Attachment::Clear(value) => LoadOp::Clear(*value),
                            Attachment::Load => LoadOp::Load,
                            Attachment::None => unreachable!("深度附件是 None 的走不到这里"),
                        },
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                (None, _) => None,
            };
            let pass_slots: Option<PassSlots> = slots;
            let timestamp_writes = stamps
                .as_ref()
                .zip(pass_slots)
                .map(|(stamps, slots)| stamps.pass_writes(slots, &mut calls));
            let descriptor = RenderPassDescriptor {
                label: Some(pass_label.as_str()),
                color_attachments: &color_attachments,
                depth_stencil_attachment: depth_attachment,
                timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            };
            let mut render_pass = encoder.begin_render_pass(&descriptor);
            if let (Some(stamps), Some(slots)) = (stamps.as_ref(), pass_slots) {
                if let Some((begin, _)) = slots.inside {
                    render_pass.write_timestamp(stamps.query_set, begin);
                    calls += 1;
                }
            }
            match pass.viewport {
                Some(rect) => {
                    render_pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0)
                }
                None => match space {
                    CellSpace::Whole => {}
                    CellSpace::Viewport(rect) => {
                        render_pass.set_viewport(rect[0], rect[1], rect[2], rect[3], 0.0, 1.0)
                    }
                    CellSpace::Scissor(rect) => render_pass.set_scissor_rect(
                        rect[0] as u32,
                        rect[1] as u32,
                        rect[2] as u32,
                        rect[3] as u32,
                    ),
                },
            }
            if let Some((pipeline, bind_group)) = &fullscreen_draw {
                render_pass.set_pipeline(pipeline);
                if let Some((_, zero)) = &frame.zero_dummy {
                    render_pass.set_bind_group(0, *zero, &[]);
                }
                render_pass.set_bind_group(layout.group, bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            } else {
                for (pipeline, geometry, material) in &draws {
                    render_pass.set_pipeline(pipeline);
                    if let Some(material) = material {
                        for group in &material.groups {
                            if group.dynamic {
                                render_pass.set_bind_group(
                                    group.group,
                                    group.bind_group,
                                    &[group.dynamic_offset],
                                );
                            } else {
                                render_pass.set_bind_group(group.group, group.bind_group, &[]);
                            }
                        }
                    }
                    if let Some(group) = &geometry_params {
                        render_pass.set_bind_group(
                            layout.geometry_group,
                            group,
                            &[pass.params_offset],
                        );
                    }
                    if let Some((buffer, _)) = &geometry.vertices {
                        render_pass.set_vertex_buffer(0, buffer.slice(..));
                    }
                    match &geometry.indices {
                        Some((buffer, format, count)) => {
                            render_pass.set_index_buffer(buffer.slice(..), *format);
                            render_pass.draw_indexed(0..*count, 0, geometry.instances.clone());
                        }
                        None => {
                            render_pass.draw(0..geometry.vertex_count, geometry.instances.clone())
                        }
                    }
                }
            }
            if let (Some(stamps), Some(slots)) = (stamps.as_ref(), pass_slots) {
                if let Some((_, end)) = slots.inside {
                    render_pass.write_timestamp(stamps.query_set, end);
                    calls += 1;
                }
            }
            drop(render_pass);

            audit.push(format!(
                "pass {index} '{}'（{}）读 [{}] 写 '{}'｜颜色 {}｜深度 {}｜层 {}｜格子 {}｜画的：{}",
                pass.label,
                pass.kind.name(),
                pass.reads.join(" / "),
                color
                    .as_ref()
                    .map(|(_, format, _)| format!("{format:?}"))
                    .unwrap_or_else(|| "（不挂）".to_string()),
                match pass.render.color {
                    Attachment::Clear(_) => "清".to_string(),
                    Attachment::Load => "接着上次".to_string(),
                    Attachment::None => "不挂".to_string(),
                },
                match pass.render.depth {
                    Attachment::Clear(_) => "清".to_string(),
                    Attachment::Load => "接着上次".to_string(),
                    Attachment::None => "不挂".to_string(),
                },
                match pass.layer {
                    Some(layer) => format!("第 {layer} 层（{} 的）", pass.depth_target.as_deref().unwrap_or("?")),
                    None => "不分层".to_string(),
                },
                match pass.viewport {
                    Some(rect) => format!(
                        "附件里的 viewport ({}, {}, {}, {})",
                        rect[0], rect[1], rect[2], rect[3]
                    ),
                    None => match space {
                        CellSpace::Whole => "整幅".to_string(),
                        CellSpace::Viewport(rect) => format!(
                            "viewport ({}, {}, {}, {})",
                            rect[0], rect[1], rect[2], rect[3]
                        ),
                        CellSpace::Scissor(rect) => format!(
                            "scissor ({}, {}, {}, {})（不设 viewport：宿主目标那一条按 oracle 的形状搬）",
                            rect[0], rect[1], rect[2], rect[3]
                        ),
                    },
                },
                if fullscreen {
                    format!("全屏三角｜参数 {} 字节｜格 {}", pass.params.len(), pass.slots.len())
                } else {
                    draws
                        .iter()
                        .map(|(_, geometry, material)| {
                            let instances = if geometry.instances.end == geometry.instances.start + 1 {
                                format!("实例 {}", geometry.instances.start)
                            } else {
                                format!("实例 {}..{}", geometry.instances.start, geometry.instances.end)
                            };
                            match material {
                                Some(material) => {
                                    format!("{}+{}（{instances}）", geometry.name, material.name)
                                }
                                None => format!("{}（无材质，{instances}）", geometry.name),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" / ")
                }
            ));
        }
        audit.push(format!(
            "时间戳：这一帧发了 {calls} 条（{}）",
            if wants_timestamps {
                "每条 pass 四格：包络一对 + pass 内一对；copy 不开 pass ⇒ 只有包络那一对"
            } else {
                "**没有要** —— 调用方没给查询集，一条 write_timestamp 都没发"
            }
        ));
        Ok((audit.join("\n"), calls))
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_pair(
        &mut self,
        device: &Device,
        plan: &Plan,
        frame: &Frame<'_>,
        index: usize,
        label: &str,
        source_name: &str,
        target_name: &str,
    ) -> Result<(Texture, Texture), String> {
        let pick = |name: &str, what: &str| -> Result<ResourceSpec, String> {
            plan.resource(name).cloned().ok_or_else(|| {
                format!(
                    "第 {index} 条 pass '{label}'（copy）{what} '{name}'：\
                     文档没声明这个资源（声明了的：{}）。拷贝的两端都要是**文档声明的资源** ——\
                     宿主给的外部目标只有视图，而拷贝要的是纹理本身",
                    plan.name_list()
                )
            })
        };
        let source = pick(source_name, "的源")?;
        let target = pick(target_name, "的目标")?;
        let (width, height) = source.size.resolve(frame.width, frame.height);
        let source_texture = self.resource_texture(device, &source, width, height);
        let (target_width, target_height) = target.size.resolve(frame.width, frame.height);
        let target_texture = self.resource_texture(device, &target, target_width, target_height);

        let source_shape = (
            source_texture.width(),
            source_texture.height(),
            source_texture.format(),
            source_texture.depth_or_array_layers(),
            source_texture.mip_level_count(),
        );
        let target_shape = (
            target_texture.width(),
            target_texture.height(),
            target_texture.format(),
            target_texture.depth_or_array_layers(),
            target_texture.mip_level_count(),
        );
        if source_shape != target_shape {
            return Err(format!(
                "第 {index} 条 pass '{label}'（copy）搬不了：'{source_name}' 是 {}×{} {:?} \
                 {} 层 {} 级 mip，'{target_name}' 是 {}×{} {:?} {} 层 {} 级 mip —— \
                 两张图必须逐格同规格（这一版不做缩放、不做格式转换，也不裁着拷）",
                source_shape.0,
                source_shape.1,
                source_shape.2,
                source_shape.3,
                source_shape.4,
                target_shape.0,
                target_shape.1,
                target_shape.2,
                target_shape.3,
                target_shape.4
            ));
        }
        Ok((source_texture, target_texture))
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
            Role::Depth => "深度",
        };
        let resource = plan.resource(name);
        let external = frame.sets.get(index).and_then(|set| {
            set.iter()
                .find(|external| external.name == name && external.role == role)
        });
        match (resource, external) {
            (Some(_), Some(external)) => Ok((external.view.clone(), external.format)),
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
                                Role::Depth => "（深度）",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_of(passes: Vec<PassPlan>) -> Plan {
        Plan {
            layout: Layout {
                group: 3,
                params_binding: 0,
                params_align: 16,
                slots: vec![Slot {
                    binding: 1,
                    dimension: Dimension::D2,
                    depth: false,
                }],
                geometry_group: 0,
                geometry_params_binding: 0,
            },
            resources: Vec::new(),
            passes,
        }
    }

    const TEST_FRAGMENT: &str = r#"
@group(3) @binding(0) var<uniform> params: vec4<f32>;
@group(3) @binding(1) var px_source: texture_2d<f32>;
@group(3) @binding(2) var px_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return textureSample(px_source, px_sampler, uv) + params;
}
"#;

    const TEST_VERTEX: &str = r#"
struct Out {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> Out {
    var out: Out;
    out.position = vec4<f32>(position, 1.0);
    return out;
}
"#;

    fn fullscreen(label: &str) -> PassPlan {
        PassPlan {
            kind: PassKind::Fullscreen,
            label: label.to_string(),
            shader: TEST_FRAGMENT.to_string(),
            entry: FRAGMENT_ENTRY.to_string(),
            writes: vec!["view".to_string()],
            params: vec![0; 16],
            render: RenderState::default(),
            ..Default::default()
        }
    }

    fn depth_only(label: &str) -> PassPlan {
        PassPlan {
            kind: PassKind::Geometry,
            label: label.to_string(),
            vertex_shader: TEST_VERTEX.to_string(),
            vertex_entry: "vs_main".to_string(),
            draws: vec![Draw {
                geometry: "planet".to_string(),
                material: String::new(),
            }],
            render: RenderState {
                color: Attachment::None,
                depth: Attachment::Clear(0.0),
                ..Default::default()
            },
            depth_target: Some("depth".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn the_default_state_is_exactly_what_the_executor_used_to_hardcode() {
        let state = RenderState::default();
        assert_eq!(state.color, Attachment::Clear(Color::TRANSPARENT));
        assert_eq!(state.depth, Attachment::None);
        assert_eq!(state.winding, Winding::Ccw);
        assert_eq!(state.compare, Compare::GreaterEqual);
        assert!(state.depth_write, "只加一个深度附件 ⇒ 想要的显然是写");
    }

    #[test]
    fn an_existing_fullscreen_pass_still_checks_out() {
        let plan = plan_of(vec![fullscreen("grade")]);
        assert!(plan.check().is_ok(), "{:?}", plan.check());
        assert_eq!(plan.passes[0].render, RenderState::default());
    }

    #[test]
    fn a_default_pass_plan_is_refused() {
        let err = plan_of(vec![PassPlan::default()])
            .check()
            .expect_err("全空的 pass ⇒ 拒");
        assert!(!err.is_empty());
        assert!(err.contains("shader") || err.contains("参数块"), "{err}");
    }

    #[test]
    fn every_render_state_variant_survives_a_text_round_trip() {
        let colors = [
            Attachment::None,
            Attachment::Load,
            Attachment::Clear(Color::TRANSPARENT),
            Attachment::Clear(Color::new(0.004, 0.005, 0.010, 1.0)),
        ];
        let depths = [
            Attachment::None,
            Attachment::Load,
            Attachment::Clear(0.0),
            Attachment::Clear(1.0),
            Attachment::Clear(0.5),
        ];
        let compares = [
            Compare::Never,
            Compare::Less,
            Compare::Equal,
            Compare::LessEqual,
            Compare::Greater,
            Compare::NotEqual,
            Compare::GreaterEqual,
            Compare::Always,
        ];
        let windings = [Winding::Ccw, Winding::Cw];

        let mut seen_text: Vec<String> = Vec::new();
        let mut count = 0_usize;
        for color in colors {
            for depth in depths {
                for depth_write in [true, false] {
                    for compare in compares {
                        for winding in windings {
                            let state = RenderState {
                                frag_depth: false,
                                color,
                                depth,
                                depth_write,
                                compare,
                                winding,
                            };
                            let text = state.name();
                            let back = RenderState::parse(&text)
                                .unwrap_or_else(|err| panic!("'{text}' 读不回来：{err}"));
                            assert_eq!(back, state, "往返不一致：'{text}'");
                            seen_text.push(text);
                            count += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(count, 4 * 5 * 2 * 8 * 2, "组合数变了");
        assert_eq!(
            seen_text.len(),
            seen_text
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "两个不同的状态写出了同一串文本 ⇒ name 不是单射，往返必有假的"
        );
        for compare in compares {
            assert!(
                seen_text
                    .iter()
                    .any(|text| text.contains(&format!("compare={}", compare.name()))),
                "{} 没出现在任何一轮里",
                compare.name()
            );
        }
        for cull in [Cull::None, Cull::Front, Cull::Back] {
            assert_eq!(Cull::parse(cull.name()).unwrap(), cull);
        }
        assert!(
            seen_text
                .iter()
                .any(|text| text.contains("color=clear(0.004,0.005,0.01,1)"))
        );
        assert!(
            seen_text
                .iter()
                .any(|text| text.contains("depth=clear(0.5)"))
        );
    }

    #[test]
    fn the_document_enums_round_trip_variant_by_variant() {
        for kind in [
            PassKind::Fullscreen,
            PassKind::Geometry,
            PassKind::Copy,
            PassKind::Compute,
        ] {
            assert_eq!(
                PassKind::parse(kind.name()).unwrap(),
                kind,
                "{}",
                kind.name()
            );
        }
        for format in [
            Format::Rgba8UnormSrgb,
            Format::Rgba16Float,
            Format::Depth32Float,
        ] {
            assert_eq!(Format::parse(format.name()).unwrap(), format);
        }
        for usage in [
            Use::RenderAttachment,
            Use::TextureBinding,
            Use::CopySrc,
            Use::CopyDst,
        ] {
            assert_eq!(Use::parse(usage.name()).unwrap(), usage, "{}", usage.name());
        }
        for compare in [
            Compare::Never,
            Compare::Less,
            Compare::Equal,
            Compare::LessEqual,
            Compare::Greater,
            Compare::NotEqual,
            Compare::GreaterEqual,
            Compare::Always,
        ] {
            assert_eq!(Compare::parse(compare.name()).unwrap(), compare);
        }
        for cull in [Cull::None, Cull::Front, Cull::Back] {
            assert_eq!(Cull::parse(cull.name()).unwrap(), cull);
        }
        for winding in [Winding::Ccw, Winding::Cw] {
            assert_eq!(Winding::parse(winding.name()).unwrap(), winding);
        }
        let color = Color::new(0.004, 0.005, 0.010, 1.0);
        assert_eq!(Color::parse(&color.name()).unwrap(), color);
        assert_eq!(color.name(), "0.004,0.005,0.01,1");
    }

    #[test]
    fn a_wrong_document_is_refused_with_the_offending_piece_named() {
        let err =
            RenderState::parse("color=none|depth=none|depth_write=true|compare=nope|winding=ccw")
                .expect_err("认不出的比较档 ⇒ 拒");
        assert!(err.contains("nope"), "{err}");
        assert!(err.contains("greater_equal"), "要列出认哪些：{err}");

        let err = RenderState::parse(
            "color=none|depth=none|depth_write=maybe|compare=always|winding=ccw",
        )
        .expect_err("depth_write 不是 bool ⇒ 拒");
        assert!(err.contains("maybe"), "{err}");

        let err = RenderState::parse("color=none|depth=none|compare=always|winding=ccw")
            .expect_err("缺 depth_write ⇒ 拒");
        assert!(err.contains("depth_write"), "{err}");
        assert!(err.contains("缺"), "{err}");

        let err = RenderState::parse(
            "color=none|depth=none|depth_write=true|compare=always|winding=ccw|nope=1",
        )
        .expect_err("多一个不认识的键 ⇒ 拒");
        assert!(err.contains("nope"), "{err}");

        let err = Attachment::<Color>::parse("clear(1,2,3)").expect_err("三个通道 ⇒ 拒");
        assert!(err.contains("四个数"), "{err}");

        let err = Attachment::<f32>::parse("clear(abc)").expect_err("深度不是数 ⇒ 拒");
        assert!(err.contains("abc"), "{err}");

        let err = Color::parse("1,2,x,4").expect_err("第三个通道不是数 ⇒ 拒");
        assert!(err.contains("第 3 个通道"), "{err}");

        let err = PassKind::parse("shadow").expect_err("认不出的 kind ⇒ 拒");
        assert!(err.contains("shadow"), "{err}");
        assert!(err.contains("geometry"), "要列出认哪些：{err}");
    }

    #[test]
    fn a_geometry_pass_may_carry_depth_only() {
        let plan = plan_of(vec![depth_only("prepass")]);
        assert!(plan.check().is_ok(), "{:?}", plan.check());
        assert_eq!(plan.passes[0].render.depth, Attachment::Clear(0.0));
        assert_eq!(plan.passes[0].render.color, Attachment::None);

        let mut opaque = fullscreen("opaque");
        opaque.kind = PassKind::Geometry;
        opaque.vertex_shader = TEST_VERTEX.to_string();
        opaque.vertex_entry = "vs_main".to_string();
        opaque.draws = vec![Draw {
            geometry: "planet".to_string(),
            material: "surface".to_string(),
        }];
        opaque.params = Vec::new();
        opaque.shader = String::new();
        opaque.entry = String::new();
        opaque.render = RenderState::parse(
            "color=load|depth=load|depth_write=true|compare=greater_equal|winding=ccw",
        )
        .expect("状态文本要能读回来");
        opaque.depth_target = Some("depth".to_string());
        let plan = plan_of(vec![opaque]);
        assert!(plan.check().is_ok(), "{:?}", plan.check());
        assert_eq!(plan.passes[0].render.winding, Winding::Ccw);
    }

    #[test]
    fn a_pass_with_no_attachment_at_all_is_refused() {
        let mut pass = fullscreen("nothing");
        pass.render.color = Attachment::None;
        pass.writes = Vec::new();
        let err = plan_of(vec![pass])
            .check()
            .expect_err("两个附件都不挂 ⇒ 拒");
        assert!(err.contains("附件"), "{err}");
    }

    #[test]
    fn a_pass_with_a_write_target_but_no_color_attachment_is_refused() {
        let mut pass = depth_only("confused");
        pass.writes = vec!["view".to_string()];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("空挂颜色却声明写目标 ⇒ 拒");
        assert!(err.contains("写目标"), "{err}");
    }

    #[test]
    fn a_fullscreen_pass_without_a_color_attachment_is_refused() {
        let mut pass = fullscreen("useless");
        pass.render.color = Attachment::None;
        pass.writes = Vec::new();
        pass.render.depth = Attachment::Clear(0.0);
        pass.depth_target = Some("depth".to_string());
        let err = plan_of(vec![pass])
            .check()
            .expect_err("全屏 pass 没有颜色附件 ⇒ 拒");
        assert!(err.contains("颜色附件"), "{err}");
    }

    #[test]
    fn a_compute_pass_must_not_carry_a_color_attachment() {
        let mut pass = fullscreen("reduce");
        pass.kind = PassKind::Compute;
        let err = plan_of(vec![pass])
            .check()
            .expect_err("compute 挂了颜色 ⇒ 拒");
        assert!(err.contains("compute"), "{err}");
        assert!(err.contains("颜色附件"), "{err}");
    }

    #[test]
    fn a_compute_pass_without_attachments_is_refused_for_the_capability_reason() {
        let mut pass = fullscreen("reduce");
        pass.kind = PassKind::Compute;
        pass.render.color = Attachment::None;
        pass.writes = Vec::new();
        let err = plan_of(vec![pass]).check().expect_err("compute ⇒ 拒");
        assert!(err.contains("静默跳过"), "{err}");
    }

    #[test]
    fn the_depth_target_must_pair_with_the_depth_attachment() {
        let mut pass = depth_only("prepass");
        pass.depth_target = None;
        let err = plan_of(vec![pass])
            .check()
            .expect_err("挂了深度却不说用哪张 ⇒ 拒");
        assert!(err.contains("depth_target"), "{err}");

        let mut pass = fullscreen("grade");
        pass.depth_target = Some("depth".to_string());
        let err = plan_of(vec![pass])
            .check()
            .expect_err("没挂深度却给了目标 ⇒ 拒");
        assert!(err.contains("没人用"), "{err}");

        let mut plan = plan_of(vec![depth_only("prepass")]);
        plan.resources.push(ResourceSpec {
            layers: 1,
            name: "depth".to_string(),
            format: Format::Rgba8UnormSrgb,
            size: SizeRule::View,
            usage: vec![Use::RenderAttachment],
        });
        let err = plan.check().expect_err("深度目标不是 depth32float ⇒ 拒");
        assert!(err.contains("depth32float"), "{err}");

        plan.resources[0].format = Format::Depth32Float;
        assert!(plan.check().is_ok(), "{:?}", plan.check());
    }

    #[test]
    fn a_geometry_pass_needs_a_vertex_stage_and_its_params_must_be_whole() {
        let mut pass = depth_only("prepass");
        pass.vertex_shader = String::new();
        let err = plan_of(vec![pass]).check().expect_err("没有顶点阶段 ⇒ 拒");
        assert!(err.contains("vertex_shader"), "{err}");

        let mut pass = depth_only("prepass");
        pass.params = vec![0; 16];
        assert!(
            plan_of(vec![pass]).check().is_ok(),
            "几何 pass 的参数块是虚拟影图那条路，不该再被拒"
        );

        let mut pass = depth_only("prepass");
        pass.params = vec![0; 20];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("20 字节不是 16 的正数倍 ⇒ 拒");
        assert!(err.contains("正数倍"), "{err}");

        let mut pass = depth_only("prepass");
        pass.reads = vec!["view".to_string()];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("几何 pass 给了 reads ⇒ 拒");
        assert!(err.contains("reads"), "{err}");
        assert!(err.contains("宿主解析"), "{err}");

        let mut pass = depth_only("prepass");
        pass.draws = vec![Draw::default()];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("一笔不说画哪份几何 ⇒ 拒");
        assert!(err.contains("geometry"), "{err}");

        let mut pass = depth_only("prepass");
        pass.shader = "fragment".to_string();
        pass.entry = "fs_main".to_string();
        let err = plan_of(vec![pass])
            .check()
            .expect_err("几何 pass 带片元阶段 ⇒ 拒");
        assert!(err.contains("材质"), "{err}");
        assert!(err.contains("fragment_shader"), "要指出该挪到哪一格：{err}");
    }

    #[test]
    fn a_fullscreen_pass_with_draws_is_refused() {
        let mut pass = fullscreen("grade");
        pass.draws = vec![Draw {
            geometry: "planet".to_string(),
            material: "surface".to_string(),
        }];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("全屏 pass 带 draws ⇒ 拒");
        assert!(err.contains("draws"), "{err}");
    }

    #[test]
    fn an_entry_point_that_is_not_in_the_shader_is_refused_by_name() {
        let mut pass = fullscreen("wrong-entry");
        pass.entry = "fs_wrong".to_string();
        let err = plan_of(vec![pass])
            .check()
            .expect_err("入口名不在 shader 里 ⇒ 拒");
        assert!(err.contains("fs_wrong"), "{err}");
        assert!(err.contains("fs_main"), "要列出它真有的入口：{err}");
    }

    #[test]
    fn a_vertex_entry_point_that_is_not_in_the_shader_is_refused_by_name() {
        let mut pass = depth_only("prepass");
        pass.vertex_entry = "vs_wrong".to_string();
        let err = plan_of(vec![pass])
            .check()
            .expect_err("顶点入口名不在那份 WGSL 里 ⇒ 拒");
        assert!(err.contains("vs_wrong"), "{err}");
        assert!(err.contains("vs_main"), "要列出它真有的入口：{err}");
    }

    #[test]
    fn a_shader_that_does_not_parse_is_refused_with_the_line() {
        let mut pass = fullscreen("broken");
        pass.shader = "这不是 WGSL".to_string();
        let err = plan_of(vec![pass]).check().expect_err("解析不过 ⇒ 拒");
        assert!(err.contains("WGSL 解析不过"), "{err}");
    }

    #[test]
    fn an_unresolved_draw_name_lists_the_ones_the_host_did_give() {
        let list = name_list(["planet", "atmosphere"].into_iter());
        assert_eq!(list, "planet / atmosphere");
        assert_eq!(name_list(std::iter::empty()), "（一个都没有）");
    }

    fn plan_with(resources: Vec<ResourceSpec>, passes: Vec<PassPlan>) -> Plan {
        Plan {
            layout: Layout::default(),
            resources,
            passes,
        }
    }

    fn resource(name: &str, size: SizeRule) -> ResourceSpec {
        ResourceSpec {
            name: name.to_string(),
            format: Format::Rgba8UnormSrgb,
            size,
            layers: 1,
            usage: vec![Use::RenderAttachment, Use::TextureBinding],
        }
    }

    fn geometry_into(label: &str, target: &str) -> PassPlan {
        PassPlan {
            kind: PassKind::Geometry,
            label: label.to_string(),
            vertex_shader: TEST_VERTEX.to_string(),
            vertex_entry: "vs_main".to_string(),
            draws: vec![Draw {
                geometry: "planet".to_string(),
                material: String::new(),
            }],
            writes: vec![target.to_string()],
            render: RenderState {
                color: Attachment::Clear(Color::TRANSPARENT),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    const CELL: [f32; 4] = [960.0, 0.0, 960.0, 640.0];

    #[test]
    fn a_view_sized_attachment_follows_the_cell() {
        let plan = plan_with(
            vec![resource("scene_color_a", SizeRule::View)],
            vec![geometry_into("opaque", "scene_color_a")],
        );
        assert_eq!(
            cell_space_for(&plan.passes[0], 0, &plan, CELL, false).unwrap(),
            CellSpace::Viewport(CELL)
        );
    }

    #[test]
    fn a_fixed_size_attachment_is_not_part_of_the_cell() {
        let mut shadow = depth_only("point_shadow");
        shadow.depth_target = Some("point_shadow_textures".to_string());
        let plan = plan_with(
            vec![resource(
                "point_shadow_textures",
                SizeRule::Fixed(1024, 1024),
            )],
            vec![shadow],
        );
        assert_eq!(
            cell_space_for(&plan.passes[0], 0, &plan, CELL, false).unwrap(),
            CellSpace::Whole,
            "影子图不跟着格子走"
        );
    }

    #[test]
    fn the_pass_that_hands_over_to_the_host_target_is_scissored() {
        let plan = plan_with(
            vec![resource("scene_color_a", SizeRule::View)],
            vec![fullscreen("blit")],
        );
        assert_eq!(
            cell_space_for(&plan.passes[0], 0, &plan, CELL, true).unwrap(),
            CellSpace::Scissor(CELL)
        );
    }

    #[test]
    fn the_timestamp_slots_have_no_gap_a_copy_leaves_unwritten() {
        assert_eq!(TIMESTAMP_SLOTS_PER_PASS, 4);
        assert_eq!(TIMESTAMP_FRAME_SLOTS, 2);
        let kinds = [PassKind::Geometry, PassKind::Copy, PassKind::Fullscreen];
        let (slots, frame_begin) = frame_slots(&kinds);
        assert_eq!(
            slots[0],
            PassSlots {
                envelope: (0, 3),
                inside: Some((1, 2)),
            }
        );
        assert_eq!(
            slots[1],
            PassSlots {
                envelope: (4, 5),
                inside: None,
            },
            "copy 不开 render pass ⇒ 它只有两格（那两格就是起 / 止），不是四格里空两格"
        );
        assert_eq!(
            slots[2],
            PassSlots {
                envelope: (6, 9),
                inside: Some((7, 8)),
            }
        );
        assert_eq!(frame_begin, 10, "帧级那一对紧跟在最后一条 pass 之后");
        assert_eq!(
            pass_slot_count(PassKind::Geometry)
                + pass_slot_count(PassKind::Copy)
                + pass_slot_count(PassKind::Fullscreen),
            frame_begin
        );
        assert_eq!(frame_begin + TIMESTAMP_FRAME_SLOTS, 12, "一帧一共 12 格");
    }

    #[test]
    fn a_geometry_pass_writing_the_host_target_is_refused() {
        let plan = plan_with(vec![], vec![geometry_into("opaque", "view")]);
        let why = cell_space_for(&plan.passes[0], 0, &plan, CELL, true).unwrap_err();
        assert!(why.contains("opaque"), "要说清是哪一条：{why}");
        assert!(why.contains("geometry"), "要说清是什么形状：{why}");
        assert!(why.contains("scissor"), "要说清冲突在哪：{why}");
    }

    #[test]
    fn the_frame_cell_moves_the_geometry_and_not_only_the_audit() {
        let (device, queue) = test_device();
        let mut executor = Executor::new();
        let side = SIDE;
        let target = device.create_texture(&TextureDescriptor {
            label: Some("px_pass 判据：格子里的中间目标"),
            size: Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let spec = ResourceSpec {
            name: "a".to_string(),
            format: Format::Rgba8UnormSrgb,
            size: SizeRule::View,
            layers: 1,
            usage: vec![Use::RenderAttachment, Use::TextureBinding, Use::CopySrc],
        };
        executor
            .seed(&spec, side, side, target.clone())
            .expect("把中间目标 seed 进池子");

        let plan = plan_with(
            vec![spec],
            vec![PassPlan {
                kind: PassKind::Geometry,
                label: "opaque".to_string(),
                vertex_shader: TRIANGLE_VERTEX.to_string(),
                vertex_entry: "vs_main".to_string(),
                writes: vec!["a".to_string()],
                draws: vec![draw("near", "white")],
                render: RenderState::parse(STATE_BASE).expect("状态文本"),
                ..Default::default()
            }],
        );

        let near = vertex_buffer(&device, 0.5);
        let vertex_layout = VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TINT_ATTRIBUTES,
        };
        let geometries = [ResolvedGeometry {
            name: "near",
            vertices: Some((&near, vertex_layout.clone())),
            indices: None,
            vertex_count: 3,
            instances: 0..1,
        }];
        let tint_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass 判据材质布局（格子）"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let white = test_material(&device, &tint_layout, [1.0, 1.0, 1.0, 1.0]);
        let materials = [resolved_material("white", &white, &tint_layout, Cull::None)];

        let frame = Frame {
            zero_dummy: None,
            width: side,
            height: side,
            viewport: Some([4.0, 0.0, 4.0, 8.0]),
            sets: &[],
            geometries: &geometries,
            materials: &materials,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据（格子）"),
        });
        let audit = executor
            .execute(&device, &mut encoder, &plan, &frame)
            .unwrap_or_else(|err| panic!("execute 失败：{err}"));
        println!("{audit}");
        assert!(
            audit.contains("viewport (4, 0, 4, 8)"),
            "审计要说清这一条落在格子里：{audit}"
        );
        let pixels = read_points(&device, &queue, encoder, &target, &[(6, 4), (4, 4), (0, 4)]);
        assert_eq!(pixels[0], WHITE, "格子里那一笔应当被移到右半幅（(6,4) 白）");
        assert_eq!(
            pixels[1], RED,
            "(4,4) 落在三角外面 ⇒ 清屏色（不带格子时它是白的）"
        );
        assert_eq!(pixels[2], RED, "格子外面一个像素都不许动");
    }

    #[test]
    fn the_pass_viewport_moves_the_geometry_and_not_only_the_audit() {
        let (device, queue) = test_device();
        let mut executor = Executor::new();
        let side = SIDE;
        let target = device.create_texture(&TextureDescriptor {
            label: Some("px_pass 判据：pass viewport 的池内目标"),
            size: Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let spec = ResourceSpec {
            name: "a".to_string(),
            format: Format::Rgba8UnormSrgb,
            size: SizeRule::View,
            layers: 1,
            usage: vec![Use::RenderAttachment, Use::TextureBinding, Use::CopySrc],
        };
        executor
            .seed(&spec, side, side, target.clone())
            .expect("把中间目标 seed 进池子");

        let plan = plan_with(
            vec![spec],
            vec![PassPlan {
                kind: PassKind::Geometry,
                label: "page".to_string(),
                vertex_shader: TRIANGLE_VERTEX.to_string(),
                vertex_entry: "vs_main".to_string(),
                writes: vec!["a".to_string()],
                draws: vec![draw("near", "white")],
                render: RenderState::parse(STATE_BASE).expect("状态文本"),
                viewport: Some([4.0, 0.0, 4.0, 8.0]),
                ..Default::default()
            }],
        );

        let near = vertex_buffer(&device, 0.5);
        let vertex_layout = VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TINT_ATTRIBUTES,
        };
        let geometries = [ResolvedGeometry {
            name: "near",
            vertices: Some((&near, vertex_layout.clone())),
            indices: None,
            vertex_count: 3,
            instances: 0..1,
        }];
        let tint_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass 判据材质布局（pass viewport）"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let white = test_material(&device, &tint_layout, [1.0, 1.0, 1.0, 1.0]);
        let materials = [resolved_material("white", &white, &tint_layout, Cull::None)];

        let frame = Frame {
            zero_dummy: None,
            width: side,
            height: side,
            viewport: None,
            sets: &[],
            geometries: &geometries,
            materials: &materials,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据（pass viewport）"),
        });
        let audit = executor
            .execute(&device, &mut encoder, &plan, &frame)
            .unwrap_or_else(|err| panic!("execute 失败：{err}"));
        println!("{audit}");
        assert!(
            audit.contains("附件里的 viewport (4, 0, 4, 8)"),
            "审计要说清这一条落在附件的哪一块：{audit}"
        );
        let pixels = read_points(&device, &queue, encoder, &target, &[(6, 4), (4, 4), (0, 4)]);
        assert_eq!(pixels[0], WHITE, "附件里那一块应当被移到右半幅（(6,4) 白）");
        assert_eq!(
            pixels[1], RED,
            "(4,4) 落在三角外面 ⇒ 清屏色（没发 viewport 时它是白的）"
        );
        assert_eq!(pixels[2], RED, "附件里那一块外面一个像素都不许动");
    }

    #[test]
    fn a_pass_viewport_with_a_degenerate_shape_is_refused() {
        let mut pass = geometry_into("page", "a");
        pass.viewport = Some([0.0, 0.0, 0.0, 8.0]);
        let why = plan_with(
            vec![ResourceSpec {
                name: "a".to_string(),
                format: Format::Depth32Float,
                size: SizeRule::Fixed(8, 8),
                layers: 1,
                usage: vec![Use::RenderAttachment],
            }],
            vec![pass],
        )
        .check()
        .expect_err("零宽的格子 ⇒ 拒");
        assert!(why.contains("page"), "要说清是哪一条：{why}");
        assert!(why.contains("viewport"), "要说清是哪一栏：{why}");

        let mut orphan = geometry_into("orphan", "a");
        orphan.viewport = Some([0.0, 0.0, 4.0, 4.0]);
        orphan.writes.clear();
        orphan.render.color = Attachment::None;
        orphan.render.depth = Attachment::None;
        orphan.depth_target = None;
        let why = plan_with(
            vec![ResourceSpec {
                name: "a".to_string(),
                format: Format::Depth32Float,
                size: SizeRule::Fixed(8, 8),
                layers: 1,
                usage: vec![Use::RenderAttachment],
            }],
            vec![orphan],
        )
        .check()
        .expect_err("没有附件却给 viewport ⇒ 拒");
        assert!(
            why.contains("viewport") || why.contains("附件"),
            "拒词要说清是 viewport 与附件对不上：{why}"
        );
    }

    #[test]
    fn the_timestamps_measure_the_pass_and_the_reading_is_not_all_zero() {
        use wgpu::Features;
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .expect("Vulkan 适配器");
        let wanted = Features::TIMESTAMP_QUERY
            | Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
            | Features::TIMESTAMP_QUERY_INSIDE_PASSES;
        let available = adapter.features();
        assert!(
            available.contains(wanted),
            "后端断言失败：这一台缺时间戳能力（{:?} 里没有 {:?}）⇒ \
             逐条 pass 的 GPU 时间戳在这台机器上量不了；这一条**不静默跳过**",
            available,
            wanted - available
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("px_pass 判据设备（时间戳）"),
            required_features: wanted,
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .expect("带时间戳的设备");

        let side = SIDE;
        let target = device.create_texture(&TextureDescriptor {
            label: Some("px_pass 判据：时间戳那张目标"),
            size: Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let spec = ResourceSpec {
            name: "a".to_string(),
            format: Format::Rgba8UnormSrgb,
            size: SizeRule::View,
            layers: 1,
            usage: vec![Use::RenderAttachment, Use::TextureBinding, Use::CopySrc],
        };
        let mut executor = Executor::new();
        executor
            .seed(&spec, side, side, target.clone())
            .expect("把中间目标 seed 进池子");
        let plan = plan_with(
            vec![spec],
            vec![PassPlan {
                kind: PassKind::Geometry,
                label: "opaque".to_string(),
                vertex_shader: TRIANGLE_VERTEX.to_string(),
                vertex_entry: "vs_main".to_string(),
                writes: vec!["a".to_string()],
                draws: vec![draw("near", "white")],
                render: RenderState::parse(STATE_BASE).expect("状态文本"),
                ..Default::default()
            }],
        );

        let near = vertex_buffer(&device, 0.5);
        let vertex_layout = VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TINT_ATTRIBUTES,
        };
        let geometries = [ResolvedGeometry {
            name: "near",
            vertices: Some((&near, vertex_layout.clone())),
            indices: None,
            vertex_count: 3,
            instances: 0..1,
        }];
        let tint_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass 判据材质布局（时间戳）"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let white = test_material(&device, &tint_layout, [1.0, 1.0, 1.0, 1.0]);
        let materials = [resolved_material("white", &white, &tint_layout, Cull::None)];
        let frame = Frame {
            zero_dummy: None,
            width: side,
            height: side,
            viewport: None,
            sets: &[],
            geometries: &geometries,
            materials: &materials,
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据（不要时间戳）"),
        });
        let audit = executor
            .execute(&device, &mut encoder, &plan, &frame)
            .unwrap_or_else(|err| panic!("execute 失败：{err}"));
        assert!(
            audit.contains("时间戳：这一帧发了 0 条"),
            "不要仪器的那一档必须**明说一条都没发**（缺席 ⇒ 零调用）：{audit}"
        );

        let (layout, frame_begin) = frame_slots(&[PassKind::Geometry]);
        let count = frame_begin + TIMESTAMP_FRAME_SLOTS;
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("px_pass 判据时间戳"),
            ty: wgpu::QueryType::Timestamp,
            count,
        });
        let bytes = u64::from(count) * u64::from(wgpu::QUERY_SIZE);
        let align = u64::from(wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_pass 判据时间戳 resolve"),
            size: bytes.div_ceil(align) * align,
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_pass 判据时间戳回读"),
            size: bytes.div_ceil(align) * align,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let stamps = PassTimestamps::new(&query_set, &layout);
        let expected = stamps.slots(0).expect("第 0 条 pass 的格");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据（要时间戳）"),
        });
        encoder.write_timestamp(&query_set, frame_begin);
        let (audit, calls) = executor
            .execute_timed(&device, &mut encoder, &plan, &frame, stamps)
            .unwrap_or_else(|err| panic!("execute_timed 失败：{err}"));
        encoder.write_timestamp(&query_set, frame_begin + 1);
        queue.submit(Some(encoder.finish()));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据（时间戳 resolve）"),
        });
        encoder.resolve_query_set(&query_set, 0..count, &resolve, 0);
        encoder.copy_buffer_to_buffer(&resolve, 0, &readback, 0, bytes);
        queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(60)),
            })
            .expect("等时间戳");
        receiver.recv().expect("映射回调").expect("映射时间戳缓冲");
        let ticks: Vec<u64> = {
            let data = slice.get_mapped_range();
            data.chunks_exact(8)
                .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 字节")))
                .collect()
        };
        readback.unmap();
        let slots = expected;
        println!(
            "审计：{audit}\n时间戳（{} 格）：{:?}｜pass 内那一对 {:?}",
            ticks.len(),
            ticks,
            slots.inside
        );
        assert_eq!(
            calls, TIMESTAMP_SLOTS_PER_PASS,
            "一条 render pass 该发四条（包络一对 + pass 内一对）"
        );
        let (inside_begin, inside_end) = slots.inside.expect("geometry pass 有 pass 内那一对");
        let inside = ticks[inside_end as usize].wrapping_sub(ticks[inside_begin as usize]);
        let envelope =
            ticks[slots.envelope.1 as usize].wrapping_sub(ticks[slots.envelope.0 as usize]);
        let whole = ticks[frame_begin as usize + 1].wrapping_sub(ticks[frame_begin as usize]);
        println!(
            "读数（格，周期 {} ns）：pass 内 {inside}｜包络 {envelope}｜帧级 {whole}",
            queue.get_timestamp_period()
        );
        assert!(
            inside > 0,
            "一条真画了像素的 pass，pass 内那一段必须 > 0 格（实测 {inside}）"
        );
        assert!(
            envelope >= inside,
            "包络含 pass 的开/关 ⇒ 它不可能比 pass 内那一段还短（包络 {envelope} < 内 {inside}）"
        );
        assert!(
            whole >= envelope,
            "帧级那条 span 含这条 pass 的全部 + 清屏 ⇒ 不可能比一条 pass 的包络还短\
             （帧级 {whole} < 包络 {envelope}）"
        );
    }

    fn read_points(
        device: &Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        target: &Texture,
        points: &[(u32, u32)],
    ) -> Vec<[u8; 4]> {
        let padded = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_pass 判据回读（格子）"),
            size: padded * u64::from(SIDE),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(SIDE),
                },
            },
            Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
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
        let out = points
            .iter()
            .map(|(x, y)| {
                let at = (y * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT + x * 4) as usize;
                [data[at], data[at + 1], data[at + 2], data[at + 3]]
            })
            .collect();
        drop(data);
        readback.unmap();
        out
    }

    fn copy_plan() -> Plan {
        Plan {
            layout: Layout::default(),
            resources: vec![
                ResourceSpec {
                    layers: 1,
                    name: "src".to_string(),
                    format: Format::Rgba8UnormSrgb,
                    size: SizeRule::View,
                    usage: vec![Use::RenderAttachment, Use::CopySrc, Use::TextureBinding],
                },
                ResourceSpec {
                    layers: 1,
                    name: "dst".to_string(),
                    format: Format::Rgba8UnormSrgb,
                    size: SizeRule::View,
                    usage: vec![Use::RenderAttachment, Use::CopyDst, Use::TextureBinding],
                },
            ],
            passes: vec![PassPlan {
                kind: PassKind::Copy,
                label: "copy".to_string(),
                reads: vec!["src".to_string()],
                writes: vec!["dst".to_string()],
                render: RenderState::parse(
                    "color=none|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
                )
                .expect("copy 的状态就是『没有附件』那一种"),
                ..Default::default()
            }],
        }
    }

    #[test]
    fn a_copy_pass_is_exactly_one_read_and_one_write_and_nothing_else() {
        assert!(copy_plan().check().is_ok(), "{:?}", copy_plan().check());

        let mut pass = copy_plan();
        pass.passes[0].reads.push("dst".to_string());
        let err = pass.check().expect_err("两读 ⇒ 拒");
        assert!(err.contains("恰好一读一写"), "{err}");

        let mut pass = copy_plan();
        pass.passes[0].writes.push("src".to_string());
        let err = pass.check().expect_err("两写 ⇒ 拒");
        assert!(err.contains("恰好一读一写"), "{err}");

        let mut pass = copy_plan();
        pass.passes[0].render.color = Attachment::Clear(Color::new(0.0, 0.0, 0.0, 1.0));
        let err = pass.check().expect_err("挂了颜色附件 ⇒ 拒");
        assert!(err.contains("color=none"), "要指出那一栏该怎么写：{err}");

        let mut pass = copy_plan();
        pass.passes[0].render.depth = Attachment::Clear(0.0);
        pass.passes[0].depth_target = Some("src".to_string());
        let err = pass.check().expect_err("挂了深度附件 ⇒ 拒");
        assert!(err.contains("depth=none"), "要指出那一栏该怎么写：{err}");

        let mut pass = copy_plan();
        pass.passes[0].draws = vec![draw("planet", "white")];
        let err = pass.check().expect_err("copy 带 draws ⇒ 拒");
        assert!(err.contains("draws"), "{err}");

        let mut pass = copy_plan();
        pass.passes[0].shader = "fragment".to_string();
        let err = pass.check().expect_err("copy 带 shader ⇒ 拒");
        assert!(err.contains("不建管线"), "{err}");

        let mut pass = copy_plan();
        pass.passes[0].vertex_shader = TRIANGLE_VERTEX.to_string();
        let err = pass.check().expect_err("copy 带顶点阶段 ⇒ 拒");
        assert!(err.contains("vertex_shader"), "{err}");

        let mut pass = copy_plan();
        pass.passes[0].params = vec![0; 16];
        let err = pass.check().expect_err("copy 带参数块 ⇒ 拒");
        assert!(err.contains("绑定组"), "{err}");

        let mut pass = copy_plan();
        pass.passes[0].writes = vec!["src".to_string()];
        let err = pass.check().expect_err("读写同一张 ⇒ 拒");
        assert!(err.contains("同一张图"), "{err}");
    }

    #[test]
    fn a_copy_needs_the_declared_usage_on_both_ends() {
        let mut pass = copy_plan();
        pass.resources[0].usage = vec![Use::RenderAttachment];
        let err = pass.check().expect_err("源没有 copy_src ⇒ 拒");
        assert!(err.contains("copy_src"), "{err}");
        assert!(err.contains("resources"), "要指出去哪一栏加：{err}");

        let mut pass = copy_plan();
        pass.resources[1].usage = vec![Use::RenderAttachment];
        let err = pass.check().expect_err("目标没有 copy_dst ⇒ 拒");
        assert!(err.contains("copy_dst"), "{err}");

        let mut pass = copy_plan();
        pass.resources[1].format = Format::Rgba16Float;
        let err = pass.check().expect_err("格式不同 ⇒ 拒");
        assert!(err.contains("格式不同"), "{err}");

        let mut pass = copy_plan();
        pass.resources[1].size = SizeRule::Half;
        let err = pass.check().expect_err("尺寸规则不同 ⇒ 拒");
        assert!(err.contains("尺寸规则"), "{err}");
    }

    #[test]
    fn a_seeded_texture_must_match_the_declared_spec_and_be_used() {
        let (device, _queue) = test_device();
        let mut executor = Executor::new();
        let resource = ResourceSpec {
            layers: 1,
            name: "depth".to_string(),
            format: Format::Depth32Float,
            size: SizeRule::View,
            usage: vec![Use::RenderAttachment, Use::CopySrc],
        };
        let make = |usage: TextureUsages, format: TextureFormat| {
            device.create_texture(&TextureDescriptor {
                label: Some("seed 判据"),
                size: Extent3d {
                    width: SIDE,
                    height: SIDE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: GpuDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };

        let wrong_format = make(
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            TextureFormat::Rgba8UnormSrgb,
        );
        let err = executor
            .seed(&resource, SIDE, SIDE, wrong_format)
            .expect_err("格式对不上 ⇒ 拒");
        assert!(err.contains("格式"), "{err}");

        let missing_usage = make(
            TextureUsages::RENDER_ATTACHMENT,
            TextureFormat::Depth32Float,
        );
        let err = executor
            .seed(&resource, SIDE, SIDE, missing_usage)
            .expect_err("用途盖不住 ⇒ 拒");
        assert!(err.contains("copy_src"), "{err}");
        assert!(err.contains("用途"), "{err}");

        let good = make(
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            TextureFormat::Depth32Float,
        );
        let err = executor
            .seed(&resource, SIDE + 1, SIDE, good.clone())
            .expect_err("尺寸对不上 ⇒ 拒");
        assert!(err.contains("尺寸"), "{err}");

        executor
            .seed(&resource, SIDE, SIDE, good)
            .expect("规格对得上就该收下");
        let other = Plan {
            layout: Layout::default(),
            resources: vec![resource.clone()],
            passes: vec![PassPlan {
                kind: PassKind::Copy,
                label: "copy".to_string(),
                reads: vec!["depth".to_string()],
                writes: vec!["other".to_string()],
                render: RenderState::parse(
                    "color=none|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
                )
                .expect("copy 的状态"),
                ..Default::default()
            }],
        };
        let frame = Frame {
            zero_dummy: None,
            width: SIDE,
            height: SIDE,
            viewport: None,
            sets: &[],
            geometries: &[],
            materials: &[],
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass seed 判据"),
        });
        let err = executor
            .execute(&device, &mut encoder, &other, &frame)
            .expect_err("seed 了却没人用 ⇒ 拒");
        assert!(err.contains("depth"), "要报出 seed 的名字：{err}");
        assert!(
            err.contains("resources") || err.contains("声明"),
            "要报出计划声明过的资源：{err}"
        );
    }

    #[test]
    fn a_copy_pass_really_moves_a_depth_texture() {
        let (device, queue) = test_device();
        let mut executor = Executor::new();

        let target = device.create_texture(&TextureDescriptor {
            label: Some("px_pass 判据 copy 目标"),
            size: Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass 判据 tint 布局"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let material = test_material(&device, &layout, [1.0, 1.0, 1.0, 1.0]);
        let materials = vec![resolved_material("white", &material, &layout, Cull::None)];

        let near = vertex_buffer(&device, 0.5);
        let far = vertex_buffer(&device, 0.2);
        let indices = index_buffer(&device);
        let attribute = TINT_ATTRIBUTES;
        let geometry_layout = || VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attribute,
        };
        let geometries = vec![
            ResolvedGeometry {
                name: "near",
                vertices: Some((&near, geometry_layout())),
                indices: Some((&indices, IndexFormat::Uint32, 3)),
                vertex_count: 3,
                instances: 0..1,
            },
            ResolvedGeometry {
                name: "far",
                vertices: Some((&far, geometry_layout())),
                indices: Some((&indices, IndexFormat::Uint32, 3)),
                vertex_count: 3,
                instances: 0..1,
            },
        ];

        let depth_state =
            "color=none|depth=clear(0)|depth_write=true|compare=greater_equal|winding=ccw";
        let test_state =
            "color=clear(0,1,0,1)|depth=load|depth_write=false|compare=greater_equal|winding=ccw";
        let depth_only = |label: &str, geometry: &str, depth_target: &str| PassPlan {
            kind: PassKind::Geometry,
            label: label.to_string(),
            vertex_shader: TRIANGLE_VERTEX.to_string(),
            vertex_entry: "vs_main".to_string(),
            writes: Vec::new(),
            draws: vec![draw(geometry, "white")],
            render: RenderState::parse(depth_state).expect("深度-only 的状态"),
            depth_target: Some(depth_target.to_string()),
            ..Default::default()
        };
        let test = |depth_target: &str| PassPlan {
            kind: PassKind::Geometry,
            label: "test".to_string(),
            vertex_shader: TRIANGLE_VERTEX.to_string(),
            vertex_entry: "vs_main".to_string(),
            writes: vec!["out".to_string()],
            draws: vec![draw("far", "white")],
            render: RenderState::parse(test_state).expect("主 pass 的状态"),
            depth_target: Some(depth_target.to_string()),
            ..Default::default()
        };
        let copy = PassPlan {
            kind: PassKind::Copy,
            label: "copy".to_string(),
            reads: vec!["depth".to_string()],
            writes: vec!["depth_copy".to_string()],
            render: RenderState::parse(
                "color=none|depth=none|depth_write=true|compare=greater_equal|winding=ccw",
            )
            .expect("copy 的状态"),
            ..Default::default()
        };
        let depth_resources = || {
            vec![
                ResourceSpec {
                    layers: 1,
                    name: "depth".to_string(),
                    format: Format::Depth32Float,
                    size: SizeRule::View,
                    usage: vec![Use::RenderAttachment, Use::CopySrc],
                },
                ResourceSpec {
                    layers: 1,
                    name: "depth_copy".to_string(),
                    format: Format::Depth32Float,
                    size: SizeRule::View,
                    usage: vec![Use::RenderAttachment, Use::CopyDst, Use::TextureBinding],
                },
            ]
        };

        let with_copy = Plan {
            layout: Layout::default(),
            resources: depth_resources(),
            passes: vec![
                depth_only("write", "near", "depth"),
                copy.clone(),
                test("depth_copy"),
            ],
        };
        let view = target.create_view(&TextureViewDescriptor::default());
        let sets = vec![
            Vec::new(),
            Vec::new(),
            vec![External {
                name: "out",
                role: Role::Write,
                view: &view,
                format: TextureFormat::Rgba8UnormSrgb,
            }],
        ];
        let (inside, outside) = run_case_with_sets(
            &device,
            &queue,
            &mut executor,
            &with_copy,
            &target,
            &geometries,
            &materials,
            &sets,
        );
        assert_eq!(
            (inside, outside),
            (GREEN, GREEN),
            "拷贝生效 ⇒ 更远的三角（0.2）过不了拷贝过来的深度（0.5）"
        );

        let mut fresh = Executor::new();
        let without_copy = Plan {
            layout: Layout::default(),
            resources: depth_resources(),
            passes: vec![depth_only("write", "near", "depth"), test("depth_copy")],
        };
        let sets = vec![
            Vec::new(),
            vec![External {
                name: "out",
                role: Role::Write,
                view: &view,
                format: TextureFormat::Rgba8UnormSrgb,
            }],
        ];
        let (inside, outside) = run_case_with_sets(
            &device,
            &queue,
            &mut fresh,
            &without_copy,
            &target,
            &geometries,
            &materials,
            &sets,
        );
        assert_eq!(
            (inside, outside),
            (WHITE, GREEN),
            "没有拷贝时那张深度是全 0 ⇒ 三角照画（这条对照证明上面那个绿是拷贝造成的）"
        );
    }

    const TRIANGLE_VERTEX: &str = r#"
struct Out {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> Out {
    var out: Out;
    out.position = vec4<f32>(position, 1.0);
    return out;
}
"#;

    const TINT_FRAGMENT: &str = r#"
struct Tint {
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> tint: Tint;

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return tint.color;
}
"#;

    const TINT_ATTRIBUTES: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![0 => Float32x3];
    const SIDE: u32 = 8;
    const INSIDE: (u32, u32) = (4, 4);
    const OUTSIDE: (u32, u32) = (0, 0);
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const CYAN: [u8; 4] = [0, 255, 255, 255];
    const BLACK: [u8; 4] = [0, 0, 0, 255];

    const STATE_BASE: &str =
        "color=clear(1,0,0,1)|depth=none|depth_write=true|compare=greater_equal|winding=ccw";

    struct TestMaterial {
        _buffer: Buffer,
        bind_group: BindGroup,
    }

    fn test_device() -> (Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::VULKAN,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .unwrap_or_else(|err| {
            panic!(
                "后端断言失败：这台机器上拿不到 Vulkan 适配器（{err}）。\
                 几何判据必须真跑 —— 它要一个能用的 Vulkan 驱动；\
                 这一档**不静默跳过**（同 `px_render::gpu::connect`）"
            )
        });
        let info = adapter.get_info();
        assert_eq!(
            info.backend,
            wgpu::Backend::Vulkan,
            "这一档只认 Vulkan（与产品同一条约束）"
        );
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("px_pass 判据设备"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .unwrap_or_else(|err| {
            panic!(
                "后端断言失败：Vulkan 适配器有了（{}）但设备建不出来：{err}",
                info.name
            )
        });
        println!("判据设备：{:?}｜{}", info.backend, info.name);
        (device, queue)
    }

    fn vertex_buffer(device: &Device, z: f32) -> Buffer {
        let corners = [(-0.6_f32, -0.6_f32), (0.6, -0.6), (0.0, 0.6)];
        let mut data: Vec<u8> = Vec::with_capacity(3 * 12);
        for (x, y) in corners {
            for value in [x, y, z] {
                data.extend_from_slice(&value.to_le_bytes());
            }
        }
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass 判据顶点"),
            usage: BufferUsages::VERTEX,
            contents: &data,
        })
    }

    fn index_buffer(device: &Device) -> Buffer {
        let mut data: Vec<u8> = Vec::with_capacity(3 * 4);
        for index in [0_u32, 1, 2] {
            data.extend_from_slice(&index.to_le_bytes());
        }
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass 判据索引"),
            usage: BufferUsages::INDEX,
            contents: &data,
        })
    }

    const TWO_CLASS_VERTEX: &str = r#"
struct PassView {
    view_proj: mat4x4<f32>,
}

struct MeshInstance {
    world_from_local: mat4x4<f32>,
    normal: mat3x3<f32>,
}

@group(1) @binding(0) var<uniform> pass_view: PassView;
@group(1) @binding(1) var<storage, read> mesh: array<MeshInstance>;

struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) tint: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @builtin(instance_index) instance_index: u32,
) -> Out {
    var out: Out;
    out.position = vec4<f32>(position, 1.0);
    out.tint = vec4<f32>(
        0.0,
        mesh[instance_index].world_from_local[3].x,
        pass_view.view_proj[3].x,
        1.0,
    );
    return out;
}
"#;

    const TWO_CLASS_FRAGMENT: &str = r#"
@fragment
fn fs_main(@location(0) tint: vec4<f32>) -> @location(0) vec4<f32> {
    return tint;
}
"#;

    fn two_class_layout(device: &Device) -> BindGroupLayout {
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass 判据：组 1（PassView + 实例数组）"),
            entries: &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    fn instance_array(device: &Device, values: &[f32]) -> Buffer {
        assert!(!values.is_empty(), "实例数组至少一格");
        let mut data = vec![0_u8; values.len() * 112];
        for (index, value) in values.iter().enumerate() {
            data[index * 112 + 48..index * 112 + 52].copy_from_slice(&value.to_le_bytes());
        }
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass 判据实例数组"),
            usage: BufferUsages::STORAGE,
            contents: &data,
        })
    }

    struct TwoClass {
        _view: Buffer,
        bind_group: BindGroup,
    }

    fn two_class(
        device: &Device,
        layout: &BindGroupLayout,
        instances: &Buffer,
        super_value: f32,
    ) -> TwoClass {
        let mut data = vec![0_u8; 64];
        data[48..52].copy_from_slice(&super_value.to_le_bytes());
        let view = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass 判据 PassView"),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            contents: &data,
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("px_pass 判据组 1"),
            layout,
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: view.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: instances.as_entire_binding(),
                },
            ],
        });
        TwoClass {
            _view: view,
            bind_group,
        }
    }

    fn two_class_material<'a>(
        name: &'a str,
        group: &'a TwoClass,
        layout: &BindGroupLayout,
    ) -> ResolvedMaterial<'a> {
        ResolvedMaterial {
            name,
            groups: vec![ResolvedGroup {
                group: 1,
                bind_group: &group.bind_group,
                layout: layout.clone(),
                layout_id: 1,
                dynamic: false,
                dynamic_offset: 0,
            }],
            blend: None,
            cull: Cull::None,
            fragment_shader: TWO_CLASS_FRAGMENT,
            fragment_entry: "fs_main",
        }
    }

    fn corner_vertex_buffer(device: &Device) -> Buffer {
        let corners = [(-1.0_f32, 1.0_f32), (-0.5, 1.0), (-1.0, 0.5)];
        let mut data: Vec<u8> = Vec::with_capacity(3 * 12);
        for (x, y) in corners {
            for value in [x, y, 0.0] {
                data.extend_from_slice(&value.to_le_bytes());
            }
        }
        device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass 判据顶点（角上）"),
            usage: BufferUsages::VERTEX,
            contents: &data,
        })
    }

    fn test_material(device: &Device, layout: &BindGroupLayout, color: [f32; 4]) -> TestMaterial {
        let mut bytes: Vec<u8> = Vec::with_capacity(16);
        for value in color {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        let buffer = device.create_buffer_init(&BufferInitDescriptor {
            label: Some("px_pass 判据 tint"),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            contents: &bytes,
        });
        let bind_group = device.create_bind_group(&BindGroupDescriptor {
            label: Some("px_pass 判据 tint 组"),
            layout,
            entries: &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: None,
                }),
            }],
        });
        TestMaterial {
            _buffer: buffer,
            bind_group,
        }
    }

    fn resolved_material<'a>(
        name: &'a str,
        material: &'a TestMaterial,
        layout: &BindGroupLayout,
        cull: Cull,
    ) -> ResolvedMaterial<'a> {
        ResolvedMaterial {
            name,
            groups: vec![ResolvedGroup {
                group: 0,
                bind_group: &material.bind_group,
                layout: layout.clone(),
                layout_id: 1,
                dynamic: false,
                dynamic_offset: 0,
            }],
            blend: None,
            cull,
            fragment_shader: TINT_FRAGMENT,
            fragment_entry: "fs_main",
        }
    }

    fn resolved_material_depth_only<'a>(
        name: &'a str,
        material: &'a TestMaterial,
        layout: &BindGroupLayout,
    ) -> ResolvedMaterial<'a> {
        ResolvedMaterial {
            fragment_shader: "",
            fragment_entry: "",
            ..resolved_material(name, material, layout, Cull::None)
        }
    }

    fn geometry_plan(label: &str, state: &str, draws: Vec<Draw>) -> Plan {
        let render = RenderState::parse(state).unwrap_or_else(|err| panic!("状态文本：{err}"));
        let depth_target = match render.depth {
            Attachment::None => None,
            _ => Some("depth".to_string()),
        };
        Plan {
            layout: Layout::default(),
            resources: vec![ResourceSpec {
                layers: 1,
                name: "depth".to_string(),
                format: Format::Depth32Float,
                size: SizeRule::View,
                usage: vec![Use::RenderAttachment],
            }],
            passes: vec![PassPlan {
                kind: PassKind::parse("geometry").expect("geometry 是我们自己写的"),
                label: label.to_string(),
                vertex_shader: TRIANGLE_VERTEX.to_string(),
                vertex_entry: "vs_main".to_string(),
                writes: vec!["out".to_string()],
                draws,
                render,
                depth_target,
                ..Default::default()
            }],
        }
    }

    fn draw(geometry: &str, material: &str) -> Draw {
        Draw {
            geometry: geometry.to_string(),
            material: material.to_string(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run_case(
        device: &Device,
        queue: &wgpu::Queue,
        executor: &mut Executor,
        plan: &Plan,
        target: &Texture,
        geometries: &[ResolvedGeometry<'_>],
        materials: &[ResolvedMaterial<'_>],
    ) -> ([u8; 4], [u8; 4]) {
        let view = target.create_view(&TextureViewDescriptor::default());
        let sets = vec![vec![External {
            name: "out",
            role: Role::Write,
            view: &view,
            format: TextureFormat::Rgba8UnormSrgb,
        }]];
        run_case_with_sets(
            device, queue, executor, plan, target, geometries, materials, &sets,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_case_with_sets(
        device: &Device,
        queue: &wgpu::Queue,
        executor: &mut Executor,
        plan: &Plan,
        target: &Texture,
        geometries: &[ResolvedGeometry<'_>],
        materials: &[ResolvedMaterial<'_>],
        sets: &[Vec<External<'_>>],
    ) -> ([u8; 4], [u8; 4]) {
        let frame = Frame {
            zero_dummy: None,
            width: SIDE,
            height: SIDE,
            viewport: None,
            sets,
            geometries,
            materials,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据"),
        });
        let audit = executor
            .execute(device, &mut encoder, plan, &frame)
            .unwrap_or_else(|err| panic!("execute 失败：{err}"));
        println!("{audit}");
        read_back(device, queue, encoder, target)
    }

    fn read_back(
        device: &Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        target: &Texture,
    ) -> ([u8; 4], [u8; 4]) {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_pass 判据回读"),
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * u64::from(SIDE),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(SIDE),
                },
            },
            Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = readback.slice(..);
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
        let pixel = |x: u32, y: u32| {
            let at = (y * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT + x * 4) as usize;
            [data[at], data[at + 1], data[at + 2], data[at + 3]]
        };
        let inside = pixel(INSIDE.0, INSIDE.1);
        let outside = pixel(OUTSIDE.0, OUTSIDE.1);
        drop(data);
        readback.unmap();
        (inside, outside)
    }

    #[test]
    fn a_geometry_pass_draws_reads_back_and_culls() {
        let (device, queue) = test_device();
        let mut executor = Executor::new();

        let target = device.create_texture(&TextureDescriptor {
            label: Some("px_pass 判据目标"),
            size: Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let near = vertex_buffer(&device, 0.75);
        let far = vertex_buffer(&device, 0.25);
        let indices = index_buffer(&device);
        let vertex_layout = VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TINT_ATTRIBUTES,
        };
        let geometries = [
            ResolvedGeometry {
                name: "near",
                vertices: Some((&near, vertex_layout.clone())),
                indices: None,
                vertex_count: 3,
                instances: 0..1,
            },
            ResolvedGeometry {
                name: "far",
                vertices: Some((&far, vertex_layout.clone())),
                indices: Some((&indices, IndexFormat::Uint32, 3)),
                vertex_count: 3,
                instances: 0..1,
            },
        ];

        let tint_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass 判据材质布局"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let white = test_material(&device, &tint_layout, [1.0, 1.0, 1.0, 1.0]);
        let green = test_material(&device, &tint_layout, [0.0, 1.0, 0.0, 1.0]);
        let materials = [
            resolved_material("white", &white, &tint_layout, Cull::None),
            resolved_material("green", &green, &tint_layout, Cull::None),
        ];

        let baseline = geometry_plan("baseline", STATE_BASE, vec![draw("near", "white")]);
        let (inside, outside) = run_case(
            &device,
            &queue,
            &mut executor,
            &baseline,
            &target,
            &geometries,
            &materials,
        );
        assert_eq!(inside, WHITE, "三角形里应当是材质给的白色");
        assert_eq!(outside, RED, "三角形外应当是清屏色");
        assert_ne!(inside, outside, "三角内外必须不同（否则这一档什么都没画）");

        let cull_state =
            "color=clear(1,0,0,1)|depth=none|depth_write=true|compare=greater_equal|winding=ccw";
        let back = geometry_plan("cull-back", cull_state, vec![draw("near", "white-back")]);
        let front = geometry_plan("cull-front", cull_state, vec![draw("near", "white-front")]);
        let cull_materials = [
            resolved_material("white-back", &white, &tint_layout, Cull::Back),
            resolved_material("white-front", &white, &tint_layout, Cull::Front),
        ];
        let (back_inside, back_outside) = run_case(
            &device,
            &queue,
            &mut executor,
            &back,
            &target,
            &geometries,
            &cull_materials,
        );
        let (front_inside, front_outside) = run_case(
            &device,
            &queue,
            &mut executor,
            &front,
            &target,
            &geometries,
            &cull_materials,
        );
        println!("剔除：cull=back ⇒ 里 {back_inside:?}｜cull=front ⇒ 里 {front_inside:?}");
        assert_eq!(back_outside, RED);
        assert_eq!(front_outside, RED);
        assert_ne!(
            back_inside, front_inside,
            "剔 back 与剔 front 必须一个画一个不画（两个一样 ⇒ 剔除没生效）"
        );
        assert!(
            (back_inside == WHITE && front_inside == RED)
                || (back_inside == RED && front_inside == WHITE),
            "两档必须恰好一档画出白三角：back={back_inside:?} front={front_inside:?}"
        );
        assert_eq!(
            back_inside, WHITE,
            "剔 back 应当把它留下（它在 NDC 里是正面）"
        );
        assert_eq!(
            front_inside, RED,
            "剔 front 应当把它剔掉（剔掉 ⇒ 只剩清屏色）"
        );

        let depth_state = "color=clear(1,0,0,1)|depth=clear(0)|depth_write=true|compare=greater_equal|winding=ccw";
        let with_depth = geometry_plan(
            "depth-on",
            depth_state,
            vec![draw("near", "white"), draw("far", "green")],
        );
        let (depth_inside, _) = run_case(
            &device,
            &queue,
            &mut executor,
            &with_depth,
            &target,
            &geometries,
            &materials,
        );
        assert_eq!(depth_inside, WHITE, "近的那笔先画，远的那笔应当被深度挡掉");

        let without_depth = geometry_plan(
            "depth-off",
            STATE_BASE,
            vec![draw("near", "white"), draw("far", "green")],
        );
        let (no_depth_inside, _) = run_case(
            &device,
            &queue,
            &mut executor,
            &without_depth,
            &target,
            &geometries,
            &materials,
        );
        assert_eq!(
            no_depth_inside, GREEN,
            "不挂深度时后画的那笔应当盖上去（否则 ③ 的白分不清是深度还是别的）"
        );
        assert_ne!(depth_inside, no_depth_inside, "有没有深度必须是看得出来的");

        let blind = geometry_plan("no-fragment", STATE_BASE, vec![draw("near", "blind")]);
        let blind_materials = [resolved_material_depth_only("blind", &white, &tint_layout)];
        let view = target.create_view(&TextureViewDescriptor::default());
        let sets = vec![vec![External {
            name: "out",
            role: Role::Write,
            view: &view,
            format: TextureFormat::Rgba8UnormSrgb,
        }]];
        let frame = Frame {
            zero_dummy: None,
            width: SIDE,
            height: SIDE,
            viewport: None,
            sets: &sets,
            geometries: &geometries,
            materials: &blind_materials,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据（材质没有片元阶段）"),
        });
        let err = executor
            .execute(&device, &mut encoder, &blind, &frame)
            .expect_err("挂了颜色却没有片元阶段 ⇒ 拒");
        assert!(err.contains("blind"), "要说出是哪份材质：{err}");
        assert!(err.contains("片元"), "{err}");

        let missing = geometry_plan("missing", STATE_BASE, vec![draw("planet", "white")]);
        let view = target.create_view(&TextureViewDescriptor::default());
        let sets = vec![vec![External {
            name: "out",
            role: Role::Write,
            view: &view,
            format: TextureFormat::Rgba8UnormSrgb,
        }]];
        let frame = Frame {
            zero_dummy: None,
            width: SIDE,
            height: SIDE,
            viewport: None,
            sets: &sets,
            geometries: &geometries,
            materials: &materials,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_pass 判据（名字写错）"),
        });
        let err = executor
            .execute(&device, &mut encoder, &missing, &frame)
            .expect_err("几何名解析不到 ⇒ 拒");
        assert!(err.contains("planet"), "{err}");
        assert!(err.contains("near"), "要列出宿主给了哪些名字：{err}");

        let mut shadowed = geometry_plan("shadowed", STATE_BASE, vec![draw("near", "white")]);
        shadowed.resources = vec![ResourceSpec {
            layers: 1,
            name: "out".to_string(),
            format: Format::Rgba8UnormSrgb,
            size: SizeRule::View,
            usage: vec![Use::RenderAttachment],
        }];
        let (inside, outside) = run_case(
            &device,
            &queue,
            &mut executor,
            &shadowed,
            &target,
            &geometries,
            &materials,
        );
        assert_eq!(
            (inside, outside),
            (WHITE, RED),
            "宿主给的外部目标必须顶掉同名的池资源（否则画进了池里那张没人看的图）"
        );
    }

    #[test]
    fn layered_depth_states_that_cannot_be_meant_are_refused() {
        let layered = |layers: u32| ResourceSpec {
            layers,
            name: "shadow".to_string(),
            format: Format::Depth32Float,
            size: SizeRule::View,
            usage: vec![Use::RenderAttachment],
        };
        let pass = |depth_target: &str, layer: Option<u32>| PassPlan {
            kind: PassKind::Geometry,
            label: "shadow".to_string(),
            vertex_shader: TRIANGLE_VERTEX.to_string(),
            vertex_entry: "vs_main".to_string(),
            draws: vec![draw("near", "white")],
            render: RenderState::parse(
                "color=none|depth=clear(0)|depth_write=true|compare=greater_equal|winding=ccw",
            )
            .expect("状态文本"),
            depth_target: Some(depth_target.to_string()),
            layer,
            ..Default::default()
        };
        let plan = |resource: ResourceSpec, pass: PassPlan| Plan {
            layout: Layout::default(),
            resources: vec![resource],
            passes: vec![pass],
        };

        let err = plan(layered(6), pass("shadow", None))
            .check()
            .expect_err("6 层的图不说写哪一层 ⇒ 拒");
        assert!(err.contains("没说写第几层"), "{err}");
        let err = plan(layered(6), pass("shadow", Some(6)))
            .check()
            .expect_err("第 6 层不存在 ⇒ 拒");
        assert!(err.contains("只有 6 层"), "{err}");
        let err = plan(layered(0), pass("shadow", None))
            .check()
            .expect_err("0 层 ⇒ 拒");
        assert!(err.contains("至少一层"), "{err}");
        let mut outside = plan(layered(1), pass("host_depth", Some(0)));
        outside.passes[0].render = RenderState::parse(
            "color=clear(1,0,0,1)|depth=load|depth_write=true|compare=greater_equal|winding=ccw",
        )
        .expect("状态文本");
        outside.passes[0].depth_target = Some("host_depth".to_string());
        outside.passes[0].writes = vec!["out".to_string()];
        let err = outside.check().expect_err("外部目标上分层 ⇒ 拒");
        assert!(err.contains("外部目标"), "{err}");
        let mut single = plan(layered(1), pass("shadow", Some(0)));
        single.passes[0].layer = Some(3);
        let err = single.check().expect_err("单层图上写第 3 层 ⇒ 拒");
        assert!(err.contains("只有 1 层"), "{err}");
    }

    #[test]
    fn a_pass_writes_exactly_the_layer_it_names() {
        let (device, queue) = test_device();
        let mut executor = Executor::new();

        let target = device.create_texture(&TextureDescriptor {
            label: Some("px_pass 判据（分层）目标"),
            size: Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let near = vertex_buffer(&device, 0.75);
        let far = vertex_buffer(&device, 0.25);
        let vertex_layout = VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TINT_ATTRIBUTES,
        };
        let geometries = [
            ResolvedGeometry {
                name: "near",
                vertices: Some((&near, vertex_layout.clone())),
                indices: None,
                vertex_count: 3,
                instances: 0..1,
            },
            ResolvedGeometry {
                name: "far",
                vertices: Some((&far, vertex_layout.clone())),
                indices: None,
                vertex_count: 3,
                instances: 0..1,
            },
        ];
        let tint_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("px_pass 判据（分层）布局"),
            entries: &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let white = test_material(&device, &tint_layout, [1.0, 1.0, 1.0, 1.0]);
        let green = test_material(&device, &tint_layout, [0.0, 1.0, 0.0, 1.0]);
        let materials = [
            resolved_material("white", &white, &tint_layout, Cull::None),
            resolved_material("green", &green, &tint_layout, Cull::None),
        ];

        let layer_pass =
            |label: &str, geometry: &str, material: &str, layer: u32, state: &str| PassPlan {
                kind: PassKind::Geometry,
                label: label.to_string(),
                vertex_shader: TRIANGLE_VERTEX.to_string(),
                vertex_entry: "vs_main".to_string(),
                writes: vec!["out".to_string()],
                draws: vec![draw(geometry, material)],
                render: RenderState::parse(state).expect("状态文本"),
                depth_target: Some("shadow".to_string()),
                layer: Some(layer),
                ..Default::default()
            };
        let plan = Plan {
            layout: Layout::default(),
            resources: vec![ResourceSpec {
                layers: 6,
                name: "shadow".to_string(),
                format: Format::Depth32Float,
                size: SizeRule::View,
                usage: vec![Use::RenderAttachment],
            }],
            passes: vec![
                layer_pass(
                    "face0",
                    "near",
                    "white",
                    0,
                    "color=clear(1,0,0,1)|depth=clear(0)|depth_write=true|compare=greater_equal|winding=ccw",
                ),
                layer_pass(
                    "face0_again",
                    "far",
                    "green",
                    0,
                    "color=load|depth=load|depth_write=true|compare=greater_equal|winding=ccw",
                ),
                layer_pass(
                    "face1",
                    "far",
                    "green",
                    1,
                    "color=load|depth=load|depth_write=true|compare=greater_equal|winding=ccw",
                ),
            ],
        };
        plan.check().expect("这份计划说得通");

        let view = target.create_view(&TextureViewDescriptor::default());
        let external = || {
            vec![External {
                name: "out",
                role: Role::Write,
                view: &view,
                format: TextureFormat::Rgba8UnormSrgb,
            }]
        };
        let sets = vec![external(), external(), external()];
        let (inside, outside) = run_case_with_sets(
            &device,
            &queue,
            &mut executor,
            &plan,
            &target,
            &geometries,
            &materials,
            &sets,
        );
        assert_eq!(
            inside, GREEN,
            "第 1 层是空的（新纹理按规范清成 0）⇒ 远三角画得出来；\
             层号被忽略的话它撞的是第 0 层里那个近三角写下的深度 ⇒ 白"
        );
        assert_eq!(outside, RED, "三角外仍是 pass 0 的清屏色");
    }

    #[test]
    fn the_pass_parameter_and_the_instance_index_both_reach_the_shader() {
        let (device, queue) = test_device();

        let target = device.create_texture(&TextureDescriptor {
            label: Some("px_pass 判据：两类参数"),
            size: Extent3d {
                width: SIDE,
                height: SIDE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: GpuDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let instances = instance_array(&device, &[0.0, 1.0]);
        let layout = two_class_layout(&device);
        let super_one = two_class(&device, &layout, &instances, 1.0);
        let super_zero = two_class(&device, &layout, &instances, 0.0);
        let material = two_class_material;
        let materials = [
            material("super_one", &super_one, &layout),
            material("super_zero", &super_zero, &layout),
        ];

        let middle = vertex_buffer(&device, 0.0);
        let corner = corner_vertex_buffer(&device);
        let vertex_layout = VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &TINT_ATTRIBUTES,
        };

        let state_one =
            "color=clear(1,0,0,1)|depth=none|depth_write=true|compare=greater_equal|winding=ccw";
        let state_zero = "color=load|depth=none|depth_write=true|compare=greater_equal|winding=ccw";
        let plan = Plan {
            layout: Layout::default(),
            resources: Vec::new(),
            passes: vec![
                PassPlan {
                    kind: PassKind::Geometry,
                    label: "one".to_string(),
                    vertex_shader: TWO_CLASS_VERTEX.to_string(),
                    vertex_entry: "vs_main".to_string(),
                    writes: vec!["out".to_string()],
                    draws: vec![draw("middle", "super_one")],
                    render: RenderState::parse(state_one).expect("状态文本"),
                    ..Default::default()
                },
                PassPlan {
                    kind: PassKind::Geometry,
                    label: "zero".to_string(),
                    vertex_shader: TWO_CLASS_VERTEX.to_string(),
                    vertex_entry: "vs_main".to_string(),
                    writes: vec!["out".to_string()],
                    draws: vec![draw("corner", "super_zero")],
                    render: RenderState::parse(state_zero).expect("状态文本"),
                    ..Default::default()
                },
            ],
        };
        plan.check().expect("这份计划说得通");

        let geometries = |middle_instances: std::ops::Range<u32>,
                          corner_instances: std::ops::Range<u32>| {
            vec![
                ResolvedGeometry {
                    name: "middle",
                    vertices: Some((&middle, vertex_layout.clone())),
                    indices: None,
                    vertex_count: 3,
                    instances: middle_instances,
                },
                ResolvedGeometry {
                    name: "corner",
                    vertices: Some((&corner, vertex_layout.clone())),
                    indices: None,
                    vertex_count: 3,
                    instances: corner_instances,
                },
            ]
        };
        let view = target.create_view(&TextureViewDescriptor::default());
        let sets = || {
            vec![
                vec![External {
                    name: "out",
                    role: Role::Write,
                    view: &view,
                    format: TextureFormat::Rgba8UnormSrgb,
                }],
                vec![External {
                    name: "out",
                    role: Role::Write,
                    view: &view,
                    format: TextureFormat::Rgba8UnormSrgb,
                }],
            ]
        };

        let mut executor = Executor::new();
        let (inside, outside) = run_case_with_sets(
            &device,
            &queue,
            &mut executor,
            &plan,
            &target,
            &geometries(1..2, 0..1),
            &materials,
            &sets(),
        );
        assert_eq!(
            inside, CYAN,
            "中点那一笔：super=1（蓝）且实例第 1 格=1（绿）。读到 GREEN ⇒ pass 那一格没到达；\
             读到 BLUE ⇒ 下标没到达（读的永远是第 0 格）；读到 RED ⇒ 这一笔根本没画"
        );
        assert_eq!(
            outside, BLACK,
            "角上那一笔：super=0 且实例第 0 格=0 ⇒ 两个通道都是 0。\
             ⚠ 清屏色是 RED，所以 BLACK **不是**「没画」"
        );

        let mut fresh = Executor::new();
        let (inside, outside) = run_case_with_sets(
            &device,
            &queue,
            &mut fresh,
            &plan,
            &target,
            &geometries(0..1, 1..2),
            &materials,
            &sets(),
        );
        assert_eq!(
            inside, BLUE,
            "区间对调之后中点那笔读第 0 格（绿=0）、super 仍是 1（蓝）—— \
             要是执行器把区间写死成 0..1，这一遍会与上一遍逐位相同"
        );
        assert_eq!(
            outside, GREEN,
            "角上那笔改读第 1 格（绿=1），而它那条 pass 的 super 仍是 0"
        );
    }
}
