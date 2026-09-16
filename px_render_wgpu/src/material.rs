//! 材质那一层：**固定超集的 group 3 绑定组 + 管线缓存**（§111 第 5 件）。
//!
//! 这一篇是"内容不认识渲染器、渲染器也不认识内容"那条线的最后一米：
//!
//! - **绑定布局**是一个**固定超集**（参数块 + 契约表里的 12 格贴图/采样器对）。
//!   Bevy 的 `MaterialPlugin` 每种材质类型只建**一份**布局（`AsBindGroup::bind_group_layout_entries`
//!   是静态函数，拿不到 `&self`），所以布局不可能随实例变；跟着的代价是"空着的格也得绑上东西"
//!   ⇒ 空格一律绑兜底白图。⚠ 这不是假想路径：`orbit-bare` 的 `planet` 声明了第 3 格
//!   （`glow`，2D）与第 5 格（`coverage_map`，cube），而产物**一张都没给**（§108.1）。
//! - **状态只有两条**（混合档、剔除档），但它们的值不是我们挑的：`Add` 在 Bevy 0.19 里
//!   **不是加法混合**（见 [`PipelineKey::blend`]），深度是**无限 reverse-Z**（§110.1）。
//!   这两条都属于"按直觉写就会错、而且错了画面只是'有点不一样'"的那一类。
//! - **管线缓存的键**是 `(shader 内容版本, 剔除, 混合)`：管线是不可变状态对象，
//!   换其中任何一个都是另一条管线。⚠ 版本用**内容键**而不是槽位/时间戳（§17.1：键 = 内容）。

use std::collections::HashMap;
use std::sync::Arc;

use px_protocol::material::{MATERIAL_BIND_GROUP, PARAMS_BINDING, TEXTURE_SLOTS, TextureDimension};
use px_protocol::scene::{AlphaMode, CullMode, Sampler};
use wgpu::util::{DeviceExt, TextureDataOrder};

use crate::art::{LoadedObject, LoadedTexture};
use crate::shot::FORMAT;

/// 内容 shader 的片元入口名。四份 pxart 内容（surface / atmosphere / clouds / ring）
/// 都声明 `@fragment fn fragment(...)` —— 换成别的名字等于换了一份 shader 契约。
pub const FRAGMENT_ENTRY: &str = "fragment";

/// **本宿主自己那份顶点阶段的入口名**（内容 shader 是纯片元的，顶点阶段是我们写的）。
///
/// ⚠ 顶点阶段本身是下一件（§111 第 6 件）的事 —— 这里先把名字定下来，
/// 是为了让"管线两边各自的入口名"只有一处来源：写死两个字面量，
/// 改了一边没改另一边就是"管线建不出来"，而那种错看起来像 shader 坏了。
pub const VERTEX_ENTRY: &str = "vertex";

/// 深度格式，**只有这一个**：无限 reverse-Z 用 `Depth32Float`（§110.1）。
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// 兜底图的那一个像素：纯白。
///
/// 为什么是白、而不是黑或洋红：内容 shader 拿它**乘**（albedo / glow / 覆盖度），
/// 白 = 1.0 = 那一项不参与；黑会把"没给这张图"变成"给了一张全黑图"，而那是另一种画面。
/// 洋红那种"一眼看出缺东西"的兜底属于**占位**（Bevy 宿主在槽里还没装上 shader 时用的），
/// 不是这里：产物说"这张图我不给"是合法状态，不该看起来像坏了。
pub const FALLBACK_PIXEL: [u8; 4] = [255, 255, 255, 255];

/// 兜底图边长：1×1。采样它永远得到同一个值，所以尺寸不影响像素。
pub const FALLBACK_SIZE: u32 = 1;

/// 兜底 cube 的层数（必须与契约里 cube 那一档的层数一致：6）。
pub const FALLBACK_CUBE_LAYERS: u32 = 6;

// ---------------------------------------------------------------------------
// 内容版本与两条状态
// ---------------------------------------------------------------------------

