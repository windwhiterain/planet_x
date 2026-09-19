//! 驱动：读参数、算键、查 CAS、叫算子、写产物、记清单。
//!
//! ⚠ 这里是**唯一**知道「一个节点怎么走完一趟」的地方。它与算子之间只有两样东西：
//! 描述符表（`px_graph_schema::OpLibrary`）与序列化载荷（`PayloadBundle`）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use px_graph_schema::{
    GraphSpec, Grid, Key, ManifestEntry, PayloadBundle,
    fnv1a, hex, hex_short,
};
use px_protocol::art::Camera;
use px_protocol::stream::{self, Frame};

use px_field_schema::field::{Field, Stats};
use px_field_schema::payload as field_payload;
use px_mesh_schema::MeshData;
use px_mesh_schema::payload as mesh_payload;
use px_volume_schema::payload as volume_payload;
use px_volume_schema::VolumeData;

/// **缓存机制**：图脚本那边的类型化门面（`px_cook`）只认这一个接口。
///
/// ⚠ 它**不认识任何算子** —— 报上下文、读参数原文、查/写 CAS、记读数，就这四件事。
/// 于是「类型化的算子契约」与「缓存的实现」各自独立：前者在 `px_cook`，
/// 后者在这里，两边都不需要知道对方的算子长什么样。
pub trait Cache {
    /// **画布 = 算子拿到的那个 `Grid`**（尺寸 + 投影）。
    ///
    /// ⚠ 只有一个出口：键里那一份与算子 `render` 手里那一份必须是同一个值。
    ///   从前它们是两处（`canvas()` + `projection()` 给键、`cook` 的参数给算子）——
    ///   那种"双份真相"是错的。
    fn grid(&self) -> Grid;
    fn cameras(&self) -> &[Camera];
    /// `art/<图>/<name>.toml` 的原文；`None` = 文件不存在 ⇒ 用算子默认值。
    fn params_text(&self, name: &str) -> Option<String>;
    /// 记一条"这个节点读了哪些参数"（诊断用：写进 `<图>/params.json`）。
    ///
    /// ⚠ 缺文件是**静默用默认值**的（老口径，不动）—— 于是设计师看不到自己少写了什么。
    /// 这一条记录就是那个缺口：跑完能拿到"每个节点实际生效的参数值 + 它的字段名"。
    fn record_params(&self, node: &str, op: &str, params_json: &str, from_file: bool);
    /// CAS 里那份字节。`PX_PCG_FRESH=1` 时一律 `None`（本次全部重算）。
    fn fetch(&self, key: Key) -> Option<Vec<u8>>;
    /// 键不在盘上：把产物写进 CAS、记索引与清单，并把读数打出来。
    fn store(&self, report: Report<'_>, bytes: &[u8]) -> Result<(), String>;
}

/// 一次 cook 的读数 —— 与 `node()` 打出来的那几行同一档。
pub struct Report<'a> {
    pub node: &'a str,
    pub op: &'static str,
    /// **接口形状哈希**（完整 64 位）—— 它取代了手写的 `version`。
    pub interface: u64,
    pub key: Key,
    pub hit: bool,
    pub millis: u64,
    pub bytes: usize,
    /// 体积那一档不掺评审相机。
    pub with_cameras: bool,
    /// 清单里那三个读数 `(min, max, mean)` —— 由**算子那一侧**算（类型化的值在它手里，
    /// 不必让驱动解字节去猜域）。
    pub stats: (f64, f64, f64),
}

/// 缓存机制的句柄。`begin(GraphSpec)` 之后才有，用 `driver()` 取。
pub struct Driver;


impl Cache for Driver {

    fn grid(&self) -> Grid {
        context().grid
    }

    fn cameras(&self) -> &[Camera] {
        &context().spec.cameras
    }

    fn params_text(&self, name: &str) -> Option<String> {
        load_params_text(&context().param_dir, name)
    }

    fn record_params(&self, node: &str, op: &str, params_json: &str, from_file: bool) {
        let value = serde_json::from_str(params_json).unwrap_or(serde_json::Value::Null);
        context()
            .params_used
            .lock()
            .expect("参数表锁坏了")
            .insert(
                node.to_string(),
                ParamsUsed {
                    op: op.to_string(),
                    from_file,
                    params: value,
                },
            );
    }

