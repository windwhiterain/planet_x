//! See docs/programs.md
//!
//! The kinds a failure can be, and the value that carries one. This is the canonical home of the
//! closed set: `px_cook::fault` builds its lines from here, and the graph entrances print them as
//! `px-error[<kind>]: … | …`. A caller branches on `Kind` instead of matching prose.
//!
//! The shape is the enum plus a message, rather than one enum variant per kind carrying a `String`:
//! the closed set is the thing a caller matches on, and this keeps it in one place (the variants)
//! with one mapping to the wire name, instead of a variant list and a name list that can drift apart.

use std::fmt;

/// The closed set of kinds. Adding one is a documentation change: `docs/programs.md` carries the
/// table and the grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A parameter file or parameter value the operator refused.
    Params,
    /// The operator body itself failed.
    Operator,
    /// The plan wants an instance library that is not built.
    MissingInstance,
    /// A present library was recorded by another build.
    StaleToolchain,
    /// A graph executable that does not exist.
    MissingGraph,
    /// Reading or looking up a manifest entry.
    Manifest,
    /// Loading, opening or identifying an implementation library.
    Library,
    /// A symbol the loader needs is not in the library.
    Symbol,
    /// A payload that will not decode or encode.
    Payload,
    /// A shape that does not match its declaration.
    Shape,
    /// Writing an artifact, sidecar or index.
    Write,
    /// Reading a scene document.
    Scene,
    /// A worker panicked where a panic has no route back to the caller.
    Panic,
    /// The entrance was called wrongly: unknown subcommand or flag, missing argument or graph name.
    Usage,
    /// The failure reached an entrance without having been classified. Seeing one is a finding.
    Internal,
}

impl Kind {
    /// Every kind, in the order `docs/programs.md` lists them.
    pub const NAMES: &'static [&'static str] = &[
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

    /// The wire name, which is what appears between the brackets.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Params => "params",
            Kind::Operator => "operator",
            Kind::MissingInstance => "missing-instance",
            Kind::StaleToolchain => "stale-toolchain",
            Kind::MissingGraph => "missing-graph",
            Kind::Manifest => "manifest",
            Kind::Library => "library",
            Kind::Symbol => "symbol",
            Kind::Payload => "payload",
            Kind::Shape => "shape",
            Kind::Write => "write",
            Kind::Scene => "scene",
            Kind::Panic => "panic",
            Kind::Usage => "usage",
            Kind::Internal => "internal",
        }
    }

    /// The kind a wire name denotes, or `None` for a name outside the set. A caller that receives a
    /// failure line from elsewhere reads its kind back with this.
    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::NAMES
            .iter()
            .position(|known| *known == name)
            .map(|index| Kind::ALL[index])
    }

    /// Every variant, indexed the same way as `NAMES` — the one place the two orders are tied.
    const ALL: [Kind; Kind::NAMES.len()] = [
        Kind::Params,
        Kind::Operator,
        Kind::MissingInstance,
        Kind::StaleToolchain,
        Kind::MissingGraph,
        Kind::Manifest,
        Kind::Library,
        Kind::Symbol,
        Kind::Payload,
        Kind::Shape,
        Kind::Write,
        Kind::Scene,
        Kind::Panic,
        Kind::Usage,
        Kind::Internal,
    ];
}

impl fmt::Display for Kind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// A classified failure: the kind a caller branches on, and the sentence a person reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub kind: Kind,
    pub message: String,
}

impl Fault {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A failure that was never classified. The entrance's fallback, and a finding wherever it
    /// appears.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Kind::Internal, message)
    }

    pub fn library(message: impl Into<String>) -> Self {
        Self::new(Kind::Library, message)
    }

    pub fn symbol(message: impl Into<String>) -> Self {
        Self::new(Kind::Symbol, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(Kind::Usage, message)
    }

    /// The stable line: `px-error[<kind>]: <message>`. The kind comes first so a caller can grep the
    /// first line and branch without reading the sentence.
    pub fn line(&self) -> String {
        format!("px-error[{}]: {}", self.kind.name(), self.message)
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.line())
    }
}

impl std::error::Error for Fault {}

/// A bare `String` carries no kind, so it becomes the unclassified one. Callers that know the kind
/// construct the `Fault` themselves.
impl From<String> for Fault {
    fn from(message: String) -> Self {
        Fault::internal(message)
    }
}

impl From<&str> for Fault {
    fn from(message: &str) -> Self {
        Fault::internal(message)
    }
}

/// The stable line, for the crate boundaries that still speak `String` (see `docs/backlog.md`, the
/// gradual migration to `Fault`).
impl From<Fault> for String {
    fn from(fault: Fault) -> Self {
        fault.line()
    }
}
