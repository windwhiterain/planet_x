use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use px_protocol::scene::Value;

/// 材质绑定组的约定。**这就是通用渲染的全部契约**：
///
/// | 格 | 声明 | 谁填 |
/// |---|---|---|
/// | 0 | `var<uniform> params: <这份 shader 自己声明的结构体>` | 产物里按名字给的参数（这一篇按结构体的真布局打包） |
/// | 1 / 2 | `texture_2d<f32>` / `sampler` | 产物里 `binding = 1` 的那张贴图 |
/// | 3 / 4 | `texture_2d<f32>` / `sampler` | 产物里 `binding = 3` 的那张 |
/// | 5 / 6 | `texture_cube<f32>` / `sampler` | 产物里 `binding = 5` 的那张 |
/// | 7 / 8 | `texture_cube<f32>` / `sampler` | 产物里 `binding = 7` 的那张 |
///
/// **布局必须与实例无关**（Bevy 的 `MaterialPlugin` 每种材质类型只建一份绑定布局），
/// 所以空着的格一律填兜底贴图/采样器 —— shader 里声明了就一定绑得上。
/// 参数块是唯一可变长的一格：布局上写 `min_binding_size = None`，每个材质各自建一块缓冲。
///
/// ⚠ 参数结构体必须**声明在入口 shader 里**（库只放函数）：反射读的就是它。
pub const MATERIAL_BIND_GROUP: u32 = 2;
pub const PARAMS_BINDING: u32 = 0;
/// 贴图格：`(绑定下标, 维度)`。采样器永远在 `+1`。
pub const TEXTURE_SLOTS: [(u32, TextureDimension); 4] = [
    (1, TextureDimension::D2),
    (3, TextureDimension::D2),
    (5, TextureDimension::Cube),
    (7, TextureDimension::Cube),
];
/// 参数块的字节上限。布局上不写死大小，这条只是"别把 uniform 撑爆"的护栏。
pub const MAX_PARAMS_BYTES: u32 = 1024;
/// uniform 的绑定大小按 16 字节对齐（`mat4` / `vec4` 的天然对齐）。
pub const PARAMS_ALIGN: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureDimension {
    D2,
    Cube,
}

impl TextureDimension {
    pub fn name(self) -> &'static str {
        match self {
            Self::D2 => "texture_2d",
            Self::Cube => "texture_cube",
        }
    }

    /// 贴图产物那边对应的层数：2D = 1 层、cube = 6 层。
    pub fn layers(self) -> u32 {
        match self {
            Self::D2 => 1,
            Self::Cube => px_protocol::art::CUBE_FACES,
        }
    }
}

/// 参数的类型档。只收**产物能表达**的那几种（`Value` 只有 数 / 文本 / 三数 / 四数）：
/// 多一种类型就是多一条"产物里写不出来"的路。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    F32,
    I32,
    U32,
    Vec3,
    Vec4,
}

