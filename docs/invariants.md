# Invariants

Every rule here is load-bearing. Most were paid for with a bug that took a while to find.
Read this before changing anything.

## Identity and keys

**A key is content plus identity.** Drop one axis and you get "same key, different content".
Cache hits are then silently wrong, which is the worst failure mode this repository has.

**Editing a comment changes the key.** `px_fingerprint` hashes the full text of the Rust files a
crate's module tree declares, every `.wgsl` under its `src/`, its `build.rs`, and the same for every
reachable path dependency taken over a runtime or build edge (`[dev-dependencies]` is not compiled
into the artifact, so it is not followed). The crates whose fingerprints reach a key include
`px_fingerprint`, `px_graph_schema`, `px_elem`, `px_graphs`, the `px_*_schema` crates, the `px_*_alg`
crates and the `px_*_op` crates.

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

⚠ **`px_graph` is in the same position, and the four crates that carry it pay for edits to it.**
`px_graph` / `px_graphs` / `px_cook` / `px_probe` each hold `px_graph`'s sources in their roster, so
editing `px_graph/src` (the driver, the measurement ledger, the CAS) moves those four crates' own
`PX_SOURCE_HASH`. No instance key moves — no instance's roots reach `px_graph` — and no node key
moves either, for the same reason `px_cook` and `px_decls` are free: no shipped graph has a
`px_local_op!`. It becomes a key rotation the day one does. Free today is not free forever.

Consequence: **a repository-wide comment edit is never free** — count the files first.

**A *criterion* on a crate that participates in a fingerprint belongs in the crate-root `tests/`**,
because that directory is walked by no roster, while adding a file under `src/` would rotate a key. A
*production* gate still lives in `src/`, where the binaries can call it: the toolchain gate's runtime
half is `px_graphs::assert_toolchain_matches` in `src/lib.rs`, and its five criteria are in
`tests/toolchain_gate.rs`. `px_render` is the exception, and it is the only one: it is a bin-only host
that appears in no roster, no instance closure and no dependency list, so **its criteria** live in an
in-file `#[cfg(test)] mod tests` (as 17 of its source files already do) and rotating nothing is not at
stake. **Do not copy that shape onto a crate whose key face is not zero** — there the in-file module is a
key rotation waiting to be committed.

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
**Read bytes, not views.** Where a file's text is *assembled* into something else (a WGSL string
assembled from modules and continuation lines, an include!), the verdict is taken on the source
blob (`git show <rev>:<path>`, or the file itself) — never on the assembled view, where two
different sources can concatenate to the same text and a taken-out blank line disappears into an
adjacent continuation.

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
exits 1 (`px_render/src/client.rs::request` -> `std::process::exit` in `main`,
`px_render/src/main.rs`), so the tool itself is honest; what still went wrong is a harness
that shells out without reading the status. The failing invocation also returns
quickly, so a harness watching elapsed time can mistake "did nothing" for "got
faster"; the missing artefact stays invisible until someone opens the image.

⚠ **A GPU-dependent test that prints "no device, skipping" and returns is a policy violation, not a
convenience.** `px_volume_gpu_op` and `px_nurbs_gpu_op` are default members, so on a machine without
a GPU the fast chain reports green while the GPU comparisons never executed. Either require the
device (fail, do not return) or take the check out of the default chain. Inconsistency inside one
crate — one test deliberately panicking while its neighbours skip — is how this rule gets lost.

**A diagnostic is not a verdict.** Some binaries under `px_probe` are called probes and exit 0
without asserting anything; they print measurements for a human. Do not cite their exit code as
evidence.

**One target, one reader.** When a verdict's fixture moves, change the path that reads it in the
same commit, and run it once to confirm it is not skipping. Moving the fixture and leaving the
reader behind is how a verdict dies silently.

**A verdict that can be deleted is not a verdict.** Anything under `target/` can vanish to a
`rm -rf`, so fixtures live in git and instruments do not. A fixture nobody reads is not a verdict
either, and should be deleted rather than kept for comfort.

**Fixtures and stubs must not shield the thing under test.** Text compiled for a probe must be the
same text the real host compiles. Two GPU backends is worse than one: bake and render must run on
the same backend.

