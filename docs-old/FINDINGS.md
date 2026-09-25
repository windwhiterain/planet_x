# Findings ledger — documentation rewrite

Verified defects and stale artefacts discovered while rewriting `docs/`. Each entry is either
reproduced or traced to the source line that proves it. Nothing here has been fixed yet.

## Code defects

### 0. `TextureData::encode` is broken for `Rgba8Srgb`

`px_graph_schema/src/build.rs:82` writes `shape = [bytes.len()/2]` for **both** texture formats, but
`Rgba8Srgb` maps to `DType::U8` (element size 1) at `:77`. `Blob::new` therefore computes a byte
length of `bytes/2` and returns `BadPayloadLength` for every non-empty rgba8 texture. Only
`rgba16_float` (element size 2) satisfies the check, and the test at `:236-259` covers only that
format.

Correct form: `bytes.len() / dtype.elem_size()`, as `px_graph/src/generate/store.rs:167` does.

**Latent today**, which is why nobody has hit it: the only operator emitting `TextureData` is
`sky.nebula`, and it emits `Rgba16Float` (`px_volume_gpu_op/src/lib.rs:2195-2202`). The `Rgba8Srgb`
textures go through `write_texture`, which builds the blob itself.

### 0b. `mesh.cubesphere`'s parameter struct is the only one without `deny_unknown_fields`

`px_mesh_schema/src/params.rs:8`. Every sibling parameter struct has it (compare `:34`). The
consequence is a typo in `art/<graph>/surface.toml` being silently defaulted instead of reported.

### 1. Any `elem::*` node on a `Domain::Volume` field aborts the process
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

### 2. `art/nebula/density_volume.toml` uses a removed field

- `res_ratio = 1.0` while `DensityParams` (`px_volume_schema/src/params.rs:498`) declares `res`
  and is `deny_unknown_fields`.
- `nebula.rs::volume_layers()` parses that file directly, so `px run nebula` fails at startup:
  `unknown field res_ratio, expected one of res, layers, inner, outer, reach`.
- It only appears to work when `--layers` is passed.

## Stale comments that contradict the code

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

### Skip-as-pass in the default chain

`px_gpu/src/lib.rs:346-351`, `px_volume_gpu_op/src/lib.rs` (about eight sites) and
`px_nurbs_gpu_op/src/tests.rs` (seven sites) print `没有可用 GPU，跳过` and **pass**. Both operator
crates are **default members**, so on a machine without a GPU the fast chain is green while every
GPU comparison silently never ran. This is exactly the failure the invariants warn about.

The policy is also inconsistent inside one crate: `px_volume_gpu_op/src/lib.rs:1314-1319`
deliberately panics instead of skipping.

### A build warning everyone sees

`px_shader/src/host_stubs.rs:165-167` and `:370-372` use a backslash-newline continuation that spans
a blank line, so rustc reports "multiple lines skipped by escaped newline" on every build. The
pinned byte test covers the consequence, so it is harmless — but it trains people to ignore warnings.

### Two test gates are weaker than their comments claim

- `px_graph/tests/source_hash.rs`: `FINGERPRINTED_CRATES` (`:30-39`) and
  `every_implementation_library_exports_an_identity` (`:106-116`) omit the three NURBS crates, so
  their `build.rs` and `px_impl_lib!()` are ungated.
- `px_graphs/tests/ops_load.rs`: claims to load every declared operator but covers 20 of 32.
  Missing: Craters, Stamps, Fbm3, Ridged3, Warp3, Density, Emission, SkyNebula, Stars,
  SurfaceTessellateGpu, CurveTessellateGpu. (`FieldRemap` is excluded by design — it is declared as
  `field.remap/inst` and has no preset body.)

### 4. `px_graphs/src/bin/scene.rs` states a byte-equality claim with no live reader

The module doc asserts that the six frozen artifacts are what the "escape hatch" verdict measures.
That verdict no longer exists (the frozen artifacts were scene format v2 and were deleted). The
module doc needs to describe the current contract instead of pointing at a retired one.

### 5. `px_protocol/src/sparse.rs:104` — `bit_get` is dead code

Compiled out but never called, producing a warning on every build.

## Stale inputs

### 6. 25 of the 44 scene recipes in `art/scene/` have no consumer

Not referenced by any `.rs` or `.ps1` in the tree:

```
orbit-allmiss           orbit-bound             orbit-nograd
orbit-soft-nocloudshadow  orbit-soft-noshadow   orbit-soft-plain
orbit-soft-proxy        probe-farsun20          probe-farsun5
probe-hi1               probe-hi1-ns            probe-hi5
probe-hi5-ns            probe-hires             probe-hires-ns
probe-noshadow-1x       probe-noshadow-20x      probe-noshadow-5x
probe-ringsun1-ns       probe-ringsun20         probe-ringsun20-ns
probe-ringsun5          probe-ringsun5-ns       soft-e24000
soft-e6000
```

These are one-off comparison recipes from earlier tuning. They are inputs, so deleting them changes
no key; but a measurement someone wants to re-run may depend on one, so this needs a decision rather
than a silent sweep.

### 7. `art/frame/default.toml` contains history in its comments

The file is a product input (frame materials are inlined into artifacts), so editing it costs a
re-cook. Its comments currently narrate removals and reference deleted resources
(`copy_shadow_atlas`, `point_shadow_atlas_sample`, `px_render_wgpu`). The rules they encode are
worth keeping; the narration is not. Deferred because the edit has a key cost.

## Documentation defects found in the old set

Recorded as subagents report them.
