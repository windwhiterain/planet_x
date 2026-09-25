# Graph programs and the `px` driver

A graph is a Rust binary. There is no graph file, no runtime DAG, and no interpreter: a graph *is*
a `main()` that calls operators in order, with `px_cook::cached` in between. `px_graphs/Cargo.toml`
declares no `[[bin]]` section, so every file in `px_graphs/src/bin/` is a graph program or a tool
that ships with them.

`cached` is the only cache entry point in the engine. It computes the node key from the operator's
identity, the canonical parameter JSON, and the upstream keys; on a hit it decodes the CAS payload,
on a miss it calls the operator, encodes the payload, and writes it to
`target/pcg/ab/<first 2 hex>/<64 hex>.pxart`. `Graph::finish()` then writes the graph's
`target/pcg/<graph>/manifest.json` (node name → key), `target/pcg/<graph>/params.json`, and appends
this run to `target/pcg/<graph>/metrics.jsonl` (see [graph.md](graph.md) for the ledger's fields and
rotation).

## The programs

| Binary | Graph name | Produces |
|---|---|---|
| `planet` | `planet` | two fbm/ridged fields, a constant mask, mixed terrain, remapped height, and a `mesh.cubesphere` surface |
| `desert` | `desert` | two fbm fields, one ridged field, a warped canyon field, a constant blend mask, mixed terrain, remapped height, and a cubesphere surface |
| `clouds` | `clouds` | a seven-step cloud field chain with three `field.gradient` slope fields, two `cloud.coarse` volumes (coarse and fine) and their two `mesh.proxy` hulls; runs the containment and gradient-bound verdicts on the baked artifacts |
| `moon` | `moon` | an fbm base field, three `field.stamps` crater layers, a remapped height field, and a cubesphere surface |
| `gasgiant` | `gasgiant` | fbm turbulence, a `field.remap/latbands` band field (generic instance), two warp stages, a filament warp, and a remapped band field — **no clouds** |
| `nebula` | `nebula` **and** `nebulasky` | 3D fbm/ridged/warp fields, an envelope, a carved dust layer, a `cloud.density` volume, a `sky.stars` point field, a `cloud.emission` volume, and (second graph) one `sky.nebula` cube-map sky texture |
| `field_remap` | `field_remap` | an fbm field and a `field.remap/waves` field (generic instance); a minimal end-to-end graph for the instance mechanism |
| `scene` | `scene` | a scene document (`.pxart`) compiled from an `art/scene/*.toml` recipe, plus its baked `generated` graph |
| `shaders` | `shaders` | one reflected shader contract per entry in `art/shaders/*.wgsl` |
| `passes` | `passdoc` | a copy of a scene document with a content pass chain spliced into its frame graph |
| `inst_probe` | `inst-op` | no artifacts; loads one instance library and proves it cooks, caches, and re-hits |
| `field_probe` | — | no artifacts; prints readings of an existing field `.pxart` |
| `px` | — | the driver (see below) |

Everything a program bakes lands in the CAS; nothing writes into `art/`.

### Field and mesh graphs

`planet`, `desert`, `moon`, and `gasgiant` are plain Rust call chains with no arguments of their own
beyond `--store <dir>`; the shape (width, height, projection) is written in the script and the
per-node parameters come from `art/<graph>/<node>.toml`.

- `planet` — `art/planet/{continents,mountains,weight,terrain,height,surface}.toml`, shape
  `780×520`, projection `Cube`.
- `desert` — `art/desert/{plateaus,canyons,flow,carved,blend,terrain,height,surface}.toml`, shape
  `780×520`, projection `Cube`.
- `moon` — `art/moon/{terra,basins,craters,pits,height,surface}.toml`, shape `780×520`,
  projection `Cube`.
- `clouds` — `art/clouds/*.toml`, shape `cube_map_extent(256)` with projection `CubeMap`. The node
  named `coarse_fine` is the fine volume, though its parameter file is `coarse_fine.toml` and its
  op id is `cloud.coarse`. `--bound` raises the gradient-bound measurement grid from
  `6 × 64² × 48` to `6 × 256² × 96`, which turns a run of seconds into minutes. The containment and
  gradient-bound assertions fail the process; they are re-checked on every cook, including an
  all-hits run, and they read the **stored artifacts**, not values still in memory.