**Performance comparisons must be paired.** A single A→B measurement has shown 10.43 ms and 5.49 ms
for the same executable with a different load in between. Alternate A/B/A/B.

**Evidence is graded.** A measured number or an official source can be cited directly. Anything
unverified must be re-checked before it is used.

## Protocol and compatibility

**One source for the mapping between parameter names and bytes.** Today that source is the shader
text (reflected with naga). Moving it elsewhere requires generating the WGSL or adding a gate, or
you end up with two truths that drift.

**Widen by appending only.** Never move an existing binding slot. Moving one silently points an
existing shader's texture at a different slot.

**What a recipe may specify is decided by the shader contract, not by a Rust whitelist.** So adding
a parameter is: declare it in WGSL, give it a value in the artifact, make the names match. Names
must match exactly in both directions — a missing or extra parameter is refused at load.

**Fail at cook time, not at load time.** A misspelled name, a missing value, a wrongly shaped
value — all three are reported while cooking, not when the client connects.

**A schema descriptor is baked with the artifact and checked on load.** Changing reflection rules
without changing the key leaves two contracts under one key; the loader refuses. Reflection rule
changes must also bump `SHADER_VERSION`, which `px_graph` owns (`px_graph::SHADER_VERSION`) and
`px_cook` re-exports.

**The snapshot JSON is sorted and machine-written.** Hand-editing it is always wrong; regenerate it
with `PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol`. A change to the snapshot changes
`protocol_hash`, which is the point: a running peer with the old shape is refused at handshake.

**Node names in a graph are not free to rename.** Diffing pairs nodes by name; a rename reads as
one removal and one addition.

## GPU

**The backend is not optional.** Every `wgpu` dependency is `default-features = false` with
explicit features. The defaults pull in `dx12`, `metal`, `gles` and `webgpu`, all of which this
project has rejected.

**`std` must be named explicitly.** With `default-features = false`, omitting `std` makes wgpu-core
use its no-std synchronisation path. The symptom looks like a race — `Mismatched pop_error_scope
call: error scopes must be popped in reverse order`, with a different number of failures each run —
but it is a configuration difference, not a race.

**Feature lists are part of the product contract.** After `default-features = false`, name every
feature, and make sure what you declared stands on its own rather than depending on another crate
happening to unify the missing piece in.

## Volume rendering

**GPU is the single implementation.** Marching lives in `px_volume_gpu_op` and `src/sampler.wgsl`.
Rendering, probes, and readings go through it. `px_volume_alg` holds a CPU copy that exists only as
a draft and as a test oracle; no operator and no graph may take the CPU path, and no feature may be
added that exists only on the CPU.

**The occupancy mask has one source and three readers.** `from_emission` writes it, `at_cell` reads
the internal layout, and the WGSL `occupancy_class` reads the upload layout. The internal storage
(one word per cell in `sidecar`, `WORDS_PER_BLOCK` words per block in `words`) is not the same as
the upload layout (`[L1 bits][L2 mask]`). Change one and you must change all three.

`WORDS_PER_BLOCK = 4³/32 = 2`: the L2 mask exactly fills two `u32`s, so the L1 bits **must** occupy
their own section. They cannot be folded into "word 0 of each block".

**Crossing a coarse-block boundary must actually cross it.** `layer_radius` is a `pow`, and
`layer_index` differing by one ulp at a boundary sends the cursor back to the previous block — the
cursor spins in place and every ray near a block seam goes black.

**The CAS key does not include environment variables.** To measure a feature toggled by an
environment variable, vary a *parameter* too, or the second run is a cache hit and you measure disk
read time instead.

**A node key must determine its content, not merely exclude inputs.** The rule above is about what
the hash leaves out; the obligation on the other side is stronger. Nothing that produces a payload
may read a source the key cannot see — no environment variable, no process id, no wall clock, no RNG
that is not seeded from a parameter. When one of those enters an operator, two runs of the same key
differ, the second overwrites the first in the CAS, and the manifest stays self-consistent, so nothing
reports it. Every repair, retry and search loop rests on this: re-cooking a key is expected to give
back what is already stored unless something real changed.

