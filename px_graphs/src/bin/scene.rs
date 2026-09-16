//! 场景编译器：**配方**（`art/scene/*.toml`）→ **通用渲染文档**（`.pxart`）。
//!
//! 「行星 / 云 / 大气 / 环」这些语义**住在这里**，不在渲染器里（§65）。它做三件事：
//!
//! 1. 把配方里的 part 展开成**物体**：几何（网格产物或内建图元）+ 材质（shader 产物 +
//!    按名字给的参数 + 按绑定下标给的贴图）+ 世界系变换；
//! 2. 把需要程序化生成的东西**烘成产物**（色板贴图 / 覆盖度立方图 / 星空 / 环），
//!    写进 CAS 的 `generated` 图 —— 渲染器只认产物，不生成任何东西；
//! 3. 把"怎么看"（评审相机表）与"照什么"（灯表、环境）一并写进文档。
//!
//! 配方文件的形状**没变**（还是 `[[parts]]` + `kind` + `members` + `params`）：它是艺术内容，
//! 改的是它编译成什么。
//!
//! ⚠ 倾斜（`SYSTEM_TILT`）只在这里出现一次：文档里的方向与朝向**都是世界系**的。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_ops::generate::{self, Generated, Palette};
use px_ops::{GraphSpec, ManifestEntry};
use px_protocol::art::{Camera, TextureFormat};
use px_protocol::scene::{
    AlphaMode, CullMode, Environment, Geometry, Light, Material, Member, Object, Sampler, SceneSpec,
    TextureRef, Transform, Value,
};
use px_protocol::wire::DType;
use serde::Deserialize;

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_ops::noise::fnv1a(include_str!("scene.rs"));
const DEFAULT_SCENE: &str = "orbit";
/// 生成物的图名：文档里的成员写成 `generated::surface_color` 这种。
const GENERATED: &str = "generated";
/// 天空盒的面尺寸。原来是渲染器里写死的 512。
const STARS_FACE: u32 = 512;
/// 环的段数与环带贴图尺寸。原来是渲染器里写死的 384 / 1024×4。
const RING_SEGMENTS: u32 = 384;
const RING_BAND: (u32, u32) = (1024, 4);
/// 场景倾斜：局部系 → 世界系。**只在这里出现一次**（渲染器里没有这个常数了）。
const SYSTEM_TILT: f32 = 0.34;
/// 点光源的射程系数（`|position| × 2.5`）。原来是渲染器的 `SUN_RANGE_FACTOR`。
const SUN_RANGE_FACTOR: f32 = 2.5;
/// 天空盒亮度。原来是渲染器的 `SKY_BRIGHTNESS`。
const SKYBOX_BRIGHTNESS: f32 = 900.0;
/// 云影的默认"有多不透明"（覆盖度 1 处压掉约 86% 直接光）与"指定高度"缺省。
const CLOUD_SHADOW_GAIN: f32 = 2.0;
const CLOUD_SHADOW_HEIGHT: f32 = 0.5;
/// 云壳的缺省内/外半径因子（× 行星半径）。原来是 `px_render::clouds` 里的两个常数。
const CLOUD_BASE: f32 = 1.01;
const CLOUD_TOP: f32 = 1.06;

// ---------------------------------------------------------------------------
// 配方文件
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct SceneFile {
    name: String,
    #[serde(default)]
    ambient: f32,
    /// `"review"` = 评审相机表；缺省也用它。
    #[serde(default)]
    cameras: Option<String>,
    parts: Vec<PartFile>,
}

#[derive(Deserialize)]
struct PartFile {
    id: String,
    kind: String,
    shader: String,
    /// 这个 part 的成员默认属于哪张图；跨图的成员写 `图名::节点名`。
    #[serde(default)]
    graph: Option<String>,
    #[serde(default)]
    members: BTreeMap<String, String>,
    #[serde(default)]
    params: BTreeMap<String, toml::Value>,
}

