//! **源码指纹**：算子身份的自动来源 —— 一个**普通 rlib**，两边共用同一份。
//!
//! 它从前是 `build/fingerprint.rs`（被各 crate 的 `build.rs` 用 `include!` 进来的一段源码），
//! 现在抽成 **`px_fingerprint` 这个 crate**，因为**运行期**也要算同一个东西：泛型实例的 key
//! 必须覆盖**声明所在 crate + alg crate 的源码指纹**，而图程序不能 cargo 依赖 alg crate
//! （那会弄丢 R1）⇒ 只能**从盘上**算它的 `src/` 名册（`.agents/notes/art/19-generic-inst.md`
//! §177 / §179.2）。于是：
//!
//! * `build.rs` 走 [`cargo_fingerprint_for_crate`]（`[build-dependencies]`）；
//! * `px_cook::inst` 走 [`roster`] + [`hash`] + [`toolchain_hash`]（`[dependencies]`）。
//!
//! 它算的是**这个 crate 编译进去的全部源码**：
//!
//! * 自己 `src/` 下的 `.rs`（递归）+ 自己的 `build.rs`
//! * `Cargo.toml` 里所有 **可达的** path 依赖（**传递闭包**，不只是直接那层）的 `src/` 下的 `.rs`
//!   —— §28.2 那条：改了**共享件**（契约 / 参数 struct / 载荷编解码）也得换键
//!
//! ⚠ **为什么必须是闭包**：`px_derive` 这一档（proc-macro）是实现库的**传递**依赖 ——
//!   它生成 `PxInputs::collect`（进键）与 `PxKeyed`。只走直接那层就会漏掉它，实测过：
//!   只改 `px_derive/src/lib.rs` ⇒ `px_field_op.dll` 的身份**一位不变**、planet 六个节点
//!   **全部命中**（陈旧命中，与 §168 是同一族的形状，只是换了个方向）。
//!
//! ⇒ **加一个新文件不用改任何清单**（清单是旧的 `include_str![…]` 那套，现在不需要了）。
//!
//! ⚠ 每个文件按「**crate 目录名**/包内相对路径」进哈希，**不按绝对路径** ——
//!   绝对路径里带着 checkout 的位置，那会让"换一个目录签出"或"把仓库挪个地方"换掉所有键
//!   （§14.2：键是纯函数，不含路径）。文件**内容**与**包内名字**才是身份。
//!
//! ⚠ `cargo:rustc-env` **不跨 crate 传播**（实测：依赖设的变量，本 crate 的 `env!` 取不到）。
//!   所以这个脚本只能对自己的 crate 说话 —— 共享件的指纹由**用它的人**各算一份。
//!   这正好是对的：`px_field_op` 与 `px_volume_op` 依赖的 `px_verify` 不必是同一份。
//!
//! ⚠ `rerun-if-changed` 必须由 [`cargo_fingerprint_for_crate`] **显式**给（每个文件一行）：
//!   cargo 只对自己 package 的 `src/` 有自动指纹，path 依赖的改动**不会**自动重跑 build script。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// **名册**：`标签 → 文件路径`。
///
/// 标签是「一个文件在指纹里的**名字**」：`<crate 目录名>/<包内相对路径>`。
/// 用名字而不是绝对路径：见上面那条「键不含路径」。
///
/// ⚠ 用 `BTreeMap` 而不是 `HashMap`：哈希的顺序必须是**确定**的（同一次 checkout
///   上算两遍要得到同一个值，跑图程序与跑工具也要得到同一个值）。
pub type Roster = BTreeMap<String, PathBuf>;

/// 一个 crate + 它**可达的全部 path 依赖**（传递闭包）+ 额外的根（目录），收成名册。
///
/// 标签 = `<crate 目录名>/<包内相对路径>`；跳过 tests/ 与 *_test.rs（编不进产品）。
///
/// ⚠ 给**运行期**算 key 用（`px_cook::inst`：声明 crate 与 alg crate 的名册都不能 cargo
///   依赖地拿到，只能从盘上收）。它**不打印**任何 `cargo:` 行 —— 调用它的地方不在
///   build script 里，打印出去只会是噪音。
pub fn roster(crate_dir: &Path, extra: &[&Path]) -> Roster {
    let mut files: Roster = BTreeMap::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    collect_crate(crate_dir, &mut files, &mut seen);
    for root in extra {
        // 额外的根自己不是一个 crate（没有 `Cargo.toml` 那条规则），就按它的**目录名**
        // 当标签前缀：`art/inst/mine.rs` 进来是 `inst/mine.rs`。
        collect_tree(root, root, &dir_name(root), &mut files);
    }
    files
}

