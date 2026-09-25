# Verdicts: probes, the arbiter, images, and the render harness

How correctness is established here: which instrument answers which question, what its pass condition
is, and how to run it. A verdict is a number produced by a stated instrument and compared against a
stated condition. If an instrument cannot take a reading, it must say so and fail — a skipped check is
not a pass.

## The instruments

| Instrument | Where | What its verdict is |
|---|---|---|
| `cargo test` | default members | unit and integration gates; non-zero exit on failure |
| `cargo test --workspace` | all 29 members | the same, plus the host, the probes, and the simulation |
| probes | `px_probe` bins | **the process exit code** |
| the arbiter | `px_probe --bin field_dual` | the gradient verdict, and the only one |
| image digest | `px_render::digest` | names an image; not a judgement |
| image diff | `px_render --diff LEFT RIGHT` | reports numbers; judges nothing |
| reference images | `art/reference/` | visual targets; no verdict at all |
| offline host gate | `px_render --shaders` | assembles and validates the four content shaders; GPU-free |
| device gate | `px_render --device [--shot PNG]` | the device comes up, and a solid clear survives to PNG |
| offline readings | `px_render --offline --stats`, `--offline --time N` | per-channel min/max and colour count; wall clock |
| timing | `px_render --offline --spans 预热,测量,轮数` | per-pass GPU encoder-level timestamps |
| shot availability | the resident service's `has_cloud` / `verdict` | a real pass condition in `-Phase shot` |

`px_render`, `px_probe`, and `game` are **not** default members, so `cargo test` is the fast chain and
`cargo test --workspace` is everything. Run what the change reaches, not the full suite out of habit.

## The probes

Probes are binaries, not tests: the exit code is the verdict, and the GPU-backed ones need a device
and minutes, which is why they are not on the `cargo test` chain. They are built on a common floor
(`px_probe/src/common.rs`):

- one `wgpu` device per process (`px_gpu::connect`), backend from `px_gpu::backends()`: **Vulkan by
  default**, overridable with `WGPU_BACKEND=dx12|gl|gles`;
- `require_gpu()` exits **2** when no adapter is available — a probe never passes silently on a
  machine without a GPU;
- `run_checks(title, checks)` runs each check under `catch_unwind`, prints `✓`/`✗` per check, prints a
  total, and exits **1** if any check failed.

Run one with either:

```powershell
cargo run -p px_probe --bin <name>                 # or: --release
.\tools\px.ps1 -Task <name> [-Level dev|opt|release]
```

`-Level opt` adds `--config profile.dev.package.<pkg>.opt-level=2` for the local crates (including
`px_probe`) and rebuilds nothing else; `-Level release` is a full `--release` build.

| Bin | Measures | Pass condition | Device |
|---|---|---|---|
| `device` | that a headless device comes up | `max_compute_workgroup_size_x >= 64` | yes |
| `field_dual` | **the arbiter** (below) | four checks, see below | yes |
| `gradient` | that the probe harness itself runs the production shader, plus gate conventions and per-channel attribution of the residual | eight checks; the meta-check is that the harness reproduces the production field | yes |
| `dual` | the dual-number machinery itself | branch derivative, clamp flat outside and linear inside, smoothstep kills its own clamp at both ends, matches the closed form, product of clamped factors | no (pure CPU) |
| `dual_field` | the field's own f64 dual gradient against the field's own f32 values | gradient matches; gradient is **exactly zero** outside the shell | no |
| `dual_noise` | the noise's f64 dual gradient against its own values | gradient matches; the clamped noise has exactly zero gradient outside | no |
| `seam_probe` | the C¹ seam across volume face edges: the slope of the **same physical tangential direction** on both sides of every edge | none — it prints ratios (jump ÷ in-face rate of change); a seam is "value continuous, slope discontinuous", so the reading is the verdict | no |
| `sphere_probe` | a **standard soft-edged sphere** so brightness can only come from starlight; bakes `bake_stars` → `bake_emission` → `raymarch_sky` and writes `target/probe/sphere.png` | none — it prints distributions (emission inside the sphere, image luminance percentiles) | yes |
| `star_probe` | star position density by volume-equal bands and by octant, index/payload memory, sphere-query cost, and lit-area density per cube-map ring for both a freshly marched and a baked sky | none — it prints each ratio against 1.0 | no |

