//! 依赖门：**动态装载的算子库不许把图库/驱动拖进来**。
//!
//! ⚠ 这一篇在「类型化算子契约」那一轮**改过口径**，原文与理由是：
//!
//! 老口径（§159）：图脚本的 `[dependencies]` 里不许出现任何 `px_*_op`。
//! 那时算子与图脚本之间只有**字符串 op_id + 字节**，静态依赖就等于把算子实现链进 exe，
//! 而且「改算子不重编图程序」那条性质会静默丢掉。
//!
//! 新口径（这一轮）：**图脚本允许静态链接 `px_*_op` 的 rlib** —— 那是拿回
//! 「参数类型 / 输入个数 / 输出域在编译期」的代价与手段（`px_cook` 的类型化契约）。
//! 判据是产物**逐字节不变**（14/14，见 `.agents/notes/art/17-*.md`）。
//!
//! 但两条**更硬**的线补了上来，它们替代了老门守的东西：
//!
//! 1. **动态装载的那一半（`crate-type` 含 dylib/cdylib）不许依赖 `px_graph`**。
//!    否则运行时被装载进图程序的 dylib 会把驱动、CAS、清单再链一份 ——
//!    那是比"改算子重编图程序"严重得多的病（同一个进程里两份驱动状态）。
//! 2. **图库本体 `px_graph` 仍然不许静态依赖任何算子**：它只认 `Cache` 那几个方法，
//!    不认识任何算子的类型。这条一位没动。
//!
//! 语言管不住这两条（写进 Cargo.toml 就生效、任何一层都不会报错），所以用门看住。

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

fn dependencies(manifest: &toml::Value) -> Vec<String> {
    manifest
        .get("dependencies")
        .and_then(|value| value.as_table())
        .map(|entries| entries.keys().cloned().collect())
        .unwrap_or_default()
}

/// `[lib] crate-type` —— 是不是"运行时被装载的那一半"。
fn is_dynamic(manifest: &toml::Value) -> bool {
    manifest
        .get("lib")
        .and_then(|lib| lib.get("crate-type"))
        .and_then(|value| value.as_array())
        .map(|kinds| {
            kinds.iter().any(|kind| {
                matches!(kind.as_str(), Some("dylib") | Some("cdylib"))
            })
        })
        .unwrap_or(false)
}

/// 所有 `px_*_op`（动态装载的那一半）。
const LOADED_OPS: [&str; 3] = ["px_field_op", "px_volume_op", "px_mesh_op"];

#[test]
fn the_loaded_operator_libraries_do_not_pull_in_the_graph_library() {
    for name in LOADED_OPS {
        let manifest = manifest_of(name);
        assert!(
            is_dynamic(&manifest),
            "{name} 不是动态装载的那一半（`[lib] crate-type` 里没有 dylib/cdylib）\
             —— 那它就不再是「改实现不必重编图程序」的那一支，本门失去意义"
        );
        let deps = dependencies(&manifest);
        assert!(
            !deps.iter().any(|dep| dep == "px_graph"),
            "{name} 依赖了 px_graph：运行时装载进图程序会让**同一个进程里出现两份驱动**\
             （CAS / 清单 / 索引各一份）。算子的边界只许是 schema 与载荷。实际依赖：{}",
            deps.join(" / "),
        );
    }
}

#[test]
fn the_graph_library_does_not_link_operators_statically() {
    let manifest = manifest_of("px_graph");
    let linked = op_dependencies(&manifest, "dependencies");
    assert!(
        linked.is_empty(),
        "px_graph 的 [dependencies] 里出现了算子：{} —— 驱动只认描述符表与 `Cache`，不认算子类型",
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

/// 图脚本仍然**必须**能静态拿到类型化的契约（这是这一轮换来的东西，别被回退掉）。
#[test]
fn the_graph_scripts_can_statically_reach_the_typed_contract() {
    let manifest = manifest_of("px_graphs");
    let deps = dependencies(&manifest);
    assert!(
        deps.iter().any(|dep| dep == "px_cook"),
        "px_graphs 不再依赖 px_cook：类型化契约（参数类型 / 输入个数 / 输出域）又回到运行期了",
    );
    let dev = op_dependencies(&manifest, "dev-dependencies");
    assert!(
        !dev.is_empty(),
        "px_graphs 的 [dev-dependencies] 里一个算子都没有：测试就编不出算子库，端到端那条路会假装通过"
    );
}
