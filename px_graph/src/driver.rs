//! 驱动：读参数、算键、查 CAS、落盘、记清单。
//!
//! ⚠ 这里是**唯一**知道「一个节点怎么走完一趟」的地方。它与 `cook` 之间只有两样东西：
//! `Cache` 那几个方法（`px_graph_schema` 里的接缝）与序列化载荷（`PayloadBundle`）。
//!
//! ⚠ **没有全局单例**：`begin` 交回一个 `Graph` 句柄，图脚本拿着它跑 `cook`、最后 `finish`。
//!   从前那份状态住在一个 `OnceLock` 里，于是**第二次 `begin` 被静默忽略** —— 同一个进程里
//!   第二张图会读到第一张的画布 / 参数 / 相机，而没有任何一行代码看得见这件事。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use px_graph_schema::{
    Cache, GraphSpec, Grid, Key, ManifestEntry, PayloadBundle, Report, fnv1a, hex, hex_short,
};
use px_protocol::art::Camera;
use px_protocol::stream::{self, Frame};

/// **一张正在跑的图**：画布、参数目录、清单、以及"这一趟是不是全量重算"。
///
/// 它就是 `Cache` 的实现 —— 图脚本拿 `begin` 的返回值直接喂给 `cook`。
pub struct Graph {
    spec: GraphSpec,
    /// 画布：尺寸 + 投影。**只在这里折算一次** —— 键里那份与算子手里那份因此必然相同。
    grid: Grid,
    param_dir: PathBuf,
    cache_root: PathBuf,
    fresh: bool,
    /// 这一趟图的成员与它们的键。⚠ **命中也要记**：下游（`scene` 按节点名查键）靠它。
    manifest: Mutex<Vec<ManifestEntry>>,
    /// 每个节点实际读到的参数（诊断：缺文件是静默用默认值的，设计师看不到自己少写了什么）。
    params_used: Mutex<BTreeMap<String, ParamsUsed>>,
}

impl Graph {
    /// 这个节点的参数**原文**（`art/<图>/<节点>.toml`）。类型化的解析在领域 schema 里
    /// （`px_*_schema::params::parse`）—— 驱动不认识任何算子的参数类型。
    pub fn params_text(&self, name: &str) -> Option<String> {
        load_params_text(&self.param_dir, name)
    }

    /// 收尾：写参数索引与清单，把这一趟的产物路径与统计打出来。
    ///
    /// ⚠ 审计**不能因为走缓存而哑掉**：这里的每一行读数在"全部命中"那一趟照样打。
    pub fn finish(&self) {
        let manifest = self.manifest.lock().expect("清单锁坏了");
        let hits = manifest.iter().filter(|entry| entry.hit).count();
        let cooked = manifest.len() - hits;
        let millis: u64 = manifest.iter().map(|entry| entry.millis).sum();

        // 参数索引：每个节点实际生效的参数值 + 字段名。
        // ⚠ 缺文件是静默用默认值的 ⇒ 这是设计师唯一能看见"我少写了什么/写错了什么字段"的地方。
        let used = self.params_used.lock().expect("参数表锁坏了");
        let params_path = self.cache_root.join(&self.spec.name).join("params.json");
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

        let path = self.cache_root.join(&self.spec.name).join("manifest.json");
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
                let key = key_of_hex(&entry.key);
                format!(
                    "{} -> {}",
                    entry.node,
                    artifact_path(&self.cache_root, &key).display()
                )
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
}

impl Cache for Graph {
    fn grid(&self) -> Grid {
        self.grid
    }

    fn cameras(&self) -> &[Camera] {
        &self.spec.cameras
    }

    fn params_text(&self, name: &str) -> Option<String> {
        load_params_text(&self.param_dir, name)
    }

    fn record_params(&self, node: &str, op: &str, params_json: &str, from_file: bool) {
        let value = serde_json::from_str(params_json).unwrap_or(serde_json::Value::Null);
        self.params_used
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

    fn fetch(&self, key: Key) -> Option<PayloadBundle> {
        if self.fresh {
            return None;
        }
        let path = artifact_path(&self.cache_root, &key);
        let bytes = std::fs::read(&path).ok()?;
        // ⚠ 解不开就**当作未命中重算**，但要说出来 —— 静默吞掉一份坏产物不该无声无息。
        match PayloadBundle::from_bytes(&bytes) {
            Ok(payload) => Some(payload),
            Err(err) => {
                eprintln!(
                    "⚠ CAS 里那份 {} 解不开（{err}）；当作未命中重算",
                    hex_short(&key)
                );
                None
            }
        }
    }

    fn store(&self, report: Report<'_>, payload: &PayloadBundle) -> Result<(), String> {
        // ⚠ `id`（节点名）与相机表**只在这里**补上：算子交出来的载荷是无名、无相机的。
        //   （从前算子先编成一份"占位字节"、驱动再拆开重编 —— 那条路上漏补名字不会报错，
        //   只会写出"id 空、相机空"的产物。现在那种形状不可表达：这里收的就是载荷本身。）
        let cameras: &[Camera] = if report.with_cameras {
            &self.spec.cameras
        } else {
            &[]
        };
        let bytes = payload
            .to_bytes(report.node, cameras)
            .map_err(|err| format!("包 {} 的产物失败：{err}", report.node))?;
        let path = artifact_path(&self.cache_root, &report.key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("建目录 {} 失败：{err}", parent.display()))?;
        }
        std::fs::write(&path, &bytes)
            .map_err(|err| format!("写产物 {} 失败：{err}", path.display()))?;

        // ⚠ 那一行读数由**域自己**给（类型化的值在它手里）：驱动不必解字节去猜域，
        //   清单里也不再是"三个各表示四种含义的数"。
        let detail = report.detail.clone();
        // ⚠ **命中也要记**：清单是"这一趟图的成员与它们的键"，下游（`scene` 按节点名查键）
        //   靠它。只在重算时记的话，一趟全命中的运行会写出**空清单**，下游当场断
        //   （实测：`scene` 报"图 'planet' 的清单是空的"）。
        self.manifest
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
                detail,
            });
        println!(
            "{} {:<12} {:<16} @{}  {}  {:>5} ms  {:>9} B  {}",
            if report.hit { "命中" } else { "重算" },
            report.node,
            report.op,
            interface_tag(report.interface),
            hex_short(&report.key),
            report.millis,
            bytes.len(),
            report.detail,
        );
        Ok(())
    }
}

