use std::collections::BTreeMap;
use std::path::PathBuf;

use px_ops::{GraphSpec, ManifestEntry};
use px_protocol::scene::{Member, Part, SCENE_SCHEMA, SceneSpec, Value};
use serde::Deserialize;

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_ops::noise::fnv1a(include_str!("scene.rs"));
const DEFAULT_SCENE: &str = "orbit";

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

fn value_of(part: &str, key: &str, raw: &toml::Value) -> Value {
    match raw {
        toml::Value::Integer(number) => Value::Num(*number as f64),
        toml::Value::Float(number) => Value::Num(*number),
        toml::Value::String(text) => Value::Text(text.clone()),
        toml::Value::Array(items) if items.len() == 3 => {
            let mut out = [0.0_f32; 3];
            for (slot, item) in out.iter_mut().zip(items.iter()) {
                let number = item
                    .as_float()
                    .or_else(|| item.as_integer().map(|value| value as f64))
                    .unwrap_or_else(|| {
                        panic!("part '{part}' 参数 '{key}' 的三个数里有一个不是数：{item:?}")
                    });
                *slot = number as f32;
            }
            Value::Triple(out)
        }
        other => panic!(
            "part '{part}' 参数 '{key}' 的类型不认识：{other:?}（只认 数 / 文本 / 三个数）"
        ),
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

    let cameras = match file.cameras.as_deref() {
        Some("review") | None => px_ops::cameras::review(),
        Some(other) => panic!("不认识的相机表 '{other}'（现在只有 review）"),
    };

    let parts: Vec<Part> = file
        .parts
        .iter()
        .map(|part| {
            let members = part
                .members
                .iter()
                .map(|(role, reference)| (role.clone(), member_of(part, role, reference)))
                .collect();
            let params = part
                .params
                .iter()
                .map(|(key, raw)| (key.clone(), value_of(&part.id, key, raw)))
                .collect();
            Part {
                id: part.id.clone(),
                kind: part.kind.clone(),
                shader: part.shader.clone(),
                members,
                params,
            }
        })
        .collect();

    let spec = SceneSpec {
        schema: SCENE_SCHEMA,
        name: file.name.clone(),
        ambient: file.ambient,
        cameras,
        parts,
    };
    spec.check().unwrap_or_else(|err| panic!("场景不成形：{err}"));

    let spec_json = serde_json::to_string(&spec).unwrap_or_else(|err| panic!("{err}"));
    let member_keys = spec
        .members()
        .iter()
        .map(|member| member.key.clone())
        .collect::<Vec<_>>();
    let key = px_ops::scene_key(&spec_json, &member_keys);
    let artifact = px_protocol::scene::cas_path(&px_ops::cache_root(), &px_ops::hex(&key))
        .unwrap_or_else(|err| panic!("{err}"));
    let bytes = px_protocol::scene::write_scene(&artifact, &spec, px_ops::noise::fnv1a(&spec_json))
        .unwrap_or_else(|err| panic!("{err}"));

    println!("{}", spec.audit());
    println!("产物 scene -> {}（{}）", artifact.display(), px_ops::hex_short(&key));

    let entry = ManifestEntry {
        node: spec.name.clone(),
        op: "scene.recipe".to_string(),
        op_version: SCENE_SCHEMA,
        key: px_ops::hex(&key),
        hit: false,
        millis: 0,
        bytes,
        min: 0.0,
        max: 0.0,
        mean: spec.parts.len() as f32,
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
