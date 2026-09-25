# Findings ledger

Verified defects and stale artefacts discovered while rewriting the documentation. Each entry is
either reproduced or traced to the source that proves it.

⚠ **Line numbers in this file predate the comment strip**, which removed 15,930 lines from `src/**`
and shifted almost every one of them. Treat the file and symbol names as authoritative and the line
numbers as approximate. Re-derive a location with `git grep` before acting on one. Spot check of 6
cited locations against the blobs (`px_cook/src/lib.rs:116-117`, `px_graph_schema/src/ops.rs:5`,
`px_protocol/src/art.rs:816`, `px_render/src/serve.rs:41-42`, `px_volume_gpu_op/src/lib.rs:594-598`,
`px_shader/src/host_stubs.rs:165-167`): none points at the claimed content — five cite comments the
strip removed, and live code drifted too (the `direction_at` panic now sits near `art.rs:632`).

**Status legend:** ✅ fixed · ⬜ open.

## Fixed

| # | Defect | Fix |
|---|---|---|
| 0 | `TextureData::encode` broken for `Rgba8Srgb` | divisor is now the format's element size; a round-trip test for rgba8 was added |
| 0b | `mesh.cubesphere` missing `deny_unknown_fields` | attribute added; all three `surface.toml` recipes checked clean |
| 1 | element operator on a volume field aborts the process | `direction_at_opt` added; `px_elem::fill` probes once instead of asking per cell |
| 2 | `px run nebula` cannot start | `art/nebula/density_volume.toml` now uses `res` |
| 3 | `Shape::default()` is not a whole cube map | `Shape::check` refuses a truncated cube map at both payload loaders |
| 3 | `sky.jitter` was read by no GPU code | wired through `scalars[0]`; `0.0` reproduces the old bytes exactly |
| 3 | `tangent_frame` frame reverses at the polar ring | branch-free rotation; measured 162° → 0.13° worst row-to-row turn |
| 4a | GPU tests skipped and passed with no device | 15 sites now fail loudly through one helper, `px_gpu::require_gpu` |
| 4b | the source-fingerprint gate omitted 4 of 15 crates | roster is now every crate that calls the fingerprint entry point |
| 4c | the operator-loading gate covered 20 of 32 declarations | 31 loaded + 1 counted exclusion, asserted to equal the table |
| 8 | a cache hit re-encoded its artifact and rewrote it (`Graph::store` ran unconditionally) | `store` now skips the write on a hit and reads the on-disk length; doc + `a_hit_leaves_the_artifact_untouched` pin it |
| 9 | the manifest recorded 32 bits of a 64-bit interface hash, and the run line printed the other half | `ManifestEntry.op_version` is `u64` and `interface_tag` prints all 16 hex digits, so the tag is findable in the manifest |
| 10 | `px_protocol/src/rows.rs` was compiled by nothing and fingerprinted by everything | deleted; the collector now walks the module tree, so an undeclared file is not collected and `no_compiled_rust_file_escapes_the_fingerprint` fails on one |
| 11 | `game`'s sources sat in every operator key through one `[dev-dependencies]` edge | the dependency walk skips `[dev-dependencies]`, so a crate the product never compiles is out of identity; `px_fingerprint/tests/dev_dependency.rs` pins it |
| 12 | `collect_tree` skipped paths by name, so a declared module or shader could be compiled and invisible | the collector follows `mod` / `#[path]` / `include!` and takes every `.rs` a declaration reaches plus every `.wgsl` under `src/`; the same gate covers the reverse direction |
| 14 | 13 comment pointers in 12 `build.rs` files named `docs/system/*.md`, retired pages, and carried `§` numbers | rewritten to name the live pages; every crate's `build.rs` is inside its own roster, so this rode the same window |
| 16 | `§` pointers in the art shaders (`.wgsl`) pointed at pages the repository does not carry | deleted without inventing replacements; the file's text is inside the shader contract fingerprint, so this rotated the nine shader keys and the one scene document that inlines the frame shaders, and rode that window's re-cook |
| 7 | `art/frame/default.toml`'s comments narrated removals, cited deleted files, and carried seventeen `§` pointers | comments state the current rules only; four dangling file references name the live ones, the `§` pointers are deleted rather than remapped, and a comment edit here is zero-key and zero-rebake (the parsed values are what the scene key and artefact see) |
| 15 | the instance sidecar was not strict JSON (a trailing comma on every field), so a strict reader silently saw nothing | the comma sits between fields; `px list` parses it strictly again and separates "no sidecar" from "will not parse"; measured zero key cost |
| — | backslash-newline continuations spanning blank lines in `px_shader/src/host_stubs.rs` warned on every build | the two blank lines are deleted; the assembled WGSL is byte-identical (the pinned byte test is green) and the build is warning-free |

