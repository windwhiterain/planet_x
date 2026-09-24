//! §28.2 那条门的**后继**：算子的源码指纹必须由构建脚本自动算出来。
//!
//! 病根原先是：`const SOURCE_HASH: u64 = fnv1a(include_str!("fbm.rs"))` 只哈希算子自己那个
//! 文件 ⇒ 改 `field.rs` 的方向约定、`noise.rs` 的噪声时，陈旧命中**一声不响**。
//!
//! 后来那条病根换了个形状：清单由**人手维护**（每个算子一段 `include_str![…]`）⇒
//! 加一个共享文件忘了补进去，错误原样复发。
//!
//! 现在的口径：**清单不存在**。指纹算法是一个真 crate（`px_fingerprint`，rlib），
//! 每个算子的 `build.rs` 一行 `px_fingerprint::cargo_fingerprint_for_crate(&[])`；它遍历
//! "这个 crate 编译进去的全部源码"——自己 `src/` + 可达的 path 依赖的 `src/`（**传递闭包**）+ 本文件。
//!
//! ⚠ 从"一段被 `include!` 的源码"变成一个 crate，是两个原因叠出来的：
//!   ① 运行期也要算同一套名册（泛型实例的 key 要覆盖 alg crate，见 `docs/system/generic-instances.md` §177）；
//!   ② 一段被抄进各处 build.rs 的源码，改它要改 N 处 —— 那本身就是可漏的清单。
//!
//! 所以这道门守三件事：
//!
//! 1. **每个有身份的 crate 都有那个 `build.rs`**，且它调的是**共享 rlib** 的入口；
//! 2. **一个 `include!` 都不许有** —— 共享助手不再是"抄进来的源码"；
//! 3. **没有任何算子再手列源码清单**（`include_str!`）—— 那正是"忘了补一份"这条病的载体。

use std::path::{Path, PathBuf};

/// 有指纹脚本的 crate：三个实现库 + **契约层** + **三个声明所在 crate**（各要给 `DECL_HASH`）
/// + **图程序自己**（给图侧现写算子与泛型实例当身份）。
///
/// ⚠ 契约那一份指纹不是为了进键（它不进键），而是**装载时握手**用的：
///   实现库导出"我编的时候契约是哪一份"，两边对不上就当场拒。
const FINGERPRINTED_CRATES: &[&str] = &[
    "px_field_op",
    "px_volume_op",
    "px_mesh_op",
    "px_graph_schema",
    "px_field_schema",
    "px_volume_schema",
    "px_mesh_schema",
    "px_graphs",
];

/// 声明文件：**不许**再出现手列的源码清单。
const DECLARATIONS: &[&str] = &[
    "px_field_schema/src/ops.rs",
    "px_volume_schema/src/ops.rs",
    "px_mesh_schema/src/ops.rs",
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
        // 身份也不该有手写的版本号：接口哈希从类型名推。
        assert!(
            !text.contains("VERSION"),
            "{relative} 里又有手写的 `VERSION` 了 —— 接口形状哈希已经取代它。",
        );
    }
}

/// **每个实现库都必须有指纹脚本**：没有它就没有身份符号，装载时第一个撞上。
///
/// ⚠ 与 `px_graphs/tests/ops_load.rs` 分工：那边真的去装载（跑得慢、要库在盘上），
///   这边只读文件（任何 `cargo test -p px_graph` 都跑得动）。
#[test]
fn every_implementation_library_exports_an_identity() {
    let root = workspace();
    for name in ["px_field_op", "px_volume_op", "px_mesh_op"] {
        let lib = root.join(name).join("src/lib.rs");
        let text = std::fs::read_to_string(&lib)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", lib.display()));
        assert!(
            text.contains("px_impl_lib!"),
            "{name}/src/lib.rs 没有 `px_impl_lib!()`：那一份库没有身份符号，装载会当场拒",
        );
    }
}
