use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use px_graphs::params::{merge_named, schema_of};
use px_protocol::scene::{Member, PassResource, PassSpec, SceneSpec};
use serde::Deserialize;

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_ops::noise::fnv1a(include_str!("passes.rs"));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassFile {
    /// 可选：换掉文档的名字（不写就沿用基准场景的名字）。
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    resources: Vec<PassResourceFile>,
    #[serde(default)]
    passes: Vec<PassFileEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassResourceFile {
    name: String,
    format: String,
    size: String,
    #[serde(default)]
    usage: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PassFileEntry {
    kind: String,
    shader: String,
    #[serde(default)]
    label: String,
    #[serde(default = "fragment_entry")]
    entry: String,
    #[serde(default)]
    reads: Vec<String>,
    writes: Vec<String>,
    /// 传给这份 pass shader 的参数：**按名字**给，按它自己声明的结构体打包。
    /// 名字不认识 / 声明了没人给 / 类型不符 —— 三档都在**烘图时**红，与材质同一条路。
    #[serde(default)]
    params: BTreeMap<String, toml::Value>,
}

fn fragment_entry() -> String {
    "fs_main".to_string()
}

fn usage() -> String {
    "用法：passes <场景产物 .pxart> <pass 配方名> [输出路径]\n\
     例：cargo run -p px_graphs --bin passes -- target/pcg/ab/xx/base.pxart invert\n\
     配方住在 art/passes/<名>.toml；它引用的 shader 由 cargo run -p px_graphs --bin shaders 烘。"
        .to_string()
}

fn main() {
    px_ops::begin(px_ops::GraphSpec {
        name: "passdoc".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 0,
        height: 0,
        projection: px_ops::field::Projection::Cube,
        cameras: Vec::new(),
    });

    let mut args = std::env::args().skip(1);
    let (Some(input), Some(recipe)) = (args.next(), args.next()) else {
        panic!("{}", usage());
    };
    let out = args.next();

    let path = PathBuf::from("art")
        .join("passes")
        .join(format!("{recipe}.toml"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不了 {}：{err}\n{}", path.display(), usage()));
    let file: PassFile = toml::from_str(&text)
        .unwrap_or_else(|err| panic!("{} 解不开：{err}", path.display()));

    let mut spec: SceneSpec = px_protocol::scene::read_scene(Path::new(&input))
        .unwrap_or_else(|err| panic!("读不了场景产物 {input}：{err}"));

    if let Some(name) = &file.name {
        spec.name = name.clone();
    }

    spec.resources = file
        .resources
        .iter()
        .map(|resource| PassResource {
            name: resource.name.clone(),
            format: resource.format.clone(),
            size: resource.size.clone(),
            usage: resource.usage.clone(),
        })
        .collect();

    let mut passes: Vec<PassSpec> = Vec::new();
    for entry in &file.passes {
        let key = px_ops::manifest_key_of("shaders", &entry.shader).unwrap_or_else(|err| {
            panic!(
                "pass 要的 shader '{}' 不在 shaders 清单里：{err}\n\
                 先跑 cargo run -p px_graphs --bin shaders",
                entry.shader
            )
        });
        let label = if entry.label.is_empty() {
            entry.shader.clone()
        } else {
            entry.label.clone()
        };
        let member = Member::new("shaders", &entry.shader, &key);
        // 参数按**这份 shader 自己的契约**透传：烘图时就把三档（名字不认识 / 声明了没人给 /
        // 类型不符）全拦下来，不等装载时才拒 —— 那时候报的是渲染器的错，离改配方已经很远。
        let layout = schema_of(&member, &px_ops::cache_root())
            .unwrap_or_else(|err| panic!("pass '{label}'：{err}"));
        let params = merge_named(
            &format!("pass '{label}'"),
            &entry.params,
            &[],
            &layout,
            BTreeMap::new(),
        )
        .unwrap_or_else(|err| panic!("{err}"));
        println!(
            "pass {label}：{}｜shader {}（{}）｜读 [{}]｜写 [{}]｜参数 {} 个{}",
            entry.kind,
            entry.shader,
            &key[..key.len().min(12)],
            entry.reads.join(" / "),
            entry.writes.join(" / "),
            params.len(),
            if params.is_empty() {
                String::new()
            } else {
                format!(
                    "（{}）",
                    params
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" / ")
                )
            },
        );
        passes.push(PassSpec {
            kind: entry.kind.clone(),
            shader: member,
            label,
            entry: entry.entry.clone(),
            reads: entry.reads.clone(),
            writes: entry.writes.clone(),
            params,
        });
    }
    spec.passes = passes;
    spec.check().unwrap_or_else(|err| panic!("这份 pass 表不成立：{err}"));

    let spec_json = serde_json::to_string(&spec).unwrap_or_else(|err| panic!("{err}"));
    let member_keys = spec
        .members()
        .iter()
        .map(|member| member.key.clone())
        .collect::<Vec<_>>();
    let key = px_ops::scene_key(&spec_json, &member_keys);
    let artifact = match out {
        Some(path) => PathBuf::from(path),
        None => px_protocol::scene::cas_path(&px_ops::cache_root(), &px_ops::hex(&key))
            .unwrap_or_else(|err| panic!("{err}")),
    };
    let bytes =
        px_protocol::scene::write_scene(&artifact, &spec, px_ops::noise::fnv1a(&spec_json))
            .unwrap_or_else(|err| panic!("{err}"));

    println!("{}", spec.audit());
    println!(
        "产物 passdoc -> {}（{}｜{} 字节）",
        artifact.display(),
        px_ops::hex_short(&key),
        bytes
    );
    println!("请求：px_render --scene {}", artifact.display());
}