/// 名册的 blake3 十六进制（名字也进哈希）。
pub fn hash(files: &Roster) -> String {
    let mut hasher = blake3::Hasher::new();
    for (label, path) in files {
        // ⚠ 名字也进哈希：两份内容相同但名字不同的文件不该撞。
        hasher.update(label.as_bytes());
        hasher.update(&[0]);
        let bytes = std::fs::read(path)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        // 长度前缀：不让"两个字段拼起来相同"撞（`ab` + `c` 与 `a` + `bc`）。
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    hasher.finalize().to_hex().to_string()
}

/// `build.rs` 的入口：算名册 → 每个文件一行 `cargo:rerun-if-changed` →
/// `cargo:rustc-env=PX_SOURCE_HASH=<hex>` → `cargo:rustc-env=PX_TOOLCHAIN_HASH=<hex>`
/// → 工具链那四样**原料**（`PX_RUSTC_VERSION` / `PX_TARGET` / `PX_PROFILE` / `PX_RUSTFLAGS`）。
///
/// `extra` 是要额外收进指纹的目录（目前没人用；预留给"这个 crate 的产物还取决于
/// 仓库里某个非 crate 目录"那一档）。
///
/// ⚠ 两个**哈希**变量都发：`PX_SOURCE_HASH` 是 `PxOp::decl_hash` 与实现库身份用的那一份，
///   `PX_TOOLCHAIN_HASH` 是装载时握的**工具链**那一份（`px_impl_lib!` 导出的符号要用它，
///   见 `19-generic-inst.md` §179.3）。少发一个就是 `env!` 编译错。
///
/// ⚠ 四个**原料**变量是给 `px_jit` 用的：它要把它们原样转给嵌套的
///   `cargo build`（`--target` / `--profile` / `RUSTFLAGS`）—— 否则生成的实例库与主
///   workspace 算出**两个**工具链哈希，装载握手会（正确地）把每一条实例都拒掉。
///   拒绝是对的，但得让人有办法改对（见 [`toolchain_hash`] 那条"为什么运行期算不出同一个值"）。
pub fn cargo_fingerprint_for_crate(extra: &[&Path]) {
    let manifest =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("没有 CARGO_MANIFEST_DIR"));
    let files = roster(&manifest, extra);
    for path in files.values() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    // ⚠ **必须声明这些环境变量**：cargo 不会因为 `RUSTFLAGS`（或 profile/target/rustc）变了就
    //   重跑 build script —— 于是本脚本会把**旧的** `PX_TOOLCHAIN_HASH` 一路发下去，
    //   而那道工具链握手就变成"两边一样地旧"、对"改过 flag"完全瞎（实测：加上 `RUSTFLAGS`
    //   重建整个工作区 36 s，而 `PX_TOOLCHAIN_HASH` 一个字符没变）。
    for name in ["RUSTFLAGS", "PROFILE", "TARGET", "RUSTC"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rustc-env=PX_SOURCE_HASH={}", hash(&files));
    println!("cargo:rustc-env=PX_TOOLCHAIN_HASH={}", toolchain_hash());
    // 原料：`px_jit` 拿它们把**嵌套那次** `cargo build` 摆成同一套工具链。
    println!("cargo:rustc-env=PX_RUSTC_VERSION={}", rustc_version());
    println!(
        "cargo:rustc-env=PX_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=PX_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=PX_RUSTFLAGS={}",
        std::env::var("RUSTFLAGS").unwrap_or_default()
    );
}

/// `$RUSTC` 是 `RUSTC` 环境变量，取不到就 `"rustc"`（走 `PATH`）。
fn rustc() -> String {
    std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string())
}

