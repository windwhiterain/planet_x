# Invariants

Every rule here is load-bearing. Most were paid for with a bug that took a while to find.
Read this before changing anything.

## Identity and keys

**A key is content plus identity.** Drop one axis and you get "same key, different content".
Cache hits are then silently wrong, which is the worst failure mode this repository has.

**Editing a comment changes the key.** `px_fingerprint` hashes the full text of every `.rs` and
`.wgsl` in a crate's `src/`, plus every reachable path dependency's. The crates whose fingerprints
reach a key include `px_fingerprint`, `px_graph_schema`, `px_elem`, `px_graphs`, the `px_*_schema`
crates, the `px_*_alg` crates and the `px_*_op` crates.

Two of these are easy to get wrong:

- **`px_graphs` does participate**, through `px_graphs/build.rs`: its own fingerprint is the identity
  of `px_local_op!` nodes.
- **`px_elem` does participate**, through its `build.rs` declaration hash, which enters instance
  keys.

⚠ **`px_cook` and `px_decls` are inside `px_graphs`' roster**, because that roster is taken over
`[build-dependencies]` and therefore pulls in both crates' sources. They currently reach no key only
because nothing under `px_graphs/src` uses `px_local_op!` — the only uses are in
`px_graphs/tests/local_op.rs`, and tests are excluded from a roster. **The first local operator added
to a shipped graph makes editing a byte of `px_cook` or `px_decls` rotate that node's key.** Do not
treat them as free.

Consequence: **a repository-wide comment edit is never free** — count the files first.

**A shader's content includes its `#import` closure.** An entry file whose text did not change is
still a different key if a module it imports changed. On load the renderer recomputes the closure
from the artifact's WGSL against the modules on disk and compares it with the fingerprint recorded
in the artifact; a mismatch is refused outright, with instructions for re-cooking.

**A key change is not a content change.** Identity (source bytes) is one half of the key, so a
comment edit changes every key while every output stays identical. Do not use "the key changed" as
evidence that output changed, and do not use "the cache hit" as evidence that it did not — that
only holds when the dependencies were not rebuilt.

**One rebuild is an identity event.** Instance keys include the toolchain axis (
`rustc -vV` full text, `TARGET`, `RUSTFLAGS`, `PROFILE`), so recompiling the same sources under
another profile rotates every instance key with the sources byte-identical — and leaves the old
libraries on disk under their old names. Inside a measurement or bake window, treat a rebuild as a
key rotation and re-open the window.

**Nothing that participates in a key may be edited casually.** If it must change, expect a
repository-wide key rotation: rebuild every operator dylib, and re-cook. A warning sign of a
half-done rotation: at load time, `…dll is not from the same build as the contract`.

## Verdicts and instruments

**A verdict must be taken on the right instrument.** `cargo check` passing is not a probe passing.
A low-resolution render measures the floor, not the shader. Operator counts find suspects, they
are not a score. Read logs in full; tailing the last twelve lines hides the error that matters.

**A skipped check is not a passing check.** A probe that cannot get a device exits non-zero. A
test that cannot find its target must say so loudly — a `skip` line printed among a column of
`ok`s has already hidden a dead verdict for a full round.

**A measurement whose output did not change measured nothing.** Content-addressing makes an edit
invisible when it produces the same key: re-applying one fixed value to a parameter file cooks once
and hits forever after, with the run succeeding, the parameter index recording the new value, and the
artifact reused. Nothing reports it. So an increment or repair loop must vary its input each round and
assert the output differs (a byte count or digest of the produced image is enough), rather than timing
a repeated action and reading a hit as speed.

**A measurement must assert its own output exists.** Check that the artefact a
step was asked to produce is there and non-empty, rather than trusting that the
step ran. A render client with no service running prints how to start one and
exits 1 (`px_render/src/client.rs::request` -> `std::process::exit`,
main.rs:789), so the tool itself is honest; what still went wrong is a harness
that shells out without reading the status. The failing invocation also returns
quickly, so a harness watching elapsed time can mistake "did nothing" for "got
faster"; the missing artefact stays invisible until someone opens the image.



**`git status` cannot see line-ending changes.** CRLF and LF clean to the same blob. "Git says I am
clean" and "the bytes did not change" are different statements.

**Do not run the full test suite out of habit.** Run what the change affects. Do not write
redundant tests. A check that costs real computation belongs in a probe, not in `cargo test`.

**Format before committing.** `./format.sh`. If it touches files beyond your edit, commit those
too.
