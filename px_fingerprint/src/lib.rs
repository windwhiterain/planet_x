//! See docs/operators.md

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub type Roster = BTreeMap<String, PathBuf>;

/// The files that make up one crate's identity: its module tree, its shaders and its `build.rs`.
/// A dependency's roster is folded in through [`collect_crate`]'s path walk, so the list is built
/// from the crate directory alone.
pub fn roster(crate_dir: &Path) -> Roster {
    let mut files: Roster = BTreeMap::new();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    collect_crate(crate_dir, &mut files, &mut seen);
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

pub fn cargo_fingerprint_for_crate() {
    let manifest =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("没有 CARGO_MANIFEST_DIR"));
    let files = roster(&manifest);
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
    let mut paths = Vec::new();
    collect_source_paths(crate_dir, &mut paths);
    for path in paths {
        insert(files, crate_dir, &crate_name, path);
    }
}

/// Every file this crate compiles into its artifacts: the Rust files the module tree actually
/// declares, and the shaders under `src/`.
///
/// The module tree is the oracle, not the file names: `collect_tree` used to skip a directory named
/// `tests` and any file whose name contained `_test`, which made a file that *is* compiled (declared
/// with `mod`, or reached through `#[path]` / `include!`) invisible to identity — "same key,
/// different content". A `.rs` file under `src/` that no declaration reaches is not compiled, so it
/// is not part of this list.
///
/// ⚠ One deliberate exception: every `.wgsl` under `src/` is collected regardless of the module
/// tree. Shaders are data, not modules, and are `include_str!`d by the crate; skipping an undeclared
/// one would be the same hole in the other direction. A shader that no code reaches therefore still
/// enters the fingerprint (and only the gate can call that a defect).
fn collect_source_paths(crate_dir: &Path, out: &mut Vec<PathBuf>) {
    let src = crate_dir.join("src");
    if src.is_dir() {
        collect_rust_paths(&src, out);
        let mut shaders = Vec::new();
        collect_files_with_extension(&src, "wgsl", &mut shaders);
        shaders.sort();
        out.extend(shaders);
    }
    out.push(crate_dir.join("build.rs"));
    out.sort();
    out.dedup();
    out.retain(|path| path.is_file());
}

/// Walks the module tree from `src/lib.rs`, `src/main.rs` or `src/mod.rs`.
fn collect_rust_paths(src: &Path, out: &mut Vec<PathBuf>) {
    let mut roots: Vec<PathBuf> = ["lib.rs", "main.rs", "mod.rs"]
        .iter()
        .map(|name| src.join(name))
        .filter(|path| path.is_file())
        .collect();
    // `src/bin/*.rs` are separate binaries, each a root of its own tree. They are never modules of
    // the library, so no root file declares them.
    let mut binaries = Vec::new();
    collect_files_with_extension(&src.join("bin"), "rs", &mut binaries);
    roots.append(&mut binaries);
    roots.sort();

    let mut visited: BTreeSet<PathBuf> = BTreeSet::new();
    let mut stack: Vec<PathBuf> = roots;
    while let Some(file) = stack.pop() {
        if !visited.insert(normalized(&file)) {
            continue;
        }
        out.push(file.clone());
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        for child in declared_modules(&file, &text) {
            if child.is_file() {
                stack.push(child);
            }
        }
    }
}

/// The files one Rust source declares: `mod name;` (with an optional `#[path = "…"]`), and literal
/// `include!("…")`.
///
/// `include!(concat!(env!("OUT_DIR"), …))` is not followed on purpose: its target is generated
/// rather than checked in, and rustc records it as a dependency of the crate itself.
fn declared_modules(file: &Path, text: &str) -> Vec<PathBuf> {
    let dir = file.parent().unwrap_or(file);
    let code = sanitized(text);
    let mut out = Vec::new();
    for (at, name) in module_declarations(&code) {
        let attribute = code[..at].rfind("#[path");
        if let Some(start) = attribute {
            if let Some(path) = literal_after(&code, text, start, "path") {
                out.push(normalize_path(&dir.join(path)));
                continue;
            }
        }
        let flat = dir.join(format!("{name}.rs"));
        let nested = dir.join(&name).join("mod.rs");
        if flat.is_file() {
            out.push(normalize_path(&flat));
        } else if nested.is_file() {
            out.push(normalize_path(&nested));
        }
    }
    let mut from = 0usize;
    while let Some(offset) = code[from..].find("include!") {
        let at = from + offset;
        if let Some(path) = literal_after(&code, text, at, "include!") {
            out.push(normalize_path(&dir.join(path)));
        }
        from = at + "include!".len();
    }
    out
}

