use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::art::Camera;

pub const SCENE_SCHEMA: u32 = 3;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Num(f64),
    Text(String),
    Triple([f32; 3]),
    Quad([f32; 4]),
}

pub const SHADOW_FACE_BASIS: [[[f32; 3]; 3]; 6] = [
    [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
    [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]],
    [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
    [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, -1.0, 0.0]],
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]],
    [[-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowPlan {
    pub table: Vec<u8>,
    pub light_offsets: Vec<u32>,
    pub atlas_side: u32,
    pub layers: u32,
    pub faces: Vec<[f32; 3]>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    Mesh {
        member: Member,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bounding_radius: Option<f32>,
    },
    Primitive {
        name: String,
        #[serde(default)]
        params: BTreeMap<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bounding_radius: Option<f32>,
    },
}

impl Geometry {
    pub fn mesh(member: Member) -> Self {
        Self::Mesh {
            member,
            bounding_radius: None,
        }
    }

    pub fn primitive(name: &str, params: BTreeMap<String, Value>) -> Self {
        Self::Primitive {
            name: name.to_string(),
            params,
            bounding_radius: None,
        }
    }

    pub fn with_bounding_radius(mut self, radius: f32) -> Self {
        let slot = match &mut self {
            Self::Mesh {
                bounding_radius, ..
            } => bounding_radius,
            Self::Primitive {
                bounding_radius, ..
            } => bounding_radius,
        };
        *slot = Some(radius);
        self
    }

    pub fn bounding_radius(&self) -> Option<f32> {
        match self {
            Self::Mesh {
                bounding_radius, ..
            } => *bounding_radius,
            Self::Primitive {
                bounding_radius, ..
            } => *bounding_radius,
        }
    }

    pub fn member(&self) -> Option<&Member> {
        match self {
            Self::Mesh { member, .. } => Some(member),
            Self::Primitive { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Address {
    #[default]
    Repeat,
    ClampToEdge,
    MirrorRepeat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    #[default]
    Linear,
    Nearest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sampler {
    #[serde(default)]
    pub address_u: Address,
    #[serde(default = "clamp_edge")]
    pub address_v: Address,
    #[serde(default)]
    pub filter: Filter,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    #[default]
    Opaque,
    Premultiplied,
    Blend,
    Add,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CullMode {
    #[default]
    Back,
    Front,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub shader: Member,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub textures: BTreeMap<String, TextureRef>,
    #[serde(default)]
    pub alpha: AlphaMode,
    #[serde(default)]
    pub cull: CullMode,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub id: String,
    pub geometry: Geometry,
    pub material: Material,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default = "cast_shadow_default")]
    pub cast_shadow: bool,
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub shadow_density: f32,
}

fn is_zero_f32(value: &f32) -> bool {
    *value == 0.0
}

fn cast_shadow_default() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
    Spot,
    Directional,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Light {
    pub id: String,
    pub kind: LightKind,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default = "forward")]
    pub direction: [f32; 3],
    #[serde(default = "white")]
    pub color: [f32; 3],
    pub intensity: f32,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    #[serde(default)]
    pub ambient: f32,
    #[serde(default)]
    pub skybox: Option<Member>,
    #[serde(default = "skybox_brightness_default")]
    pub skybox_brightness: f32,
}

fn skybox_brightness_default() -> f32 {
    900.0
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassResource {
    pub name: String,
    pub format: String,
    pub size: String,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawSpec {
    pub geometry: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub material: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameMaterial {
    pub name: String,
    pub shader: String,
    pub entry: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassSpec {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shader: Option<Member>,
    #[serde(default)]
    pub label: String,
    #[serde(default = "fragment_entry")]
    pub entry: String,
    #[serde(default)]
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub draws: Vec<DrawSpec>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vertex_shader: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vertex_entry: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub render: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cube_face: Option<PassCubeFace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<[f32; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassCubeFace {
    pub light: u32,
    pub face: u32,
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
        match &self.shader {
            Some(shader) => format!("{index}:{}/{}", shader.graph, shader.node),
            None => format!("{index}:{}", self.kind),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneSpec {
    pub schema: u32,
    pub name: String,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub cameras: Vec<Camera>,
    #[serde(default)]
    pub expects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<PassResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub passes: Vec<PassSpec>,
    #[serde(default)]
    pub lights: Vec<Light>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowPlan>,
    pub objects: Vec<Object>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frame_materials: Vec<FrameMaterial>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub material_instances: Vec<MaterialInstance>,
}

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
            if !object.shadow_density.is_finite() || object.shadow_density < 0.0 {
                return Err(format!(
                    "物体 '{}' 的阴影密度是 {}：它要么是 0（不参与虚拟影图的分配），\
                     要么是一个正数（每单位世界长度要多少 texel）",
                    object.id, object.shadow_density
                ));
            }
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
                "fullscreen" | "geometry" | "copy" | "compute" => {}
                other => {
                    return Err(format!(
                        "{at} 的 kind 是 '{other}'：认 'fullscreen' 与 'geometry' 与 'copy' 与 'compute'"
                    ));
                }
            }
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
                match pass.kind.as_str() {
                    "geometry" => {}
                    "fullscreen" => {}
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
        let mut instance_names: Vec<&str> = Vec::new();
        for (index, instance) in self.material_instances.iter().enumerate() {
            let at = format!("第 {index} 份材质实例");
            if instance.name.trim().is_empty() {
                return Err(format!(
                    "{at} 没给名字：`draws[].material` 是按名字引用它的"
                ));
            }
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
                Geometry::Mesh { member, .. } => format!("网格 {member}"),
                Geometry::Primitive { name, params, .. } => {
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

pub fn write_scene(path: &Path, spec: &SceneSpec, fingerprint: u64) -> Result<u64, String> {
    let bytes = scene_bytes(spec, fingerprint)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, &bytes).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    Ok(bytes.len() as u64)
}

pub fn scene_bytes(spec: &SceneSpec, fingerprint: u64) -> Result<Vec<u8>, String> {
    spec.check()?;
    let bundle = crate::art::ArtBundle {
        assets: vec![crate::art::AssetManifest {
            id: spec.name.clone(),
            params: BTreeMap::from([
                ("schema".to_string(), f64::from(spec.schema)),
                ("objects".to_string(), spec.objects.len() as f64),
                ("lights".to_string(), spec.lights.len() as f64),
            ]),
            blobs: Vec::new(),
            fingerprint,
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

    fn doc() -> String {
        DOC.replace("\"schema\": 0", &format!("\"schema\": {SCENE_SCHEMA}"))
    }

    const DOC: &str = r#"{
        "schema": 0,
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
        let spec: SceneSpec = serde_json::from_str(&doc()).expect("自己的文档必须解析得动");
        assert_eq!(spec.objects.len(), 1);
        assert_eq!(spec.objects[0].material.params.len(), 1);
        spec.check().expect("这份夹具是合法的");
    }

    #[test]
    fn an_unknown_field_is_refused_not_ignored() {
        let object = DOC.replace(
            r#""id": "planet","#,
            r#""id": "planet", "cast_shadow_typo": true,"#,
        );
        let err = serde_json::from_str::<SceneSpec>(&object).expect_err("物体上的未知字段必须报错");
        assert!(err.to_string().contains("cast_shadow_typo"), "{err}");

        let material = DOC.replace(r#""gain": 1.0"#, r#""gain": 1.0, "roughness": 0.4"#);
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

    fn old_pass_doc() -> String {
        doc().replace(
            r#""lights": ["#,
            r#""passes": [{"kind": "fullscreen", "shader": {"graph": "shaders", "node": "grade", "key": "22"}, "writes": ["view"]}], "lights": ["#,
        )
    }

    fn geometry_doc() -> String {
        doc().replace(
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
        assert_eq!(serde_json::to_string(&spec).expect("再序列化"), text);
    }

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
        assert!(pass.shader.is_none(), "几何 pass 不该有 pass 级 shader");

        let text = serde_json::to_string(&spec).expect("序列化");
        let pass_text = serde_json::to_string(&spec.passes[0]).expect("序列化这条 pass");
        assert!(
            !pass_text.contains("\"shader\""),
            "几何 pass 落盘时不该出现 shader 键：{pass_text}"
        );
        let back: SceneSpec = serde_json::from_str(&text).expect("再解析");
        assert_eq!(back, spec, "带新字段的文档必须逐字往返");
        for key in ["draws", "vertex_shader", "render", "depth_target"] {
            assert!(text.contains(key), "{key} 没落盘：{text}");
        }
    }

    fn frame_material_doc() -> String {
        doc().replace(
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
        assert!(
            text.contains("@fragment fn fs_main"),
            "WGSL 全文必须真的在文档里：{text}"
        );
    }

    #[test]
    fn a_broken_frame_material_is_refused_by_name() {
        let clash = frame_material_doc()
            .replace(r#""name": "skybox""#, r#""name": "planet""#)
            .replace(r#""material": "skybox""#, r#""material": "planet""#);
        let spec: SceneSpec = serde_json::from_str(&clash).expect("解析");
        let err = spec.check().expect_err("帧材质与物体撞名 ⇒ 拒");
        assert!(err.contains("planet"), "要说清是哪个名字：{err}");
        assert!(err.contains("撞名"), "{err}");

        let two = frame_material_doc().replace(
            r#""frame_materials": [{"#,
            r#""frame_materials": [{"name": "skybox", "shader": "x", "entry": "fs_main"}, {"#,
        );
        let spec: SceneSpec = serde_json::from_str(&two).expect("解析");
        let err = spec.check().expect_err("帧材质名字重了 ⇒ 拒");
        assert!(err.contains("重了"), "{err}");

        let unused =
            frame_material_doc().replace(r#""material": "skybox""#, r#""material": "planet""#);
        let spec: SceneSpec = serde_json::from_str(&unused).expect("解析");
        let err = spec.check().expect_err("没人用 ⇒ 拒");
        assert!(err.contains("skybox"), "要说清是哪一份没用上：{err}");
        assert!(err.contains("planet"), "要列出 draws 真正引用的名字：{err}");

        let dangling =
            frame_material_doc().replace(r#""material": "skybox""#, r#""material": "skyboox""#);
        let spec: SceneSpec = serde_json::from_str(&dangling).expect("解析");
        let err = spec.check().expect_err("解析不到的材质名 ⇒ 拒");
        assert!(err.contains("skyboox"), "{err}");
        assert!(err.contains("planet"), "要列出物体 id 那张表：{err}");

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

    #[test]
    fn what_the_writer_produces_reads_back_and_writes_again_byte_for_byte() {
        let spec: SceneSpec = serde_json::from_str(&doc()).expect("夹具必须解析得动");
        let fingerprint = 0x0123_4567_89AB_CDEF_u64;
        let once = scene_bytes(&spec, fingerprint).expect("第一次写");
        let frames = crate::stream::read_stream(&mut once.as_slice()).expect("读回来");
        let read_back = frames
            .iter()
            .find_map(|frame| match frame {
                crate::stream::Frame::Scene(spec) => Some(spec.clone()),
                _ => None,
            })
            .expect("写出去的字节里应当有场景帧");
        assert_eq!(
            read_back, spec,
            "写出去的字节读回来与原来那份不是同一个场景（有栏丢了或被默认值顶掉了）"
        );
        let again = scene_bytes(&read_back, fingerprint).expect("第二次写");
        assert!(
            again == once,
            "读→写不是幂等：同一份场景写两遍给出了不同的字节（{} → {}）。\
             ⚠ 最常见的原因是某一栏**缺省时仍被落盘** —— 那正好就是「加字段不纯是加法」",
            once.len(),
            again.len()
        );
    }
}