impl PartFile {
    fn number(&self, key: &str) -> Result<f64, String> {
        match self.params.get(key) {
            Some(toml::Value::Integer(value)) => Ok(*value as f64),
            Some(toml::Value::Float(value)) => Ok(*value),
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要一个数，实际是 {other:?}",
                self.id
            )),
            None => Err(format!(
                "part '{}' 缺参数 '{key}'；它有的参数：{}",
                self.id,
                if self.params.is_empty() {
                    "（空）".to_string()
                } else {
                    self.params.keys().cloned().collect::<Vec<_>>().join(" / ")
                }
            )),
        }
    }

    fn number_or(&self, key: &str, fallback: f32) -> f32 {
        match self.params.get(key) {
            Some(toml::Value::Integer(value)) => *value as f32,
            Some(toml::Value::Float(value)) => *value as f32,
            _ => fallback,
        }
    }

    fn integer_or(&self, key: &str, fallback: u32) -> u32 {
        match self.params.get(key) {
            Some(toml::Value::Integer(value)) => (*value).max(0) as u32,
            Some(toml::Value::Float(value)) => (*value).max(0.0) as u32,
            _ => fallback,
        }
    }

    fn text(&self, key: &str) -> Result<&str, String> {
        match self.params.get(key) {
            Some(toml::Value::String(value)) => Ok(value),
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要一段文本，实际是 {other:?}",
                self.id
            )),
            None => Err(format!("part '{}' 缺参数 '{key}'", self.id)),
        }
    }

    fn text_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        match self.params.get(key) {
            Some(toml::Value::String(value)) => value,
            _ => fallback,
        }
    }

    fn triple(&self, key: &str) -> Result<[f32; 3], String> {
        match self.params.get(key) {
            Some(toml::Value::Array(items)) if items.len() == 3 => {
                let mut out = [0.0_f32; 3];
                for (slot, item) in out.iter_mut().zip(items.iter()) {
                    let number = item
                        .as_float()
                        .or_else(|| item.as_integer().map(|value| value as f64))
                        .ok_or_else(|| {
                            format!("part '{}' 参数 '{key}' 里有一个不是数：{item:?}", self.id)
                        })?;
                    *slot = number as f32;
                }
                Ok(out)
            }
            Some(other) => Err(format!(
                "part '{}' 参数 '{key}' 要三个数，实际是 {other:?}",
                self.id
            )),
            None => Err(format!("part '{}' 缺参数 '{key}'", self.id)),
        }
    }

    /// 只认这些参数名。**不许静默忽略**：打错一个字母在配方里看起来完全正常，
    /// 而结果是一档悄悄退回缺省的画面。
    fn check_keys(&self, known: &[&str]) -> Result<(), String> {
        for key in self.params.keys() {
            if !known.contains(&key.as_str()) {
                return Err(format!(
                    "part '{}'（kind {}）不认识参数 '{key}'；它认：{}",
                    self.id,
                    self.kind,
                    known.join(" / ")
                ));
            }
        }
        Ok(())
    }
}

fn member_of(part: &PartFile, role: &str, reference: &str) -> Member {
    let (graph, node) = match reference.split_once("::") {
        Some((graph, node)) => (graph.to_string(), node.to_string()),
        None => {
            let graph = part.graph.clone().unwrap_or_else(|| {
                panic!(
                    "part '{}' 的成员 '{role}' 没说属于哪张图：要么给 part.graph，要么写成 图名::节点名",
                    part.id
                )
            });
            (graph, reference.to_string())
        }
    };
    let key = px_ops::manifest_key_of(&graph, &node).unwrap_or_else(|err| panic!("{err}"));
    Member::new(&graph, &node, &key)
}

fn member_at(part: &PartFile, role: &str) -> Result<Member, String> {
    let reference = part.members.get(role).ok_or_else(|| {
        format!(
            "part '{}'（kind {}）要成员 '{role}'；这份有：{}",
            part.id,
            part.kind,
            if part.members.is_empty() {
                "（空）".to_string()
            } else {
                part.members.keys().cloned().collect::<Vec<_>>().join(" / ")
            }
        )
    })?;
    Ok(member_of(part, role, reference))
}

fn optional_member(part: &PartFile, role: &str) -> Option<Member> {
    part.members
        .get(role)
        .map(|reference| member_of(part, role, reference))
}

/// part 用的那份 WGSL。配方里 `shader = "surface"` 是给人看的名字，真本由
/// `members.shader` 指 —— 两个对不上就报错：否则改了一处、另一处还写着老名字，
/// 读配方的人会以为换的是另一个 shader（而槽名那套已经随通用材质一起没了）。
fn shader_member(part: &PartFile) -> Result<Member, String> {
    let member = member_at(part, "shader")?;
    if member.node != part.shader {
        return Err(format!(
            "part '{}'：`shader = \"{}\"` 与成员 `shader = \"{}\"` 指的不是同一份 WGSL",
            part.id, part.shader, member.node
        ));
    }
    Ok(member)
}

// ---------------------------------------------------------------------------
// 四元数：烘图侧要自己算朝向与倾斜（渲染器不算了）
// ---------------------------------------------------------------------------

fn quat_x(angle: f32) -> [f32; 4] {
    [(angle * 0.5).sin(), 0.0, 0.0, (angle * 0.5).cos()]
}

fn quat_y(angle: f32) -> [f32; 4] {
    [0.0, (angle * 0.5).sin(), 0.0, (angle * 0.5).cos()]
}

/// 汉密尔顿积。**逐项次序照抄 glam 的 `Quat::mul`**：数学上等价的另一种写法
/// （把同一批项按别的顺序相加）在 f32 下会差最后一位 —— 差别会被 `looking_at` 放大成
/// 亚像素抖动，量出来就是"个别像素差 1~22"（本轮真踩过：1112 个像素）。
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let (ax, ay, az, aw) = (a[0], a[1], a[2], a[3]);
    let (bx, by, bz, bw) = (b[0], b[1], b[2], b[3]);
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by + ay * bw + az * bx - ax * bz,
        aw * bz + az * bw + ax * by - ay * bx,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// 用四元数转一个向量。同样是 **glam `Quat::mul_vec3` 的逐项次序**