⚠ **Two content consequences; both are settled, and the next bake recomputes from source.**

- `tangent_frame` changes the values `east`/`north` produce, so **`field.warp` output changes** in
  the polar band. Measured one-off at the time of the fix: the old frame's worst row-to-row rotation
  on face 256 was 162°, the new one 0.13°, and `field.gradient` unchanged to 1.9e-7 (it is
  frame-covariant, so the basis flip cancels). The band's behaviour is now pinned continuously by
  `px_graphs/tests/warp_polar.rs`: a 32×16 equirect sweep through the loaded operator, both polar
  bands at `|direction[1]| > 0.99` (latitudes above 81.89°), seam included — worst neighbour turn
  must stay under 1° (reading: 0.0010°), displacement must be non-trivial, and the probe includes
  its negative control (a deliberately reversed sample reads 180°).
- `sky.jitter` now does something. Its default is 1.0, so **the nebula sky bake changes** unless the
  parameter is set to 0.0, which reproduces the previous bytes exactly.

## Code defects

### 0. ✅ `TextureData::encode` is broken for `Rgba8Srgb`

`px_graph_schema/src/build.rs`, `impl Build for TextureData` — the shape was written as
`bytes.len()/2` for **both** texture formats, but `Rgba8Srgb` maps to `DType::U8` (element size 1).
`Blob::new` therefore expected `bytes/2` bytes and returned `BadPayloadLength` for every non-empty
rgba8 texture. Only `rgba16_float` (element size 2) satisfied the check, and the only texture test
covered exactly that format, which is why this stayed invisible.

The working form is `bytes.len() / dtype.elem_size()`, as
`px_graph/src/generate/store.rs::write_texture` already did.

**Why it stayed latent:** the only operator whose payload is a `TextureData` is `sky.nebula`, and it
emits `Rgba16Float`. The `Rgba8Srgb` textures are written by `write_texture`, which builds the blob
itself.

### 0b. ✅ `mesh.cubesphere`'s parameter struct was the only one without `deny_unknown_fields`

`px_mesh_schema/src/params.rs`, the `cubesphere` module. Every sibling parameter struct had the
attribute. The consequence was a typo in `art/<graph>/surface.toml` being silently defaulted instead
of reported. All three `surface.toml` files were checked and carry no stray key.

### 8. ✅ A cache hit re-encoded and rewrote its artifact

`px_cook/src/lib.rs` calls `cache.store(Report { hit: true, … }, &payload)` on a hit, and
`px_graph/src/driver.rs::Graph::store` unconditionally re-encoded the payload (`PayloadBundle::to_bytes`)
and `fs::write` it back to the same path it had just read it from. The key is a hash of the payload,
so the rewritten bytes were identical — measured on this workspace, an all-hit `nebula` run (24 nodes,
0 recook, 0.94–0.96 s wall) rewrote **~195 MB** of `.pxart` files whose contents did not change
(probed artifact mtimes all moved; manifest-reported writes 145 MB `nebula` + 50 MB `nebulasky`).

The re-store also had a hidden behaviour: it rewrote the artifact's embedded asset `id` with the
*current* node's name, so "which node cooked this first" was silently mutable. Nothing reads that id
back (`Value/Build::decode` takes the caller's node name; `px_render` reads only `params` and blob
headers from the asset manifest), so the file now stays as first written — first writer wins.

Reachability correction recorded while this was being written up: **`px_graph` *is* inside
`px_graphs` and its fingerprint roster** - `px_fingerprint/src/lib.rs::collect_crate` recurses into
every `path_dependencies` entry, and `px_graph = { path = "../px_graph" }` is a live line in
`px_graphs/Cargo.toml`. The zero-key-rotation property of this fix is therefore *a today-fact*, not
a structural one: `px_graphs` hash is only the identity of `px_local_op!` nodes, and no shipped
graph has one yet (the only use is `px_graphs/tests/local_op.rs`, and tests are excluded from the
roster). The first shipped local operator makes a one-byte edit to `px_graph/src/driver.rs` rotate
that node key - the same warning AGENTS.md already writes for `px_cook` / `px_decls`. The gate
`px_fingerprint/tests/roster.rs::the_graphs_roster_carries_the_driver` pins the reachability.

Fix: `Graph::store` writes only on a miss; on a hit it appends the manifest entry with the byte count
taken from the on-disk file. Measured after the fix on the same workspace: the all-hit `nebula` run
drops from 0.94–0.96 s to **0.47–0.50 s** wall, and a post-run mtime probe shows **0 of 8** artifacts
touched. Pinned by `px_graphs/tests/local_op.rs::a_hit_leaves_the_artifact_untouched`
(artifact mtime and size must not change across a hit).

### 13. ✅ `-Level opt` shares instance keys with `-Level dev`, while `-Level release` does not

