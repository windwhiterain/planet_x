use std::path::Path;

use px_ops::{GraphSpec, ManifestEntry};

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_ops::noise::fnv1a(include_str!("shaders.rs"));

const SLOTS: [&str; 3] = ["clouds", "atmosphere", "surface"];

fn main() {
    px_ops::begin(GraphSpec {
        name: "shaders".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 0,
        height: 0,
        projection: px_ops::field::Projection::Cube,
        cameras: Vec::new(),
    });

    // 模块表读一次就够：三个入口共用同一批库（`planet_x::common / light / noise`）。
    let modules = px_shader::workspace_modules(&px_ops::workspace_root())
        .unwrap_or_else(|err| panic!("{err}"));

    let mut entries: Vec<ManifestEntry> = Vec::new();
    for slot in SLOTS {
        let path = Path::new("art").join("shaders").join(format!("{slot}.wgsl"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        // include 闭包进键（§17.1、§52.3）：改一个被 import 的模块也得换键，否则
        // 键不动、场景键不动、槽版本不动，而画出来的东西变了。
        let closure = px_shader::closure(&text, &modules);
        let (key, artifact, bytes) =
            px_ops::write_shader(slot, &text, &closure).unwrap_or_else(|err| panic!("{err}"));
        println!(
            "产物 {slot} -> {}（{}，{} 字节 WGSL）",
            artifact.display(),
            px_ops::hex_short(&key),
            text.len()
        );
        println!("  {}", closure.summary());
        entries.push(ManifestEntry {
            node: slot.to_string(),
            op: "shader.wgsl".to_string(),
            op_version: px_ops::SHADER_VERSION,
            key: px_ops::hex(&key),
            hit: false,
            millis: 0,
            bytes,
            min: 0.0,
            max: 0.0,
            mean: text.len() as f32,
        });
    }

    let manifest = px_ops::write_graph_manifest("shaders", &entries)
        .unwrap_or_else(|err| panic!("写清单失败：{err}"));
    println!(
        "共 {} 份 shader：{}；清单 {}",
        entries.len(),
        entries
            .iter()
            .map(|entry| format!("{}={}", entry.node, &entry.key[..12]))
            .collect::<Vec<_>>()
            .join(" "),
        manifest.display()
    );
}
