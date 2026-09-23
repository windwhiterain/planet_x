use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::art::Camera;

/// 场景描述的形状版本。它变了 ⇒ 旧场景产物不该再被当作同一份东西。
///
/// v2：从「行星配方」（part.kind = planet / clouds / atmosphere）改成**通用渲染文档** ——
/// 物体表（几何 + 材质 + 变换）、光源表、相机表、环境。格式本身与渲染器都不认识
/// 「行星 / 云 / 大气」：那些语义住在烘图侧的场景编译器里。
pub const SCENE_SCHEMA: u32 = 2;

/// CAS 里一个内容键的落盘规则。烘图侧与渲染侧共用这一份 ——
/// 两边各写一遍「键 → 路径」就是又一个「同一个键、不同内容」的入口。
pub fn cas_path(root: &Path, key: &str) -> Result<PathBuf, String> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("内容键应当是 64 位十六进制，实际是 '{key}'"));
    }
    let lower = key.to_ascii_lowercase();
    Ok(root
        .join("ab")
        .join(&lower[..2])
        .join(format!("{lower}.pxart")))
}

/// 场景对某个成员的引用：**图名 + 节点名 + 当时的键**。
/// 名字给人看与报错，键给渲染器取产物（键 = 内容）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub graph: String,
    pub node: String,
    pub key: String,
}

impl Member {
    pub fn new(graph: &str, node: &str, key: &str) -> Self {
        Self {
            graph: graph.to_string(),
            node: node.to_string(),
            key: key.to_string(),
        }
    }

    pub fn short_key(&self) -> &str {
        &self.key[..self.key.len().min(12)]
    }

    pub fn resolve(&self, root: &Path) -> Result<PathBuf, String> {
        let path = cas_path(root, &self.key)?;
        if !path.exists() {
            return Err(format!(
                "场景要的成员不在 CAS：{}/{}（键 {}）→ {}",
                self.graph,
                self.node,
                self.short_key(),
                path.display()
            ));
        }
        Ok(path)
    }
}

impl std::fmt::Display for Member {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{}/{}@{}",
            self.graph,
            self.node,
            self.short_key()
        )
    }
}

/// 材质参数的一个值。JSON 里就是 `0.35` / `"rocky"` / `[0.44,0.64,0.98]` / `[x,y,z,w]` 四种写法。
/// **类型由 shader 决定**：渲染器把 `Num` 按 WGSL 结构体里那一格的类型打包
/// （`f32` / `u32` / `i32`），把 `Triple` 打成 `vec3`、`Quad` 打成 `vec4`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Num(f64),
    Text(String),
    Triple([f32; 3]),
    Quad([f32; 4]),
}

fn keys_of<T>(map: &BTreeMap<String, T>) -> String {
    if map.is_empty() {
        return "（空）".to_string();
    }
    map.keys().cloned().collect::<Vec<_>>().join(" / ")
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Num(_) => "数",
        Value::Text(_) => "文本",
        Value::Triple(_) => "三个数",
        Value::Quad(_) => "四个数",
    }
}

/// 物体在**世界系**里的位置 / 旋转（四元数 xyzw）/ 缩放。
/// 没有父子层级：产物给的就是最终变换 —— 层级是渲染器的事，而"谁挂在谁下面"
/// 是内容，烘图侧比渲染器更清楚。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    #[serde(default)]
    pub translation: [f32; 3],
    #[serde(default = "identity_rotation")]
    pub rotation: [f32; 4],
    #[serde(default = "unit_scale")]
    pub scale: [f32; 3],
}

fn identity_rotation() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

fn unit_scale() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: identity_rotation(),
            scale: unit_scale(),
        }
    }
}

impl Transform {
    pub fn at(translation: [f32; 3]) -> Self {
        Self {
            translation,
            ..Default::default()
        }
    }

    pub fn rotated(rotation: [f32; 4]) -> Self {
        Self {
            rotation,
            ..Default::default()
        }
    }

    pub fn translated(mut self, translation: [f32; 3]) -> Self {
        self.translation = translation;
        self
    }

    pub fn scaled(mut self, scale: f32) -> Self {
        self.scale = [scale; 3];
        self
    }
}

/// 几何：CAS 里的网格产物，或者一个**内建图元**。
///
/// 图元不是"行星"：球/环这种形状是任何渲染器都有的东西（`Sphere`、`Ring`），
/// 而消融档（`orbit-soft-shell` 用一个细分球壳）要的正是"同一个球壳"。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Mesh {
        member: Member,
    },
    Primitive {
        name: String,
        #[serde(default)]
        params: BTreeMap<String, Value>,
    },
}

impl Geometry {
    pub fn mesh(member: Member) -> Self {
        Self::Mesh { member }
    }

    pub fn primitive(name: &str, params: BTreeMap<String, Value>) -> Self {
        Self::Primitive {
            name: name.to_string(),
            params,
        }
    }

    pub fn member(&self) -> Option<&Member> {
        match self {
            Self::Mesh { member } => Some(member),
            Self::Primitive { .. } => None,
        }
    }
}

/// 贴图采样的地址模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Address {
    #[default]
    Repeat,
    ClampToEdge,
    MirrorRepeat,
}

/// 过滤方式。线性 = 今天是全部贴图的默认。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    #[default]
    Linear,
    Nearest,
}

/// 采样器：**住在产物里**（"这张图该怎么采"跟图一起走）。
/// 渲染器按它建 `Image` 自带的采样器 —— 图与采样器都从文档来，渲染器不猜。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sampler {
    #[serde(default)]
    pub address_u: Address,
    #[serde(default = "clamp_edge")]
    pub address_v: Address,
    #[serde(default)]
    pub filter: Filter,
    /// 各向异性上限。0 = 不设（线性 mip 过滤）。
    #[serde(default)]
    pub anisotropy: u32,
}

fn clamp_edge() -> Address {
    Address::ClampToEdge
}

impl Default for Sampler {
    fn default() -> Self {
        Self {
            address_u: Address::Repeat,
            address_v: Address::ClampToEdge,
            filter: Filter::Linear,
            anisotropy: 0,
        }
    }
}

impl Sampler {
    pub fn repeat() -> Self {
        Self {
            address_u: Address::Repeat,
            address_v: Address::ClampToEdge,
            filter: Filter::Linear,
            anisotropy: 8,
        }
    }

    pub fn clamped() -> Self {
        Self {
            address_u: Address::ClampToEdge,
            address_v: Address::ClampToEdge,
            filter: Filter::Linear,
            anisotropy: 0,
        }
    }
}

/// 材质里的一张贴图：绑到材质绑定组的哪一格，用哪份产物，怎么采。
///
/// **绑定下标就是契约**：shader 得在那一格声明贴图，`+1` 那一格声明采样器。
/// 渲染器按反射出来的声明校验维度（2D / cube）与产物是不是同一回事 —— 对不上当场报错。
/// 哪几格能放贴图**只有一份来源**：`crate::material::TEXTURE_SLOTS`（§74.3 的契约收口）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureRef {
    pub binding: u32,
    pub member: Member,
    #[serde(default)]
    pub sampler: Sampler,
}

impl TextureRef {
    pub fn new(binding: u32, member: Member, sampler: Sampler) -> Self {
        Self {
            binding,
            member,
            sampler,
        }
    }

    pub fn sampler_binding(&self) -> u32 {
        self.binding + 1
    }
}

/// 混合档。云是 `Premultiplied`、大气是 `Add`、环是 `Blend`、其余是 `Opaque`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    #[default]
    Opaque,
    Premultiplied,
    Blend,
    Add,
}

/// 三角形朝向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CullMode {
    #[default]
    Back,
    Front,
    None,
}

/// 材质 = **一份 WGSL 产物 + 一袋按名字给的参数 + 按绑定下标给的贴图 + 两条渲染状态**。
/// 渲染器不认识参数是什么意思：它把参数按 shader 自己声明的结构体打包（反射，见 `px_render::reflect`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    /// WGSL 产物（`kind = Shader`）。绑定布局由它自己声明。
    pub shader: Member,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub textures: BTreeMap<String, TextureRef>,
    #[serde(default)]
    pub alpha: AlphaMode,
    #[serde(default)]
    pub cull: CullMode,
    /// 深度偏置（Bevy `Material::depth_bias` 的语义：深度纹理单位）。云壳压 −1 让它压在行星之后画。
    #[serde(default)]
    pub depth_bias: f32,
}

impl Material {
    pub fn new(shader: Member) -> Self {
        Self {
            shader,
            params: BTreeMap::new(),
            textures: BTreeMap::new(),
            alpha: AlphaMode::Opaque,
            cull: CullMode::Back,
            depth_bias: 0.0,
        }
    }

    pub fn with_params(mut self, params: BTreeMap<String, Value>) -> Self {
        self.params = params;
        self
    }

    pub fn with_texture(mut self, role: &str, texture: TextureRef) -> Self {
        self.textures.insert(role.to_string(), texture);
        self
    }

    pub fn number(&self, key: &str) -> Result<f64, String> {
        match self.params.get(key) {
            Some(Value::Num(value)) => Ok(*value),
            Some(other) => Err(format!(
                "材质参数 '{key}' 要一个数，实际是{}",
                value_kind(other)
            )),
            None => Err(format!(
                "材质缺参数 '{key}'；这份有：{}",
                keys_of(&self.params)
            )),
        }
    }

