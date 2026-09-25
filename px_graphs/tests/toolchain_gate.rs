//! See FINDINGS.md §13
//!
//! `-Level opt` and `-Level dev` share instance keys: the per-package `opt-level` override reaches
//! the main workspace, while instance libraries are built in their own nested workspace, so the key
//! cannot tell the two apart. The recorded toolchain can, and `px run` refuses a plan whose present
//! libraries were built by another one.
//!
//! Four cases, and the fourth is the one an implementation gets wrong: the switch must be *recoverable*
//! by rebuilding, not a permanent refusal.

use std::path::{Path, PathBuf};

use px_cook::inst::InstInfo;

fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graphs lives under the workspace")
        .join("target")
        .join("toolchain-gate")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建得了临时目录");
    dir
}

/// An instance record whose library file is real (the gate only inspects present libraries) and whose
/// sidecar carries `recorded` as its toolchain.
fn instance(dir: &Path, recorded: &str) -> InstInfo {
    let library = dir.join("px_inst__Test.dll");
    std::fs::write(
        &library,
        b"not a real library; only its presence and its sidecar are read",
    )
    .expect("写得进");
    std::fs::write(
        library.with_extension("json"),
        format!(
            "{{\"key\": \"{}\", \"op_id\": \"field.test\", \"toolchain\": \"{recorded}\"}}",
            "0".repeat(64),
        ),
    )
    .expect("写得进");
    InstInfo {
        op_id: "field.test".to_string(),
        interface: 1,
        decl_hash: "0".repeat(64),
        alg_roots: vec!["px_field_alg".to_string()],
        source: "art/inst/test.rs".to_string(),
        template: String::new(),
        library: library.display().to_string(),
    }
}

#[test]
fn a_library_from_another_toolchain_is_refused_by_name() {
    let dir = scratch("mismatch");
    let info = instance(
        &dir,
        "1111111111111111111111111111111111111111111111111111111111111111",
    );
    let err = px_graphs::assert_toolchain_matches(&[info]).expect_err("另一档编的库必须被拒");
    assert!(err.contains("field.test"), "要点名是哪个算子：{err}");
    assert!(
        err.contains("111111111111") && err.contains(&px_graphs::current_toolchain()[..12]),
        "要点名两份 toolchain：{err}"
    );
    assert!(err.contains("px build"), "要给出可执行的下一步：{err}");
}

#[test]
fn a_library_from_this_toolchain_passes() {
    let dir = scratch("current");
    let info = instance(&dir, px_graphs::current_toolchain());
    px_graphs::assert_toolchain_matches(&[info]).expect("当前档编的库要放行");
}

#[test]
fn a_library_without_a_sidecar_passes() {
    let dir = scratch("no-sidecar");
    let info = instance(
        &dir,
        "2222222222222222222222222222222222222222222222222222222222222222",
    );
    std::fs::remove_file(Path::new(&info.library).with_extension("json")).expect("删得掉");
    // Nothing is recorded, so nothing is claimed: the gate stops two *known* builds from being
    // confused, and refusing here would break every library built before sidecars existed.
    px_graphs::assert_toolchain_matches(&[info]).expect("没有记录不该被当成另一档");
}

#[test]
fn a_missing_library_is_not_this_gate_s_business() {
    let dir = scratch("absent");
    let info = instance(
        &dir,
        "3333333333333333333333333333333333333333333333333333333333333333",
    );
    std::fs::remove_file(&info.library).expect("删得掉");
    // Absence is the plan's business (`stage 1` reports a missing count and `px build` fills it);
    // this gate only speaks about libraries that are there.
    px_graphs::assert_toolchain_matches(&[info]).expect("缺库由计划那条路报，不在这里拒");
}

#[test]
fn rebuilding_clears_the_refusal() {
    let dir = scratch("recover");
    let info = instance(
        &dir,
        "4444444444444444444444444444444444444444444444444444444444444444",
    );
    assert!(
        px_graphs::assert_toolchain_matches(&[info.clone()]).is_err(),
        "先拒"
    );
    // `px build` rewrites the sidecar with the toolchain of the build that produced the library.
    let sidecar = Path::new(&info.library).with_extension("json");
    std::fs::write(
        &sidecar,
        format!(
            "{{\"key\": \"{}\", \"op_id\": \"field.test\", \"toolchain\": \"{}\"}}",
            "0".repeat(64),
            px_graphs::current_toolchain(),
        ),
    )
    .expect("写得进");
    px_graphs::assert_toolchain_matches(&[info]).expect("重编之后必须放行 —— 换档是可恢复的");
}
