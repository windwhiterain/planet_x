//! **CAS 装载**：一份 `.pxart` 渲染文档 → 一堆"可以上传 GPU 的东西"（但这一步**不碰 GPU**）。
//!
//! §111 的第 4 件。为什么单独一篇、而且要在建管线之前：要渲染**任何**场景，
//! 就得先回答"这份文档指的是哪些产物、那些产物里到底是什么"。于是这一篇把整条链走通一次：
//!
//! 文档（`read_scene`）→ 成员键 → CAS 路径（`Member::resolve`）→ 载荷（网格 / 贴图 / WGSL）
//! → 组装 + 反射 → **打好包的参数字节**。
//!
//! 三条口径（每一条都是"换个做法就会出另一种图，而门不会响"）：
//!
//! 1. **贴图与采样器都由文档说了算**：绑哪一格写在 `Material.textures[binding]` 里，
//!    "这张图该怎么采"（`Sampler`）跟图一起进屋。渲染器不猜、也不补默认值 ——
//!    补一次默认值，就有了"两个地方说同一件事"。
//! 2. **参数按 shader 自己声明的结构体打包**（`MaterialLayout::pack`，反射出来的那份）：
//!    Rust 侧没有第二张参数表。缺参 / 多参 / 类型不符三档都当场报错。
//! 3. **产物上的两道对账不许省**（`load_shader` 里那两条）：include 闭包（§52.3）与
//!    schema descriptor（§74.3）。省掉它们，一次"改了库没重烘"或"换了反射规则"
//!    会表现成**画面错但不报错**，而键 / 场景键 / 槽版本全都还是对的。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_protocol::art::{self, AssetKind, TextureFormat, TextureShape};
use px_protocol::material::{MaterialLayout, TextureDimension};
use px_protocol::scene::{
    AlphaMode, CullMode, FrameMaterial, Geometry, Member, Object, Sampler, SceneSpec, Transform,
    Value,
};
use px_protocol::wire::DType;

use crate::material::version_of;
use crate::mesh::Mesh;
use crate::shader;

/// 一份**已解开**的贴图产物：形状 + 整条 mip 链的原始字节 + 最细那一级的 8 位预览。
///
/// ⚠ 两样都要留，而且**用途不同**：
///
/// - `bytes` + `shape` 是**上传用**的（`queue.write_texture` 要按 mip 一级一级喂，
///   `shape.levels` 与各级尺寸决定 `bytes_per_row`）。只留解码后的那一级图，
///   到了上传那一步就得回头重读产物 —— 那就是同一个东西读两遍。
/// - `image` 是**给人看 / 给判据用**的预览（"这张图到底是不是我以为的那张"）。
///   `Rgba16Float` 那一档是**有损**转成 8 位的（覆盖度图的 mask + 三轴梯度），
///   所以它只能当预览，**不能**拿去上传：上传必须用 `bytes`。
///
/// 立方图（6 层）的预览把 6 个面**竖着码**成一张 `width × height*6` 的图 ——
/// 判据里对尺寸的断言（星空的 512×3072）依赖这条口径。
#[derive(Clone, Debug)]
pub struct LoadedTexture {
    pub member: Member,
    pub shape: TextureShape,
    pub bytes: Vec<u8>,
    pub image: image::RgbaImage,
}

impl LoadedTexture {
    pub fn label(&self) -> String {
        self.member.to_string()
    }

    /// 最细那一级 mip 的字节数（后面按级上传时逐级切片的起点）。
    pub fn base_level_bytes(&self) -> usize {
        self.shape.width as usize
            * self.shape.height as usize
            * self.shape.layers as usize
            * self.shape.format.texel_bytes()
    }
}

/// 材质里**实际绑上**的一格：格号（契约里的下标）+ 采样器 + 那张图。
///
/// 空着的格不在这里 —— 它们由 `material.rs` 绑兜底白图（§65 / §108.1：`orbit-bare` 的
/// `planet` 真的会用到那两格），装载这一步只负责"产物给了的"。
#[derive(Clone, Debug)]
pub struct BoundTexture {
    pub binding: u32,
    pub sampler: Sampler,
    pub texture: LoadedTexture,
}

/// 天空盒：一张 cube 贴图 + 它的采样器。
///
/// 采样器是**这里定的**（`Sampler::clamped()`），与材质那几格不同 —— 天空盒不是材质的一格，
/// 它的采样方式不在文档里，而是渲染器与 Bevy 对齐的那一份（`bevy_core_pipeline` 的
/// skybox 管线用默认采样器：clamp + 线性）。改这里等于改背景像素。
#[derive(Clone, Debug)]
pub struct Skybox {
    pub sampler: Sampler,
    pub texture: LoadedTexture,
}

impl Skybox {
    /// 这份天空盒绑到某一格的样子 —— 与内容材质那一格**同形**（`BoundTexture`）。
    ///
    /// 采样器就是它自己带的那个（[`Skybox::sampler`]，`Sampler::clamped()`）：oracle 那条路
    /// 是**显式**传的（`px_render/src/scene.rs:294`），不是"落回缺省" ——
    /// 详见 [`crate::material::sampler_of`] 那段。
    ///
    /// ⚠ 这里确实 clone 了一份**装载好的字节**（星空 6 MB）：绑定请求要的是**拥有**的
    /// `BoundTexture`（与内容材质同一条路），而一帧只建一次。为省这一次 memcpy 去把
    /// `BoundTexture` 改成借来的，等于让"格 → 图"这条契约多一种形状。
    pub fn bound(&self, binding: u32) -> BoundTexture {
        BoundTexture {
            binding,
            sampler: self.sampler,
            texture: self.texture.clone(),
        }
    }
}

/// 一份材质 shader 的三样东西：原文、**本宿主组装后的自足 WGSL**、反射出来的契约。
///
/// `assembled` 是直接喂给 `create_shader_module` 的那份文本（`#import` 全展开、
/// `#{MATERIAL_BIND_GROUP}` 已替成 3）—— 运行期不再组装第二次：
/// 组装两次就是"门测的那份"与"跑的那份"开始分岔的入口（§75 的「2/3 那颗雷」）。
#[derive(Clone, Debug)]
pub struct LoadedShader {
    pub member: Member,
    pub source: String,
    pub assembled: String,
    pub layout: MaterialLayout,
    /// include 闭包的摘要（进日志：对上了也要说清对的是哪一份）。
    pub closure: String,
}