- `gasgiant` — `art/gasgiant/*.toml`, shape `cube_map_extent(256)`, projection `CubeMap`, plus the
  generic instance node `bands`. It deliberately does not cook clouds: its consumer is the
  sub-surface-scattering material `art/shaders/gasgiant.wgsl`, which reads the band field through a
  cube map at `@binding(7)`.

### `nebula`

The only program that runs **two** graphs in one process, because the two halves need different
canvases: the volume chain needs a volume-grid canvas (`res × res²·layers·6`), the sky needs a
cube-map canvas (`res × res·6`). The graph names differ (`nebula`, `nebulasky`), so their parameter
directories and manifests do not collide.

Arguments (all read through `px_cook::args_without_store`, so `--store` never confuses them):

| Argument | Default | Meaning |
|---|---|---|
| `--shape <n>` | `64` | face resolution of the 3D field grid, and the `res` of the `cloud.density` volume; `max(8)`. Cost scales as `res²·layers`. |
| `--layers <n>` | `art/nebula/density_volume.toml`'s `layers` | radial layer count of the **volume canvas**; `max(8)`. The operator's own `layers` parameter still comes from `density_volume.toml`, so giving a different value here makes `cloud.density` resample instead of copying — allowed, but it wastes a full sampling pass. |
| `--face <n>` | `64` | parsed and echoed in the startup line only. It reaches no node: the sky resolution is `art/nebulasky/sky.toml`'s `face`, and the field/volume resolution is `--shape`. |

Parameter files: `art/nebula/*.toml` for the volume graph and `art/nebulasky/{sky,stars}.toml` for
the sky graph. The star field is cooked **once**, in the `nebula` graph, as the node `stars`; the
`nebulasky` graph consumes that same `Cooked` handle as an input to `sky` (the same way it consumes
the `emission` volume), so one key and one artifact serve both the point lights and the sky points.
Because of that, its parameters have a single source — `art/nebulasky/stars.toml`, read by hand
once and fed to the `stars` node. `px run scene nebula` renders it (the recipe sets
`skybox = "nebulasky::sky"`).

### `field_remap`

`field_remap` is the minimal graph for the generic-instance mechanism: a `field.fbm` node feeding a
`Waves` instance node called `bands`. Its parameter directory is `art/field_remap/`, which does not
exist, so every node falls back to `Default`. It asserts that the cooked field stays inside
`[0, 1]` — that bound is the field function's own contract, and one is expected to live in
`art/inst/waves.rs`.

### `shaders`

`shaders` reflects every entry point in `art/shaders/*.wgsl` into a shader contract, writes each to
the CAS, and writes `target/pcg/shaders/manifest.json`. Slots are the `.wgsl` files that do **not**
carry `#define_import_path` (that marker means "library module"); `art/shaders/lib/*.wgsl` are
libraries. Files are baked in sorted order, so two bakes of the same tree produce a byte-identical
manifest. The bake itself lives in `px_cook::bake_shader_graph` so that downstream code (the scene
frame-graph compiler and its tests) can call it instead of depending on someone having run this
binary first.

### `passes`

```
passes <scene .pxart> <recipe name> [output path] [--frame <frame name>] [--no-frame-graph]
```

The recipe is `art/passes/<name>.toml`: an optional `name` override, optional `[[resources]]`
(private scratch targets this recipe declares), and `[[passes]]` entries with `kind`, `shader`,
`label`, `entry` (default `fs_main`), `reads`, `writes`, and `params`.

In the default (frame-graph) mode the content passes are spliced **between** the frame graph's
`before` and `after` segments, targets are assigned from the frame's `chain_color` ping-pong pair,
the last pass always writes back into the chain, and the frame's fullscreen blit is rewired to read
the chain tail. `writes` may only name `view` or a resource the recipe itself declares; a declared
resource that no pass writes is rejected. `kind = "compute"` and every kind other than `fullscreen`
are rejected at bake time — the executor does not implement them.

`--no-frame-graph` is not a supported alternative: content recipes no longer name their targets, so
this path is refused with an explicit message. To reproduce the old-shape frozen documents, use
`--bin scene <recipe> --no-frame-graph` instead.

The output goes to the given path, or, when omitted, to the CAS. `passdoc` is printed as the graph
name only to get the one-line `begin` summary.

### `inst_probe`