/// （`v(w²−b·b) + 2b(v·b) + 2w(b×v)`），不是常见的 `v + 2w(q×v) + 2q×(q×v)`。
fn rotate(quaternion: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let (x, y, z, w) = (quaternion[0], quaternion[1], quaternion[2], quaternion[3]);
    let (vx, vy, vz) = (vector[0], vector[1], vector[2]);
    let axis = [x, y, z];
    let square = x * x + y * y + z * z;
    let dot = vx * x + vy * y + vz * z;
    let cross = [
        y * vz - z * vy,
        z * vx - x * vz,
        x * vy - y * vx,
    ];
    let scale = w * w - square;
    let twice = dot * 2.0;
    let spin = w * 2.0;
    [
        vx * scale + axis[0] * twice + cross[0] * spin,
        vy * scale + axis[1] * twice + cross[1] * spin,
        vz * scale + axis[2] * twice + cross[2] * spin,
    ]
}

/// 场景整体的朝向：`rot_x(TILT) × rot_y(spin)` —— 与迁移前渲染器里那个父实体 + 自转合成的一致。
fn orientation(spin: f32) -> [f32; 4] {
    quat_mul(quat_x(SYSTEM_TILT), quat_y(spin))
}

/// 向量长度。**逐项次序照抄 glam 的 `Vec3::length`**：SSE2 那条路的横向加法是
/// `(x² + z²) + y²`，不是手写的 `(x² + y²) + z²` —— 两者在 f32 下会差最后一位，
/// 而这一位会顺着 `range_attenuation` 漏进光照，表现为几十个像素差 1（本轮真踩过）。
fn length(vector: [f32; 3]) -> f32 {
    let (x, y, z) = (vector[0], vector[1], vector[2]);
    ((x * x + z * z) + y * y).sqrt()
}

// ---------------------------------------------------------------------------
// 生成物：写进 CAS，并在 `generated` 图里登记
// ---------------------------------------------------------------------------

struct Generated2 {
    generated: Vec<ManifestEntry>,
}

impl Generated2 {
    fn new() -> Self {
        Self {
            generated: Vec::new(),
        }
    }

    fn texture(&mut self, node: &str, data: generate::TextureData, op: &str) -> Member {
        let dtype = match data.format {
            TextureFormat::Rgba8Srgb => DType::U8,
            TextureFormat::Rgba16Float => DType::U16,
        };
        let written = generate::write_texture(node, data.shape(), &data.bytes, dtype)
            .unwrap_or_else(|err| panic!("写贴图产物 {node} 失败：{err}"));
        self.register(node, op, &written, data.bytes.len() as u64);
        Member::new(GENERATED, node, &px_ops::hex(&written.key))
    }

    fn mesh(&mut self, node: &str, mesh: &px_protocol::art::MeshData, op: &str) -> Member {
        let written = generate::write_generated_mesh(node, mesh)
            .unwrap_or_else(|err| panic!("写网格产物 {node} 失败：{err}"));
        self.register(node, op, &written, 0);
        Member::new(GENERATED, node, &px_ops::hex(&written.key))
    }

    fn register(&mut self, node: &str, op: &str, written: &Generated, payload: u64) {
        println!(
            "{} {node:<16} {op:<20} {}  {:>9} B{}",
            if written.hit { "命中" } else { "重算" },
            px_ops::hex_short(&written.key),
            written.bytes,
            if written.hit { "（CAS 里已有）" } else { "" },
        );
        self.generated.push(ManifestEntry {
            node: node.to_string(),
            op: op.to_string(),
            op_version: 1,
            key: px_ops::hex(&written.key),
            hit: written.hit,
            millis: written.millis,
            bytes: written.bytes,
            min: 0.0,
            max: 0.0,
            mean: payload as f32,
        });
    }

    fn finish(&self) {
        let mut entries = px_ops::graph_manifest(GENERATED).unwrap_or_default();
        for entry in &self.generated {
            entries.retain(|old| old.node != entry.node);
            entries.push(entry.clone());
        }
        entries.sort_by(|one, two| one.node.cmp(&two.node));
        let path = px_ops::write_graph_manifest(GENERATED, &entries)
            .unwrap_or_else(|err| panic!("写 {GENERATED} 清单失败：{err}"));
        println!(
            "清单 {}｜共 {} 份生成物",
            path.display(),
            entries.len()
        );
    }
}

/// 场成员 → 场（贴图/覆盖度立方图的输入）。
fn field_of(member: &Member, root: &Path) -> px_ops::field::Field {
    let path = member
        .resolve(root)
        .unwrap_or_else(|err| panic!("{err}"));
    generate::load_field(&path.display().to_string())
        .unwrap_or_else(|err| panic!("读场 {} 失败：{err}", path.display()))
}

