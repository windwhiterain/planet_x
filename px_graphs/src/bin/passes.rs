use std::path::{Path, PathBuf};

use px_protocol::scene::{Member, PassResource, PassSpec, SceneSpec};
use serde::Deserialize;

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_ops::noise::fnv1a(include_str!("passes.rs"));

#[derive(Deserialize)]
struct PassFile {
    #[serde(default)]
    resources: Vec<PassResourceFile>,
    #[serde(default)]
    passes: Vec<PassFileEntry>,
}

#[derive(Deserialize)]
struct PassResourceFile {
    name: String,
    format: String,
    size: String,
    #[serde(default)]
    usage: Vec<String>,
}

#[derive(Deserialize)]
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
        println!(
            "pass {label}：{}｜shader {}（{}）｜读 [{}]｜写 [{}]",
            entry.kind,
            entry.shader,
            &key[..key.len().min(12)],
            entry.reads.join(" / "),
            entry.writes.join(" / "),
        );
        passes.push(PassSpec {
            kind: entry.kind.clone(),
            shader: Member::new("shaders", &entry.shader, &key),
            label,
            entry: entry.entry.clone(),
            reads: entry.reads.clone(),
            writes: entry.writes.clone(),
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