/// shader **内容键 → 版本号**：取内容键的前 16 位十六进制。
///
/// 口径与 Bevy 宿主那份（`px_render::slots::version_of`）**逐字相同** —— 两处各解一次
/// 而规则不同，"同一版 shader"就会在两条宿主里得到两个版本号，管线缓存跟着分成两份。
pub fn version_of(key: &str) -> Result<u64, String> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "shader 成员的内容键应当是 64 位十六进制，实际是 '{key}'"
        ));
    }
    u64::from_str_radix(&key[..16], 16).map_err(|err| format!("内容键 '{key}' 解不开：{err}"))
}

/// 管线特化的键：**哪一版 WGSL + 哪一档剔除 + 哪一档混合**。
///
/// ⚠ 三个都要在键里：管线是不可变状态对象，混合档决定 `blend` 与 `depth_write_enabled`、
/// 剔除档决定 `cull_mode`。少带一维的后果是"换档之后画的是上一条管线的状态"，
/// 而那种错在画面上只是"某几个像素不一样"。
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

    /// 剔除面。`None` = 不剔（`cull = none`）。
    pub fn cull_face(self) -> Option<wgpu::Face> {
        match self.cull {
            0 => Some(wgpu::Face::Back),
            1 => Some(wgpu::Face::Front),
            _ => None,
        }
    }

    /// 混合档。**这一格最容易按直觉写错**：
    ///
    /// - `opaque` / `mask`（这里没有 mask，见 [`alpha_code`]）⇒ `None`：不混合；
    /// - `premultiplied` **与 `add`** ⇒ [`wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING`]
    ///   （`src = One`、`dst = OneMinusSrcAlpha`）；
    /// - `blend` ⇒ `ALPHA_BLENDING`（`src = SrcAlpha`）。
    ///
    /// ⚠ `Add` **不是加法混合**（§110.1 实测）：Bevy 0.19 里 `AlphaMode::Add` 与
    /// `AlphaMode::Premultiplied` 在 `MeshPipelineKey` 里是**同一个** `BLEND_PREMULTIPLIED_ALPHA`
    /// （`bevy_pbr-0.19.1/src/material.rs:616`），于是两者共用同一套 blend state。把 `Add`
    /// "修"成 `src = One, dst = One` 之后，大气那层会亮一档 —— 而判据是逐字节的。
    pub fn blend(self) -> Option<wgpu::BlendState> {
        match self.alpha {
            2 => Some(wgpu::BlendState::ALPHA_BLENDING),
            1 | 3 => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            _ => None,
        }
    }

    /// 写不写深度：只有不透明档写。
    ///
    /// 透明档（`blend` / `premultiplied` / `add`）**一律不写**（Bevy 的 mesh 管线对这三档都置
    /// `depth_write_enabled = false`）—— 写了的话，后画的那层透明物体会把先画的挡掉。
    /// 注意"不写"不影响"要测"：深度**测试**照旧（`GreaterEqual`）。
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

/// 混合档 → 小整数（进管线键）。
///
/// ⚠ `mask` 不在这里：`px_protocol::scene::AlphaMode` 只有 `opaque / premultiplied / blend / add`
/// 四档（文档能表达的就这么几种）。真加第五档时，这里与 [`PipelineKey::blend`] 必须一起改 ——
/// 而"忘了改"的症状是：新档静默地走了不透明那条路。
pub fn alpha_code(alpha: AlphaMode) -> u8 {
    match alpha {
        AlphaMode::Opaque => 0,
        AlphaMode::Premultiplied => 1,
        AlphaMode::Blend => 2,
        AlphaMode::Add => 3,
    }
}

/// 内容键取版本号失败时**当场拒**：管线缓存宁可报错，也不许拿一个假版本号去撞另一条管线。
pub fn key_of(object: &LoadedObject) -> Result<PipelineKey, String> {
    let version = version_of(&object.shader.member.key)
        .map_err(|err| format!("物体 '{}' 的 shader 成员：{err}", object.id))?;
    Ok(PipelineKey::new(version, object.cull, object.alpha))
}