Flags: `dual_noise --diagnose` additionally prints the raw data behind its checks (per-axis dual
gradients, central differences at four step sizes, whether each octave is clamped, the fbm mean) and
is explicitly **not** a check (it has no assertions, so a `✓` from it would be a false green).
`sphere_probe --dump` runs a small CPU-side bake and prints the emission distribution instead of
rendering. `seam_probe <VOLUME.pxart> [lane] [条数]` and `star_probe [face] [SKY.pxart]
[cloud.density.pxart]` take positional arguments.

`seam_probe`, `sphere_probe`, and `star_probe` are **diagnostic instruments**: they report numbers and
have no threshold, so their exit code only reflects "it ran". Treat their output as a reading, not as
a gate.

## The arbiter

`cargo run -p px_probe --bin field_dual` (or `.\tools\px.ps1 -Task field_dual -Level opt`) is the only
acceptable check of gradient correctness.

**What it compares.** The production clouds shader is assembled exactly as the host assembles it
(`px_probe::common::assemble` → `px_shader` roots, lookup, and `wgpu_host_stub`) and compiled onto a
probe device with an extra compute entry point appended. That entry point samples the production
field and its hand-written analytic gradient. On the CPU side, `px_verify::cloud_field::CloudFieldParams`
is the **same field written once, generically over a scalar type**, so the same code produces values in
`f32` and derivatives in `f64` dual numbers (`num-dual`'s `Dual64`). The probe compares the two.

`px_verify` depends only on `px_field_schema`, `px_volume_schema`, and `num-dual` — never on an
operator crate — so the reference cannot quietly become the implementation it is grading.

**Points and parameters.** 512 shell points from a fixed xorshift seed, radius uniform in
`[1.01, 1.06]`, direction uniform on the sphere; `CloudParams::new(1.01, 1.06, 900.0)`; a constant
coverage mask of `153/255`, and a varying one for the coverage-term check. Only points with
`field > 1e-3` are compared.

**The four checks.**

1. `the_reference_ports_the_shader_parameters_exactly` — altitude and radius within `1e-5`, direction
   within `1e-6`, coverage remap within `1e-6`. The yardstick must be a port of the shader's
   `medium_of`, not a similar-looking formula.
2. `the_reference_and_the_shader_agree_on_the_field_value` — each of the six staged terms
   (`footprint`, `lobed`, `floor_here`, `ceiling`, `under_top`, `shape`) within `1e-4`, noise within
   `1e-4`, the two coverage paths within `1e-6`, the final field value within `1e-4`, and the
   single-octave and multi-octave tower/skin samples within `1e-4`. `fbm_3` and `gradient_noise_3` are
   additionally pinned at one fixed point to `1e-6`. Requires more than 40 live points.
3. `the_shader_coverage_term_matches_the_exact_band_gradient` — with a varying coverage mask, the
   median relative error of the coverage term against the exact band gradient must be `< 1e-4`, and
   the deliberately **broken** version of that term must be `> 1e-2`. That second half is the negative
   control: the check has to demonstrate it can tell right from wrong. Also, the `baked.g` the GPU
   reads must match the baked value within `1e-6`. Requires more than 20 usable points.
4. `the_shader_analytic_gradient_matches_the_exact_field_gradient` — median relative error `< 1e-4`,
   worst point `< 1e-2`, and the median must be **better than the best of five finite-difference
   steps** (`8e-5, 4e-5, 2e-5, 1e-5, 5e-6`). Requires more than 40 live points.

**Why f64 dual numbers, and why nothing else will do.** A finite difference has two things wrong with
it as a yardstick: a step size (truncation error at one end, `f32` rounding at the other) and a
convention at every fold. This field is full of folds — `clamp`, `smoothstep`, `min`/`max` — and at a
fold a dual number returns the derivative of the branch actually taken (one-sided), while a central
difference crosses the fold and averages two branches. There is no step size at which the two agree
there, so a finite-difference verdict would be measuring its own convention. The dual reference is
exact in `f64`, so any gap that remains is the shader's error. Finite differences are still measured
and printed, in the same probe, as a floor the analytic gradient must beat — the analytic form is only
accepted if it is more accurate than the best difference quotient.

No rendering or cook path ever evaluates a CPU copy of the field. `px_volume_alg` holds a CPU march as
a draft and as an oracle, and no graph or operator may take that path; the reference field above is a
yardstick for a derivative, not a second implementation of the effect.

## Image digests

`px_render::digest` is a self-contained SHA-256 (no dependency: it is a name, not a security
boundary, and adding a crate would touch `Cargo.lock` for a naming convention). Correctness is pinned
by known vectors in its own test: the empty string, `"abc"`, the standard 56-byte vector that forces a
second block, and 64 bytes of `'a'` (the length-is-a-multiple-of-64 case).

The reporting convention is the **first 16 hex characters**, uppercased (`digest::short`), the same
convention as `Get-FileHash -Algorithm SHA256` truncated. The resident service computes
`digest::sha256_file(out)` on the PNG it just wrote and puts the full hex in the shot report's
`sha256` field while printing the first 16 characters in the log line. `--image-hash` is a **preview
window** reading only (it logs the sha16 of each frame's readback); given on any other path it refuses
with exit 64.

An image hash is a name. It says two files are identical or not; it cannot say where they differ, so
it is never the evidence for a pixel-level mistake.

## Image diffing

`px_render --diff LEFT.png RIGHT.png` needs no GPU and no other flags; it exits 0 after printing the
report and 1 if the two images cannot be compared.

What makes a comparison valid:

- **Same dimensions.** Different sizes are refused outright — that is not a difference, it is two
  different things.
- **Roles are positional.** The tool does not know which image is which. The background colour and the
  silhouette are defined by the **left** image alone, on the convention "left = what this host drew,
  right = the reference". Passing them in the wrong order flips the entire report while it still looks
  normal, so the report prints both paths next to left/right and says so at the top.
- Both images must be readable PNG; comparison is on 8-bit sRGB channels with alpha dropped, matching
  what `shot::write_png` writes — the image as seen, not the raw readback.

How it works (`px_render::diff::compare`):

- **Background** = the most frequent RGB triple in the left image. In a shot containing only a clear
  colour and a planet, that is the clear colour.
- **Silhouette** = every left-image pixel that is not the background. All "inside/outside" readings are
  relative to this silhouette, and both directions are always reported.
- **Regions**: all differing pixels; differences inside the silhouette; differences outside it.
  Differences are also banded by distance from the silhouette centre in units of the silhouette's
  equivalent radius (`sqrt(area/π)`): inner `< 0.95`, limb `0.95–1.00`, ring `1.00–1.15`, far `≥ 1.15`.
  Each band carries a pixel count, a maximum and a mean channel delta, and a bounding box, because the
  same count can mean "a scatter of rounding" or "one object drawn wrong".
- **Δ buckets** (`1 / 2 / 3–4 / 5–8 / 9–16 / 17–32 / 33–64 / 65+`) per band, because "a ring of ±1
  rounding" and "a ring of real differences" have the same pixel count and different meanings.
- **Correlations** inside the silhouette: luma, and a high-pass version (a horizontal lag-2
  difference). A smooth veil laid over the same pattern drops the luma correlation but leaves the
  high-pass one intact; a pattern that is genuinely different drops both.
- **Large differences** (channel delta > 8) get a radius range and a coarse `48 × 24` map of where
  they fall, limited to `≤ 1.0` equivalent radius — outside that the stars dominate.
- **Edge vs flat**: a pixel counts as "on an edge" if any 4-neighbour differs by more than 8. Raster
  edge and sample-position errors hug geometry; a wrong shading formula spreads over the surface.
  The report gives the counts of differing pixels that are on an edge, inside and among the large ones.
- **Isolated pixels** (a difference whose four neighbours are all bit-identical) are counted and listed
  up to a print cap, sorted by radius, split into inside/outside — outside the silhouette there is
  only clear colour, so an isolated difference there points at a path that affects everything (blit or
  sRGB round-trip), not at shading.
- **`far_not_background`**: the number of left-image pixels beyond `1.2` equivalent radius that are not
  the background. Zero means uniform input survived every path unchanged.

The diff judges nothing: it prints the readings and the judgement is the reader's.

## Reference images

`art/reference/` holds three images (`gasgiant-saturn.png`, `gasgiant-uranus.png`, `moon-real.webp`)
and a README. They are **visual targets** — "what this should look like" — for the two gas giants and
the moon.

They are explicitly **not** assets, not textures, and not verdicts:

- nothing reads them. The only mention in code is a comment in an art operator; no key, gate, test,
  or render path references the directory;
- they carry no hash that anything checks, so editing or deleting one changes no key and no test
  outcome;
- they are not pixel targets: they are not this pipeline's output, and their resolution, camera, and
  post-processing differ. The judgement is "look at them side by side and state which knob changed and
  how the numbers moved";
- their provenance is not fully established, so they must not be redistributed and must not be used as
  texture assets.

## The render-side harness (`tools/`)

`tools/harness.ps1` is the shared base, dot-sourced by the other scripts. It does four things:

- **Resolves artifacts by name, not by path.** `Resolve-Artifact -Graph <graph> -Node <node>` reads
  `target/pcg/<graph>/manifest.json`, takes the node's 64-hex key, and builds the CAS path
  `target/pcg/ab/<first two>/<key>.pxart`. A missing manifest, a missing node, a key that is not 64
  hex, or a missing artifact is an error, never a skip.
- **Asserts the batch pins one shader.** `Assert-ShaderMembersAgree` reads the scene frame out of each
  `.pxart` and groups the pinned shader members by slot; more than one content key under a slot is a
  hard failure unless `-AllowMixedShaders` is passed for a deliberate interleaved experiment. This is
  what protects "one service, one WGSL version" — the protocol assumes the service is not restarted
  between scenes.
- **Runs a single resident service.** `Assert-NoOtherRenderServer` fails if any process named
  `px_render*` is alive or if `target/render-server.json` still exists. `Start-RenderServer` starts
  the exe with `--serve --width --height`, waits for the line `渲染管线全部就绪` (default timeout
  180 s), and sets `WGPU_BACKEND=vulkan` **for that child only** (so this shell and other sessions are
  untouched). `Stop-RenderServer` stops only the pid it started and only removes the lease if it still
  owns it.
- **Turns refusals into failures.** `Invoke-Client` runs the exe, restores the previous environment,
  and throws on a non-zero exit code; `Assert-NoRenderFailure` scans stderr for
  `Refused|后端断言失败|管线编译失败|failed to process shader|设备实例已经暂停|0x887A0005`. Of these,
  `Refused` (the service refusing a request) and `后端断言失败` (the host failing to get a Vulkan
  device, `px_render::gpu::BACKEND_ASSERT`) are the ones this host can actually produce; the other
  four are strings it never emits.

The host locks Vulkan in code (`px_render::gpu::instance`), so the environment variable cannot change
which backend a shot was rendered on. Probes do honour `WGPU_BACKEND`.

**`tools/frame-probe.ps1`** is the render measurement instrument.

```
-Phase stable|scene|shot|legacy|both|ab   (default: stable)
-Scenes <names>   -Reference <name>   -Graph scene
-Frames 60   -Width 2240   -Height 1400   -Rounds N
-Sweep   -Changed   -Windows 4   -DropWindows 1
-Bake <bool>   -AllowMixedShaders   -Exe target\debug\px_render.exe   -Tag fp
```

- **Live: `-Phase shot`.** Bakes (unless `-Bake:$false`), asserts shader agreement, starts one service,
  warms with a shot request, then sends one batch request per round with every scene in that round.
  Rounds alternate direction. The report per shot carries the artifact key, path, byte size, sha256,
  resolution, `has_cloud`, and `verdict`. It then prints the availability table and lists any scene
  that declared clouds but did not draw them. Warm-up uses a **shot** request because this service is
  not started with `--fps`, so a perf request would be refused.
- **`has_cloud` is computed by the service** (`px_render::report`), from the readback bytes, not from
  the PNG. It is true when the artifact declared a `clouds` part, no placeholder-magenta pixel is
  present, and either this shot is the batch's first image and has any bright pixel (`r+g+b > 72`), or
  this shot's `16×10` grid flux sum differs from that first image's by at least `1%` of the first
  image's total flux (floor 1). Placeholder magenta is checked **first**, so "the shader never got
  installed" reads as "this shot does not count" rather than "clouds were expected and missing".
- **Live: `-Phase ab`.** Requires exactly two scenes. Bakes, asserts shader agreement with
  `-AllowMixed:$true` (a differing key is the independent variable here), warms, then shoots
  `A B A B` and compares the sha256 of `#1` vs `#3` (same scene must reproduce byte for byte) and `#1`
  vs `#2` (the two shader versions must differ). This is the positive control for "switching shader
  versions changes the image".
- **Retired: `stable` (the default), `scene`, `legacy`, `both`.** Each refuses immediately through
  `Stop-RetiredPhase` and sends no request. They need a timing frame loop: per-frame sampling,
  dropped windows, waiting for K frames after a rebuild, and per-pass encoder-level GPU timestamps.
  The only host here renders **on demand** — one request draws one frame and replies — so there is no
  per-frame sequence and no such spans, and the service rejects every job that is not a shot job
  (`px_render::serve::steps_of`).
- **`-Changed`** compares the artifact keys in the current manifest against a saved copy and reports
  which scenes actually changed. A shader edit changes every scene that pins it, so that output is a
  list of suspects, not a test plan.
- **`-Sweep` selects the five-scene set** (`orbit-bare`, `orbit`, `orbit-surface`, `orbit-proxy`,
  `orbit-soft`) and forbids passing `-Scenes` at the same time. It refuses only because the default
  phase refuses; `-Sweep -Phase shot` is a valid five-scene shot batch, and `-Phase shot` without
  `-Scenes` defaults to those same five scenes. `-Windows` and `-DropWindows` are read only by the
  retired `scene` and `legacy` phases.
- The timing instrument this host does have is
  `px_render --offline --scene <artifact> --out <png> --spans 预热,测量,轮数`: all three numbers are
  required, it is mutually exclusive with `--sheet`, and each `--scene` must have an `--out` because
  the last frame's PNG is part of the reading (it must be byte-identical to a run without `--spans`).
  It reports each pass's two boundary pairs (envelope, and inside-the-pass — the latter is the same
  boundary as an encoder-level `elapsed_gpu`) plus a frame-level span and a matched-subset sum. That
  number is deliberately **not** called `gpu_ms`. `--time N` is the coarser instrument: prepare once,
  draw N frames, report prepare / first frame / frames 2..N.
- For a wider sweep of scenes, the usable path is `-Phase shot -Sweep` (or `-Scenes <list>`). No live
  phase reports per-window timings, and there is no replacement for the retired ones.
- **`Assert-ShaderMembersAgree` also protects the measurement, not just the images.** Mixing shader
  versions means the service recompiles pipelines when the scene switches, so a timing window would
  contain compile time and a shot could contain a missing cloud shell.

**`tools/probe.ps1`** takes twelve camera views of one scene (equator at four yaws, two mid
latitudes, two poles, and four cube-map critical directions: a corner top and bottom, an edge middle,
a face centre) and tiles them into `target/probe-sheet.png` with the tag drawn on each tile. It fails
if any view produces no image. It has no numeric pass condition — the contact sheet is the reading.

**`tools/probe-clouds.ps1`** shoots four cameras for each of a list of scenes (`orbit`, `orbit-bare`,
`orbit-surface`, `orbit-proxy` by default) at `-Size 1100`, starting and stopping a service per scene.
Ablation is expressed as scene variants (`ablate = "surface"` in the cloud part parameters, baked as
`orbit-surface`), not as a flag. Fails if a shot is missing; the verdict is eyes on the images.

**`tools/px.ps1`** is the single entry point for both of these and for the bake/probe tasks.
`-Task <probe name>` runs that probe; `-Task test` runs `cargo test`; `-Task test-all` runs
`cargo test --workspace`; `-Task list|build|gc|run` drive the graph driver; `-Task planet|desert|clouds`
bake graphs. `-Level opt` promotes the local crates to `opt-level=2` without touching dependencies.

## What `cargo test` gates

528 test functions across 25 crates. The gates that carry the most weight, by area:

- **Protocol**: `roundtrip` (stream frames round-trip byte-identically, foreign magic rejected,
  handshake mismatch rejected, blob length and dtype checked, `f32` payload exact); `snapshot`
  (the canonical protocol JSON is current and drives `protocol_hash`; regenerate with
  `PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol`); `cube`, `cubemap`, `octahedral`, `domain`
  (direction/face/UV mappings round-trip); `crate_graph` (the protocol's dependencies are whitelisted
  and never pull the host).
- **Graph identity**: `px_graph/tests/keys.rs` (a key is values + interface + implementation source +
  input keys; comments and spacing do not change it; a shader key follows its import closure);
  `source_hash` (every fingerprinted crate fingerprints its own sources; nobody hand-lists operator
  sources); `px_graphs/tests/crate_graph.rs` (operator libraries are dylibs; graph scripts do not link
  operator or algorithm libraries; schemas do not link operators); `inst_gate` in both `px_decls` and
  `px_graphs` (one row per `px_op!` declaration, every row resolves, the generated instance library
  path is the planned key); `ops_load` (every declared operator loads from its library and reports its
  own source hash).
- **The shader layer**: `px_shader`'s 21 tests, including the pinned assembled byte counts and closure
  fingerprints (see `docs/shaders.md`), the module-table rules, and the stub-table identity.
- **Pass execution**: `px_pass`'s 36 tests — render-state and document round-trips, attachment and
  entry-point validation (refused by name, with the names that do exist), viewport and cell sizing,
  timestamp-slot contiguity, copy passes, layered depth, and a geometry pass that actually draws,
  reads back and culls.
- **Volume**: `px_graph/tests/volume.rs` (blob round trip, grid sampler hits nodes exactly, cube faces
  share edges), `px_volume_alg`, and `px_volume_gpu_op`'s GPU-vs-CPU point-by-point comparisons (these
  need a device and skip when there is none).
