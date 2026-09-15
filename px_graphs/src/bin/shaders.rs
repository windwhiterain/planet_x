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

    let mut entries: Vec<ManifestEntry> = Vec::new();
    for slot in SLOTS {
        let path = Path::new("art").join("shaders").join(format!("{slot}.wgsl"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        let (key, artifact, bytes) =
            px_ops::write_shader(slot, &text).unwrap_or_else(|err| panic!("{err}"));
        println!(
            "产物 {slot} -> {}（{}，{} 字节 WGSL）",
            artifact.display(),
            px_ops::hex_short(&key),
            text.len()
        );
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
