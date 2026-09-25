//! See docs/invariants.md
//!
//! The roster recursion follows path dependencies, so `px_graphs`'s roster carries
//! `px_graph`'s driver even though nothing in a shipped graph uses `px_local_op!`
//! today. This gate turns that reachability from "read the code and know it" into
//! "the gate watches it": the first shipped local operator makes every byte of
//! `px_graph/src/driver.rs` rotate that node's key (same shape as the `px_cook`
//! / `px_decls` warning).

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

/// `collect_tree` skips files whose *name* looks like a test file, but it walks the filesystem
/// rather than the module tree — so such a file under `src/` would be compiled (once declared with
/// `mod`) while staying invisible to every fingerprint. That is "same key, different content", the
/// failure mode identity exists to prevent. No such file may exist; see docs/invariants.md and
/// FINDINGS §12 for why this is gated by name rather than fixed in the collector.
#[test]
fn no_compilable_source_is_named_like_a_test_file() {
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
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let source = name.ends_with(".rs") || name.ends_with(".wgsl");
                if source && (name.contains("_test") || name.starts_with("test_")) {
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
        "这些文件会被 collect_tree 按名字跳过，因而**不进任何指纹名册**：{offenders:?}\n  \
         ⇒ 声明它们的模块照样被编译进产物 ⇒ 改它换行为不换键（同键、不同内容）。\n  \
         判据请放 `tests/` 目录；`*_test.rs` / `test_*.rs` 这个名字的含义是\u{201c}被编译但对身份不可见\u{201d}。"
    );
}
