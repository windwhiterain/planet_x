use std::collections::HashMap;

use wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu::{
    BindGroup, BindGroupDescriptor, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer, BufferBinding,
    BufferBindingType, BufferUsages, ColorTargetState, ColorWrites, CommandEncoder, Device,
    Extent3d, FragmentState, IndexFormat, LoadOp, MultisampleState, Operations, Origin3d,
    PipelineCompilationOptions, PipelineLayoutDescriptor, PrimitiveState, RenderPassColorAttachment,
    RenderPassDepthStencilAttachment, RenderPassDescriptor, RenderPipeline,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, StoreOp, TexelCopyTextureInfo, Texture, TextureAspect,
    TextureDescriptor, TextureDimension as GpuDimension, TextureFormat, TextureSampleType,
    TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension, VertexBufferLayout,
    VertexState,
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

/// 报错时把"宿主这一帧到底给了哪些名字"列出来：只说"没给这个名字"是不够的，
/// 人要看的是"你给的是哪几个"——拼错与真没给，只有列出来才分得清。
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
    /// 深度附件那一档。**只有这一种深度格式**：这个渲染器是无限 reverse-Z
    /// （`Depth32Float` + `GreaterEqual` + 清 0.0，§110.1），换格式就得同时换比较方向，
    /// 那不是"多一个格式"，是另一套约定。
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
    /// 拷贝的**源**。⚠ 少了它，校验全过、管线全对，**拷贝那一刻才失败**，
    /// 而报错指向纹理创建、不指向这条 pass（§131 之后加的 copy 那一档记着这条）。
    CopySrc,
    /// 拷贝的**目标**。同样必须在文档的 `resources` 那一栏声明出来。
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
    pub usage: Vec<Use>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PassKind {
    /// 一个全屏三角：`draw(0..3)`，顶点由执行器自备（后处理那一类）。
    #[default]
    Fullscreen,
    /// 一串几何：**画什么写在 pass 的 `draws` 里**（按名字），宿主解析成 GPU 句柄。
    Geometry,
    /// **一次搬运**：`reads[0]` → `writes[0]`（`copy_texture_to_texture`）。
    ///
    /// ⚠ 它**不建管线、不开 render pass、不挂附件** —— 一条 copy 只做一件事：把一张图的内容
    /// 搬到另一张。正因如此它不叫 blit：blit 是**画**（全屏三角 + 采样 + 混合），
    /// 而"画"会在路上顺手做别的事（采样、缩放、调色）。名字一旦叫成 blit，
    /// 迟早有人往里塞一个采样或者"顺手缩个放"，那时它就不再是一次搬运了。
    ///
    /// 为什么需要它（§131）：wgpu 不许同一条 pass 里把一张图既当（写的）深度附件、
    /// 又当资源绑进绑定组。于是"既要在主 pass 里对**真的**预通道深度做深度测试、
    /// 又要让大气采样**当时那份**深度"这件事只有一条路：**先搬一份快照**。
    /// 主 pass 照旧挂 `scene_depth`，采样那一方读 `scene_depth_sample`。
    Copy,
    /// 计算。⚠ 这一版执行器**没有** compute（`Plan::check` 当场拒）。
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

// ---------------------------------------------------------------------------
// 附件与固定功能状态：**本 crate 自己的类型**（§121 第 2 件）
//
// 这一节的每个类型都跟着本 crate 已有的那套约定走（见 `Format` / `SizeRule` / `Use`）：
// `parse(&str) -> Result<Self, String>` 把文档里的文本读成值，`name()` 把值写回文本，
// 两者互为反函数 —— `parse(&value.name()) == value` 逐变体测（`RenderState::parse`
// 那一格就是把这条性质穷举一遍）。有了这条性质，"pass 的样子可以由序列化数据配"
// 才是真的，而不是一句愿望。
//
// 为什么不直接把 wgpu 的类型当字段（`wgpu::Color` / `CompareFunction` / `Face` /
// `FrontFace`）：那几种都只有"值"这一半，没有"从文本读回来"的那一半。镜像一份还有第二个
// 好处：**合法取值由我们说了算** —— wgpu 加一档不该悄悄变成文档里多一个合法值。
// 转换（`to_wgpu`）只发生在边缘：建管线、开 render pass、拼附件的那三处。
// ---------------------------------------------------------------------------

/// 颜色：**写进附件的四个数**。
///
/// ⚠ 它是**线性**的：wgpu 的 clear 值不做 sRGB 变换，硬件按目标格式自己编码。
/// 想表达"文档里那个 sRGB 的 0.004"，得先自己转成线性再填进来。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Color {
    /// 透明黑：**颜色附件的默认清屏值**（与搬进数据模型之前写死在执行器里的那个逐位相同）。
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
            channels[0], channels[1], channels[2], channels[3],
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

/// 深度比较函数（镜像 wgpu 的那一个）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Compare {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    /// 缺省档，也是这个渲染器唯一在用的那一档：无限 reverse-Z（近处 1.0、无限远 0.0）。
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

/// 剔除哪一面。字与 `px_protocol::scene::CullMode` 同一套（`none` / `front` / `back`）。
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

/// 哪一边算正面。⚠ 它与 [`Cull`] 是同一件事的两半：绕向反了，被剔掉的就是**近**面
/// （`mesh.rs::outward_winding` 改的正是绕向）。
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

/// 一个附件这一帧怎么来：**不挂** / **清成某个值** / **接着上一次留下的内容**。
///
/// 为什么是枚举而不是 `Option<LoadOp>`：`Clear` 与 `Load` 的分野是"谁负责擦干净"
/// （清屏的那条 pass 与接着画的那条是两种东西），而"根本没挂这个附件"又是第三件事 ——
/// 挤进 `Option` 的同一个 `None` 上，读的人就分不出"不清"与"不挂"了。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Attachment<T> {
    /// 这一帧不挂这个附件。
    None,
    /// 挂上，并清成这个值。
    Clear(T),
    /// 挂上，并用上一次留在这个附件里的内容。
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
                Some(inner) => Ok(Attachment::Clear(inner.trim().parse::<f32>().map_err(
                    |_| format!("深度值不是数：'{inner}'（整串 '{other}'）"),
                )?)),
                None => Err(format!(
                    "认不出深度附件 '{other}'：这一版认 'none' / 'load' / 'clear(0.0)'"
                )),
            },
        }
    }
}

/// 一条 pass 的**附件 + 固定功能状态**。管线与 render pass 都从这一份建（§121.1：乙案）。
///
/// [`Default`] **就是这一版之前的写死行为**：颜色清成透明、不挂深度、不剔除、逆时针为正面。
/// ⚠ 这条等式是**判据**，不是风格：五格锚（§113）就是在"默认状态"下取的，
/// 默认值一变，那些哈希全都要重取。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderState {
    /// 颜色附件。⚠ `Clear` 的那个颜色是**线性**的（见 [`Color`]）。
    pub color: Attachment<Color>,
    /// 深度附件。`Clear` 的那个数就是**深度值本身**（reverse-Z 下远处是 0.0）。
    pub depth: Attachment<f32>,
    /// 挂上深度之后写不写。
    ///
    /// 缺省是**写**：只加一个深度附件、别的一个字不说，想要的显然是"这是一条深度 pass"
    /// （prepass 就是它）；缺省成"不写"的话，那条 pass 会静默地什么都不留下。
    /// 透明档要的是"测但不写"，那种 pass 自己写 `depth_write = false`。
    pub depth_write: bool,
    /// 深度比较函数。缺省 `greater_equal`（reverse-Z）。
    pub compare: Compare,
    /// 正面朝向。⚠ 它**留在 pass 上**（不是漏搬）：这个项目只有**一套**绕向约定
    /// （无限 reverse-Z 那一套，§110），没有"每份材质各自的正面"这回事。
    /// 将来真出现第二种绕向，它才跟着 `cull` 一起搬到材质那一层。
    pub winding: Winding,
}

impl Default for RenderState {
    fn default() -> Self {
        Self {
            color: Attachment::Clear(Color::TRANSPARENT),
            depth: Attachment::None,
            depth_write: true,
            compare: Compare::GreaterEqual,
            winding: Winding::Ccw,
        }
    }
}

/// `RenderState::name` / `parse` 认的五格。**五格都要写全** —— 缺一格就是"没说"，
/// 而"没说"与"用了默认值"在文档里长得一模一样，那种含糊正是要避免的。
///
/// ⚠ 这里**没有 `cull`**：剔除属于材质（见 [`ResolvedMaterial::cull`]），
/// 所以它既不在 pass 的状态里、也不在这串文本里。
const STATE_KEYS: [&str; 5] = ["color", "depth", "depth_write", "compare", "winding"];