    pub fn number_or(&self, key: &str, fallback: f64) -> f64 {
        match self.params.get(key) {
            Some(Value::Num(value)) => *value,
            _ => fallback,
        }
    }
}

/// 场景里的一个物体。**没有 kind**：几何 + 材质 + 变换就是全部。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    /// 实体身份。重建场景时按它配对，所以同一份场景里不许重名。
    pub id: String,
    pub geometry: Geometry,
    pub material: Material,
    #[serde(default)]
    pub transform: Transform,
    /// 投不投阴影。云壳**不投**（它是一整颗球，进 shadow map 就是一颗球形硬影）。
    #[serde(default = "cast_shadow_default")]
    pub cast_shadow: bool,
}

fn cast_shadow_default() -> bool {
    true
}

/// 光源种类。今天只有点光源在用（宇宙里没有平行光，§64.9），另外两种是通用渲染该有的：
/// 用不用由产物说了算。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
    Spot,
    Directional,
}

/// 一盏灯。位置 / 方向 / 颜色 / 强度 / 开不开影全部来自产物 —— 渲染器里没有
/// `SUN_DIRECTION` 这种常量（§60）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Light {
    pub id: String,
    pub kind: LightKind,
    #[serde(default)]
    pub position: [f32; 3],
    /// 聚光与平行光用。点光源忽略。
    #[serde(default = "forward")]
    pub direction: [f32; 3],
    #[serde(default = "white")]
    pub color: [f32; 3],
    /// Bevy 的语义：点/聚光 = 流明（内部除以 4π），平行光 = 勒克斯。
    pub intensity: f32,
    /// 点/聚光的射程（米）。`None` = 用**渲染器策略**里那一个缺省：oracle 是
    /// `light.range.unwrap_or_else(|| PointLight::default().range)`（`px_render/src/scene.rs:233`）
    /// = **20.0**（`bevy_light-0.19.1/src/point_light.rs:133`）。
    ///
    /// ⚠ 这里原来写的是"缺省 = 按强度反推（`range = √(intensity/最小照度)`）"—— **那是错的**：
    /// oracle 从来没算过那个式子，照它实现出来的射程会与锚图差一大截（而画面上只表现为
    /// "衰减快慢不对"）。口径以 oracle 的行为为准，分岔记在这里，免得下一个人照着旧注释
    /// 自信地实现另一条规则。
    #[serde(default)]
    pub range: Option<f32>,
    #[serde(default)]
    pub inner_angle: f32,
    #[serde(default)]
    pub outer_angle: f32,
    #[serde(default)]
    pub shadows: bool,
}

fn forward() -> [f32; 3] {
    [0.0, 0.0, -1.0]
}

fn white() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl Light {
    pub fn point(id: &str, position: [f32; 3], color: [f32; 3], intensity: f32) -> Self {
        Self {
            id: id.to_string(),
            kind: LightKind::Point,
            position,
            direction: forward(),
            color,
            intensity,
            range: None,
            inner_angle: 0.0,
            outer_angle: 0.0,
            shadows: false,
        }
    }

    pub fn with_shadows(mut self, shadows: bool) -> Self {
        self.shadows = shadows;
        self
    }

    pub fn with_range(mut self, range: f32) -> Self {
        self.range = Some(range);
        self
    }
}

/// 环境：环境光强度 + 天空盒（cube 贴图产物）。天空盒是**内容**：
/// 有没有星空、星空长什么样由产物说了算，渲染器只负责把它挂到相机上。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    #[serde(default)]
    pub ambient: f32,
    #[serde(default)]
    pub skybox: Option<Member>,
    /// 天空盒的亮度倍率（Bevy `Skybox.brightness`）。缺省 = 900（加这条之前的常量）。
    #[serde(default = "skybox_brightness_default")]
    pub skybox_brightness: f32,
}

fn skybox_brightness_default() -> f32 {
    900.0
}

/// 手写而不是 derive：`skybox_brightness` 的缺省必须是 900 而不是 0
/// —— 否则「JSON 里没写」与「Rust 里 Default::default()」会给出两个不同的环境。
impl Default for Environment {
    fn default() -> Self {
        Self {
            ambient: 0.0,
            skybox: None,
            skybox_brightness: skybox_brightness_default(),
        }
    }
}

pub const VIEW_BUILTIN: &str = "view";

/// pass 表要读写的中间目标（`resources` 那一节）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassResource {
    pub name: String,
    pub format: String,
    pub size: String,
    /// **层数**（数组层 / cube 的面数）。缺省 = 1（普通的单层 2D 图）。
    ///
    /// ⚠ 文档里带的是**解出来的那个整数**，不是规则：规则（"每盏投影的点光一个 cube"）
    /// 住在帧图配方里，由**烘图侧**按这一帧的场景解出来（§133 同一条口径：
    /// 文档自描述、宿主不认识规则）。所以宿主只需要会数层，不需要会算层。
    /// 缺省不落盘 ⇒ 没有分层资源的老文档逐字节不变。
    #[serde(default = "one_layer", skip_serializing_if = "is_one_layer")]
    pub layers: u32,
    #[serde(default)]
    pub usage: Vec<String>,
}

fn one_layer() -> u32 {
    1
}

fn is_one_layer(layers: &u32) -> bool {
    *layers == 1
}

fn fragment_entry() -> String {
    "fs_main".to_string()
}

/// 一笔 draw：**按名字**说"用哪份几何、哪份材质"（§125 的帧图）。
///
/// 名字是内容（"planet" / "icosphere"），不是渲染器的概念：执行器（`px_pass`）不认识它们，
/// 只把名字原样交给宿主去解析成 GPU 句柄。这里放名字、不放句柄 —— 句柄是运行期的东西，
/// 而这一份文档是要烘进产物的。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawSpec {
    pub geometry: String,
    /// 材质名。**空 = 没有材质**：这一笔只有顶点阶段（深度-only 的那一笔就是这样）。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub material: String,
}

/// **帧自有**的材质（§135）：名字 + 内联 WGSL 全文 + 片元入口 + 参数。
///
/// 为什么它既不在 CAS 里、也不在 `objects` 里：内容材质是**产物**（`.pxart` 里指一个成员键，
/// 可换、可热重载），而帧材质是**这颗渲染器自己的一部分** —— 天空盒那支 WGSL 必须与 oracle
/// 逐位对齐，它不是可以换掉的内容。所以它**内联全文**进文档：读这份产物不需要再去 CAS 里找它，
/// 也不会有人以为它能换（`art/frame/skybox.wgsl` 里那句"它不进 CAS"就是这条）。
///
/// ⚠ **改 `art/frame/**` 下任何一份 WGSL 都要重烘**：文档里存的是**当时的文本**，
/// 不是指向文件的引用。⚠ 而且"图没变"**不等于**"改动没生效" —— 这一条付过代价：
/// 一次交接写着"新加的 art 文件还没被任何东西读到 ⇒ 不影响产物键"，而那一份顶点 WGSL
/// 正是被内联进文档的（§131.2）。产物键会动，图可能一个像素都不动。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameMaterial {
    /// 名字：一条 draw 的 `material` 按它解析（今天只有 `skybox` 一个）。
    pub name: String,
    /// 内联的 WGSL 全文（**组装前**的原文：`#import` 与 `#{MATERIAL_BIND_GROUP}` 由宿主组装）。
    pub shader: String,
    /// 片元入口名（几何 pass 的顶点阶段是另一栏，属于 pass）。
    pub entry: String,
    /// 参数：按名字给的**值**，烘图时按这份 WGSL 反射出来的结构体打包（与材质同一条路）。
    ///
    /// ⚠ 帧配方的 `[[materials]]` 里写的是参数的**来源**（`environment.skybox_brightness`），
    /// 到这里已经变成值 —— 亮度这类**内容值只有一处真源**（`environment`），
    /// 帧配方里写死一个数就是 §133 那颗雷（"六份场景今天恰好都是 900"）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassSpec {
    pub kind: String,
    /// 片元阶段的 shader 成员。**`None` = 这条 pass 没有 pass 级片元阶段**：
    /// 几何 pass 的片元阶段**属于材质**（每个物体一支，§129），所以它这一栏必须是空的 ——
    /// 编一个占位成员会在文档里留下一个假引用，还会污染成员表与闭包对账。
    /// 全屏 pass 则**必须**有（那条 pass 就是它自己那支后处理）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<Member>,
    #[serde(default)]
    pub label: String,
    #[serde(default = "fragment_entry")]
    pub entry: String,
    #[serde(default)]
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    /// 给这份 pass shader 的参数：**按名字**给，按它自己声明的结构体打包。
    ///
    /// 与材质走的是同一条路（`px_protocol::material::MaterialLayout::pack`），
    /// 判据也一样是三档当场报错（缺参 / 多参 / 类型不符）。
    /// 空表不落盘 ⇒ 没有参数的老文档逐字节不变。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    // ---- 下面四栏是 §125（帧图写进产物）加的，**全是纯加法**：----------------
    //
    // 一条老形状的 pass 一个都不写，于是它的 JSON 一个字节都不变
    // （判据在 `the_frozen_originals_round_trip_byte_for_byte`：六份冻结产物读→写逐字节相同）。
    // ⚠ 顺序也在这条判据里：新字段一律**追加在末尾**，插在中间会改老文档的键序。
    /// 这一笔 pass 画什么：几何名 + 材质名。空 = 全屏 pass（顶点由执行器自备）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub draws: Vec<DrawSpec>,
    /// 几何 pass 的**顶点阶段**：WGSL 全文 + 入口名。空 = 全屏 pass。
    ///
    /// 为什么是**全文**而不是 `Member`：内容 shader 是纯片元的（没有 `@vertex`），
    /// 顶点变换是宿主与 Bevy 逐位对齐的那一段，由烘图侧**内联**进文档 ——
    /// 它不是 CAS 里的一份资产，放不下 `Member` 那张"图/节点/内容键"的表。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vertex_shader: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vertex_entry: String,
    /// 附件与固定功能状态，**文本**：
    /// `color=clear(0,0,0,0)|depth=none|depth_write=true|compare=greater_equal|winding=ccw`。
    /// ⚠ **没有 `cull`**：剔除属于材质（§127），不在这一串里。
    ///
    /// ⚠ 这里**不解析、也不重写那套规则**：解析器只有一份，住在 `px_pass`
    /// （`RenderState::parse` / `name`）。协议把它当**不透明文本**带过去 ——
    /// 两处各写一份解析就是"同一份契约、两个数"，而那种漂移只在出图那一刻才露头。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub render: String,
    /// 深度附件用哪张图：`resources` 里的一个名字，或者宿主这一帧给的外部目标。
    /// 与 `render` 文本里的 `depth=` 成对出现，配对规则同样由 `px_pass` 判。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_target: Option<String>,
    /// 这一条 pass 写**点光 cube 影子的哪一面**（§109：每面一条单层 pass）。
    ///
    /// ⚠ 三格**必须一起来**（`light` / `face` / `layer` 同在一个结构体里）：只给其中两格
    /// 是一句说不清的话，而"两格凑一格"这种状态在类型上就不该存在。
    /// ⚠ `layer` 是**说出来让人对账的**，不是唯一的真本：`light` 与 `face` 已经确定了它
    /// （`layer = light × 6 + face`），而那条算式由**宿主**当场核对 —— 于是这个数是一条
    /// **验证过的**事实。反过来说，只给 `layer` 会逼宿主自己发明一条"层号怎么排"的规则，
    /// 而那正是 §109.2 那个 1 ulp 风险旁边的东西。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cube_face: Option<PassCubeFace>,
}