The toolchain fingerprint is `blake3(rustc -vV, TARGET, RUSTFLAGS, PROFILE)`
(`px_fingerprint/src/lib.rs:70-91`), and an instance key folds it in (`px_cook/src/inst.rs:128`). Cargo
sets `PROFILE` to only `debug` or `release`, so the axis distinguishes those two and nothing else.

**The root cause is not that the key lacks a level field: `-Level opt` never reaches the instance
libraries at all.** The override is `--config profile.dev.package.<pkg>.opt-level=2`, which applies to
the main workspace, while each instance library is generated into its own nested workspace under
`target/jit/<key>/` (its own `[workspace]`, absolute path dependencies) and built by a nested `cargo`
that is given only `PX_PROFILE` / `PX_TARGET` / `PX_RUSTFLAGS` / `PX_CARGO` (`px_cook/build.rs:19-27`)
— no `--config` anywhere. So `-Level opt` and `-Level dev` produce **the same instance-library
bytes**, not merely the same key.

The real hazard is the other direction: a graph program and the implementation libraries are compiled at
the main workspace's level, while the search for an instance library goes by `PROFILE`
(`profile_dir()`), so **changing level changes the directory that is searched while the key stays the
same** — a library built by another level can be loaded by a binary that never built it.

Measured (same source, `PX_PCG_FRESH=1` so every node re-cooks and rewrites its artifact, one machine,
`planet` and a small `nebula`):
- Node identity — `node` / `op` / interface tag / `bytes` / detail — is identical across levels for
  6/6 `planet` nodes and 20/20 `nebula` rows, and so are the node keys.
- A full SHA256 snapshot of `target/pcg/ab/**` (426 artifacts after the `planet` pair, 446 after the
  `nebula` pair) shows **zero byte changes** across a fresh run at either level.

`PX_PCG_FRESH=1` is a necessary condition for that reading: it is what makes each node take `store`'s
miss branch and rewrite its artifact, so a byte difference would appear as a changed file hash. A
comparison without it degenerates into "the second run hit the cache" and proves nothing. `PX_PCG_FRESH`
also applies to the graph whose `begin()` runs in that process, not to a whole graph tree: in the
`nebula` pair the `nebulasky` row is still `hit=true`, which is the expected shape.

What was the warning half is unchanged: `tools/px.ps1` warns on stderr for `-Level opt` and
`-Level release` (what each does to identity), and `docs/programs.md` states both. What is new is the
gate, and what it does is compare a library's **recorded** build against the current one rather than
reading a level out of the key: `px run` refuses a plan whose present libraries were recorded by another
build, naming the operator and both hashes; `px build` treats such a library as work to redo (it compiles
with the current toolchain, so refusing there would leave no way to switch level); a library with no
sidecar is not a mismatch, since the gate separates two *known* builds. Its subject is narrow — **one key
produced by two different builds** — because a `release` switch rotates the key itself and so appears as
missing libraries rather than as a mismatch at one path; across `opt`↔`dev` the hash and the bytes are
the same and the gate stays quiet. The loader performs the same comparison from the library's toolchain
symbol, so a mismatch unnoticed by the plan is refused when the library is opened. The gate lives in
`px_graphs/src/lib.rs` and is covered by `px_graphs/tests/toolchain_gate.rs`; the change is zero-key,
because `px_graphs` and `px_cook` are in no instance root's closure.

`-Level release` still rotates every instance key, since `PROFILE` changes, and the superseded libraries
stay on disk (`target/pcg/inst/` has been observed holding two generations side by side). That remains
the operational trap it was.

### 15. ✅ The instance sidecar was not strict JSON (a trailing comma on every field)

`px_cook::inst::sidecar_text` (`px_cook/src/inst.rs`) wrote each field with a comma and then closed
the object, so `target/pcg/inst/<key>.json` was accepted by lenient parsers (PowerShell's
`ConvertFrom-Json` reads it) and rejected by strict ones — `serde_json::from_str` reported `trailing
comma at line 11 column 1`. Found while giving the sidecar's `toolchain` field its first reader: the
reader silently returned `None` for all seven libraries, which is the failure shape worth recording,
because "no sidecar" and "unparseable sidecar" looked identical to a caller.

Fix: the comma goes between fields and the closing brace is bare, so the sidecar parses. The reader in
`px_graphs/src/bin/px.rs` went back to a strict `serde_json` parse and now returns five distinct
outcomes instead of an `Option` (`Current`, `Old`, `NotBuilt`, `Unrecorded`, `Invalid`), so a missing
sidecar and an unparseable one are reported as different conditions, each naming the row it is about.
Verified in both directions: deleting one sidecar prints `没有 sidecar` for that row, writing garbage
into it prints the parse error itself, and restoring it returns the listing to clean.

