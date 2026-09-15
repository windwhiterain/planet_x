pub mod field;
pub mod generate;
pub mod noise;
pub mod cameras;
pub mod ops;
pub mod volume;

pub use field::{Field, Projection, ProjectionKind, Stats};
pub use px_protocol::art::{MeshData, VolumeData};
pub use volume::{PATCHES, VolumeGrid, VolumeSampler, direction_of, point_of};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use px_protocol::art::{ArtBundle, AssetManifest};
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

/// 烘一张 3D 标量网格（立方球参数空间）。
///
/// 它是**驱动**那一侧的算子：粗场怎么算只有驱动知道（`px_verify` 里的参照实现
/// 依赖 `px_ops`，所以 `px_ops` 不能反过来依赖它 —— 接口得倒过来）。
pub trait VolumeOp: Sized {
    type Params: Serialize + DeserializeOwned + Default;
    const ID: &'static str;
    const VERSION: u32;
    const SOURCE_HASH: u64;
    const INPUTS: &'static [&'static str];
    fn bake(params: &Self::Params, inputs: &[&Field]) -> VolumeData;
}

/// 拿一张 3D 标量网格出等值面网格。
///
/// 它是**实现库**那一侧的算子：`px_mc` 这种叶子 crate 实现它，只依赖 `px_ops` + 等值面库，
/// 不认识云、不认识驱动 ⇒ 换库不动 `px_graphs`。
pub trait IsosurfaceOp: Sized {
    type Params: Serialize + DeserializeOwned + Default;
    const ID: &'static str;
    const VERSION: u32;
    const SOURCE_HASH: u64;
    const INPUTS: &'static [&'static str];
    fn surface(params: &Self::Params, field: &dyn VolumeSampler) -> Result<MeshData, String>;
}