    fn fetch(&self, key: Key) -> Option<Vec<u8>> {
        let context = context();
        if context.fresh {
            return None;
        }
        std::fs::read(artifact_path(&context.cache_root, &key)).ok()
    }

    fn store(&self, report: Report<'_>, bytes: &[u8]) -> Result<(), String> {
        let context = context();
        // ⚠ 算子回的是**无名、无相机**的占位载荷（"算子只回一个载荷，驱动把它补成产物"）。
        // 所以这里必须把节点名与相机表补回去，漏了就会写出"id 空、相机空"的产物。
        let cameras: &[Camera] = if report.with_cameras {
            &context.spec.cameras
        } else {
            &[]
        };
        let bundle = PayloadBundle::from_bytes(bytes)
            .unwrap_or_else(|err| panic!("{}（{}）回的载荷解不开：{err}", report.node, report.op));
        let bytes = bundle
            .to_bytes(report.node, cameras)
            .unwrap_or_else(|err| panic!("包 {} 的产物失败：{err}", report.node));
        let path = artifact_path(&context.cache_root, &report.key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("建目录 {} 失败：{err}", parent.display()))?;
        }
        std::fs::write(&path, &bytes)
            .map_err(|err| format!("写产物 {} 失败：{err}", path.display()))?;

        // ⚠ 清单那三个读数由**算子那一侧**算（类型化的值在它手里，不必让驱动解字节猜域）。
        let (min, max, mean) = report.stats;
        // ⚠ **命中也要记**：清单是"这一趟图的成员与它们的键"，下游（`scene` 按节点名查键）
        //   靠它。只在重算时记的话，一趟全命中的运行会写出**空清单**，下游当场断
        //   （实测：`scene` 报"图 'planet' 的清单是空的"）。
        context
            .manifest
            .lock()
            .expect("清单锁坏了")
            .push(ManifestEntry {
                node: report.node.to_string(),
                op: report.op.to_string(),
                op_version: interface_version(report.interface),
                key: hex(&report.key),
                hit: report.hit,
                millis: report.millis,
                bytes: bytes.len() as u64,
                min: min as f32,
                max: max as f32,
                mean: mean as f32,
            });
        println!(
            "{} {:<12} {:<16} @{}  {}  {:>5} ms  {:>9} B",
            if report.hit { "命中" } else { "重算" },
            report.node,
            report.op,
            interface_tag(report.interface),
            hex_short(&report.key),
            report.millis,
            bytes.len(),
        );
        Ok(())
    }

}


/// 接口哈希的低 32 位：清单里那个只用于显示/对账的 `op_version` 字段。
///
/// ⚠ 真正进键的是接口哈希的十六进制文本（64 位）；这里只是把它塞进老字段的形状里。
fn interface_version(interface: u64) -> u32 {
    (interface & 0xffff_ffff) as u32
}

/// 接口哈希的前 8 位十六进制：读数里那个 `@…` 就用它（比十进制好认）。
fn interface_tag(interface: u64) -> String {
    format!("{:016x}", interface)[..8].to_string()
}


/// 取缓存机制的句柄（`begin` 之后才有效）。
pub fn driver() -> Driver {
    Driver
}







struct Context {
    spec: GraphSpec,
    /// 画布：尺寸 + 投影。**只在这里折算一次** —— 键里那份与算子手里那份因此必然相同。
    grid: Grid,
    param_dir: PathBuf,
    cache_root: PathBuf,
    fresh: bool,
    /// 算子库**按需装载**：只写 shader / 只写场景的图（`--bin shaders` / `--bin scene`）
    /// 一个算子都不需要，不该因为找不到 dylib 就起不来。
    manifest: Mutex<Vec<ManifestEntry>>,
    /// 每个节点实际读到的参数（诊断：缺文件是静默用默认值的，设计师看不到自己少写了什么）。
    params_used: Mutex<BTreeMap<String, ParamsUsed>>,
}

/// 一个节点用的参数：算子、是不是来自文件、规范 JSON（**键的名字就在里面**）。
#[derive(Clone, serde::Serialize)]
struct ParamsUsed {
    op: String,
    from_file: bool,
    params: serde_json::Value,
}

impl Context {



}

