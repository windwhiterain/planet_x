//! 驱动：读参数、算键、查 CAS、叫算子、写产物、记清单。
//!
//! ⚠ 这里是**唯一**知道「一个节点怎么走完一趟」的地方。它与算子之间只有两样东西：
//! 描述符表（`px_graph_schema::OpLibrary`）与序列化载荷（`PayloadBundle`）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use px_graph_schema::{
    GraphSpec, Grid, IndexEntry, Key, ManifestEntry, OpDescriptor, OpKind, OpLibrary, PayloadBundle,
    fnv1a, hex, hex_short, key_with_cameras, node_key,
};
use px_protocol::art::{Camera, Domain};
use px_protocol::stream::{self, Frame};

use px_field_schema::field::{Field, Stats};
use px_field_schema::payload as field_payload;
use px_mesh_schema::MeshData;
use px_mesh_schema::payload as mesh_payload;
use px_volume_schema::payload as volume_payload;
use px_volume_schema::{PATCHES, VolumeData};

/// **缓存机制**：图脚本那边的类型化门面（`px_cook`）只认这一个接口。
///
/// ⚠ 它**不认识任何算子** —— 报上下文、读参数原文、查/写 CAS、记读数，就这四件事。
/// 于是「类型化的算子契约」与「缓存的实现」各自独立：前者在 `px_cook`，
/// 后者在这里，两边都不需要知道对方的算子长什么样。
pub trait Cache {
    fn graph_version(&self) -> u32;
    fn canvas(&self) -> (u32, u32);
    fn projection(&self) -> Domain;
    fn cameras(&self) -> &[Camera];
    /// `art/<图>/<name>.toml` 的原文；`None` = 文件不存在 ⇒ 用算子默认值。
    fn params_text(&self, name: &str) -> Option<String>;
    /// CAS 里那份字节。`PX_PCG_FRESH=1` 时一律 `None`（本次全部重算）。
    fn fetch(&self, key: Key) -> Option<Vec<u8>>;
    /// 键不在盘上：把产物写进 CAS、记索引与清单，并把读数打出来。
    fn store(&self, report: Report<'_>, bytes: &[u8]) -> Result<(), String>;
}

/// 一次 cook 的读数 —— 与 `node()` 打出来的那几行同一档。
pub struct Report<'a> {
    pub node: &'a str,
    pub op: &'static str,
    pub op_version: u32,
    pub key: Key,
    pub hit: bool,
    pub millis: u64,
    pub bytes: usize,
    /// 与老路径同一个口径：体积那一档不掺评审相机。
    pub with_cameras: bool,
}

/// 缓存机制的句柄。`begin(GraphSpec)` 之后才有，用 `driver()` 取。
pub struct Driver;

impl Driver {
    pub fn graph_version(&self) -> u32 {
        context().spec.version
    }
}

impl Cache for Driver {
    fn graph_version(&self) -> u32 {
        context().spec.version
    }

    fn canvas(&self) -> (u32, u32) {
        (context().spec.width, context().spec.height)
    }

    fn projection(&self) -> Domain {
        context().spec.projection
    }

    fn cameras(&self) -> &[Camera] {
        &context().spec.cameras
    }

    fn params_text(&self, name: &str) -> Option<String> {
        load_params_text(&context().param_dir, name)
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
        // 所以这里必须把节点名与相机表补回去 —— 老路径那句话（`to_bytes(name, cameras)`）
        // 在这一层同样成立，漏了就会写出"id 空、相机空"的产物（逐字节对账会当场抓到）。
        let bytes = bundling(bytes, report.node, report.op, report.with_cameras, &context.spec)?;
        let path = artifact_path(&context.cache_root, &report.key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("建目录 {} 失败：{err}", parent.display()))?;
        }
        std::fs::write(&path, &bytes)
            .map_err(|err| format!("写产物 {} 失败：{err}", path.display()))?;

