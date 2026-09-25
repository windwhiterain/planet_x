//! See docs/programs.md
//!
//! The walkers that gather the source bytes of an instance key must not treat an unreadable
//! directory as an empty one: a partial read is a false identity rather than a missing one
//! (FINDINGS.md #17), so the read failing is a criterion, not a detail.

use std::path::PathBuf;

use px_cook::inst_scan;

#[test]
fn a_directory_that_cannot_be_read_is_a_failure_that_names_it() {
    // A file is passed where a directory is expected: `read_dir` refuses it, which is the same
    // refusal a permission or a vanished path produces.
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut out = Vec::new();
    let err = inst_scan::collect_rs(&file, &mut out)
        .expect_err("a path that cannot be read as a directory must not look empty");
    assert_eq!(err.kind.name(), "read");
    assert!(
        err.message.contains("Cargo.toml"),
        "the unreadable path is named: {}",
        err.message
    );
    assert!(out.is_empty(), "nothing is collected from a failed read");
}
