//! See docs/programs.md
//!
//! The one stable line every graph entrance prints when it fails, so a caller can branch on the kind
//! of failure instead of matching human prose. The grammar is
//!
//! ```text
//! px-error[<kind>]: <key=value>… | <what>
//! ```
//!
//! The subject is terminated by ` | ` rather than by `: `, because a colon after a `key=value` field
//! cannot be told from a colon inside the value (the criteria in `tests/error_line.rs` caught exactly
//! that, which is why the separator is explicit). The first line is machine-readable and every
//! entrance prints it; further lines are detail and are free-form. `docs/programs.md` carries the
//! grammar, the kind table, and the collision table that explains why a kind is needed at all (one
//! sentence like "读不了" appears at many sites for different causes).

use std::error::Error;

/// The closed set of kinds. A caller may rely on these names; adding one is a documentation change.
pub const KINDS: &[&str] = &[
    "params",
    "operator",
    "missing-instance",
    "stale-toolchain",
    "missing-graph",
    "manifest",
    "library",
    "symbol",
    "payload",
    "shape",
    "write",
    "scene",
    "panic",
    "usage",
    "internal",
];

/// The token a caller greps for, at the start of the line.
pub const PREFIX: &str = "px-error";

/// One stable line. `subject` holds the identity that is already known at the failure site, as
/// `key=value` pairs separated by spaces; pass an empty string when there is none.
pub fn line(kind: &str, subject: &str, what: &str) -> String {
    debug_assert!(KINDS.contains(&kind), "unregistered kind: {kind}");
    if subject.is_empty() {
        format!("{PREFIX}[{kind}]: {what}")
    } else {
        format!("{PREFIX}[{kind}]: {subject} | {what}")
    }
}

/// The identity of a node failure: the node name, the operator id, and a short key. Every one of them
/// is known before `render` runs, so no failure below it has to be anonymous.
pub fn node_subject(node: &str, op: &str, key: &px_graph_schema::Key) -> String {
    format!("node={node} op={op} key={}", crate::hex_short(key))
}

/// Whether a message already carries the stable line, which is how a message that was classified at
/// its source keeps its kind instead of being re-labelled here.
pub fn is_line(message: &str) -> bool {
    let first = message.lines().next().unwrap_or("");
    let Some(rest) = first.strip_prefix(PREFIX) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('[') else {
        return false;
    };
    match rest.split_once("]: ") {
        Some((kind, _)) => KINDS.contains(&kind),
        None => false,
    }
}

/// Prints exactly one stable line and exits non-zero. A message classified at its source passes
/// through; anything else is reported as `internal`, because a failure that arrives here without a
/// kind is one the code never classified. `internal` is a finding, not a resting place.
pub fn report(error: &str) -> ! {
    if is_line(error) {
        eprintln!("{error}");
    } else {
        eprintln!("{}", line("internal", "", error));
    }
    std::process::exit(1);
}

/// Turns a worker panic into a stable line instead of an unhandled abort. The panic still unwinds and
/// the exit code stays the panic one, so a caller can tell a refusal from a crash by exit code alone.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let what = match info.payload().downcast_ref::<&str>() {
            Some(text) => (*text).to_string(),
            None => match info.payload().downcast_ref::<String>() {
                Some(text) => text.clone(),
                None => "panic with a payload that is not text".to_string(),
            },
        };
        let subject = match info.location() {
            Some(at) => format!("at={}:{}", at.file(), at.line()),
            None => String::new(),
        };
        eprintln!("{}", line("panic", &subject, &what));
    }));
}

/// The entry point every graph program uses: installs the panic hook, and reduces whatever the body
/// returns to one stable line. A graph program's body returns `Result` so that a failure is spoken
/// rather than unwound; where it panics anyway, the hook above speaks for it.
pub fn graph_main(body: fn() -> Result<(), Box<dyn Error>>) -> ! {
    install_panic_hook();
    match body() {
        Ok(()) => std::process::exit(0),
        Err(error) => report(&error.to_string()),
    }
}
