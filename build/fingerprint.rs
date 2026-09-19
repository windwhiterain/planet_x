// **源码指纹**：算子身份的自动来源。
//
// 由每个算子 crate 的 `build.rs` 用 `include!` 进来。它算的是**这个 crate 编译进去的
// 全部源码**：
//
// * 自己 `src/` 下的 `.rs`（递归）
// * `Cargo.toml` 里所有 **path 依赖**的 `src/` 下的 `.rs`（递归）
// * 自己的 `build.rs`
//
// ⇒ **加一个新文件不用改任何清单**（清单是旧的 `include_str![…]` 那套，现在不需要了）。
//
// ⚠ `cargo:rustc-env` **不跨 crate 传播**（实测：依赖设的变量，本 crate 的 `env!` 取不到）。
//   所以这个脚本只能对自己的 crate 说话 —— 共享件的指纹由**用它的人**各算一份。
//   这正好是对的：`px_field_op` 与 `px_volume_op` 依赖的 `px_verify` 不必是同一份。
//
// ⚠ 各 crate 的 `build.rs` 本身不做 `rerun-if-changed` 握手 —— cargo 对 **path 依赖**
//   与**自己这个 package** 的 `src/` 有自动指纹，改了就重跑脚本。手工清单反而会漏。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 算子库用的入口：算**这个 crate**（自己 `src/` + 所有 path 依赖的 `src/`）。
///
/// ⚠ 它**不叫 `main`**：这个文件被 `include!` 进 `build.rs`，而 `build.rs` 自己要有 `main`。
pub fn px_fingerprint_for_crate() {
    let manifest =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("没有 CARGO_MANIFEST_DIR"));
    let mut files = BTreeSet::new();
    collect_sources(&manifest, &mut files);

    for (name, path) in path_dependencies(&manifest) {
        if path.starts_with(&manifest) {
            continue; // 自己的 src 已经在上面收过
        }
        let before = files.len();
        collect_sources(&path, &mut files);
        if files.len() == before {
            // 不是问题（对方可能什么都没有），但值得留一行痕迹，免得"少算了一份"无声无息。
            println!("cargo:warning=源码指纹：{name}（{}）没有可收的 .rs", path.display());
        }
    }

    emit(&manifest, &files);
}

/// 生成物（图侧自建的那份 dylib）用的入口：**显式给**一串契约文件。
///
/// ⚠ 它不能走 `path_dependencies`：那份 crate 的依赖是生成器写的，而"契约文件"
///   （stage 1 的场函数、`px_cook` 的抽象…）在它的 `Cargo.toml` 里看不出来。
pub fn px_mono_fingerprint(extra: &[&str]) {
    let manifest =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("没有 CARGO_MANIFEST_DIR"));
    let mut files = BTreeSet::new();
    collect_sources(&manifest, &mut files);
    for path in extra {
        let absolute = manifest.join(path);
        assert!(
            absolute.is_file(),
            "契约文件不存在：{}（生成器写进去的路径要与工作区对得上）",
            absolute.display(),
        );
        files.insert(absolute);
    }
    emit(&manifest, &files);
}

/// 收完文件之后：算哈希、写进 `cargo:rustc-env`、给每个文件打 `rerun-if-changed`。
fn emit(manifest: &Path, files: &BTreeSet<PathBuf>) {
    let mut hash = blake3::Hasher::new();
    for file in files {
        let relative = file
            .strip_prefix(&manifest)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| file.clone());
        // ⚠ 路径也进哈希：两份内容相同但名字不同的文件不该撞。
        hash.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hash.update(&[0]);
        let bytes = std::fs::read(file)
            .unwrap_or_else(|err| panic!("读不了 {}：{err}", file.display()));
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    println!("cargo:rustc-env=PX_SOURCE_HASH={}", hash.finalize().to_hex());
}

/// `src/` 下的 `.rs` + 这个 crate 的 `build.rs`。
///
/// ⚠ 跳过测试代码（`tests/`、`*_test.rs`、`test_*.rs`）：它们编不进算子，改它们不该变键。
fn collect_sources(crate_dir: &Path, files: &mut BTreeSet<PathBuf>) {
    let src = crate_dir.join("src");
    if src.is_dir() {
        walk(&src, files);
    }
    let build = crate_dir.join("build.rs");
    if build.is_file() {
        files.insert(build);
    }
}

fn walk(dir: &Path, files: &mut BTreeSet<PathBuf>) {
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
            walk(&path, files);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.contains("_test") || name.starts_with("test_") {
                continue;
            }
            files.insert(path);
        }
    }
}

/// `Cargo.toml` 里所有带 `path` 的依赖 → `(名字, 绝对路径)`。
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
        out.push((
            name.trim().to_string(),
            manifest.join(relative),
        ));
    }
    out
}
