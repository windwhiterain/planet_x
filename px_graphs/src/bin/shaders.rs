use std::path::{Path, PathBuf};

use px_graph::{GraphSpec, ManifestEntry};

const GRAPH_VERSION: u32 = 1;
const SOURCE_HASH: u64 = px_graph::fnv1a(include_str!("shaders.rs"));

/// 要烘的槽 = `art/shaders/*.wgsl` 里**每一个入口 shader**（§80 第 2 步）。
///
/// 原来是一张写死的数组（`const SLOTS: [&str; 3] = [...]`）：加一种材质要改 Rust，
/// 而「加一种材质 / 加一份 shader」正是美术要做的事 —— 那正是这一轮要拆掉的墙。
///
/// 判据「是不是入口」只有一条：**没有 `#define_import_path`**（那是模块的标记；
/// 库住 `art/shaders/lib`，规则住在 `px_shader::import_path_of`）。
/// 排在名字序上 ⇒ 同一棵树两次烘出来的清单逐字节相同。
fn entry_slots() -> Result<Vec<String>, String> {
    let dir: PathBuf = Path::new("art").join("shaders");
    let entries = std::fs::read_dir(&dir)
        .map_err(|err| format!("读不了 shader 目录 {}：{err}", dir.display()))?;
    let mut slots: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("wgsl") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("读不了 {}：{err}", path.display()))?;
        if px_shader::import_path_of(&text).is_some() {
            // 模块不是入口：库那一侧的 `.wgsl` 不在这里，扫到了也不烘。
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
    Ok(slots)
}

fn main() {
    px_graph::begin(GraphSpec {
        name: "shaders".to_string(),
        version: GRAPH_VERSION,
        source_hash: SOURCE_HASH,
        width: 0,
        height: 0,
        projection: px_protocol::art::Domain::Cube,
        cameras: Vec::new(),
    });

    // 模块表读一次就够：所有入口共用同一批库（`planet_x::common / light / noise`）。
    let modules = px_shader::workspace_modules(&px_graph::workspace_root())
        .unwrap_or_else(|err| panic!("{err}"));

    let slots = entry_slots().unwrap_or_else(|err| panic!("{err}"));
    println!("入口 shader {} 份：{}", slots.len(), slots.join(" / "));

    let mut entries: Vec<ManifestEntry> = Vec::new();
    for slot in &slots {
        let path = Path::new("art").join("shaders").join(format!("{slot}.wgsl"));
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        // include 闭包进键（§17.1、§52.3）：改一个被 import 的模块也得换键，否则
        // 键不动、场景键不动、槽版本不动，而画出来的东西变了。
        let closure = px_shader::closure(&text, &modules);
        let (key, artifact, bytes) =
            px_graph::write_shader(slot, &text, &closure, &modules).unwrap_or_else(|err| panic!("{err}"));
        println!(
            "产物 {slot} -> {}（{}，{} 字节 WGSL）",
            artifact.display(),
            px_graph::hex_short(&key),
            text.len()
        );
        println!("  {}", closure.summary());
        entries.push(ManifestEntry {
            node: slot.to_string(),
            op: "shader.wgsl".to_string(),
            op_version: px_graph::SHADER_VERSION,
            key: px_graph::hex(&key),
            hit: false,
            millis: 0,
            bytes,
            min: 0.0,
            max: 0.0,
            mean: text.len() as f32,
        });
    }

    let manifest = px_graph::write_graph_manifest("shaders", &entries)
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