// ---------------------------------------------------------------------------
// 契约 → 布局（**不在 Rust 里抄第二份表**）
// ---------------------------------------------------------------------------

pub fn view_dimension(dimension: TextureDimension) -> wgpu::TextureViewDimension {
    match dimension {
        TextureDimension::D2 => wgpu::TextureViewDimension::D2,
        TextureDimension::Cube => wgpu::TextureViewDimension::Cube,
    }
}

/// group 3 的绑定项，**从契约表推出来**（[`PARAMS_BINDING`] + [`TEXTURE_SLOTS`]）。
///
/// 为什么不在这里写一张 24 项的清单：那张清单就是 §74.3 收口掉的东西
/// （"每多抄一份就多一个两边会漂开的地方"，而漂开的症状是画面错、不是报错）。
/// 布局与 shader 的对账在判据里做：反射出来的第 3 组声明必须是这份布局的**子集**。
pub fn layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let mut entries = vec![wgpu::BindGroupLayoutEntry {
        binding: PARAMS_BINDING,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            // 每个 shader 的结构体大小不同，而布局只有一份：真实大小由各自的缓冲决定
            // （wgpu 按 shader 声明的那一份校验）。
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

// ---------------------------------------------------------------------------
// 兜底图与采样器
// ---------------------------------------------------------------------------

/// 空槽的兜底：1×1 白 2D + 1×1 白 cube + 一个采样器。
pub struct Fallbacks {
    /// 上传用的那四个字节（判据在校验它，也校验上传之后 GPU 上那一个像素）。
    pub pixel: [u8; 4],
    pub texture_2d: wgpu::Texture,
    pub view_2d: wgpu::TextureView,
    pub texture_cube: wgpu::Texture,
    pub view_cube: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl Fallbacks {
    /// ⚠ `COPY_SRC` 只为了判据能把它读回来（"真的绑上了白图"这件事只能这么证）；
    /// 渲染路径用不到它，多这一个 usage 不影响像素。
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
        // 采样器与 Bevy 的 `FallbackImage` 同款（默认：最近邻 + clamp）。
        // ⚠ 内容真去采兜底图时它才起作用（这一档 `shadow = 0` ⇒ 覆盖度图根本不采），
        //    所以它错了也不会立刻在 S2 的图上露出来 —— 那就更该照抄而不是自己挑一个。
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
        }
    }

    pub fn texture(&self, dimension: TextureDimension) -> &wgpu::Texture {
        match dimension {
            TextureDimension::D2 => &self.texture_2d,
            TextureDimension::Cube => &self.texture_cube,
        }
    }
}

/// 文档里的地址模式 → wgpu 的地址模式。单拎出来是为了**不需要设备也能判**（取值来源会漂）。
pub fn address_mode(mode: px_protocol::scene::Address) -> wgpu::AddressMode {
    match mode {
        px_protocol::scene::Address::Repeat => wgpu::AddressMode::Repeat,
        px_protocol::scene::Address::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        px_protocol::scene::Address::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
    }
}

/// 文档里的过滤器 → wgpu 的过滤器（同上）。
pub fn filter_mode(mode: px_protocol::scene::Filter) -> wgpu::FilterMode {
    match mode {
        px_protocol::scene::Filter::Linear => wgpu::FilterMode::Linear,
        px_protocol::scene::Filter::Nearest => wgpu::FilterMode::Nearest,
    }
}

/// 文档里的采样器 → wgpu 的采样器。
///
/// 口径逐字照抄 Bevy 宿主那一份（`px_render::art_cache::sampler_descriptor`）：
///
/// - `address_mode_w` 用的是 **`address_v`**（不是 u）—— 这是既有行为，不是笔误：
///   三轴的取值一旦"改对"，立方图与 2D 的边界像素就会换一种采法；
/// - `mag / min / mipmap` 三个过滤器都是产物里那一个 `filter`（产物只表达"线性还是最近"）；
/// - `anisotropy_clamp` 夹到 `1..=16`（wgpu 的上限是 16；产物写 0 = 不设 ⇒ 落回 1）。
///
/// `lod_clamp` 是 `0..32`（`SamplerDescriptor::default()` 那一份）：贴图最多 10 级，
/// 32 够不着，所以它不参与像素。
///
/// ## 天空盒也走这一条（§135 的那一格）—— 三条实测，一条更正
///
/// **为什么复用 `sampler_of`**：oracle 那条路就是"图用它自己带的采样器" ——
/// `image.sampler = ImageSampler::Descriptor(sampler_descriptor(sampler))`
/// （`px_render/src/art_cache.rs:477`，`sampler_descriptor` 在同文件 `:510-518`），
/// 天空盒的图也是从同一个 `ArtCache::texture` 出来的，**没有第二套采样器**。
/// 所以本宿主这边也只有一条路：拿文档里的 `Sampler` 建采样器，天空盒与内容贴图共用它。
///
/// ⚠ **更正一处容易想当然的地方（我实测过）**："星空产物没带采样器 ⇒ 两边都落回各自的缺省"
/// **不成立** —— 根本没有"落回"这回事：oracle 是**显式**传的
/// `&Sampler::clamped()`（`px_render/src/scene.rs:288-296`）。
/// 而 `Sampler::clamped()` 与 `Sampler::default()` **不是同一个采样器**：
/// 前者 `address_u = ClampToEdge`，后者 `address_u = Repeat`（`address_v` 两者都是
/// ClampToEdge）。立方图每个面的边界纹素正好落在这个差别上 ⇒ 宿主若图省事写成
/// `Sampler::default()`，背景就是"看起来一样、逐位不一样"。
/// **宿主必须用 `Sampler::clamped()`**（这就是 oracle 那个数），理由与 §110.1 同族：
/// 缺省值不是判据，"oracle 传了什么"才是。
///
/// ⚠ 顺带核对过的一份产物（自己读的，不是转述）：`generated/stars` 的清单参数是
/// `{"format":0.0,"height":512.0,"layers":6.0,"levels":1.0,"width":512.0}` ——
/// 五个形状数，**一个采样器字段都没有**（`px_protocol::art::TextureShape::params` 就是这五个）。
/// 采样器从哪来这件事因此完全由**装载方**决定，上面那条更正才要紧。
///
/// ⚠⚠ **另一个陷阱**：谁要是"反正产物没给，就用 Bevy 的缺省" ——
/// Bevy 的 `ImageSamplerDescriptor::default()` 过滤是 **Nearest**
/// （`bevy_image-0.19.1`：`ImageFilterMode` 的 `#[default]` 是 `Nearest`），
/// 而本工程的 `Sampler::default()` / `Sampler::clamped()` 过滤是 **Linear**
/// （`Filter` 的 `#[default]` 是 `Linear`）。星空是高频点状内容，Nearest 与 Linear
/// 在那六张 512² 脸上出来的像素不同，而两张图看起来都"是星空" ——
/// 这正是"用错缺省"最难被发现的那种形状。本宿主只认文档那一份。
pub fn sampler_of(device: &wgpu::Device, sampler: &Sampler) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("material sampler"),
        address_mode_u: address_mode(sampler.address_u),
        address_mode_v: address_mode(sampler.address_v),
        address_mode_w: address_mode(sampler.address_v),
        mag_filter: filter_mode(sampler.filter),
        min_filter: filter_mode(sampler.filter),
        // ⚠ mipmap 那一格是**另一个类型**（`MipmapFilterMode`）：文档只表达"线性还是最近"，
        // 所以两档要各自映射，不能拿 `FilterMode` 硬塞。
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

/// 贴图产物 → GPU 纹理 + 视图（整条 mip 链一次喂进去）。
///
/// ⚠ **次序用 `LayerMajor`**：Bevy 的 `Image` 上传用的就是它
/// （`bevy_render` 的 `prepare_asset`：`TextureDataOrder::LayerMajor`），而判据是"与 Bevy
/// 逐字节同图" ⇒ 读同一串字节的次序必须也是同一个。1 层多级（albedo 780×520×10）与
/// 6 层单级（星空 512×512×6）两种情形下两种次序恰好相同，所以 S2 分辨不出这个选择；
/// **多层 + 多级**的 cube（覆盖度图那种）才会分岔，那时这条注释就是线索。
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

/// 管线布局：**组 0（宿主给的）+ 组 3（材质）**，中间的 1 / 2 留空。
///
/// ⚠ 组号 3 不是我们挑的：它就是 [`MATERIAL_BIND_GROUP`]（Bevy 的 `MATERIAL_BIND_GROUP_INDEX`，
/// 而内容 shader 里的 `#{MATERIAL_BIND_GROUP}` 早被组装器替成了那个数）。
/// 中间的 1 / 2 是 Bevy 的 mesh / 光照那两组，本宿主不用 —— 但**下标不能挪**：
/// 布局数组的**位置**就是组号。
pub fn pipeline_layout(
    device: &wgpu::Device,
    group_zero: &wgpu::BindGroupLayout,
    material: &wgpu::BindGroupLayout,
) -> wgpu::PipelineLayout {
    let mut groups: Vec<Option<&wgpu::BindGroupLayout>> =
        vec![None; MATERIAL_BIND_GROUP as usize + 1];
    groups[0] = Some(group_zero);
    groups[MATERIAL_BIND_GROUP as usize] = Some(material);
    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("material pipeline layout"),
        bind_group_layouts: &groups,
        immediate_size: 0,
    })
}

