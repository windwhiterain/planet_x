//! See docs/operators.md

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub type Roster = BTreeMap<String, PathBuf>;

pub fn roster(crate_dir: &Path, extra: &[&Path]) -> Roster {
    let mut files: Roster = BTreeMap::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    collect_crate(crate_dir, &mut files, &mut seen);
    for root in extra {
        collect_tree(root, root, &dir_name(root), &mut files);
    }
    files
}

pub fn hash(files: &Roster) -> String {
    let mut hasher = blake3::Hasher::new();
    for (label, path) in files {
        hasher.update(label.as_bytes());
        hasher.update(&[0]);
        let bytes =
            std::fs::read(path).unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    hasher.finalize().to_hex().to_string()
}

pub fn cargo_fingerprint_for_crate(extra: &[&Path]) {
    let manifest =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("没有 CARGO_MANIFEST_DIR"));
    let files = roster(&manifest, extra);
    for path in files.values() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for name in ["RUSTFLAGS", "PROFILE", "TARGET", "RUSTC"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rustc-env=PX_SOURCE_HASH={}", hash(&files));
    println!("cargo:rustc-env=PX_TOOLCHAIN_HASH={}", toolchain_hash());
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

fn rustc() -> String {
    std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string())
}

fn rustc_version() -> String {
    std::process::Command::new(rustc())
        .arg("-vV")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

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

fn field(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    hasher.update(&[0]);
}

fn collect_crate(crate_dir: &Path, files: &mut Roster, seen: &mut BTreeSet<PathBuf>) {
    let key = normalized(crate_dir);
    if !seen.insert(key) {
        return;
    }
    let before = files.len();
    collect_sources(crate_dir, files);
    if files.len() == before && std::env::var_os("CARGO_MANIFEST_DIR").is_some() {
        println!(
            "cargo:warning=源码指纹：{} 没有可收的 .rs / .wgsl",
            crate_dir.display()
        );
    }
    for (name, path) in path_dependencies(crate_dir) {
        if normalized(&path) == normalized(crate_dir) {
            continue;
        }
        let _ = name;
        collect_crate(&path, files, seen);
    }
}

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
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("rs") | Some("wgsl")
        ) {
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
    let label = format!("{prefix}/{}", relative.to_string_lossy().replace('\\', "/"));
    files.insert(label, path);
}

fn dir_name(dir: &Path) -> String {
    dir.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("crate")
        .to_string()
}

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
            normalized(&manifest.join(relative)),
        ));
    }
    out
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roster_carries_the_shaders() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("px_fingerprint 住在 workspace 下")
            .to_path_buf();
        for (crate_name, shader) in [
            ("px_nurbs_gpu_op", "src/surface.wgsl"),
            ("px_nurbs_gpu_op", "src/curve.wgsl"),
            ("px_volume_gpu_op", "src/sampler.wgsl"),
        ] {
            let roster = roster(&workspace.join(crate_name), &[]);
            let label = format!("{crate_name}/{shader}");
            assert!(
                roster.contains_key(&label),
                "{crate_name} 的名册里没有 {shader}：{}",
                roster.keys().cloned().collect::<Vec<_>>().join(" / ")
            );
        }
    }
}
