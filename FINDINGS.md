# Findings ledger

Verified defects and stale artefacts discovered while rewriting the documentation. Each entry is
either reproduced or traced to the source that proves it.

⚠ **Line numbers in this file predate the comment strip**, which removed 15,930 lines from `src/**`
and shifted almost every one of them. Treat the file and symbol names as authoritative and the line
numbers as approximate. Re-derive a location with `git grep` before acting on one.

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
| 15 | the instance sidecar was not strict JSON (a trailing comma on every field), so a strict reader silently saw nothing | the comma sits between fields; `px list` parses it strictly again and separates "no sidecar" from "will not parse"; measured zero key cost |

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

### 13. ⬜ `-Level opt` shares instance keys with `-Level dev`, while `-Level release` does not

The toolchain fingerprint is `blake3(rustc -vV, TARGET, RUSTFLAGS, PROFILE)`
(`px_fingerprint/src/lib.rs:70-91`), and an instance key folds it in (`px_cook/src/inst.rs:128`). Cargo
sets `PROFILE` to only `debug` or `release`, so the axis distinguishes those two and nothing else.

That collides with the documented measurement practice. `px.ps1 -Level opt` appends
`--config profile.dev.package.<pkg>.opt-level=2` (programs.md) — still `PROFILE=debug`. So **an arbiter
run at `-Level opt` produces byte-identical keys to a `-Level dev` run of the same source, and whichever
built first leaves its libraries on disk for both.** The same driver also warns that `-Level` overrides
are deliberately withheld from `list`/`build`/`gc` because instance libraries form their own workspace
root under `target/jit/<key>/` that the main config cannot reach — so the very runs that disagree about
optimisation level agree about the key, by construction.

Two things follow, and they point in opposite directions from what operators.md currently claims:

- [operators.md](docs/operators.md) justifies excluding `DEBUG`/`opt-level` because they "do not move
  layout". For the
  *numerical* payload that holds — which is why the CAS can be shared across levels at all. But float
  results are exactly where this bites: `opt-level=2` may reassociate FP ops through
  `RUSTFLAGS`-invisible codegen options, so an "optimized" reading and a "dev" reading of the same key
  can differ in content while sharing identity. The doc's reason is sound for layout and silent about
  numerics; the gap should be stated rather than inferred.
- `-Level release` **does** rotate every instance key, since `PROFILE` changes. This is the operational
  trap deepseek measured: switching build environment silently rotates all instance keys with zero
  source edits, and the superseded libraries stay on disk (`target/pcg/inst/` was observed holding two
  generations side by side). Nothing in the key says which level produced a library, and no gate checks
  it — `sidecar_text` records a `toolchain` field and `px list` now reads it back, so a library built by
  another level is marked `!` (see below); the key itself still cannot say.

The warning half is now in place at zero key cost: `tools/px.ps1` warns on stderr for `-Level opt` and
`-Level release` (what each does to identity), `-Task list` marks a present library whose sidecar
`toolchain` differs from the current build, and `docs/programs.md` states both. What remains open is the
identity question: `opt` and `dev` still share keys, so a library built by either answers for both.

Not fixed here: any change either way touches `px_fingerprint/src/lib.rs` or `px_cook/src/inst.rs`, both
inside every roster ⇒ full-family rotation, so it belongs in the scheduled batch (§11) or needs a
decision. Cheap part available now at zero key cost: a `tests/` gate asserting one canonical build
configuration per measurement session.

The convention that costs nothing today and prevents the misreading: **within a batch window or a timing
session, fix `--release` vs debug and never mix**; treat `-Level opt` as numerically distinct evidence
even though it shares keys.

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

### A build warning everyone sees

`px_shader/src/host_stubs.rs:165-167` and `:370-372` use a backslash-newline continuation that spans
a blank line, so rustc reports "multiple lines skipped by escaped newline" on every build. The
pinned byte test covers the consequence, so it is harmless — but it trains people to ignore warnings.

### 4. `px_graphs/src/bin/scene.rs` states a byte-equality claim with no live reader

The module doc asserts that the six frozen artifacts are what the "escape hatch" verdict measures.
That verdict no longer exists (the frozen artifacts were scene format v2 and were deleted). The
module doc needs to describe the current contract instead of pointing at a retired one.

### 5. `px_protocol/src/sparse.rs:104` — `bit_get` is dead code

Compiled out but never called, producing a warning on every build.

## Stale inputs

### 6. 34 of the 44 scene recipes in `art/scene/` have no consumer

Not referenced by any `.rs` or `.ps1` in the tree:

```
orbit-allmiss            orbit-soft-wind          probe-noshadow-5x
orbit-bare-shadow        orbit-uranus             probe-ringsun1
orbit-bound              probe-farsun20           probe-ringsun1-ns
orbit-nograd             probe-farsun5            probe-ringsun20
orbit-proxy-fine         probe-hi1                probe-ringsun20-ns
orbit-proxy-fine-bound   probe-hi1-ns             probe-ringsun5
orbit-rings              probe-hi5                probe-ringsun5-ns
orbit-soft-nocloudshadow probe-hi5-ns             probe-vs-center
orbit-soft-noshadow      probe-hires              soft-e24000
orbit-soft-plain         probe-hires-ns           soft-e6000
orbit-soft-proxy         probe-noshadow-1x
orbit-soft-shell         probe-noshadow-20x
```

These are single-purpose comparison recipes from earlier measurements. They are inputs, so deleting
them changes no key, but a measurement someone wants to re-run may depend on one, so this needs a
decision rather than a silent sweep. The count moves when a recipe gains no consumer: re-derive it by
listing `art/scene/*.toml` and grepping each stem across the `.rs` and `.ps1` files.

### 7. `art/frame/default.toml` contains history in its comments

The file is a product input (frame materials are inlined into artifacts), so editing it costs a
re-cook. Its comments currently narrate removals and reference deleted resources
(`copy_shadow_atlas`, `point_shadow_atlas_sample`, `px_render_wgpu`). The rules they encode are
worth keeping; the narration is not. Deferred because the edit has a key cost.

## Documentation defects found in the old set

Recorded as subagents report them.