/// 几何：CAS 里的网格产物，或者一个**内建图元**（参数在文档里）。
///
/// ⚠ 图元这一支特意**把 `params` 原样留着**（而不是只留造出来的网格）：
/// 「半径 / 细分是文档里的数」这件事得能被查（判据里就查它），
/// 而且第 6 件之后要把这几个数写进日志时不必回头再读一遍文档。
#[derive(Clone, Debug)]
pub enum LoadedGeometry {
    Mesh {
        member: Member,
        mesh: Mesh,
    },
    Primitive {
        name: String,
        params: BTreeMap<String, Value>,
        mesh: Mesh,
    },
}

impl LoadedGeometry {
    pub fn mesh(&self) -> &Mesh {
        match self {
            LoadedGeometry::Mesh { mesh, .. } => mesh,
            LoadedGeometry::Primitive { mesh, .. } => mesh,
        }
    }

    pub fn member(&self) -> Option<&Member> {
        match self {
            LoadedGeometry::Mesh { member, .. } => Some(member),
            LoadedGeometry::Primitive { .. } => None,
        }
    }

    pub fn primitive(&self) -> Option<(&str, &BTreeMap<String, Value>)> {
        match self {
            LoadedGeometry::Mesh { .. } => None,
            LoadedGeometry::Primitive { name, params, .. } => Some((name.as_str(), params)),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            LoadedGeometry::Mesh { member, mesh } => format!(
                "网格 {member}（{} 顶点 / {} 三角形）",
                mesh.vertex_count(),
                mesh.triangle_count()
            ),
            LoadedGeometry::Primitive { name, mesh, .. } => format!(
                "图元 {name}（{} 顶点 / {} 三角形）",
                mesh.vertex_count(),
                mesh.triangle_count()
            ),
        }
    }
}

/// 一个物体：几何 + 材质（shader / 参数 / 贴图）+ 两条渲染状态 + 变换。
///
/// 这就是第 5、6 件要的全部输入。⚠ `alpha` / `cull` / `depth_bias` **原样带着**：
/// 它们决定混合档、剔除面与透明排序，而"哪个档是什么状态"只有一份口径（`material.rs`），
/// 这里不许先翻译一道 —— 提前翻译就是把同一个决定抄成两份。
#[derive(Clone, Debug)]
pub struct LoadedObject {
    pub id: String,
    pub geometry: LoadedGeometry,
    pub shader: LoadedShader,
    pub params: Vec<u8>,
    pub textures: Vec<BoundTexture>,
    pub alpha: AlphaMode,
    pub cull: CullMode,
    pub depth_bias: f32,
    pub cast_shadow: bool,
    pub transform: Transform,
}

impl LoadedObject {
    pub fn texture(&self, binding: u32) -> Option<&BoundTexture> {
        self.textures.iter().find(|bound| bound.binding == binding)
    }

    pub fn param_offset(&self, name: &str) -> Option<u32> {
        self.shader.layout.param(name).map(|slot| slot.offset)
    }

    /// 按**名字**读回参数块里的一个 `f32`。偏移来自反射 —— 判据里量"打包对不对"用它，
    /// 于是判据本身也不必知道那 64 字节是怎么排的。
    pub fn f32_at(&self, name: &str) -> Option<f32> {
        let slot = self.shader.layout.param(name)?;
        if slot.kind != px_protocol::material::ParamKind::F32 {
            return None;
        }
        let start = slot.offset as usize;
        let bytes = self.params.get(start..start + 4)?;
        Some(f32::from_le_bytes(bytes.try_into().ok()?))
    }
}

/// 一份装载好的文档。渲染器从这里取"这一步要画什么"，不认识行星、云、大气。
#[derive(Clone, Debug)]
pub struct LoadedScene {
    pub name: String,
    pub ambient: f32,
    pub skybox: Option<Skybox>,
    pub skybox_brightness: f32,
    pub objects: Vec<LoadedObject>,
}

impl LoadedScene {
    pub fn object(&self, id: &str) -> Option<&LoadedObject> {
        self.objects.iter().find(|object| object.id == id)
    }

    /// 给人看/进日志的装载审计。口径与 Bevy 宿主那份**不要求逐字相同**（那是报告的事，
    /// 不是像素的事），但要说清"这一步到底装了什么"：§12.2 —— 仪器不许哑掉。
    pub fn audit(&self) -> String {
        let mut lines = vec![format!(
            "渲染文档 {}｜物体 {} 个｜环境光 {}｜天空盒 {}（亮度 {}）",
            self.name,
            self.objects.len(),
            self.ambient,
            match &self.skybox {
                Some(skybox) => format!(
                    "{}｜{}×{}×{} 层｜{} 级 mip",
                    skybox.texture.label(),
                    skybox.texture.shape.width,
                    skybox.texture.shape.height,
                    skybox.texture.shape.layers,
                    skybox.texture.shape.levels
                ),
                None => "无".to_string(),
            },
            self.skybox_brightness
        )];
        for object in &self.objects {
            lines.push(format!(
                "  {}：{}｜shader {}｜参数 {} 字节｜贴图 {} 张｜{:?}/{:?}{}",
                object.id,
                object.geometry.describe(),
                object.shader.member,
                object.params.len(),
                object.textures.len(),
                object.alpha,
                object.cull,
                if object.cast_shadow {
                    ""
                } else {
                    "｜不投影"
                }
            ));
            for bound in &object.textures {
                lines.push(format!(
                    "    第 {} 格：{}｜{}×{}×{} 层｜{} 级 mip｜{}",
                    bound.binding,
                    bound.texture.label(),
                    bound.texture.shape.width,
                    bound.texture.shape.height,
                    bound.texture.shape.layers,
                    bound.texture.shape.levels,
                    bound.texture.shape.format.name()
                ));
            }
        }
        lines.join("\n")
    }
}

/// CAS 根的缺省值：`<工作区>/target/pcg`。
///
/// ⚠ **按 `CARGO_MANIFEST_DIR` 取，不按当前目录取**（`px_render` **那边**是
/// `PathBuf::from("target/pcg")` —— ⚠ 那个 `px_render` 是**已删的 Bevy 宿主**（§154），
/// 而本 crate 从 §157 起也叫 `px_render`：这一句说的是"当年那支凭相对路径解"的教训，不是本 crate）。差别不是风格：`cargo test` 的当前目录是**包目录**
/// （`px_render/`），照当前目录解会去 `px_render/target/pcg` 找一个不存在的 CAS；
/// 而服务是别的进程按绝对路径调起来的，当前目录也不归我们管。
///
/// 真跑起来仍然应该由命令行 `--pcg-root` 说了算，这个函数只是它的缺省值。
pub fn default_pcg_root() -> PathBuf {
    shader::workspace().join("target/pcg")
}

