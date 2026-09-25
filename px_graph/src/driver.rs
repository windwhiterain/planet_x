use std::collections::BTreeMap;
use std::io::Write;
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
    /// A graph whose cache directory is given rather than derived from the workspace, so a gate can
    /// drive a whole run into a throwaway directory. `param_dir` comes from the spec's name, exactly
    /// as `begin` does it.
    pub fn with_cache_root(spec: GraphSpec, cache_root: PathBuf) -> Graph {
        let param_dir = param_root().join(&spec.name);
        Graph {
            param_dir,
            cache_root,
            fresh: false,
            manifest: Mutex::new(Vec::new()),
            params_used: Mutex::new(BTreeMap::new()),
            spec,
        }
    }

    /// Records one finished node in this run's manifest. The graph side calls this with the composed
    /// label (`<graph>::<node>`), which is why the label is opaque here: the graph already owns the
    /// label, and writing it in two places would mean two sources for the ledger's `node` field.
    pub fn record(&self, entry: ManifestEntry) {
        self.manifest.lock().expect("清单锁坏了").push(entry);
    }

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

        // The measurement ledger, written last and non-fatally: a bake that finished has finished,
        // and the recording of it is a separate concern from whether it happened. Appending here (and
        // not in `store`) keeps FINDINGS.md §8 intact: a hit still writes nothing but its row.
        drop(used);
        if let Err(err) = append_metrics(&self.cache_root, &self.spec.name, &manifest) {
            eprintln!("⚠ 这轮的测量没记上：{err}");
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

/// The interface hash is folded into the node key at full width (`keys.rs` hashes
/// `op.interface.to_le_bytes()`), so the manifest records the same full value: recording half of it
/// would let two operators with different keys show one `op_version`.
fn interface_version(interface: u64) -> u64 {
    interface
}

/// The same number `interface_version` records, so a tag copied off the run line can be found in the
/// manifest.
fn interface_tag(interface: u64) -> String {
    format!("{interface:016x}")
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

/// One run's header line in `metrics.jsonl`. `seq` is the run key: it is read back from the last line
/// already in the file, so it survives a process and orders the file even when two runs start inside
/// the same second. `started` is the human-readable anchor only — nothing reads it to decide
/// anything.
#[derive(serde::Serialize)]
struct MetricsRun<'a> {
    seq: u64,
    graph: &'a str,
    started: String,
    node_count: usize,
}

/// One node line. `hit` is recorded even though it looks derivable from `cook_millis == 0`, because
/// "a hit rewrote its artifact" was a real defect (FINDINGS.md §8) and the difference between "read
/// from the CAS" and "cooked in no time" is what made it findable.
#[derive(serde::Serialize)]
struct MetricsNode<'a> {
    seq: u64,
    node: &'a str,
    key: &'a str,
    hit: bool,
    cook_millis: u64,
    bytes: u64,
}

/// The measurement ledger for one graph: append-only JSONL under the graph's cache directory. It is
/// **not** part of identity — nothing under `target/` is in any roster — and it must not be, or
/// recording a measurement would invalidate the thing it measured (docs/backlog.md).
pub fn metrics_path(cache_root: &Path, graph: &str) -> PathBuf {
    cache_root.join(graph).join("metrics.jsonl")
}

/// Rotation limits: whichever is reached first sends the current file to `<name>.1` and starts a new
/// one. `seq` keeps counting across the rename, so a reader never has to merge two sequences.
const METRICS_MAX_BYTES: u64 = 256 * 1024;
const METRICS_MAX_LINES: u64 = 4096;

