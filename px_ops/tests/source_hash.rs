//! §28.2 的门：算子的 `SOURCE_HASH` 必须覆盖它的**共享依赖**。
//!
//! 病根：`const SOURCE_HASH: u64 = fnv1a(include_str!("fbm.rs"))` 只哈希算子自己那个文件
//! ⇒ 改 `field.rs` 的方向约定、`noise.rs` 的噪声时，§19.1 那条「源码变了但版本仍是 N」
//! 的告警**一声不响**，而缓存照旧命中 —— 那份告警存在的唯一理由就是拦住这种陈旧命中。
//!
//! 语言管不住这件事（单文件版照样编译、照样跑），所以用门看住（与 §10.2 的 crate 图门同一条思路）。
//! 门只查**形状**：用了 `fnv1a_sources`、至少两段、点到了这几份文件。
//! 「语义上到底依赖谁」是人写在每个算子那一行里的表 —— 门保证它不会悄悄退回单文件版。

use std::path::{Path, PathBuf};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_ops 必须住在 workspace 下")
        .to_path_buf()
}

/// 每个算子：它的文件 → 那一行 `SOURCE_HASH` 里必须出现的 `include_str!` 路径。
fn operators() -> Vec<(&'static str, Vec<&'static str>)> {
    let shared = vec!["\"../field.rs\"", "\"../noise.rs\""];
    let mut operators = Vec::new();
    for name in [
        "constant",
        "cubesphere",
        "fbm",
        "gradient",
        "mix",
        "remap",
        "ridged",
        "warp",
    ] {
        operators.push((name, shared.clone()));
    }
    operators
}

#[test]
fn every_operator_source_hash_covers_its_shared_dependencies() {
    let root = workspace();
    let mut checked = 0_usize;

    for (name, deps) in operators() {
        let path = root.join("px_ops").join("src").join("ops").join(format!("{name}.rs"));
        assert_operator_hash(&path, &deps, &format!("ops/{name}.rs"));
        checked += 1;
    }

    // 另外两个「算子」不在 px_ops 里，但走的是同一个告警机制，所以同样要有共享依赖。
    assert_operator_hash(
        &root.join("px_mc").join("src").join("lib.rs"),
        &["\"../../px_ops/src/volume.rs\""],
        "px_mc/src/lib.rs",
    );
    checked += 1;
    assert_operator_hash(
        &root.join("px_graphs").join("src").join("cloud_proxy.rs"),
        &[
            "\"../../px_ops/src/field.rs\"",
            "\"../../px_ops/src/volume.rs\"",
            "\"../../px_verify/src/cloud_field.rs\"",
        ],
        "px_graphs/src/cloud_proxy.rs",
    );
    checked += 1;

    assert!(checked > 0, "一个算子都没检查到");
    println!("SOURCE_HASH 覆盖共享依赖：检查了 {checked} 个算子");
}

fn assert_operator_hash(path: &Path, deps: &[&str], label: &str) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
    // 取**整条语句**（到 `;` 为止），不是一行：共享依赖多的时候那一行会折行。
    let start = text
        .find("const SOURCE_HASH")
        .unwrap_or_else(|| panic!("{label} 里没有 const SOURCE_HASH"));
    let statement = &text[start..];
    let end = statement
        .find(';')
        .unwrap_or_else(|| panic!("{label} 的 SOURCE_HASH 没有以 `;` 收尾"));
    let statement = &statement[..end];
    assert!(
        statement.contains("fnv1a_sources(&["),
        "{label} 的 SOURCE_HASH 还是单文件版（§28.2）：{statement}"
    );
    let parts = statement.matches("include_str!").count();
    assert!(
        parts >= 2,
        "{label} 的 SOURCE_HASH 只哈希了 {parts} 份源码：{statement}"
    );
    for dep in deps {
        assert!(
            statement.contains(dep),
            "{label} 的 SOURCE_HASH 没点到共享依赖 {dep}：{statement}"
        );
    }
}

/// 多段哈希本身：长度前缀挡住「拼起来一样」的两种切法。
#[test]
fn the_multi_part_hash_does_not_confuse_a_split() {
    use px_ops::noise::fnv1a_sources;
    assert_ne!(
        fnv1a_sources(&["ab", "c"]),
        fnv1a_sources(&["a", "bc"]),
        "没有长度前缀的话这两种切法会撞"
    );
    assert_ne!(fnv1a_sources(&["a"]), fnv1a_sources(&["a", ""]));
    assert_ne!(
        fnv1a_sources(&["a", "b"]),
        fnv1a_sources(&["b", "a"]),
        "顺序由调用点写死：换了顺序 = 另一份列表"
    );
    assert_eq!(fnv1a_sources(&["a", "b"]), fnv1a_sources(&["a", "b"]));
}