/// `$RUSTC -vV` 的**全文**（换行保留）。跑不起来就是空串。
///
/// ⚠ 用 `std::process::Command` 直接跑 `$RUSTC`（`RUSTC` 取不到就用 `PATH` 上的 `rustc`）：
///   这个 crate 不许引任何依赖（它自己会被收进别人的指纹名册）。
fn rustc_version() -> String {
    std::process::Command::new(rustc())
        .arg("-vV")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

/// 工具链指纹：`$RUSTC -vV` 全文 + `TARGET` + `RUSTFLAGS` + `PROFILE` 的 blake3 十六进制。
///
/// ⚠ **只给 `build.rs` 用。运行期算不出同一个值** —— 实测（`pcg-op-dylib` 那一轮）：
///
/// ```text
/// build script  PX_TOOLCHAIN_HASH = b5b4454cb159847d81bd99ad1b0a4cd4113cdd0842e9eaaa94ab9a32a95e655d
/// 图程序那一侧（只有 PATH 那套环境）:
///   TARGET=<未设>  PROFILE=<未设>  RUSTFLAGS=<未设>
///   toolchain_hash()               = 08aac4ead90d445006a8d5e3d02f0b53b758fdc7b91dc1c89be4c76efc7e6625   ← 不一样
/// 换个进程，把 build script 那三个 env 摆上:
///   TARGET=x86_64-pc-windows-msvc  PROFILE=debug  RUSTFLAGS=""
///   toolchain_hash()               = b5b4454cb159847d81bd99ad1b0a4cd4113cdd0842e9eaaa94ab9a32a95e655d   ← 逐字符相同
/// ```
///
/// 根因一句话：**`TARGET` 与 `PROFILE` 是 cargo 只给 build script 设的环境变量**
/// ⇒ 同一个函数在两处拿到的输入不同、值当然不同。
///
/// ⇒ 握手的两个操作数因此**都取编译期嵌进各自构件的常量**（`19-generic-inst.md` §179.3）：
///   图程序那一侧用 `px_graph_schema::TOOLCHAIN_HASH`，库那一侧用它自己 `build.rs` 发的
///   `…__toolchain_hash` —— 一条运行期算哈希的路径都没有。
///
/// ⚠ **刻意不含** `DEBUG` / `OPT_LEVEL`（不动布局；进了只会让两个 workspace 必须逐字对齐）。
///   它要回答的问题是"两份 dylib 的 `extern "Rust"` ABI 是不是同一个编译器给的"，
///   而 `debug` / `opt-level` 不动布局（见 `19-generic-inst.md` §176 第 2 条）。
///
/// ⚠ `px_jit` 要复现这一份值，就用同一个函数、但先把四个原料摆回环境（本 crate 发的
///   `PX_RUSTC_VERSION` / `PX_TARGET` / `PX_PROFILE` / `PX_RUSTFLAGS`）。
///
/// ⚠ 字段之间用 `\0` + 长度前缀分隔（别让"两个字段拼起来相同"撞）。
pub fn toolchain_hash() -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"px_toolchain/v1");
    field(&mut hasher, rustc_version().as_bytes());
    field(
        &mut hasher,
        std::env::var("TARGET").unwrap_or_default().as_bytes(),
    );
    field(
        &mut hasher,
        std::env::var("RUSTFLAGS").unwrap_or_default().as_bytes(),
    );
    field(
        &mut hasher,
        std::env::var("PROFILE").unwrap_or_default().as_bytes(),
    );
    hasher.finalize().to_hex().to_string()
}

/// 一个字段进哈希：长度前缀 + 内容 + 分隔符（"拼起来相同"撞不上）。
fn field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    hasher.update(&[0]);
}