/// Renames the ledger to `<name>.jsonl.1` when it has outgrown either limit. `append_metrics` is the
/// only caller, so the thresholds are driven through that path rather than exposed.
fn rotate_if_large(path: &Path) -> Result<(), String> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("看不了 {}：{err}", path.display())),
    };
    let lines = std::fs::read_to_string(path)
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count() as u64)
        .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
    if meta.len() <= METRICS_MAX_BYTES && lines <= METRICS_MAX_LINES {
        return Ok(());
    }
    let rotated = path.with_extension("jsonl.1");
    std::fs::rename(path, &rotated).map_err(|err| {
        format!(
            "轮转不了（{} → {}）：{err}",
            path.display(),
            rotated.display()
        )
    })?;
    println!(
        "测量账本轮转：{} → {}（上一份已满）",
        path.display(),
        rotated.display()
    );
    Ok(())
}

/// Appends one run (header + one line per node) to the graph's `metrics.jsonl`.
///
/// Every failure is reported and the run continues: telemetry is a record of what happened, not a
/// reason to fail a bake. The one thing it must not do is stay silent — a reader has to be able to
/// tell "the file was written" from "the file could not be written".
pub fn append_metrics(
    cache_root: &Path,
    graph: &str,
    manifest: &[ManifestEntry],
) -> Result<(), String> {
    let path = metrics_path(cache_root, graph);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("建不了 {}：{err}", parent.display()))?;
    }
    let seq = match last_seq(&path) {
        Ok(seq) => seq + 1,
        Err(err) => {
            eprintln!(
                "⚠ 读不回 {} 的上一轮序号（{err}）；这一轮记作 seq 1",
                path.display()
            );
            1
        }
    };
    rotate_if_large(&path)?;

    let header = MetricsRun {
        seq,
        graph,
        started: iso_utc(epoch_seconds()),
        node_count: manifest.len(),
    };
    let mut text = serde_json::to_string(&header)
        .map_err(|err| format!("这一轮的 run 头序列化不了：{err}"))?;
    text.push('\n');
    for entry in manifest {
        let line = MetricsNode {
            seq,
            node: &entry.node,
            key: &entry.key,
            hit: entry.hit,
            cook_millis: entry.millis,
            bytes: entry.bytes,
        };
        text.push_str(
            &serde_json::to_string(&line)
                .map_err(|err| format!("{} 这一行的测量序列化不了：{err}", entry.node))?,
        );
        text.push('\n');
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("开不了 {}：{err}", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|err| format!("往 {} 追加不了：{err}", path.display()))?;
    Ok(())
}

/// The `seq` of the last line in the file that carries one: 0 when the file is absent or holds no
/// such line, which is also what makes the next run start at 1.
///
/// A line that will not parse is reported rather than skipped quietly, because the alternative —
/// treating it as "no previous run" — resets the sequence, and a sequence that restarts is a
/// sequence a reader cannot order.
fn last_seq(path: &Path) -> Result<u64, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.to_string()),
    };
    let mut broken = 0_u32;
    for line in text.lines().rev() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(line) {
            Ok(value) => match value.get("seq").and_then(|seq| seq.as_u64()) {
                Some(seq) => {
                    if broken > 0 {
                        eprintln!(
                            "⚠ {} 里有 {broken} 行读不了（跳过它们，用能读的那一行）",
                            path.display()
                        );
                    }
                    return Ok(seq);
                }
                None => broken += 1,
            },
            Err(_) => broken += 1,
        }
    }
    if broken > 0 {
        eprintln!(
            "⚠ {} 里有 {broken} 行读不了、也没有能读的 seq ⇒ 这一轮从 seq 1 起",
            path.display()
        );
    }
    Ok(0)
}

fn epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time. The repository carries no date library and this is the only
/// place that needs one, so this is the standard civil-from-days arithmetic instead of a dependency.
fn iso_utc(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60,
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
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
    let cache_root = workspace_root().join("target").join("pcg");
    let fresh = std::env::var("PX_PCG_FRESH")
        .map(|value| value != "0")
        .unwrap_or(false);
    let cached = cache_root.join(&spec.name).join("manifest.json").is_file() as usize;

    let graph = Graph::with_cache_root(spec, cache_root);
    let graph = Graph { fresh, ..graph };
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
            op_version: u64::from(SHADER_VERSION),
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