**The writer fix cost zero keys, and that was measured rather than inferred.** The edit was applied
temporarily to the pristine tree, with a forced rebuild on each step (an unforced `cargo test` after a
same-second checkout reads the previous build, which produced two wrong readings before the
measurement was redone):

| tree | `px_graphs` `PX_SOURCE_HASH` | `px list` |
|---|---|---|
| pristine | `5f4446de63c8bbd1…` | 7 instances, all `有` |
| comma between fields | `22610d2d96a2ddbe…` | **byte-identical** |
| restored | `5f4446de63c8bbd1…` | byte-identical |

So the edit does move `px_graphs`' own source fingerprint — `px_cook` reaches that roster through
`[build-dependencies]`, which is why the hash changed — but that fingerprint is only the identity of a
`px_local_op!` node and no shipped graph has one today, so **no instance key moves**.

⚠ Regenerating the sidecars exposed one more thing: **`px build`'s completion test is whether the
library is on disk, not whether the sidecar is.** Deleting sidecars alone rebuilds nothing, because the
plan considers those instances done; the library has to go for `compile_one` to run and rewrite the
sidecar.

### 1. ✅ Any `elem::*` node on a `Domain::Volume` field aborts the process

`px_elem::fill` asked every cell for its direction, and `px_protocol::art::direction_at` panicked for
`Domain::Volume` — which has no direction. In the operator path that panic crosses a dylib boundary,
so it is fatal and uncatchable.

Fix: `direction_at_opt` returns `Option` and is the non-panicking entry; `Field::direction_probe`
wraps it; `fill` probes once before the loop instead of asking per cell. The four element bodies all
ignored the direction argument, so no existing operator's output changed.

- **Repro:** `cargo build -p px_graphs --bin nebula`, then
  `target/debug/nebula.exe --face 8 --shape 8 --layers 8`.
- **Symptom:** stdout stops after `warped` is cached; stderr shows
  `panicked at px_protocol/src/art.rs:816` with
  `体网格（Domain::Volume）没有「一个方向」这回事`, followed by
  `fatal runtime error: Rust cannot catch foreign exceptions, aborting`.
- **Cause:** `px_elem::fill` (`px_elem/src/lib.rs:183`) unconditionally calls
  `out.direction(x, y)`. For `Projection::Volume` that reaches the deliberate panic in
  `px_protocol::art::direction_at` (`px_protocol/src/art.rs:816`). The panic crosses the dylib
  boundary in the operator path, which is fatal rather than catchable.
- **Scope:** the next node after `warped` in the nebula graph is `elem::Remap` on a Volume field.
  Any element operator applied to a volume field hits this.
- **Why it is invisible:** the graph is currently also broken earlier (see 2), so it only shows up
  when the user passes `--layers`.

### 2. ✅ `art/nebula/density_volume.toml` used a removed field

- `res_ratio = 1.0` while `DensityParams` declares `res` and is `deny_unknown_fields`.
- `nebula.rs::volume_layers()` parses that file directly, so `px run nebula` failed at startup:
  `unknown field res_ratio, expected one of res, layers, inner, outer, reach`.
- It only appeared to work when `--layers` was passed.
- Fixed: the file now sets `res = 64` (an absolute count). The complete graph now runs:
  19 + 1 nodes, 20.7 s, exit 0.

### 3. ✅ The comment contradictions below no longer exist as comments

Everything from here to the end of the comment section described live `src/**` comments that stated
the opposite of the code beneath them. Those comments have since been **deleted wholesale** — every
crate carries a single `//! See docs/<page>.md` line and nothing else — so the contradictions are
gone.

The section is kept for one reason: several of the entries record a **defect rather than a stale
sentence**, and those defects are still open. The ones that mattered:

- numeric constants that disagreed with the code (`SHELL_WALL_FADE` documented as 0.10, actually
  0.14; the `fbm3` panic message teaching `res² × layers × 6` where the true height is
  `res × layers × 6`, a form the consumer silently accepts, so obeying the message multiplies the
  volume layer count by `res`),
- a live parameter no code reads (`SkyParams::jitter`),
- `bevy_stub` claimed where `wgpu_host_stub` is passed, in three places, one of which justified a
  false conclusion about the offline gate,
- `cube_face_of` warned against as "a different face order" when it is the same table, pinned by a
  test as the exact inverse of `cube_direction`.

Read the rest for those findings, not as a description of the tree.

### 17. ✅ A directory that cannot be read is silently treated as empty while collecting key inputs

`px_cook/src/inst_scan.rs` walks a crate to collect the files that go into an instance key. Both walkers
swallowed a failed `read_dir` and returned as if the directory were empty:

```rust
// collect_rs, before          // walk_crate, before
let Ok(entries) = std::fs::read_dir(dir) else { return; };
let entries = match std::fs::read_dir(dir) { Ok(entries) => entries, Err(_) => return Ok(()) };
```