/// 读文档 + `check()`（形状 / 版本 / 引用完整性）。**不碰 CAS** ——
/// "这份文档自己说得通吗"与"它要的产物在不在"是两件事，报错也该是两条。
pub fn read_spec(scene_path: &Path) -> Result<SceneSpec, String> {
    let spec = px_protocol::scene::read_scene(scene_path)?;
    spec.check()?;
    Ok(spec)
}

/// 文档 + CAS 根 → 装载好的场景。**次序就是依赖次序**：shader 要先反射出契约，
/// 才知道参数怎么打包、哪几格是贴图（§65：渲染器不认识内容，只认识契约）。
pub fn load_scene(scene_path: &Path, pcg_root: &Path) -> Result<LoadedScene, String> {
    let spec = read_spec(scene_path)?;
    // 库表一个请求读一次就够；对账要用**这一次**盘上的库，所以不缓存到进程级。
    let modules = shader::modules();
    let skybox = match &spec.environment.skybox {
        Some(member) => Some(Skybox {
            sampler: Sampler::clamped(),
            texture: load_texture(member, pcg_root, "天空盒")?,
        }),
        None => None,
    };
    let mut objects = Vec::with_capacity(spec.objects.len());
    for object in &spec.objects {
        objects.push(load_object(object, pcg_root, &modules)?);
    }
    Ok(LoadedScene {
        name: spec.name.clone(),
        ambient: spec.environment.ambient,
        skybox,
        skybox_brightness: spec.environment.skybox_brightness,
        objects,
    })
}

fn load_object(
    object: &Object,
    pcg_root: &Path,
    modules: &px_shader::ModuleTable,
) -> Result<LoadedObject, String> {
    let id = object.id.as_str();
    let shader = load_shader(&object.material.shader, pcg_root, modules)?;
    // 参数按**它自己声明的结构体**打包（缺参 / 多参 / 类型不符都当场报错）。
    let params = shader
        .layout
        .pack(&object.material.params)
        .map_err(|err| format!("物体 '{id}' 的材质参数：{err}"))?;

    let mut textures: Vec<BoundTexture> = Vec::with_capacity(object.material.textures.len());
    for (role, reference) in &object.material.textures {
        // 产物想在某一格绑贴图，shader 就必须在那一格声明过 —— 否则这一格永远采不到，
        // 而画面"看着还行"（采到兜底白图），错到看不出来。
        let slot = shader.layout.texture(reference.binding).ok_or_else(|| {
            format!(
                "物体 '{id}' 的贴图 '{role}' 要绑在第 {} 格，但 shader {}/{} 没在那里声明贴图",
                reference.binding, shader.member.graph, shader.member.node
            )
        })?;
        let texture = load_texture(
            &reference.member,
            pcg_root,
            &format!("物体 '{id}' 的贴图 '{role}'"),
        )?;
        // 维度也要对账：cube 塞进 2D 那一格，运行期建 bind group 时才炸，而那时已经晚了。
        if texture.shape.layers != slot.dimension.layers() {
            return Err(format!(
                "物体 '{id}' 的贴图 '{role}' 是 {} 层，而 shader 第 {} 格声明的是 {}（{} 层）",
                texture.shape.layers,
                reference.binding,
                slot.dimension.name(),
                slot.dimension.layers()
            ));
        }
        textures.push(BoundTexture {
            binding: reference.binding,
            sampler: reference.sampler,
            texture,
        });
    }
    // 按格号排序：贴图表在文档里是 map（`BTreeMap` 已按角色名排），而下游要按**格号**走。
    textures.sort_by_key(|bound| bound.binding);

    Ok(LoadedObject {
        id: object.id.clone(),
        geometry: load_geometry(&object.geometry, id, pcg_root)?,
        shader,
        params,
        textures,
        alpha: object.material.alpha,
        cull: object.material.cull,
        depth_bias: object.material.depth_bias,
        cast_shadow: object.cast_shadow,
        transform: object.transform,
    })
}

/// 组装 + naga 校验 + 反射：**内容 shader 与帧自有材质共用的那一段**（§135）。
///
/// ⚠ 这一段只有一份。帧自有材质若另走一条反射路，同一份 WGSL 就会在两条路上读出两个契约
/// （§66.1 那颗「同一条契约、两个数字」的雷），而它响的方式是"某个参数打包错位、
/// 画面上只差一点点"，任何门都不会响。
///
/// `module` 一并留着，因为**入口名要在那份真模块里查**（[`fragment_entry`]）——
/// 组装后的文本与反射出来的契约是两件事，"这个名字在不在"只有 naga 说得准。
pub(crate) struct Reflected {
    pub assembled: String,
    pub layout: MaterialLayout,
    pub module: naga::Module,
}

pub(crate) fn reflect_source(
    name: &str,
    source: &str,
    modules: &px_shader::ModuleTable,
) -> Result<Reflected, String> {
    let assembled = shader::assemble(source, modules, crate::stubs::stubs);
    // 校验是**门**：坏管线当场拒，不静默出缺材质的图（§104 第 5 条把这条判据留下来了）。
    let module = shader::validate(name, &assembled)?;
    let layout = px_shader::reflect::reflect_assembled(&assembled, name)?;
    Ok(Reflected {
        assembled,
        layout,
        module,
    })
}

/// 一个（片元）入口名 → 核对它真的在那份 WGSL 里。
///
/// ⚠ 这是 §136 补上的一条守卫，代价付过：产物的 `frame_materials[].entry` 写的是
/// `fs_main`（全屏 pass 那条约定），而它自己的 WGSL 里那个函数叫 `fragment` ——
/// 名字指不到东西，于是 `sky` 那条 pass **永远建不起来**，报错来自 wgpu
/// （"找不到入口"），离病因（配方里一个词写错）已经很远。
///
/// 拒的时候**必须把实际的入口列出来**：不列的话，作者只能对着两个名字猜。
pub(crate) fn fragment_entry(module: &naga::Module, entry: &str, at: &str) -> Result<(), String> {
    let names = |stage: Option<naga::ShaderStage>| -> String {
        let found: Vec<&str> = module
            .entry_points
            .iter()
            .filter(|point| stage.is_none_or(|wanted| point.stage == wanted))
            .map(|point| point.name.as_str())
            .collect();
        if found.is_empty() {
            "（一个都没有）".to_string()
        } else {
            found.join(" / ")
        }
    };
    if module
        .entry_points
        .iter()
        .any(|point| point.name == entry && point.stage == naga::ShaderStage::Fragment)
    {
        return Ok(());
    }
    Err(format!(
        "{at} 要的片元入口 '{entry}' 在它自己的 WGSL 里不存在。\n  \
         那份 WGSL 的**片元**入口：{}\n  全部入口：{}",
        names(Some(naga::ShaderStage::Fragment)),
        names(None)
    ))
}

