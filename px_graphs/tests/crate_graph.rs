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

fn has(manifest: &toml::Value, table: &str, name: &str) -> bool {
    manifest
        .get(table)
        .and_then(|value| value.as_table())
        .map(|entries| entries.contains_key(name))
        .unwrap_or(false)
}

fn declares_dynamic(manifest: &toml::Value) -> bool {
    manifest
        .get("lib")
        .and_then(|lib| lib.get("crate-type"))
        .and_then(|value| value.as_array())
        .map(|kinds| {
            kinds
                .iter()
                .any(|kind| matches!(kind.as_str(), Some("dylib") | Some("cdylib")))
        })
        .unwrap_or(false)
}

const OPS: [&str; 5] = [
    "px_field_op",
    "px_volume_op",
    "px_mesh_op",
    "px_nurbs_op",
    "px_nurbs_gpu_op",
];

#[test]
fn operator_libraries_are_dynamic_libraries() {
    for name in OPS {
        assert!(
            declares_dynamic(&manifest_of(name)),
            "{name} 必须是 dylib：图程序按身份在运行期装载它的符号（`px_op!` 声明的那个）"
        );
    }
}

#[test]
fn the_graph_scripts_do_not_link_operator_libraries() {
    let manifest = manifest_of("px_graphs");
    let linked = op_dependencies(&manifest, "dependencies");
    assert!(
        linked.is_empty(),
        "px_graphs 又静态依赖实现库了：{} —— 那会让「改一行实现」重编重链图程序",
        linked.join(" / "),
    );
    let dev = op_dependencies(&manifest, "dev-dependencies");
    assert!(
        dev.is_empty(),
        "px_graphs 的 [dev-dependencies] 里出现了实现库：{} —— 判据也得走装载那条真路",
        dev.join(" / "),
    );
    assert!(
        has(&manifest, "dependencies", "px_cook"),
        "px_graphs 不再依赖 px_cook：图脚本唯一那扇门没了（`begin` / `cached` / 各域算子表）",
    );
}

#[test]
fn operator_libraries_do_not_link_the_driver() {
    for name in OPS {
        let manifest = manifest_of(name);
        for table in ["dependencies", "dev-dependencies"] {
            for forbidden in ["px_graph", "px_cook"] {
                assert!(
                    !has(&manifest, table, forbidden),
                    "{name} 的 [{table}] 里出现了 {forbidden}：实现库只许链契约（px_graph_schema）与各域 schema",
                );
            }
        }
    }
}

#[test]
fn the_graph_scripts_do_not_link_algorithm_libraries() {
    let manifest = manifest_of("px_graphs");
    for table in ["dependencies", "dev-dependencies"] {
        let linked: Vec<String> = manifest
            .get(table)
            .and_then(|value| value.as_table())
            .map(|entries| {
                entries
                    .keys()
                    .filter(|name| name.ends_with("_alg"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            linked.is_empty(),
            "px_graphs 的 [{table}] 里出现了 alg crate：{} —— 依赖它 = 改泛型算法要重编图程序",
            linked.join(" / "),
        );
    }
}

#[test]
fn the_graph_library_does_not_link_operators_statically() {
    let manifest = manifest_of("px_graph");
    let linked = op_dependencies(&manifest, "dependencies");
    assert!(
        linked.is_empty(),
        "px_graph 的 [dependencies] 里出现了算子：{} —— 驱动只认 `Cache`，不认算子类型",
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
        "px_nurbs_schema",
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