An unreadable directory (permissions, a lock, a path that vanished between the walk and the read) was
therefore indistinguishable from a directory with no sources: the key was computed over the files that
*were* readable and the run reported success. Since the key's whole job is to identify the source bytes
a node was built from, a partial read is a false identity, not a missing one — and it is silent, so
nothing downstream could notice.

Fixed: both walkers return `Result<(), Fault>` and an unreadable directory is `Fault::read`, naming the
path. The reachable tier is the walker itself, so the criterion is
`px_cook/tests/inst_scan.rs::a_directory_that_cannot_be_read_is_a_failure_that_names_it` — it hands
`collect_rs` a file where a directory is expected (the same refusal a permission produces) and requires
a `read` failure naming it, with nothing collected. A failure here cannot be reached from a command line
without first making a directory unreadable, which is why the criterion sits at the walker.

## Superseded comment findings


### 3. `px_cook/src/lib.rs:116-117` still describes a "canvas"

> `⚠ 画布从 `cache` 取，**不是参数**`

Two dozen lines below, `px_cook/src/lib.rs:139-141` says the opposite: shape and projection are
parameters of the operator, not a canvas supplied by the driver. The canvas was removed.
`GraphSpec` is `{ name: String }` (`px_graph_schema/src/protocol.rs:15-17`) — there is no canvas
field to hold it.

### 3b. Other comment/code contradictions (each verified against the source)

| Where | Says | Actually |
|---|---|---|
| `px_cook/src/lib.rs:48` | the catalogue is `inst_out/insts_gen_catalogue.rs` | the generator writes `OUT_DIR/insts_gen_catalogue.rs`; `inst_out/` occurs nowhere else |
| `px_cook/src/inst.rs:579-580` | `InstCodegen::body` is the recipe's *original* body | it stores the ARG-substituted body (`px_graphs/build.rs:177`), as `inst.rs:806-808` itself says |
| `px_graph_schema/src/ops.rs:5` | "one `GetProcAddress` at runtime" | `load_at` re-resolves the symbol on every call; only the `Library` is cached (`ops.rs:43-48`, `ops.rs:64-72`) |
| `px_field_op/src/lib.rs:1` | "six preset operators" | `px_field_op/src/ops/mod.rs:5-13` declares nine |
| `px_graphs/src/bin/nebula.rs:35-36` | `--face` sets the volume canvas and the sky face | `face` is only printed (`:187`); `--shape` sets the resolution and `art/nebulasky/sky.toml` sets the sky. **Dead knob** |
| `px_graphs/src/bin/nebula.rs:182` | `--layers` overrides the TOML value via `params_override` | no such function; it only changes the layer count and forces a resample |
| `px_graphs/src/bin/scene.rs` module doc | states a byte-equality verdict on six frozen artifacts | that verdict no longer exists (the artifacts were scene format v2 and were deleted) |
| `px_graphs/src/inst_recipe.rs:22`, `insts.rs:20`, `bin/field_remap.rs:8`, `px_graphs/build.rs:13` | "seven graph exes" | there are 9 graph-program bins (13 bins total) |
| `tools/px.ps1:10,30,33` | "bevy stays -O0", "nine crates" | bevy is not in the workspace; there are 29 members, 26 of them default |
| `px_graph/tests/source_hash.rs:25-39` | "3 implementation libraries + 3 schema crates" | 5 and 4 |
| `px_graphs/tests/ops_load.rs:21` | "every declared operator" | the body lists 20 of 32 declarations |
| `Elementwise<F>` | referenced in `px_elem/src/lib.rs:8,17-24`, `px_cook/src/inst.rs:494,512,821`, `px_graphs/src/insts.rs:22`, `px_graphs/tests/elem.rs`, `px_graphs/Cargo.toml:29-30` | does not exist. The `PxOp` impl is `ElemOp<F>` in `px_graphs/src/elem.rs`; the script-facing value is the generated unit struct `elem::Constant` |
| `px_field_schema/src/ops.rs:33,92,95,97` | reference the `px_inst!` macro | that macro no longer exists |
| `px_cook/src/inst.rs`, `px_fingerprint/src/lib.rs:94,114,172`, `px_graph_schema/src/contract.rs:289`, `ops.rs:110,124`, `px_volume_alg/src/lib.rs:16` | reference `px_jit` | that binary no longer exists. ⚠ These files participate in keys, so editing them costs a repository-wide key rotation |

### 3d. Comments that contradict the code beside them, found while verifying the renderer

Each of these is a live comment stating something the code does not do:

| Where | Says | Actually |
|---|---|---|
| `px_gpu/src/lib.rs:13-14` | backend defaults to DX12 | `:47-53` defaults to **Vulkan** |
| `px_gpu/src/lib.rs:279-281` | a TODO saying wgpu 29 has no `pop_error_scope` here | `:282-283`, `:303` push and pop a validation error scope |
| `px_protocol/src/scene.rs:868` | an empty pass list means "main pass only" | `px_pass/src/lib.rs:2870-2872` returns early and **draws nothing**; the host synthesises no main pass |
| `px_render/src/material.rs:441-446` | groups 0 and 3 with 1 and 2 left empty | `:462` sets `groups[1]` |
| `px_probe/src/common.rs:59,103`, `probe.rs:403`, `gradient.rs:716` | the probe device requests `downlevel_defaults()` limits | the probe creates no device; it uses `px_gpu`'s, which requests `adapter.limits()` (`px_gpu/src/lib.rs:74`) |
| `px_volume_gpu_op/src/lib.rs:594-598` | the compute code is split across `sampler_fn.wgsl` / `sample_points.wgsl` / `march.wgsl` | one `src/sampler.wgsl` with eight compute entries, plus an inline `REPACK_WGSL`; the binding-number collision is solved with explicit slots |
| `px_volume_gpu_op/src/lib.rs:67-69` | the WGSL "is only a source shipped with the crate" | the same constant is the shader module of every dispatch |
| `px_render/src/serve.rs:41-42` | the lease check runs "every 120 frames, exits within 4 s" | a flat 2 s interval (`:43`) |

### 3e. `px_render` and friends still narrate the deleted Bevy host

`px_render/src/main.rs:1-8`, `art.rs:297-303`, `digest.rs:7-10`, `serve.rs:1-8`, `client.rs:6-8`,
`report.rs:8-15` and many others cite `§` numbers, dates, and "本轮/从前", and describe a renderer
that no longer exists. They are not a usable source of present-tense fact — which is why the
rewrite reads code rather than comments.

### 3f. Three comments claim the cook side assembles content shaders with `bevy_stub`

`px_graph/src/driver.rs:581-586` says the cook-side stub table "must be Bevy's (`bevy_stub`)", but
`:586` passes `px_shader::host_stubs::wgpu_host_stub`. `px-scene/src/frame.rs:1154` repeats the
claim and cites that comment as its authority. `px_shader/src/assemble.rs:30` says the same.

Related, and more consequential: `px_shader/src/assemble.rs:8-12` justifies the assembler being
"looser than runtime" by arguing that Bevy's `naga_oil` inlines only the symbols named in an
`#import`, so passing the offline gate proves nothing about runtime. **There is no Bevy and no
`naga_oil` anywhere** (`Cargo.lock` has zero bevy entries and one `naga 29`), and the runtime host
calls this same function — the two sides coincide. The comment describes a constraint that no
longer exists and would mislead anyone deciding how much the offline gate is worth.

### 3g. `modules_fingerprint`'s doc points at a module that does not exist

`px_shader/src/lib.rs:284-288` cites `px_render::reflect`. The function has no caller outside its own
test.

### 3h. `px_render/src/diff.rs` and `digest.rs` still frame their readings around retired images

The module docs describe "oracle" and "slice 1" images that are not in the repository. `--diff`
today compares two operator-supplied PNG files.

### 3i. `host_stubs.rs`'s doc-comment table reads like a current reading but is history

`px_shader/src/host_stubs.rs:50-55` lists byte counts and a fingerprint (8311 / 52769 / 1096 / 25915,
`33881b68…`) that do not match what the test pins today (9528 / 71282 / 32321 / 45290,
`7c31cc92…`). It is labelled as a rename comparison, so it is not false — but it sits exactly where
a current reading is expected.

### 3j. `-Sweep` is documented as dead but works

`tools/frame-probe.ps1:33` says `-Sweep` "only serves the two retired paths, and is dead along with
them". It sets the five-scene list and works with `-Phase shot`; `-Phase shot` without `-Scenes`
also defaults to those five scenes.

### 3k. `px_probe/src/common.rs:55` cites a path that does not exist

It points at `art/docs/render/schema-limits.md`.

## Comments that contradict the code

### 3l. `cube_face_of` is documented as "a different face order", but it is now the same table

`px_volume_gpu_op/src/occupancy.rs:126-128,163-165` and `src/sampler.wgsl:172-175` warn that
`px_protocol::art::cube_face_of` uses "another face order and axis convention" and must not be used
to index the mask. It is now the **identical** table as `grid_coords_of`
(`px_protocol/src/art.rs:650-679` vs `occupancy.rs:129-155` vs `sampler.wgsl:66-98`), and
`px_protocol/tests/cube.rs:26-41` pins it as the exact inverse of `cube_direction`. The warning
describes a divergence that has been fixed and would make a future reader avoid a correct function.

### 3m. `sky.jitter` is a live parameter no GPU code reads