**A failure is a value with a kind, and the kind has one home.** `px_graph_schema::Fault` carries a
`Kind` (the closed set of fifteen) plus the sentence a person reads, and `Display` writes the stable
line `px-error[<kind>]: <message>`. The contract crate owns the set because every entrance's first
failure passes through its loader, and because a caller that branches on a kind must be able to depend
on that name without depending on a graph program. A bare `String` converts to `Fault::Internal` — the
unclassified kind, whose appearance is a finding — and `From<Fault> for String` converts back, which is
what keeps the string-shaped boundaries (the cook-side APIs and the exported operator body signature)
from having to move. The remaining raise points across the tree migrate per crate, each paying its own
key rotation (docs/backlog.md).

**A worker's panic needs the scope's own teardown panic caught too.** `std::thread::scope` panics
while it tears the scope down when any of its threads panicked, and it does so **after** the handle was
joined: handling the `join()` result is therefore not enough, because the scope still unwinds a moment
later with `a scoped thread panicked` — the worker's own message is not in that payload. Turning a band
worker's panic into a returned error takes both halves: the join result is kept in preference (that is
where the original message survives), and the whole scoped call is wrapped in
`std::panic::catch_unwind(AssertUnwindSafe(…))` so the teardown panic is consumed rather than unwinding
into the operator dylib boundary, where an uncaught panic aborts the process instead of failing.
`px_field_schema::parallel::rows` is the one place this is done; the criterion for it drives a closure
that panics and requires an `Err` naming that panic.

**A signature change is counted by grepping the changed item, not by walking its callers.** Estimating
what a changed function will touch is a counting problem with a known failure mode: following one call
chain finds the callers you thought of and misses the rest. `px_elem::fill` was changed to return a
`Result` and the estimate was "two files", taken from the one path that reaches `fill` from generated
code; grepping the callee itself — `parallel::rows` — finds five production call sites, of which the
missed four were in `px_volume_alg` (two) and `px_field_op` (two), and the window was reopened for
them. The same family as "read bytes, not views": the instrument that answers the question is the one
aimed at the changed thing, not at the path someone happened to be looking down.

## Repository conventions

**An explicit path does not separate two authors inside one file.** `git add <path>` stages the whole
file, so a path that is yours to edit can still be carrying someone else's uncommitted work in the same
file: staging it commits their text under your message, and their half leaves `git status` before they
ever read it. Listing paths instead of `git add -A` prevents sweeping files you never touched; it does
not prevent sweeping a *shared* file. Read the diff of every path before staging, and when a file
carries two authors' work, either agree on who commits it or commit it once and name the other
author's part in the message.

**Line endings are mixed.** Some files are CRLF on disk and some are LF, and the
bytes on disk are an input to every key. Before a bulk edit, check per file. The
stored side is what keys see: with `core.autocrlf=true` a checkout can be CRLF
while every blob is LF, so scanning bytes on disk reports endings the repository
never stores and may look like corruption in a healthy file. Ask git instead —
`git cat-file -p HEAD:<path>` for the stored form, `git cat-file -p :<path>`
once staged. Any bulk rewrite must preserve each file's line endings exactly.

**`git status` cannot see line-ending changes.** CRLF and LF clean to the same blob. "Git says I am
clean" and "the bytes did not change" are different statements.

**A shared checkout is shared territory down to the index.** Three ad-hoc agents work one tree; the
staging index belongs to whoever staged, and a spilled `git status` shows everyone's half-finished
work side by side. So: add **by path**, never `-a`/`-A`; before any commit, `git diff --cached
--stat` must contain only your own files (if it does not, `git reset` the foreign entries instead
of committing or reverting them); never run git *write* operations against a colleague's uncommitted
work (`checkout`/`restore`/`stash`, and per-path adds are not a shield against one file holding two
authors' edits); and confirm with the other author before touching anything you did not write.
`./format.sh` counts as a source edit: run it **before** any bake/final-build step of a window, not
after, or it rotates every key you just baked.

**Do not run the full test suite out of habit.** Run what the change affects. Do not write
redundant tests. A check that costs real computation belongs in a probe, not in `cargo test`.

**Format before committing.** `./format.sh`. If it touches files beyond your edit, commit those
too.