/// 一份组装好的 shader 的**内容键**（管线键里"哪一版 WGSL"那一格）。
///
/// 内容 shader 的键是 CAS 成员键（那里面有 include 闭包指纹）；帧自有材质没有成员，
/// 它的键就是**组装后全文**的 sha256 前 16 位 —— 同一把尺子（§17.1：键 = 内容），
/// 而且组装后的文本把 `#import` 的闭包也含进去了（库改了、文本就变、键就变）。
fn content_key(assembled: &str) -> Result<u64, String> {
    version_of(&crate::digest::sha256_hex(assembled.as_bytes()))
}

/// 一份**帧自有材质**（§135）装载好的样子：组装后的全文 + 反射出来的契约 + 打包好的参数。
///
/// 它与内容材质共用**同一条**组装 / 反射 / 打包路（[`reflect_source`] +
/// `MaterialLayout::pack`），差别只有两处，而且都是"它没有那个东西"：
///
/// 1. 文本在**文档里**（全文内联），不是 CAS 成员 ⇒ 没有 include 闭包指纹与
///    schema descriptor 可以对账 —— 那两道对账的对象是产物，而它没有产物；
/// 2. 片元入口名也在**文档里**给（内容材质那条路写死 `fragment`），所以这里要核对
///    它真的存在（[`fragment_entry`]）。
#[derive(Clone, Debug)]
pub struct LoadedFrameMaterial {
    pub name: String,
    /// 片元入口名（已核对过它在组装后的 WGSL 里存在）。
    pub entry: String,
    /// 组装后的全文（直接喂 `create_shader_module`）。
    pub assembled: String,
    /// 参数：按**它自己声明的结构体**打包（缺参 / 多参 / 类型不符三档当场报错）。
    pub params: Vec<u8>,
    /// 它声明的贴图格（格号, 维度），升序。⚠ 帧材质在文档里**没有**"哪一格是哪张图"
    /// 这一栏（内容材质有 `material.textures`），所以这一串是宿主唯一能问的东西。
    pub textures: Vec<(u32, TextureDimension)>,
    /// 内容键（见 [`content_key`]）。
    pub version: u64,
}

/// 文档里的 `frame_materials` 一节 → 可以绑上 GPU 的一份材质。
///
/// ⚠ 这里**不碰 CAS**：那份 WGSL 的全文就在文档里（那正是"帧自有"的意思：
/// 它不属于可换的内容，见 `px_protocol::scene::FrameMaterial`）。
pub fn load_frame_material(
    material: &FrameMaterial,
    modules: &px_shader::ModuleTable,
) -> Result<LoadedFrameMaterial, String> {
    let at = format!("帧自有材质 '{}'", material.name);
    let reflected = reflect_source(&at, &material.shader, modules)?;
    fragment_entry(&reflected.module, &material.entry, &at)?;
    let params = reflected
        .layout
        .pack(&material.params)
        .map_err(|err| format!("{at} 的参数：{err}"))?;
    Ok(LoadedFrameMaterial {
        name: material.name.clone(),
        entry: material.entry.clone(),
        textures: reflected
            .layout
            .textures
            .iter()
            .map(|slot| (slot.binding, slot.dimension))
            .collect(),
        version: content_key(&reflected.assembled)?,
        assembled: reflected.assembled,
        params,
    })
}

/// 装一份 shader：读产物 → **两道对账** → 组装 → naga 校验 → 反射契约。
///
/// 两道对账都是"拒绝，而不是静默出图"（与 Bevy 宿主同款，改的只是措辞）：
///
/// 1. **include 闭包**（§52.3）：`#import planet_x::…` 的真本住在
///    `art/shaders/lib/*.wgsl`（S8-a 从 `px_render/assets/shaders` 搬过来），
///    由组装器在**运行期**读盘。改了库、没重烘 ⇒
///    场景指的还是老产物、而组装用的是新库 —— 画出来的东西既不是老那一版、也不是新那一版，
///    而键 / 清单 / 场景键 / 槽版本**全都没动**：所有门都是绿的。所以在这里当场拒。
/// 2. **schema descriptor**（§74.3）：反射**规则本身**会变（契约表加宽、`ParamKind` 多一档、
///    偏移规则修正）。规则一变，同一份 WGSL 读出的是另一份契约，而产物里的参数值是按
///    **老契约**打包的 —— 装出来是另一份东西却不报错。所以拿产物记的那份规范 JSON
///    跟现在反射出来的逐字节比。
/// ⚠ `pub(crate)`：**全屏 pass 的片元阶段走的是同一条路**（`plan.rs` 拿它组装 + 反射 +
/// 打包参数）。全屏 pass 与材质共用同一份绑定契约，装载这两份 shader 的两道对账
/// （include 闭包 / schema descriptor）也就只有这一份 —— 抄一遍就是"同一个成员、两处验"。
pub(crate) fn load_shader(
    member: &Member,
    pcg_root: &Path,
    modules: &px_shader::ModuleTable,
) -> Result<LoadedShader, String> {
    const REBAKE: &str = "重烘配方：cargo run -p px_graphs --bin shaders；再逐个 cargo run -p px_graphs --bin scene <名>";
    let path = member.resolve(pcg_root)?;
    let bundle = art::read_manifest(&path)?;
    let recorded = bundle
        .assets
        .first()
        .and_then(|asset| px_shader::closure_from_params(&asset.params));
    let (source, schema) = art::read_shader_parts(&path)?;
    let closure = px_shader::closure(&source, modules);
    let fingerprint = closure.fingerprint();
    match recorded {
        Some(value) if value == fingerprint => {}
        Some(value) => {
            return Err(format!(
                "shader 成员 {member} 的 include 闭包对不上：产物记的 {value:016x}｜盘上现在的 {fingerprint:016x}\n  \
                 ⇒ 这份产物是拿另一版 include 烘的，画出来既不是老那一版也不是新那一版。\n  {REBAKE}"
            ));
        }
        None => {
            return Err(format!(
                "shader 成员 {member} 的产物没有 include 闭包指纹。\n  {REBAKE}"
            ));
        }
    }

    let name = format!("{}/{}", member.graph, member.node);
    // ⚠ 组装 / 校验 / 反射走的是与帧自有材质**同一条**路（[`reflect_source`]）：
    //    这里与它只差那两道对账（它们的对账对象是产物，帧材质没有产物）。
    let reflected = reflect_source(&name, &source, modules)?;
    let layout = reflected.layout;
    let now = layout.to_json()?;
    match &schema {
        Some(text) if *text == now => {}
        Some(text) => {
            return Err(format!(
                "shader 成员 {member} 的 schema descriptor 对不上：\n  产物记的 {text}\n  现在的   {now}\n  {REBAKE}"
            ));
        }
        None => {
            return Err(format!(
                "shader 成员 {member} 的产物没有 schema descriptor。\n  {REBAKE}"
            ));
        }
    }

    Ok(LoadedShader {
        member: member.clone(),
        source,
        assembled: reflected.assembled,
        layout,
        closure: closure.summary(),
    })
}