`inst_probe [op_id]` (default `cloud.coarse/band`, and it asserts that this is the only op id it
knows). It builds the same build graph the driver uses, finds the instance's library path, refuses
to continue if the `.dll` is missing (printing the driver command that would build it), then cooks
the instance twice and asserts the second cook was a cache hit with the same key. This is the only
verdict that an instance library actually loads.

### `field_probe`

`field_probe <FIELD.pxart> [projection]`, projection one of `cube_map` (default), `cube`,
`equirect`, `octahedral`. It prints the width/height, the longitude wrap seam (column 0 vs column
`W-1`), the six rows with the largest mean adjacent-row delta, the three columns with the largest
mean adjacent-column delta, the median row delta, and — for spherical projections — the
latitude-explainable fraction `R²`, the median in-row longitude standard deviation, the median
in-row autocorrelation at `lag = W/8`, and the longitude/latitude gradient ratio. `R²` alone cannot
separate "pure latitude bands" from "smooth long-wavelength streamers"; the two extra readings can.

## What a graph's directory under `art/` contains

```
art/<graph>/<node>.toml     parameters of one node, keyed by the node's name
art/scene/<recipe>.toml     scene recipes (the `scene` graph)
art/frame/<name>.toml       frame graphs, plus the .wgsl they reference
art/passes/<recipe>.toml    content pass recipes (the `passes` tool)
art/shaders/*.wgsl          shader entries (and lib/*.wgsl modules)
art/inst/*.rs               generic operator bodies compiled into instance libraries
art/reference/              reference images and their README
```

A node's parameter file is `<param_root>/<graph>/<node>.toml`, where `<graph>` is the name passed
to `begin(GraphSpec { name })` and `<node>` is the string passed to `cached(&graph, "<node>", …)`.
`node_params(&graph, "<node>")` reads it; a missing file is **not** an error, it yields
`P::default()`. `Graph::finish()` prints how many nodes had no parameter file at all and writes the
effective values to `target/pcg/<graph>/params.json`; it also appends the run's measurements to
`metrics.jsonl` (a failed append is reported and does not fail the run).

`param_root()` is `art/` unless `PX_ART` is set — either in the environment, or by the `--store
<dir>` argument, which `px_cook::apply_store_args()` must be called with as the **first line of
`main`**, before any `begin` or `node_params`. A relative `--store` value resolves against the
workspace root, not the process working directory. The parameter directory is not part of any node
key: keys follow the bytes, not where they live. Every program that also reads positional arguments
does so through `px_cook::args_without_store()`, so that `--store <dir>` is stripped rather than
being mistaken for a recipe name.

`art/inst/*.rs` are not graphs and not operators in any library. They are the generic-parameter
files that `px build` compiles into a content-addressed instance library. Each must define the type
named in `px_graphs/src/inst_recipe.rs` and use only full paths (no `crate::`, no `#[cfg(test)]`,
no private names), because the generated crate `include!`s the file verbatim.

## Generic instances: what stage 1 builds

Stage 1 has exactly one source of truth, `px_graphs/src/inst_recipe.rs`: a data table of
`InstRecipe { op_id, decl, type_name, roots, source, body }` literals. `px_graphs/build.rs` reads
that table (as a module, via `#[path]`), validates it against `px_decls` and the workspace member
list, and writes `OUT_DIR/insts_gen.rs` (one generated type per instance, implementing
`PxOp`/`InstNode` with const facts), `OUT_DIR/elem_gen.rs` (one unit struct per element function),
and `OUT_DIR/insts_gen_catalogue.rs` (op id → how to compile it). It never invokes cargo.
`px_graphs/src/insts.rs` `include!`s `insts_gen.rs` (re-exporting `Band`, `LatBands`, `Waves`) and
the catalogue; `px_graphs/src/elem.rs` `include!`s `elem_gen.rs`. Stage 1's nodes come from

```rust
px_graphs::insts::build(&mut graph)   // px_cook::inst::BuildGraph
```

which registers every instance as a build-graph node; `px_graphs::insts::codegen()` is the
catalogue.

There are two kinds of rows and **seven** instances today:

- the recipe table's three declarations — `cloud.coarse/band` (`Band`, body `art/inst/band.rs`),
  `field.remap/waves` (`Waves`, `art/inst/waves.rs`), `field.remap/latbands` (`LatBands`,
  `art/inst/latbands.rs`);