- **NURBS and mesh**: `px_nurbs_schema/tests/analytic.rs` (a rational circle stays on its radius, the
  hodograph is perpendicular to the radius, knot insertion and degree elevation leave the curve where
  it was, plane normals, bit-exact payload round trip); `px_graphs/tests/nurbs_pipeline.rs` (a sphere
  and a circle reach a watertight mesh through the loading gate, reproducibly, on both CPU and GPU).
- **Field and clouds**: `px_field_schema/tests/cube_map_sampling.rs` (a smooth field stays smooth
  across face edges); `px_verify/tests/detail_curve.rs` (the concave remap's chain-rule factor equals
  `d√b/db`, the reference gradient follows the remap, and the remapped noise keeps a positive lower
  bound on the surface so the `1/(2√b)` factor cannot blow up); `px_graphs/tests/cloud_proxy.rs` (the
  proxy is closed, the mesh op is reproducible, the proxy encloses the coarse and final fields, the
  gradient bound is above the measured gradient).
- **Scene**: `px-scene`'s 54 tests — recipe parsing, frame materials, contract merging, shadow tables.
- **Host**: `px_render`'s 85 tests (group 0 layout against the view stub, the stub table's three named
  differences from the placeholder table, unknown symbols rejected, digest vectors, diff regions,
  cloud verdict branches). These are in the fast chain's blind spot: `px_render` is not a default
  member, so run `cargo test -p px_render` when you touch it.