static CONTEXT: OnceLock<Context> = OnceLock::new();

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 把算子库都装进来。一个都没找到就**当场拒**，并把该跑的命令打出来。
pub fn begin(spec: GraphSpec) {
    let root = workspace_root();
    let param_dir = root.join("art").join(&spec.name);
    let cache_root = root.join("target").join("pcg");
    let fresh = std::env::var("PX_PCG_FRESH")
        .map(|value| value != "0")
        .unwrap_or(false);
    let cached = cache_root.join(&spec.name).join("manifest.json").is_file() as usize;

    let _ = CONTEXT.set(Context {
        grid: Grid {
            width: spec.width,
            height: spec.height,
            projection: spec.projection,
        },
        param_dir: param_dir.clone(),
        cache_root: cache_root.clone(),
        fresh,
        manifest: Mutex::new(Vec::new()),
        params_used: Mutex::new(BTreeMap::new()),
        spec,
    });

    let context = context();
    std::fs::create_dir_all(&context.cache_root).ok();

    println!(
        "图 {}｜画布 {}×{}｜参数 {}{}｜缓存 {} 条{}",
        context.spec.name,
        context.spec.width,
        context.spec.height,
        param_dir.display(),
        if param_dir.is_dir() { "" } else { "（不存在，全部用默认值）" },
        cached,
        if context.fresh { "｜PX_PCG_FRESH=1：本次全部重算" } else { "" },
    );
}

fn context() -> &'static Context {
    CONTEXT.get().expect("先调用 px_graph::begin")
}




pub fn finish() {
    let context = context();
    let manifest = context.manifest.lock().expect("清单锁坏了");
    let hits = manifest.iter().filter(|entry| entry.hit).count();
    let cooked = manifest.len() - hits;
    let millis: u64 = manifest.iter().map(|entry| entry.millis).sum();

    // 参数索引：每个节点实际生效的参数值 + 字段名。
    // ⚠ 缺文件是静默用默认值的 ⇒ 这是设计师唯一能看见"我少写了什么/写错了什么字段"的地方。
    let used = context.params_used.lock().expect("参数表锁坏了");
    let params_path = context
        .cache_root
        .join(&context.spec.name)
        .join("params.json");
    let defaults: Vec<&String> = used
        .iter()
        .filter(|(_, entry)| !entry.from_file)
        .map(|(node, _)| node)
        .collect();
    if !used.is_empty() {
        println!(
            "参数索引：{} 个节点（{} 个走默认值{}）；字段名与生效值见 {}{}",
            used.len(),
            defaults.len(),
            if defaults.is_empty() {
                String::new()
            } else {
                format!(
                    "：{}",
                    defaults
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            },
            params_path.display(),
            if defaults.is_empty() {
                ""
            } else {
                "（⚠ 缺参数文件的都在默认值上）"
            },
        );
    }
    if let Some(parent) = params_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match serde_json::to_string_pretty(&*used) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&params_path, text) {
                eprintln!("写参数索引失败：{err}");
            }
        }
        Err(err) => eprintln!("参数索引无法序列化：{err}"),
    }

    let path = context
        .cache_root
        .join(&context.spec.name)
        .join("manifest.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match serde_json::to_string_pretty(&*manifest) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                eprintln!("写清单失败：{err}");
            }
        }
        Err(err) => eprintln!("清单无法序列化：{err}"),
    }
    let mut paths: Vec<String> = manifest
        .iter()
        .map(|entry| {
            let mut raw = [0_u8; 32];
            for index in 0..32 {
                raw[index] = u8::from_str_radix(&entry.key[index * 2..index * 2 + 2], 16)
                    .unwrap_or_default();
            }
            format!("{} -> {}", entry.node, artifact_path(&context.cache_root, &raw).display())
        })
        .collect();
    paths.sort();
    for line in &paths {
        println!("产物 {line}");
    }

    println!(
        "共 {} 个节点：命中 {hits}、重算 {cooked}，合计 {millis} ms；清单 {}",
        manifest.len(),
        path.display(),
    );

    if cooked == 0 && !manifest.is_empty() {
        println!("本次没有任何节点重算 —— 改参数只影响它自己与下游，上游会命中");
    }
}

pub fn hex_short_of(key: &Key) -> String {
    hex_short(key)
}

/// 节点的参数**原文**（`art/<图>/<节点>.toml`）。类型化的解析在领域的 schema 里
/// （`px_*_schema::params::parse`）—— 驱动不认识任何算子的参数类型。
pub fn params_text(name: &str) -> Option<String> {
    load_params_text(&context().param_dir, name)
}