impl ParamKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::U32 => "u32",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
        }
    }

    pub fn width(self) -> u32 {
        match self {
            Self::F32 | Self::I32 | Self::U32 => 4,
            Self::Vec3 => 12,
            Self::Vec4 => 16,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParamSlot {
    pub name: String,
    pub offset: u32,
    pub kind: ParamKind,
}

#[derive(Debug, Clone)]
pub struct TextureSlot {
    pub binding: u32,
    pub dimension: TextureDimension,
}

/// 一份 shader 反射出来的材质契约：参数块的布局 + 贴图格的维度。
#[derive(Debug, Clone)]
pub struct MaterialLayout {
    pub params: Vec<ParamSlot>,
    /// 打包出来的 uniform 缓冲大小（16 对齐，至少 16）。
    pub params_bytes: u32,
    pub textures: Vec<TextureSlot>,
}

impl MaterialLayout {
    pub fn param(&self, name: &str) -> Option<&ParamSlot> {
        self.params.iter().find(|slot| slot.name == name)
    }

    pub fn texture(&self, binding: u32) -> Option<&TextureSlot> {
        self.textures.iter().find(|slot| slot.binding == binding)
    }

    /// 参数名清单（报错用）。
    pub fn param_names(&self) -> String {
        if self.params.is_empty() {
            return "（它一个参数都没声明）".to_string();
        }
        self.params
            .iter()
            .map(|slot| format!("{}: {}", slot.name, slot.kind.name()))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    /// 把产物里按名字给的参数**按 shader 自己声明的布局**打成字节。
    ///
    /// 三档都当场报错，不静默：缺参（shader 要、产物没给）、多参（产物给了、shader 没声明）、
    /// 类型不符（数是小数却要 `u32`、三数给了四数……）。
    pub fn pack(&self, params: &BTreeMap<String, Value>) -> Result<Vec<u8>, String> {
        for key in params.keys() {
            if self.param(key).is_none() {
                return Err(format!(
                    "产物给了参数 '{key}'，但这份 shader 没声明它；它声明的参数：{}",
                    self.param_names()
                ));
            }
        }

        let mut bytes = vec![0_u8; self.params_bytes as usize];
        for slot in &self.params {
            let value = params.get(&slot.name).ok_or_else(|| {
                format!(
                    "shader 声明了参数 '{}'（{}），产物没给；它声明的参数：{}",
                    slot.name,
                    slot.kind.name(),
                    self.param_names()
                )
            })?;
            let start = slot.offset as usize;
            let end = start + slot.kind.width() as usize;
            let Some(target) = bytes.get_mut(start..end) else {
                return Err(format!(
                    "参数 '{}' 落在 {}..{}，超出了参数块（{} 字节）",
                    slot.name, start, end, self.params_bytes
                ));
            };
            write_value(slot, value, target)?;
        }
        Ok(bytes)
    }
}

fn number_of(slot: &ParamSlot, value: &Value) -> Result<f64, String> {
    match value {
        Value::Num(number) => Ok(*number),
        other => Err(format!(
            "参数 '{}' 是 {}，shader 要的是 {}",
            slot.name,
            describe(other),
            slot.kind.name()
        )),
    }
}

fn describe(value: &Value) -> &'static str {
    match value {
        Value::Num(_) => "一个数",
        Value::Text(_) => "一段文本",
        Value::Triple(_) => "三个数",
        Value::Quad(_) => "四个数",
    }
}

fn write_value(slot: &ParamSlot, value: &Value, target: &mut [u8]) -> Result<(), String> {
    match slot.kind {
        ParamKind::F32 => {
            let number = number_of(slot, value)? as f32;
            target.copy_from_slice(&number.to_le_bytes());
        }
        ParamKind::U32 => {
            let number = number_of(slot, value)?;
            if number.fract() != 0.0 || !(0.0..=u32::MAX as f64).contains(&number) {
                return Err(format!(
                    "参数 '{}' 是 {number}，shader 要的是 u32（非负整数）",
                    slot.name
                ));
            }
            target.copy_from_slice(&(number as u32).to_le_bytes());
        }
        ParamKind::I32 => {
            let number = number_of(slot, value)?;
            if number.fract() != 0.0 || !(i32::MIN as f64..=i32::MAX as f64).contains(&number) {
                return Err(format!(
                    "参数 '{}' 是 {number}，shader 要的是 i32（整数）",
                    slot.name
                ));
            }
            target.copy_from_slice(&(number as i32).to_le_bytes());
        }
        ParamKind::Vec3 => match value {
            Value::Triple(items) => {
                for (index, item) in items.iter().enumerate() {
                    target[index * 4..index * 4 + 4].copy_from_slice(&item.to_le_bytes());
                }
            }
            other => {
                return Err(format!(
                    "参数 '{}' 是 {}，shader 要的是三个数",
                    slot.name,
                    describe(other)
                ));
            }
        },
        ParamKind::Vec4 => match value {
            Value::Quad(items) => {
                for (index, item) in items.iter().enumerate() {
                    target[index * 4..index * 4 + 4].copy_from_slice(&item.to_le_bytes());
                }
            }
            other => {
                return Err(format!(
                    "参数 '{}' 是 {}，shader 要的是四个数",
                    slot.name,
                    describe(other)
                ));
            }
        },
    }
    Ok(())
}

fn kind_of(
    module: &naga::Module,
    handle: naga::Handle<naga::Type>,
) -> Result<ParamKind, String> {
    match &module.types[handle].inner {
        naga::TypeInner::Scalar(scalar) => scalar_kind(*scalar, 1),
        naga::TypeInner::Vector { size, scalar } => {
            let width = match size {
                naga::VectorSize::Bi => {
                    return Err("vec2 参数还没有产物能表达（Value 只有 数 / 三数 / 四数）".to_string());
                }
                naga::VectorSize::Tri => 3,
                naga::VectorSize::Quad => 4,
            };
            scalar_kind(*scalar, width)
        }
        other => Err(format!("参数块里有不认识的成员类型：{other:?}")),
    }
}

fn scalar_kind(scalar: naga::Scalar, width: usize) -> Result<ParamKind, String> {
    if scalar.width != 4 {
        return Err(format!(
            "参数里的标量是 {} 字节的：只认 4 字节的 f32 / i32 / u32",
            scalar.width
        ));
    }
    match (width, scalar.kind) {
        (1, naga::ScalarKind::Float) => Ok(ParamKind::F32),
        (1, naga::ScalarKind::Sint) => Ok(ParamKind::I32),
        (1, naga::ScalarKind::Uint) => Ok(ParamKind::U32),
        (3, naga::ScalarKind::Float) => Ok(ParamKind::Vec3),
        (4, naga::ScalarKind::Float) => Ok(ParamKind::Vec4),
        (_, kind) => Err(format!(
            "参数里的向量是 {kind:?} × {width}：只认 vec3<f32> / vec4<f32>"
        )),
    }
}

/// 反射：从组装好的 WGSL 里读出材质契约。
///
/// 为什么是**读 shader**而不是再写一张 Rust 侧的表：Bevy 的绑定布局是编译期的
/// （`Material::fragment_shader()` 是静态函数，§52.3），产物能决定的只有槽里的源码。
/// 那么"参数怎么排"这件事只能有**一个**来源 —— 就是那份源码。写第二张表就是第二个会漂开的默认值。
pub fn reflect_assembled(assembled: &str, name: &str) -> Result<MaterialLayout, String> {
    let module = naga::front::wgsl::parse_str(assembled).map_err(|error| {
        format!(
            "{name} 解析失败：{}",
            error.emit_to_string(assembled)
        )
    })?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .map_err(|error| format!("{name} 校验失败：{error:?}"))?;

    let mut params: Option<(Vec<ParamSlot>, u32)> = None;
    let mut textures: Vec<TextureSlot> = Vec::new();
    let mut samplers: Vec<u32> = Vec::new();

    for (_handle, global) in module.global_variables.iter() {
        let Some(binding) = global.binding else {
            continue;
        };
        if binding.group != MATERIAL_BIND_GROUP {
            continue;
        }
        let inner = &module.types[global.ty].inner;
        if binding.binding == PARAMS_BINDING {
            if !matches!(global.space, naga::AddressSpace::Uniform) {
                return Err(format!(
                    "{name} 的第 {PARAMS_BINDING} 格不是 uniform：参数块必须是 `var<uniform>`"
                ));
            }
            let naga::TypeInner::Struct { members, span } = inner else {
                return Err(format!(
                    "{name} 的第 {PARAMS_BINDING} 格不是结构体：参数块要声明成一个 `struct`"
                ));
            };
            let mut slots = Vec::new();
            for member in members {
                let member_name = member.name.clone().ok_or_else(|| {
                    format!("{name} 的参数块里有没名字的成员：它必须是 `名字: 类型` 的形状")
                })?;
                slots.push(ParamSlot {
                    name: member_name,
                    offset: member.offset,
                    kind: kind_of(&module, member.ty)
                        .map_err(|err| format!("{name} 的参数 '{:?}'：{err}", member.name))?,
                });
            }
            let bytes = span.div_ceil(PARAMS_ALIGN) * PARAMS_ALIGN;
            if bytes > MAX_PARAMS_BYTES {
                return Err(format!(
                    "{name} 的参数块是 {bytes} 字节，超过上限 {MAX_PARAMS_BYTES}"
                ));
            }
            params = Some((slots, bytes));
            continue;
        }
        match inner {
            naga::TypeInner::Image { dim, arrayed, .. } => {
                let dimension = match (dim, arrayed) {
                    (naga::ImageDimension::D2, false) => TextureDimension::D2,
                    (naga::ImageDimension::Cube, false) => TextureDimension::Cube,
                    _ => {
                        return Err(format!(
                            "{name} 第 {} 格是 {dim:?}（arrayed = {arrayed}）的贴图：\
                             材质只认 texture_2d 与 texture_cube",
                            binding.binding
                        ));
                    }
                };
                textures.push(TextureSlot {
                    binding: binding.binding,
                    dimension,
                });
            }
            naga::TypeInner::Sampler { .. } => samplers.push(binding.binding),
            other => {
                return Err(format!(
                    "{name} 第 {} 格是 {other:?}：材质绑定组只有参数块（第 0 格）、\
                     贴图（奇数格）与采样器（贴图 + 1）",
                    binding.binding
                ));
            }
        }
    }

    let Some((slots, params_bytes)) = params else {
        return Err(format!(
            "{name} 没有声明参数块：材质绑定组第 {PARAMS_BINDING} 格必须是 `var<uniform> params: <struct>`"
        ));
    };

    for texture in &textures {
        let Some((_, dimension)) = TEXTURE_SLOTS
            .iter()
            .find(|(binding, _)| *binding == texture.binding)
        else {
            return Err(format!(
                "{name} 把贴图声明在第 {} 格：贴图只能占 {} 这几格",
                texture.binding,
                TEXTURE_SLOTS
                    .iter()
                    .map(|(binding, _)| binding.to_string())
                    .collect::<Vec<_>>()
                    .join(" / ")
            ));
        };
        if *dimension != texture.dimension {
            return Err(format!(
                "{name} 第 {} 格声明的是 {}，约定里这一格是 {}",
                texture.binding,
                texture.dimension.name(),
                dimension.name()
            ));
        }
        let sampler = texture.binding + 1;
        if !samplers.contains(&sampler) {
            return Err(format!(
                "{name} 第 {} 格有贴图但没有采样器：采样器要声明在第 {sampler} 格",
                texture.binding
            ));
        }
    }

    textures.sort_by_key(|slot| slot.binding);
    Ok(MaterialLayout {
        params: slots,
        params_bytes,
        textures,
    })
}

// ---------------------------------------------------------------------------
// 反射缓存
//
// 反射是**纯函数**（WGSL 文本 → 契约），而 WGSL 的版本号就是它的内容键（键 = 内容，§17.1）
// ⇒ 同一版永远反射出同一份契约。缓存键加上库的指纹：库文件换了内容时，
// 即使入口 shader 一个字没动，也必须重新反射。
//
// ⚠ 库指纹的算口径只有一份，住在 `px_shader`（`modules_fingerprint`）：烘图侧把**闭包**
// 指纹算进 shader 产物键（§52.3），这里算的是**整张模块表**。两处都不许自己搓 FNV ——
// 各搓一份的结果是"键说没变、反射说变了"这种谁也说不清的分歧。
// ---------------------------------------------------------------------------

fn library() -> &'static (px_shader::ModuleTable, u64) {
    static LIBRARY: OnceLock<(px_shader::ModuleTable, u64)> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let modules = crate::shaders::module_sources();
        let fingerprint = px_shader::modules_fingerprint(&modules);
        (modules, fingerprint)
    })
}