- the four element functions from `px_elem::ELEM_SPECS` — `field.constant`, `field.mix`,
  `field.remap`, `field.fuse`, whose bodies live in `px_elem/body/*.rs`.

The key of an instance is `px_cook::inst::key_of_facts`: `"px_inst/v1"` ‖ toolchain hash ‖
declaration hash ‖ the source roster of every `roots` crate ‖ interface hash ‖ the normalized body
template ‖ the body file's bytes. The **op id is not in the key**, and neither is the graph
program's own source fingerprint — two graphs using the same body file and template share one
library and one artifact.

## The `px` driver

`px` is the only driver. It owns three verbs; `list` and `build` take no graph name, because stage 1
is a property of the crate, not of a graph.

### `px list`

Prints `实例 N 条：` and then one line per instance, in build-graph order:

```
  <op_id:<20> <decl_hash[0..12]:<12> <roots joined by ",":<16> <source:<18> <key[0..12]> 有|缺
```

`有`/`缺` is whether `target/pcg/inst/<key>.dll` exists. Nothing is compiled, nothing is written.
Today it prints seven lines.

### `px build [--gc] [--deep] [--target]`

Compiles the instances whose library file is missing, **one at a time and continuing after a
failure** — unlike `compile_missing()`, which returns at the first error. Both use the same
`inst::compile_one`. Output per instance is `已有 <op_id>（<short key>）`,
`已编 <op_id> → <short key>`, or `失败 <op_id>（key <full key>）:` followed by cargo's stderr
verbatim. It ends with `共 N 条：已编 a、已有 b、失败 c`, and returns a non-zero exit code if any
instance failed.

Compiling one instance means: generate `target/jit/<key>/{Cargo.toml,src/lib.rs,build.rs}` — an
isolated workspace (`crate-type = ["dylib"]`, its own `[workspace]`, absolute path dependencies)
whose `[dependencies]` are `px_graph_schema` plus the instance's `roots` (and its declaring schema,
for the declaration tier) — run cargo against `target/jit/target/`, copy the produced dylib to
`target/pcg/inst/<key>.dll`, and write the sidecar `target/pcg/inst/<key>.json` next to it. A
failed instance's generated sources are left on disk so the compile error can be mapped back to a
recipe line.

The sidecar is strict JSON (one field per line, no trailing comma) so that tools can read it back:
the key file name is the identity, and the sidecar's `toolchain` field is a readable copy of the axis
the instance key already folds in — it records the library's `TOOLCHAIN_HASH`, so it **cannot separate
`-Level opt` from `-Level dev`**, which produce that same hash. What it enables is a plan-time
comparison: `px list` marks a library whose recorded build differs from the current one, and `px run`
refuses one. The compiled library exports the same hash as a symbol, so the sidecar's text and the
library's own symbol carry the same value. A sidecar that is missing and one that will not parse are
reported as different conditions, because they call for different reactions (`-Task list` prints
`没有 sidecar` for the first and the parse error for the second).

Flags:

| Flag | Effect |
|---|---|
| `--gc` | after compiling, remove non-live instance artifacts (see below) |
| `--deep` | refinement of `--gc`: also remove non-live `target/jit/<key>/` source directories |
| `--target` | refinement of `--gc`: also remove the shared `target/jit/target/` build cache |

`--deep` and `--target` are rejected unless `--gc` is also given. Any other flag is rejected with
the usage text. All rejections exit 1.

### `px run <graph> [--build] [--store <dir>] [-- graph args…]`

1. **Stage 1 (plan only).** Builds the build graph and prints
   `stage 1｜实例 N 条：命中 x、缺 y`. If anything is missing:
   - with `--build`: prints `stage 1｜--build：编缺的那些`, compiles via `graph.compile_missing()`
     (first failure aborts), and prints `stage 1｜<summary>`;
   - without `--build`: it **does not compile**, prints
     `缺 <n> 条实例库（共 N 条）：` followed by the missing keys and the two exact commands
     (`<driver> build` and `<driver> run <graph> --build`), and exits **1**.

   Refusing to compile by default is deliberate: a run is only meaningful once the state is known
   to be static, and letting stage 2 silently trigger a build destroys that. `--build` is the
   explicit request, and compiling is the only thing either verb does that writes to disk besides
   the CAS.