`px_volume_gpu_op/src/lib.rs:452`, `sampler.wgsl:559` and
`px_volume_schema/src/params.rs:273-279`. Every live dispatch writes
`scalars: [0.0, 0.0, 0.0, enter]` (`lib.rs:995,1663,2095`) and the shader never reads
`sky.scalars.x/y/z`. The bake is unjittered; only the CPU draft uses the value. It is set to 1.0 in
`art/nebulasky/sky.toml`, so the parameter reads as active.

### 3n. Numeric and structural claims that are simply wrong

| Where | Says | Actually |
|---|---|---|
| `px_volume_alg/src/density.rs:28-32` | `SHELL_WALL_FADE` is 0.10 | the constant is **0.14** |
| `px_volume_alg/src/emission.rs:3-10,94` | the output has **four** channels | six lanes: `[emit R, emit G, emit B, σ_R, σ_G, σ_B]` (`:246-262`) |
| `px_volume_alg/src/emission.rs:419-422` | test helper `mean_alpha` measures mean σ_R | it reads `chunks(4)` over six-lane data |
| `px_volume_op/src/density.rs:7-9` | the volume resolution comes from the canvas, and `g` is the only canvas source | `shape_of()` returns `(res, layers)` from the parameters; there is no canvas and the body takes no `g` |
| `px_volume_op/src/stars.rs:12-13`, `px_graphs/src/bin/nebula.rs:30` | cite `RESOLUTION_IS_CANVAS = false` | that property no longer exists; the rule is now structural (`px_cook/src/lib.rs:289-295`) |
| `px_volume_schema/src/params.rs:291-295` | `star_core` is radians, then says it is a world length | it is a world length (`raymarch.rs:77-85`, `sampler.wgsl:506-511`); `art/nebulasky/sky.toml` repeats the radians wording |
| `px_volume_gpu_op/src/lib.rs:2945-2946` | the single-lane guard in `VolumeData::at` is a `debug_assert` | it is a hard `assert_eq!` (`px_protocol/src/art.rs:502`) |
| `px_graphs/tests/cloud_proxy.rs:61` | calls `offset` "P20 dead code to be deleted" | it is implemented and live (`px_mesh_op/src/proxy.rs:288-326`) |
| `px_mesh_op/src/proxy.rs:69-70` | `radial_gradient` is "the field gradient at a world point" | it differences the face's parameter triple; the file's own `world_gradient` doc (`:91-95`) says exactly that this must not be used as a world gradient |
| `px_volume_alg/src/stars.rs:263`, `px_volume_schema/src/params.rs:365-367` | the star field does not take the density volume | it does (`StarsInput`, `gas_biased`, `params.rs:418-435`) |

### 3o. Rejected-design post-mortems left inside doc comments

`px_volume_alg/src/raymarch.rs:282-298` hangs a "we tried cross-face blending once and reverted;
here is where it failed for the next person" note off `sample_volume`'s doc. It documents code that
is not present. Same class: `px_volume_alg/src/density.rs:18` and
`px_field_schema/src/volume.rs:18` say "changed in round 29", `raymarch.rs:340` says "fixed in round
30".

### 3p. Test fixtures declare `lanes: 1` over six-lane data

`px_volume_alg/src/raymarch.rs:817` and `px_volume_gpu_op/src/lib.rs:379,1163,1461,1885,2295,2491,2559,2671,2783`.
Only a hard-coded stride of 6 in the samplers hides it. `Occupancy::from_flat` does use `lanes`, so
the same fixture routed through `raymarch_sky` would build a wrong mask.

Related: `px_volume_gpu_op/src/lib.rs:2016-2064` forwards `emission.lanes()` to the mask but a
constant 6 to the shader, with no assert. The six-lane contract is enforced only by graph wiring.

### 3q. `px_graph/src/driver.rs:581-586` and friends claim the cook side assembles content shaders with `bevy_stub`

`:586` passes `px_shader::host_stubs::wgpu_host_stub`. `px-scene/src/frame.rs:1154` repeats the
claim citing that comment as authority, and `px_shader/src/assemble.rs:30` says the same.

More consequential: `px_shader/src/assemble.rs:8-12` justifies the assembler being "looser than
runtime" by arguing that Bevy's `naga_oil` inlines only the symbols an `#import` names, so passing
the offline gate proves nothing about runtime. **There is no Bevy and no `naga_oil` anywhere**
(`Cargo.lock` has zero bevy entries, one `naga 29`), and the runtime host calls this same function.

### 3r. Remaining stale comments found while verifying the volume and mesh domains