/// 一条 pass 的**点光 cube 面**：写第 `light` 个 cube 的第 `face` 面，落在第 `layer` 层。
///
/// `light` 是 **cube 的下标**（= 内容 shader 里那个 `light_id`，也是聚类缓冲里的下标）。
/// 它等于"文档 `lights` 里第几盏**投影的点光**"（按文档次序）—— 因为宿主那一步排序
/// （`lights_of`：开影子的在前、同档稳定）把投影的那些灯**原样**排在最前面，
/// 所以第 m 盏投影点光的聚类下标恒为 m。⚠ 这一条不依赖那个**不可实测**的 entity 次序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassCubeFace {
    pub light: u32,
    /// 0..5，次序与 `bevy_camera` 的 `CUBE_MAP_FACES` 一致（+X −X +Y −Y +Z −Z，§109.2）。
    pub face: u32,
    /// 那份 cube array 里的层号：`light × 6 + face`。
    pub layer: u32,
}

impl PassSpec {
    pub fn writes_view(&self) -> bool {
        self.writes.first().map(String::as_str) == Some(VIEW_BUILTIN)
    }

    pub fn label_or(&self, index: usize) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        // 没给标签时的兜底名字：拿它自己那支 shader 的图/节点。
        // ⚠ 几何 pass 没有 pass 级 shader（片元阶段属于材质）⇒ 退回 kind，
        //    免得文档里出现一个空名字（那种"报错里认不出是哪条 pass"的坑）。
        match &self.shader {
            Some(shader) => format!("{index}:{}/{}", shader.graph, shader.node),
            None => format!("{index}:{}", self.kind),
        }
    }
}

/// 一般渲染文档：环境 + 相机表 + 灯表 + 物体表 + pass 表。
/// 渲染器只吃这一份，不再从命令行接收内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSpec {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub environment: Environment,
    /// 评审相机表（`--sheet` 用）。空 = 走 `--cam`。
    #[serde(default)]
    pub cameras: Vec<Camera>,
    /// 内容自己声明的**期望标签**。渲染器不认识这些字符串，只把它们逐字交给报告：
    /// 出图判据里那条「这一档该看见云」（`ShotReport.declared_clouds`）就是问标签里有没有
    /// `clouds`。这样"该看见什么"仍然由内容说，而不是渲染器猜 —— 猜的那一天它就又认识行星了。
    #[serde(default)]
    pub expects: Vec<String>,
    /// pass 表要读写的中间目标。一个资源在这里声明一次，pass 按名字引用。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<PassResource>,
    /// pass 表：**数组顺序就是执行顺序**。空表 = 只有主 pass，与没有这一节时逐字节相同。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passes: Vec<PassSpec>,
    #[serde(default)]
    pub lights: Vec<Light>,
    pub objects: Vec<Object>,
    /// **帧自有**的材质（§135）：天空盒那种"属于这颗渲染器、不是可换内容"的材质。
    /// 名字由 `draws[].material` 引用；全文内联，因为改它要重烘（见 [`FrameMaterial`]）。
    ///
    /// ⚠ 空表不落盘 ⇒ 六份冻产物与 `--no-frame-graph` 那条逃生门**逐字节不变**
    /// （判据在 `the_frozen_originals_round_trip_byte_for_byte`）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame_materials: Vec<FrameMaterial>,
    /// **生成出来的材质实例**（§139 的用户裁决）：又一份材质，内容照 `base` 那一份，
    /// 只是**另起一个名字**。
    ///
    /// 为什么需要它：一条 pass 引用一个材质名，而一条材质名在宿主那儿恰好解析出**一套组**
    /// （组号 + 句柄 + 布局）。点光 cube 影子要六面，每面一套组（§148 之后是那一面的 `PassView`）⇒ 六面各要
    /// **各自的名字**。名字由烘图侧**生成**（人写的那一层仍然只有一份意图 + "影子要六面"），
    /// 而 `.pxart` 本来就是生成出来的指令流 —— "给一份不同的绑定状态起个名字"在指令流里
    /// 不是范畴错误。
    ///
    /// ⚠ 为什么是**引用**（`base`）而不是把材质描述抄一遍：抄一遍会在文档里多出六份
    /// 一模一样的 shader/参数/贴图表，而**抄不动的部分**（几何）还得跟着抄 ——
    /// `objects[]` 一条就是"几何 + 材质"，宿主按物体装载网格，六份副本就是六次网格解码
    /// 与六份上传。引用则一分不多：名字是新的，材质还是那一份。
    ///
    /// ⚠ 「这一面的 view 是哪一面」**不在这里**：它由**用到这个名字的那条 pass** 说
    /// （`PassSpec::cube_face`）—— 同一个事实只有一处（§66.1），而宿主当场判
    /// "一个名字只能被一条带 cube_face 的 pass 用"（两处用、面不同 ⇒ 歧义 ⇒ 拒）。
    /// 空表不落盘 ⇒ 没有影子实例的文档逐字节不变。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub material_instances: Vec<MaterialInstance>,
}

/// 一份**生成的材质实例**：`name` 是它的名字（`draws[].material` 引它），
/// `base` 是它的来源（某个物体的 id，或者一份帧自有材质的名字）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialInstance {
    pub name: String,
    pub base: String,
}

impl SceneSpec {
    pub fn object(&self, id: &str) -> Option<&Object> {
        self.objects.iter().find(|object| object.id == id)
    }

    /// 文档引用的全部成员（物体几何 / 材质 shader / 贴图 / 天空盒）。
    pub fn members(&self) -> Vec<&Member> {
        let mut out: Vec<&Member> = Vec::new();
        for object in &self.objects {
            if let Some(member) = object.geometry.member() {
                out.push(member);
            }
            out.push(&object.material.shader);
            out.extend(
                object
                    .material
                    .textures
                    .values()
                    .map(|texture| &texture.member),
            );
        }
        if let Some(skybox) = &self.environment.skybox {
            out.push(skybox);
        }
        for pass in &self.passes {
            if let Some(shader) = &pass.shader {
                out.push(shader);
            }
        }
        out
    }

