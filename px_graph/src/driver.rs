//! 驱动：读参数、算键、查 CAS、落盘、记清单。
//!
//! ⚠ 这里是**唯一**知道「一个节点怎么走完一趟」的地方。它与 `cached` 之间只有两样东西：
//! `Cache` 那几个方法（`px_graph_schema` 里的接缝）与序列化载荷（`PayloadBundle`）。
//!
//! ## 参数目录可以被**换指**（`PX_ART` / `--store`）
//!
//! 参数默认住在 `art/<图>/`，而「换一个目录读同一批节点」这件事有一个真实用户：
//! **预览窗口里那块调参面板**（`px_render/src/edit.rs`）。它要的是"在盘上改一个数、
//! 重烘、画面变"，而**不许**把工作树里的 `art/` 改脏（那是作品的源码，得留下"我到底改没改"
//! 这个判断）。⇒ 面板把 `art/<图>/` 复制到 `target/pcg/edit/<图>/`，在副本上编辑，
//! 再让烘图的那两个进程从副本读：
//!
//! ```text
//! px_render --view …                       # 面板：写 target/pcg/edit/<图>/<节点>.toml
//!   └─ px run nebula --store target/pcg/edit   # 烘体积与天空
//!   └─ px run scene  nebula --store target/pcg/edit   # 烘场景文档
//! ```
//!
//! ⚠⚠ **它不进键，也不许进键**：「这一趟读的是哪个目录」是一句**归档的话**
//!   （产物落到哪一格 CAS、清单写在哪），不是"这个节点算什么"。把它塞进键就等于
//!   让同一份内容在两个目录下算出两个键 —— 那是本仓最不该有的那种重复。
//!   这条口径与 CAS 的键不含环境变量是**同一条**（`AGENTS.md` 里量 `PX_SKIP_OFF` 那条）。
//! ⚠ 参数**文件在不在**照旧要进键（`node_params` 读到的是 `Some` 还是 `None`
//!   会改变 `canonical_params` 的结果）：键跟的是**字节**，不是**路径**。
//!
//! ⚠ **没有全局单例**：`begin` 交回一个 `Graph` 句柄，图脚本拿着它跑 `cached`、最后 `finish`。
//!   从前那份状态住在一个 `OnceLock` 里，于是**第二次 `begin` 被静默忽略** —— 同一个进程里
//!   第二张图会读到第一张的画布 / 参数 / 相机，而没有任何一行代码看得见这件事。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use px_graph_schema::{
    Cache, GraphSpec, Key, ManifestEntry, PayloadBundle, Report, fnv1a, hex, hex_short,
};
use px_protocol::stream::{self, Frame};

/// **一张正在跑的图**：参数目录、清单、以及"这一趟是不是全量重算"。
///
/// 它就是 `Cache` 的实现 —— 图脚本拿 `begin` 的返回值直接喂给 `cached`。
pub struct Graph {
    spec: GraphSpec,
    param_dir: PathBuf,
    cache_root: PathBuf,
    fresh: bool,
    /// 这一趟图的成员与它们的键。⚠ **命中也要记**：下游（`scene` 按节点名查键）靠它。
    manifest: Mutex<Vec<ManifestEntry>>,
    /// 每个节点实际读到的参数（诊断：缺文件是静默用默认值的，设计师看不到自己少写了什么）。
    params_used: Mutex<BTreeMap<String, ParamsUsed>>,
}

