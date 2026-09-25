//! See docs/invariants.md
//!
//! Four gates over one mechanism: `px_fingerprint::roster()` is the list identity is computed from,
//! so anything that is compiled but missing from it is "same key, different content". The list is
//! built by walking the **module tree** (`mod`, `#[path]`, `include!`) plus every shader under
//! `src/`; a `.rs` file no declaration reaches is not compiled and is deliberately absent. These
//! gates hold that line from four sides:
//!
//! 1. the roster reaches a dependency's sources (`the_graphs_roster_carries_the_driver`);
//! 2. no `.rs` under `src/` is compiled while missing from the roster
//!    (`no_compiled_rust_file_escapes_the_fingerprint`);
//! 3. a payload-producing source reads no input its key cannot see
//!    (`no_payload_source_reads_an_input_its_key_cannot_see`);
//! 4. an embedded file is in a roster or on the exception list
//!    (`every_embedded_file_is_in_a_roster_or_on_the_exception_list`).

use std::path::Path;

fn slash(key: &str) -> String {
    key.replace(std::path::MAIN_SEPARATOR, "/")
}

#[test]
fn the_graphs_roster_carries_the_driver() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_fingerprint 住在 workspace 下")
        .to_path_buf();
    let roster = px_fingerprint::roster(&workspace.join("px_graphs"));
    for file in ["px_graph/src/driver.rs", "px_graphs/src/lib.rs"] {
        assert!(
            roster.keys().any(|key| slash(key).contains(file)),
            "px_graphs 的名册没递归跟到 {file}：{keys}",
            keys = roster
                .keys()
                .map(|key| slash(key))
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
}

/// The two shapes under `src/` that are allowed to be absent from a roster, because a normal build
/// does not compile them:
///
/// * a `src/tests.rs` or `src/tests/*.rs` tree — the convention pairs it with
///   `#[cfg(test)] mod tests;`, and the collector deliberately does not evaluate `cfg`, so today it
///   does pull those files into a roster (over-collection: a key that can only rotate, never a
///   wrong content);
/// * a file whose name says test (`*_test.rs` / `test_*.rs`), the shape the old name-based
///   collector used to skip silently.
///
/// The convention is what makes an absence legible: `px_protocol/src/rows.rs` sat in `src/` for a
/// long time, compiled by nothing, and its name gave no hint.
fn test_named_source(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !name.ends_with(".rs") {
        return false;
    }
    if name.contains("_test") || name.starts_with("test_") {
        return true;
    }
    let under = |dir: &str| {
        path.parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some(dir)
    };
    under("tests") || (name == "tests.rs" && under("src"))
}

/// The collector walks the module tree now, so the check is its dual: every `.rs` file under `src/`
/// either reaches the roster (it is compiled, hence it must be part of identity) or is named like a
/// test helper (it is not compiled, and the name says so).
///
/// ⚠ `.wgsl` is deliberately not checked here: the collector takes every shader under `src/`
/// whether or not the module tree names it, so a missed shader cannot exist — only an over-collected
/// one, which costs a key without costing correctness.
#[test]
fn no_compiled_rust_file_escapes_the_fingerprint() {
    let workspace = workspace_dir();
    let mut offenders: Vec<String> = Vec::new();
    for crate_dir in fingerprinting_crates(&workspace) {
        let src = crate_dir.join("src");
        if !src.is_dir() {
            continue;
        }
        let roster = px_fingerprint::roster(&crate_dir);
        let tracked: Vec<String> = roster
            .values()
            .map(|path| normalized(&slash(&path.to_string_lossy())))
            .collect();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("读不了目录").flatten() {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if path.is_dir() {
                    if name.starts_with('.') {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs")
                    || test_named_source(&path)
                {
                    continue;
                }
                let absolute = normalized(&slash(&path.to_string_lossy()));
                if !tracked.iter().any(|entry| entry == &absolute) {
                    offenders.push(
                        path.strip_prefix(&workspace)
                            .unwrap_or(path.as_path())
                            .display()
                            .to_string(),
                    );
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "这些 `.rs` 既没进任何指纹名册、名字又不像测试件：{offenders:?}\n  \
         ⇒ 要么它被 `mod` / `#[path]` / `include!` 声明着 **却被名册漏掉**（同键、不同内容），\n  \
         要么它谁都不声明（那它就是 `px_protocol/src/rows.rs` 那种死文件：删掉，或改成\n  \
         `*_test.rs` / `test_*.rs` 并放进 crate 根的 `tests/`）。"
    );
}

/// Calls that make a payload depend on something no key carries. Each entry is a literal needle
/// so the list stays greppable; an identifier such as `random` is deliberately absent because the
/// direction of that check is the inverse (a seeded generator driven by a parameter is legal, and
/// a name-based ban would flag it).
const UNKEYED_READS: &[&str] = &[
    "std::env::var",
    "env::var_os",
    "SystemTime::now",
    "Instant::now",
    "std::process::id",
    "process::id()",
    "thread::current().id()",
    "thread_rng",
    "getrandom",
    "from_entropy",
    "gethostname",
    "as_ptr() as usize",
];

/// One call site inside a file that may read an unkeyed source, and why. `argument` is the literal
/// the call passes, which is also what the scan keys the exception on, so an exception cannot be
/// written for one call and silently cover its neighbour on the same line — the file may hold two.
///
/// ⚠ An entry here is a claim that the read cannot reach a payload that shares a key with a run
/// that did not take this branch. "It is only a debug knob" is not a reason on its own; the
/// measurement reason below is.
struct ExemptRead {
    needle: &'static str,
    path: &'static str,
    argument: &'static str,
    /// Why the read cannot reach a payload that shares a key with a run that did not take this
    /// branch, and — when the read exists for a measurement — which knob it is.
    reason: &'static str,
}

impl ExemptRead {
    /// The pre-image of the call after `rustfmt`, whitespace removed: the scanner compares this
    /// against the stripped line, so the exception does not depend on where a line happened to
    /// break, and an exception written for one call never covers its neighbour on the same line.
    fn call(&self) -> String {
        without_whitespace(&format!("\"{}\").is_ok()", self.argument))
    }
}

const EXEMPT_READS: &[ExemptRead] = &[
    ExemptRead {
        needle: "std::env::var",
        path: "px_volume_gpu_op/src/lib.rs",
        argument: "PX_SKIP_REPORT",
        reason: "diagnostic only: the read selects a block that prints to stderr and returns ()\
                 without touching the payload",
    },
    ExemptRead {
        needle: "std::env::var",
        path: "px_volume_gpu_op/src/lib.rs",
        argument: "PX_SKIP_OFF",
        reason: "deliberate: this is the one knob whose purpose is to make two contents share a\
                 key, so the empty-skip comparison can run against the dense march\
                 (docs/invariants.md, \"The CAS key does not include environment variables\")",
    },
];

/// Which files can produce a payload. Instance bodies live outside every crate's `src/` on
/// purpose — that placement is what makes "edit one body, rotate one key" true — so a scan of
/// `src/` alone would watch the least likely place to fail. `px_fingerprint` is excluded because
/// it only hashes bytes, and the graph programs are excluded because they assemble a graph rather
/// than a payload.
fn payload_source_roots(workspace: &Path) -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(workspace)
        .expect("读不了 workspace")
        .flatten()
        .map(|entry| entry.path())
    {
        if !entry.join("Cargo.toml").is_file() {
            continue;
        }
        let name = entry
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let implementation = (name.starts_with("px_") && name.ends_with("_op"))
            || (name.starts_with("px_") && name.ends_with("_alg"));
        if implementation {
            roots.push(entry.join("src"));
        }
    }
    roots.push(workspace.join("px_elem").join("body"));
    roots.push(workspace.join("art").join("inst"));
    roots
}

fn rust_sources(root: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn workspace_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_fingerprint 住在 workspace 下")
        .to_path_buf()
}

/// Crates that participate in identity: a `px_` prefix keeps `game` out (its manifest is not a
/// product crate's), and `px_fingerprint` itself has no roster to check a target against.
fn fingerprinting_crates(workspace: &Path) -> Vec<std::path::PathBuf> {
    let mut crates: Vec<std::path::PathBuf> = std::fs::read_dir(workspace)
        .expect("读不了 workspace")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.join("Cargo.toml").is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("px_"))
        })
        .collect();
    crates.sort();
    crates
}

/// The bytes a line would have after `rustfmt`, so an exception can be written naturally and still
/// match the source (the two files it names are formatted, but a wrapped call would otherwise
/// depend on where the line happened to break).
fn without_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The neighbourhood of one occurrence, as a pre-image candidate: 80 characters either side of the
/// needle, floored to `char` boundaries because a comment on the same line can be non-ASCII. The
/// window is wide enough for the call's argument on either side of the needle, which is what the
/// exception is keyed on; a line holding two reads is then resolved per occurrence.
fn call_context(line: &str, at: usize) -> String {
    let mut start = at.saturating_sub(80);
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (at + 80).min(line.len());
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    without_whitespace(&line[start..end])
}

#[test]
fn no_payload_source_reads_an_input_its_key_cannot_see() {
    let workspace = workspace_dir();
    let mut scanned = Vec::new();
    for root in payload_source_roots(&workspace) {
        rust_sources(&root, &mut scanned);
    }
    scanned.sort();
    for wanted in [
        "px_volume_gpu_op/src/lib.rs",
        "px_elem/body/remap.rs",
        "art/inst/latbands.rs",
    ] {
        let full = workspace.join(wanted);
        assert!(
            scanned.iter().any(|path| path == &full),
            "扫描面漏了 {wanted} ⇒ 这道门看的是空集也会绿"
        );
    }

    let mut violations: Vec<String> = Vec::new();
    for path in &scanned {
        let relative = path
            .strip_prefix(&workspace)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        for (number, line) in text.lines().enumerate() {
            for needle in UNKEYED_READS {
                let Some(at) = line.find(needle) else {
                    continue;
                };
                if EXEMPT_READS.iter().any(|exempt| {
                    relative == exempt.path && call_context(line, at).contains(&exempt.call())
                }) {
                    continue;
                }
                violations.push(format!(
                    "{relative}:{} 读 {needle} ⇒ {}",
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    for exempt in EXEMPT_READS {
        let path = workspace.join(exempt.path);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("豁免指着 {} 却读不了：{err}", exempt.path));
        // The exemption is a claim about a call site that exists, not about a file: a list copied
        // forward after the call moved reads as coverage while covering nothing. Whether the file's
        // *other* reads are covered is the main scan's question, asked with all exemptions at once.
        let matched = text
            .lines()
            .filter_map(|line| line.find(exempt.needle).map(|at| call_context(line, at)))
            .filter(|context| context.contains(&exempt.call()))
            .count();
        assert!(
            matched > 0,
            "死豁免：{} 里没有一处 `{}` 的上下文等于 {}（调用改了、挪走了，或豁免是照抄来的）",
            exempt.path,
            exempt.needle,
            exempt.call()
        );
    }

    assert!(
        violations.is_empty(),
        "这些产出载荷的源码读了键看不见的输入 ⇒ 同一个键两次运行可以给出两种内容，\
         第二次覆盖第一次而清单自洽（docs/invariants.md）:\n  {}\n  \
         修法：把那个输入变成参数（进接口哈希），或把它写进 EXEMPT_READS 并给出\
         \u{201c}为什么它到不了载荷\u{201d}的理由。",
        violations.join("\n  ")
    );
}

/// The reasons are prose, but the list is not: an entry whose reason does not name a mechanism
/// (a diagnostic sink, a deliberate measurement knob) is the one place this rule gets lost. Both
/// exception tables are checked here, so a new entry cannot ship as a bare path.
#[test]
fn every_exemption_names_its_mechanism() {
    let mechanisms = ["diagnostic", "deliberate", "parameter"];
    for exempt in EXEMPT_READS {
        assert!(
            mechanisms.iter().any(|word| exempt.reason.contains(word)),
            "{} 的豁免理由没有点名机制：{}",
            exempt.path,
            exempt.reason
        );
    }
    for exempt in EXEMPT_EMBEDS {
        assert!(
            exempt.reason.contains("consumer"),
            "{} 的豁免理由没有说清谁消费它：{}",
            exempt.path,
            exempt.reason
        );
        assert!(
            exempt.reason.contains("Expiry"),
            "{} 的豁免没有写失效条件（没有失效条件的豁免会一直留着）：{}",
            exempt.path,
            exempt.reason
        );
    }
}

/// One embedded file that no roster carries, and the mechanism that makes it safe. `consumer` is
/// the one symbol whose reads are allowed to reach the embedded bytes; the expiry is the check the
/// fourth gate runs against that symbol rather than a note in the reason.
struct ExemptEmbed {
    path: &'static str,
    consumer: &'static str,
    reason: &'static str,
}

const EXEMPT_EMBEDS: &[ExemptEmbed] = &[ExemptEmbed {
    path: "px_protocol/snapshots/protocol.snapshot.json",
    consumer: "PROTOCOL_SNAPSHOT",
    reason: "its single consumer is `protocol_hash()`, which enters `ProtocolId` and is compared \
             value-by-value by `Handshake::verify`: two different snapshots refuse to communicate \
             instead of silently exchanging wrong content. `.gitattributes` marks the path `-text` \
             so the bytes are stable everywhere. ⚠ Expiry: a second consumer — anything that \
             derives payloads, artifacts or keys from it — ends the exemption and puts the file in \
             a roster.",
}];

/// Collapses `.` and `..` without touching the filesystem: the roster keys are built from the
/// paths the collector walked, so a resolved embed target has to look like one of those to be
/// compared with them (`src/../snapshots/x.json` is the same file as `snapshots/x.json`, and a
/// `canonicalize` would add a `\\?\` prefix on this platform while leaving symlinks a second
/// spelling).
fn normalized(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.join("/")
}

/// The literal path of one `include_str!`/`include_bytes!` in a file, if the macro is given a
/// literal. A `concat!(env!("OUT_DIR"), …)` form is invisible here by construction: the target is
/// generated rather than checked in, and rustc records it as a dependency of the crate anyway.
///
/// ⚠ The search runs over a copy whose comment bodies and string contents are blanked out. Without
/// that, this gate's own failure message — which quotes `include_str!` inside a string literal —
/// reads as a real embed site and the gate reports against itself.
fn embedded_targets(text: &str) -> Vec<String> {
    let code = blank_comments_and_strings(text);
    let mut targets = Vec::new();
    for macro_name in ["include_str!", "include_bytes!"] {
        let mut from = 0usize;
        while let Some(offset) = code[from..].find(macro_name) {
            let at = from + offset;
            if let Some(path) = quoted_after(text, at + macro_name.len()) {
                targets.push(path);
            }
            from = at + macro_name.len();
        }
    }
    targets
}

fn quoted_after(text: &str, from: usize) -> Option<String> {
    let open = text[from..].find('"')?;
    let tail = &text[from + open + 1..];
    let close = tail.find('"')?;
    Some(tail[..close].to_string())
}

fn blank(out: &mut [u8], at: usize, width: usize) {
    for byte in out.iter_mut().skip(at).take(width) {
        *byte = b' ';
    }
}

/// A copy of `text` with comment bodies and string/char contents replaced by spaces, keeping the
/// quote characters themselves so offsets stay usable.
fn blank_comments_and_strings(text: &str) -> String {
    let mut out = text.as_bytes().to_vec();
    let mut index = 0usize;
    let mut line_comment = false;
    let mut block_comment = false;
    let mut string = false;
    while index < text.len() {
        let ch = text[index..].chars().next().unwrap_or(' ');
        let width = ch.len_utf8();
        let next = text[index + width..].chars().next();
        if line_comment {
            if ch == '\n' {
                line_comment = false;
            } else {
                blank(&mut out, index, width);
            }
        } else if block_comment {
            if ch == '*' && next == Some('/') {
                block_comment = false;
                blank(&mut out, index, width);
                blank(&mut out, index + width, 1);
                index += width + 1;
                continue;
            }
            blank(&mut out, index, width);
        } else if string {
            if ch == '\\' {
                blank(&mut out, index, width);
                let escaped = next.map_or(1, char::len_utf8);
                blank(&mut out, index + width, escaped);
                index += width + escaped;
                continue;
            }
            if ch == '"' {
                string = false;
            } else {
                blank(&mut out, index, width);
            }
        } else if ch == '/' && next == Some('/') {
            line_comment = true;
            index += width;
            continue;
        } else if ch == '/' && next == Some('*') {
            block_comment = true;
            index += width;
            continue;
        } else if ch == '"' {
            string = true;
        }
        index += width;
    }
    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}

/// Counts the reads of one symbol over the same files identity is computed from, so the exemption's
/// expiry ("no second consumer") is a check and not a note. Over-counting cannot hide a consumer:
/// the assertion below only cares that the count is exactly the one reader named by the exception.
fn symbol_reads(workspace: &Path, symbol: &str) -> Vec<String> {
    let mut readers = Vec::new();
    for crate_dir in fingerprinting_crates(workspace) {
        let mut files = Vec::new();
        rust_sources(&crate_dir.join("src"), &mut files);
        let build = crate_dir.join("build.rs");
        if build.is_file() {
            files.push(build);
        }
        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if text.contains(symbol) {
                readers.push(
                    path.strip_prefix(workspace)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    readers
}

/// Backlog's second designed gate. The hole is not hypothetical in shape: `collect_tree` accepts
/// only `.rs` and `.wgsl`, so any other file type reaches a binary through `include_str!` /
/// `include_bytes!` while no roster names it, and editing it then changes behaviour without
/// changing any key.
#[test]
fn every_embedded_file_is_in_a_roster_or_on_the_exception_list() {
    let workspace = workspace_dir();
    let mut violations: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for crate_dir in fingerprinting_crates(&workspace) {
        if !crate_dir.join("src").is_dir() {
            continue;
        }
        let roster = px_fingerprint::roster(&crate_dir);
        let tracked: Vec<String> = roster
            .values()
            .map(|path| slash(&path.to_string_lossy()))
            .collect();
        let crate_name = crate_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_string();
        let mut files = Vec::new();
        rust_sources(&crate_dir.join("src"), &mut files);
        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let source = path
                .strip_prefix(&workspace)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            for target in embedded_targets(&text) {
                checked += 1;
                let resolved = normalized(
                    &path
                        .parent()
                        .unwrap_or(&crate_dir)
                        .join(&target)
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
                let relative = normalized(
                    &Path::new(&resolved)
                        .strip_prefix(&workspace)
                        .unwrap_or(Path::new(&resolved))
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
                let present = Path::new(&resolved).is_file();
                let in_roster = tracked
                    .iter()
                    .any(|entry| entry == &resolved || entry.ends_with(&format!("/{target}")));
                let exempt = EXEMPT_EMBEDS.iter().any(|exempt| relative == exempt.path);
                if !present {
                    violations.push(format!(
                        "{source} 嵌入 {target}，但 {resolved} 不在（路径写错或文件被删）"
                    ));
                } else if !in_roster && !exempt {
                    violations.push(format!(
                        "{source} 嵌入 {relative}，而 {crate_name} 的名册里没有它 ⇒ \
                         改这个文件会换行为不换键"
                    ));
                }
            }
        }
    }
    for exempt in EXEMPT_EMBEDS {
        let readers = symbol_reads(&workspace, exempt.consumer);
        assert!(
            readers.len() == 1,
            "豁免失效：`{}` 的消费者现在有 {} 个（{}）—— 该文件必须进名册，豁免到此结束",
            exempt.consumer,
            readers.len(),
            readers.join(" / ")
        );
        assert_eq!(
            readers[0], "px_protocol/src/lib.rs",
            "`{}` 的消费者挪到了 {}：豁免说的是 `protocol_hash()` 那一处",
            exempt.consumer, readers[0]
        );
        assert!(
            workspace.join(exempt.path).is_file(),
            "豁免指着 {}，而它不在",
            exempt.path
        );
    }
    assert!(checked > 0, "一个嵌入点都没扫到 ⇒ 这道门看的是空集也会绿");
    assert!(
        violations.is_empty(),
        "这些文件被 `include_str!`/`include_bytes!` 带进二进制，却不在任何名册里（`collect_tree` \
         只收 `.rs`/`.wgsl`）⇒ 同键、不同内容:\n  {}\n  \
         修法：把它放进该 crate 的 `src/`（.rs/.wgsl），或写进 EXEMPT_EMBEDS 并给出\
         \u{201c}谁消费它、为什么失败会响\u{201d}的理由，以及它的失效条件。",
        violations.join("\n  ")
    );
}