    pub fn check(&self) -> Result<(), String> {
        if self.schema != SCENE_SCHEMA {
            return Err(format!(
                "场景描述是 v{}，这份渲染器认 v{}",
                self.schema, SCENE_SCHEMA
            ));
        }
        if self.objects.is_empty() {
            return Err("场景里一个物体都没有".to_string());
        }
        let mut seen: Vec<&str> = Vec::new();
        for object in &self.objects {
            if object.id.is_empty() {
                return Err("有个物体没给 id".to_string());
            }
            if seen.contains(&object.id.as_str()) {
                return Err(format!("物体 id 重了：'{}'", object.id));
            }
            seen.push(&object.id);
            let rotation = object.transform.rotation;
            let length = (rotation[0] * rotation[0]
                + rotation[1] * rotation[1]
                + rotation[2] * rotation[2]
                + rotation[3] * rotation[3])
                .sqrt();
            if !length.is_finite() || length < 1e-6 {
                return Err(format!(
                    "物体 '{}' 的旋转四元数长度为 0（[{}, {}, {}, {}]）：它没法当朝向用",
                    object.id, rotation[0], rotation[1], rotation[2], rotation[3]
                ));
            }
            for (role, texture) in &object.material.textures {
                // 格号是不是合法的**只有一份**判据：契约表（`crate::material::TEXTURE_SLOTS`）。
                // 这一段原来自己写「奇数格、≥1」——那是那张表的半份手抄（§67.4 第 4 处）：
                // 表加宽之后（§74.4 裁决 (a)：4 格 → 12 格），半份抄本会开始拒合法的格。
                if crate::material::texture_slot_of(texture.binding).is_none() {
                    return Err(format!(
                        "物体 '{}' 的贴图 '{role}' 绑在第 {} 格：材质只认 {} 这几格（采样器占下一格）",
                        object.id,
                        texture.binding,
                        crate::material::texture_bindings()
                    ));
                }
            }
        }
        let mut lights: Vec<&str> = Vec::new();
        for light in &self.lights {
            if lights.contains(&light.id.as_str()) {
                return Err(format!("灯 id 重了：'{}'", light.id));
            }
            lights.push(&light.id);
            if !light.intensity.is_finite() || light.intensity < 0.0 {
                return Err(format!(
                    "灯 '{}' 的强度是 {}：要一个非负的有限数",
                    light.id, light.intensity
                ));
            }
        }
        let mut resources: Vec<&str> = Vec::new();
        for resource in &self.resources {
            if resource.name.is_empty() {
                return Err("有个 pass 资源没给名字".to_string());
            }
            if resource.name == VIEW_BUILTIN {
                return Err(format!(
                    "pass 资源名 '{VIEW_BUILTIN}' 是内建名（相机自己的目标），不许重名"
                ));
            }
            if resources.contains(&resource.name.as_str()) {
                return Err(format!("pass 资源名重了：'{}'", resource.name));
            }
            if resource.format.trim().is_empty() || resource.size.trim().is_empty() {
                return Err(format!(
                    "pass 资源 '{}' 没给格式或尺寸规则（两样都要：执行器按它建中间目标）",
                    resource.name
                ));
            }
            if resource.layers == 0 {
                return Err(format!(
                    "pass 资源 '{}' 的层数是 0：一份资源至少一层",
                    resource.name
                ));
            }
            resources.push(&resource.name);
        }
        let declared = if resources.is_empty() {
            "（一个都没有）".to_string()
        } else {
            resources.join(" / ")
        };
        let mut labelled: Vec<String> = Vec::new();
        for (index, pass) in self.passes.iter().enumerate() {
            let label = pass.label_or(index);
            let at = format!("第 {index} 条 pass '{label}'");
            match pass.kind.as_str() {
                // ⚠ 这里只列**文档认得的类型名**；"这一档执行器会不会画"是另一件事，
                // 由执行器自己判（`px_pass::Plan::check`：compute 它当场拒）。
                //
                // `copy` 是一条**搬运**（§131）：`reads[0]` → `writes[0]`，不建管线、不挂附件。
                // 它为什么存在、形状上还要满足什么，由 `px_pass::Plan::check` 一条条判 ——
                // 这里只负责让这个名字**是文档里写得出来的**（白名单少一个词，
                // 批准的机制就根本不存在）。
                "fullscreen" | "geometry" | "copy" | "compute" => {}
                other => {
                    return Err(format!(
                        "{at} 的 kind 是 '{other}'：认 'fullscreen' 与 'geometry' 与 'copy' 与 'compute'"
                    ));
                }
            }
            // 片元阶段归谁：**全屏在 pass 上，几何在材质上**（§129）。
            // 这条是文档级的形状判据；执行器那侧 `px_pass::Plan::check` 说的是同一件事。
            match (&pass.shader, pass.kind.as_str()) {
                (None, "fullscreen") => {
                    return Err(format!(
                        "{at} 是 fullscreen，却没给 shader：全屏 pass 就是它自己那支后处理"
                    ));
                }
                (Some(_), "geometry") => {
                    return Err(format!(
                        "{at} 是 geometry，却给了 shader：几何 pass 的片元阶段属于**材质**                         （每个物体一支）"
                    ));
                }
                (Some(_), "copy") => {
                    return Err(format!(
                        "{at} 是 copy，却给了 shader：拷贝不建管线、也没有片元阶段\
                         （片元阶段属于材质，pass 级 shader 只有全屏那一档才有）"
                    ));
                }
                _ => {}
            }
            if pass.shader.is_some() && pass.entry.trim().is_empty() {
                return Err(format!("{at} 没给入口点名字（@fragment 那个函数叫什么）"));
            }
            if pass.writes.is_empty() {
                // ⚠ 几何那一档**允许空**：深度-only 的那条 pass 不写颜色
                // （`render` 文本里 `color=none`）。它成不成立由执行器判 ——
                // "挂了颜色却没写目标"那条判据住在 `px_pass`，因为只有它会去解析那串文本。
                // ⚠ copy 那一档**必须写一个**：一次搬运没有目标就是空转
                //（它连"画"都不是，没有"没人看得见"这回事，是**没搬**）。
                match pass.kind.as_str() {
                    "geometry" => {}
                    "copy" => {
                        return Err(format!(
                            "{at} 是 copy，却没有 writes：一次搬运必须说清**搬到哪张图**\
                             （`writes` 那一栏就是它的目标）"
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "{at} 没有 writes：它不写任何东西，画了也没人看得见"
                        ));
                    }
                }
            }
            if pass.writes.len() > 1 {
                let what = if pass.kind == "copy" {
                    "一次搬运只搬一张图"
                } else {
                    "一条 pass 只画一个颜色附件"
                };
                return Err(format!("{at} 写了 {} 个目标：{what}", pass.writes.len()));
            }
            if labelled.contains(&label) {
                return Err(format!("pass 标签重了：'{label}'"));
            }
            labelled.push(label);
            // 点光 cube 的那一面（§109：六面各一条单层 pass）。
            //
            // ⚠ 三处各管一件事，一件都不许抄成两处（§66.1）：
            //    · **这里**判"文档自己说得通"：面号在 0..5 里、而且它得有个去处；
            //    · **宿主**判那条算式 `layer = light × 6 + face`（只有它知道 cube 怎么排）；
            //    · **`px_pass::Plan::check`** 判"那一层在不在那份资源里"。
            if let Some(cube) = &pass.cube_face {
                if cube.face >= 6 {
                    return Err(format!(
                        "{at} 的 cube_face.face 是 {}：cube 只有 6 面（0..5）",
                        cube.face
                    ));
                }
                if pass.depth_target.is_none() {
                    return Err(format!(
                        "{at} 给了 cube_face，却没给 depth_target：那一面要写进哪张 cube 影图？"
                    ));
                }
            }
            for name in pass.reads.iter().chain(pass.writes.iter()) {
                if name == VIEW_BUILTIN {
                    continue;
                }
                if !resources.contains(&name.as_str()) {
                    return Err(format!(
                        "{at} 用了 '{name}'，但 resources 里没声明它。声明了的：{declared}"
                    ));
                }
            }
            if pass.reads.iter().any(|read| pass.writes.contains(read)) {
                let shared: Vec<&String> = pass
                    .reads
                    .iter()
                    .filter(|read| pass.writes.contains(read))
                    .collect();
                for name in shared {
                    if resources.contains(&name.as_str()) {
                        return Err(format!(
                            "{at} 同时读和写 '{name}'：它是一条 resources 声明（只有一张纹理）\
                             ；读写同一张画面要靠宿主给两张（例如 'view' 的 ping-pong）"
                        ));
                    }
                }
            }
        }
        if !self.passes.is_empty() && !self.passes.iter().any(PassSpec::writes_view) {
            return Err(format!(
                "这份文档有 {} 条 pass，但没有一条写 '{VIEW_BUILTIN}'：画面不会被改动",
                self.passes.len()
            ));
        }

        // ---- 帧自有材质（§135）--------------------------------------------------
        //
        // ⚠ 一条 draw 的 `material` 只有两个可能的出处：**物体 id**（内容材质 ——
        //    `px_scene::frame::draws_of` 就是拿物体 id 当材质名的）与**帧自有材质**。
        //    两边撞名 ⇒ 宿主解析时"同一个名字在两张表里"，而它只能猜一个：画面错、没人报错。
        //    所以撞名在这里就拒，并把**两处**都列出来。
        let mut frame_names: Vec<&str> = Vec::new();
        for (index, material) in self.frame_materials.iter().enumerate() {
            if material.name.trim().is_empty() {
                return Err(format!(
                    "第 {index} 份帧材质没有名字：`draws[].material` 是按名字引用它的"
                ));
            }
            if frame_names.contains(&material.name.as_str()) {
                return Err(format!("帧材质名字重了：'{}'", material.name));
            }
            if let Some(object) = self
                .objects
                .iter()
                .find(|object| object.id == material.name)
            {
                return Err(format!(
                    "帧材质 '{}' 与物体 '{}' 撞名：物体 id **就是**它的材质名\
                     （`draws[].material` 用的就是它）⇒ 同一个名字在两张表里。\
                     要改的是帧配方那一份（`art/frame/*.toml` 的 `[[materials]] name`）",
                    material.name, object.id
                ));
            }
            if material.shader.trim().is_empty() {
                return Err(format!(
                    "帧材质 '{}' 没给 WGSL 全文：它必须内联进文档\
                     （不内联就等于文档指向了一个文档里没有的东西）",
                    material.name
                ));
            }
            if material.entry.trim().is_empty() {
                return Err(format!(
                    "帧材质 '{}' 没给片元入口名（@fragment 那个函数叫什么）",
                    material.name
                ));
            }
            frame_names.push(&material.name);
        }
        // ---- 生成的材质实例（§139）--------------------------------------------
        //
        // ⚠ 这一节是**生成物**：不要手写它，也不要为它做去重（"六份名字其实一套组，
        //    合成一份吧"）—— 那会把形状打回"一个名字两套组"，也就是需要给执行器
        //    加优先级规则的那种形状（而那条路已经被用户裁决否掉了）。
        let mut instance_names: Vec<&str> = Vec::new();
        for (index, instance) in self.material_instances.iter().enumerate() {
            let at = format!("第 {index} 份材质实例");
            if instance.name.trim().is_empty() {
                return Err(format!(
                    "{at} 没给名字：`draws[].material` 是按名字引用它的"
                ));
            }
            // 三张表共用一个名字空间：宿主那边材质是**按名字**查的一张平表，
            // 同一个名字在两处有真本就是歧义（它只能猜一个：画面错、没人报错）。
            if self.objects.iter().any(|object| object.id == instance.name)
                || frame_names.contains(&instance.name.as_str())
                || instance_names.contains(&instance.name.as_str())
            {
                return Err(format!(
                    "{at} 的名字 '{}' 与物体 id / 帧自有材质重了：材质名是一张平表，\
                     一个名字只能有一份真本",
                    instance.name
                ));
            }
            if !self.objects.iter().any(|object| object.id == instance.base)
                && !frame_names.contains(&instance.base.as_str())
            {
                return Err(format!(
                    "{at} '{}' 的 base 是 '{}'，而那不是任何物体的 id、也不是帧自有材质：\
                     它没有可照的那一份",
                    instance.name, instance.base
                ));
            }
            instance_names.push(&instance.name);
        }
        // 一笔 draw 的材质名必须落在**那两张表**之一。⚠ 几何名这里查不了：
        // 文档里没有几何表（图元与网格由宿主按名字解析），所以这条只管材质。
        for (index, pass) in self.passes.iter().enumerate() {
            let label = pass.label_or(index);
            for draw in &pass.draws {
                if draw.material.is_empty() {
                    continue;
                }
                if self.objects.iter().any(|object| object.id == draw.material)
                    || frame_names.contains(&draw.material.as_str())
                    || instance_names.contains(&draw.material.as_str())
                {
                    continue;
                }
                return Err(format!(
                    "第 {index} 条 pass '{label}' 的 draw（几何 '{}'）要材质 '{}'，\
                     而它既不是物体 id 也不是帧自有材质、也不是生成的材质实例。\n  物体 id（它们的 id 就是材质名）：{}\n  \
                     帧自有材质（`frame_materials`）：{}\n  生成的材质实例（`material_instances`）：{}",
                    draw.geometry,
                    draw.material,
                    if self.objects.is_empty() {
                        "（一个都没有）".to_string()
                    } else {
                        self.objects
                            .iter()
                            .map(|object| object.id.as_str())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    },
                    if frame_names.is_empty() {
                        "（一个都没有）".to_string()
                    } else {
                        frame_names.join(" / ")
                    },
                    if instance_names.is_empty() {
                        "（一个都没有）".to_string()
                    } else {
                        instance_names.join(" / ")
                    }
                ));
            }
        }
        // 声明了却没人用：与 `params` / `slots` / `reads` / `shader` 同一条规则
        // （§129 拿它拒"几何 pass 带片元阶段"）。一个没人引用的帧材质在文档里就是个假引用：
        // 它多半意味着名字写错了 —— 而那种错在画面上表现为"天空盒不画了"，不像是个拼写问题。
        let used: Vec<&str> = self
            .passes
            .iter()
            .flat_map(|pass| pass.draws.iter())
            .map(|draw| draw.material.as_str())
            .collect();
        let unused: Vec<&str> = frame_names
            .iter()
            .copied()
            .filter(|name| !used.contains(name))
            .collect();
        // 生成出来的材质实例同理：没人引用它 = 生成多了（或者名字写错了）。
        let unused_instances: Vec<&str> = instance_names
            .iter()
            .copied()
            .filter(|name| !used.contains(name))
            .collect();
        if !unused_instances.is_empty() {
            return Err(format!(
                "生成的材质实例 [{}] 没有任何一条 draw 用它 —— 生成多了，\
                 或者名字与 draws 里写的对不上（声明了没人用的东西，在文档里就是一个假引用）",
                unused_instances.join(" / ")
            ));
        }
        if !unused.is_empty() {
            return Err(format!(
                "帧材质 [{}] 声明了却没有任何一条 draw 用它。draws 引用的材质名是 [{}] —— \
                 要么是名字写错了，要么这一份该删掉（声明了没人用的东西，在文档里就是一个假引用）",
                unused.join(" / "),
                if used.is_empty() {
                    "（一个都没有）".to_string()
                } else {
                    used.iter()
                        .filter(|name| !name.is_empty())
                        .copied()
                        .collect::<Vec<_>>()
                        .join(" / ")
                }
            ));
        }
        Ok(())
    }