2. **Stage 2.** Prints `stage 2｜<path to the graph exe>` and runs it. The graph executable is
   looked up **next to the `px` executable** (the same `target/<profile>/`), then in
   `target/debug/`, then in `target/release/`; if none exists the error names the command
   `cargo build -p px_graphs --bin <name>`. Stage 2 never nests `cargo`.
3. **Exit code.** The graph executable's status code is passed through unchanged; a graph's verdict
   *is* its exit code.

Argument parsing is positional-tolerant: `px run scene orbit-bare` and `px run scene -- orbit-bare`
are the same thing, because every `px` switch starts with `-`, so anything that does not start with
`-` can only belong to the graph. `--store <dir>` and `--store=<dir>` are both accepted; the value
is forwarded to the graph executable as `--store <dir>` **before** the graph's own arguments (where
it belongs semantically, not positionally — graph programs strip it themselves).

### Exit codes

| Situation | Code |
|---|---|
| `list` on a healthy tree | 0 |
| `build` with instances compiled or already present | 0 |
| `build` with at least one failed instance | 1 |
| `build` with an unknown flag, or `--deep`/`--target` without `--gc` | 1 |
| `run` with a missing instance and no `--build` | 1 |
| `run` where the graph executable cannot be found | 1 |
| `run` otherwise | the graph executable's own code |
| no subcommand, or an unknown one | 1 (usage on stderr) |

## Garbage collection

Instance libraries are a pure cache: the file name **is** the build fingerprint, that fingerprint
is recomputable from disk by `px_cook::inst::key_of_facts`, and `px build` can recreate any of them.
The worst case for deleting a live one is one rebuild; correctness never depends on the file being
present, because keys are computed, not looked up on disk.

- **Live set** = the keys of every node in this crate's build graph (`inst::live_keys(graph)`, i.e.
  `px_graphs::insts::build`). Judging liveness is per graph-program crate; a second crate declaring
  instances would have its libraries treated as garbage (still rebuildable, but wasted work).
- `--gc` scans `target/pcg/inst/`. Entries whose extension is `.dll` or `.json` and whose file stem
  is not live are deleted; anything else in that directory is reported as `跳过 …（不是实例构件）`.
  Live entries also count as skipped.
- `--deep` additionally scans **every** directory under `target/jit/`, not only the keys whose dll
  was just deleted — a key that never got a dll, or whose dll was removed by hand, still leaves a
  source directory behind. Live keys are kept, `target/jit/target` is skipped (it is shared,
  keyless build cache), and every other directory is removed recursively.
- `--target` removes `target/jit/target/` outright — the cargo intermediate directory shared by all
  instances, which cannot belong to any single key and therefore cannot be reached by the per-key
  loop. It is recreated on the next build.

Each removal prints the path and its size; the closing line is
`共 N 条：活 N、删 d（释放 X MB）、跳过 s`.

## The PowerShell wrapper

`tools/px.ps1 -Task <name> [-Level dev|opt|release] [-Graph <graph>] [-Deep] [-- <extra cargo args>]`
changes to the workspace root, builds a cargo command line, prints it, and forwards the exit code.
`-Task` is not named `-Target` because PowerShell binds by prefix and `--target` would be swallowed
by `-Target` instead of reaching the driver.

| `-Task` | cargo command |
|---|---|
| `list` | `cargo run -q -p px_graphs --bin px -- list` |
| `build` | `cargo run -q -p px_graphs --bin px -- build` |
| `gc` | `cargo run -q -p px_graphs --bin px -- build --gc` (`-Deep` adds `--deep`; `$Rest` forwards `--target`) |
| `run` | `cargo run -q -p px_graphs --bin px -- run <Graph>`; exits 2 with a usage line when `-Graph` is empty |
| `planet`, `desert`, `clouds` | `cargo run -p px_graphs --bin <task>` |
| `field_dual`, `gradient`, `device`, `dual`, `dual_field`, `dual_noise` | `cargo run -p px_probe --bin <task>` |
| `test` | `cargo test` |
| `test-all` | `cargo test --workspace` |
| `check` | `cargo check -p` the eleven pipeline packages: `px_protocol`, `px_graph_schema`, `px_graph`, `px_field_schema`, `px_field_op`, `px_volume_schema`, `px_volume_op`, `px_mesh_schema`, `px_mesh_op`, `px_verify`, `px_graphs` |