/// 收一个 crate，然后**递归**收它可达的 path 依赖（传递闭包，`seen` 防环）。
///
/// ⚠ 递归而不是只走一层：见文件头那条「为什么必须是闭包」。
fn collect_crate(crate_dir: &Path, files: &mut Roster, seen: &mut BTreeSet<PathBuf>) {
    let key = normalized(crate_dir);
    if !seen.insert(key) {
        return;
    }
    let before = files.len();
    collect_sources(crate_dir, files);
    // ⚠ **只在 build script 里打印**（`CARGO_MANIFEST_DIR` 只有 cargo 给 build script 设）：
    //   [`roster`] 也会在**运行期**被调用（`px_cook::inst` 算 key），那时候往 stdout 写字
    //   会污染调用方自己的输出（图程序的标准输出是有用的东西）。
    //   不是问题时也值得留一行痕迹，免得"少算了一份"无声无息。
    if files.len() == before && std::env::var_os("CARGO_MANIFEST_DIR").is_some() {
        println!(
            "cargo:warning=源码指纹：{} 没有可收的 .rs",
            crate_dir.display()
        );
    }
    for (name, path) in path_dependencies(crate_dir) {
        if normalized(&path) == normalized(crate_dir) {
            continue; // 自己（`path = "."` 那种写法）已经在上面收过
        }
        let _ = name;
        collect_crate(&path, files, seen);
    }
}

/// `src/` 下的 `.rs` + 这个 crate 的 `build.rs`。
///
/// ⚠ 跳过测试代码（`tests/`、`*_test.rs`、`test_*.rs`）：它们编不进算子，改它们不该变键。
fn collect_sources(crate_dir: &Path, files: &mut Roster) {
    let crate_name = dir_name(crate_dir);
    let src = crate_dir.join("src");
    if src.is_dir() {
        collect_tree(&src, crate_dir, &crate_name, files);
    }
    let build = crate_dir.join("build.rs");
    if build.is_file() {
        insert(files, crate_dir, &crate_name, build);
    }
}

/// 把一棵目录树（`root` 之下）的 `.rs` 收进名册，标签前缀用 `prefix`。
///
/// ⚠ 它同时给 crate 自己的 `src/`（前缀 = crate 目录名）与额外的根用。
fn collect_tree(dir: &Path, root: &Path, prefix: &str, files: &mut Roster) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "tests" || name.starts_with('.') {
                continue;
            }
            collect_tree(&path, root, prefix, files);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.contains("_test") || name.starts_with("test_") {
                continue;
            }
            insert(files, root, prefix, path);
        }
    }
}

fn insert(files: &mut Roster, root: &Path, prefix: &str, path: PathBuf) {
    let relative = path.strip_prefix(root).unwrap_or(&path);
    let label = format!(
        "{prefix}/{}",
        relative.to_string_lossy().replace('\\', "/")
    );
    files.insert(label, path);
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("crate")
        .to_string()
}

/// `Cargo.toml` 里所有带 `path` 的依赖 → `(名字, **规范化后**的绝对路径)`。
///
/// ⚠ 用一个够用的行扫描器，不引 `toml` 依赖：这个脚本只需要 `name = { path = "…" }`
///   与 `name = { … path = "…" … }` 两种写法。
fn path_dependencies(manifest: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(manifest.join("Cargo.toml")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut in_dependencies = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed.contains("dependencies");
            continue;
        }
        if !in_dependencies || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, rest)) = trimmed.split_once('=') else {
            continue;
        };
        let Some(at) = rest.find("path") else {
            continue;
        };
        let after = &rest[at + 4..];
        let Some(start) = after.find('"') else {
            continue;
        };
        let Some(end) = after[start + 1..].find('"') else {
            continue;
        };
        let relative = &after[start + 1..start + 1 + end];
        out.push((name.trim().to_string(), normalized(&manifest.join(relative))));
    }
    out
}

/// 词法规范化：去掉 `.` 与 `..` 组件。
///
/// ⚠ 这不是洁癖，是一个**真缺陷**的修法：`manifest.join("../px_cook")` 里带着 `..`，
///   而 `Path` 的比较是按**组件**比的（不化简 `..`）⇒ 不规范化的话，"这一份是不是已经收过"
///   的判断（`seen` 集合，以及"自己（`path = "."`）跳过"那一句）会把**每一个** path 依赖
///   都判成"自己"而跳过 —— 指纹里只剩本 crate 的文件，**改共享依赖不换键**（实测过：
///   `rerun-if-changed` 只有自己的 12 个文件）。
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