impl Graph {
    /// 这个节点的参数**原文**（`art/<图>/<节点>.toml`）。类型化的解析在门那一侧
    /// （`px_cook::node_params`）—— 驱动不认识任何算子的参数类型。
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
        // ⚠ 参数今天有两条来源（`node_params` 从 `art/<图>/<节点>.toml` 读的打底值，
        //   与图脚本自己算出来的值）⇒ 这一行说清"哪些节点连参数文件都没打底"。
        let used = self.params_used.lock().expect("参数表锁坏了");
        let params_path = self.cache_root.join(&self.spec.name).join("params.json");
        let no_file: Vec<&String> = used
            .iter()
            .filter(|(_, entry)| !entry.from_file)
            .map(|(node, _)| node)
            .collect();
        if !used.is_empty() {
            println!(
                "参数索引：{} 个节点（{} 个没有参数文件{}）；字段名与生效值见 {}{}",
                used.len(),
                no_file.len(),
                if no_file.is_empty() {
                    String::new()
                } else {
                    format!(
                        "：{}",
                        no_file
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    )
                },
                params_path.display(),
                if no_file.is_empty() {
                    ""
                } else {
                    "（⚠ 这些节点的参数由图脚本给的值决定，改它们要重编图程序）"
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
    fn params_text(&self, name: &str) -> Option<String> {
        load_params_text(&self.param_dir, name)
    }

    fn record_params(&self, node: &str, op: &str, params_json: &str, from_file: bool) {
        let value = serde_json::from_str(params_json).unwrap_or(serde_json::Value::Null);
        let mut used = self.params_used.lock().expect("参数表锁坏了");
        // ⚠ **两条记录都指同一个节点，要合并、不是覆盖**：
        //   `node_params`（参数是从文件打底的）先来、`cached`（生效值 + 算子）后到
        //   ⇒ `from_file` 只许由 false 变 true，`op` 只许填一次。
        let entry = used.entry(node.to_string()).or_insert_with(|| ParamsUsed {
            op: String::new(),
            from_file: false,
            params: serde_json::Value::Null,
        });
        if !op.is_empty() {
            entry.op = op.to_string();
        }
        entry.from_file |= from_file;
        entry.params = value;
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
        // ⚠ `id`（节点名）**只在这里**补上：算子交出来的载荷是无名的。
        //   （从前算子先编成一份"占位字节"、驱动再拆开重编 —— 那条路上漏补名字不会报错，
        //   只会写出"id 空"的产物。现在那种形状不可表达：这里收的就是载荷本身。）
        let bytes = payload
            .to_bytes(report.node)
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

/// **参数目录的根**：默认 `art/`，可用 `PX_ART` 换指（见模块文档）。
///
/// ⚠ 读的是**本进程的环境变量**，所以钉它的那一句话必须发生在任何 `begin` **之前**：
///   图程序用 [`apply_store_args`] 在 `main` 的第一行就把 `--store` 落成 `PX_ART`。
/// ⚠ 它是**路径**那一半，与键无关：换指不换键（同一批字节在哪儿都是同一批字节）。
pub fn param_root() -> PathBuf {
    match std::env::var("PX_ART") {
        Ok(text) if !text.is_empty() => {
            let path = PathBuf::from(&text);
            if path.is_absolute() {
                path
            } else {
                // 相对路径按**工作区根**解释。⚠ 不按当前目录：图程序是 `px` 起的，
                // 而"px 从哪儿起"不归图管（§104 第 3 条：缺省值也是一处会漂的真相）。
                workspace_root().join(path)
            }
        }
        _ => workspace_root().join("art"),
    }
}

/// **图程序的第一行**：把命令行的 `--store <目录>` 落成 `PX_ART`。
///
/// ```ignore
/// fn main() -> Result<(), Fault> {
///     px_cook::apply_store_args()?;   // ← 必须在 begin / node_params 之前
///     let graph = px_cook::begin(GraphSpec { name: "nebula".to_string() });
///     …
/// ```
///
/// ⚠ 参数目录**不属于节点键**，所以它没有走 `GraphSpec`：进了 `GraphSpec` 就等于
///   宣布"它是一张图的身份的一部分"，而它不是（同一张图可以从任何目录读参数）。
///
/// ⚠ 认两种写法（`--store D` 与 `--store=D`），而且**要把它从命令行上摘掉**：
///   有几个图程序自己按位置读参数（`scene` 的配方名、`passes` 的三个位置参数），
///   不摘的话它们会把 `--store` 与它那个目录当成内容 —— 那不是报错，是**读错东西**
///   （`scene --store X orbit` 会去烘一份叫 `--store` 的配方）。
///   ⇒ 自己还有参数要读的图程序请拿 [`args_without_store`] 的那一份，别回头去读
///   `std::env::args()`。
pub fn apply_store_args() -> Result<(), String> {
    let (store, _) = split_store_args()?;
    let Some(store) = store else {
        return Ok(());
    };
    if store.is_empty() {
        return Err("--store 的目录是空的".to_string());
    }
    // ⚠ SAFETY 那条纪律（Rust 2024）在这里适用得起来：`main` 的第一行是**单线程**的，
    //   还没有第二个线程会读环境（图脚本是同步的，`std::thread` 一个都没起）。
    unsafe { std::env::set_var("PX_ART", &store) };
    Ok(())
}

/// `std::env::args()` **去掉 `--store` 与它那个值**的那一份（argv[0] 也去掉了）。
///
/// 它就是"图程序自己那几条参数"：自己读位置参数的图程序（`scene` / `passes`）
/// 必须走这一份，理由见 [`apply_store_args`]。
pub fn args_without_store() -> Result<Vec<String>, String> {
    Ok(split_store_args()?.1)
}

/// **唯一那一处解析**：命令行长什么样、`--store` 怎么认，只在这里回答一次。
/// 认两种写法（`--store D` 与 `--store=D`）；交回 `(那个目录, 剩下的参数)`。
///
/// ⚠ 两种写法都要，而"值"只在 `--store` 后面那一格 —— 写成 `--store=D` 时
///   `strip_prefix` 拿到的就是值本身，别再去看下一格。
fn split_store_args() -> Result<(Option<String>, Vec<String>), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut store: Option<String> = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--store" {
            let Some(value) = args.get(index + 1) else {
                return Err("--store 后面要跟一个目录（--store <目录>）".to_string());
            };
            store = Some(value.clone());
            index += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--store=") {
            store = Some(value.to_string());
            index += 1;
            continue;
        }
        rest.push(arg.to_string());
        index += 1;
    }
    Ok((store, rest))
}

/// **开始一张图**：把画布、参数目录、清单都挂在一个 `Graph` 句柄上交回去。
///
/// ⚠ 它交回句柄而不是往全局塞 —— 同**一个进程里可以同时跑两张图**（测试、
///   "一把跑全部图"那种入口），彼此不会串。
pub fn begin(spec: GraphSpec) -> Graph {
    let root = workspace_root();
    // ⚠ 参数根走 `param_root()`（`PX_ART` 可以把它换到会话副本上，见模块文档）：
    //   这里**只读路径**，`spec` 那一边一个字都不用知道。
    let param_dir = param_root().join(&spec.name);
    let cache_root = root.join("target").join("pcg");
    let fresh = std::env::var("PX_PCG_FRESH")
        .map(|value| value != "0")
        .unwrap_or(false);
    let cached = cache_root.join(&spec.name).join("manifest.json").is_file() as usize;

    let graph = Graph {
        param_dir,
        cache_root,
        fresh,
        manifest: Mutex::new(Vec::new()),
        params_used: Mutex::new(BTreeMap::new()),
        spec,
    };
    std::fs::create_dir_all(&graph.cache_root).ok();

    println!(
        "图 {}｜参数 {}{}｜缓存 {} 条{}",
        graph.spec.name,
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
            params: shader_params(text, closure, &descriptor),
            blobs: vec![wgsl_blob.header.clone(), schema_blob.header.clone()],
            fingerprint: fnv1a(text),
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
    let assembled = px_shader::assemble::render_source(
        text,
        modules,
        px_shader::host_stubs::wgpu_host_stub,
        &mut seen,
    );
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
        ("closure_modules".to_string(), closure.modules.len() as f64),
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

// ---------------------------------------------------------------------------
// `shaders` 那张图（`art/shaders/*.wgsl` → CAS + 清单）
// ---------------------------------------------------------------------------

/// 一份烘好的入口 shader（读数用；写盘的事在 [`bake_shader_graph`] 里做完了）。
pub struct BakedShader {
    pub slot: String,
    pub key: Key,
    pub artifact: PathBuf,
    pub bytes: u64,
    pub wgsl_bytes: usize,
    /// 闭包摘要（`px_shader::Closure::summary`）。
    pub closure: String,
}

/// **烘整张 `shaders` 图**：`art/shaders/` 下**每一份入口**（没有 `#define_import_path`
/// 的 `.wgsl`）反射出契约、写进 CAS，再写 `target/pcg/shaders/manifest.json`。
///
/// ⚠ 它从前**只住在 bin 里**（`px_graphs/src/bin/shaders.rs`），于是任何需要这张图的
///   下游（`px-scene` 的帧图测试、场景编译）都只能**假设盘上已经有**那份清单：
///   干净 checkout（`target/` 被忽略）一跑测试就红 —— 实测 `px-scene` 的
///   `frame::tests` 三条全红（"图 'shaders' 的清单读不到"）。
///   判据要的是"靶子在仓库里、产物可重跑"，所以这份"可重跑"必须是一个**能被调用的函数**，
///   而不是一段只活在某个 bin 的 `main` 里的代码。
/// ⚠ 路径一律从 [`workspace_root`] 起算（**不看 CWD**）：测试的 CWD 是 crate 目录，
///   而 bin 的 CWD 是工作区根 —— 同一份逻辑在两个 CWD 下必须读到同一批文件。
/// ⚠ 幂等：键是内容的纯函数，重复烘只是重写同一批字节（清单也逐字节相同）。
pub fn bake_shader_graph() -> Result<Vec<BakedShader>, String> {
    let root = workspace_root();
    let dir = root.join("art").join("shaders");
    let modules = px_shader::workspace_modules(&root)?;

    let mut slots: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(&dir)
        .map_err(|err| format!("读不了 shader 目录 {}：{err}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("wgsl") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
        // 「是不是入口」只有一条判据：**没有 `#define_import_path`**（那是模块的标记）。
        if px_shader::import_path_of(&text).is_some() {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
            return Err(format!("{} 没有文件名", path.display()));
        };
        slots.push(name.to_string());
    }
    slots.sort();
    if slots.is_empty() {
        return Err(format!(
            "{} 里一个入口 shader 都没有：槽表是从目录扫出来的，扫不到就没有东西可烘",
            dir.display()
        ));
    }

    let mut baked = Vec::with_capacity(slots.len());
    let mut manifest: Vec<ManifestEntry> = Vec::with_capacity(slots.len());
    for slot in &slots {
        let path = dir.join(format!("{slot}.wgsl"));
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
        // include 闭包进键（§17.1、§52.3）：改一个被 import 的模块也得换键。
        let closure = px_shader::closure(&text, &modules);
        let (key, artifact, bytes) = write_shader(slot, &text, &closure, &modules)?;
        let summary = closure.summary();
        manifest.push(ManifestEntry {
            node: slot.clone(),
            op: "shader.wgsl".to_string(),
            op_version: SHADER_VERSION,
            key: hex(&key),
            hit: false,
            millis: 0,
            bytes,
            detail: format!("{} 字节 WGSL｜{summary}", text.len()),
        });
        baked.push(BakedShader {
            slot: slot.clone(),
            key,
            artifact,
            bytes,
            wgsl_bytes: text.len(),
            closure: summary,
        });
    }
    write_graph_manifest("shaders", &manifest)?;
    Ok(baked)
}
