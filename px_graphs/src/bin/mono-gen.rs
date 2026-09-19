//! **生成 + 编译一个单态化实例**（那一级的全部机械部分）。
//!
//! 它做四件事，每一步都必要：
//!
//! 1. 从 [`px_graphs::mono`] 取身份（`id` / `version` / `source_hash`）；
//! 2. 把 `mono/template/` 复制到**固定**的构建位置 `target/mono/crate/`，
//!    并把身份具体化成 `src/identity.rs`；
//! 3. `cargo build` 那个目录（`--target-dir target/mono/build`，**固定**，
//!    这样依赖只编一次、之后按 cargo 自己的指纹复用）；
//! 4. 把 cdylib 复制成 `px_mono_op.dll` 放进 `target/debug/`
//!    —— 入口名由库名派生（`px_graph_schema::op::table_symbol`），驱动一行不用改。
//!
//! ⚠ 两条**试过且是错的**做法，留在这里当路标：
//!
//! * 让生成物**共用主 workspace 的 `target/`** ⇒ 它的依赖图不同（自己一份
//!   `Cargo.lock`、自己的特征统一）⇒ cargo 把 `px_volume_op.dll` 等**再编一份**
//!   进同一个 `target/debug/deps/` 覆盖主 workspace 那份 ⇒ `LoadLibraryExW failed`；
//! * **构建目录也按 mono_key 分** ⇒ 每次改一行场函数都在新目录里从零编依赖
//!   （实测 20.8 s/次，比不带这一级还差）。
//!
//! 用法：`cargo run -q -p px_graphs --bin mono-gen`

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_graphs 必须住在 workspace 下")
        .to_path_buf()
}

fn main() {
    let root = workspace();
    let template = root.join("px_graphs/mono/template");
    let crate_dir = root.join("target/mono/crate");
    let build_dir = root.join("target/mono/build");
    let identity = px_graphs::mono::MONO_ID;
    let version = px_graphs::mono::VERSION;
    let source_hash = px_graphs::mono::source_hash();

    println!("单态化实例：id={identity} v{version} source_hash={source_hash:016x}");

    // ── 2) 落到固定的构建位置（内容 → 身份写在源码里，路径只是"当前那一份"的落点）────
    let src = crate_dir.join("src");
    std::fs::create_dir_all(&src).expect("建不了生成目录");
    copy(&template.join("Cargo.toml"), &crate_dir.join("Cargo.toml"));
    copy(&template.join("lib.rs"), &src.join("lib.rs"));
    copy(&template.join("identity.rs"), &src.join("identity.rs"));
    copy(&root.join("px_graphs/mono/fields.rs"), &src.join("fields.rs"));

    // 身份具体化：把模板里的占位符换成真值。
    // ⚠ stage 1 的源码**不进生成的 crate 的 `src/` 之外**：它就在 `fields.rs` 里，
    //   而 `SOURCE_HASH` 已经是它的内容哈希 ⇒ 改它身份必变。
    let text = std::fs::read_to_string(src.join("identity.rs")).expect("读不了 identity.rs");
    let text = text
        .replace("@MONO_ID@", &format!("\"{identity}\""))
        .replace("@VERSION@", &version.to_string())
        .replace("@SOURCE_HASH@", &source_hash.to_string());
    std::fs::write(src.join("identity.rs"), text).expect("写不了 identity.rs");

    // ── 3) 编 ─────────────────────────────────────────────────────────────────
    let status = Command::new("cargo")
        .args([
            "build",
            "--manifest-path",
            crate_dir.join("Cargo.toml").to_str().expect("路径不是 UTF-8"),
            "--target-dir",
            build_dir.to_str().expect("路径不是 UTF-8"),
        ])
        .status()
        .expect("起不了 cargo");
    assert!(status.success(), "编单态化实例失败：{status}");

    // ── 4) 放进装载目录 ────────────────────────────────────────────────────────
    let built = build_dir.join("debug/px_mono.dll");
    let deposit = root.join("target/debug/px_mono_op.dll");
    let bytes = std::fs::copy(&built, &deposit)
        .unwrap_or_else(|err| panic!("复制 {} 失败：{err}", built.display()));
    println!(
        "装入 {}（{:.2} MB）",
        deposit.display(),
        bytes as f64 / (1024.0 * 1024.0),
    );
    println!(
        "⚠ 这一份 dylib 在运行时还要它自己的上游 DLL（`dylib` 这个 crate-type 的 ABI）；\
         现在靠主 workspace 的 `target/debug/` 提供 —— 见笔记 §166.4。"
    );
}

fn copy(from: &Path, to: &Path) {
    std::fs::copy(from, to)
        .unwrap_or_else(|err| panic!("复制 {} → {} 失败：{err}", from.display(), to.display()));
}