- **Simulation**: `game`'s 84 tests (conservation, boundedness, solubility over long runs). Not a
  default member and not part of the rendering or PCG pipeline.

**GPU-backed tests require a device.** There is no skip path: `px_gpu::require_gpu` panics with
`this check requires a working GPU` when no device is available, so `px_gpu`, `px_volume_gpu_op` and
`px_nurbs_gpu_op` fail rather than report green without having run. A machine without a GPU is not a
valid place to take those verdicts, and it now says so instead of hiding it. The instruments that
never degrade silently are the probes (`require_gpu` exits 2) and the host
(`px_render::gpu::refuse`, exit 2).

## Discipline

- **A skipped check is not a pass.** A probe that cannot get a device exits 2. `run_checks` prints a
  line per check and exits 1 on any failure. Probe checks that need data first assert they have it
  (`rows.len() == POINTS`, "more than N live points", "the sampler covered a free region"), so a
  pipeline that silently produced nothing cannot read as success. Unit tests that need a device fail
  loudly for the same reason.
- **Do not run the full suite out of habit.** Run what the change affects. Do not add redundant tests.
  A check that costs real computation belongs in a probe, not in `cargo test`.
- **One target, one reader.** When a fixture moves, change the reader in the same commit and run it
  once to confirm it is not skipping.
- **A verdict that can be deleted is not a verdict.** Anything under `target/` can vanish, so fixtures
  live in git and instruments do not. A fixture nobody reads is not a verdict either.
- **Fixtures and stubs must not shield the thing under test.** The text a probe compiles must be the
  text the host compiles; the bake side and the render side must run on the same backend.
- **Paired measurements for performance.** Alternate A/B/A/B. A single A→B reading has shown a 2×
  spread for the same executable.
- **The CAS key does not include environment variables.** To measure a feature toggled by an
  environment variable, vary a parameter as well, or the second run is a cache hit and you are timing
  the disk. The diagnostic toggles that exist are `PX_SKIP_OFF` / `PX_SKIP_REPORT` (occupancy mask),
  `PX_AUDIT_SHADOW` / `PX_SHADOW_FULLPAGE` / `PX_NO_SHADOW_BIAS` (shadows), `PX_PASS_LIST` (print the
  full executed-pass list), `PX_OP_DIR`, `PX_ART`, `PX_PCG_FRESH`.
- **Read logs in full.** Tailing the last few lines hides the error that matters.