impl RenderState {
    /// 写回文本：`color=clear(0,0,0,0)|depth=none|depth_write=true|compare=greater_equal|winding=ccw`。
    pub fn name(&self) -> String {
        format!(
            "color={}|depth={}|depth_write={}|compare={}|winding={}",
            self.color.name(),
            self.depth.name(),
            self.depth_write,
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
            if !STATE_KEYS.contains(&key) {
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
                "depth_write" => {
                    state.depth_write = match value {
                        "true" => true,
                        "false" => false,
                        other => {
                            return Err(format!(
                                "depth_write 只认 'true' 与 'false'，实际是 '{other}'"
                            ))
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

// ---------------------------------------------------------------------------
// 绑定布局是**宿主给的**（§85 的 C 案）
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

/// 一条 pass 要画的一笔：**按名字**说"用哪份几何、哪份材质"。
///
/// ⚠ 这两个名字是**内容**（"planet" / "surface"），不是这个 crate 的概念：`px_pass`
/// 一个字都不认识它们，也不许认识 —— 它把名字原样交给宿主解析（[`Frame::geometries`] /
/// [`Frame::materials`]），解析不到就当场报错。这样"这条 pass 画什么"才是**数据**，
/// 加一条 pass 或换一份几何都不用改执行器。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draw {
    pub geometry: String,
    /// 材质名。**空 = 没有材质**：这一笔只有顶点阶段（深度-only 的那一笔就是这样）。
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
    /// 参数块的字节：宿主按**这份 shader 自己声明的结构体**打好了（与材质同一条路）。
    /// ⚠ 只有全屏 pass 用它（绑定组由执行器造）；几何 pass 的绑定组由宿主解析，
    /// 所以那两栏必须空着，给了就当场拒 —— "给了没人用"就是"说了没做"。
    pub params: Vec<u8>,
    /// `reads[k]` 落在哪一格（`layout.slots` 里的 `binding`）。同样只有全屏 pass 用。
    pub slots: Vec<u32>,
    /// 附件与固定功能状态。缺省 = 这一版之前写在执行器里的那一套（见 [`RenderState`]）。
    pub render: RenderState,
    /// 这条 pass 画什么（**序列化数据**：名字由宿主解析）。全屏 pass 留空 ——
    /// 它的顶点由执行器自备。
    pub draws: Vec<Draw>,
    /// 深度附件用哪张图：`plan.resources` 里的一个名字（走纹理池），
    /// 或者宿主这一帧给的外部目标（`Frame::sets` 里 `Role::Depth` 那一份）。
    ///
    /// ⚠ 与 `render.depth` 一一对应，两边都不许单独出现：挂了深度却不说用哪张图 =
    /// "画到一张没名字的图上"，那种状态说不清，[`Plan::check`] 当场拒。
    pub depth_target: Option<String>,
    /// 几何 pass 的**顶点阶段**（WGSL 全文 + 入口名）。全屏 pass 留空 ⇒ 执行器自备全屏三角。
    ///
    /// 为什么顶点阶段必须由宿主给：内容 shader 是**纯片元**的（pxart 那几份没有 `@vertex`），
    /// 顶点变换是宿主与 Bevy 逐位对齐的那一段（§110）。执行器要是自己写一份"差不多"的，
    /// 两条宿主就会在两套矩阵算法上分岔 —— 而那正是逐字节判据最怕的漂移。
    ///
    /// ⚠ 它挂在 pass 上**只是因为守卫还没响过**：顶点阶段其实由**几何**决定
    /// （oracle 是按 mesh 的顶点布局选它的），而"一条 pass 里的几何恰好共用一套布局"
    /// 是**当前数据的巧合**，不是结构保证。`execute` 里有一条硬守卫：
    /// 同一条 pass 里两笔 draw 的顶点布局不同 ⇒ **当场拒**（不许拿 pass 的顶点阶段
    /// 套到另一套布局上）。它要是哪天响了，这个字段就搬到**几何**那一层 ——
    /// 不是搬到材质：顶点阶段说的是"这份几何**提供**什么"，不是"材质**期望**什么"。
    pub vertex_shader: String,
    pub vertex_entry: String,
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
            // ---- copy 先判（§131）----
            //
            // ⚠ 它**天生就没有附件**（一次搬运不画任何东西），所以"至少要有一个附件"那条
            //    对它不成立；而它的 `writes` 说的是**搬运的目标**，不是颜色目标 ——
            //    "没挂颜色附件却声明了写目标"那条对它同样不成立。顺序反了的话，
            //    一条规规矩矩的 copy 会被这两条拦下，而真正的理由反而说不出口
            //    （与下面 compute 那一段是同一个次序问题）。
            if pass.kind == PassKind::Copy {
                // 附件：一格都不许挂。⚠ 这是**当场拒**，不是"被忽略的字段"——
                //    状态那一栏仍然必填，只是它必须写成"没有附件"那一种。
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
                // ⚠ 读写同一个名字要在**用途那几条之前**判：它是更基本的一条错，
                //    先说"两端不能是同一张"比先说"这一端少了个 copy_dst"更贴切。
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
                // ---- 两端的**规格**：能比的都在这儿比掉 ----
                //
                // ⚠ 只比格式是不够的：尺寸/层数/mip 对不上时，`copy_texture_to_texture`
                //    会在**执行那一刻**才失败，而报错指向纹理创建、不指向这条 pass ——
                //    离病因很远。这里把它们一次比完（池里建出来的纹理一律 1 层 1 级 mip，
                //    所以层数与 mip 是**构造保证**；尺寸与格式是文档里能写的不一样的两样）。
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
                    if !source.usage.contains(&Use::TextureBinding) {
                        // 只搬不给人看是合法的（例如中间快照），所以这里**不要求**它；
                        // 但反过来"谁要读它"那一条在别处管。这里只把该说的说清：
                        // 源不需要 texture_binding，一个用途都不缺才放行。
                    }
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
                    // ⚠ 尺寸规则也要在这里比一遍：`check` 拿不到这一帧的尺寸，
                    //    所以它比的是**规则**（`view` / `half` / `WxH`）。
                    //    真正权威的那一次在 `execute` 里 —— 那时两张纹理都建出来了，
                    //    宽/高/格式/层数/mip 逐格比（见 `copy_texture`）。
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
                // ⚠ 读/写同一个名字在上面判过了（那一条更基本）。
                continue;
            }
            // ---- 附件（§121 第 1 件）----
            //
            // ⚠ compute 先判：它**天生就没有附件**（不画三角形），所以"至少要有一个附件"
            //    那条对它不成立 —— 顺序反了的话，一条规规矩矩的 compute pass 会被
            //    "你不挂附件"拦下，而真正的理由（这一版执行器没有 compute）反而说不出口。
            if pass.kind == PassKind::Compute {
                // 两条都拒，理由不同：data model 先说清"compute 不该有颜色附件"，
                // 再说清"这一版执行器根本没有 compute"。
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
            // ⚠ "一个附件都不挂"与"没有 writes"是**两件事**：附件说"画到哪张图上"，
            //    `writes` 说"文档里哪个名字是它的颜色目标"。深度-only 的 prepass
            //    有附件、没有 writes（它不写颜色），所以这两条判据不能互相顶替。
            if pass.render.color == Attachment::None && pass.render.depth == Attachment::None {
                return Err(format!(
                    "{at} 既不挂颜色也不挂深度：它画到哪儿去？一条 pass 至少要有一个附件"
                ));
            }
            if pass.kind == PassKind::Fullscreen && pass.render.color == Attachment::None {
                return Err(format!(
                    "{at} 的 kind 是 fullscreen，却没挂颜色附件：全屏三角只会写颜色，\
                     不挂颜色就等于什么都没做（要只写深度就该是一条 geometry pass）"
                ));
            }
            // ---- 深度目标：说了清/接，就得说清用哪张图 ----
            //
            // ⚠ 这两格必须成对出现。只挂深度不给名字 = "画到一张没名字的图上"；
            //    只给名字不挂深度 = 声明了一张用不上的图 —— 两种含糊都在这里拒掉。
            match (&pass.render.depth, &pass.depth_target) {
                (Attachment::None, Some(name)) => {
                    return Err(format!(
                        "{at} 没挂深度附件，却给了 depth_target '{name}'：那张图没人用"
                    ))
                }
                (Attachment::None, None) => {}
                (_, None) => {
                    return Err(format!(
                        "{at} 挂了深度附件，却没给 depth_target：深度图从哪来？\
                         （要么是 resources 里一份 depth32float，要么是宿主这一帧给的 Role::Depth）"
                    ))
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
                    }
                }
            }
            // ---- 类型各自的形状 ----
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
                    // 几何 pass 的绑定组由宿主解析（`Frame::materials`），执行器一个都不造：
                    // 参数块 / 格位 / reads 三栏给了也没人用 ⇒ 给了就拒（"说了没做"那一类）。
                    //
                    // ⚠ `reads` 尤其要拒：执行器既解析不了它、也校验不了宿主到底绑了什么，
                    //    留着就是一条**没人验的声明**，迟早烂掉。几何 pass 的纹理绑定
                    //    住在宿主的绑定组里 —— 那是它自己的事，这里不替它记账。
                    // ⚠ `shader` / `entry` 同理：几何 pass 的片元阶段**属于材质**（§129），
                    //    挂在 pass 上只会让一条 pass 里的多种材质共用一支 shader。
                    if !pass.shader.trim().is_empty() || !pass.entry.trim().is_empty() {
                        return Err(format!(
                            "{at} 是 geometry，却给了 shader/entry：片元阶段属于**材质**                             （每个物体一支），几何 pass 的这一栏没人用 ——                              把 shader 挪到材质的 `fragment_shader` 那一格去"
                        ));
                    }
                    if !pass.params.is_empty() || !pass.slots.is_empty() || !pass.reads.is_empty() {
                        return Err(format!(
                            "{at} 是 geometry，却给了 reads（{} 个）/ params（{} 字节）/ \
                             slots（{} 个）：几何 pass 的绑定组由宿主解析（材质名 → 组），\
                             执行器不自己造，这几栏没人用",
                            pass.reads.len(),
                            pass.params.len(),
                            pass.slots.len()
                        ));
                    }
                }
            }
            // 几何 pass 的每一笔都要说清用哪份几何；材质可以空（深度-only 那一笔）。
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
                // 深度-only 的 pass（prepass）：`writes` 必须空着 —— 写目标就是颜色附件，
                // 挂了空的却在 writes 里点名，说明这条 pass 自己也没想清楚画到哪。
                if !pass.writes.is_empty() {
                    return Err(format!(
                        "{at} 没挂颜色附件，却声明了写目标 [{}]：写目标就是颜色附件",
                        pass.writes.join(" / ")
                    ));
                }
                continue;
            }
            let target = pass.target().ok_or_else(|| {
                format!("{at} 没有 writes：它不写任何东西，画了也没人看得见")
            })?;
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
    /// 深度附件那一份。它与"写"分开：同一张图可以是某条 pass 的深度附件，
    /// 而"写颜色"是另一回事（一张深度图永远不是颜色目标）。
    Depth,
}

pub struct External<'a> {
    pub name: &'a str,
    pub role: Role,
    pub view: &'a TextureView,
    pub format: TextureFormat,
}

/// 宿主**解析好的几何**：`Draw::geometry` 那个名字 → GPU 上的缓冲。
///
/// 名字由宿主与文档约定（"planet" / "icosphere"），`px_pass` 只按名字查表。
pub struct ResolvedGeometry<'a> {
    pub name: &'a str,
    /// 顶点缓冲 + 它的布局。`None` = 这一笔没有顶点缓冲：顶点由顶点着色器按
    /// `@builtin(vertex_index)` 现算（天空盒那种三个顶点的全屏三角就是它）。
    /// ⚠ 布局进管线缓存键 —— 换一份交错方式就是另一条管线。
    pub vertices: Option<(&'a Buffer, VertexBufferLayout<'a>)>,
    /// 索引缓冲 + 格式 + 条数。`None` = 不索引：画 `vertex_count` 个顶点。
    pub indices: Option<(&'a Buffer, IndexFormat, u32)>,
    /// 不索引时画几个顶点（索引时以 `indices` 里那个条数为准）。
    pub vertex_count: u32,
}

/// 宿主给的**一组绑定组**：组号 + 句柄 + **它的布局**（外加布局的身份）。
///
/// ⚠ 布局必须一并给：管线的组布局要按它拼，而 `wgpu::BindGroup` **不暴露自己那份布局**
/// （查过 API，只有一个 `as_custom`）。组号也是数据（材质契约里材质在第 3 组），
/// 执行器不认识任何一组叫什么。
pub struct ResolvedGroup<'a> {
    pub group: u32,
    pub bind_group: &'a BindGroup,
    pub layout: BindGroupLayout,
    /// 这份布局的**身份**（宿主给的数）。
    ///
    /// ⚠ 契约有两条，缺一不可：**同一个布局必须给同一个 id，不同的布局必须给不同的 id**。
    /// 它进管线缓存键，而键必须唯一确定被缓存的那条管线 —— 两种布局共用一个键，
    /// 就是 §66.1 那种"一份契约、两个数"：错的那条管线在**别的 pass、别的帧**才炸出来。
    ///
    /// 为什么不让执行器自己算 id：`wgpu::BindGroupLayout` 既没有内容访问器也没有哈希，
    /// 执行器手里没有"这两份布局是不是同一份"的判据；而布局本来就是宿主建的，
    /// 只有它知道。
    pub layout_id: u64,
}

/// 宿主**解析好的材质**：`Draw::material` 那个名字 → 要设的绑定组 + 混合档 + 剔除。
pub struct ResolvedMaterial<'a> {
    pub name: &'a str,
    pub groups: Vec<ResolvedGroup<'a>>,
    /// 混合档。⚠ 取值由**材质契约**那一侧算（`Add` 与 `Premultiplied` 在 Bevy 0.19 里
    /// 是同一档 PREMULTIPLIED_ALPHA_BLENDING），`px_pass` 不替它翻译 ——
    /// 翻译一遍就是"同一个混合档、两处说法"，那颗雷 §110.1 已经踩过一次。
    pub blend: Option<BlendState>,
    /// 剔哪一面。⚠ 它**属于材质，不属于 pass**（§127）：剔除是材质决定的
    /// （oracle 是**每份材质**一条管线，带着那份材质的 cull），而一条 pass 里
    /// 同时有"剔背面"和"两面都画"是常态 —— 环（`cull = none`）与行星（`Back`）
    /// 就在同一条透明 pass 里。放在 pass 上就是"一份契约两处说"（§66.1），
    /// 而环那一档会当场露馅（要么描述错、要么把一条 pass 拆成两条，
    /// 后者会改掉透明排序 —— 那是拿"说不清的顺序"换"说不清的状态"）。
    ///
    /// 没有材质的那一笔（深度-only）取 [`Cull::None`]：没有任何东西声明过它要剔谁，
    /// 那就两面都画 —— 猜一个方向是这里最不该做的事。
    pub cull: Cull,
    /// 片元阶段的 WGSL 全文。**空 = 没有片元阶段**（深度-only 的那一笔就是这样）。
    ///
    /// ⚠ 它属于**材质**（§129），不属于 pass：一份材质就是那支片元 shader
    /// （`surface.wgsl` / `clouds.wgsl` / `atmosphere.wgsl` 就是材质的身份）。
    /// 一条透明 pass 里同时有大气的 `Add`、云的 `Premultiplied`、环的 `Blend`
    /// 三支不同的片元 shader —— 把片元阶段挂在 pass 上，那三支里只能活一支。
    /// 这与 `blend`（§124）、`cull`（§127）是同一条推理，而这一条最强：
    /// 前两个只是**受材质影响**，片元阶段**就是**材质。
    pub fragment_shader: &'a str,
    /// 片元阶段的入口名（`fragment_shader` 非空时才有意义）。
    pub fragment_entry: &'a str,
}

pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub sets: &'a [Vec<External<'a>>],
    /// 这一帧解析好的几何（按名字）。pass 的 `draws` 里点名谁就取谁。
    pub geometries: &'a [ResolvedGeometry<'a>],
    /// 这一帧解析好的材质（按名字）。同上。
    pub materials: &'a [ResolvedMaterial<'a>],
}

struct Pooled {
    width: u32,
    height: u32,
    format: TextureFormat,
    usage: TextureUsages,
    view: TextureView,
    /// 纹理本身。⚠ 视图给不了纹理：`TextureView` 没有父纹理的访问器（查过 API），
    /// 而 `copy_texture_to_texture` 要的正是纹理。所以池子两样都留着 ——
    /// 拷贝那一档就是靠这一格落地的。
    texture: Texture,
}

/// 拷贝时的**面**：深度格式只认 `DepthOnly`（`All` 对纯深度格式是非法的），
/// 其它格式用 `All`。⚠ 这一格写错的症状是"拷贝那一刻才报"，所以它跟着格式一起决定。
fn copy_aspect(format: TextureFormat) -> TextureAspect {
    if format.has_depth_aspect() {
        TextureAspect::DepthOnly
    } else {
        TextureAspect::All
    }
}

/// 文档里声明的用途 → wgpu 的 `TextureUsages`。
///
/// ⚠ 这是**唯一**一处映射：池子建纹理用它，宿主 `seed` 一张纹理时**也用它**
/// （别在宿主里再写一份 —— 那种"一份契约两处说"的漂移只在拷贝那一刻才露头）。
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
    pool: HashMap<String, Pooled>,
    /// 没被 reads 占到的格一律绑它：布局是固定超集，shader 里声明了就一定绑得上。
    fallback: HashMap<Dimension, TextureView>,
    /// 宿主 `seed` 过的名字（见 [`Executor::seed`]）。
    ///
    /// ⚠ 它留在这里是为了 `execute` 能判"seed 了却没人用"：名字对不上（例如
    /// `scene_depth_snapshot` vs `scene_depth_sample`）会让宿主**悄悄** seed 一张
    /// 没人用的纹理，而那条 pass 照样让池子自建真的那张 —— 那是同一个 bug 换条路回来。
    seeded: Vec<String>,
}