        if !report.hit {
            let payload = decode_payload(
                &bytes,
                kind_of(report.op),
                context.spec.projection,
                report.node,
            );
            let stats = payload_stats(&payload);
            let entry = ManifestEntry {
                node: report.node.to_string(),
                op: report.op.to_string(),
                op_version: report.op_version,
                key: hex(&report.key),
                hit: false,
                millis: report.millis,
                bytes: bytes.len() as u64,
                min: stats.min,
                max: stats.max,
                mean: stats.mean,
            };
            println!(
                "重算 {:<12} {:<16} v{}  {}  {:>5} ms  {:>9} B",
                report.node,
                report.op,
                report.op_version,
                hex_short(&report.key),
                report.millis,
                bytes.len(),
            );
            context
                .manifest
                .lock()
                .expect("清单锁坏了")
                .push(entry);
        } else {
            println!(
                "命中 {:<12} {:<16} v{}  {}  {:>5} ms  {:>9} B",
                report.node,
                report.op,
                report.op_version,
                hex_short(&report.key),
                report.millis,
                bytes.len(),
            );
        }
        Ok(())
    }
}

/// 把算子回的占位载荷补成产物：**节点名 + 本次该带的相机表**。
///
/// ⚠ 相机那一档与老路径逐字同一条口径：体积不进相机（相机是「怎么看」，体积没人看）。
fn bundling(
    bytes: &[u8],
    node: &str,
    op_id: &str,
    with_cameras: bool,
    spec: &GraphSpec,
) -> Result<Vec<u8>, String> {
    let bundle = PayloadBundle::from_bytes(bytes)?;
    let _ = op_id;
    let cameras: &[Camera] = if with_cameras { &spec.cameras } else { &[] };
    bundle.to_bytes(node, cameras)
}

/// 取缓存机制的句柄（`begin` 之后才有效）。
pub fn driver() -> Driver {
    Driver
}

fn kind_of(op_id: &str) -> OpKind {
    if op_id.starts_with("volume.") || op_id == px_volume_schema::params::CLOUD_COARSE {
        OpKind::Volume
    } else if op_id.starts_with("mesh.") {
        OpKind::Mesh
    } else {
        OpKind::Field
    }
}

fn payload_stats(payload: &Payload) -> Stats {
    match payload {
        Payload::Field(field) => field.stats(),
        Payload::Mesh(mesh) => Stats {
            min: mesh.vertices() as f32,
            max: mesh.triangles() as f32,
            mean: 0.0,
        },
        Payload::Volume(volume) => volume_stats(volume),
    }
}

pub enum Payload {
    Field(Field),
    Mesh(MeshData),
    Volume(VolumeData),
}

impl Payload {
    pub fn field(&self) -> &Field {
        match self {
            Self::Field(field) => field,
            _ => panic!("这个产物是网格/体积，不是场"),
        }
    }

    pub fn mesh(&self) -> &MeshData {
        match self {
            Self::Mesh(mesh) => mesh,
            _ => panic!("这个产物是场/体积，不是网格"),
        }
    }

    pub fn volume(&self) -> &VolumeData {
        match self {
            Self::Volume(volume) => volume,
            _ => panic!("这个产物是场/网格，不是体积"),
        }
    }
}

pub struct Artifact {
    pub key: Key,
    pub payload: Payload,
    /// 产物在 CAS 里的那串字节。下游算子吃的是它（**不是**重新序列化一遍）。
    pub bytes: Vec<u8>,
}

impl Artifact {
    pub fn field(&self) -> &Field {
        self.payload.field()
    }

    pub fn mesh(&self) -> &MeshData {
        self.payload.mesh()
    }

    pub fn volume(&self) -> &VolumeData {
        self.payload.volume()
    }
}

struct Context {
    spec: GraphSpec,
    param_dir: PathBuf,
    cache_root: PathBuf,
    fresh: bool,
    /// 算子库**按需装载**：只写 shader / 只写场景的图（`--bin shaders` / `--bin scene`）
    /// 一个算子都不需要，不该因为找不到 dylib 就起不来。
    libraries: OnceLock<Vec<OpLibrary>>,
    index: Mutex<BTreeMap<String, IndexEntry>>,
    manifest: Mutex<Vec<ManifestEntry>>,
}

impl Context {
    fn libraries(&self) -> &[OpLibrary] {
        self.libraries.get_or_init(|| {
            let loaded = load_ops();
            let ops: Vec<String> = loaded
                .iter()
                .flat_map(|library| library.descriptors())
                .map(|op| format!("{}@v{}", op.id, op.version))
                .collect();
            println!(
                "算子库 {} 个：{}",
                loaded.len(),
                ops.join(" / "),
            );
            loaded
        })
    }