fn load_params_text(param_dir: &Path, name: &str) -> Option<String> {
    let path = param_dir.join(format!("{name}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(_) => None,
    }
}

pub fn artifact_path_of(key: &Key) -> PathBuf {
    artifact_path(&context().cache_root, key)
}

fn artifact_path(cache_root: &Path, key: &Key) -> PathBuf {
    px_protocol::scene::cas_path(cache_root, &hex(key)).unwrap_or_else(|_| cache_root.join("ab"))
}

/// 按**图名**读那份图的清单（`target/pcg/<图>/manifest.json`）。
/// 场景产物要按节点名取成员键，走的就是这条：名字给人用，键给 CAS 用。
pub fn graph_manifest(graph: &str) -> Result<Vec<ManifestEntry>, String> {
    let path = context()
        .cache_root
        .join(graph)
        .join("manifest.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("图 '{graph}' 的清单读不到（{}）：{err}", path.display()))?;
    let entries: Vec<ManifestEntry> =
        serde_json::from_str(&text).map_err(|err| format!("图 '{graph}' 的清单解不开：{err}"))?;
    if entries.is_empty() {
        return Err(format!("图 '{graph}' 的清单是空的（{}）", path.display()));
    }
    Ok(entries)
}

pub fn manifest_key_of(graph: &str, node: &str) -> Result<String, String> {
    let entries = graph_manifest(graph)?;
    entries
        .iter()
        .find(|entry| entry.node == node)
        .map(|entry| entry.key.clone())
        .ok_or_else(|| {
            let known = entries
                .iter()
                .map(|entry| entry.node.as_str())
                .collect::<Vec<_>>()
                .join(" / ");
            format!("图 '{graph}' 里没有节点 '{node}'；这份清单有的节点：{known}")
        })
}

pub fn cache_root() -> PathBuf {
    context().cache_root.clone()
}

pub fn write_graph_manifest(graph: &str, entries: &[ManifestEntry]) -> Result<PathBuf, String> {
    let path = context().cache_root.join(graph).join("manifest.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let text = serde_json::to_string_pretty(entries).map_err(|err| err.to_string())?;
    std::fs::write(&path, text).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    Ok(path)
}


pub const SHADER_VERSION: u32 = 1;

/// 场景的键 = 场景描述 JSON + 它引用的全部成员键。
/// 成员内容一变（哪怕只是换了一个梯度场的版本），场景键就变 ⇒ 不会认错东西。
pub fn scene_key(spec_json: &str, member_keys: &[String]) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_scene/v1");
    hasher.update(&px_protocol::SCENE_SCHEMA.to_le_bytes());
    hasher.update(spec_json.as_bytes());
    for key in member_keys {
        hasher.update(key.as_bytes());
    }
    *hasher.finalize().as_bytes()
}

/// Shader 和场、网格一样是内容寻址的：键 = WGSL 的字节 ‖ 它的 **include 闭包**指纹（§17.1、§52.3）。
/// 改一个字、或者改它 `#import` 到的任一模块，都换一个键 ⇒ 不会出现「同一个键、不同内容」。
/// 闭包由 `px_shader` 算（与运行期 naga_oil 的模块解析同一份规则）：外部符号（`bevy_pbr::…`）
/// 只按**名字**进指纹，它们的实现归 `SHADER_VERSION` 手动那一档管（§19.1）。
///
/// ⚠ **改反射规则（`px_shader::reflect` / `px_protocol::material` 那张表）也要升
/// `SHADER_VERSION`**：schema descriptor 是随产物一起烘的，规则变了而键没变，
/// 盘上就会出现「同一个键、两份契约」—— 装载时的 `schema_check` 拦得住，但键该先换。
pub fn shader_key(text: &str, closure: &px_shader::Closure) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_shader/v2");
    hasher.update(&SHADER_VERSION.to_le_bytes());
    hasher.update(&closure.fingerprint().to_le_bytes());
    hasher.update(text.as_bytes());
    *hasher.finalize().as_bytes()
}

/// 把一份 WGSL 写进 CAS：清单帧（`kind = Shader`）+ **两个** U8 blob
/// （`[0]` WGSL、`[1]` schema descriptor 的规范 JSON）。
///
/// 顺带把闭包指纹记进清单参数：渲染器装载时拿它跟**盘上现在的**闭包对账，
/// 对不上就当场拒（§52.3）—— 键拦住的是"新烘的认错旧产物"，这一条拦的是
/// "改了 include 但没重烘"：那时候场景指的还是老产物，而画出来的东西已经换了一版。
///
/// descriptor 同理（§74.3 的契约收口）：它是**装载时对账**用的那一半 ——
/// 渲染器照着同一份规则再反射一次，对不上就是"这份产物是拿另一版反射规则烘的"。
/// ⚠ 它**不参与键**（键 = WGSL 字节 ‖ 闭包指纹）⇒ 加这一条不换任何产物键。
pub fn write_shader(
    id: &str,
    text: &str,
    closure: &px_shader::Closure,
    modules: &px_shader::ModuleTable,
) -> Result<(Key, PathBuf, u64), String> {
    let key = shader_key(text, closure);
    let path = artifact_path(&context().cache_root, &key);
    let descriptor = shader_schema(id, text, modules)?;
    let wgsl_blob = px_protocol::Blob::new(
        px_protocol::BlobHeader {
            dtype: px_protocol::DType::U8,
            shape: vec![text.len() as u32],
        },
        text.as_bytes().to_vec(),
    )
    .map_err(|err| err.to_string())?;
    let schema = descriptor.as_bytes().to_vec();
    let schema_blob = px_protocol::Blob::new(
        px_protocol::BlobHeader {
            dtype: px_protocol::DType::U8,
            shape: vec![schema.len() as u32],
        },
        schema,
    )
    .map_err(|err| err.to_string())?;
    let bundle = px_protocol::art::ArtBundle {
        assets: vec![px_protocol::art::AssetManifest {
            id: id.to_string(),
            kind: px_protocol::AssetKind::Shader,
            params: shader_params(text, closure, &descriptor),
            blobs: vec![wgsl_blob.header.clone(), schema_blob.header.clone()],
            fingerprint: fnv1a(text),
            cameras: Vec::new(),
        }],
    };
    let frames = vec![
        Frame::Art(bundle),
        Frame::Blob(wgsl_blob),
        Frame::Blob(schema_blob),
    ];
    let mut out = Vec::new();
    stream::write_stream(&mut out, &frames).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, &out).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    Ok((key, path, out.len() as u64))
}

