//! 依赖门：**谁可以静态依赖谁**。
//!
//! ⚠ 这一篇跟着架构改过两轮口径，原文与理由是：
//!
//! * **老口径（§159）**：图脚本的 `[dependencies]` 里不许出现任何 `px_*_op`。
//!   那时算子与图脚本之间只有**字符串 op_id + 字节**。
//! * **中口径（类型化契约那一轮）**：图脚本**允许**静态链接 `px_*_op` 的 rlib
//!   —— 那是"参数类型 / 输入个数 / 输出域在编译期"的代价与手段。同时补了
//!   "动态装载的那一半不许依赖 `px_graph`"这条更硬的线。
//! * **现口径（这一轮）**：**动态装载那一半整个删掉了**。算子就是图程序静态链进来的
//!   一个 Rust 库，没有描述符、没有 `OpTable`、没有 loader、没有生成的实例 dylib。
//!   于是"中口径"那条 dylib 门也没有对象了 —— 但它的**精神**留着，换成一条新的：
//!   **没有哪个算子库再声明 `dylib`/`cdylib`**，否则"运行时装载"会悄悄长回来。
//!
//! 两条一位没动的线：
//!
//! 1. **`px_graph` 仍然不许静态依赖任何算子**：它只认 `Cache` 那几个方法。
//! 2. **schema 层不许依赖算子**。
//!
//! 语言管不住这些（写进 Cargo.toml 就生效、任何一层都不会报错），所以用门看住。

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

/// `[lib] crate-type` 里有没有 `dylib` / `cdylib`。
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

const OPS: [&str; 3] = ["px_field_op", "px_volume_op", "px_mesh_op"];

/// 算子库是**静态 Rust 库**。谁再声明 `dylib`/`cdylib`，就说明"运行时按描述符装载"
/// 那一套正在长回来 —— 那正是这一轮删掉的东西。
#[test]
fn operator_libraries_are_plain_static_libraries() {
    for name in OPS {
        let manifest = manifest_of(name);
        assert!(
            !declares_dynamic(&manifest),
            "{name} 又声明了 dylib/cdylib：算子现在应该是图程序**静态链**进来的一个 Rust 库。\
             动态装载那一套（描述符表 / OpTable / loader / 生成的实例）已经删掉了"
        );
    }
}

/// `px_graph` 只认 `Cache`，不认识任何算子的类型。
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

/// schema 层不许依赖算子（它们是算子与驱动共用的**数据**层）。
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

/// 图脚本**必须**能静态拿到类型化的契约（这是那一轮换来的东西，别被回退掉）。
///
/// ⚠ 它同时钉住"算子在图脚本的 `[dependencies]` 里"—— 从前算子只在
/// `[dev-dependencies]` 里（那时图脚本靠字符串 id 接算子，只需要测试时把 dylib 编出来）。
#[test]
fn the_graph_scripts_statically_reach_the_typed_contract() {
    let manifest = manifest_of("px_graphs");
    let deps = dependencies(&manifest);
    assert!(
        deps.iter().any(|dep| dep == "px_cook"),
        "px_graphs 不再依赖 px_cook：类型化契约（参数类型 / 输入个数 / 输出域）又回到运行期了",
    );
    let linked = op_dependencies(&manifest, "dependencies");
    assert_eq!(
        linked.len(),
        OPS.len(),
        "px_graphs 的 [dependencies] 里应当有三个算子（它们是图脚本静态链接的库）：{:?}",
        linked,
    );
    assert!(
        manifest.get("dev-dependencies").is_none(),
        "px_graphs 又有 [dev-dependencies] 了：那是「只为把 dylib 编出来」那一套的残留",
    );
}
