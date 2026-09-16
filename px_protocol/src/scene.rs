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
        write!(formatter, "{}/{}@{}", self.graph, self.node, self.short_key())
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
    Mesh { member: Member },
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
    /// 点/聚光的射程（米）。缺省 = 按强度反推（`range = √(intensity/最小照度)`）。
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

/// 一般渲染文档：环境 + 相机表 + 灯表 + 物体表。
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
    #[serde(default)]
    pub lights: Vec<Light>,
    pub objects: Vec<Object>,
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
            out.extend(object.material.textures.values().map(|texture| &texture.member));
        }
        if let Some(skybox) = &self.environment.skybox {
            out.push(skybox);
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, &bytes).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    Ok(bytes.len() as u64)
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
        let object = DOC.replace(r#""id": "planet","#, r#""id": "planet", "cast_shadow_typo": true,"#);
        let err = serde_json::from_str::<SceneSpec>(&object).expect_err("物体上的未知字段必须报错");
        assert!(err.to_string().contains("cast_shadow_typo"), "{err}");

        let material = DOC.replace(r#""gain": 1.0"#, r#""gain": 1.0, "roughness": 0.4"#);
        // 材质参数表是**按名字自由**的（那正是「加参数」要走的路），所以这里**不该**报错 ——
        // 名字对不对由 shader 的 descriptor 说了算，不是协议说了算。
        let spec: SceneSpec = serde_json::from_str(&material).expect("材质参数表按名字自由");
        assert_eq!(spec.objects[0].material.params.len(), 2);

        let texture = DOC.replace(r#""binding": 1,"#, r#""binding": 1, "wrap_typo": "repeat","#);
        let err = serde_json::from_str::<SceneSpec>(&texture).expect_err("贴图上的未知字段必须报错");
        assert!(err.to_string().contains("wrap_typo"), "{err}");

        let spec_level = DOC.replace(r#""name": "夹具","#, r#""name": "夹具", "ambient_typo": 1.0,"#);
        let err = serde_json::from_str::<SceneSpec>(&spec_level).expect_err("文档上的未知字段必须报错");
        assert!(err.to_string().contains("ambient_typo"), "{err}");

        let geometry = DOC.replace(
            r#"{"source": "primitive", "name": "icosphere", "params": {"radius": 1.0}}"#,
            r#"{"source": "primitive", "name": "icosphere", "subdivisons": 64, "params": {"radius": 1.0}}"#,
        );
        let err = serde_json::from_str::<SceneSpec>(&geometry).expect_err("几何上的未知字段必须报错");
        assert!(err.to_string().contains("subdivisons"), "{err}");
    }
}
