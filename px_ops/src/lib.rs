pub mod field;
pub mod noise;
pub mod ops;

pub use field::{Field, Projection, ProjectionKind, Stats};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use px_protocol::art::{ArtBundle, AssetManifest, MeshData};
use px_protocol::stream::{self, Frame};

pub type Key = [u8; 32];

pub trait FieldOp {
    type Params: Serialize + DeserializeOwned + Default;
    const ID: &'static str;
    const VERSION: u32;
    const SOURCE_HASH: u64;
    const INPUTS: &'static [&'static str];
    fn eval(params: &Self::Params, inputs: &[&Field], grid: Grid) -> Field;
}

pub trait MeshOp {
    type Params: Serialize + DeserializeOwned + Default;
    const ID: &'static str;
    const VERSION: u32;
    const SOURCE_HASH: u64;
    const INPUTS: &'static [&'static str];
    fn eval(params: &Self::Params, inputs: &[&Field], grid: Grid) -> MeshData;
}

#[derive(Debug, Clone)]
pub enum Payload {
    Field(Field),
    Mesh(MeshData),
}

#[derive(Debug, Clone)]
pub struct Artifact {
    pub payload: Payload,
    pub key: Key,
}

impl Artifact {
    pub fn field(&self) -> &Field {
        match &self.payload {
            Payload::Field(field) => field,
            Payload::Mesh(_) => panic!("这个产物是网格，不是场"),
        }
    }