    pub fn audit(&self) -> String {
        let mut lines = vec![format!(
            "场景 {}（v{}）｜环境光 {}｜天空盒 {}｜相机 {} 个｜灯 {} 盏｜物体 {} 个",
            self.name,
            self.schema,
            self.environment.ambient,
            match &self.environment.skybox {
                Some(member) => member.to_string(),
                None => "无".to_string(),
            },
            self.cameras.len(),
            self.lights.len(),
            self.objects.len()
        )];
        if !self.passes.is_empty() {
            lines.push(format!(
                "  pass 表 {} 条（数组顺序即执行顺序）：",
                self.passes.len()
            ));
            for (index, pass) in self.passes.iter().enumerate() {
                lines.push(format!(
                    "    [{index}] {}｜{}｜读 [{}]｜写 [{}]｜参数 {} 个{}｜shader {}",
                    pass.label_or(index),
                    pass.kind,
                    pass.reads.join(" / "),
                    pass.writes.join(" / "),
                    pass.params.len(),
                    if pass.params.is_empty() {
                        String::new()
                    } else {
                        format!("（{}）", keys_of(&pass.params))
                    },
                    match &pass.shader {
                        Some(shader) => shader.to_string(),
                        None => "（无 pass 级片元阶段：属于材质）".to_string(),
                    }
                ));
            }
        }
        // 帧自有材质（§135）：**内联文本的长度要打出来** —— "改 art/frame 要不要重烘"
        // 这个问题，答案就在这一行里（文档里存的是当时的文本，不是文件引用）。
        for material in &self.frame_materials {
            lines.push(format!(
                "  帧材质 '{}'｜片元入口 {}｜内联 WGSL {} 字节｜参数 {}",
                material.name,
                material.entry,
                material.shader.len(),
                keys_of(&material.params)
            ));
        }
        for light in &self.lights {
            lines.push(format!(
                "  [{}] 灯 {:?}｜位置 ({:.2},{:.2},{:.2})｜色 ({:.2},{:.2},{:.2})｜强度 {:.3e}{}",
                light.id,
                light.kind,
                light.position[0],
                light.position[1],
                light.position[2],
                light.color[0],
                light.color[1],
                light.color[2],
                light.intensity,
                if light.shadows { "｜阴影贴图" } else { "" },
            ));
        }
        for object in &self.objects {
            let geometry = match &object.geometry {
                Geometry::Mesh { member } => format!("网格 {member}"),
                Geometry::Primitive { name, params } => {
                    format!("图元 {name}（{}）", keys_of(params))
                }
            };
            let textures = object
                .material
                .textures
                .iter()
                .map(|(role, texture)| format!("{role}@{}={}", texture.binding, texture.member))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(format!(
                "  [{}] {geometry}｜shader {}｜参数 {}｜贴图 {}",
                object.id,
                object.material.shader,
                keys_of(&object.material.params),
                if textures.is_empty() {
                    "无".to_string()
                } else {
                    textures
                }
            ));
        }
        lines.join("\n")
    }
}

pub fn read_scene(path: &Path) -> Result<SceneSpec, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读 {} 失败：{err}", path.display()))?;
    let frames =
        crate::stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    frames
        .into_iter()
        .find_map(|frame| match frame {
            crate::stream::Frame::Scene(spec) => Some(spec),
            _ => None,
        })
        .ok_or_else(|| format!("{} 里没有场景帧（Scene）", path.display()))
}

