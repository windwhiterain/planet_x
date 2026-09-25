use std::path::{Path, PathBuf};

const FINGERPRINTED_CRATES: &[&str] = &[
    "px_field_op",
    "px_volume_op",
    "px_mesh_op",
    "px_nurbs_op",
    "px_nurbs_gpu_op",
    "px_graph_schema",
    "px_field_schema",
    "px_volume_schema",
    "px_mesh_schema",
    "px_nurbs_schema",
    "px_field_alg",
    "px_volume_alg",
    "px_elem",
    "px_decls",
    "px_graphs",
];

const DECLARATIONS: &[&str] = &[
    "px_field_schema/src/ops.rs",
    "px_volume_schema/src/ops.rs",
    "px_mesh_schema/src/ops.rs",
    "px_nurbs_schema/src/ops.rs",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graph 必须住在 workspace 下")
        .to_path_buf()
}

#[test]
fn every_fingerprinted_crate_fingerprints_its_own_sources() {
    let root = workspace();
    let shared = root.join("px_fingerprint/src/lib.rs");
    assert!(
        shared.is_file(),
        "共享的指纹助手不在了：{}（各 build.rs 都调它，运行期也算它那套名册）",
        shared.display(),
    );

    for name in FINGERPRINTED_CRATES {
        let build = root.join(name).join("build.rs");
        let text = std::fs::read_to_string(&build)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", build.display()));
        assert!(
            text.contains("px_fingerprint::cargo_fingerprint_for_crate"),
            "{name}/build.rs 没有调共享 rlib 的指纹入口 —— 自己写一套就等于又有了可漏的清单",
        );
        assert!(
            !text.contains("include!"),
            "{name}/build.rs 还在 `include!` 一份源码：共享助手现在是 **crate**（`px_fingerprint`），\
             抄进来就又变成 N 处要同步的副本",
        );
    }
}

#[test]
fn nobody_hand_lists_operator_sources_anymore() {
    let root = workspace();
    for relative in DECLARATIONS {
        let path = root.join(relative);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        assert!(
            !text.contains("include_str!"),
            "{relative} 里又有手列的源码清单了 —— 那正是「忘了补一份 ⇒ 陈旧命中」这条病的载体。\
             源码指纹由 build.rs 遍历源码树算，不用列。",
        );
        assert!(
            !text.contains("VERSION"),
            "{relative} 里又有手写的 `VERSION` 了 —— 接口形状哈希已经取代它。",
        );
    }
}

#[test]
fn every_implementation_library_exports_an_identity() {
    let root = workspace();
    for name in [
        "px_field_op",
        "px_volume_op",
        "px_mesh_op",
        "px_nurbs_op",
        "px_nurbs_gpu_op",
    ] {
        let lib = root.join(name).join("src/lib.rs");
        let text = std::fs::read_to_string(&lib)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", lib.display()));
        assert!(
            text.contains("px_impl_lib!"),
            "{name}/src/lib.rs 没有 `px_impl_lib!()`：那一份库没有身份符号，装载会当场拒",
        );
    }
}