    pub fn mesh(&self) -> &MeshData {
        match &self.payload {
            Payload::Mesh(mesh) => mesh,
            Payload::Field(_) => panic!("这个产物是场，不是网格"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GraphSpec {
    pub name: String,
    pub version: u32,
    pub source_hash: u64,
    pub width: u32,
    pub height: u32,
    pub projection: Projection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub width: u32,
    pub height: u32,
    pub projection: Projection,
}

impl Grid {
    pub fn filled(&self, value: f32) -> Field {
        Field::filled_with(self.width, self.height, value, self.projection)
    }

    pub fn direction(&self, x: u32, y: u32) -> [f32; 3] {
        field::direction_at(self.width, self.height, self.projection, x, y)
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct IndexEntry {
    pub op_id: String,
    pub op_version: u32,
    pub graph_version: u32,
    pub source_hash: u64,
    pub graph_source_hash: u64,
    pub node: String,
    pub millis: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ManifestEntry {
    pub node: String,
    pub op: String,
    pub op_version: u32,
    pub key: String,
    pub hit: bool,
    pub millis: u64,
    pub bytes: u64,
    pub min: f32,
    pub max: f32,
    pub mean: f32,
}

struct Context {
    spec: GraphSpec,
    param_dir: PathBuf,
    cache_root: PathBuf,
    fresh: bool,
    index: Mutex<BTreeMap<String, IndexEntry>>,
    manifest: Mutex<Vec<ManifestEntry>>,
}

static CONTEXT: OnceLock<Context> = OnceLock::new();

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn begin(spec: GraphSpec) {
    let root = workspace_root();
    let param_dir = root.join("art").join(&spec.name);
    let cache_root = root.join("target").join("pcg");
    let fresh = std::env::var("PX_PCG_FRESH")
        .map(|value| value != "0")
        .unwrap_or(false);
    let index = load_index(&cache_root);
    let cached = index.len();

    let _ = CONTEXT.set(Context {
        param_dir: param_dir.clone(),
        cache_root: cache_root.clone(),
        fresh,
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
    CONTEXT.get().expect("先调用 px_ops::begin")
}

pub fn node<Op: FieldOp>(name: &str, inputs: &[&Artifact]) -> Artifact {
    let context = context();
    assert_eq!(
        inputs.len(),
        Op::INPUTS.len(),
        "{} 需要 {} 个输入 {}，实际给了 {}",
        Op::ID,
        Op::INPUTS.len(),
        Op::INPUTS.join(" / "),
        inputs.len(),
    );

    let params = load_params::<Op::Params>(&context.param_dir, name);
    let params_json = canonical_params(&params);
    let input_keys: Vec<Key> = inputs.iter().map(|artifact| artifact.key).collect();
    let key = node_key(
        Op::ID,
        Op::VERSION,
        context.spec.version,
        (context.spec.width, context.spec.height),
        context.spec.projection,
        &params_json,
        &input_keys,
    );
    let path = artifact_path(&context.cache_root, &key);
    let short = hex_short(&key);
    let started = Instant::now();

    let cached = if context.fresh {
        None
    } else {
        load_artifact(&path, context.spec.projection).ok()
    };

    let (payload, hit, bytes) = match cached {
        Some(payload) => {
            let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            (payload, true, bytes)
        }
        None => {
            let borrowed: Vec<&Field> = inputs.iter().map(|artifact| artifact.field()).collect();
            let field = Op::eval(
                &params,
                &borrowed,
                Grid {
                    width: context.spec.width,
                    height: context.spec.height,
                    projection: context.spec.projection,
                },
            );
            let payload = Payload::Field(field);
            let bytes = write_artifact(&path, name, &payload).unwrap_or_else(|err| {
                panic!("写产物 {} 失败：{err}", path.display());
            });
            (payload, false, bytes)
        }
    };
    let field = match &payload {
        Payload::Field(field) => field.clone(),
        Payload::Mesh(_) => unreachable!(),
    };

    let millis = started.elapsed().as_millis() as u64;
    let stats = field.stats();

    if hit {
        let index = context.index.lock().expect("索引锁坏了");
        if let Some(meta) = index.get(&hex(&key)) {
            if meta.op_version == Op::VERSION && meta.source_hash != Op::SOURCE_HASH {
                eprintln!(
                    "⚠ {name}（{}）的源码变了但 VERSION 仍是 {}；若输出语义变了，请升版本并加 PX_PCG_FRESH=1 重烘",
                    Op::ID, Op::VERSION,
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
        }
    } else {
        let mut index = context.index.lock().expect("索引锁坏了");
        index.insert(
            hex(&key),
            IndexEntry {
                op_id: Op::ID.to_string(),
                op_version: Op::VERSION,
                graph_version: context.spec.version,
                source_hash: Op::SOURCE_HASH,
                graph_source_hash: context.spec.source_hash,
                node: name.to_string(),
                millis,
                bytes,
            },
        );
        save_index(&context.cache_root, &index);
    }


    println!(
        "{} {:<12} {:<16} v{}  {}  {:>4} ms  {:>9} B  值域 {:.4}..{:.4} 均 {:.4}",
        if hit { "命中" } else { "重算" },
        name,
        Op::ID,
        Op::VERSION,
        short,
        millis,
        bytes,
        stats.min,
        stats.max,
        stats.mean,
    );

    context
        .manifest
        .lock()
        .expect("清单锁坏了")
        .push(ManifestEntry {
            node: name.to_string(),
            op: Op::ID.to_string(),
            op_version: Op::VERSION,
            key: hex(&key),
            hit,
            millis,
            bytes,
            min: stats.min,
            max: stats.max,
            mean: stats.mean,
        });

    Artifact { payload, key }
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

pub fn canonical_params<T: Serialize>(params: &T) -> String {
    let value = serde_json::to_value(params).expect("参数无法序列化");
    serde_json::to_string(&sorted(value)).expect("参数无法规范化")
}

fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, sorted(v))).collect()),
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

pub fn node_key(
    op_id: &str,
    op_version: u32,
    graph_version: u32,
    canvas: (u32, u32),
    projection: Projection,
    params_json: &str,
    input_keys: &[Key],
) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_pcg/v1");
    hasher.update(op_id.as_bytes());
    hasher.update(&op_version.to_le_bytes());
    hasher.update(&graph_version.to_le_bytes());
    hasher.update(&canvas.0.to_le_bytes());
    hasher.update(&canvas.1.to_le_bytes());
    hasher.update(projection.name().as_bytes());
    hasher.update(params_json.as_bytes());
    for key in input_keys {
        hasher.update(key);
    }
    *hasher.finalize().as_bytes()
}

pub fn hex(key: &Key) -> String {
    let mut out = String::with_capacity(64);
    for byte in key {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn hex_short(key: &Key) -> String {
    hex(key)[..12].to_string()
}

pub fn artifact_path_of(key: &Key) -> PathBuf {
    artifact_path(&context().cache_root, key)
}

fn artifact_path(cache_root: &Path, key: &Key) -> PathBuf {
    let full = hex(key);
    cache_root
        .join("ab")
        .join(&full[..2])
        .join(format!("{full}.pxart"))
}

fn load_params<P: Serialize + DeserializeOwned + Default>(param_dir: &Path, name: &str) -> P {
    let path = param_dir.join(format!("{name}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text)
            .unwrap_or_else(|err| panic!("读参数 {} 失败：{err}", path.display())),
        Err(_) => P::default(),
    }
}

fn write_artifact(path: &Path, id: &str, payload: &Payload) -> Result<u64, String> {
    let (kind, blobs, params) = match payload {
        Payload::Field(field) => (
            field.projection.asset_kind(),
            vec![field.to_blob()],
            BTreeMap::from([
                ("width".to_string(), field.width as f64),
                ("height".to_string(), field.height as f64),
            ]),
        ),
        Payload::Mesh(mesh) => (
            px_protocol::art::AssetKind::Mesh,
            mesh.blobs(),
            BTreeMap::from([
                ("vertices".to_string(), mesh.vertices() as f64),
                ("triangles".to_string(), mesh.triangles() as f64),
            ]),
        ),
    };
    let bundle = ArtBundle {
        assets: vec![AssetManifest {
            id: id.to_string(),
            kind,
            params,
            blobs: blobs.iter().map(|blob| blob.header.clone()).collect(),
        }],
    };
    let mut frames = vec![Frame::Art(bundle)];
    frames.extend(blobs.into_iter().map(Frame::Blob));

    let mut bytes = Vec::new();
    stream::write_stream(&mut bytes, &frames).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(path, &bytes).map_err(|err| err.to_string())?;
    Ok(bytes.len() as u64)
}

fn load_artifact(path: &Path, projection: Projection) -> Result<Payload, String> {
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    let frames = stream::read_stream(&mut bytes.as_slice()).map_err(|err| err.to_string())?;
    let blob = frames
        .iter()
        .find_map(|frame| match frame {
            Frame::Blob(blob) => Some(blob),
            _ => None,
        })
        .ok_or_else(|| "产物里没有数据块".to_string())?;
    let kind = frames.iter().find_map(|frame| match frame {
        Frame::Art(bundle) => bundle.assets.first().map(|asset| asset.kind),
        _ => None,
    });
    if kind == Some(px_protocol::art::AssetKind::Mesh) {
        let blobs: Vec<&px_protocol::wire::Blob> = frames
            .iter()
            .filter_map(|frame| match frame {
                Frame::Blob(blob) => Some(blob),
                _ => None,
            })
            .collect();
        return MeshData::from_blobs(&blobs)
            .map(Payload::Mesh)
            .map_err(|err| err.to_string());
    }

    let mut field = Field::from_blob(blob).map_err(|err| err.to_string())?;
    field.projection = projection;
    Ok(Payload::Field(field))
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

pub fn mesh_node<Op: MeshOp>(name: &str, inputs: &[&Artifact]) -> Artifact {
    let context = context();
    assert_eq!(
        inputs.len(),
        Op::INPUTS.len(),
        "{} 需要 {} 个输入 {}，实际给了 {}",
        Op::ID,
        Op::INPUTS.len(),
        Op::INPUTS.join(" / "),
        inputs.len(),
    );

    let params = load_params::<Op::Params>(&context.param_dir, name);
    let params_json = canonical_params(&params);
    let input_keys: Vec<Key> = inputs.iter().map(|artifact| artifact.key).collect();
    let key = node_key(
        Op::ID,
        Op::VERSION,
        context.spec.version,
        (context.spec.width, context.spec.height),
        context.spec.projection,
        &params_json,
        &input_keys,
    );
    let path = artifact_path(&context.cache_root, &key);
    let short = hex_short(&key);
    let started = Instant::now();

    let cached = if context.fresh {
        None
    } else {
        load_artifact(&path, context.spec.projection).ok()
    };

    let (payload, hit, bytes) = match cached {
        Some(payload) => {
            let bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
            (payload, true, bytes)
        }
        None => {
            let borrowed: Vec<&Field> = inputs.iter().map(|artifact| artifact.field()).collect();
            let mesh = Op::eval(
                &params,
                &borrowed,
                Grid {
                    width: context.spec.width,
                    height: context.spec.height,
                    projection: context.spec.projection,
                },
            );
            let payload = Payload::Mesh(mesh);
            let bytes = write_artifact(&path, name, &payload).unwrap_or_else(|err| {
                panic!("写产物 {} 失败：{err}", path.display());
            });
            (payload, false, bytes)
        }
    };

    let millis = started.elapsed().as_millis() as u64;
    let (vertices, triangles) = match &payload {
        Payload::Mesh(mesh) => (mesh.vertices(), mesh.triangles()),
        Payload::Field(_) => unreachable!(),
    };

    if hit {
        let index = context.index.lock().expect("索引锁坏了");
        if let Some(meta) = index.get(&hex(&key)) {
            if meta.op_version == Op::VERSION && meta.source_hash != Op::SOURCE_HASH {
                eprintln!(
                    "⚠ {name}（{}）的源码变了但 VERSION 仍是 {}；若输出语义变了，请升版本并加 PX_PCG_FRESH=1 重烘",
                    Op::ID, Op::VERSION,
                );
            }
        }
    } else {
        let mut index = context.index.lock().expect("索引锁坏了");
        index.insert(
            hex(&key),
            IndexEntry {
                op_id: Op::ID.to_string(),
                op_version: Op::VERSION,
                graph_version: context.spec.version,
                source_hash: Op::SOURCE_HASH,
                graph_source_hash: context.spec.source_hash,
                node: name.to_string(),
                millis,
                bytes,
            },
        );
        save_index(&context.cache_root, &index);
    }


    println!(
        "{} {:<12} {:<16} v{}  {}  {:>4} ms  {:>9} B  {} 顶点 / {} 三角形",
        if hit { "命中" } else { "重算" },
        name,
        Op::ID,
        Op::VERSION,
        short,
        millis,
        bytes,
        vertices,
        triangles,
    );

    context
        .manifest
        .lock()
        .expect("清单锁坏了")
        .push(ManifestEntry {
            node: name.to_string(),
            op: Op::ID.to_string(),
            op_version: Op::VERSION,
            key: hex(&key),
            hit,
            millis,
            bytes,
            min: vertices as f32,
            max: triangles as f32,
            mean: 0.0,
        });

    Artifact { payload, key }
}