/// 烘的时候反射一次：**契约由这份文本自己说了算**（表与反射都住在 `px_shader`，
/// 运行期用的是同一份实现）。反射不出来 ⇒ 这次烘就失败，不许写出一个没有契约的产物。
fn shader_schema(id: &str, text: &str, modules: &px_shader::ModuleTable) -> Result<String, String> {
    let mut seen = Vec::new();
    // 烘图侧是 **Bevy 那一侧**：它烘出来的契约要给运行期那个宿主用，
    // 所以桩表必须是 Bevy 那张（`bevy_stub`），不能是裸 wgpu 宿主那张。
    let assembled =
        px_shader::assemble::render_source(text, modules, px_shader::assemble::bevy_stub, &mut seen);
    px_shader::reflect::reflect_assembled(&assembled, id)?.to_json()
}

/// 一份 shader 产物的清单参数。除了载荷形状，还记**闭包指纹**与它的规模：
/// 指纹给渲染器对账用（对不上 = 这份产物是拿另一版 include 烘的），规模给人/报告看。
/// schema 的规模也记在这里：`read_manifest` 只读前缀就能看出"这份产物有没有契约"。
fn shader_params(
    text: &str,
    closure: &px_shader::Closure,
    descriptor: &str,
) -> BTreeMap<String, f64> {
    let fingerprint = closure.fingerprint();
    let mut params = BTreeMap::from([
        ("wgsl_bytes".to_string(), text.len() as f64),
        ("shader_version".to_string(), f64::from(SHADER_VERSION)),
        (
            "closure_modules".to_string(),
            closure.modules.len() as f64,
        ),
        (
            "closure_externals".to_string(),
            closure.externals.len() as f64,
        ),
        ("schema_bytes".to_string(), descriptor.len() as f64),
    ]);
    for (name, value) in px_shader::closure_params(fingerprint) {
        params.insert(name, value);
    }
    params
}

// 读一份 Shader 产物由渲染侧负责（`px_protocol::art::read_shader`）——
// 渲染器不许依赖 PCG 这一侧（§10.1）。