`-Level opt` appends `--config profile.dev.package.<pkg>.opt-level=2` for each of
`px_graph_schema`, `px_graph`, `px_field_schema`, `px_field_op`, `px_volume_schema`,
`px_volume_op`, `px_mesh_schema`, `px_mesh_op`, `px_verify`, `px_graphs` (plus `px_probe` for the
probe tasks); `-Level release` appends `--release` instead. The driver tasks (`list`, `build`,
`gc`, `run`) deliberately do **not** receive these per-package overrides: instance libraries form
their own workspace root under `target/jit/<key>/`, which the main-workspace config cannot reach.

The toolchain axis of an instance key is `rustc -vV` + `TARGET` + `RUSTFLAGS` + `PROFILE`, so the two
non-default levels relate to identity differently and the wrapper warns on stderr when either is
selected:

- **`-Level opt` shares instance keys with `-Level dev`.** Cargo still reports `PROFILE=debug`, and the
  per-package override does not reach an instance library at all: those are built in their own nested
  workspace under `target/jit/<key>/`, to which only `PROFILE`/`TARGET`/`RUSTFLAGS` are passed. The two
  levels therefore produce the same instance-library bytes and the same keys.
- **`-Level release` rotates every instance key**, because `PROFILE` changes. Expect
  `-Task list` to report every instance `缺` on the first run after switching; that is the rotation,
  not a regression, and `-Task build` compiles the new family.

What the gate compares is a library's **recorded** build against the current one, rather than trying to
read a level out of it, and its subject is narrow: **one key produced by two different builds**. A
`-Level release` switch is not that case — it changes `PROFILE`, which changes every instance key and
with it every library path, so the switch presents itself as a plan full of **missing** libraries and
`build` fills it. What the comparison catches is a library that sits at a key the plan still computes
while its record says another build: the environment captured when the library was built differs from
the one the key was computed with, which a partially rebuilt tree can produce. `-Task list` reads the
`toolchain` field of the sidecar next to each compiled library (`target/pcg/inst/<key>.json`) and marks
such a library with `!`; `px run` **refuses** that plan, naming the operator and both hashes, and
`px build` treats the library as work to redo. The loader makes the same comparison from the library's
own symbol, so a mismatch that goes unnoticed at plan time is refused when the library is opened. Across
`opt`/`dev` the gate stays quiet, because those two produce the same hash and the same bytes. A library
with no sidecar is neither marked nor refused — an absent record is not evidence of another build.

The three task names `planet`, `desert`, `clouds` and the `run` task first build
`px_field_op`, `px_volume_op`, and `px_mesh_op` with the same flags, because the operator
implementations are not linked into the graph executable and a graph would otherwise refuse to load
its dylibs. `list`, `build`, and `gc` do not pre-build them — they only touch stage 1. The graph
names `moon`, `nebula`, and `gasgiant` are not task names; they run as
`-Task run -Graph <graph>` (add `--build` after the graph name when stage 1 is incomplete).

## Tests: the fast chain and the full one

`Cargo.toml` lists all crates in `members` and a strict subset in `default-members`. The difference
is deliberate:

- **excluded from `default-members`**: `game` (an unrelated market/economy simulation),
  `px_render` (the wgpu host — needs a GPU and minutes), `px_probe` (GPU probes whose verdict is
  their exit code).

So `cargo test` (`tools/px.ps1 -Task test`) is the fast chain and `cargo test --workspace`
(`-Task test-all`) is everything. When adding a crate, both lists must be edited, or the crate
either escapes the gates or silently slows the default chain down.

`px_graphs/tests/` holds the graph-side gates:

| File | What it gates |
|---|---|
| `crate_graph.rs` | the dependency gates: the five operator crates declare `dylib`/`cdylib`; `px_graphs` has no `px_*_op` in `[dependencies]` or `[dev-dependencies]` **and does** depend on `px_cook`; no `px_*_op` depends on `px_graph`/`px_cook`; `px_graphs` depends on no `*_alg`; `px_graph` depends on no operator; the schema crates depend on no operator |
| `inst_gate.rs` | the stage-1 facts, no artifacts needed: `inst_recipe.rs` rows plus `px_elem::ELEM_SPECS` rows equals the number of nodes in `insts::build`; the generated `INST_TEMPLATE` equals the recipe column verbatim and `INST_BODY` is the body with `ARG` substituted; the symbol name matches on both sides; every instance's library path matches the key the generator planned |
| `ops_load.rs` | every declared preset operator really loads its symbol out of its implementation dylib; every library reports its own source hash; the five libraries have distinct identities |
| `elem.rs` | the element tier end to end: compiles the missing instance libraries (via `missing()`/`compile_missing()`), cooks, checks values against independently written closed forms, checks cross-graph key sharing, content identity tied to the body bytes, symbol naming, and that a fused node is bit-identical to remap-then-mix |
| `local_op.rs` | the graph-local operator tier (`px_local_op!`): it cooks, caches, hits, its identity is this graph program's source, and two local operators in one program differ |
| `bare_value.rs` | bare values wrapped with `Cooked::of` are content-keyed, usable as upstreams, and changing their content changes the downstream key |
| `cloud_proxy.rs` | the cloud proxy instrument: the hull is closed, the mesh operator is reproducible, the proxy encloses the coarse field, the final field gives a tighter proxy, and the measured gradient is below the bound |
| `nurbs_pipeline.rs` | NURBS curves and surfaces evaluate and tessellate **through the real loading path** (declaration → `PxOp::render` → dylib symbol) into watertight meshes; tessellation is reproducible; the GPU tessellation path also reaches a watertight mesh |

The expensive half of the instance mechanism (loading a *declared* instance library) is not a test:
it needs an artifact that `cargo test` must not build, so it lives in `--bin inst_probe`, which
fails loudly and prints the build command instead of skipping.

## The scene pipeline: recipe → scene document

`scene` is an ordinary graph, but its output contract is distinct: it does not cook graph nodes into
fields or meshes, it compiles `art/scene/<name>.toml` into one low-level frame-graph document. The
semantics live in `px-scene`; the binary only loads, keys, writes, and registers.

1. `recipe::load(name)` reads `art/scene/<name>.toml` (`recipe_path`), where `name` is the single
   positional argument, default `orbit`.
2. `recipe::compile(&file, &mut baked, with_graph)` assembles the `SceneSpec`:
   - `[[parts]]` become objects: geometry (a member mesh artifact, or the built-in `icosphere` when
     `primitive = "icosphere"`), materials (a shader member plus `params` and texture bindings),
     and world-space transforms. Part `members` are references of the form `graph::node` or a bare
     node name resolved through the part's `graph`; `recipe::members` turns each name into a key by
     reading that graph's manifest (`px_graph::manifest_key_of`) — the compiler never guesses a
     path.
   - Things that must be generated (palette textures, coverage cube maps, star fields, rings) are
     baked into the CAS and registered in the **`generated` graph**
     (`target/pcg/generated/manifest.json`); the document refers to them as `generated::<node>`.
     Only textures that the chosen surface shader's contract actually consumes are baked.
   - Cameras (`cameras = "review"`), lights, environment, and — unless `--no-frame-graph` — the
     frame graph sections (passes, intermediate targets, frame-owned materials, material
     instances) are written into the document. The frame comes from `art/frame/<frame>.toml`
     (`frame` in the recipe, default `default`).
3. `baked.finish()` merges the newly baked generated artifacts into the `generated` manifest.
4. The key is `px_cook::scene_key(spec_json, member_keys)`, and the artifact path is
   `px_protocol::scene::cas_path(cache_root, hex(key))` —
   `target/pcg/ab/<2 hex>/<64 hex>.pxart`.
5. `px_protocol::scene::write_scene` writes the bytes and returns their length. The program prints
   the document audit, then
   `产物 scene -> <path>（内容键 <12 hex>，不是文件字节的 sha256）`. The number in that line is the
   **content key**, not a hash of the file bytes; the retired escape-hatch verdict is about the
   bytes, so comparing that value against a registered digest will report a false mismatch.
6. An entry `{ node: <document name>, op: "scene.document", op_version: SCENE_SCHEMA (= 3), key,
   hit: false, millis: 0, bytes, detail: "<n> objects" }` is merged into
   `target/pcg/scene/manifest.json`, replacing any earlier entry with the same node name and
   keeping the rest sorted — several scene documents coexist on one machine.

The single flag is `--no-frame-graph`, a compatibility escape hatch whose only purpose is proving
that the six frozen old-shape documents can still be reproduced byte for byte. It is not a second
supported way to bake a scene, and `passes` refuses to run on that shape.

Render a document with
`px_render --offline --scene <artifact> --out x.png --width 960 --height 640`.
