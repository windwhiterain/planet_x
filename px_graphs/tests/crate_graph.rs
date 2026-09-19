//! 依赖门：**图脚本与图库不许静态依赖任何算子**。
//!
//! 这一轮的全部意义就是那条边界：算子住在 `px_*_op` 的 dylib 里，运行时按描述符表装载。
//! 一旦有人把它写进 `[dependencies]`（而不是 `[dev-dependencies]`），算子代码就会被链进
//! 图程序的 exe —— 改一个算子要重编全部图程序，而**任何一层都不会报错**。
//! 语言管不住这件事，所以用门看住（与 `px_protocol/tests/crate_graph.rs` 同一条思路）。

use std::path::{Path, PathBuf};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graphs 必须住在 workspace 下")
        .to_path_buf()
}

fn manifest_of(name: &str) -> toml::Value {
    let path = workspace().join(name).join("Cargo.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
    toml::from_str(&text).unwrap_or_else(|err| panic!("{} 不是合法 TOML：{err}", path.display()))
}

/// 一张表的键里有没有算子 crate。
fn op_dependencies(manifest: &toml::Value, table: &str) -> Vec<String> {
    manifest
        .get(table)
        .and_then(|value| value.as_table())
        .map(|entries| {
            entries
                .keys()
                .filter(|name| name.starts_with("px_") && name.ends_with("_op"))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_graph_scripts_do_not_link_operators_statically() {
    let manifest = manifest_of("px_graphs");
    let linked = op_dependencies(&manifest, "dependencies");
    assert!(
        linked.is_empty(),
        "px_graphs 的 [dependencies] 里出现了算子：{} —— 算子必须走 dylib（[dev-dependencies] 只为把库编出来）",
        linked.join(" / "),
    );
    let dev = op_dependencies(&manifest, "dev-dependencies");
    assert!(
        !dev.is_empty(),
        "px_graphs 的 [dev-dependencies] 里一个算子都没有：测试就编不出算子库，端到端那条路会假装通过"
    );
}

#[test]
fn the_graph_library_does_not_link_operators_statically() {
    let manifest = manifest_of("px_graph");
    let linked = op_dependencies(&manifest, "dependencies");
    assert!(
        linked.is_empty(),
        "px_graph 的 [dependencies] 里出现了算子：{} —— 驱动只认描述符表，不认算子",
        linked.join(" / "),
    );
}

#[test]
fn the_schemas_do_not_link_operators() {
    for name in [
        "px_graph_schema",
        "px_field_schema",
        "px_volume_schema",
        "px_mesh_schema",
    ] {
        let manifest = manifest_of(name);
        let linked = op_dependencies(&manifest, "dependencies");
        assert!(
            linked.is_empty(),
            "{name} 的 [dependencies] 里出现了算子：{}",
            linked.join(" / "),
        );
    }
}
