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

## Repository conventions

**Line endings are mixed.** Some files are CRLF on disk and some are LF, and the bytes on disk are
an input to every key. Before a bulk edit, check per file. Any bulk rewrite must preserve each
file's line endings exactly.

**`git status` cannot see line-ending changes.** CRLF and LF clean to the same blob. "Git says I am
clean" and "the bytes did not change" are different statements.

**Do not run the full test suite out of habit.** Run what the change affects. Do not write
redundant tests. A check that costs real computation belongs in a probe, not in `cargo test`.

**Format before committing.** `./format.sh`. If it touches files beyond your edit, commit those
too.