/// A copy of `text` in which comment bodies and string/char contents are replaced by spaces, so a
/// search finds **code** and not a mention of it. The quote characters themselves stay in place, so
/// an offset valid in this copy finds the literal's opening quote in the original.
fn sanitized(text: &str) -> String {
    #[derive(PartialEq)]
    enum State {
        Code,
        Line,
        Block,
        Text,
        Quote,
    }
    let mut out = text.as_bytes().to_vec();
    let mut state = State::Code;
    let mut index = 0usize;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap_or(' ');
        let width = ch.len_utf8();
        match state {
            State::Code => {
                if ch == '/' && text[index..].starts_with("//") {
                    state = State::Line;
                } else if ch == '/' && text[index..].starts_with("/*") {
                    state = State::Block;
                } else if ch == '"' {
                    state = State::Text;
                } else if ch == '\'' {
                    state = State::Quote;
                } else {
                    index += width;
                    continue;
                }
                index += width;
            }
            State::Line => {
                if ch == '\n' {
                    state = State::Code;
                } else {
                    blank(&mut out, index, width);
                }
                index += width;
            }
            State::Block => {
                if ch == '*' && text[index..].starts_with("*/") {
                    blank(&mut out, index, width);
                    blank(&mut out, index + width, 1);
                    index += width + 1;
                    state = State::Code;
                    continue;
                }
                blank(&mut out, index, width);
                index += width;
            }
            State::Text => {
                if ch == '\\' {
                    blank(&mut out, index, width);
                    let next = text[index + width..].chars().next().unwrap_or(' ');
                    blank(&mut out, index + width, next.len_utf8());
                    index += width + next.len_utf8();
                    continue;
                }
                if ch == '"' {
                    state = State::Code;
                } else {
                    blank(&mut out, index, width);
                }
                index += width;
            }
            State::Quote => {
                if ch == '\\' {
                    blank(&mut out, index, width);
                    let next = text[index + width..].chars().next().unwrap_or(' ');
                    blank(&mut out, index + width, next.len_utf8());
                    index += width + next.len_utf8();
                    continue;
                }
                // A lifetime (`'a`) has no closing quote right after its identifier, so it is not a
                // character literal and ends the state without blanking anything.
                let closes = text[index..].find('\'').is_some_and(|end| end <= 2);
                if closes {
                    blank(&mut out, index, width);
                } else {
                    state = State::Code;
                }
                index += width;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

fn blank(out: &mut [u8], at: usize, width: usize) {
    for byte in out.iter_mut().skip(at).take(width) {
        *byte = b' ';
    }
}

/// The literal one call or attribute takes, searched in `code` (a [`sanitized`] copy) and read out
/// of `text`.
fn literal_after(code: &str, text: &str, from: usize, name: &str) -> Option<String> {
    let rest = &code[from..];
    let name_at = rest.find(name)?;
    let open = rest[name_at..].find('"')?;
    let quote = from + name_at + open;
    let tail = &text[quote + 1..];
    let mut path = String::new();
    let mut chars = tail.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => return Some(path),
            '\\' => {
                if let Some(escaped) = chars.next() {
                    path.push(escaped);
                }
            }
            other => path.push(other),
        }
    }
    None
}

/// Offsets of every `mod <name>;` declaration in a sanitized source, with the tool attributes that
/// do not name a module filtered out (`mod tests { … }` is inline and has no file).
fn module_declarations(code: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(offset) = code[from..].find("mod ") {
        let at = from + offset;
        from = at + 4;
        let rest = &code[at + 4..];
        let Some(name) = identifier_at(rest) else {
            continue;
        };
        let after = rest[name.len()..].trim_start();
        if after.starts_with(';')
            && !matches!(
                name.as_str(),
                "tests" | "benches" | "examples" | "doc" | "no_std" | "allow" | "warn" | "deny"
            )
        {
            out.push((at, name));
        }
    }
    out
}

fn identifier_at(text: &str) -> Option<String> {
    let name: String = text
        .chars()
        .take_while(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

fn collect_files_with_extension(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extension(&path, extension, out);
        } else if path.extension().and_then(|value| value.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}

fn normalize_path(path: &Path) -> PathBuf {
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

/// Every manifest section that names dependencies, except `[dev-dependencies]`, with the path edges
/// it declares. Public because the gate over this rule needs the same parse the collector uses —
/// a second parser in `tests/` would be free to disagree with the one that computes identity.
pub fn path_dependencies(manifest: &Path) -> Vec<(String, PathBuf)> {
    let Ok(text) = std::fs::read_to_string(manifest.join("Cargo.toml")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut in_dependencies = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            // The dev edge is skipped because a dev-dependency is not compiled into the artifact:
            // its sources cannot change what this crate produces, so they must not change what its
            // key is. Following it once put all of a sibling simulation into every roster
            // downstream, a quarter to a third of what identity was computed from.
            in_dependencies =
                trimmed.contains("dependencies") && !trimmed.contains("dev-dependencies");
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
            let roster = roster(&workspace.join(crate_name));
            let label = format!("{crate_name}/{shader}");
            assert!(
                roster.contains_key(&label),
                "{crate_name} 的名册里没有 {shader}：{}",
                roster.keys().cloned().collect::<Vec<_>>().join(" / ")
            );
        }
    }
}
