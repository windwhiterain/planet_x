//! See docs/programs.md
//!
//! The closed kind set and the value that carries one: the contract every entrance's failure line is
//! built from, and the loader's promise that a failure is a value rather than an unwind.

use px_graph_schema::{Fault, Kind};

/// The grammar's head, without a regex dependency: `px-error[<kind>]: ` and nothing before it.
fn line_kind(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("px-error[")?;
    let (kind, _) = rest.split_once("]: ")?;
    Some(kind)
}

#[test]
fn every_kind_is_a_grammar_word_and_reads_back() {
    for name in Kind::NAMES {
        assert!(
            name.chars().next().is_some_and(|c| c.is_ascii_lowercase()),
            "kind `{name}` must start lowercase"
        );
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "kind `{name}` must be [a-z-] only"
        );
        let kind = Kind::from_name(name).unwrap_or_else(|| panic!("`{name}` is not readable back"));
        assert_eq!(kind.name(), *name);
        assert_eq!(
            line_kind(&Fault::new(kind, "why").line()),
            Some(*name),
            "`{name}` does not survive the line"
        );
    }
    let mut sorted = Kind::NAMES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), Kind::NAMES.len(), "a kind is listed twice");
    assert_eq!(Kind::NAMES.len(), 15, "the set is closed at fifteen");
    assert!(
        Kind::from_name("nonesuch").is_none(),
        "unknown names are not kinds"
    );
    // The three that a caller's reaction differs most on: they must stay distinguishable.
    for name in ["operator", "usage", "internal"] {
        assert!(Kind::from_name(name).is_some(), "`{name}` must be a kind");
    }
}

#[test]
fn a_string_carries_no_kind_and_the_line_carries_it_out() {
    let bare = Fault::from("something went wrong".to_string());
    assert_eq!(
        bare.kind,
        Kind::Internal,
        "an unclassified message is internal"
    );
    assert_eq!(bare.line(), "px-error[internal]: something went wrong");
    let classified = Fault::library("找不到实现库 px_no_such");
    assert_eq!(
        String::from(classified.clone()),
        classified.line(),
        "the String boundary keeps the line, kind included"
    );
    assert!(String::from(classified).starts_with("px-error[library]: "));
}

#[test]
fn a_library_that_cannot_be_opened_is_a_fault_not_a_panic() {
    let err = px_graph_schema::ops::source_hash("px_no_such_operator_library")
        .expect_err("a library that is not on disk must be an Err");
    assert_eq!(err.kind, Kind::Library);
    assert!(
        err.message.contains("px_no_such_operator_library"),
        "the refusal names the library: {}",
        err.message
    );
    assert_eq!(line_kind(&err.line()), Some("library"));
}

/// The operator-library table's lock refuses instead of unwrapping. `Mutex::lock` returns `Err` only
/// after a thread panicked while holding it, so the fixture poisons a lock of the same shape and
/// drives the loader's own conversion with the real error.
#[test]
fn a_poisoned_lock_is_refused_rather_than_unwrapped() {
    let mutex = std::sync::Mutex::new(0_u32);
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = mutex.lock().expect("第一次上锁");
        panic!("持锁时 panic ⇒ 锁中毒");
    }));
    assert!(
        poisoned.is_err(),
        "the fixture must actually poison the lock"
    );
    let error = mutex.lock().err().expect("锁应当已经中毒");
    let fault = px_graph_schema::ops::lock_error(error);
    assert_eq!(fault.kind, Kind::Internal);
    assert_eq!(line_kind(&fault.line()), Some("internal"));
    assert!(
        fault.message.contains("锁"),
        "the refusal names what broke: {}",
        fault.message
    );
}