    /// 按 op_id 在**已装载的**算子库里找它。找不到就把已装载的算子列出来 ——
    /// 「静默地什么也没算」在这条路上是不允许的。
    fn find(&self, op_id: &str) -> (&OpLibrary, OpDescriptor) {
        let libraries = self.libraries();
        for library in libraries {
            if let Some(descriptor) = library.descriptor(op_id) {
                return (library, descriptor);
            }
        }
        let known: Vec<&str> = libraries
            .iter()
            .flat_map(|library| library.descriptors())
            .map(|op| op.id)
            .collect();
        panic!(
            "不认识算子 '{op_id}'；已装载的算子库 {} 个，算子：{}",
            libraries.len(),
            known.join(" / "),
        );
    }
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 算子库找哪几个目录（按顺序，先在哪个目录找到就只认那一个）。
///
/// `cargo run`/`cargo test` 把 exe 放进 `target/<profile>`（测试在 `.../deps`），
/// 所以 exe 目录、它的上一级、以及 workspace 的 `target/<profile>` 都要看一遍。
fn op_directories() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var("PX_GRAPH_OP_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        dirs.push(dir.to_path_buf());
        if let Some(parent) = dir.parent() {
            dirs.push(parent.to_path_buf());
        }
    }
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    dirs.push(workspace_root().join("target").join(profile));
    dirs
}

/// 把算子库都装进来。一个都没找到就**当场拒**，并把该跑的命令打出来。
fn load_ops() -> Vec<OpLibrary> {
    let dirs = op_directories();
    let mut notes = Vec::new();
    for dir in &dirs {
        match px_graph_schema::load::load_directory(dir) {
            Ok(found) if !found.is_empty() => return found,
            Ok(_) => notes.push(format!("{}：没有 px_*_op", dir.display())),
            Err(err) => notes.push(err),
        }
    }
    panic!(
        "一个算子库（px_*_op 动态库）都没找到。\n\
         找过：{}\n\
         先 `cargo build -p px_field_op -p px_volume_op -p px_mesh_op`（改完算子之后要重跑这一条），\n\
         或者用 PX_GRAPH_OP_DIR 指到它们所在的目录。",
        dirs.iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join("、"),
    );
}