// ---------------------------------------------------------------------------
// 三个装配器（语义全在这里）
// ---------------------------------------------------------------------------

/// 云的形状档：原来是 `px_render::clouds::CloudShape`（渲染器侧）。缺省值逐项照抄。
#[derive(Clone, Copy)]
struct CloudShape {
    coverage: f32,
    base: f32,
    top: f32,
    detail_scale: f32,
    detail_strength: f32,
    erode: f32,
    phase: f32,
    shadow: f32,
    steps: u32,
    bump: f32,
    seed: u32,
    slope_scale: f32,
    taper: f32,
    coverage_gain: f32,
    surface_level: f32,
    bound: u32,
    gradient: u32,
    wind: f32,
    wind_skin: f32,
}

impl Default for CloudShape {
    fn default() -> Self {
        Self {
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            detail_scale: 16.0,
            detail_strength: 0.55,
            erode: 0.0,
            phase: 0.62,
            shadow: 1.0,
            steps: 56,
            bump: 0.85,
            seed: 7,
            slope_scale: 0.12,
            taper: 0.45,
            coverage_gain: 2.6,
            surface_level: 0.20,
            bound: 0,
            gradient: 1,
            wind: 0.0,
            wind_skin: 0.0,
        }
    }
}

/// 消融档：仪器档的名字 → 码（`clouds.wgsl` 里的 `ABLATE_*`）。
fn ablate_code(name: &str) -> Result<u32, String> {
    match name {
        "none" => Ok(0),
        "sun" => Ok(1),
        "noise" => Ok(2),
        "fetch" => Ok(3),
        "detail" => Ok(4),
        "surface" => Ok(5),
        "normals" => Ok(6),
        other => Err(format!(
            "消融档只认 none / sun / noise / fetch / detail / surface / normals，不认 '{other}'"
        )),
    }
}

const PLANET_KEYS: [&str; 15] = [
    "palette",
    "displace",
    "sea_level",
    "radius",
    "spin",
    "rings",
    "shadows",
    "cloud_shadow",
    "shadow_height",
    "light_position",
    "light_color",
    "light_intensity",
    "light_range",
    "atmo",
    "ablate",
];

const CLOUDS_KEYS: [&str; 24] = [
    "inner",
    "outer",
    "extinction",
    "coverage",
    "base",
    "top",
    "detail_scale",
    "detail_strength",
    "erode",
    "phase",
    "shadow",
    "steps",
    "bump",
    "seed",
    "slope_scale",
    "taper",
    "coverage_gain",
    "surface_level",
    "bound",
    "gradient",
    "wind",
    "wind_skin",
    "ablate",
    "tint",
];

const ATMOSPHERE_KEYS: [&str; 5] = ["inner", "outer", "density", "softness", "tint"];

fn cloud_shape_of(part: &PartFile) -> Result<CloudShape, String> {
    part.check_keys(&CLOUDS_KEYS)?;
    let default = CloudShape::default();
    Ok(CloudShape {
        coverage: part.number_or("coverage", default.coverage),
        base: part.number_or("base", default.base),
        top: part.number_or("top", default.top),
        detail_scale: part.number_or("detail_scale", default.detail_scale),
        detail_strength: part.number_or("detail_strength", default.detail_strength),
        erode: part.number_or("erode", default.erode),
        phase: part.number_or("phase", default.phase),
        shadow: part.number_or("shadow", default.shadow),
        steps: part.integer_or("steps", default.steps),
        bump: part.number_or("bump", default.bump),
        seed: part.integer_or("seed", default.seed),
        slope_scale: part.number_or("slope_scale", default.slope_scale),
        taper: part.number_or("taper", default.taper),
        coverage_gain: part.number_or("coverage_gain", default.coverage_gain),
        surface_level: part.number_or("surface_level", default.surface_level),
        bound: part.integer_or("bound", default.bound),
        gradient: part.integer_or("gradient", default.gradient),
        wind: part.number_or("wind", default.wind),
        wind_skin: part.number_or("wind_skin", default.wind_skin),
    })
}