`px_shader/src/lib.rs:284-288` cites `px_render::reflect`, which does not exist.
`px_shader/src/host_stubs.rs:50-55` presents byte counts and a fingerprint as a current reading
when they are a historical rename comparison (the test pins different numbers).
`px_render/src/diff.rs` (module doc) and `digest.rs:9-10` frame their readings around retired
"oracle"/"slice 1" images. `px_probe/src/common.rs:55` cites `art/docs/render/schema-limits.md`,
which does not exist. `tools/frame-probe.ps1:33` says `-Sweep` is dead, but it works with
`-Phase shot`. `tools/px.ps1:31-33` still says "nine crates". `px_probe/src/lib.rs:22` still says
"does not recompile bevy". `art/frame/vertex_mesh.wgsl:4` cites `px_render_wgpu/src/material.rs`.

## Weakened gates

### ✅ The backslash-newline continuations no longer span blank lines

`px_shader/src/host_stubs.rs` had two continuations that each skipped a blank line, so rustc
reported "multiple lines skipped by escaped newline" on every build. The two blank lines are
deleted; the assembled WGSL is byte-identical (the pinned byte test is green), the build is
warning-free, and the key surface was measured before landing: instance keys identical across
`px list`, six crate rosters rotate (`px_shader`, `px_graph`, `px_graphs`, `px-scene`,
`px_render`, `px_probe`), so `shaders` and `scene` were re-baked in the same window.

### 4. `px_graphs/src/bin/scene.rs` states a byte-equality claim with no live reader

The module doc asserts that the six frozen artifacts are what the "escape hatch" verdict measures.
That verdict no longer exists (the frozen artifacts were scene format v2 and were deleted). The
module doc needs to describe the current contract instead of pointing at a retired one.

### 5. `px_protocol/src/sparse.rs:104` — `bit_get` is dead code

Compiled out but never called, producing a warning on every build.

## Stale inputs

### 6. ✅ 34 orphan scene recipes in `art/scene/` — deleted

The 34 recipes referenced by no `.rs` or `.ps1` in the tree are gone from the working tree; the
list of names and their contents are in git history (the deleting commit names all 34). Scene
recipes sit outside every fingerprint roster, so the deletion rotates no key. Ten files remain and
each stem does occur in `.rs`: the eight content scenes tabulated in `docs/art.md`, plus
`orbit-bare-nolight.toml` and `soft-e300.toml`, whose stems appear only in `#[cfg(test)]` code (an
oracle scene-list file name and a test fixture path) — leftovers by the spirit of the criterion but
not by its letter, so they were kept. Re-derive at any time by listing `art/scene/*.toml` and
grepping each stem across the `.rs` and `.ps1` files.

### 7. ✅ `art/frame/default.toml` contained history in its comments

Resolved. The comments state the current rules only: per-level shadow atlases and why they cannot
share one texture, the 1×1 dummy binding that lets the page pass write and sample the same atlas, the
downsample chain and its ordering, the clear colour's linear values, and that the skybox material's
entry is `fragment`. What was removed is the narration wrapped around them (the
`point_shadow_atlas_sample` resource, the `copy_shadow_atlas` pass, the `point_shadow_textures`
atlas, the `fs_main` entry mistake, the 400 MB/24 MB bandwidth figures) and every citation that
pointed at nothing: four file or path references now name the live ones, and the seventeen `§`
pointers are gone — the repository carries no numbered pages for them to resolve to, and
`docs/README.md` gives references as a file path plus a symbol name.

A comment edit to this file is zero-key and zero-rebake. What the scene key and the scene artefact
see is the file's parsed values: with one comment character changed, the scene content key stays
`f601002f018a` and the artefact under that key is byte-identical. Editing what the file says about
the frame re-cooks; editing how it is explained does not.

The same class was swept across the recipes beside it: every `§` pointer in `art/**/*.toml` is gone
(twelve files, nineteen occurrences — eight pass recipes and four content scenes). Both halves were
measured zero-key by the same probe: one changed comment character in `art/scene/orbit.toml` and in
`art/passes/invert.toml` leaves the five scene content keys unchanged, and neither loader hashes the
recipe text (`px_pass` hashes shader *names*, and the recipes are parsed into values).

### 16. ✅ `§` pointers in the art shaders (`.wgsl`)

Cleared: forty-two numbered pointers and four `§本轮` across twelve shader files are gone, deleted
rather than remapped (`docs/README.md` gives references as a file path plus a symbol name, and the
repository carries no page for any `§` number). The rules they were attached to are untouched; one
history line became a rule (`art/shaders/lib/common.wgsl`: the deleted `const SUN_DIRECTION`).

The key face is why this needed a window, and it splits in two: a changed comment character in a
shader under `art/shaders/` rotates that shader's contract key (and, through the `#import` closure,
every shader that reaches the changed module — `gasgiant` and `ring` rotate although their own files
were not edited), while a changed character in `art/frame/*.wgsl` rotates the scene document that
inlines the frame shaders. Instance keys never move. Re-cooking `shaders` turned all nine shader keys
and re-baking `scene` turned the one scene that follows them.

## Documentation defects found in the old set

Recorded as subagents report them.