pub fn begin(spec: GraphSpec) {
    let root = workspace_root();
    let param_dir = root.join("art").join(&spec.name);
    let cache_root = root.join("target").join("pcg");
    let fresh = std::env::var("PX_PCG_FRESH")
        .map(|value| value != "0")
        .unwrap_or(false);
    let libraries = OnceLock::new();
    let index = load_index(&cache_root);
    let cached = index.len();

    let _ = CONTEXT.set(Context {
        param_dir: param_dir.clone(),
        cache_root: cache_root.clone(),
        fresh,
        libraries,
        index: Mutex::new(index),
        manifest: Mutex::new(Vec::new()),
        spec,
    });

    let context = context();
    std::fs::create_dir_all(&context.cache_root).ok();

    println!(
        "图 {} v{}｜画布 {}×{}｜参数 {}{}｜缓存 {} 条{}",
        context.spec.name,
        context.spec.version,
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

/// 一个节点：`op_id` 说「用哪个算子」，`name` 说「参数文件叫什么、清单里记什么」。
///
/// 两遍走（§17.1）：先按描述符表把参数规范化、算键、查 CAS；键不在盘上才叫算子求值。
pub fn node(op_id: &str, name: &str, inputs: &[&Artifact]) -> Artifact {
    let context = context();
    let (library, descriptor) = context.find(op_id);
    assert_eq!(
        inputs.len(),
        descriptor.inputs.len(),
        "{} 需要 {} 个输入 {}，实际给了 {}",
        op_id,
        descriptor.inputs.len(),
        descriptor.inputs.join(" / "),
        inputs.len(),
    );

    let params_path = context.param_dir.join(format!("{name}.toml"));
    let params_text = load_params_text(&context.param_dir, name);
    let params_json = library
        .canonical_params(op_id, params_text.as_deref())
        .unwrap_or_else(|err| panic!("读参数 {} 失败：{err}", params_path.display()));

    let input_keys: Vec<Key> = inputs.iter().map(|artifact| artifact.key).collect();
    let key = node_key(
        op_id,
        descriptor.version,
        context.spec.version,
        (context.spec.width, context.spec.height),
        context.spec.projection,
        &params_json,
        &input_keys,
    );
    // ⚠ 体积那一档**不掺相机**：相机是「怎么看」，而体积没人看（渲染器只读 mesh）。
    let key = if descriptor.kind == OpKind::Volume {
        key
    } else {
        key_with_cameras(key, &context.spec.cameras)
    };
    let path = artifact_path(&context.cache_root, &key);
    let short = hex_short(&key);
    let started = Instant::now();

    let cached = if context.fresh {
        None
    } else {
        std::fs::read(&path).ok()
    };

    let (bytes, hit, size) = match cached {
        Some(bytes) => {
            let size = bytes.len() as u64;
            (bytes, true, size)
        }
        None => {
            let borrowed: Vec<&[u8]> = inputs
                .iter()
                .map(|artifact| artifact.bytes.as_slice())
                .collect();
            let grid = Grid {
                width: context.spec.width,
                height: context.spec.height,
                projection: context.spec.projection,
            };
            let out = library
                .call(op_id, &params_json, grid, &borrowed)
                .unwrap_or_else(|err| panic!("{name}（{op_id}）求值失败：{err}"));
            let bundle = PayloadBundle::from_bytes(&out)
                .unwrap_or_else(|err| panic!("{name}（{op_id}）回的载荷解不开：{err}"));
            let cameras: &[Camera] = if descriptor.kind == OpKind::Volume {
                &[]
            } else {
                &context.spec.cameras
            };
            let bytes = bundle
                .to_bytes(name, cameras)
                .unwrap_or_else(|err| panic!("包 {name} 的产物失败：{err}"));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            std::fs::write(&path, &bytes)
                .unwrap_or_else(|err| panic!("写产物 {} 失败：{err}", path.display()));
            let size = bytes.len() as u64;
            (bytes, false, size)
        }
    };

    let payload = decode_payload(&bytes, descriptor.kind, context.spec.projection, name);
    let millis = started.elapsed().as_millis() as u64;

    {
        let mut index = context.index.lock().expect("索引锁坏了");
        if hit {
            if let Some(meta) = index.get(&hex(&key)) {
                if meta.op_version == descriptor.version
                    && meta.source_hash != descriptor.source_hash
                {
                    eprintln!(
                        "⚠ {name}（{op_id}）的源码变了但 VERSION 仍是 {}；若输出语义变了，请升版本并加 PX_PCG_FRESH=1 重烘",
                        descriptor.version,
                    );
                }
                if meta.graph_version == context.spec.version
                    && meta.graph_source_hash != context.spec.source_hash
                {
                    eprintln!(
                        "⚠ 图 {} 的源码变了但 GRAPH_VERSION 仍是 {}；若拓扑或写死的常量变了，请升版本并加 PX_PCG_FRESH=1 重烘",
                        context.spec.name, context.spec.version,
                    );
                }
                if meta.dll != library.fingerprint() {
                    eprintln!(
                        "⚠ {name}（{op_id}）这次是拿**另一份**算子库算的（dll {:016x} → {:016x}）而 VERSION 仍是 {}；若输出语义变了，请升版本并加 PX_PCG_FRESH=1 重烘",
                        meta.dll,
                        library.fingerprint(),
                        descriptor.version,
                    );
                }
            }
        } else {
            index.insert(
                hex(&key),
                IndexEntry {
                    op_id: op_id.to_string(),
                    op_version: descriptor.version,
                    graph_version: context.spec.version,
                    source_hash: descriptor.source_hash,
                    graph_source_hash: context.spec.source_hash,
                    node: name.to_string(),
                    millis,
                    bytes: size,
                    dll: library.fingerprint(),
                },
            );
            save_index(&context.cache_root, &index);
        }
    }

    let entry = match &payload {
        Payload::Field(field) => {
            let stats = field.stats();
            println!(
                "{} {:<12} {:<16} v{}  {}  {:>4} ms  {:>9} B  值域 {:.4}..{:.4} 均 {:.4}",
                if hit { "命中" } else { "重算" },
                name,
                op_id,
                descriptor.version,
                short,
                millis,
                size,
                stats.min,
                stats.max,
                stats.mean,
            );
            ManifestEntry {
                node: name.to_string(),
                op: op_id.to_string(),
                op_version: descriptor.version,
                key: hex(&key),
                hit,
                millis,
                bytes: size,
                min: stats.min,
                max: stats.max,
                mean: stats.mean,
            }
        }
        Payload::Mesh(mesh) => {
            println!(
                "{} {:<12} {:<16} v{}  {}  {:>4} ms  {:>9} B  {} 顶点 / {} 三角形",
                if hit { "命中" } else { "重算" },
                name,
                op_id,
                descriptor.version,
                short,
                millis,
                size,
                mesh.vertices(),
                mesh.triangles(),
            );
            ManifestEntry {
                node: name.to_string(),
                op: op_id.to_string(),
                op_version: descriptor.version,
                key: hex(&key),
                hit,
                millis,
                bytes: size,
                min: mesh.vertices() as f32,
                max: mesh.triangles() as f32,
                mean: 0.0,
            }
        }
        Payload::Volume(volume) => {
            let stats = volume_stats(volume);
            println!(
                "{} {:<12} {:<16} v{}  {}  {:>5} ms  {:>9} B  值域 {:.4}..{:.4} 均 {:.4}  {} 面 {}×{}×{} 层",
                if hit { "命中" } else { "重算" },
                name,
                op_id,
                descriptor.version,
                short,
                millis,
                size,
                stats.min,
                stats.max,
                stats.mean,
                PATCHES,
                volume.res,
                volume.res,
                volume.layers,
            );
            ManifestEntry {
                node: name.to_string(),
                op: op_id.to_string(),
                op_version: descriptor.version,
                key: hex(&key),
                hit,
                millis,
                bytes: size,
                min: stats.min,
                max: stats.max,
                mean: stats.mean,
            }
        }
    };

    context
        .manifest
        .lock()
        .expect("清单锁坏了")
        .push(entry);

    Artifact { key, payload, bytes }
}

fn decode_payload(bytes: &[u8], kind: OpKind, projection: Domain, name: &str) -> Payload {
    match kind {
        OpKind::Field => Payload::Field(
            field_payload::decode(bytes, projection)
                .unwrap_or_else(|err| panic!("{name} 的场载荷解不开：{err}")),
        ),
        OpKind::Mesh => Payload::Mesh(
            mesh_payload::decode(bytes)
                .unwrap_or_else(|err| panic!("{name} 的网格载荷解不开：{err}")),
        ),
        OpKind::Volume => Payload::Volume(
            volume_payload::decode(bytes)
                .unwrap_or_else(|err| panic!("{name} 的体积载荷解不开：{err}")),
        ),
    }
}

fn volume_stats(volume: &VolumeData) -> Stats {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    let mut sum = 0.0_f64;
    for value in &volume.data {
        min = min.min(*value);
        max = max.max(*value);
        sum += *value as f64;
    }
    Stats {
        min,
        max,
        mean: if volume.data.is_empty() {
            0.0
        } else {
            (sum / volume.data.len() as f64) as f32
        },
    }
}

pub fn finish() {
    let context = context();
    let manifest = context.manifest.lock().expect("清单锁坏了");
    let hits = manifest.iter().filter(|entry| entry.hit).count();
    let cooked = manifest.len() - hits;
    let millis: u64 = manifest.iter().map(|entry| entry.millis).sum();

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

fn load_index(cache_root: &Path) -> BTreeMap<String, IndexEntry> {
    std::fs::read_to_string(cache_root.join("index.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_index(cache_root: &Path, index: &BTreeMap<String, IndexEntry>) {
    if let Some(parent) = cache_root.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match serde_json::to_string_pretty(index) {
        Ok(text) => {
            if let Err(err) = std::fs::write(cache_root.join("index.json"), text) {
                eprintln!("写缓存索引失败：{err}");
            }
        }
        Err(err) => eprintln!("缓存索引无法序列化：{err}"),
    }
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