type Cache = Mutex<HashMap<(u64, u64), Arc<MaterialLayout>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 「这一版 WGSL 的材质契约是什么」。`version` = 产物成员的内容键前 16 位（`slots::version_of`）。
pub fn layout_of(version: u64, name: &str, source: &str) -> Result<Arc<MaterialLayout>, String> {
    let (modules, library_hash) = library();
    let key = (version, *library_hash);
    if let Some(layout) = cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(&key)
    {
        return Ok(layout.clone());
    }
    let mut seen = Vec::new();
    let assembled = crate::shaders::render_source(source, modules, &mut seen);
    let layout = Arc::new(reflect_assembled(&assembled, name)?);
    cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(key, layout.clone());
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params_of(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect()
    }

    fn layout_of_source(name: &str) -> MaterialLayout {
        let modules = crate::shaders::module_sources();
        let path = crate::shaders::shader_source_of(name);
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        let mut seen = Vec::new();
        let assembled = crate::shaders::render_source(&source, &modules, &mut seen);
        reflect_assembled(&assembled, name).unwrap_or_else(|err| panic!("{err}"))
    }

    /// 契约的**形状**由 shader 自己的结构体说了算：这三个偏移与字段顺序就是
    /// 迁移前 Rust 侧 `CloudParams`（`#[derive(ShaderType)]`）的布局，逐项相同。
    #[test]
    fn the_cloud_params_layout_comes_from_the_shader() {
        let layout = layout_of_source("clouds.wgsl");
        let first = &layout.params[0];
        assert_eq!(first.name, "orientation");
        assert_eq!(first.offset, 0);
        assert_eq!(first.kind, ParamKind::Vec4);
        assert_eq!(layout.param("tint").expect("tint 在").offset, 16);
        assert_eq!(layout.param("inner").expect("inner 在").offset, 32);
        assert_eq!(layout.param("steps").expect("steps 在").kind, ParamKind::U32);
        assert_eq!(layout.param("seed").expect("seed 在").kind, ParamKind::U32);
        assert_eq!(layout.param("wind_skin").expect("wind_skin 在").offset, 120);
        assert_eq!(layout.params_bytes, 128, "124 字节的块按 16 对齐到 128");
    }

    /// 贴图格：云只有一张 cube（第 5 格），地表有两张 2D（1、3）+ 一张 cube（5）。
    #[test]
    fn the_texture_slots_follow_the_convention() {
        let clouds = layout_of_source("clouds.wgsl");
        assert_eq!(clouds.textures.len(), 1);
        assert_eq!(clouds.textures[0].binding, 5);
        assert_eq!(clouds.textures[0].dimension, TextureDimension::Cube);

        let surface = layout_of_source("surface.wgsl");
        let bindings: Vec<u32> = surface.textures.iter().map(|slot| slot.binding).collect();
        assert_eq!(bindings, vec![1, 3, 5]);
        assert_eq!(surface.texture(1).expect("在").dimension, TextureDimension::D2);
        assert_eq!(surface.texture(5).expect("在").dimension, TextureDimension::Cube);
    }

    #[test]
    fn packing_uses_the_offsets_the_shader_declared() {
        let layout = layout_of_source("clouds.wgsl");
        let mut params: BTreeMap<String, Value> = BTreeMap::new();
        for slot in &layout.params {
            params.insert(
                slot.name.clone(),
                match slot.kind {
                    ParamKind::Vec4 => Value::Quad([1.0, 2.0, 3.0, 4.0]),
                    ParamKind::Vec3 => Value::Triple([1.0, 2.0, 3.0]),
                    ParamKind::U32 | ParamKind::I32 => Value::Num(7.0),
                    ParamKind::F32 => Value::Num(0.5),
                },
            );
        }
        let bytes = layout.pack(&params).expect("打包失败");
        assert_eq!(bytes.len() as u32, layout.params_bytes);
        // orientation（第 0 格 vec4）与 inner（32 字节处的 f32）落在 shader 说的位置上。
        assert_eq!(&bytes[0..4], &1.0_f32.to_le_bytes());
        assert_eq!(&bytes[12..16], &4.0_f32.to_le_bytes());
        assert_eq!(&bytes[32..36], &0.5_f32.to_le_bytes());
        // steps 是 u32：按整数位打包，不是浮点位。
        let steps = layout.param("steps").expect("steps 在");
        let at = steps.offset as usize;
        assert_eq!(&bytes[at..at + 4], &7_u32.to_le_bytes());
    }

    #[test]
    fn missing_extra_and_ill_typed_params_are_errors() {
        let layout = layout_of_source("atmosphere.wgsl");
        let full: Vec<(&str, Value)> = layout
            .params
            .iter()
            .map(|slot| {
                (
                    slot.name.as_str(),
                    match slot.kind {
                        ParamKind::Vec4 => Value::Quad([0.0; 4]),
                        ParamKind::Vec3 => Value::Triple([0.0; 3]),
                        _ => Value::Num(1.0),
                    },
                )
            })
            .collect();

        let mut missing = params_of(&full);
        let dropped = missing.keys().next().cloned().expect("至少有一个参数");
        missing.remove(&dropped);
        let err = layout.pack(&missing).expect_err("缺参必须报错");
        assert!(err.contains(&dropped), "报错要点名：{err}");

        let mut extra = params_of(&full);
        extra.insert("nonsense".to_string(), Value::Num(1.0));
        let err = layout.pack(&extra).expect_err("多参必须报错");
        assert!(err.contains("nonsense"), "报错要点名：{err}");

        let mut wrong = params_of(&full);
        wrong.insert("softness".to_string(), Value::Text("软".to_string()));
        let err = layout.pack(&wrong).expect_err("类型不符必须报错");
        assert!(err.contains("softness"), "报错要点名：{err}");

        let mut fractional = params_of(&full);
        fractional.insert("softness".to_string(), Value::Num(0.5));
        assert!(
            layout.pack(&fractional).is_ok(),
            "f32 参数收小数：这是它的正常形态"
        );

        let layout_u32 = layout_of_source("clouds.wgsl");
        let mut cloud: BTreeMap<String, Value> = BTreeMap::new();
        for slot in &layout_u32.params {
            cloud.insert(
                slot.name.clone(),
                match slot.kind {
                    ParamKind::Vec4 => Value::Quad([0.0; 4]),
                    ParamKind::U32 => Value::Num(if slot.name == "steps" { 1.5 } else { 0.0 }),
                    _ => Value::Num(0.0),
                },
            );
        }
        let err = layout_u32.pack(&cloud).expect_err("小数给 u32 必须报错");
        assert!(err.contains("steps"), "报错要点名：{err}");
    }

    #[test]
    fn a_param_block_that_is_not_a_struct_is_rejected() {
        let source = "#import bevy_pbr::forward_io::VertexOutput\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: vec4<f32>;\n\
                      @fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> { return params; }\n";
        let modules = crate::shaders::module_sources();
        let mut seen = Vec::new();
        let assembled = crate::shaders::render_source(source, &modules, &mut seen);
        let err = reflect_assembled(&assembled, "夹具").expect_err("必须是结构体");
        assert!(err.contains("结构体"), "报错要说清要什么：{err}");
    }

    #[test]
    fn a_texture_on_the_wrong_slot_is_rejected() {
        let source = "#import bevy_pbr::forward_io::VertexOutput\n\
                      struct P { x: f32 };\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: P;\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(2) var cover: texture_cube<f32>;\n\
                      @group(#{MATERIAL_BIND_GROUP}) @binding(3) var cover_sampler: sampler;\n\
                      @fragment fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {\n\
                      \x20   return textureSample(cover, cover_sampler, vec3<f32>(0.0, 0.0, 1.0));\n\
                      }\n";
        let modules = crate::shaders::module_sources();
        let mut seen = Vec::new();
        let assembled = crate::shaders::render_source(source, &modules, &mut seen);
        let err = reflect_assembled(&assembled, "夹具").expect_err("偶数格必须被拒");
        assert!(err.contains("第 2 格"), "报错要点名那一格：{err}");
    }
}
