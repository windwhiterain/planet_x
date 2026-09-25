//! See docs/programs.md
//!
//! The stable failure line is a contract with callers, so it is pinned here: the token, the closed
//! kind set, and the identity fields a caller branches on. The gates themselves are driven
//! end-to-end in `px_graphs/tests/gate_ready.rs` and by the `px` command's own output.

use px_cook::fault;
use px_cook::inst::{Missing, Plan};

/// The documented grammar, checked without a regex dependency: `px-error[<kind>]: ` at the start of
/// the first line, kind limited to `[a-z][a-z-]*`.
fn parsed_kind(message: &str) -> Option<&str> {
    let first = message.lines().next()?;
    let rest = first.strip_prefix(fault::PREFIX)?.strip_prefix('[')?;
    let (kind, _) = rest.split_once("]: ")?;
    let mut chars = kind.chars();
    let head = chars.next()?;
    if !head.is_ascii_lowercase() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c == '-') {
        return None;
    }
    Some(kind)
}

/// A field of the subject, as `name=value`, which is how a caller reads identity out of the line.
/// The subject ends at ` | `, so a value never swallows the separator.
fn field<'a>(message: &'a str, name: &str) -> Option<&'a str> {
    let first = message.lines().next()?;
    let subject = first.split_once(" | ").map_or(first, |(head, _)| head);
    let mut rest = subject;
    let needle = format!("{name}=");
    while let Some(at) = rest.find(&needle) {
        let start = at + needle.len();
        let tail = &rest[start..];
        let end = tail.find(' ').unwrap_or(tail.len());
        let value = &tail[..end];
        // `node=` must not match the tail of another field (there is no such field today, so this is
        // the guard that keeps a future `subnode=` from being read as `node=`).
        let preceded_by_field = at == 0 || rest[..at].ends_with(' ');
        if preceded_by_field && !value.is_empty() {
            return Some(value);
        }
        rest = &rest[start..];
    }
    None
}

#[test]
fn every_kind_is_lowercase_and_registered() {
    for kind in fault::KINDS {
        assert!(
            parsed_kind(&fault::line(kind, "", "why")) == Some(*kind),
            "kind `{kind}` does not survive the documented grammar"
        );
    }
    let mut sorted = fault::KINDS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), fault::KINDS.len(), "a kind is listed twice");
}

#[test]
fn a_node_failure_carries_the_node_the_op_and_the_key() {
    let key = [0xab_u8; 32];
    let subject = fault::node_subject("weight", "field.constant", &key);
    let message = fault::line("operator", &subject, "值域反了");
    assert_eq!(parsed_kind(&message), Some("operator"));
    assert_eq!(field(&message, "node"), Some("weight"));
    assert_eq!(field(&message, "op"), Some("field.constant"));
    assert_eq!(field(&message, "key"), Some("abababababab"));
}

#[test]
fn a_message_classified_at_its_source_keeps_its_kind() {
    // The gate a caller meets most often, and the one that must not be re-labelled on the way out.
    let plan = Plan {
        total: 7,
        present: 6,
        missing: vec![Missing {
            op_id: "field.constant".to_string(),
            key: "1".repeat(64),
            library: "target/pcg/inst/x.dll".to_string(),
        }],
    };
    let hint = plan.hint("planet");
    assert_eq!(parsed_kind(&hint), Some("missing-instance"));
    assert_eq!(field(&hint, "graph"), Some("planet"));
    assert!(fault::is_line(&hint));
    assert!(
        hint.contains("共 7 条"),
        "the count stays in the detail: {hint}"
    );
}

#[test]
fn prose_that_was_never_classified_does_not_parse() {
    // The line is what makes a failure branchable; a bare sentence must not look like one, and the
    // fallback label is what an unclassified failure gets instead.
    assert!(!fault::is_line("参数解不开：TOML parse error at line 1"));
    assert!(!fault::is_line("px-error[nosuchkind]: x"));
    let fallback = fault::line("internal", "", "who knows");
    assert_eq!(parsed_kind(&fallback), Some("internal"));
}