/// 贴图产物 → 形状 + 整条 mip 链 + 预览。
///
/// 形状（宽高 / 层数 / mip 级数 / 格式）**只信清单参数**，并且逐项与载荷字节数对账：
/// 对不上就是产物坏了，当场报错，不许"按能读的读一部分"。
fn load_texture(member: &Member, pcg_root: &Path, what: &str) -> Result<LoadedTexture, String> {
    let path = member.resolve(pcg_root)?;
    let bytes = std::fs::read(&path).map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    let frames = px_protocol::stream::read_stream(&mut bytes.as_slice())
        .map_err(|err| format!("{what} {member}（{}）不是产物流：{err}", path.display()))?;
    let manifest = frames
        .iter()
        .find_map(|frame| match frame {
            px_protocol::stream::Frame::Art(bundle) => bundle.assets.first(),
            _ => None,
        })
        .ok_or_else(|| format!("{what} {member}（{}）里没有清单帧", path.display()))?;
    if manifest.kind != AssetKind::Texture {
        return Err(format!(
            "{what} {member}（{}）不是 Texture 产物：{:?}",
            path.display(),
            manifest.kind
        ));
    }
    let shape = TextureShape::from_params(&manifest.params)
        .map_err(|err| format!("{what} {member}：{err}"))?;
    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            px_protocol::stream::Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .ok_or_else(|| format!("{what} {member}（{}）里没有载荷块", path.display()))?;
    // 位深与格式是同一个事实的两半：`rgba8_srgb` 的载荷必须是 U8、`rgba16_float` 必须是 U16。
    let expected = match shape.format {
        TextureFormat::Rgba8Srgb => DType::U8,
        TextureFormat::Rgba16Float => DType::U16,
    };
    if blob.header.dtype != expected {
        return Err(format!(
            "{what} {member}：格式是 {} 但载荷位深是 {:?}（应当是 {expected:?}）",
            shape.format.name(),
            blob.header.dtype
        ));
    }
    if blob.bytes.len() != shape.chain_bytes() {
        return Err(format!(
            "{what} {member}：{}×{}×{} 层 {} 级 mip 应当是 {} 字节，实际 {} 字节",
            shape.width,
            shape.height,
            shape.layers,
            shape.levels,
            shape.chain_bytes(),
            blob.bytes.len()
        ));
    }
    let image = decode_base_level(&shape, &blob.bytes)?;
    Ok(LoadedTexture {
        member: member.clone(),
        shape,
        bytes: blob.bytes.clone(),
        image,
    })
}

fn load_geometry(geometry: &Geometry, id: &str, pcg_root: &Path) -> Result<LoadedGeometry, String> {
    match geometry {
        Geometry::Mesh { member, .. } => {
            let path = member.resolve(pcg_root)?;
            // 审计文本照打（焊接 / 缠绕 / 法线）：它是"这张网格长什么样"的唯一仪器，
            // 而 §102 记过两次搬运里那几个括号**改的是像素**，所以它必须一直看得见。
            let (mesh, audit) = crate::mesh::load_mesh(&path.display().to_string())
                .map_err(|err| format!("物体 '{id}'：{err}"))?;
            println!("{}", audit.trim_end());
            Ok(LoadedGeometry::Mesh {
                member: member.clone(),
                mesh,
            })
        }
        Geometry::Primitive { name, params, .. } => {
            let mesh = primitive_mesh(name, params).map_err(|err| format!("物体 '{id}'：{err}"))?;
            Ok(LoadedGeometry::Primitive {
                name: name.clone(),
                params: params.clone(),
                mesh,
            })
        }
    }
}

/// 内建图元 → 网格。**参数全部来自文档**，一个数都不写死。
///
/// ⚠ 现在只认 `icosphere`：`px_render/src/icosphere.rs` 只移植了细分球。
/// Bevy 宿主还有 `uv_sphere`（`Sphere::uv`）—— 那一支**故意不装**：
/// 移植件没有、拿一个"看起来差不多"的球去顶，就是让两条宿主在同一个场景上出两张不同的图，
/// 而报错信息比一张对不上的图便宜得多。要用它的那一天，先把移植件补上（`target/oracle/`
/// 里有 `bevy-uvsphere-*.bin` 三份现成的 oracle 可以对着逐字节比）。
pub fn primitive_mesh(name: &str, params: &BTreeMap<String, Value>) -> Result<Mesh, String> {
    let number = |key: &str| -> Result<f32, String> {
        match params.get(key) {
            Some(Value::Num(value)) => Ok(*value as f32),
            Some(other) => Err(format!(
                "图元 '{name}' 的参数 '{key}' 要一个数，实际是 {other:?}"
            )),
            None => Err(format!(
                "图元 '{name}' 缺参数 '{key}'；它有的参数：{}",
                if params.is_empty() {
                    "（空）".to_string()
                } else {
                    params.keys().cloned().collect::<Vec<_>>().join(" / ")
                }
            )),
        }
    };
    match name {
        "icosphere" => {
            let radius = number("radius")?;
            let subdivisions = number("subdivisions")?;
            // 上界 64 与 Bevy 那边一致（`Sphere::ico` 在 80 上会 Err，而 `MeshBuilder::build()`
            // 直接 unwrap ⇒ 越界不是"图难看"，是 panic）；下界 1 同款。
            if !(subdivisions.fract() == 0.0 && (1.0..=64.0).contains(&subdivisions)) {
                return Err(format!(
                    "图元 'icosphere' 的 'subdivisions' 是 {subdivisions}：要 1..=64 的整数"
                ));
            }
            Ok(crate::icosphere::icosphere(radius, subdivisions as u32))
        }
        other => Err(format!("不认识的图元 '{other}'；这份渲染器认：icosphere")),
    }
}