/// 接口哈希的低 32 位：清单里那个只用于显示/对账的 `op_version` 字段。
///
/// ⚠ 真正进键的是接口哈希的完整 64 位；这里只是把它塞进清单那个形状里。
fn interface_version(interface: u64) -> u32 {
    (interface & 0xffff_ffff) as u32
}

/// 接口哈希的前 8 位十六进制：读数里那个 `@…` 就用它（比十进制好认）。
fn interface_tag(interface: u64) -> String {
    format!("{:016x}", interface)[..8].to_string()
}

/// 清单里那一格是十六进制文本（给人看的），要拿它去 CAS 取文件时再换回来。
fn key_of_hex(text: &str) -> Key {
    let mut key = [0_u8; 32];
    let bytes = text.as_bytes();
    for index in 0..32 {
        let at = index * 2;
        if at + 2 > bytes.len() {
            break;
        }
        key[index] = u8::from_str_radix(&text[at..at + 2], 16).unwrap_or_default();
    }
    key
}

/// 一个节点用的参数：算子、是不是来自文件、规范 JSON（**键的名字就在里面**）。
#[derive(Clone, serde::Serialize)]
struct ParamsUsed {
    op: String,
    from_file: bool,
    params: serde_json::Value,
}

/// 工作区根目录（`Cargo.toml` 的上一级）。纯函数：与哪张图无关。
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// CAS 的根（`target/pcg/`）。纯函数：与哪张图无关。
pub fn cache_root() -> PathBuf {
    workspace_root().join("target").join("pcg")
}

/// **开始一张图**：把画布、参数目录、清单都挂在一个 `Graph` 句柄上交回去。
///
/// ⚠ 它交回句柄而不是往全局塞 —— 同**一个进程里可以同时跑两张图**（测试、
///   "一把跑全部图"那种入口），彼此不会串。
pub fn begin(spec: GraphSpec) -> Graph {
    let root = workspace_root();
    let param_dir = root.join("art").join(&spec.name);
    let cache_root = root.join("target").join("pcg");
    let fresh = std::env::var("PX_PCG_FRESH")
        .map(|value| value != "0")
        .unwrap_or(false);
    let cached = cache_root.join(&spec.name).join("manifest.json").is_file() as usize;

    let graph = Graph {
        grid: Grid {
            width: spec.width,
            height: spec.height,
            projection: spec.projection,
        },
        param_dir,
        cache_root,
        fresh,
        manifest: Mutex::new(Vec::new()),
        params_used: Mutex::new(BTreeMap::new()),
        spec,
    };
    std::fs::create_dir_all(&graph.cache_root).ok();

    println!(
        "图 {}｜画布 {}×{}｜参数 {}{}｜缓存 {} 条{}",
        graph.spec.name,
        graph.spec.width,
        graph.spec.height,
        graph.param_dir.display(),
        if graph.param_dir.is_dir() {
            ""
        } else {
            "（不存在，全部用默认值）"
        },
        cached,
        if graph.fresh {
            "｜PX_PCG_FRESH=1：本次全部重算"
        } else {
            ""
        },
    );
    graph
}

fn load_params_text(param_dir: &Path, name: &str) -> Option<String> {
    let path = param_dir.join(format!("{name}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(_) => None,
    }
}

pub fn artifact_path_of(key: &Key) -> PathBuf {
    artifact_path(&cache_root(), key)
}

fn artifact_path(cache_root: &Path, key: &Key) -> PathBuf {
    px_protocol::scene::cas_path(cache_root, &hex(key)).unwrap_or_else(|_| cache_root.join("ab"))
}

/// 按**图名**读那份图的清单（`target/pcg/<图>/manifest.json`）。
/// 场景产物要按节点名取成员键，走的就是这条：名字给人用，键给 CAS 用。
pub fn graph_manifest(graph: &str) -> Result<Vec<ManifestEntry>, String> {
    let path = cache_root().join(graph).join("manifest.json");
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

pub fn write_graph_manifest(graph: &str, entries: &[ManifestEntry]) -> Result<PathBuf, String> {
    let path = cache_root().join(graph).join("manifest.json");
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
    let path = artifact_path(&cache_root(), &key);
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