// ---------------------------------------------------------------------------
// 材质系统：布局 + 采样器表 + 兜底图 + 管线缓存
// ---------------------------------------------------------------------------

/// 一个物体绑好的那一份东西：参数缓冲 + group 3 的绑定组 + 它用的是哪条管线。
pub struct MaterialBinding {
    pub key: PipelineKey,
    pub params: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    /// 产物**真给了**的格（升序）。
    pub bound_slots: Vec<u32>,
    /// 走了兜底白图的格（升序）—— 判据靠它证明"空槽真的绑上了兜底"。
    pub fallback_slots: Vec<u32>,
}

/// 建管线要的那几样输入。`key` 决定状态，其余的是"这一台/这一版不变"的东西。
pub struct PipelineRequest<'a> {
    pub key: PipelineKey,
    pub vertex: &'a wgpu::ShaderModule,
    pub fragment: &'a wgpu::ShaderModule,
    pub pipeline_layout: &'a wgpu::PipelineLayout,
    /// 顶点缓冲布局。⚠ 这一档只用来把管线建起来（S2 的绘制在下一件），
    /// 真正的交错布局归画那一步定 —— 它与**管线键无关**（同一个 mesh 布局配所有材质），
    /// 所以不进键。
    pub vertex_buffers: &'a [wgpu::VertexBufferLayout<'a>],
}