impl Executor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 宿主把某份资源的**纹理**预置进池子：这份资源从头到尾就是宿主给的这一张。
    ///
    /// 为什么需要它（§131 之后）：一条 `copy` 的两端要的是**纹理**，而 group 0 要的是
    /// **同一张纹理的视图** —— 一个资源名只能有**一张**纹理，否则就是"copy 写池里那张、
    /// 着色器读宿主那张"这种**一声不吭的错像素**。能让"一张"成立的形状只有
    /// "宿主建、池子照单收下"。所以 `seed` 之后，这个名字在池子里就是那一张 ——
    /// 附件、绑定、拷贝全都是它。
    ///
    /// ⚠ 规格要照**文档声明的**核一遍（格式 / 尺寸 / 层数 / mip / **用途**），对不上当场拒：
    /// 用途那一栏尤其要紧 —— 宿主建纹理时给的用法必须**涵盖**文档声明的那几项，
    /// 少了 `copy_src` / `copy_dst` 就是"校验全过、管线全对，拷贝那一刻才炸"。
    ///
    /// ⚠ "谁被顶掉了要看得见"是 §130 立的规矩：**顶掉这件事由宿主打印**
    /// （`Plan::resources` 是公开的，谁被顶掉宿主知道）。这里不闷着替换。
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
        if texture.depth_or_array_layers() != 1 || texture.mip_level_count() != 1 {
            problems.push(format!(
                "有 {} 层 / {} 级 mip，而池子建出来的一律是 1 层 1 级",
                texture.depth_or_array_layers(),
                texture.mip_level_count()
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

    fn resource_view(
        &mut self,
        device: &Device,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
    ) -> TextureView {
        self.pooled(device, resource, width, height).view.clone()
    }

    /// 池子里那张纹理（没有就按文档声明的规格建一张）。
    ///
    /// ⚠ 复用条件里带了 `usage`：用途是**文档说了算**的，第二帧要是把 `copy_src` 加上了，
    /// 复用第一帧那张就拷不了 —— 那种失败离病因很远，所以用途进复用条件。
    fn resource_texture(
        &mut self,
        device: &Device,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
    ) -> Texture {
        self.pooled(device, resource, width, height).texture.clone()
    }

    /// 该资源的池子条目（必要时新建）。**视图与纹理是同一份** ——
    /// 视图由这张纹理建出来、缓存在一起，所以"同一张图"的两次解析拿到的是**同一个视图对象**
    /// （`TextureView` 的相等是对象相等，`execute` 里那条"读写同一个视图"的守卫靠它）。
    fn pooled(
        &mut self,
        device: &Device,
        resource: &ResourceSpec,
        width: u32,
        height: u32,
    ) -> &Pooled {
        let format = resource.format.to_wgpu();
        let usage = texture_usage(resource);
        let stale = match self.pool.get(&resource.name) {
            Some(pooled) => {
                pooled.width != width
                    || pooled.height != height
                    || pooled.format != format
                    || pooled.usage != usage
            }
            None => true,
        };
        if stale {
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
                    usage,
                    view,
                    texture,
                },
            );
        }
        self.pool.get(&resource.name).expect("刚插进去的")
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

    /// 管线键里"状态"那一半。
    ///
    /// ⚠ 只放**管线真的会用到的**那几格：附件清成什么色不进键（clear 值不是管线状态），
    /// 否则两条只差清屏色的 pass 会各建一条一模一样的管线。
    /// 剔除与混合来自**材质**（`cull` / `blend` 两个入参就是它们）——
    /// 键必须跟着它们走，否则两种剔除档会共用一条管线。
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

    /// 顶点布局进键的那一半（步长 + 每一格的格式与位置）。
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

    /// 全屏那一档的管线（顶点由执行器自备）。
    fn pipeline_fullscreen(
        &mut self,
        device: &Device,
        layout: &Layout,
        pass: &PassPlan,
        format: TextureFormat,
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
            // 全屏三角两面都画：它没有"材质"，也就没有任何东西声明过要剔谁
            // （§127：剔除住在材质那一层）。绕向照旧 CCW。
            primitive: PrimitiveState {
                cull_mode: None,
                front_face: pass.render.winding.to_wgpu(),
                ..Default::default()
            },
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

    /// 几何那一档的管线（§121.1 乙案：**状态只有一个真源**，管线在这里按它建）。
    ///
    /// 键含：顶点 WGSL 内容 + 片元 WGSL 内容 + 两个入口名 + 顶点布局 + 颜色格式
    /// + 状态里管线会用到的每一格 + 混合档 + **每一组的布局身份**（组号 + `layout_id`）。
    ///
    /// ⚠ 布局身份那一格必须在：键**唯一确定**被缓存的那条管线，少一格就是一条键对两条
    /// 管线（§66.1）。`layout_id` 由宿主保证"同布局同 id、异布局异 id"。
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
        // 剔除**只**从材质来：没有材质 ⇒ 两面都画（见 `ResolvedMaterial::cull`）。
        let cull = material.map(|material| material.cull).unwrap_or(Cull::None);
        // 片元阶段同理：**材质就是那支 shader**（§129）。没有材质 ⇒ 没有片元阶段。
        //
        // ⚠ 还有一条：**没有颜色附件 ⇒ 不建片元阶段**（这一笔就是深度-only 的）。
        //    深度预通道那一笔**必须**带着材质（**剔除是从材质来的**，§127）—— 材质带着
        //    片元 shader，而它在 `color=none` 的 pass 上会把 `@location(0)` 写到零个颜色
        //    目标上，wgpu 当场拒建管线（§131 实测）。所以"带材质但不要片元阶段"必须是
        //    可表达的，而它的表达方式就是"这条 pass 没有颜色附件"。
        //    ⚠ 代价说清楚：alpha-mask 那种"没有颜色输出、只为 discard 而存在"的
        //    预通道专用片元 shader，现在**没法表达** —— 那是一个缺口，不是静默行为：
        //    真需要它时，症状是 mask 没生效（画得比该画的满），而不是悄悄画错。
        //    **缺口将来怎么补**（现在不补：`AlphaMode` 里没有 `Mask`，没有任何一档场景用它，
        //    为够不着的分支加机器只会让状态文本再动一次、白白作废已烘的产物）：
        //    表达它的形状是**在状态文本上开一格 per-pass 的选择**，形如
        //    `fragment=auto|material|none`，默认 `auto` = 今天这条规则（有颜色附件才建片元
        //    阶段）；`material` = 无颜色附件也照建材质的片元阶段（为 discard），
        //    `none` = 永远不建。为什么必须是新的一格：现有的字段**任何组合**都表达不了
        //    "这条 pass 没有颜色附件、但我仍要材质的片元阶段" —— 这正是缺口本身。
        let fragment = material
            .map(|material| (material.fragment_shader, material.fragment_entry))
            .filter(|(shader, _)| !shader.trim().is_empty())
            .filter(|_| color_format.is_some());
        // 布局身份按组号排一遍再进键：宿主给组的次序不该改变"这是哪条管线"。
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
            // ⚠ 键里放的是**材质那份**片元 shader：两种材质就是两条管线。
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
        let vertex = device.create_shader_module(ShaderModuleDescriptor {
            label: Some(label.as_str()),
            source: ShaderSource::Wgsl(pass.vertex_shader.as_str().into()),
        });
        // 片元阶段来自**材质**（§129）。没有材质 ⇒ 没有片元阶段（深度-only 的那一笔）。
        // ⚠ 挂了颜色附件却没有片元阶段 = 画不出东西来 ⇒ 当场拒（不是让 wgpu 在建管线时报一个
        // 离现场很远的错）。⚠ 材质**有**片元 shader 却没有颜色目标（alpha-mask 的 discard
        // 就是这种）就照建：片元阶段带**零个**颜色目标，shader 里多出来的 `@location(0)`
        // 由 wgpu 当场拒 —— 那种"prepass 专用片元 shader"该不该有输出，是内容那一侧的事。
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
        let fragment_module = fragment.map(|(shader, _)| {
            device.create_shader_module(ShaderModuleDescriptor {
                label: Some(label.as_str()),
                source: ShaderSource::Wgsl(shader.into()),
            })
        });
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
        plan.check()?;
        if plan.is_empty() {
            return Ok("pass 表是空的：这一帧没有任何 pass".to_string());
        }
        // ⚠ 每一个 seed 过的名字都必须被**这份计划**用到（§131 的后半条）。
        //
        // seed 先发生、计划后到：一个错拼的名字（`scene_depth_snapshot` vs
        // `scene_depth_sample`）会**悄悄** seed 一张没人用的纹理，而那条 pass 照样让池子
        // 自建真的那张 —— 同一个 bug 换条路又回来了。seed 了却没人用 = **宿主与文档对
        // "存在什么"意见不一致**，而那种不一致不许是静默的。
        //
        // "用到"的口径：这个名字出现在某条 pass 的 reads / writes / depth_target 里。
        // 只"声明在 resources 里"不算 —— 声明了没人碰，正是这条要抓的那种不一致。
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
            // ---- copy（§131）：一次搬运。**不建管线、不开 render pass、不挂附件** ----
            //
            // ⚠ 帧序 = 数组顺序：执行器不替它排序，也不为它插入任何别的一步
            //    （"谁先谁后"是帧表说的，不是执行器猜的）。
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

            // ---- 颜色目标：只有**挂了颜色附件的** pass 才有颜色目标 ----
            //
            // ⚠ 深度-only 的 prepass 没有颜色目标，所以 `writes` 也空着 ——
            //    "附件"与"writes"是两件事（见 `Plan::check`），这里按附件那一侧走。
            let color = match pass.render.color {
                Attachment::None => None,
                load => {
                    let target = pass.target().ok_or_else(|| {
                        format!(
                            "第 {index} 条 pass '{}' 挂了颜色附件却没有 writes",
                            pass.label
                        )
                    })?;
                    let (view, format) = self.resolve(
                        device, plan, frame, index, &pass.label, target, Role::Write,
                    )?;
                    Some((view, format, load))
                }
            };

            // ---- 深度目标：名字 → 视图（`plan.resources` 的池，或者宿主给的外部目标）----
            let depth = match pass.render.depth {
                Attachment::None => None,
                load => {
                    let name = pass.depth_target.as_deref().ok_or_else(|| {
                        format!(
                            "第 {index} 条 pass '{}' 挂了深度附件，却没给 depth_target",
                            pass.label
                        )
                    })?;
                    let (view, format) = self.resolve(
                        device, plan, frame, index, &pass.label, name, Role::Depth,
                    )?;
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

            // ---- 全屏那一档才由执行器造绑定组（几何那一档的由宿主解析）----
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

            // ---- 几何那一档：**先把每一笔解析出来**，再开 pass ----
            //
            // ⚠ 顺序要紧：`begin_render_pass` 借走了编码器，中途出错就得先把它丢掉 ——
            //    所以"名字查不到"这类错必须在这一步之前全部报掉。
            let mut draws: Vec<(RenderPipeline, &ResolvedGeometry<'_>, Option<&ResolvedMaterial<'_>>)> =
                Vec::new();
            if !fullscreen {
                let color_format = color.as_ref().map(|(_, format, _)| *format);
                // ⚠ 硬守卫（§129）：同一条 pass 里的几何必须**共用一套顶点布局**。
                //    顶点阶段挂在 pass 上（`PassPlan::vertex_shader`），拿它去套另一套布局
                //    就是错的；而"当前数据恰好共用"不是结构保证。这条守卫要是响了，
                //    那个字段就搬到**几何**那一层（见 `PassPlan::vertex_shader` 的注释）。
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
                    match &shapes {
                        Some((first, first_shape)) if *first_shape != shape => {
                            return Err(format!(
                                "pass '{}' 里两笔 draw 的顶点布局不同：'{}' 是 {first_shape}，                                 '{}' 是 {shape} —— 顶点阶段挂在 pass 上，套到另一套布局上就是错的。                                 一条 pass 只能画同一种布局的几何（§129）",
                                pass.label, first, draw.geometry
                            ))
                        }
                        Some(_) => {}
                        None => shapes = Some((draw.geometry.clone(), shape)),
                    }
                    let pipeline =
                        self.pipeline_geometry(device, pass, color_format, geometry, material)?;
                    draws.push((pipeline, geometry, material));
                }
            }

            // ---- 全屏那一档：管线与绑定组都在开 pass **之前**备好 ----
            //
            // ⚠ 顺序要紧：`begin_render_pass` 借走了编码器，而兜底贴图与绑定组都要用编码器
            //    （兜底白图是拿清屏 pass 写进去的）。所以它们必须在开 pass 之前做完。
            let mut fullscreen_draw = None;
            if fullscreen {
                let pipeline = self.pipeline_fullscreen(
                    device,
                    &layout,
                    pass,
                    color
                        .as_ref()
                        .map(|(_, format, _)| *format)
                        .expect("全屏 pass 的颜色附件由 check 保证"),
                );
                let bind_group =
                    self.bind_group(device, encoder, &layout, &sampler, &pass.params, &bound);
                fullscreen_draw = Some((pipeline, bind_group));
            }

            let pass_label = format!("px_pass {}", pass.label);
            // 附件状态是**数据**（§121 第 1 件）：这里只做"状态 → LoadOp"的翻译，
            // 一个常量都不许再写死 —— 写死的那天，`plan` 说的与实际画的就是两回事。
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
            let descriptor = RenderPassDescriptor {
                label: Some(pass_label.as_str()),
                color_attachments: &color_attachments,
                depth_stencil_attachment: depth_attachment,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            };
            let mut render_pass = encoder.begin_render_pass(&descriptor);
            if let Some((pipeline, bind_group)) = &fullscreen_draw {
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(layout.group, bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            } else {
                for (pipeline, geometry, material) in &draws {
                    render_pass.set_pipeline(pipeline);
                    if let Some(material) = material {
                        for group in &material.groups {
                            render_pass.set_bind_group(group.group, group.bind_group, &[]);
                        }
                    }
                    if let Some((buffer, _)) = &geometry.vertices {
                        render_pass.set_vertex_buffer(0, buffer.slice(..));
                    }
                    match &geometry.indices {
                        Some((buffer, format, count)) => {
                            render_pass.set_index_buffer(buffer.slice(..), *format);
                            render_pass.draw_indexed(0..*count, 0, 0..1);
                        }
                        None => render_pass.draw(0..geometry.vertex_count, 0..1),
                    }
                }
            }
            drop(render_pass);

            audit.push(format!(
                "pass {index} '{}'（{}）读 [{}] 写 '{}'｜颜色 {}｜深度 {}｜画的：{}",
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
                if fullscreen {
                    format!("全屏三角｜参数 {} 字节｜格 {}", pass.params.len(), pass.slots.len())
                } else {
                    pass.draws
                        .iter()
                        .map(|draw| {
                            if draw.material.is_empty() {
                                format!("{}（无材质）", draw.geometry)
                            } else {
                                format!("{}+{}", draw.geometry, draw.material)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" / ")
                }
            ));
        }
        Ok(audit.join("\n"))
    }

    /// 一条 copy 的两端：**都必须是文档声明的资源**（池子里那两张纹理）。
    ///
    /// ⚠ 为什么宿主给的外部目标在这儿不行：`External` 只给一个 `TextureView`
    /// （附件与绑定组要的是它），而 `copy_texture_to_texture` 要的是**纹理本身** ——
    /// 视图没有父纹理的访问器。所以拷贝的两端只能走池子；真要拷到宿主的图上，
    /// 那是"宿主的目标也得按资源声明一遍"的另一件事，不是这里悄悄的第二次解析。
    ///
    /// ⚠ 这里把两张**已经建出来的**纹理逐格比一遍（宽/高/格式/层数/mip）：
    /// `Plan::check` 只能比文档里的**规格**，而这里比的是实物 —— 拷贝那一刻的失败
    /// 离病因太远（报错指向纹理创建），所以宁可在这里当场说清。
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
            // ⚠ **宿主给的外部目标赢**（S2 宿主需要它，§128 之后定下的规则）：
            //    文档把深度声明成池里的资源，而大气的 group 0 binding 20 要**采样同一张**
            //    深度图 —— 那张 bind group 是宿主建的，池里的纹理视图它根本拿不到。
            //    宿主给出同名外部目标，就是明确说"用这一张"，那就用它。
            //
            //    为什么不是"拒"：拒掉的话宿主只剩两条路 —— 自己另建一张深度图
            //    （于是文档声明的资源成了摆设），或者去拆文档（更糟）。
            //    而"宿主顶掉了资源"这件事**必须看得见**：宿主那一侧要自己打出来
            //    （`Plan::resources` 是公开的，谁被顶掉宿主知道）。静默顶掉才是这里
            //    唯一不能接受的。
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

    // -----------------------------------------------------------------------
    // 纯数据：文档 ↔ 值（无 GPU）
    // -----------------------------------------------------------------------

    fn plan_of(passes: Vec<PassPlan>) -> Plan {
        Plan {
            layout: Layout {
                group: 3,
                params_binding: 0,
                params_align: 16,
                slots: vec![Slot {
                    binding: 1,
                    dimension: Dimension::D2,
                }],
            },
            resources: Vec::new(),
            passes,
        }
    }

    /// 一条最普通的全屏后处理 pass（就是这一版之前 `px_render` 造出来的那种）。
    fn fullscreen(label: &str) -> PassPlan {
        PassPlan {
            kind: PassKind::Fullscreen,
            label: label.to_string(),
            shader: "x".to_string(),
            entry: FRAGMENT_ENTRY.to_string(),
            writes: vec!["view".to_string()],
            params: vec![0; 16],
            render: RenderState::default(),
            ..Default::default()
        }
    }

    /// 一条深度-only 的几何 pass（prepass）：颜色不挂、writes 空着、只有深度目标与顶点阶段。
    fn depth_only(label: &str) -> PassPlan {
        PassPlan {
            kind: PassKind::Geometry,
            label: label.to_string(),
            vertex_shader: "vertex".to_string(),
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

    /// **默认状态就是这一版之前的行为** —— 五格锚（§113）是在那套状态上取的，
    /// 所以这不是风格问题，是判据问题：下面几格一个都不许动。
    #[test]
    fn the_default_state_is_exactly_what_the_executor_used_to_hardcode() {
        let state = RenderState::default();
        assert_eq!(state.color, Attachment::Clear(Color::TRANSPARENT));
        assert_eq!(state.depth, Attachment::None);
        assert_eq!(state.winding, Winding::Ccw);
        // 深度这两格在"不挂深度"时看不出效果，但一旦有 pass 只写 `depth: Clear(..)`
        // 就立刻成判据 ⇒ 缺省必须是这个渲染器唯一的那套约定（reverse-Z）。
        assert_eq!(state.compare, Compare::GreaterEqual);
        assert!(state.depth_write, "只加一个深度附件 ⇒ 想要的显然是写");
    }

    /// 既有的全屏 pass **一字不改**也必须过 check（缺省值就是它要的那套）。
    #[test]
    fn an_existing_fullscreen_pass_still_checks_out() {
        let plan = plan_of(vec![fullscreen("grade")]);
        assert!(plan.check().is_ok(), "{:?}", plan.check());
        assert_eq!(plan.passes[0].render, RenderState::default());
    }

    /// `PassPlan::default()` **必须被拒**：否则 `..Default::default()` 会把
    /// "我漏了一个必填字段"从编译错误变成一条静默的、什么都没说的 pass。
    #[test]
    fn a_default_pass_plan_is_refused() {
        let err = plan_of(vec![PassPlan::default()])
            .check()
            .expect_err("全空的 pass ⇒ 拒");
        assert!(!err.is_empty());
        // 每一条必填项都得有人管：空的 kind（fullscreen）缺 shader/入口/参数块。
        assert!(err.contains("shader") || err.contains("参数块"), "{err}");
    }

    // -----------------------------------------------------------------------
    // 往返：每一个变体都要 parse(name(x)) == x
    // -----------------------------------------------------------------------

    /// **穷举**往返。⚠ 断言的是 `parse(name(x)) == x` 本身，不是"parse 没报错" ——
    /// 一个什么文本都收、永远返回同一个变体的 `parse` 也能过后者。
    /// 组合数是 4 × 5 × 2 × 8 × 2 = 640（`cull` 搬去材质那一层之后少了一维）。
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
        // ⚠ 这里没有 `cull` 了：它属于材质（§127），所以不在这一串文本的取值空间里。
        let windings = [Winding::Ccw, Winding::Cw];

        let mut seen_text: Vec<String> = Vec::new();
        let mut count = 0_usize;
        for color in colors {
            for depth in depths {
                for depth_write in [true, false] {
                    for compare in compares {
                        for winding in windings {
                            let state = RenderState {
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
        // 每一档都要在文本里出现过 —— 否则上面那个"穷举"是假的。
        for compare in compares {
            assert!(
                seen_text
                    .iter()
                    .any(|text| text.contains(&format!("compare={}", compare.name()))),
                "{} 没出现在任何一轮里",
                compare.name()
            );
        }
        // `cull` 的每一档也要能往返（它只是搬去了材质那一层，类型还在）。
        for cull in [Cull::None, Cull::Front, Cull::Back] {
            assert_eq!(Cull::parse(cull.name()).unwrap(), cull);
        }
        assert!(seen_text
            .iter()
            .any(|text| text.contains("color=clear(0.004,0.005,0.01,1)")));
        assert!(seen_text.iter().any(|text| text.contains("depth=clear(0.5)")));
    }

    /// 单个枚举的每一档也要能往返（`PassKind` / `Format` / `Use` 同样进文档）。
    #[test]
    fn the_document_enums_round_trip_variant_by_variant() {
        for kind in [
            PassKind::Fullscreen,
            PassKind::Geometry,
            PassKind::Copy,
            PassKind::Compute,
        ] {
            assert_eq!(PassKind::parse(kind.name()).unwrap(), kind, "{}", kind.name());
        }
        for format in [
            Format::Rgba8UnormSrgb,
            Format::Rgba16Float,
            Format::Depth32Float,
        ] {
            assert_eq!(Format::parse(format.name()).unwrap(), format);
        }
        // ⚠ `copy_src` / `copy_dst` 两档必须能往返：它们是 copy 那条 pass 的**前提**
        //    （文档的 resources 那一栏要写得出这两个词，纹理才建得出对应用途）。
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
        // 颜色与深度的文本形状：数值也要逐位回来（f64 / f32 的 Display 是可往返的）。
        let color = Color::new(0.004, 0.005, 0.010, 1.0);
        assert_eq!(Color::parse(&color.name()).unwrap(), color);
        assert_eq!(color.name(), "0.004,0.005,0.01,1");
    }

    /// 文本错的那些：报错要说清**错在哪一格**，而不是"解析失败"。
    #[test]
    fn a_wrong_document_is_refused_with_the_offending_piece_named() {
        let err = RenderState::parse(
            "color=none|depth=none|depth_write=true|compare=nope|winding=ccw",
        )
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

    // -----------------------------------------------------------------------
    // check()：附件、深度目标、类型各自的形状
    // -----------------------------------------------------------------------

    /// 深度-only 的几何 pass 是**合法**的（prepass 就是它）。
    #[test]
    fn a_geometry_pass_may_carry_depth_only() {
        let plan = plan_of(vec![depth_only("prepass")]);
        assert!(plan.check().is_ok(), "{:?}", plan.check());
        assert_eq!(plan.passes[0].render.depth, Attachment::Clear(0.0));
        assert_eq!(plan.passes[0].render.color, Attachment::None);

        // 颜色 + 深度的几何 pass 也合法（不透明那一档就是它）。
        let mut opaque = fullscreen("opaque");
        opaque.kind = PassKind::Geometry;
        opaque.vertex_shader = "vertex".to_string();
        opaque.vertex_entry = "vs_main".to_string();
        opaque.draws = vec![Draw {
            geometry: "planet".to_string(),
            material: "surface".to_string(),
        }];
        opaque.params = Vec::new();
        // 片元阶段属于材质（§129）：几何 pass 上这两栏必须空着。
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

    /// 一个附件都不挂的 pass 不存在：它画到哪儿去？
    #[test]
    fn a_pass_with_no_attachment_at_all_is_refused() {
        let mut pass = fullscreen("nothing");
        pass.render.color = Attachment::None;
        pass.writes = Vec::new();
        let err = plan_of(vec![pass]).check().expect_err("两个附件都不挂 ⇒ 拒");
        assert!(err.contains("附件"), "{err}");
    }

    /// 没挂颜色附件却在 `writes` 里点名 = 这条 pass 自己没想清楚画到哪。
    #[test]
    fn a_pass_with_a_write_target_but_no_color_attachment_is_refused() {
        let mut pass = depth_only("confused");
        pass.writes = vec!["view".to_string()];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("空挂颜色却声明写目标 ⇒ 拒");
        assert!(err.contains("写目标"), "{err}");
    }

    /// 全屏 pass 不挂颜色就等于什么都没做（全屏三角只会写颜色）。
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

    /// compute 不挂附件 —— 这一条**在**"执行器还没有 compute"那条之前，两条各有各的理由。
    #[test]
    fn a_compute_pass_must_not_carry_a_color_attachment() {
        let mut pass = fullscreen("reduce");
        pass.kind = PassKind::Compute;
        let err = plan_of(vec![pass]).check().expect_err("compute 挂了颜色 ⇒ 拒");
        assert!(err.contains("compute"), "{err}");
        assert!(err.contains("颜色附件"), "{err}");
    }

    /// 把颜色摘掉的 compute pass 才会走到"执行器还没有 compute"那条能力理由上。
    #[test]
    fn a_compute_pass_without_attachments_is_refused_for_the_capability_reason() {
        let mut pass = fullscreen("reduce");
        pass.kind = PassKind::Compute;
        pass.render.color = Attachment::None;
        pass.writes = Vec::new();
        let err = plan_of(vec![pass]).check().expect_err("compute ⇒ 拒");
        assert!(err.contains("静默跳过"), "{err}");
    }

    /// 深度目标与 `render.depth` 必须成对：单出一边就是"画到一张没名字的图上"。
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

        // 池里的那份资源要是颜色格式，也当场拒（深度附件只能是 depth32float）。
        let mut plan = plan_of(vec![depth_only("prepass")]);
        plan.resources.push(ResourceSpec {
            name: "depth".to_string(),
            format: Format::Rgba8UnormSrgb,
            size: SizeRule::View,
            usage: vec![Use::RenderAttachment],
        });
        let err = plan.check().expect_err("深度目标不是 depth32float ⇒ 拒");
        assert!(err.contains("depth32float"), "{err}");

        // 正确的深度资源 ⇒ 过。
        plan.resources[0].format = Format::Depth32Float;
        assert!(plan.check().is_ok(), "{:?}", plan.check());
    }

    /// 几何 pass 的形状：顶点阶段必须有；绑定组那两栏（params/slots）不许给。
    #[test]
    fn a_geometry_pass_needs_a_vertex_stage_and_builds_no_bind_group() {
        let mut pass = depth_only("prepass");
        pass.vertex_shader = String::new();
        let err = plan_of(vec![pass]).check().expect_err("没有顶点阶段 ⇒ 拒");
        assert!(err.contains("vertex_shader"), "{err}");

        let mut pass = depth_only("prepass");
        pass.params = vec![0; 16];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("几何 pass 给了参数块 ⇒ 拒");
        assert!(err.contains("宿主解析"), "{err}");

        // `reads` 与 params/slots 同一条理由：执行器解析不了、也验不了 ⇒ 留着就是烂账。
        let mut pass = depth_only("prepass");
        pass.reads = vec!["view".to_string()];
        let err = plan_of(vec![pass])
            .check()
            .expect_err("几何 pass 给了 reads ⇒ 拒");
        assert!(err.contains("reads"), "{err}");
        assert!(err.contains("宿主解析"), "{err}");

        let mut pass = depth_only("prepass");
        pass.draws = vec![Draw::default()];
        let err = plan_of(vec![pass]).check().expect_err("一笔不说画哪份几何 ⇒ 拒");
        assert!(err.contains("geometry"), "{err}");

        // ③ 片元阶段属于**材质**（§129）：几何 pass 给了 shader/entry 就当场拒 ——
        //    挂在 pass 上只会让一条 pass 里的多种材质共用一支 shader。
        let mut pass = depth_only("prepass");
        pass.shader = "fragment".to_string();
        pass.entry = "fs_main".to_string();
        let err = plan_of(vec![pass]).check().expect_err("几何 pass 带片元阶段 ⇒ 拒");
        assert!(err.contains("材质"), "{err}");
        assert!(err.contains("fragment_shader"), "要指出该挪到哪一格：{err}");
    }

    /// 全屏 pass 声明了 draws 就说不清谁说了算（顶点是执行器自备的）。
    #[test]
    fn a_fullscreen_pass_with_draws_is_refused() {
        let mut pass = fullscreen("grade");
        pass.draws = vec![Draw {
            geometry: "planet".to_string(),
            material: "surface".to_string(),
        }];
        let err = plan_of(vec![pass]).check().expect_err("全屏 pass 带 draws ⇒ 拒");
        assert!(err.contains("draws"), "{err}");
    }

    /// 名字查不到时报错要**列出宿主给了哪些名字**（拼错与真没给，只有列出来才分得清）。
    #[test]
    fn an_unresolved_draw_name_lists_the_ones_the_host_did_give() {
        let list = name_list(["planet", "atmosphere"].into_iter());
        assert_eq!(list, "planet / atmosphere");
        assert_eq!(name_list(std::iter::empty()), "（一个都没有）");
    }

    // -----------------------------------------------------------------------
    // copy（§131）：一次搬运。**形状那几条先拒，像素那一条真跑**
    // -----------------------------------------------------------------------

    /// 一条规矩的 copy：两端都在 `resources` 里、用途声明齐、状态写着"没有附件"。
    fn copy_plan() -> Plan {
        Plan {
            layout: Layout::default(),
            resources: vec![
                ResourceSpec {
                    name: "src".to_string(),
                    format: Format::Rgba8UnormSrgb,
                    size: SizeRule::View,
                    usage: vec![Use::RenderAttachment, Use::CopySrc, Use::TextureBinding],
                },
                ResourceSpec {
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

    /// 一条 copy 的形状：**恰好一读一写**、不挂附件、没有 draws/管线两栏/参数两栏。
    /// 每一条都要指出**该去哪一栏**改 —— 只给罪名不够。
    #[test]
    fn a_copy_pass_is_exactly_one_read_and_one_write_and_nothing_else() {
        assert!(copy_plan().check().is_ok(), "{:?}", copy_plan().check());

        // 两端都在，先确认基线没写错。
        let mut pass = copy_plan();
        pass.passes[0].reads.push("dst".to_string());
        let err = pass.check().expect_err("两读 ⇒ 拒");
        assert!(err.contains("恰好一读一写"), "{err}");

        let mut pass = copy_plan();
        pass.passes[0].writes.push("src".to_string());
        let err = pass.check().expect_err("两写 ⇒ 拒");
        assert!(err.contains("恰好一读一写"), "{err}");

        // 挂附件：**当场拒**（不是被忽略的字段）。
        let mut pass = copy_plan();
        pass.passes[0].render.color =
            Attachment::Clear(Color::new(0.0, 0.0, 0.0, 1.0));
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

        // 读写的名字一样：一次搬运的两端不能是同一张图。
        let mut pass = copy_plan();
        pass.passes[0].writes = vec!["src".to_string()];
        let err = pass.check().expect_err("读写同一张 ⇒ 拒");
        assert!(err.contains("同一张图"), "{err}");
    }

    /// `copy_src` / `copy_dst` **必须在文档的 resources 那一栏声明**：少了它们，
    /// 纹理建出来就拷不了，而失败发生在拷贝那一刻（报错指向纹理创建、不指向这条 pass）。
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

        // 格式不同 / 尺寸规则不同：也在这里拒（权威的那一次在 execute 里比实物）。
        let mut pass = copy_plan();
        pass.resources[1].format = Format::Rgba16Float;
        let err = pass.check().expect_err("格式不同 ⇒ 拒");
        assert!(err.contains("格式不同"), "{err}");

        let mut pass = copy_plan();
        pass.resources[1].size = SizeRule::Half;
        let err = pass.check().expect_err("尺寸规则不同 ⇒ 拒");
        assert!(err.contains("尺寸规则"), "{err}");
    }

    /// `seed`（§132）：宿主把某份资源的纹理交进池子 —— 规格照**文档声明的**核，
    /// 而且"seed 了却没人用"要当场拒（那是宿主与文档对"存在什么"意见不一致）。
    #[test]
    fn a_seeded_texture_must_match_the_declared_spec_and_be_used() {
        let (device, _queue) = test_device();
        let mut executor = Executor::new();
        let resource = ResourceSpec {
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

        // ① 格式对不上 ⇒ 拒。
        let wrong_format = make(
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            TextureFormat::Rgba8UnormSrgb,
        );
        let err = executor
            .seed(&resource, SIDE, SIDE, wrong_format)
            .expect_err("格式对不上 ⇒ 拒");
        assert!(err.contains("格式"), "{err}");

        // ② 用途少了 copy_src ⇒ 拒（少了它就是"校验全过、拷贝那一刻才炸"）。
        let missing_usage = make(TextureUsages::RENDER_ATTACHMENT, TextureFormat::Depth32Float);
        let err = executor
            .seed(&resource, SIDE, SIDE, missing_usage)
            .expect_err("用途盖不住 ⇒ 拒");
        assert!(err.contains("copy_src"), "{err}");
        assert!(err.contains("用途"), "{err}");

        // ③ 尺寸对不上 ⇒ 拒。
        let good = make(
            TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            TextureFormat::Depth32Float,
        );
        let err = executor
            .seed(&resource, SIDE + 1, SIDE, good.clone())
            .expect_err("尺寸对不上 ⇒ 拒");
        assert!(err.contains("尺寸"), "{err}");

        // ④ 规格都对 ⇒ 收下；但"seed 了却没人用"要在**执行时**当场拒，
        //    而且要把 seed 的名字与计划声明过的资源名一起报出来。
        executor
            .seed(&resource, SIDE, SIDE, good)
            .expect("规格对得上就该收下");
        // 这条计划合法（check 过得了），但它**一条 pass 都没用到** 'depth' ——
        // 正是"宿主与文档对『存在什么』意见不一致"的形状。
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
            width: SIDE,
            height: SIDE,
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
        assert!(err.contains("resources") || err.contains("声明"), "要报出计划声明过的资源：{err}");
    }

    /// **判据（要真设备）**：一次拷贝真的搬了东西。    ///
    /// 形状：① 一条深度-only 的几何 pass 在盘内写下 z=0.5；② 一次 copy 把那张深度搬到
    /// `depth_copy`；③ 第三条 pass 用 `depth_copy` 做深度测试、画一个**更远**的三角（z=0.2）。
    /// - 拷贝生效 ⇒ `0.2 >= 0.5` 不成立 ⇒ 三角被挡掉 ⇒ 盘内是**清屏色**；
    /// - 拷贝没生效（目标还是一张全 0 的新图）⇒ `0.2 >= 0.0` 成立 ⇒ 盘内是**材质色**。
    /// 对照：把 copy 那一条从计划里拿掉，同一个计划必须画出材质色。
    /// 两条一起看，判据才落在"**搬运**"这件事上，而不是"pass 跑过了"。
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
            },
            ResolvedGeometry {
                name: "far",
                vertices: Some((&far, geometry_layout())),
                indices: Some((&indices, IndexFormat::Uint32, 3)),
                vertex_count: 3,
            },
        ];

        let depth_state = "color=none|depth=clear(0)|depth_write=true|compare=greater_equal|winding=ccw";
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
                    name: "depth".to_string(),
                    format: Format::Depth32Float,
                    size: SizeRule::View,
                    usage: vec![Use::RenderAttachment, Use::CopySrc],
                },
                ResourceSpec {
                    name: "depth_copy".to_string(),
                    format: Format::Depth32Float,
                    size: SizeRule::View,
                    usage: vec![Use::RenderAttachment, Use::CopyDst, Use::TextureBinding],
                },
            ]
        };

        // 有拷贝：三角被挡掉 ⇒ 盘内是清屏色（纯绿）。
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
            &device, &queue, &mut executor, &with_copy, &target, &geometries, &materials, &sets,
        );
        assert_eq!(
            (inside, outside),
            (GREEN, GREEN),
            "拷贝生效 ⇒ 更远的三角（0.2）过不了拷贝过来的深度（0.5）"
        );

        // 对照：没有拷贝 ⇒ 目标是一张全 0 的新图 ⇒ 三角画得出来（纯白在盘内）。
        // ⚠ 换一个**新执行器**：池子是执行器上的，同一个执行器里 `depth_copy` 会**复用**
        //    上一轮那张纹理（还带着 0.5），对照就变成了"拷贝的残留"，测不出东西。
        let mut fresh = Executor::new();
        let without_copy = Plan {
            layout: Layout::default(),
            resources: depth_resources(),
            passes: vec![
                depth_only("write", "near", "depth"),
                test("depth_copy"),
            ],
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

    // -----------------------------------------------------------------------
    // 判据（要真设备）：一条 geometry pass 画进离屏纹理、读回来，
    // 而且**剔除与深度都真的生效** —— 不是"建起来了"就算过。
    // -----------------------------------------------------------------------

    /// 顶点：位置就是**裁剪空间坐标**（这一格不测矩阵，测的是"这一笔到底画没画"）。
    /// z 用 reverse-Z 的语义：大的近、小的远。
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

    /// 片元：颜色来自**材质那一组**（组 0 第 0 格）。这个 shader 里**没有参数块** ——
    /// 它证明几何 pass 的绑定组确实来自宿主解析，而不是执行器自己造的那一组。
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
    /// 两个取样点：`(4,4)` 在三角形里，`(0,0)` 在四个角上（三角形碰不到角）。
    const INSIDE: (u32, u32) = (4, 4);
    const OUTSIDE: (u32, u32) = (0, 0);
    /// 三角写纯白、清屏写纯红、另一笔写纯绿：三个都是**精确**的 8 位值
    /// （线性 0.0/1.0 的 sRGB 编解码恰好落在 0/255，不涉及舍入）。
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];

    const STATE_BASE: &str =
        "color=clear(1,0,0,1)|depth=none|depth_write=true|compare=greater_equal|winding=ccw";

    /// 一份"宿主解析好的材质"**连同它借出去的东西**：缓冲与绑定组都得活到 `execute` 之后。
    struct TestMaterial {
        /// 只为了活着（绑进组里的是它的引用）。
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
            // ⚠ 说清**缺的是什么**：这一档不是"跳过"，是"这台机器现在跑不了"。
            //    （`px_pass` 在 workspace 的 default-members 里 ⇒ `cargo test` 会走到这里。）
            panic!(
                "后端断言失败：这台机器上拿不到 Vulkan 适配器（{err}）。\
                 几何判据必须真跑 —— 它要一个能用的 Vulkan 驱动；\
                 这一档**不静默跳过**（同 `px_render_wgpu::gpu::connect`）"
            )
        });
        let info = adapter.get_info();
        assert_eq!(
            info.backend,
            wgpu::Backend::Vulkan,
            "这一档只认 Vulkan（与产品同一条约束）"
        );
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
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
                // 这一台只有一份材质布局 ⇒ 一个 id。⚠ 契约：同布局同 id、异布局异 id。
                layout_id: 1,
            }],
            blend: None,
            // ⚠ 剔除**在这里**（材质那一层，§127），不在 pass 的状态文本里。
            cull,
            // ⚠ 片元阶段也在这里（§129）：**材质就是那支 shader**。
            //    搬过来之前它是 `PassPlan.shader`，而那时一条 pass 里的多种材质
            //    只能共用一支 —— 这正是这一档要证伪的事。
            fragment_shader: TINT_FRAGMENT,
            fragment_entry: "fs_main",
        }
    }

    /// 一份**没有片元阶段**的材质（深度-only 那一笔要它）。
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

    /// 从**序列化文本**拼出这条 pass：`kind`、状态、每一笔画什么，全是数据。
    /// ⚠ 判据要走的正是这条路 —— 手写结构体只能证明"执行器会画"，
    /// 证明不了"pass 可以由序列化数据配"。
    fn geometry_plan(label: &str, state: &str, draws: Vec<Draw>) -> Plan {
        let render = RenderState::parse(state).unwrap_or_else(|err| panic!("状态文本：{err}"));
        let depth_target = match render.depth {
            Attachment::None => None,
            _ => Some("depth".to_string()),
        };
        Plan {
            layout: Layout::default(),
            resources: vec![ResourceSpec {
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
                // ⚠ 几何 pass **不给** shader/entry：片元阶段属于材质（§129），
                //    给了会被 `check` 拒（"说了没做"那一类）。
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

    /// 跑一条 pass 并回读 `(三角形里, 三角形外)` 两个像素。
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

    /// 同上，但**外部目标表由调用方给**（多 pass 的计划里，"out" 那一份要落在
    /// 真正写它的那条 pass 的下标上 —— 表是按 pass 下标查的）。
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
            width: SIDE,
            height: SIDE,
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

    /// **§121 第 2 件的判据**：一条 geometry pass 走完整条路 ——
    /// `Draw` 里的名字 → 宿主解析的句柄 → 执行器建管线/设组/draw → 离屏纹理 → 回读。
    /// 断言的是**像素**：三角里的白来自材质那一组、清屏的红来自状态、剔除与深度各改一个像素。
    #[test]
    fn a_geometry_pass_draws_reads_back_and_culls() {
        let (device, queue) = test_device();
        let mut executor = Executor::new();

        // ---- 目标：宿主自己的纹理（宿主也要读回来，所以走外部视图那条路）----
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

        // ---- 几何：近的那个**不索引**，远的那个走索引缓冲（两条 draw 路都走一遍）----
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
            },
            ResolvedGeometry {
                name: "far",
                vertices: Some((&far, vertex_layout.clone())),
                indices: Some((&indices, IndexFormat::Uint32, 3)),
                vertex_count: 3,
            },
        ];

        // ---- 材质：布局由宿主建、`layout_id` 也由宿主给（契约见 `ResolvedGroup`）----
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
        // 这两份材质**两面都画**（`Cull::None`）：基线/深度那几档要的是"三角形一定画得出来"。
        let materials = [
            resolved_material("white", &white, &tint_layout, Cull::None),
            resolved_material("green", &green, &tint_layout, Cull::None),
        ];

        // ① 基线：白三角画进红的清屏里。白来自**材质那一组**（片元真的跑了），
        //    红来自**状态里的 clear**（颜色附件真的按数据清了）。
        let baseline = geometry_plan("baseline", STATE_BASE, vec![draw("near", "white")]);
        let (inside, outside) = run_case(
            &device, &queue, &mut executor, &baseline, &target, &geometries, &materials,
        );
        assert_eq!(inside, WHITE, "三角形里应当是材质给的白色");
        assert_eq!(outside, RED, "三角形外应当是清屏色");
        assert_ne!(inside, outside, "三角内外必须不同（否则这一档什么都没画）");

        // ② 剔除：同一个三角形，只把**材质**的 cull 换一档就该有像素消失。
        //    ⚠ 剔除住在材质那一层（§127），所以这两条 pass 的**状态文本一模一样**，
        //    差别只在材质表里那两格 —— 这正是"一份契约一处说"的判据。
        //    ⚠ 绕向在 NDC 里算（**不是**帧缓冲坐标）：顶点给的 (-0.6,-0.6) → (0.6,-0.6) → (0.0,0.6)
        //    在 NDC 里是逆时针 ⇒ `winding=ccw` 认定它是**正面** ⇒ 剔 back 留下、剔 front 剔掉。
        //    这一档的数值是**量出来的**（先是按"帧缓冲 y 朝下"推的，量出来是反的，照量到的钉住）。
        let cull_state =
            "color=clear(1,0,0,1)|depth=none|depth_write=true|compare=greater_equal|winding=ccw";
        let back = geometry_plan("cull-back", cull_state, vec![draw("near", "white-back")]);
        let front = geometry_plan("cull-front", cull_state, vec![draw("near", "white-front")]);
        let cull_materials = [
            resolved_material("white-back", &white, &tint_layout, Cull::Back),
            resolved_material("white-front", &white, &tint_layout, Cull::Front),
        ];
        let (back_inside, back_outside) = run_case(
            &device, &queue, &mut executor, &back, &target, &geometries, &cull_materials,
        );
        let (front_inside, front_outside) = run_case(
            &device, &queue, &mut executor, &front, &target, &geometries, &cull_materials,
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
        // 钉住量出来的那一档：NDC 里逆时针 ⇒ 正面 ⇒ 剔 back 留下、剔 front 剔掉。
        assert_eq!(back_inside, WHITE, "剔 back 应当把它留下（它在 NDC 里是正面）");
        assert_eq!(front_inside, RED, "剔 front 应当把它剔掉（剔掉 ⇒ 只剩清屏色）");

        // ③ 深度：两笔都在同一个像素上，近的（z=0.75）先画、远的（z=0.25）后画。
        //    reverse-Z + `greater_equal` ⇒ 远的那笔**测不过**，像素保持白的。
        //    这就是"深度附件真的挂上了、而且比较方向是对的"的证据。
        let depth_state =
            "color=clear(1,0,0,1)|depth=clear(0)|depth_write=true|compare=greater_equal|winding=ccw";
        let with_depth = geometry_plan(
            "depth-on",
            depth_state,
            vec![draw("near", "white"), draw("far", "green")],
        );
        let (depth_inside, _) = run_case(
            &device, &queue, &mut executor, &with_depth, &target, &geometries, &materials,
        );
        assert_eq!(depth_inside, WHITE, "近的那笔先画，远的那笔应当被深度挡掉");

        // ④ 同一份 draw 表、只是**不挂深度** ⇒ 后画的那笔直接盖上去（绿）——
        //    与 ③ 只差 `depth=` 一格，所以 ③ 的白只可能来自深度测试本身。
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

        // ⑤ 挂了颜色附件、而材质没有片元阶段 ⇒ **当场拒**（运行时守卫）。
        //    它是 §129 搬家之后新出现的一种错：以前片元阶段在 pass 上，永远不会缺；
        //    现在它在材质上，于是"这份材质没写颜色"必须当场说出来，
        //    而不是等 wgpu 在建设备管线时报一个离现场很远的错。
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
            width: SIDE,
            height: SIDE,
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

        // ⑥ 名字写错时**当场报错**（不是静默少画一笔）。
        let missing = geometry_plan("missing", STATE_BASE, vec![draw("planet", "white")]);
        let view = target.create_view(&TextureViewDescriptor::default());
        let sets = vec![vec![External {
            name: "out",
            role: Role::Write,
            view: &view,
            format: TextureFormat::Rgba8UnormSrgb,
        }]];
        let frame = Frame {
            width: SIDE,
            height: SIDE,
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

        // ⑦ 名字**既是文档声明的资源、又是宿主给的外部目标** ⇒ 宿主的赢。
        //    这是 S2 宿主真正需要的那一条：深度声明在文档里，而大气的 group 0
        //    binding 20 要采样同一张深度图，那张 bind group 只有宿主建得出来。
        //    ⚠ 判据落在**像素**上：宿主赢了 ⇒ 三角画进外部目标（外面仍是清屏色）；
        //      池里的资源赢了 ⇒ 外部目标一个像素都不会动。
        let mut shadowed = geometry_plan("shadowed", STATE_BASE, vec![draw("near", "white")]);
        shadowed.resources = vec![ResourceSpec {
            name: "out".to_string(),
            format: Format::Rgba8UnormSrgb,
            size: SizeRule::View,
            usage: vec![Use::RenderAttachment],
        }];
        let (inside, outside) = run_case(
            &device, &queue, &mut executor, &shadowed, &target, &geometries, &materials,
        );
        assert_eq!(
            (inside, outside),
            (WHITE, RED),
            "宿主给的外部目标必须顶掉同名的池资源（否则画进了池里那张没人看的图）"
        );
    }
}