/// 云的参数块：**逐项就是 `clouds.wgsl` 里 `CloudParams` 的那 25 格**（顺序无所谓，
/// 渲染器按名字反射打包）。这里放的是值，不是布局。
fn cloud_params(
    part: &PartFile,
    shape: CloudShape,
    inner: f32,
    outer: f32,
    shape_orientation: [f32; 4],
) -> Result<BTreeMap<String, Value>, String> {
    let ablate = ablate_code(part.text_or("ablate", "none"))?;
    let tint = match part.params.get("tint") {
        Some(_) => part.triple("tint")?,
        None => [1.0, 0.99, 0.97],
    };
    Ok(BTreeMap::from([
        (
            "orientation".to_string(),
            Value::Quad(shape_orientation),
        ),
        (
            "tint".to_string(),
            Value::Quad([tint[0], tint[1], tint[2], 1.0]),
        ),
        ("inner".to_string(), Value::Num(f64::from(inner))),
        ("outer".to_string(), Value::Num(f64::from(outer))),
        (
            "density".to_string(),
            Value::Num(part.number("extinction")?),
        ),
        ("coverage".to_string(), Value::Num(f64::from(shape.coverage))),
        ("base".to_string(), Value::Num(f64::from(shape.base))),
        ("top".to_string(), Value::Num(f64::from(shape.top))),
        (
            "detail_scale".to_string(),
            Value::Num(f64::from(shape.detail_scale)),
        ),
        (
            "detail_strength".to_string(),
            Value::Num(f64::from(shape.detail_strength)),
        ),
        ("erode".to_string(), Value::Num(f64::from(shape.erode))),
        ("phase".to_string(), Value::Num(f64::from(shape.phase))),
        ("shadow".to_string(), Value::Num(f64::from(shape.shadow))),
        ("steps".to_string(), Value::Num(f64::from(shape.steps))),
        ("bump".to_string(), Value::Num(f64::from(shape.bump))),
        ("seed".to_string(), Value::Num(f64::from(shape.seed))),
        ("ablate".to_string(), Value::Num(f64::from(ablate))),
        (
            "slope_scale".to_string(),
            Value::Num(f64::from(shape.slope_scale)),
        ),
        ("taper".to_string(), Value::Num(f64::from(shape.taper))),
        (
            "coverage_gain".to_string(),
            Value::Num(f64::from(shape.coverage_gain)),
        ),
        (
            "surface_level".to_string(),
            Value::Num(f64::from(shape.surface_level)),
        ),
        ("bound".to_string(), Value::Num(f64::from(shape.bound))),
        (
            "gradient".to_string(),
            Value::Num(f64::from(shape.gradient)),
        ),
        ("wind".to_string(), Value::Num(f64::from(shape.wind))),
        (
            "wind_skin".to_string(),
            Value::Num(f64::from(shape.wind_skin)),
        ),
    ]))
}

/// 一份配方 → 一份通用渲染文档。
struct Compiled {
    document: SceneSpec,
}

