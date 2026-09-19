//! §28.2 那条门的**后继**：算子的源码指纹必须由构建脚本自动算出来。
//!
//! 病根原先是：`const SOURCE_HASH: u64 = fnv1a(include_str!("fbm.rs"))` 只哈希算子自己那个
//! 文件 ⇒ 改 `field.rs` 的方向约定、`noise.rs` 的噪声时，陈旧命中**一声不响**。
//!
//! 后来那条病根换了个形状：清单由**人手维护**（每个算子一段 `include_str![…]`）⇒
//! 加一个共享文件忘了补进去，错误原样复发。
//!
//! 现在的口径：**清单不存在**。每个算子库的 `build.rs` `include!` 那份共享助手
//! （`build/fingerprint.rs`），它遍历"这个 crate 编译进去的全部源码"——
//! 自己 `src/` + `Cargo.toml` 里所有 path 依赖的 `src/` + 本文件。
//!
//! 所以这道门守两件事：
//!
//! 1. **每个算子库都有那个 `build.rs`**，而且它是 `include!` 共享助手而不是自己写一套；
//! 2. **没有任何算子再手列源码清单** —— 一旦有人把 `include_str!` 那张表加回声明里，
//!    这里就炸。（那正是"忘了补一份"这条病的载体。）

use std::path::{Path, PathBuf};

/// 装了算子的 crate：每个都该有指纹脚本。
const OPERATOR_CRATES: &[&str] = &["px_field_op", "px_volume_op", "px_mesh_op"];

/// 声明文件：**不许**再出现手列的源码清单。
const DECLARATIONS: &[&str] = &[
    "px_field_op/src/typed.rs",
    "px_volume_op/src/typed.rs",
    "px_mesh_op/src/typed.rs",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graph 必须住在 workspace 下")
        .to_path_buf()
}

#[test]
fn every_operator_crate_fingerprints_its_own_sources() {
    let root = workspace();
    let shared = root.join("build/fingerprint.rs");
    assert!(
        shared.is_file(),
        "共享的指纹助手不在了：{}（三个算子库的 build.rs 都 include! 它）",
        shared.display(),
    );

    for name in OPERATOR_CRATES {
        let build = root.join(name).join("build.rs");
        let text = std::fs::read_to_string(&build)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", build.display()));
        assert!(
            text.contains("include!"),
            "{name}/build.rs 没有 include! 共享助手 —— 自己写一套就等于又有了可漏的清单",
        );
        assert!(
            text.contains("build/fingerprint.rs"),
            "{name}/build.rs 没指向 `build/fingerprint.rs`",
        );
        assert!(
            text.contains("px_fingerprint_for_crate"),
            "{name}/build.rs 没有调指纹入口",
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