/// 写一份场景产物：清单帧（`kind = Scene`，带指纹与相机表）+ 场景帧。
/// 指纹由调用方算（协议 crate 不引哈希库，只承诺形状）。
pub fn write_scene(path: &Path, spec: &SceneSpec, fingerprint: u64) -> Result<u64, String> {
    let bytes = scene_bytes(spec, fingerprint)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, &bytes).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    Ok(bytes.len() as u64)
}

/// 一份场景产物的**字节**（清单帧 + 场景帧）。`write_scene` 就是它加一次落盘。
///
/// 分开是为了判据：往返要**逐字节**比，就不该为了比一次往磁盘上写一份。
pub fn scene_bytes(spec: &SceneSpec, fingerprint: u64) -> Result<Vec<u8>, String> {
    spec.check()?;
    let bundle = crate::art::ArtBundle {
        assets: vec![crate::art::AssetManifest {
            id: spec.name.clone(),
            kind: crate::art::AssetKind::Scene,
            params: BTreeMap::from([
                ("schema".to_string(), f64::from(spec.schema)),
                ("objects".to_string(), spec.objects.len() as f64),
                ("lights".to_string(), spec.lights.len() as f64),
            ]),
            blobs: Vec::new(),
            fingerprint,
            cameras: spec.cameras.clone(),
        }],
    };
    let frames = vec![
        crate::stream::Frame::Art(bundle),
        crate::stream::Frame::Scene(spec.clone()),
    ];
    let mut bytes = Vec::new();
    crate::stream::write_stream(&mut bytes, &frames).map_err(|err| err.to_string())?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份最小的合法文档：一个物体、一份材质、一盏灯。
    const DOC: &str = r#"{
        "schema": 2,
        "name": "夹具",
        "objects": [{
            "id": "planet",
            "geometry": {"source": "primitive", "name": "icosphere", "params": {"radius": 1.0}},
            "material": {
                "shader": {"graph": "shaders", "node": "surface", "key": "00"},
                "params": {"gain": 1.0},
                "textures": {"albedo": {"binding": 1, "member": {"graph": "generated", "node": "color", "key": "11"}}}
            },
            "transform": {"rotation": [0.0, 0.0, 0.0, 1.0]}
        }],
        "lights": [{"id": "sun", "kind": "point", "position": [1.0, 2.0, 3.0], "intensity": 1.0}]
    }"#;

    #[test]
    fn our_own_documents_parse() {
        let spec: SceneSpec = serde_json::from_str(DOC).expect("自己的文档必须解析得动");
        assert_eq!(spec.objects.len(), 1);
        assert_eq!(spec.objects[0].material.params.len(), 1);
        spec.check().expect("这份夹具是合法的");
    }

    /// **未知字段不许静默忽略**（§73 用户裁决）：旧渲染器读到新文档时，
    /// 静默忽略一个新字段就等于「少画了东西还报成功」—— 那正是 §34 禁止的那种绿灯。
    #[test]
    fn an_unknown_field_is_refused_not_ignored() {
        let object = DOC.replace(
            r#""id": "planet","#,
            r#""id": "planet", "cast_shadow_typo": true,"#,
        );
        let err = serde_json::from_str::<SceneSpec>(&object).expect_err("物体上的未知字段必须报错");
        assert!(err.to_string().contains("cast_shadow_typo"), "{err}");

        let material = DOC.replace(r#""gain": 1.0"#, r#""gain": 1.0, "roughness": 0.4"#);
        // 材质参数表是**按名字自由**的（那正是「加参数」要走的路），所以这里**不该**报错 ——
        // 名字对不对由 shader 的 descriptor 说了算，不是协议说了算。
        let spec: SceneSpec = serde_json::from_str(&material).expect("材质参数表按名字自由");
        assert_eq!(spec.objects[0].material.params.len(), 2);

        let texture = DOC.replace(
            r#""binding": 1,"#,
            r#""binding": 1, "wrap_typo": "repeat","#,
        );
        let err =
            serde_json::from_str::<SceneSpec>(&texture).expect_err("贴图上的未知字段必须报错");
        assert!(err.to_string().contains("wrap_typo"), "{err}");

        let spec_level = DOC.replace(
            r#""name": "夹具","#,
            r#""name": "夹具", "ambient_typo": 1.0,"#,
        );
        let err =
            serde_json::from_str::<SceneSpec>(&spec_level).expect_err("文档上的未知字段必须报错");
        assert!(err.to_string().contains("ambient_typo"), "{err}");

        let geometry = DOC.replace(
            r#"{"source": "primitive", "name": "icosphere", "params": {"radius": 1.0}}"#,
            r#"{"source": "primitive", "name": "icosphere", "subdivisons": 64, "params": {"radius": 1.0}}"#,
        );
        let err =
            serde_json::from_str::<SceneSpec>(&geometry).expect_err("几何上的未知字段必须报错");
        assert!(err.to_string().contains("subdivisons"), "{err}");

        // 新加的那几栏同样不许被静默忽略（与上面同一条裁决）。
        let pass = old_pass_doc().replace(
            r#""writes": ["view"]"#,
            r#""writes": ["view"], "render_typo": "x""#,
        );
        let err = serde_json::from_str::<SceneSpec>(&pass).expect_err("pass 上的未知字段必须报错");
        assert!(err.to_string().contains("render_typo"), "{err}");

        let draw = geometry_doc().replace(
            r#""geometry": "planet""#,
            r#""geometry": "planet", "matrial": "surface""#,
        );
        let err = serde_json::from_str::<SceneSpec>(&draw).expect_err("draw 上的未知字段必须报错");
        assert!(err.to_string().contains("matrial"), "{err}");
    }

    /// 允许漂移的名单 —— **现在是空的，而且必须一直是空的**。
    ///
    /// 它曾经有两份：`orbit-proxy-fine-bound.pxart` 与 `orbit-soft.pxart` 里的
    /// `"slope_scale":0.11999999731779099`（`0.12f32` 的精确 f64 值）读回来会**大 1 个 ulp**，
    /// 写回去少 2 字节。根因不在这个 crate：**`serde_json` 默认的浮点解析不是正确舍入的**
    /// （`float_roundtrip` 特性默认关着）。实测：
    /// `"0.11999999731779099".parse::<f64>()` = `…000`，而
    /// `serde_json::from_str::<f64>(同串)` = `…001`。
    ///
    /// 那个特性已经在五个 `Cargo.toml` 里打开，实测六份现在**全部**逐字节相同。
    /// ⚠ 名单空着不等于判据松了：下面那条"只差 1 个 ulp"的宽容通道还在，
    /// 只是**谁都不许走** —— 将来再漂一份，这里就该红，而不是被宽容掉。
    const KNOWN_DRIFT: [&str; 0] = [];

    /// 两份载荷的差异是不是**只在一个数上、而且只差 1 个 ulp**（连 `f32` 视角都相同）。
    ///
    /// 返回一句人能读的结论；不成立就返回原因。判据比"长度差不多"严得多：
    /// 它把左边那一处数换成右边的写法之后，两份必须**逐字节相同** —— 也就是
    /// 「除了这一个数，别的地方一处都不许变」。
    fn only_one_ulp_of_one_number(left: &str, right: &str) -> Result<String, String> {
        let a = left.as_bytes();
        let b = right.as_bytes();
        let at = a
            .iter()
            .zip(b.iter())
            .position(|(x, y)| x != y)
            .ok_or_else(|| "两份载荷逐字节相同（那它不该走到这里）".to_string())?;
        let is_number = |c: u8| c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E');
        let token_at = |bytes: &[u8], from: usize| -> (String, usize, usize) {
            let mut start = from;
            while start > 0 && is_number(bytes[start - 1]) {
                start -= 1;
            }
            let mut end = from;
            while end < bytes.len() && is_number(bytes[end]) {
                end += 1;
            }
            (
                String::from_utf8_lossy(&bytes[start..end]).to_string(),
                start,
                end,
            )
        };
        let (left_token, left_start, left_end) = token_at(a, at);
        let (right_token, _, _) = token_at(b, at);
        let x: f64 = left_token
            .parse()
            .map_err(|err| format!("'{left_token}' 不是数：{err}"))?;
        let y: f64 = right_token
            .parse()
            .map_err(|err| format!("'{right_token}' 不是数：{err}"))?;
        if x == y {
            return Err(format!(
                "'{left_token}' 与 '{right_token}' 是同一个值 —— 差异不在值上"
            ));
        }
        let ulps = (i128::from(x.to_bits()) - i128::from(y.to_bits())).abs();
        if ulps > 1 {
            return Err(format!(
                "'{left_token}' 与 '{right_token}' 差了 {ulps} 个 ulp"
            ));
        }
        if (x as f32) != (y as f32) {
            return Err(format!(
                "'{left_token}' 与 '{right_token}' 只差 1 个 ulp，但 **f32 视角也不同** —— \
                 渲染器吃的就是 f32，那就不再是「看不见的漂移」了"
            ));
        }
        let mut mended = left.to_string();
        mended.replace_range(left_start..left_end, &right_token);
        if mended != right {
            return Err("除了这一处数，别的地方也变了".to_string());
        }
        Ok(format!(
            "'{left_token}' → '{right_token}'（差 1 个 ulp，f32 视角相同，别处一字未动）"
        ))
    }

    /// 一份**老形状**的文档：pass 表只有全屏那一档，新字段一个都不出现。
    fn old_pass_doc() -> String {
        DOC.replace(
            r#""lights": ["#,
            r#""passes": [{"kind": "fullscreen", "shader": {"graph": "shaders", "node": "grade", "key": "22"}, "writes": ["view"]}], "lights": ["#,
        )
    }

    /// 把一个流的 `(载荷起点, 载荷长度)` 逐个切出来（跳过 MAGIC + 版本号）。
    ///
    /// 判据要在**原始字节**上比，所以需要这个：解码再编码会把"两种写法、同一个值"
    /// 的差异抹平 —— 而那种差异正是会改产物键的东西。
    fn payload_spans(bytes: &[u8]) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut at = 8;
        while at + 4 <= bytes.len() {
            let len = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
                as usize;
            spans.push((at + 4, len));
            at += 4 + len;
        }
        spans
    }

    /// 一份**新形状**的文档：一条几何 pass，四栏新字段全用上。
    ///
    /// ⚠ 第二笔的材质名写的是**物体 id**（`planet`），不是那支 shader 的节点名
    /// （`surface`）：`draws[].material` 只有两个出处 —— 物体 id 与帧自有材质
    /// （`px_scene::frame::draws_of` 就是拿 id 当材质名的），而 `check` 从 §135 起
    /// 会拒"两边都不是"的名字。原来那个 `"surface"` 是**从来就解析不到**的写法，
    /// 只是当时没有门去问它。这条判据判的是"几何 pass 表达得出来、往返得回去"，
    /// 与那个名字是什么无关。
    fn geometry_doc() -> String {
        DOC.replace(
            r#""lights": ["#,
            r#""resources": [{"name": "depth", "format": "depth32float", "size": "view", "usage": ["render_attachment"]}],
               "passes": [{
                 "kind": "geometry",
                 "label": "prepass",
                 "vertex_shader": "struct Out { @builtin(position) position: vec4<f32> }\n@vertex fn vertex() -> Out { return Out(vec4<f32>(0.0)); }",
                 "vertex_entry": "vertex",
                 "writes": ["view"],
                 "draws": [{"geometry": "planet", "material": "planet"}, {"geometry": "planet"}],
                 "render": "color=none|depth=clear(0)|depth_write=true|compare=greater_equal|winding=ccw",
                 "depth_target": "depth"
               }],
               "lights": ["#,
        )
    }

    /// 新字段是**纯加法**：老形状的 pass 落盘时一个都不许出现。
    ///
    /// ⚠ 这条不是锦上添花：`skip_serializing_if` 少写一个，老文档就会多出一串
    /// `"draws":[]`，产物键跟着变 —— 而那正是五个锚会碎掉的方式。
    #[test]
    fn the_new_pass_fields_stay_out_of_old_documents() {
        let spec: SceneSpec =
            serde_json::from_str(&old_pass_doc()).expect("老形状的 pass 要能解析");
        assert_eq!(spec.passes.len(), 1);
        assert!(spec.passes[0].draws.is_empty());
        assert!(spec.passes[0].vertex_shader.is_empty());
        assert!(spec.passes[0].render.is_empty());
        assert_eq!(spec.passes[0].depth_target, None);
        let text = serde_json::to_string(&spec).expect("序列化");
        for key in [
            "draws",
            "vertex_shader",
            "vertex_entry",
            "depth_target",
            "\"render\"",
        ] {
            assert!(
                !text.contains(key),
                "老形状的 pass 落盘时不该出现 {key}：{text}"
            );
        }
        // 而且它逐字往返（键序、缺省值都不许变）。
        assert_eq!(serde_json::to_string(&spec).expect("再序列化"), text);
    }

    /// 新形状（几何 pass）要能表达、也要能往返。
    #[test]
    fn a_geometry_pass_is_expressible_and_round_trips() {
        let spec: SceneSpec =
            serde_json::from_str(&geometry_doc()).expect("几何 pass 的文档要能解析");
        spec.check().expect("这份文档是合法的");
        let pass = &spec.passes[0];
        assert_eq!(pass.kind, "geometry");
        assert_eq!(pass.draws.len(), 2);
        assert_eq!(pass.draws[0].material, "planet");
        assert_eq!(
            pass.draws[1].material, "",
            "第二笔没有材质（深度-only 那一笔）"
        );
        assert_eq!(pass.vertex_entry, "vertex");
        assert!(pass.render.starts_with("color=none|depth=clear(0)"));
        assert_eq!(pass.depth_target.as_deref(), Some("depth"));
        // ⚠ 几何 pass 的片元阶段属于**材质**（§129）⇒ 这一栏必须是 `None`，
        //    落盘时**一个 `shader` 键都不该有**（`skip_serializing_if` 那一条）。
        assert!(pass.shader.is_none(), "几何 pass 不该有 pass 级 shader");

        let text = serde_json::to_string(&spec).expect("序列化");
        // ⚠ 只对**这条 pass** 断言：物体的材质那一栏**应当**有 `shader`（材质就是那支
        //    shader，§129）—— 拿整份文档去 grep 会把材质那一栏也算进来。
        let pass_text = serde_json::to_string(&spec.passes[0]).expect("序列化这条 pass");
        assert!(
            !pass_text.contains("\"shader\""),
            "几何 pass 落盘时不该出现 shader 键：{pass_text}"
        );
        let back: SceneSpec = serde_json::from_str(&text).expect("再解析");
        assert_eq!(back, spec, "带新字段的文档必须逐字往返");
        // 新字段**真的落盘了**（不是"解析进默认值"那种假通过）。
        for key in ["draws", "vertex_shader", "render", "depth_target"] {
            assert!(text.contains(key), "{key} 没落盘：{text}");
        }
    }

    /// 一份带**帧自有材质**的文档：一条几何 pass 画 `skybox`，材质内联在 `frame_materials` 里。
    fn frame_material_doc() -> String {
        DOC.replace(
            r#""lights": ["#,
            r#""passes": [{
                 "kind": "geometry",
                 "label": "sky",
                 "vertex_shader": "struct Out { @builtin(position) position: vec4<f32> }\n@vertex fn vertex() -> Out { return Out(vec4<f32>(0.0)); }",
                 "vertex_entry": "vertex",
                 "writes": ["view"],
                 "draws": [{"geometry": "skybox", "material": "skybox"}],
                 "render": "color=load|depth=load|depth_write=false|compare=greater_equal|winding=ccw"
               }],
               "frame_materials": [{
                 "name": "skybox",
                 "shader": "@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
                 "entry": "fs_main",
                 "params": {"brightness": 900.0}
               }],
               "lights": ["#,
        )
    }

    /// 帧自有材质这一栏是**纯加法**：老文档落盘时一个键都不许多出来。
    ///
    /// ⚠ 与 `the_new_pass_fields_stay_out_of_old_documents` 同一条理由：`skip_serializing_if`
    /// 少写一个，六份冻产物就会多出一串 `"frame_materials":[]`，产物键跟着变。
    #[test]
    fn the_frame_material_field_stays_out_of_old_documents() {
        let spec: SceneSpec = serde_json::from_str(&old_pass_doc()).expect("老形状要能解析");
        assert!(spec.frame_materials.is_empty());
        let text = serde_json::to_string(&spec).expect("序列化");
        assert!(
            !text.contains("frame_materials"),
            "老形状的文档里不该出现这一栏：{text}"
        );
    }

    /// 帧自有材质表达得出来、往返得回去，而且**全文真的落盘了**。
    #[test]
    fn a_frame_material_is_expressible_and_round_trips() {
        let spec: SceneSpec = serde_json::from_str(&frame_material_doc()).expect("要能解析");
        spec.check().expect("这份文档是合法的");
        assert_eq!(spec.frame_materials.len(), 1);
        assert_eq!(spec.frame_materials[0].name, "skybox");
        assert_eq!(spec.frame_materials[0].entry, "fs_main");
        assert_eq!(
            spec.frame_materials[0].params["brightness"],
            Value::Num(900.0),
            "参数是**值**（来源在帧配方里，烘图时已经换成值）"
        );
        let text = serde_json::to_string(&spec).expect("序列化");
        let back: SceneSpec = serde_json::from_str(&text).expect("再解析");
        assert_eq!(back, spec, "带帧自有材质的文档必须逐字往返");
        // 内联的是**全文**：读这份产物不需要再回 CAS 找它。
        assert!(
            text.contains("@fragment fn fs_main"),
            "WGSL 全文必须真的在文档里：{text}"
        );
    }

    /// 帧自有材质的四条拒法：都当场说清楚**是谁**、**跟谁**冲突。
    #[test]
    fn a_broken_frame_material_is_refused_by_name() {
        // ① 与物体 id 撞名 ⇒ 拒，而且两处都要列出来。
        let clash = frame_material_doc()
            .replace(r#""name": "skybox""#, r#""name": "planet""#)
            .replace(r#""material": "skybox""#, r#""material": "planet""#);
        let spec: SceneSpec = serde_json::from_str(&clash).expect("解析");
        let err = spec.check().expect_err("帧材质与物体撞名 ⇒ 拒");
        assert!(err.contains("planet"), "要说清是哪个名字：{err}");
        assert!(err.contains("撞名"), "{err}");

        // ② 名字重了 ⇒ 拒（同名两张材质，宿主只能猜一个）。
        let two = frame_material_doc().replace(
            r#""frame_materials": [{"#,
            r#""frame_materials": [{"name": "skybox", "shader": "x", "entry": "fs_main"}, {"#,
        );
        let spec: SceneSpec = serde_json::from_str(&two).expect("解析");
        let err = spec.check().expect_err("帧材质名字重了 ⇒ 拒");
        assert!(err.contains("重了"), "{err}");

        // ③ 声明了却没人用 ⇒ 拒，并把 draws 里真正的名字列出来。
        let unused =
            frame_material_doc().replace(r#""material": "skybox""#, r#""material": "planet""#);
        let spec: SceneSpec = serde_json::from_str(&unused).expect("解析");
        let err = spec.check().expect_err("没人用 ⇒ 拒");
        assert!(err.contains("skybox"), "要说清是哪一份没用上：{err}");
        assert!(err.contains("planet"), "要列出 draws 真正引用的名字：{err}");

        // ④ 一笔 draw 要了个两边都没有的材质 ⇒ 拒，并列出两张表。
        let dangling =
            frame_material_doc().replace(r#""material": "skybox""#, r#""material": "skyboox""#);
        let spec: SceneSpec = serde_json::from_str(&dangling).expect("解析");
        let err = spec.check().expect_err("解析不到的材质名 ⇒ 拒");
        assert!(err.contains("skyboox"), "{err}");
        assert!(err.contains("planet"), "要列出物体 id 那张表：{err}");

        // 空文本与空入口名同样是"说了没做"（`skip_serializing_if` 会把空串整个藏起来，
        // 于是文档里看起来"没这一栏"，而 draws 仍然指着它）。
        let shader_text =
            "@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }";
        for (field, original, fragment) in [
            ("shader", shader_text, "WGSL 全文"),
            ("entry", "fs_main", "片元入口名"),
        ] {
            let empty = frame_material_doc().replace(
                &format!(r#""{field}": "{original}""#),
                &format!(r#""{field}": """#),
            );
            let spec: SceneSpec = serde_json::from_str(&empty).expect("解析");
            let err = spec.check().expect_err("空的那一栏 ⇒ 拒");
            assert!(
                err.contains(fragment),
                "要说清是哪一栏（{fragment}）：{err}"
            );
        }
    }

    /// **§125 的硬判据**：六份冻结的原始产物，读进来再写回去必须**逐字节相同**。
    ///
    /// 这是"给 `PassSpec` 加字段是纯加法"的唯一证据：字节不动 ⇒ 产物键不动 ⇒
    /// 五个锚**由构造保证**仍然有效。⚠ 只判"还能解析"是不够的 —— 那种判据在
    /// "新字段被写成默认值落盘"时照样绿，而那正是会改字节的情形。
    ///
    /// sha256 那一栏是烘图时的 oracle 读数（`Get-FileHash`，§125 那张表）；这里对的是
    /// **字节数**，因为 `px_protocol` 里没有 sha256，而"为一个判据引 crate"或
    /// "抄第三份摘要"都被本仓库自己的口径否掉（`px_render::digest` 开头那段）。
    /// 逐字节相同比 sha256 相同**更强**，所以缺的不是判据、只是"输入没被人换过"那道保险。
    #[test]
    fn the_frozen_originals_round_trip_byte_for_byte() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("px_protocol 上面就是工作区根")
            .join("target/oracle/pxart-frozen");
        if !dir.is_dir() {
            println!(
                "⚠ 跳过：{} 不在（target/ 不入 git）—— 不是通过，是没测",
                dir.display()
            );
            return;
        }
        // 名字 + 字节数（§125 那张表）。字节数是"输入没被换过"的那道保险：
        // 六份互不相同，换掉任何一份都对不上。
        let frozen = [
            ("orbit-bare.pxart", 4126_usize),
            ("orbit-bare-nolight.pxart", 4136),
            ("orbit-bare-shadow.pxart", 4138),
            ("orbit-proxy-fine-bound.pxart", 5749),
            ("orbit-rings.pxart", 4890),
            ("orbit-soft.pxart", 5721),
        ];
        // ⚠ `orbit-proxy-fine-bound` 有一处**先于本次改动**的漂移，见 `KNOWN_DRIFT` 那段。
        let mut identical: Vec<String> = Vec::new();
        let mut drifted: Vec<String> = Vec::new();
        for (name, size) in frozen {
            let path = dir.join(name);
            if !path.exists() {
                println!("⚠ 跳过 {name}：不在");
                continue;
            }
            let original = std::fs::read(&path).expect("读冻结产物");
            assert_eq!(
                original.len(),
                size,
                "{name} 的字节数与 §125 那张表不符 —— 输入被换过了？"
            );
            let frames = crate::stream::read_stream(&mut original.as_slice())
                .unwrap_or_else(|err| panic!("{name} 读不动：{err}"));
            let spec = frames
                .iter()
                .find_map(|frame| match frame {
                    crate::stream::Frame::Scene(spec) => Some(spec.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name} 里没有场景帧"));
            let fingerprint = frames
                .iter()
                .find_map(|frame| match frame {
                    crate::stream::Frame::Art(bundle) => {
                        bundle.assets.first().map(|asset| asset.fingerprint)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{name} 里没有清单帧"));
            let again = scene_bytes(&spec, fingerprint).expect("重新拼字节");
            if again == original {
                identical.push(name.to_string());
                println!("{name}：{} 字节，读→写逐字节相同 ✓", original.len());
                continue;
            }
            // 不一样就得说清**差在哪**：逐载荷比，而且差异必须落在数上、只差 1 个 ulp。
            let before = payload_spans(&original);
            let after = payload_spans(&again);
            assert_eq!(
                before.len(),
                after.len(),
                "{name}：帧数不同（原 {}、新 {}）",
                before.len(),
                after.len()
            );
            let mut verdicts: Vec<String> = Vec::new();
            for (index, ((left_at, left_len), (right_at, right_len))) in
                before.iter().zip(after.iter()).enumerate()
            {
                let left = &original[*left_at..*left_at + *left_len];
                let right = &again[*right_at..*right_at + *right_len];
                if left == right {
                    continue;
                }
                let verdict = only_one_ulp_of_one_number(
                    &String::from_utf8_lossy(left),
                    &String::from_utf8_lossy(right),
                )
                .unwrap_or_else(|err| {
                    let dump = Path::new(env!("CARGO_MANIFEST_DIR"))
                        .parent()
                        .expect("工作区根")
                        .join("target/pxart-roundtrip");
                    let _ = std::fs::create_dir_all(&dump);
                    let _ =
                        std::fs::write(dump.join(format!("{name}.frame{index}.original")), left);
                    let _ =
                        std::fs::write(dump.join(format!("{name}.frame{index}.rewritten")), right);
                    panic!(
                        "{name}：第 {index} 帧（{} → {} 字节）不是「只差一个 ulp 的数」：{err}\n  \
                         两份都落在 {}，直接 diff 就能看出是哪一格",
                        left.len(),
                        right.len(),
                        dump.display()
                    )
                });
                verdicts.push(format!("第 {index} 帧：{verdict}"));
            }
            assert_eq!(
                verdicts.len(),
                1,
                "{name}：有 {} 帧都不一样，判据只认「恰好一帧、一个数、1 个 ulp」",
                verdicts.len()
            );
            drifted.push(format!("{name}：{}", verdicts[0]));
            println!(
                "{name}：{} 字节 ⇒ {} 字节（{}）",
                original.len(),
                again.len(),
                verdicts[0]
            );
        }
        println!("逐字节相同：{}", identical.join(" / "));
        for line in &drifted {
            println!("有漂移：{line}");
        }
        let done = identical.len() + drifted.len();
        assert!(done == frozen.len(), "有冻结产物没跑到（缺文件？）");
        // ⚠ 名单是**钉住**的：将来谁再漂一份，这里就红 —— 而不是被
        // 「只差 1 个 ulp」那条宽容的判据悄悄放过（那正是最坏的一种绿灯）。
        // 六份**全部**逐字节相同：开启 `serde_json` 的 `float_roundtrip` 之后就是这样（§126）。
        assert_eq!(
            identical,
            vec![
                "orbit-bare.pxart",
                "orbit-bare-nolight.pxart",
                "orbit-bare-shadow.pxart",
                "orbit-proxy-fine-bound.pxart",
                "orbit-rings.pxart",
                "orbit-soft.pxart",
            ],
            "逐字节相同的名单变了"
        );
        // ⚠ 这两条**由名单驱动**（而不是写死"必须为空"）：名单现在是空的，
        // 所以结论就是"一份都不许漂"；将来真要放宽，改的是名单那一行，
        // 而不是把这里的判据删掉 —— 判据松掉是最坏的一种绿灯。
        assert_eq!(
            drifted.len(),
            KNOWN_DRIFT.len(),
            "漂移的份数变了（名单里 {} 份）：{drifted:?}",
            KNOWN_DRIFT.len()
        );
        for (line, expected) in drifted.iter().zip(KNOWN_DRIFT.iter()) {
            assert!(
                line.starts_with(expected),
                "漂移的不是预期那一份：{line}（预期 {expected}）"
            );
        }
    }
}
