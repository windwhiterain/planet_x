//! See docs/invariants.md
//!
//! The gate over one direction of the identity edge: a crate reached **only** through a
//! `[dev-dependencies]` path edge is not compiled into anything shipped, so its sources must not
//! appear in a roster. `px_protocol` reaches `game` that way (its snapshot tests build fixtures from
//! the real types), and following the edge put all of `game/src` into every roster downstream —
//! roughly a quarter to a third of what identity was computed from, including code that never runs
//! in the product.
//!
//! "Only through dev" is computed, not listed: a crate named as a dev-dependency stays a product
//! crate when some crate also depends on it through a runtime or build edge (`px_derive` is both).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn workspace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_fingerprint lives under the workspace")
        .to_path_buf()
}

fn crate_dirs(workspace: &Path) -> Vec<PathBuf> {
    let mut crates: Vec<PathBuf> = std::fs::read_dir(workspace)
        .expect("readable workspace")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("Cargo.toml").is_file())
        .collect();
    crates.sort();
    crates
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn slash(text: &str) -> String {
    text.replace('\\', "/")
}

/// The dev-only edges of one manifest. The product edges come from `px_fingerprint` itself, so this
/// parser only answers the question the collector deliberately does not: which path dependencies sit
/// under `[dev-dependencies]`.
fn dev_edges(crate_dir: &Path) -> Vec<PathBuf> {
    let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap_or_default();
    let mut out = Vec::new();
    let mut in_dev = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dev = trimmed.contains("dev-dependencies");
            continue;
        }
        if !in_dev || trimmed.starts_with('#') {
            continue;
        }
        let Some(at) = trimmed.find("path") else {
            continue;
        };
        let after = &trimmed[at + 4..];
        let Some(start) = after.find('"') else {
            continue;
        };
        let Some(end) = after[start + 1..].find('"') else {
            continue;
        };
        out.push(normalize(
            &crate_dir.join(&after[start + 1..start + 1 + end]),
        ));
    }
    out
}

/// Everything reachable from `roots` by following the edges `next` reports.
fn closure(roots: Vec<PathBuf>, next: impl Fn(&Path) -> Vec<PathBuf>) -> BTreeSet<PathBuf> {
    let mut reached: BTreeSet<PathBuf> = BTreeSet::new();
    let mut stack = roots;
    while let Some(crate_dir) = stack.pop() {
        if !reached.insert(normalize(&crate_dir)) {
            continue;
        }
        stack.extend(next(&crate_dir));
    }
    reached
}

/// The crates a product build compiles. The seeds are the workspace's `default-members` — the set
/// `cargo build` actually visits, which excludes both the host crates and `game` — and everything
/// reachable from them through runtime / build edges. Seeding with every workspace member would make
/// this trivially true, because `game` is a member too.
fn product_crates(workspace: &Path, crates: &[PathBuf]) -> BTreeSet<PathBuf> {
    let manifest = std::fs::read_to_string(workspace.join("Cargo.toml")).unwrap_or_default();
    let seeds: Vec<PathBuf> = crates
        .iter()
        .filter(|crate_dir| {
            let name = crate_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            manifest
                .lines()
                .map(str::trim)
                .skip_while(|line| !line.starts_with("default-members"))
                .take_while(|line| !line.starts_with(']'))
                .any(|line| line.contains(&format!("\"{name}\"")))
        })
        .cloned()
        .collect();
    assert!(
        seeds.len() > 1,
        "workspace `default-members` did not parse into seeds: {seeds:?}"
    );
    closure(seeds, |crate_dir| {
        px_fingerprint::path_dependencies(crate_dir)
            .into_iter()
            .map(|(_, path)| path)
            .collect()
    })
}

#[test]
fn no_roster_carries_a_dev_only_crate() {
    let workspace = workspace_dir();
    let crates = crate_dirs(&workspace);
    let product = product_crates(&workspace, &crates);
    let with_dev = closure(crates.clone(), |crate_dir| {
        let mut edges: Vec<PathBuf> = px_fingerprint::path_dependencies(crate_dir)
            .into_iter()
            .map(|(_, path)| path)
            .collect();
        edges.extend(dev_edges(crate_dir));
        edges
    });
    let dev_only: Vec<PathBuf> = with_dev.difference(&product).cloned().collect();
    assert!(
        !dev_only.is_empty(),
        "no dev-only crate exists, so this gate would watch an empty set: {product:?}"
    );

    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for crate_dir in &crates {
        if !crate_dir.join("src").is_dir() || !product.contains(&normalize(crate_dir)) {
            continue;
        }
        checked += 1;
        let name = crate_dir.file_name().unwrap().to_str().unwrap().to_string();
        for path in px_fingerprint::roster(crate_dir).values() {
            let text = slash(&normalize(path).to_string_lossy());
            for dev_crate in &dev_only {
                let prefix = format!("{}/src/", slash(&normalize(dev_crate).to_string_lossy()));
                if text.starts_with(&prefix) {
                    offenders.push(format!("{name} 的名册里有 {text}"));
                }
            }
        }
    }
    assert!(checked > 5, "only {checked} product roster(s) checked");
    assert!(
        offenders.is_empty(),
        "这些名册收了只经 `[dev-dependencies]` 才够得到的 crate 的源码 ⇒ \
         一个从不进产物的 crate 却在决定键:\n  {}\n  \
         判据：`px_fingerprint::path_dependencies` 只跟 runtime / build 边。",
        offenders.join("\n  ")
    );
}
