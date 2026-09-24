//! 依赖门：**谁可以静态依赖谁**。
//!
//! ⚠ 这一篇跟着架构改过三轮口径，原文与理由是：
//!
//! * **老口径（§159）**：图脚本的 `[dependencies]` 里不许出现任何 `px_*_op`。
//!   那时算子与图脚本之间只有**字符串 op_id + 字节**。
//! * **中口径（类型化契约那一轮）**：图脚本**允许**静态链接 `px_*_op` 的 rlib
//!   —— 那是"参数类型 / 输入个数 / 输出域在编译期"的代价与手段。
//! * **现口径（这一轮）**：算子回到 dylib，但**不是链接期依赖**：图程序按身份**运行时装载**，
//!   于是 `[dependencies]` 里一个 `px_*_op` 都没有。这不是回到老口径（那时接口是字符串 + 字节），
//!   而是"声明住 schema、实现在 dylib、类型在编译期"三样同时成立。
//!
//! 四条**一位都不许动**的线：
//!
//! 1. **图程序不许 cargo 依赖实现库** —— 这条线一破，"改一行实现不重编图程序"立刻没了
//!    （实测：静态链 1.91 s、直接依赖 dylib 3.41 s 且 exe 被重链；运行期装载 0.44 s
//!    且图 exe **字节不变**）。
//! 2. **实现库不许依赖 `px_graph` / `px_cook`**（§162）：那会把驱动与门面链进 dylib，
//!    一个进程里就有两份驱动。
//! 3. **`px_graph` 不许依赖任何算子**：它只认 `Cache` 那几个方法。
//! 4. **schema 层不许依赖算子**：它们是算子与驱动共用的**数据**层。
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

/// 一张表里有没有算子 crate。
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

const OPS: [&str; 5] = [
    "px_field_op",
    "px_volume_op",
    "px_mesh_op",
    "px_nurbs_op",
    "px_nurbs_gpu_op",
];

/// 算子库是**运行时装载**的 dylib（`px_graph_schema::ops`）。
#[test]
fn operator_libraries_are_dynamic_libraries() {
    for name in OPS {
        assert!(
            declares_dynamic(&manifest_of(name)),
            "{name} 必须是 dylib：图程序按身份在运行期装载它的符号（`px_op!` 声明的那个）"
        );
    }
}

/// ⚠ **这条就是"改一行实现不重编图程序"**：图程序里一个实现库都不许出现。
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

/// 实现库只许链**契约 + 各域 schema**。链进驱动或门面 ⇒ 一个进程里两份驱动（§162）。
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

/// ⚠ 图程序**也不许**依赖 alg crate（`px_*_alg`）。
///
/// 那是"实现体"那一半：依赖了就等于把「改泛型算法 ⇒ 重编图程序」请回来 ——
/// 而内容寻址的泛型实例（`21-codegen-types.md`：`inst_recipe.rs` 那张表 + 生成物）的全部意义
/// 正是不让它发生。图程序只在表里声明实例，算法由 `px build` 在**另一个 workspace** 里编成实例库。
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
