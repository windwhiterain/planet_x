//! See docs/invariants.md
//!
//! Two gates over one mechanism: `px_fingerprint::roster()` walks the filesystem rather than the
//! module tree, so anything it skips by name is compiled when declared yet invisible to identity —
//! "same key, different content". The skip rules below are copied from `collect_tree` deliberately:
//! a gate that invents its own stricter list drifts away from the rule it watches.

use std::path::Path;

fn slash(key: &str) -> String {
    key.replace(std::path::MAIN_SEPARATOR, "/")
}

#[test]
fn the_graphs_roster_carries_the_driver() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_fingerprint 住在 workspace 下")
        .to_path_buf();
    let roster = px_fingerprint::roster(&workspace.join("px_graphs"), &[]);
    for file in ["px_graph/src/driver.rs", "px_graphs/src/lib.rs"] {
        assert!(
            roster.keys().any(|key| slash(key).contains(file)),
            "px_graphs 的名册没递归跟到 {file}：{keys}",
            keys = roster
                .keys()
                .map(|key| slash(key))
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
}

/// Every shape `collect_tree` skips, applied inside a crate's `src/`: a directory named `tests`
/// (a `mod tests;` can resolve to `src/tests/mod.rs`), or a source file whose name contains
/// `_test` / starts with `test_`, for **both** fingerprinted extensions. Measured on this checkout:
/// all four shapes leave the enclosing roster at the same entry count, i.e. none of them is
/// visible to identity while remaining compilable.
fn skipped_by_the_collector(name: &str) -> bool {
    name == "tests" || name.starts_with('.')
}

fn test_named_source(name: &str) -> bool {
    let source = name.ends_with(".rs") || name.ends_with(".wgsl");
    source && (name.contains("_test") || name.starts_with("test_"))
}

/// §12, gated instead of fixed: hardening `collect_tree` to consult the module tree would edit a
/// crate that sits in every roster, so it costs a full-family rotation. This check buys the same
/// safety at zero key cost, because `tests/` directories are themselves outside every roster.
#[test]
fn no_compilable_source_escapes_the_fingerprint_by_name() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_fingerprint 住在 workspace 下")
        .to_path_buf();
    let mut offenders: Vec<String> = Vec::new();
    for crate_dir in std::fs::read_dir(&workspace)
        .expect("读不了 workspace")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("Cargo.toml").is_file())
    {
        let src = crate_dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("读不了目录").flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    // A skipped directory hides everything under it, so report the directory
                    // itself rather than descending to blame its contents.
                    if skipped_by_the_collector(name) {
                        offenders.push(format!(
                            "{}{} (整棵被跳过)",
                            path.strip_prefix(&workspace).unwrap_or(&path).display(),
                            std::path::MAIN_SEPARATOR
                        ));
                    } else {
                        stack.push(path);
                    }
                    continue;
                }
                if test_named_source(name) {
                    offenders.push(
                        path.strip_prefix(&workspace)
                            .unwrap_or(path.as_path())
                            .display()
                            .to_string(),
                    );
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "这些路径会被 collect_tree 按名字/目录名跳过，因而**不进任何指纹名册**：{offenders:?}\n  \
         ⇒ 声明它们的模块照样被编译进产物 ⇒ 改它换行为不换键（同键、不同内容）。\n  \
         判据请放 crate 根的 `tests/`；在 `src/` 里用 `tests` 目录或 \
         `*_test.rs` / `test_*.rs` / `*_test.wgsl` / `test_*.wgsl` 这些名字的含意是\
         \u{201c}被编译但对身份不可见\u{201d}。"
    );
}