#[derive(Debug, Clone)]
pub enum Payload {
    Field(Field),
    Mesh(MeshData),
    Volume(VolumeData),
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
            _ => panic!("这个产物是网格/体积，不是场"),
        }
    }

    pub fn mesh(&self) -> &MeshData {
        match &self.payload {
            Payload::Mesh(mesh) => mesh,
            _ => panic!("这个产物是场/体积，不是网格"),
        }
    }

    pub fn volume(&self) -> &VolumeData {
        match &self.payload {
            Payload::Volume(volume) => volume,
            _ => panic!("这个产物是场/网格，不是体积"),
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
    /// 评审相机表：写进每一个产物（局部方向 + 距离，见 `px_protocol::art::Camera`）。
    /// 它属于「怎么看」不属于「是什么」，但住在产物里 ⇒ 必须进缓存键（见 `key_with_cameras`）。
    pub cameras: Vec<px_protocol::art::Camera>,
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
    let key = key_with_cameras(key, &context.spec.cameras);
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
            let bytes = write_artifact(&path, name, &payload, &context.spec.cameras).unwrap_or_else(|err| {
                panic!("写产物 {} 失败：{err}", path.display());
            });
            (payload, false, bytes)
        }
    };
    let field = match &payload {
        Payload::Field(field) => field.clone(),
        _ => unreachable!(),
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

/// 相机表住在产物里 ⇒ 它变了产物内容就变了 ⇒ 必须进键。
/// 否则 CAS 会出现「同一个键、不同内容」（§17.1 那条「键 = 内容」）。
fn key_with_cameras(key: Key, cameras: &[px_protocol::art::Camera]) -> Key {
    if cameras.is_empty() {
        return key;
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(&key);
    for camera in cameras {
        for value in camera.direction {
            hasher.update(&value.to_le_bytes());
        }
        hasher.update(&camera.distance.to_le_bytes());
        hasher.update(camera.tag.as_bytes());
    }
    *hasher.finalize().as_bytes()
}

/// 载荷内容的 FNV-1a。节点名也混进去，免得两个载荷相同的节点指纹撞上。
/// （`generate` 那一侧的贴图/网格产物走的就是这里，口径与场、体积一致。）
pub fn payload_fingerprint(id: &str, blobs: &[px_protocol::wire::Blob]) -> u64 {
    let mut hash = noise::fnv1a(id);
    for blob in blobs {
        for byte in &blob.bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(noise::FNV_PRIME);
        }
    }
    hash
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

/// Shader 和场、网格一样是内容寻址的：键 = WGSL 的字节。
/// 改一个字就换一个键 ⇒ 不会出现「同一个键、不同内容」。
pub fn shader_key(text: &str) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_shader/v1");
    hasher.update(&SHADER_VERSION.to_le_bytes());
    hasher.update(text.as_bytes());
    *hasher.finalize().as_bytes()
}

/// 把一份 WGSL 写进 CAS：清单帧（`kind = Shader`）+ 一个 U8 blob。
pub fn write_shader(id: &str, text: &str) -> Result<(Key, PathBuf, u64), String> {
    let key = shader_key(text);
    let path = artifact_path(&context().cache_root, &key);
    let bytes = text.as_bytes().to_vec();
    let blob = px_protocol::Blob::new(
        px_protocol::BlobHeader {
            dtype: px_protocol::DType::U8,
            shape: vec![bytes.len() as u32],
        },
        bytes,
    )
    .map_err(|err| err.to_string())?;
    let bundle = ArtBundle {
        assets: vec![AssetManifest {
            id: id.to_string(),
            kind: px_protocol::AssetKind::Shader,
            params: BTreeMap::from([
                ("wgsl_bytes".to_string(), text.len() as f64),
                ("shader_version".to_string(), f64::from(SHADER_VERSION)),
            ]),
            blobs: vec![blob.header.clone()],
            fingerprint: noise::fnv1a(text),
            cameras: Vec::new(),
        }],
    };
    let frames = vec![Frame::Art(bundle), Frame::Blob(blob)];
    let mut out = Vec::new();
    stream::write_stream(&mut out, &frames).map_err(|err| err.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    std::fs::write(&path, &out).map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
    Ok((key, path, out.len() as u64))
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

/// 读一份 Shader 产物由渲染侧负责（`px_protocol::art::read_shader`）——
/// 渲染器不许依赖 PCG 这一侧（§10.1）。

fn load_params<P: Serialize + DeserializeOwned + Default>(param_dir: &Path, name: &str) -> P {
    let path = param_dir.join(format!("{name}.toml"));
    match std::fs::read_to_string(&path) {
        Ok(text) => toml::from_str(&text)
            .unwrap_or_else(|err| panic!("读参数 {} 失败：{err}", path.display())),
        Err(_) => P::default(),
    }
}

/// 读某个节点的参数（与 `node` / `volume_node` 用的是同一份 toml）。
/// 仪器（判据检查、量 `L`）要拿同一份数，不能自己再抄一遍。
pub fn params_of<P: Serialize + DeserializeOwned + Default>(name: &str) -> P {
    load_params(&context().param_dir, name)
}

fn write_artifact(
    path: &Path,
    id: &str,
    payload: &Payload,
    cameras: &[px_protocol::art::Camera],
) -> Result<u64, String> {
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
        Payload::Volume(volume) => (
            px_protocol::art::AssetKind::Volume,
            volume.blobs(),
            BTreeMap::from([
                ("res".to_string(), volume.res as f64),
                ("layers".to_string(), volume.layers as f64),
                ("inner".to_string(), volume.inner as f64),
                ("outer".to_string(), volume.outer as f64),
            ]),
        ),
    };
    let fingerprint = payload_fingerprint(id, &blobs);
    let bundle = ArtBundle {
        assets: vec![AssetManifest {
            id: id.to_string(),
            kind,
            params,
            blobs: blobs.iter().map(|blob| blob.header.clone()).collect(),
            fingerprint,
            cameras: cameras.to_vec(),
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
    if kind == Some(px_protocol::art::AssetKind::Volume) {
        // 壳半径住在清单参数里（载荷只存数组），所以这里要把清单一起看。
        let asset = frames.iter().find_map(|frame| match frame {
            Frame::Art(bundle) => bundle.assets.first(),
            _ => None,
        });
        let params = asset.map(|asset| &asset.params);
        let mut volume = VolumeData::from_blob(blob).map_err(|err| err.to_string())?;
        if let Some(params) = params {
            volume.inner = params.get("inner").copied().unwrap_or(0.0) as f32;
            volume.outer = params.get("outer").copied().unwrap_or(0.0) as f32;
        }
        return Ok(Payload::Volume(volume));
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
    let key = key_with_cameras(key, &context.spec.cameras);
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
            let bytes = write_artifact(&path, name, &payload, &context.spec.cameras).unwrap_or_else(|err| {
                panic!("写产物 {} 失败：{err}", path.display());
            });
            (payload, false, bytes)
        }
    };

    let millis = started.elapsed().as_millis() as u64;
    let (vertices, triangles) = match &payload {
        Payload::Mesh(mesh) => (mesh.vertices(), mesh.triangles()),
        _ => unreachable!(),
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

/// 体积网格的键**不**带评审相机：相机是「怎么看」，而体积没人看（渲染器只读 mesh）。
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

/// 粗场 → 3D 标量网格。体积只按内容进缓存（参数 + 输入键），不掺相机。
pub fn volume_node<Op: VolumeOp>(name: &str, inputs: &[&Artifact]) -> Artifact {
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
            let volume = Op::bake(&params, &borrowed);
            let payload = Payload::Volume(volume);
            let bytes = write_artifact(&path, name, &payload, &[]).unwrap_or_else(|err| {
                panic!("写产物 {} 失败：{err}", path.display());
            });
            (payload, false, bytes)
        }
    };
    let volume = match &payload {
        Payload::Volume(volume) => volume.clone(),
        _ => unreachable!(),
    };

    let millis = started.elapsed().as_millis() as u64;
    let stats = volume_stats(&volume);

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
        "{} {:<12} {:<16} v{}  {}  {:>5} ms  {:>9} B  值域 {:.4}..{:.4} 均 {:.4}  {} 面 {}×{}×{} 层",
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
        PATCHES,
        volume.res,
        volume.res,
        volume.layers,
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

/// 3D 标量网格 → 等值面网格。输入是体积产物的**键** ⇒ 只改等值面参数时体积照命中。
pub fn surface_node<Op: IsosurfaceOp>(name: &str, inputs: &[&Artifact]) -> Artifact {
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
    let key = key_with_cameras(key, &context.spec.cameras);
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
            let volume = inputs[0].volume();
            let sampler = VolumeGrid::new(volume);
            let mesh = Op::surface(&params, &sampler)
                .unwrap_or_else(|err| panic!("{name} 出等值面失败：{err}"));
            let payload = Payload::Mesh(mesh);
            let bytes = write_artifact(&path, name, &payload, &context.spec.cameras).unwrap_or_else(
                |err| {
                    panic!("写产物 {} 失败：{err}", path.display());
                },
            );
            (payload, false, bytes)
        }
    };

    let millis = started.elapsed().as_millis() as u64;
    let (vertices, triangles) = match &payload {
        Payload::Mesh(mesh) => (mesh.vertices(), mesh.triangles()),
        _ => unreachable!(),
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
        "{} {:<12} {:<16} v{}  {}  {:>5} ms  {:>9} B  {} 顶点 / {} 三角形",
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