/// 最细那一级 mip → `RgbaImage`（**预览**，不是上传源）。
///
/// 立方图把 6 个面竖着码（`height * layers`）；`Rgba16Float` 走半精度 → f32 → 8 位
/// （截到 [0,1]，因为预览就是给人看的）。⚠ 这条有损路径**只服务预览**：上传要用
/// [`LoadedTexture::bytes`] 那一条。
fn decode_base_level(shape: &TextureShape, bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let texels = shape.width as usize * shape.height as usize * shape.layers as usize;
    let level_bytes = texels * shape.format.texel_bytes();
    let level = bytes.get(..level_bytes).ok_or_else(|| {
        format!(
            "载荷只有 {} 字节，最细那一级就要 {level_bytes}",
            bytes.len()
        )
    })?;
    let mut rgba = Vec::with_capacity(texels * 4);
    match shape.format {
        TextureFormat::Rgba8Srgb => rgba.extend_from_slice(level),
        TextureFormat::Rgba16Float => {
            for texel in level.chunks_exact(8) {
                for channel in 0..4 {
                    let bits = u16::from_le_bytes([texel[channel * 2], texel[channel * 2 + 1]]);
                    let value = half_to_f32(bits).clamp(0.0, 1.0);
                    rgba.push((value * 255.0 + 0.5) as u8);
                }
            }
        }
    }
    let height = shape.height * shape.layers;
    image::RgbaImage::from_raw(shape.width, height, rgba)
        .ok_or_else(|| format!("解出来的字节数与 {}×{} 对不上", shape.width, height))
}

