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