fn main() {
    px_ops::begin(GraphSpec {
        name: "scene".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 0,
        height: 0,
        projection: px_ops::field::Projection::Cube,
        cameras: Vec::new(),
    });

    let recipe = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_SCENE.to_string());
    let path = PathBuf::from("art").join("scene").join(format!("{recipe}.toml"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
    let file: SceneFile = toml::from_str(&text)
        .unwrap_or_else(|err| panic!("{} 解不开：{err}", path.display()));

    let root = px_ops::cache_root();
    let compiled = compile(&file, &root).unwrap_or_else(|err| panic!("{} 编译失败：{err}", file.name));

    let spec_json = serde_json::to_string(&compiled.document).unwrap_or_else(|err| panic!("{err}"));
    let member_keys = compiled
        .document
        .members()
        .iter()
        .map(|member| member.key.clone())
        .collect::<Vec<_>>();
    let key = px_ops::scene_key(&spec_json, &member_keys);
    let artifact = px_protocol::scene::cas_path(&root, &px_ops::hex(&key))
        .unwrap_or_else(|err| panic!("{err}"));
    let bytes = px_protocol::scene::write_scene(&artifact, &compiled.document, px_ops::noise::fnv1a(&spec_json))
        .unwrap_or_else(|err| panic!("{err}"));

    println!("{}", compiled.document.audit());
    println!("产物 scene -> {}（{}）", artifact.display(), px_ops::hex_short(&key));

    let entry = ManifestEntry {
        node: compiled.document.name.clone(),
        op: "scene.document".to_string(),
        op_version: px_protocol::SCENE_SCHEMA,
        key: px_ops::hex(&key),
        hit: false,
        millis: 0,
        bytes,
        min: 0.0,
        max: 0.0,
        mean: compiled.document.objects.len() as f32,
    };
    // 清单按场景名合并：一台机器上会并存好几份场景（有云 / 无云 / …），
    // 后烘的不许把先烘的挤掉。
    let mut entries = px_ops::graph_manifest("scene").unwrap_or_default();
    entries.retain(|old| old.node != entry.node);
    entries.push(entry);
    entries.sort_by(|one, two| one.node.cmp(&two.node));
    let manifest = px_ops::write_graph_manifest("scene", &entries)
        .unwrap_or_else(|err| panic!("写清单失败：{err}"));
    println!(
        "清单 {}｜共 {} 份场景：{}",
        manifest.display(),
        entries.len(),
        entries
            .iter()
            .map(|entry| format!("{}={}", entry.node, &entry.key[..12]))
            .collect::<Vec<_>>()
            .join(" ")
    );
}

fn compile(file: &SceneFile, root: &Path) -> Result<Compiled, String> {
    let mut generated = Generated2::new();
    let planet = file
        .parts
        .iter()
        .find(|part| part.kind == "planet")
        .ok_or_else(|| "场景里没有 kind planet 的 part：主体（height / mesh / palette / …）全在它身上".to_string())?;
    let clouds = file.parts.iter().find(|part| part.kind == "clouds");
    let atmosphere = file.parts.iter().find(|part| part.kind == "atmosphere");
    for part in &file.parts {
        if !["planet", "clouds", "atmosphere"].contains(&part.kind.as_str()) {
            return Err(format!(
                "part '{}' 的 kind '{}' 不认识；场景编译器认：planet / clouds / atmosphere",
                part.id, part.kind
            ));
        }
    }

    planet.check_keys(&PLANET_KEYS)?;
    let palette_name = planet.text("palette")?;
    let palette = Palette::parse(palette_name).ok_or_else(|| {
        format!(
            "part '{}' 参数 'palette' 是 '{palette_name}'，认不出来；可用：{}",
            planet.id,
            Palette::NAMES.join(" / ")
        )
    })?;
    let radius = planet.number("radius")? as f32;
    let spin = planet.number_or("spin", 0.0);
    let sea_level = planet.number("sea_level")? as f32;
    // `displace` 只对「从场现造球面网格」那条路有意义 —— 那条路随渲染器里的程序化网格
    // 一起没了（网格现在是 `planet` 图的产物）。配方里留着它是因为它本来就是场那一侧的数。
    let _displace = planet.number_or("displace", 0.075);
    let rings = planet.number_or("rings", 0.0);
    let world = orientation(spin);

    // ---- 生成物：色板贴图 + 星空 ----
    let height_member = member_at(planet, "height")?;
    let height = field_of(&height_member, root);
    let (color, glow, audit) = generate::surface_color(&height, palette, sea_level);
    println!("{audit}");
    let color_member = generated.texture("surface_color", color, "texture.palette");
    let glow_member = glow.map(|glow| generated.texture("surface_glow", glow, "texture.palette"));
    let stars_member = generated.texture("stars", generate::stars(STARS_FACE), "texture.stars");

    // ---- 云：壳、覆盖度立方图、形状档 ----
    let (cloud_inner, cloud_outer, cloud_shape, coverage_member) = match clouds {
        Some(clouds) => {
            let shape = cloud_shape_of(clouds)?;
            let inner = radius * clouds.number_or("inner", CLOUD_BASE) as f32;
            let outer = radius * clouds.number_or("outer", CLOUD_TOP) as f32;
            let mask = field_of(&member_at(clouds, "field")?, root);
            let slopes = [
                field_of(&member_at(clouds, "slope_x")?, root),
                field_of(&member_at(clouds, "slope_y")?, root),
                field_of(&member_at(clouds, "slope_z")?, root),
            ];
            let cube = generate::coverage_cube(&mask, [&slopes[0], &slopes[1], &slopes[2]])
                .map_err(|err| format!("覆盖度立方图：{err}"))?;
            let member = generated.texture("cloud_coverage", cube, "texture.coverage");
            (inner, outer, shape, Some(member))
        }
        None => {
            let default = CloudShape::default();
            (
                radius * CLOUD_BASE,
                radius * CLOUD_TOP,
                default,
                None,
            )
        }
    };

    // ---- 物体 ----
    let mut objects: Vec<Object> = Vec::new();

    // 行星本体：走自写的 surface 材质。云影那几个参数与云材质同一口径。
    let surface_shader = shader_member(planet)?;
    let cloud_shadow = if coverage_member.is_some() {
        planet.number_or("cloud_shadow", 0.0)
    } else {
        0.0
    };
    let has_glow = glow_member.is_some();
    let mut surface = Material::new(surface_shader).with_params(BTreeMap::from([
        ("orientation".to_string(), Value::Quad(world)),
        (
            "emissive".to_string(),
            Value::Quad(if has_glow {
                [3.0, 3.0, 3.0, 1.0]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }),
        ),
        ("inner".to_string(), Value::Num(f64::from(cloud_inner))),
        ("outer".to_string(), Value::Num(f64::from(cloud_outer))),
        (
            "coverage".to_string(),
            Value::Num(f64::from(cloud_shape.coverage)),
        ),
        (
            "shadow".to_string(),
            Value::Num(f64::from(cloud_shadow)),
        ),
        (
            "height".to_string(),
            Value::Num(planet.number_or("shadow_height", CLOUD_SHADOW_HEIGHT) as f64),
        ),
        ("gain".to_string(), Value::Num(f64::from(CLOUD_SHADOW_GAIN))),
    ]));
    surface = surface.with_texture(
        "albedo",
        TextureRef::new(1, color_member.clone(), Sampler::repeat()),
    );
    if let Some(glow) = &glow_member {
        surface = surface.with_texture("glow", TextureRef::new(3, glow.clone(), Sampler::repeat()));
    }
    if let Some(coverage) = &coverage_member {
        surface = surface.with_texture(
            "coverage",
            TextureRef::new(5, coverage.clone(), Sampler::clamped()),
        );
    }
    objects.push(Object {
        id: "planet".to_string(),
        geometry: Geometry::mesh(member_at(planet, "mesh")?),
        material: surface,
        transform: Transform::rotated(world),
        cast_shadow: true,
    });

    // ⚠ 物体的次序**就是文档的次序**，而透明物体（大气、云）都摆在原点 ⇒ 深度排序分不出
    // 先后，谁先画谁后画完全由这张表决定。次序照迁移前的渲染器摆：行星 → 大气 → 云。
    // 反过来的话，云壳薄的像素会差 1~2 个色阶（实测 33/614400 个像素、最大差 2）。
    if let Some(atmosphere) = atmosphere {
        atmosphere.check_keys(&ATMOSPHERE_KEYS)?;
        let inner = atmosphere.number_or("inner", radius);
        if (inner - radius).abs() > 1e-3 {
            return Err(format!(
                "大气 part 的 'inner' 是 {inner}，而行星半径是 {radius}：大气壳的内半径就是行星半径，\
                 这两个对不上"
            ));
        }
        let outer_factor = atmosphere.number("outer")? as f32;
        let outer = radius * outer_factor;
        let density = atmosphere.number("density")? as f32 * planet.number_or("atmo", 1.0);
        let tint = atmosphere.triple("tint")?;
        let material = Material::new(shader_member(atmosphere)?).with_params(BTreeMap::from(
            [
                ("inner".to_string(), Value::Num(f64::from(radius))),
                ("outer".to_string(), Value::Num(f64::from(outer))),
                ("density".to_string(), Value::Num(f64::from(density))),
                (
                    "softness".to_string(),
                    Value::Num(atmosphere.number("softness")?),
                ),
                (
                    "tint".to_string(),
                    Value::Quad([tint[0], tint[1], tint[2], 1.0]),
                ),
            ],
        ));
        let mut material = material;
        material.alpha = AlphaMode::Add;
        objects.push(Object {
            id: "atmosphere".to_string(),
            geometry: Geometry::primitive(
                "icosphere",
                BTreeMap::from([
                    ("radius".to_string(), Value::Num(f64::from(outer))),
                    ("subdivisions".to_string(), Value::Num(64.0)),
                ]),
            ),
            material,
            // 大气壳是**球对称**的：迁移前它挂在根上（不带倾斜），这里也就给单位变换。
            transform: Transform::default(),
            cast_shadow: false,
        });
    }

    if let Some(clouds) = clouds {
        let shape = cloud_shape_of(clouds)?;
        let mut material = Material::new(shader_member(clouds)?)
            .with_params(cloud_params(clouds, shape, cloud_inner, cloud_outer, world)?);
        material.alpha = AlphaMode::Premultiplied;
        // 云壳压一点深度：它整颗球都盖在行星上。
        material.depth_bias = -1.0;
        material = material.with_texture(
            "coverage",
            TextureRef::new(
                5,
                coverage_member
                    .clone()
                    .ok_or_else(|| "云 part 没有覆盖度成员".to_string())?,
                Sampler::clamped(),
            ),
        );
        // 几何：有代理 mesh 就用它（空区域在光栅阶段就被剔除），没有就是一个细分球壳。
        let geometry = match optional_member(clouds, "proxy") {
            Some(proxy) => Geometry::mesh(proxy),
            None => Geometry::primitive(
                "icosphere",
                BTreeMap::from([
                    ("radius".to_string(), Value::Num(f64::from(cloud_outer))),
                    ("subdivisions".to_string(), Value::Num(64.0)),
                ]),
            ),
        };
        objects.push(Object {
            id: "clouds".to_string(),
            geometry,
            material,
            transform: Transform::rotated(world),
            // 云壳**不投**阴影：它是一整颗球，进 shadow map 就是一颗球形硬影。
            cast_shadow: false,
        });
    }

    if rings > 0.0 {
        let inner = radius * 1.30;
        let outer = radius * rings.max(1.45);
        let mesh = generated.mesh(
            "ring_mesh",
            &generate::ring_mesh(inner, outer, RING_SEGMENTS),
            "mesh.ring",
        );
        let band = generated.texture(
            "ring_band",
            generate::ring_band(RING_BAND.0, RING_BAND.1),
            "texture.ring",
        );
        let mut material = Material::new(ring_shader()?).with_params(BTreeMap::from([(
            "tint".to_string(),
            Value::Quad([1.0, 1.0, 1.0, 1.0]),
        )]));
        material.alpha = AlphaMode::Blend;
        material.cull = CullMode::None;
        material = material.with_texture(
            "color",
            TextureRef::new(1, band, Sampler::clamped()),
        );
        objects.push(Object {
            id: "rings".to_string(),
            geometry: Geometry::mesh(mesh),
            material,
            transform: Transform::rotated(world),
            cast_shadow: true,
        });
    }

    // ---- 灯：那盏太阳（点光源，§60）----
    let position = match planet.params.get("light_position") {
        Some(_) => planet.triple("light_position")?,
        None => [-4.2, 1.15, 2.35],
    };
    let color = match planet.params.get("light_color") {
        Some(_) => planet.triple("light_color")?,
        None => [1.0, 1.0, 1.0],
    };
    let intensity = planet.number_or("light_intensity", 7.6e5);
    // 射程 = `|position| × 2.5`（迁移前是渲染器里的 `SUN_RANGE_FACTOR`）。
    let reach = planet.number_or("light_range", length(position) * SUN_RANGE_FACTOR);
    let mut sun = Light::point("sun", position, color, intensity as f32).with_range(reach as f32);
    sun.shadows = planet.number_or("shadows", 0.0) > 0.5;

    // ---- 相机：局部方向 → 世界系 ----
    let cameras = match file.cameras.as_deref() {
        Some("review") | None => px_ops::cameras::review(),
        Some(other) => return Err(format!("不认识的相机表 '{other}'（现在只有 review）")),
    }
    .into_iter()
    .map(|camera| {
        let world_direction = rotate(quat_x(SYSTEM_TILT), camera.direction);
        Camera::new(world_direction, camera.distance, &camera.tag)
    })
    .collect();

    generated.finish();

    let document = SceneSpec {
        schema: px_protocol::SCENE_SCHEMA,
        name: file.name.clone(),
        environment: Environment {
            ambient: file.ambient,
            skybox: Some(stars_member),
            skybox_brightness: SKYBOX_BRIGHTNESS,
        },
        cameras,
        expects: if clouds.is_some() {
            vec!["clouds".to_string()]
        } else {
            Vec::new()
        },
        resources: Vec::new(),
        passes: Vec::new(),
        lights: vec![sun],
        objects,
    };
    document.check()?;
    Ok(Compiled { document })
}

/// 环的自写材质（原来是 Bevy 的 `StandardMaterial { unlit: true, blend, cull: none }`）。
/// 和 `shaders` 图里那三份同规矩：**include 闭包进键**（§17.1、§52.3）。
fn ring_shader() -> Result<Member, String> {
    let text = std::fs::read_to_string("art/shaders/ring.wgsl")
        .map_err(|err| format!("读不了 art/shaders/ring.wgsl：{err}"))?;
    let modules = px_shader::workspace_modules(&px_ops::workspace_root())?;
    let closure = px_shader::closure(&text, &modules);
    let (key, _path, _bytes) = px_ops::write_shader("ring", &text, &closure)
        .map_err(|err| format!("写环 shader 失败：{err}"))?;
    println!("环 shader {}｜{}", px_ops::hex_short(&key), closure.summary());
    Ok(Member::new("shaders", "ring", &px_ops::hex(&key)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 朝向的合成顺序：`rot_x(TILT) × rot_y(spin)` —— 与迁移前渲染器那个
    /// 「父实体带倾斜 + 子实体带自转」合成出来的世界旋转是同一个四元数。
    #[test]
    fn the_world_orientation_is_tilt_then_spin() {
        let half = std::f32::consts::FRAC_PI_2;
        let composed = quat_mul(quat_x(half), quat_y(half));
        let rotated = rotate(composed, [0.0, 0.0, 1.0]);
        // 先绕 Y 转 90°（Z → X），再绕 X 转 90°（X 不动）⇒ 结果是 X 轴。
        assert!(
            (rotated[0] - 1.0).abs() < 1e-5
                && rotated[1].abs() < 1e-5
                && rotated[2].abs() < 1e-5,
            "合成朝向不对：{rotated:?}"
        );
        assert!((rotate(quat_x(0.0), [0.0, 1.0, 0.0])[1] - 1.0).abs() < 1e-6);
    }

    /// 消融档的名字是**内容**，码是 shader 定的；两边对不上就等于切了个不存在的档。
    #[test]
    fn the_ablation_names_map_to_the_codes_the_shader_knows() {
        assert_eq!(ablate_code("none").unwrap(), 0);
        assert_eq!(ablate_code("surface").unwrap(), 5);
        assert_eq!(ablate_code("normals").unwrap(), 6);
        assert!(ablate_code("surfaces").is_err());
    }

    /// 打错的参数名必须当场报错：配方里看着完全正常，结果是悄悄少一个旋钮。
    #[test]
    fn an_unknown_parameter_is_rejected() {
        let part = PartFile {
            id: "planet".to_string(),
            kind: "planet".to_string(),
            shader: "surface".to_string(),
            graph: Some("planet".to_string()),
            members: BTreeMap::new(),
            params: BTreeMap::from([("paltte".to_string(), toml::Value::String("rocky".into()))]),
        };
        let err = part.check_keys(&PLANET_KEYS).unwrap_err();
        assert!(err.contains("paltte"), "报错要点名：{err}");
    }
}