/// IEEE 754 半精度 → 单精度。
///
/// ⚠ 手写而不是引 `half`：预览这条路只需要"读得懂"，而半精度→单精度是**精确**的
/// （除次正规数外不丢位），十行就够；为它多一个依赖，等于让每个新克隆多编一个 crate。
/// 次正规数与 ±∞ / NaN 两支都按标准处理（判据里各钉了一格）。
pub fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1F);
    let mantissa = u32::from(bits & 0x03FF);
    let value = match exponent {
        0 => {
            if mantissa == 0 {
                // ±0
                sign
            } else {
                // 次正规数：左移到第一个 1 进隐含位，指数跟着减。
                let mut mantissa = mantissa;
                let mut exponent = 127 - 15 + 1;
                while mantissa & 0x0400 == 0 {
                    mantissa <<= 1;
                    exponent -= 1;
                }
                sign | (exponent << 23) | ((mantissa & 0x03FF) << 13)
            }
        }
        // ±∞ / NaN：指数全 1，尾数搬过去即可。
        0x1F => sign | 0x7F80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 127 - 15) << 23) | (mantissa << 13),
    };
    f32::from_bits(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest;

    /// 判据的锚：`target/oracle/orbit-bare-nolight.txt`（一行，内容是场景产物的绝对路径）。
    const SCENE_LIST: &str = "target/oracle/orbit-bare-nolight.txt";

    /// 锚在不在。**`target/` 不入 git** ⇒ 新克隆上它一定不在，而"硬失败"会让新克隆
    /// 因为一个不是回归的原因变红。所以这里返回 `None`，由调用方**大声**说明"没测"。
    fn scene_path() -> Option<PathBuf> {
        let list = shader::workspace().join(SCENE_LIST);
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

    /// 装载这一档的判据入口。
    ///
    /// ⚠ 三道门（清单文件 / 场景产物 / CAS 根）任一不在都**只打印不判**，
    /// 而且打印里明说"不是通过，是没测"—— 静默跳过（`#[ignore]`、直接 `return`）是
    /// 判据的敌人：它会让"这一档从来没跑过"看起来跟"这一档一直是对的"一样（§101）。
    /// 反过来说，能跑的时候它必须**硬失败**：任何一个成员解析不出来都是错，不是警告。
    fn loaded() -> Option<LoadedScene> {
        let path = scene_path()?;
        let root = default_pcg_root();
        if !root.exists() {
            println!("⚠ 跳过：CAS 根 {} 不在（target/ 不入 git）", root.display());
            return None;
        }
        Some(
            load_scene(&path, &root)
                .unwrap_or_else(|err| panic!("载入 {} 失败：{err}", path.display())),
        )
    }

    fn number(params: &BTreeMap<String, Value>, key: &str) -> f64 {
        match params.get(key) {
            Some(Value::Num(value)) => *value,
            other => panic!("参数 '{key}' 不是一个数：{other:?}"),
        }
    }

    fn same_mesh(one: &Mesh, other: &Mesh) -> bool {
        one.positions == other.positions
            && one.normals == other.normals
            && one.uvs == other.uvs
            && one.indices == other.indices
    }

    /// 网格的内容摘要，口径与 §110.3 那张 oracle 表**逐字相同**
    /// （小端拼 `positions → normals → uvs → indices`）—— 口径不一致的话，
    /// 下面拿它跟 oracle 对账的那一格就成了自说自话。
    fn digest_of(mesh: &Mesh) -> String {
        let mut bytes = Vec::new();
        for position in &mesh.positions {
            for value in position {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for normal in &mesh.normals {
            for value in normal {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for uv in &mesh.uvs {
            for value in uv {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        for index in &mesh.indices {
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        digest::sha256_hex(&bytes)[..16].to_uppercase()
    }

    /// 本档的判据：五类成员**每一个**都解得出来，而且解出来的就是烘图侧印的那些值
    /// （表里的期望值来自烘图侧的日志，一个字都不许改 —— 改了就是改判据）。
    #[test]
    fn the_orbit_bare_nolight_scene_resolves_member_by_member() {
        let Some(scene) = loaded() else {
            println!("⚠ 这一档判据没跑（见上面那行）：不是通过，是没测");
            return;
        };
        println!("{}", scene.audit());

        let ids: Vec<&str> = scene
            .objects
            .iter()
            .map(|object| object.id.as_str())
            .collect();
        assert_eq!(ids, vec!["planet", "atmosphere"], "物体表");
        assert_eq!(scene.ambient, 80.0, "环境光");

        let planet = scene.object("planet").expect("planet 在");
        assert_eq!(
            planet.geometry.member().expect("网格成员").to_string(),
            "planet/surface@f5d69bcdde79"
        );
        assert_eq!(
            planet.shader.member.to_string(),
            "shaders/surface@f679cdf81015"
        );
        assert_eq!(
            planet
                .texture(1)
                .expect("第 1 格贴图")
                .texture
                .member
                .to_string(),
            "generated/surface_color@94acc1920c4a"
        );
        assert_eq!(planet.alpha, AlphaMode::Opaque);
        assert_eq!(planet.cull, CullMode::Back);
        assert!(planet.cast_shadow);
        assert_eq!(planet.textures.len(), 1);
        // 顶点 / 三角形数与产物清单里的 `params` 对得上：网格真的整个读进来了。
        assert_eq!(planet.geometry.mesh().vertex_count(), 155_526);
        assert_eq!(planet.geometry.mesh().triangle_count(), 307_200);
        // 预览的最细一级 = 780×520；整条链是 10 级 —— 上传要用的是**后者**。
        assert_eq!(
            planet
                .texture(1)
                .expect("第 1 格")
                .texture
                .image
                .dimensions(),
            (780, 520)
        );
        assert_eq!(planet.texture(1).expect("第 1 格").texture.shape.levels, 10);
        assert_eq!(
            planet.texture(1).expect("第 1 格").texture.bytes.len(),
            2_162_808
        );
        // 参数块的长度是**反射出来的**那个数，不是猜的。
        assert_eq!(
            planet.params.len(),
            planet.shader.layout.params_bytes as usize
        );
        // 值也要对：按名字读回来（偏移来自反射）。
        assert_eq!(planet.f32_at("gain"), Some(2.0));
        assert_eq!(planet.f32_at("coverage"), Some(0.3499999940395355));
        assert_eq!(planet.f32_at("height"), Some(0.5));
        println!(
            "planet｜网格 {}｜shader {}｜参数 {} 字节｜第 1 格 {}",
            planet.geometry.describe(),
            planet.shader.member,
            planet.params.len(),
            planet.texture(1).expect("第 1 格").texture.label()
        );

        let atmosphere = scene.object("atmosphere").expect("atmosphere 在");
        let (name, params) = atmosphere.geometry.primitive().expect("图元");
        assert_eq!(name, "icosphere");
        // ⚠ 期望值是**文档里那串 f64**（= f32 的 1.14）：`as f32` 那一步不许改成 `as f64` 再取整。
        assert_eq!(number(params, "radius"), 1.1399999856948853);
        assert_eq!(number(params, "subdivisions"), 64.0);
        assert_eq!(
            atmosphere.shader.member.to_string(),
            "shaders/atmosphere@d4501946bb0c"
        );
        assert_eq!(atmosphere.alpha, AlphaMode::Add);
        assert_eq!(atmosphere.cull, CullMode::Back);
        assert!(!atmosphere.cast_shadow);
        assert!(atmosphere.textures.is_empty());
        assert_eq!(atmosphere.geometry.mesh().vertex_count(), 42_252);
        assert_eq!(atmosphere.geometry.mesh().triangle_count(), 84_500);
        println!(
            "atmosphere｜{}｜shader {}｜参数 {} 字节｜{:?}/{:?}",
            atmosphere.geometry.describe(),
            atmosphere.shader.member,
            atmosphere.params.len(),
            atmosphere.alpha,
            atmosphere.cull
        );

        let skybox = scene.skybox.as_ref().expect("天空盒");
        assert_eq!(
            skybox.texture.member.to_string(),
            "generated/stars@bc2ac082b43d"
        );
        assert_eq!(skybox.texture.shape.layers, 6);
        assert_eq!(skybox.texture.shape.levels, 1);
        // 6 个面竖着码的预览口径（512×3072）。
        assert_eq!(skybox.texture.image.dimensions(), (512, 3072));
        println!(
            "天空盒｜{}｜{}×{}×{} 层｜{} 级 mip｜{} 字节",
            skybox.texture.label(),
            skybox.texture.shape.width,
            skybox.texture.shape.height,
            skybox.texture.shape.layers,
            skybox.texture.shape.levels,
            skybox.texture.bytes.len()
        );
    }

    /// "半径 / 细分是**文档里的数**，不是写死的常数" —— 三路取证。
    ///
    /// 一条判据最容易被自己骗过去的地方是：`primitive_mesh` 与"期望值"都是我们写的，
    /// 两边一起错就是绿的。所以这里一路对着**外部真值**（§110.3 的 oracle 摘要）比，
    /// 另外两路把"文档 → 网格"这条链的两端各自钉住。
    #[test]
    fn the_primitive_parameters_come_from_the_document() {
        let Some(scene) = loaded() else {
            println!("⚠ 这一档判据没跑（见上面那行）：不是通过，是没测");
            return;
        };
        let atmosphere = scene.object("atmosphere").expect("atmosphere 在");
        let (name, params) = atmosphere.geometry.primitive().expect("图元");
        let radius = number(params, "radius") as f32;
        assert_eq!(radius.to_bits(), 1.14_f32.to_bits());
        assert_eq!(number(params, "subdivisions"), 64.0);

        // 路（1）：物体里那份网格 **就是** 文档里那几个参数造出来的那一份。
        let from_document = primitive_mesh(name, params).expect("按文档再造一份");
        assert!(
            same_mesh(atmosphere.geometry.mesh(), &from_document),
            "物体里那份网格必须就是**文档里那几个参数**造出来的那一份"
        );
        // 路（2）：摘要口径先对着外部 oracle 验一次（半径 1.0、细分 64 那一格），
        // 否则下面那个"等于 1.14 那份"可能只是两边一起错。
        assert_eq!(
            digest_of(&crate::icosphere::icosphere(1.0, 64)),
            "B4B37AB464C3A743",
            "摘要口径必须与 §110.3 那张 oracle 表一致（小端拼 positions → normals → uvs → indices）"
        );
        assert_eq!(
            digest_of(atmosphere.geometry.mesh()),
            digest_of(&crate::icosphere::icosphere(1.14, 64)),
            "文档里那个半径真的进了位置（1.14 那份与 1.0 那份不是同一网格）"
        );
        assert_ne!(digest_of(atmosphere.geometry.mesh()), "B4B37AB464C3A743");

        // 路（3）：改一个数，网格必须跟着变 —— 常数（写死的半径）不会跟着变。
        let mut bigger = params.clone();
        bigger.insert("radius".to_string(), Value::Num(7.5));
        let other = primitive_mesh(name, &bigger).expect("换个半径再造一份");
        assert!(
            !same_mesh(&other, atmosphere.geometry.mesh()),
            "半径换了网格必须跟着换 —— 说明它读的是文档里的数，不是写死的常数"
        );
        assert_eq!(
            other.vertex_count(),
            atmosphere.geometry.mesh().vertex_count()
        );
        let worst = other
            .positions
            .iter()
            .map(|position| {
                (position[0] * position[0] + position[1] * position[1] + position[2] * position[2])
                    .sqrt()
            })
            .fold(0.0_f32, f32::max);
        assert!(
            (worst - 7.5).abs() < 1e-4,
            "换过半径的那份外接半径是 {worst}"
        );

        let mut denser = params.clone();
        let dense = 8.0;
        denser.insert("subdivisions".to_string(), Value::Num(dense));
        let coarse = primitive_mesh(name, &denser).expect("换个细分再造一份");
        assert_eq!(
            coarse.vertex_count(),
            10 * (dense as usize + 1).pow(2) + 2,
            "细分改了顶点数就该按闭式 10(s+1)²+2 变"
        );
        assert_ne!(
            coarse.indices.len(),
            atmosphere.geometry.mesh().indices.len()
        );
    }

    /// 解码那条路**不依赖 `target/`**：新克隆上唯一还能跑的判据就是它（所以它必须自己站得住）。
    /// 8 位直通、半精度五位（0 / 0.5 / 1 / 2 / −1）、立方图竖码、以及各级形状的字节数都钉住。
    #[test]
    fn the_base_level_decoder_handles_both_formats() {
        let shape = TextureShape {
            width: 2,
            height: 2,
            layers: 1,
            levels: 1,
            format: TextureFormat::Rgba8Srgb,
        };
        let bytes: Vec<u8> = (0..16).collect();
        let image = decode_base_level(&shape, &bytes).expect("8 位解码");
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.get_pixel(1, 1).0, [12, 13, 14, 15]);

        let float = TextureShape {
            width: 2,
            height: 1,
            layers: 1,
            levels: 1,
            format: TextureFormat::Rgba16Float,
        };
        let mut payload = Vec::new();
        for bits in [
            0x3C00u16, 0x3800, 0x0000, 0x4000, 0xBC00, 0x7C00, 0x0001, 0x3555,
        ] {
            payload.extend_from_slice(&bits.to_le_bytes());
        }
        // 半精度那五档：1 / 0.5 / 2 / −1 / 0（外加 ±∞ 与次正规数走一遍不 panic）。
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0x4000), 2.0);
        assert_eq!(half_to_f32(0xBC00), -1.0);
        assert_eq!(half_to_f32(0x0000), 0.0);
        let image = decode_base_level(&float, &payload).expect("半精度解码");
        assert_eq!(image.dimensions(), (2, 1));
        // 第二个 texel 是 (−1, +∞, 次正规数, 0.3333) ⇒ 截到 [0,1] 再落 8 位。
        assert_eq!(image.get_pixel(0, 0).0, [255, 128, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 255, 0, 85]);

        let cube = TextureShape {
            width: 2,
            height: 2,
            layers: 6,
            levels: 1,
            format: TextureFormat::Rgba8Srgb,
        };
        let image = decode_base_level(&cube, &vec![7_u8; 2 * 2 * 6 * 4]).expect("立方图解码");
        assert_eq!(image.dimensions(), (2, 12));
    }

    /// 一份**帧自有材质**的夹具：WGSL 用的是**盘上那份真本**
    /// （`art/frame/skybox.wgsl`，它在 git 里 ⇒ 这一档不依赖 `target/`），
    /// 参数取烘图时那一份值（`environment.skybox_brightness`）。
    fn frame_material_fixture(entry: &str) -> px_protocol::scene::FrameMaterial {
        let path = shader::workspace().join("art/frame/skybox.wgsl");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        let mut params = BTreeMap::new();
        params.insert("brightness".to_string(), Value::Num(900.0));
        px_protocol::scene::FrameMaterial {
            name: "skybox".to_string(),
            shader: text,
            entry: entry.to_string(),
            params,
        }
    }

    /// **帧自有材质**装载这一档（§135/§136）：与内容材质**同一条**组装 / 反射 / 打包路。
    ///
    /// 期望值来自烘图侧的日志（`px_scene::frame::bake_material` 印的那一版）：
    /// 入口 `fragment`、参数一个 `f32`、只声明第 5 格（Cube）。
    #[test]
    fn the_frame_material_is_reflected_and_packed_like_a_content_material() {
        let declared = frame_material_fixture("fragment");
        let loaded = load_frame_material(&declared, &shader::modules()).expect("装载帧材质");

        assert_eq!(loaded.name, "skybox");
        assert_eq!(
            loaded.entry, "fragment",
            "入口名是文档给的那个（已核对它存在）"
        );
        // 声明的格子**从反射来**：第 5 格是契约表里那几档 cube 之一。
        assert_eq!(loaded.textures, vec![(5, TextureDimension::Cube)]);
        // 参数按**它自己声明的结构体**打包：一个 f32，补齐到 16 字节（WGSL 的 uniform 对齐）。
        assert_eq!(loaded.params.len(), 16);
        assert_eq!(
            f32::from_le_bytes(loaded.params[..4].try_into().expect("四个字节")),
            900.0
        );
        // ⚠ 曝光那一乘**必须真的在组装后的文本里**：它是策略，不是注释。
        //    少了它整幅背景会被推到 255（§136 实测：非黑像素数一样、值全错）。
        assert!(
            loaded
                .assembled
                .contains("params.brightness * view.exposure"),
            "亮度那一格要乘上相机的曝光（oracle：`skybox.brightness * exposure`）"
        );
        // 内容键 = 组装后全文的 sha256 前 16 位（它没有 CAS 成员，见 [`content_key`]）。
        assert_eq!(
            loaded.version,
            version_of(&crate::digest::sha256_hex(loaded.assembled.as_bytes())).expect("内容键")
        );
        println!(
            "帧自有材质 '{}'：entry {}｜组装后 {} 字节｜参数 {} 字节｜贴图格 {:?}",
            loaded.name,
            loaded.entry,
            loaded.assembled.len(),
            loaded.params.len(),
            loaded.textures
        );
    }

    /// 入口名指不到东西 ⇒ **宿主当场拒**，并把那份 WGSL 实际的入口列出来（§136）。
    ///
    /// ⚠ 这一条与烘图侧那条（`px_scene::frame::bake_material` 的第 ⑥ 条）是**两道**守卫，
    /// 不是重复：烘图侧拦的是"配方写错了"，这里拦的是"手上这份文本里的名字指不到东西"
    /// （老产物、手改的文档都会走到这一条）。
    #[test]
    fn a_frame_material_entry_that_does_not_exist_is_refused_by_name() {
        // 全屏 pass 那条约定（`art/shaders/blit.wgsl` 的入口就叫这个）——
        // 这正是 §136 实测踩到的那一个词。
        let declared = frame_material_fixture("fs_main");
        let err = load_frame_material(&declared, &shader::modules()).expect_err("指不到 ⇒ 拒");
        assert!(err.contains("fs_main"), "要点名那个指不到的名字：{err}");
        assert!(
            err.contains("fragment"),
            "要把那份 WGSL 实际的入口列出来：{err}"
        );
        assert_eq!(
            err.matches("fragment").count(),
            2,
            "片元那一行与「全部入口」那一行都要有：{err}"
        );
    }
}
