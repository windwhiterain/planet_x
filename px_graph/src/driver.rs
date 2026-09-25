use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use px_graph_schema::{
    Cache, GraphSpec, Key, ManifestEntry, PayloadBundle, Report, fnv1a, hex, hex_short,
};
use px_protocol::stream::{self, Frame};

pub struct Graph {
    spec: GraphSpec,
    param_dir: PathBuf,
    cache_root: PathBuf,
    fresh: bool,
    manifest: Mutex<Vec<ManifestEntry>>,
    params_used: Mutex<BTreeMap<String, ParamsUsed>>,
}

impl Graph {
    pub fn params_text(&self, name: &str) -> Option<String> {
        load_params_text(&self.param_dir, name)
    }

    pub fn finish(&self) {
        let manifest = self.manifest.lock().expect("清单锁坏了");
        let hits = manifest.iter().filter(|entry| entry.hit).count();
        let cooked = manifest.len() - hits;
        let millis: u64 = manifest.iter().map(|entry| entry.millis).sum();

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
        let path = artifact_path(&self.cache_root, &report.key);
        let bytes = if report.hit {
            // A hit must not write: the artifact is already on disk and the key is a
            // hash of the payload, so re-encoding and rewriting identical bytes is
            // pure IO (measured: ~195 MB per all-hit nebula run). See docs/graph.md.
            std::fs::metadata(&path)
                .map(|meta| meta.len())
                .unwrap_or_else(|_| payload.bytes() as u64)
        } else {
            let bytes = payload
                .to_bytes(report.node)
                .map_err(|err| format!("包 {} 的产物失败：{err}", report.node))?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("建目录 {} 失败：{err}", parent.display()))?;
            }
            std::fs::write(&path, &bytes)
                .map_err(|err| format!("写产物 {} 失败：{err}", path.display()))?;
            bytes.len() as u64
        };

        let detail = report.detail.clone();
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
                bytes,
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
            bytes,
            report.detail,
        );
        Ok(())
    }
}

fn interface_version(interface: u64) -> u32 {
    (interface & 0xffff_ffff) as u32
}

fn interface_tag(interface: u64) -> String {
    format!("{:016x}", interface)[..8].to_string()
}

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

#[derive(Clone, serde::Serialize)]
struct ParamsUsed {
    op: String,
    from_file: bool,
    params: serde_json::Value,
}

pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn cache_root() -> PathBuf {
    workspace_root().join("target").join("pcg")
}

pub fn param_root() -> PathBuf {
    match std::env::var("PX_ART") {
        Ok(text) if !text.is_empty() => {
            let path = PathBuf::from(&text);
            if path.is_absolute() {
                path
            } else {
                workspace_root().join(path)
            }
        }
        _ => workspace_root().join("art"),
    }
}

pub fn apply_store_args() -> Result<(), String> {
    let (store, _) = split_store_args()?;
    let Some(store) = store else {
        return Ok(());
    };
    if store.is_empty() {
        return Err("--store 的目录是空的".to_string());
    }
    unsafe { std::env::set_var("PX_ART", &store) };
    Ok(())
}

pub fn args_without_store() -> Result<Vec<String>, String> {
    Ok(split_store_args()?.1)
}

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

pub fn begin(spec: GraphSpec) -> Graph {
    let root = workspace_root();
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

pub fn shader_key(text: &str, closure: &px_shader::Closure) -> Key {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_shader/v2");
    hasher.update(&SHADER_VERSION.to_le_bytes());
    hasher.update(&closure.fingerprint().to_le_bytes());
    hasher.update(text.as_bytes());
    *hasher.finalize().as_bytes()
}

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

fn shader_schema(id: &str, text: &str, modules: &px_shader::ModuleTable) -> Result<String, String> {
    let mut seen = Vec::new();
    let assembled = px_shader::assemble::render_source(
        text,
        modules,
        px_shader::host_stubs::wgpu_host_stub,
        &mut seen,
    );
    px_shader::reflect::reflect_assembled(&assembled, id)?.to_json()
}

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

pub struct BakedShader {
    pub slot: String,
    pub key: Key,
    pub artifact: PathBuf,
    pub bytes: u64,
    pub wgsl_bytes: usize,
    pub closure: String,
}

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