/// 本宿主的材质系统。一个进程一份：布局、采样器、兜底图都不随物体变，
/// 而管线缓存必须跨物体共享（否则"两条管线"会变成"每个物体各一条"）。
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

    /// 一个物体 → 参数缓冲 + group 3 绑定组。
    ///
    /// 参数缓冲**每个物体一份**（布局里 `min_binding_size: None`，真实大小由缓冲决定）：
    /// 两份材质共用一块缓冲时，谁先写谁说了算 —— 那是"同一件事、两处维护"的另一种写法。
    ///
    /// ⚠ 贴图在这里**现传**（`upload_texture`）。判据与这一件只需要"绑得对"；
    /// 一张 1.6 MB 的 albedo 每帧重传当然不行，所以**上传缓存归调用方**
    /// （装载那一档已经有内容键，`bind` 只负责把"哪一格用哪张图"说清楚）。
    pub fn bind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        object: &LoadedObject,
    ) -> Result<MaterialBinding, String> {
        let key = key_of(object)?;
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("material params"),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            contents: &object.params,
        });

        // 视图先全收进一个表里：`entries` 借的是它们，而它们必须活到 `create_bind_group` 之后。
        let mut views: Vec<wgpu::TextureView> = Vec::with_capacity(TEXTURE_SLOTS.len());
        let mut samplers: Vec<wgpu::Sampler> = Vec::with_capacity(TEXTURE_SLOTS.len());
        let mut bound_slots = Vec::new();
        let mut fallback_slots = Vec::new();
        for (binding, dimension) in TEXTURE_SLOTS {
            match object.texture(binding) {
                Some(bound) => {
                    let (_, view) = upload_texture(device, queue, &bound.texture);
                    views.push(view);
                    samplers.push(self.sampler(device, &bound.sampler));
                    bound_slots.push(binding);
                }
                None => {
                    // 产物没给这一格 ⇒ 兜底白图。布局、shader、管线**一个字都不用改**。
                    views.push(self.fallbacks.view(dimension).clone());
                    samplers.push(self.fallbacks.sampler.clone());
                    fallback_slots.push(binding);
                }
            }
        }

        let mut entries: Vec<wgpu::BindGroupEntry> = vec![wgpu::BindGroupEntry {
            binding: PARAMS_BINDING,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &params,
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("material bind group"),
            layout: &self.layout,
            entries: &entries,
        });
        Ok(MaterialBinding {
            key,
            params,
            bind_group,
            bound_slots,
            fallback_slots,
        })
    }

    /// 建/取管线。命中就返回**同一个 `Arc`**（判据按指针相等看"真的复用了吗"）。
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
            // 三角形列表 + 逆时针为正面（wgpu 的缺省）：绕向由 `mesh.rs::outward_winding`
            // 保证朝外（§102 记过：朝里的话被剔掉的是**近**面）。
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: key.cull_face(),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            // 无限 reverse-Z：远处是 0（清屏值也是 0），比较方向是 `GreaterEqual`。
            // 深度偏置三项全 0 —— 自定义材质的 `depth_bias` 只是**透明排序**的偏移，
            // 从不上 GPU（§110.1）。
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
                // 颜色目标就是判据那一张：`Rgba8UnormSrgb`，**硬件做 sRGB 编码**。
                // 一个色调映射都不跑（§110.1），所以这里没有中间格式、没有第二遍编码。
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

    /// 缓存里那条管线（不建）。判据用它证明"同一个键拿到的是同一个 `Arc`"。
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

    /// 本档判据的锚。⚠ 与 `art.rs` 用的是同一个（`target/oracle/...`），
    /// 重复那十行路径代码是有意的：这一篇只许动 material.rs，
    /// 把 art.rs 的测试助手改成 `pub` 就等于让"测试用的东西"进产品表面。
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
            println!("⚠ 跳过：场景产物 {} 不在（target/ 不入 git）", path.display());
            return None;
        }
        Some(path)
    }

    /// 没有 GPU 也要能跑的那部分：状态映射、版本号、布局是不是真从契约表来的。
    #[test]
    fn the_states_follow_the_document_and_the_contract_table() {
        // `Add` 与 `Premultiplied` 是**同一套**状态（§110.1）—— 这一条是那口陷阱的钉子。
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

        // 剔除三档。
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

        // 键的每一维都真的进键（少一维就是"换档走了上一条管线"）。
        let base = PipelineKey::new(7, CullMode::Back, AlphaMode::Opaque);
        assert_ne!(base, PipelineKey::new(8, CullMode::Back, AlphaMode::Opaque));
        assert_ne!(base, PipelineKey::new(7, CullMode::None, AlphaMode::Opaque));
        assert_ne!(base, PipelineKey::new(7, CullMode::Back, AlphaMode::Add));

        // 内容键 → 版本号：前 16 位十六进制；不是 64 位十六进制就当场拒。
        assert_eq!(
            version_of("f679cdf810156a75ee3dcd477c0d9c9871c34574eacbd20398407996d3e6136c").unwrap(),
            0xf679_cdf8_1015_6a75
        );
        assert!(version_of("f679cdf8").is_err());
        assert!(version_of(&"z".repeat(64)).is_err());

        // 布局从契约表推出来：参数块占第 0 格，之后每一格贴图都跟着一个采样器。
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

    /// 采样器的口径（不碰 GPU 的那一半）：**w 轴跟的是 v 不是 u**、各向异性夹到 1..=16。
    ///
    /// 这一条单独拧出来量，是因为 `sampler_of` 要设备才跑得起来，而"w 轴取哪个地址模式"
    /// 恰恰是照抄时最容易"顺手改对"的一格 —— 改错了，立方图与 2D 的边界像素会换一种采法。
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
        // 三档地址模式**两两不同**：否则"映射错了"会与"文档写的就是这个"分不开。
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

        // ⚠ 天空盒那一格（§135）用的是 `Sampler::clamped()`，它与 `default()` **只差
        // `address_u`**（ClampToEdge / Repeat）。这一条把这个差别钉住：哪天有人把
        // `clamped()` 写成 `default()`，"背景看起来一样、逐位不一样"就没有线索可循了。
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

    /// 兜底图的形状：1×1、六层的 cube 视图（尺寸不影响像素，但**维度**必须对，
    /// 否则建 bind group 时是运行期错误）。
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

    // -----------------------------------------------------------------------
    // 下面这些要真设备（Vulkan 锁死，见 `gpu.rs`）
    // -----------------------------------------------------------------------

    /// 顶点阶段的**替身**：字段/origin/次序必须与内容 shader 的 `VertexOutput` 逐字一致。
    ///
    /// ⚠ 真正的顶点阶段（世界矩阵乘、法线转置、`uv`）是**下一件**（§111 第 6 件）的事。
    /// 这里只用来证明"管线建得起来、而且顶点↔片元的接口对得上"：
    /// 它把局部坐标直接当裁剪坐标写出去，所以**不会**画出正确的图 —— 也不会被当成成品。
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

    /// 组 0 的布局**替身**：从反射出来的全局变量现推。
    ///
    /// ⚠ 真正的组 0 布局归第 6 件（它要知道深度预通道那张图从哪来、`clustered_lights`
    /// 是只读还是可写）。这里必须造一份是因为**管线布局要覆盖 shader 声明的每一组**，
    /// 少一组 `create_render_pipeline` 当场拒。为了让替身不悄悄漂开，它只认反射出来的
    /// `(group, binding)`，并且返回值里带上那张表供判据对账。
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
                (naga::AddressSpace::Handle, naga::TypeInner::Image { dim, class, .. }) => {
                    let view_dimension = match dim {
                        naga::ImageDimension::D2 => wgpu::TextureViewDimension::D2,
                        naga::ImageDimension::Cube => wgpu::TextureViewDimension::Cube,
                        other => panic!("组 0 的贴图维度不认识：{other:?}"),
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

    /// 反射一份组装后的 WGSL 里**第 3 组**的声明（贴图格 + 维度）。
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
        // `TextureViewDimension` 没有 `Ord`：按格号排就够（格号本来就唯一）。
        slots.sort_by_key(|slot| slot.0);
        slots
    }

    /// 读回一张纹理头几个字节（判据用；渲染路径不做这件事）。
    fn read_pixel(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> [u8; 4] {
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

    /// 本档的判据：**两个物体**的 group 3 绑定组与管线都要真的建得起来，而且
    /// 布局 ⊇ shader 声明、空槽真的绑兜底、两条管线各自的键不同、同一个键拿回同一个 `Arc`。
    ///
    /// ⚠ 这一条要真设备（Vulkan 锁死）。拿不到设备就**不是"跳过"**：
    /// `gpu::connect()` 打 `后端断言失败` 并 `exit(2)`（§101：探针拿不到设备要 exit(2)，
    /// 不许静默变绿）—— 所以这一档的失败方式是**响的**，不会伪装成通过。
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

        // 兜底图：先看 CPU 那一份，再看 GPU 上真的那一个像素。
        assert_eq!(
            materials.fallbacks().pixel,
            [255, 255, 255, 255],
            "上传前的那四个字节"
        );
        assert_eq!(
            read_pixel(&gpu.device, &gpu.queue, materials.fallbacks().texture(TextureDimension::D2)),
            [255, 255, 255, 255],
            "兜底 2D 图上真的那一个像素"
        );
        assert_eq!(
            read_pixel(&gpu.device, &gpu.queue, materials.fallbacks().texture(TextureDimension::Cube)),
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
            let module = crate::shader::validate(
                &format!("{id}（组装后）"),
                &object.shader.assembled,
            )
            .unwrap_or_else(|err| panic!("{id} 的组装文本过不了 naga：{err}"));

            // ① 布局（超集）必须是 shader 第 3 组声明的**超集**。
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
                    } => assert_eq!(
                        found, *dimension,
                        "{id} 第 {binding} 格的维度与布局不一致"
                    ),
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

            // ② 绑定组真的建得起来，而且"哪几格走了兜底"是**看得见**的。
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

            // ③ 管线：组装后的真 WGSL 当片元阶段，替身顶点阶段只为把管线建起来。
            let fragment = gpu
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(id),
                    source: wgpu::ShaderSource::Wgsl(object.shader.assembled.as_str().into()),
                });
            let (group_zero, rows) = group_zero_layout(&gpu.device, &module);
            // ⚠ **组 0 的声明是「每个 shader 各一份子集」**，不是固定五格：
            // `surface.wgsl` 读 view / lights / clustered_lights，`atmosphere.wgsl` 读
            // view / clustered_lights / depth_prepass_texture（各自 import 什么就是什么）。
            // 所以管线布局必须**按 shader 声明的那些格**建；反过来，真正给 GPU 用的
            // 组 0 布局应该做成五格的**超集**（布局可以多、绑定不能少），
            // 这一条是第 6 件要做的决定，这里只把"声明到底是什么"钉住。
            let known = [
                (0, 0, "view".to_string()),
                (0, 1, "lights".to_string()),
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
                "planet" => vec!["view", "lights", "clustered_lights"],
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
            let pipeline_layout = pipeline_layout(&gpu.device, &group_zero, &material_layout);
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
                materials.cached(material.key).map(|found| Arc::as_ptr(&found)),
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

        // ④ 两个物体必须是**两条**管线：`Opaque/Back` 与 `Add/Back` 的混合档不同。
        assert_ne!(keys[0], keys[1], "planet 与 atmosphere 的状态不同");
        assert_eq!(keys[0].cull, keys[1].cull, "剔除档都是 back");
        assert_ne!(keys[0].alpha, keys[1].alpha, "混合档一个 opaque 一个 add");
        assert_ne!(
            keys[0].shader, keys[1].shader,
            "两份 shader 的内容版本必须不同（否则它们会共用一条管线）"
        );
        assert_eq!(materials.len(), 2, "两条管线，不多不少");

        // ⑤ 再要一次同样的键：`len` 不涨，且拿回同一个 `Arc`。
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
        let pipeline_layout = pipeline_layout(&gpu.device, &group_zero, &material_layout);
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

    /// 贴图上传：形状、格式、mip 级数、以及"6 层就是 cube 视图"（要设备）。
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

        // 采样器那条路：文档里那几个数落到 wgpu 上不许漂（`address_mode_w` 跟的是 **v**）。
        let sampler = planet.texture(1).expect("第 1 格").sampler;
        assert_eq!(address_mode(sampler.address_u), wgpu::AddressMode::Repeat);
        assert_eq!(
            address_mode(sampler.address_v),
            wgpu::AddressMode::ClampToEdge,
            "这一档的 v 轴是夹边（w 轴也用它）"
        );
        assert_eq!(filter_mode(sampler.filter), wgpu::FilterMode::Linear);
        assert_eq!(sampler.anisotropy.clamp(1, 16), 8);

        // 一个 1×1 的半精度贴图走一遍上传（覆盖度图那一档的格式）。
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
