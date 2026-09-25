# Executing a graph: `cached`, the CAS, and the driver

A graph is a **Rust binary**. There is no graph file, no runtime DAG, and no interpreter: a `main()`
calls operators in order, and one function — `cached` — sits between the call and the operator.

| Where | What it owns |
|---|---|
| `px_cook` | the door a graph program uses: `cached`, `node_params`, `apply_store_args` / `args_without_store`, and re-exports of the driver and the per-domain operator tables |
| `px_graph` | the driver (`begin` / `Graph` / `finish`), CAS paths, the manifest, the parameter index, the `shaders` graph, generated-asset writing |
| `px_graph_schema` | the contract: `Cache` (the only seam between driver and `cached`), `Key` / `node_key`, `PxOp`, `Cooked`, the payload wire format |

A graph program lives in `px_graphs/src/bin/<graph>.rs`, one bin per graph:

```rust
type Fault = Box<dyn std::error::Error>;

fn main() -> Result<(), Fault> {
    px_cook::apply_store_args()?;                     // must be the first statement
    let graph = begin(GraphSpec { name: "planet".to_string() });

    // `mountains` and `weight` come from two more `cached` calls of the same shape.
    let continents = cached(
        &graph, "continents", field::Fbm,
        field_params::fbm::Params { shape, ..node_params(&graph, "continents")? },
        (),
    )?;
    let terrain = cached(
        &graph, "terrain", elem::Mix,
        node_params(&graph, "terrain")?,
        elem::MixInput { a: continents, b: mountains, mask: weight },
    )?;
    let stats = terrain.value().stats();               // typed payload, no decoding on this side

    graph.finish();                                   // writes the manifest and the parameter index
    Ok(())
}
```

The graph name is the only thing `GraphSpec` carries, and it decides *where* things are written
(`art/<graph>/`, `target/pcg/<graph>/`). It is not part of any node key. Two graphs can run in one
process — `nebula` runs the graph `nebula` and then the graph `nebulasky`, each with its own
`begin`/`finish`, and their state cannot mix.

## `cached`

```rust
pub fn cached<O: PxOp>(
    cache: &dyn Cache,
    node: &str,
    f: O,
    params: O::Params,
    inputs: O::Inputs,
) -> Result<Cooked<O::Payload>, String>
```

`f` is a value (usually a unit struct such as `field::Fbm`); its *type* fixes the parameter type, the
input shape, the payload domain, the operator id, and the library the implementation is loaded from.
`Graph` implements `Cache`, so `&graph` is what gets passed.

What it does, in order:

1. `params_json = px_graph_schema::canonical_params(&params)` — the serialized parameter value,
   object keys sorted.
2. `cache.record_params(node, O::ID, &params_json, false)` — records the value that actually took
   effect (the `false` means "not straight from the parameter file").
3. `interface = O::interface()` — the interface shape hash, computed once and reused by the key, the
   report, and the manifest.
4. `source_hash = O::source_hash()?` — the implementation's source fingerprint, read **at run time
   from the operator library**. This happens *before* the cache lookup, so even a run that hits every
   node needs the libraries on disk and needs the contract/toolchain handshake to pass.
5. `key = node_key(&OpId { id: O::ID, interface, source_hash }, &params_json, |h| inputs.collect(h))`.
6. `cache.fetch(key)`:
   * `Some(payload)` → `Build::decode` → `cache.store(Report { hit: true, millis: 0, … }, &payload)`
     → `Cooked::new(key, value, true, 0, payload.bytes())`.
   * `None` → `f.render(&params, &inputs)?`, timed with `Instant` → `Build::encode` →
     `cache.store(Report { hit: false, millis, … }, &payload)` →
     `Cooked::new(key, value, false, millis, payload.bytes())`.

There is no second cache entry point, no `read_cache`, and no `.uncached()`. "Do not cache" is not a
flag — it is calling the operator's ordinary `render` yourself:

```rust
let raw = elem::Constant.render(&params, &())?;   // no key, no CAS, no manifest entry
let value = Cooked::of(raw)?;                     // …and now it can be an upstream
```

`Cooked::of` encodes the value, hashes the **encoded bytes** under the domain tag
`px_cook/cooked/v1` (length prefix + bytes) and returns `hit = false, millis = 0, bytes = 0`. Its key
names no artifact: the bare value was never written to the CAS, and nothing ever looks it up. What
matters is that the key is a pure function of the content, so "wrapped by hand" and "read from the
cache" are interchangeable as far as a downstream node's key is concerned.

### `Cooked<T>`

```rust
pub struct Cooked<P> { pub key: Key, value: P, pub hit: bool, pub millis: u64, pub bytes: usize }
```

`hit` says whether this run read it from the CAS. `millis` is the operator's own time (0 on a hit).
`bytes` is the **sum of the payload's blob lengths** — `payload.bytes()` on both paths — and is *not*
the size of the `.pxart` file; the manifest records the file size instead. `value()` is the typed
payload. A `Cooked<T>` is what an upstream field takes in an `Inputs` struct; when the same handle
feeds two nodes, clone it (`.clone()`).

## Parameters

Hyper-parameters are ordinary Rust values. The file `art/<graph>/<node>.toml` is their editable
form, read by `node_params`:

```rust
pub fn node_params<P: Serialize + DeserializeOwned + Default>(cache: &dyn Cache, node: &str)
    -> Result<P, String>
```

* It is **not** part of the caching machinery: it reads one file (`Cache::params_text`, i.e.
  `art/<graph>/<node>.toml`) and parses one TOML document.
* A **missing file is not an error** — the value is `P::default()`. So is any other read failure:
  `Graph::params_text` maps every `io::Error` to `None`, and an unreadable or non-UTF-8 file is
  therefore indistinguishable from an absent one and silently becomes the default.
* A **missing field** is not an error either: parameter structs carry `#[serde(default)]`, so absent
  fields keep their default values.
* An **unknown field is an error**: parameter structs carry `#[serde(deny_unknown_fields)]`, so a
  typo in a TOML key fails the run with `参数解不开：…` instead of being ignored.
* Because the key is computed from the *parsed value*, comments, spacing, field order, and
  "written out explicitly vs. left at its default" do not change anything.
* Each call records the node in the parameter index, flagged with whether the file existed;
  `cached` afterwards records the effective value with that flag unset, and the driver **merges** the
  two records (`from_file` only ever goes false → true, `op` is filled once). So for a node that goes
  through `cached`, `params.json` holds the values the operator actually received, even when the
  script overrode a field after reading the file.
* Missing files are reported only at the end: the `finish()` line lists the nodes with no parameter
  file, and those nodes' values are whatever the script computed. Changing them means recompiling
  the graph program.

The script may override anything it likes after reading the file, which is the normal style:

```rust
field_params::fbm::Params { shape, ..node_params(&graph, "continents")? }
```

## The key

`node_key` is blake3 over exactly these bytes, in this order (domain tag `px_cook/v1` first):

```
op.id  ‖  interface (u64, 8 bytes LE)  ‖  source_hash  ‖  canonical params JSON  ‖  upstream keys
```

| Axis | Identity or content | Where it comes from |
|---|---|---|
| `op.id` | identity | the operator's human id string (`field.fbm`); for an element function it is the spec's name (`field.constant`, `field.mix`, `field.remap`, `field.fuse`) |
| `interface` | identity | `interface_hash([type_name::<Params>(), type_name::<Inputs>(), type_name::<Payload>()])` — a blake3 over the tag `px_cook/interface/v1` and the three length-prefixed type names, first 8 bytes as `u64`. Changing a field of the parameter struct changes the type name, so it changes the hash and the key; changing only the algorithm body does not |
| `source_hash` | identity | the implementation's fingerprint, read at run time from the library's identity symbol. For `px_*_op` this is the dylib's `PX_SOURCE_HASH`; for `px_local_op!` it is the graph crate's own `PX_SOURCE_HASH`; for element and recorded instances it is the **instance content key**, which covers the declaration fingerprint, the toolchain hash, the algorithm roots' roster, the interface, the normalised body template, and the body file bytes |
| canonical params JSON | content | `serde_json::to_value` of the parameter value, object keys sorted recursively, serialized compactly. Arrays keep their order; `f32` fields widen to JSON doubles, so an `f32` parameter is hashed as its exact widened value |
| upstream keys | content | `Inputs::collect`: for each named field, the field-name bytes followed by that upstream's 32-byte key. `()` contributes nothing |

Editing a parameter file moves the parameter axis, and through it every downstream node's `upstream
keys` axis. The three identity axes need a code change, a rebuild, or both.

Deliberately **not** in a node key: the node's name, the graph's name, the graph script's source text,
the parameter directory, any environment variable, the artifact's own bytes, the toolchain, and the
camera. The first two are directory/naming conventions for the driver, not properties of a
computation; the parameter directory is a statement about *where the bytes were read from*, not about
what they are — the same file copied elsewhere is the same key. Two qualifications keep the
"no graph text, no toolchain" rule honest: for an operator whose implementation lives in the graph
program (`px_local_op!`), `source_hash` *is* the graph crate's fingerprint, so there the graph's text
enters by the ordinary identity axis; and an instance content key folds in the toolchain hash, which
therefore reaches a node key through `source_hash` for instance-backed operators. The consequence of
omitting the node name is worth stating plainly: two nodes with the same operator, parameters and
upstreams share one key, one artifact and one CAS slot no matter what they are called.

`Key` is `[u8; 32]`. `hex` renders 64 lowercase hex characters, `hex_short` the first 12.

## The CAS

The CAS root is `<workspace root>/target/pcg`, and it is a **compile-time** constant, not a setting:
`workspace_root()` is the parent of `px_graph`'s own `CARGO_MANIFEST_DIR`, so every path in this
document is resolved against the checkout that compiled `px_graph`, never against the current
directory. There is no environment variable that moves it.

A key becomes a path through `px_protocol::scene::cas_path`, which is shared with the renderer:

```
target/pcg/ab/<first two hex characters>/<all 64 hex characters>.pxart
```

`ab` is a literal directory name; the check is that the key is exactly 64 ASCII hex digits. The
`shaders` graph and generated assets write through the same function.

A `.pxart` is one stream (`px_protocol::stream`):

```
"PXST" (4 bytes)  ‖  u32 LE stream version (= 1)
frame *          where each frame = u32 LE payload length ‖ payload
```

A frame payload starts with `J` (JSON text: the `Art` manifest frame, or a `Scene` document) or `B`
(binary blob: JSON `BlobHeader` line, `\n`, then the raw bytes). A node artifact is one `Art` frame
carrying exactly one asset — `id` (the node name), the `params` map, one `BlobHeader` per blob, and an
FNV-1a fingerprint over the node name and the blob bytes — followed by one `Blob` frame per blob.
The `id` is written at store time and is *not* read back: `PayloadBundle::from_bytes` takes the
params and the blobs and ignores the name, which is why one key can legitimately be shared by
differently named nodes. The artifact's bytes never feed the key; only the payload that went into
them does.

Hit and miss:

* A hit decodes the payload and appends a manifest entry with `hit = true, millis = 0`. It must not
  write: the artifact is already on disk and the key is a hash of the payload, so re-encoding and
  rewriting identical bytes is pure IO. The on-disk `id` stays whatever node first cooked the key;
  nothing reads that id (`Build::decode` takes the caller's node name), which is what makes a
  first-writer-wins artifact safe to share. Nothing is deleted or evicted on this path.
* A miss calls the operator, encodes the payload, writes the file (creating
  `ab/<xx>/` as needed), prints the reading, and appends a manifest entry.
* An artifact that is present but cannot be decoded is **not** trusted: the driver prints a warning
  naming the key, treats it as a miss, and recomputes.
* Payload shapes are self-checked at encode time (`Blob::new` refuses a byte count that does not
  match `dtype × shape`), because a self-contradictory payload fails as a permanent miss with no
  error, which is a very quiet way to lose.

Generated assets take a different, simpler route (`px_graph::generate::store`): `write_texture` /
`write_generated_mesh` build the same kind of stream, hash **the whole artifact byte string** with
blake3 for the key, and if the path already exists they set `hit = true` and rewrite nothing. They
append no manifest entry — the caller registers them (`px-scene`'s `Baked::finish` writes
`target/pcg/generated/manifest.json`).

## What a run writes

Per graph, under `target/pcg/<graph>/`:

**`manifest.json`** — a JSON array of this run's nodes, in the order `cached` was called. Each entry:

| Field | Meaning |
|---|---|
| `node` | the node name passed to `cached` / `node_params` |
| `op` | `PxOp::ID` |
| `op_version` | the full interface hash, for display and cross-checking only |
| `key` | the node key, 64 hex characters |
| `hit` | whether this run read it from the CAS |
| `millis` | the operator's own time; 0 on a hit |
| `bytes` | the length of the `.pxart` file |
| `detail` | a one-line description of the payload, produced by the domain's `Build::detail` |

Hits are recorded too, and that is load-bearing: the manifest is *the* name → key index for
downstream consumers. `graph_manifest(graph)` reads it and `manifest_key_of(graph, node)` looks one
node up; the `scene` graph resolves `"<graph>::<node>"` references this way. A run that never wrote a
manifest leaves artifacts in the CAS with no names attached to them, and the scene side reports
`图 'shaders' 的清单读不到` / `图 'x' 里没有节点 'y'` (with the list of nodes it does have).

`finish()` **overwrites** `target/pcg/<graph>/manifest.json` with this run's entries. A graph that
wants several runs' entries to coexist merges them itself — the `scene` graph reads the old manifest,
replaces the entry with the same node name, sorts by node name, and writes it back.

**`params.json`** — the parameter index, a JSON object keyed by node name:

```json
{ "<node>": { "op": "field.fbm", "from_file": true, "params": { … } } }
```

`params` is the canonical parameter value as recorded by `cached` (so `f32` fields appear widened),
and `from_file` says whether a parameter file supplied the starting values. Node names are sorted
(the map is a `BTreeMap`).

On stdout, a run prints: the `begin` header (graph name, parameter directory and whether it exists,
whether a manifest for this graph already exists, and whether this run forces recomputation); one
line per node — `命中`/`重算`, node, operator, the first 8 hex characters of the interface hash, the
first 12 of the key, milliseconds, file bytes, detail; then the parameter-index line, the artifact
paths sorted, and a summary of hits / recomputations / total milliseconds with the manifest path.
Every one of those lines prints on an all-hit run as well.

## Lifecycle, roots, environment

```rust
pub fn begin(spec: GraphSpec) -> Graph           // GraphSpec { name: String } — nothing else
pub fn finish(&self)                              // writes params.json and manifest.json, prints the readings
pub fn params_text(&self, name: &str) -> Option<String>
```

`begin` returns a handle; it puts nothing in a global. It computes the parameter directory
(`param_root()/<graph>`), fixes the CAS root, reads `PX_PCG_FRESH`, creates the CAS root, and prints
its header. It does not read the existing manifest — a run always starts from an empty manifest and a
cache that is whatever is on disk.

`Graph` is the `Cache` implementation and exposes exactly the four trait methods plus `params_text`
and `finish`. `finish` takes `&self` and returns `()`: the writes are best-effort (a failure is
printed to stderr), and the reading lines are the point of the call.

Environment variables the driver honours:

* **`PX_ART`** — the parameter-directory root. Unset or empty means `<workspace root>/art`; a relative
  value is resolved against the workspace root (never the current directory); an absolute value is
  used as is. It is read when `begin` runs, so it must be set before any `begin` / `node_params`. It
  is a *path*, and paths are not part of a key: pointing at another directory re-reads the same bytes
  without invalidating anything. The parameter panel uses this: it copies `art/<graph>/` to
  `target/pcg/edit/<graph>/`, edits the copy, and runs `px run <graph> --store target/pcg/edit`.
* **`PX_PCG_FRESH`** — set to any value other than `0` (the empty string counts), it makes `fetch`
  return `None` for every key: everything is recomputed and rewritten. Unset is the normal,
  cache-reading mode. Its value never enters a key.

`apply_store_args()` is the graph program's first statement: it extracts `--store <dir>` or
`--store=<dir>` from `std::env::args()`, sets `PX_ART`, and errors on a missing or empty value. It
also **removes** the flag from the argument list, which matters because some graph programs read
their own arguments positionally; those must use `args_without_store()` rather than
`std::env::args()`, or `scene --store X orbit` would look for a recipe called `--store`. The store
directory is not part of `GraphSpec`, precisely so that "which directory was read" cannot be mistaken
for part of a graph's identity.

The operator libraries themselves are found by `px_graph_schema::ops::library_path`, not by the
driver: `PX_OP_DIR` first, then the directory of the running executable and its parent, then the
current directory.

## Failure behaviour

Loud, before or instead of any computation:

* **A parameter file that does not parse**, including an unknown field name — `node_params` returns
  an error and the graph program's `?` ends the run.
* **A missing operator library**, or one that fails the handshake with the graph program (different
  contract sources, different toolchain). This fails even on an all-hit run, because the source
  fingerprint is read from the library before the cache is consulted. The error names the library and
  the command to build it.
* **A payload that fails to decode on a hit** (the domain's `Build::decode`).
* **A failed artifact write** — `store` returns an error rather than continuing.
* **`--store` with no value, or an empty value.**
* **Reading a manifest** that is missing, unparseable, or empty, or looking up a node that is not in
  it (the error lists the nodes it does have).

Quiet, by design, but visible in the audit lines:

* A missing (or unreadable) parameter file falls back to `Default`. The only trace is
  `from_file: false` in `params.json` and the parameter-index line of `finish()`. That is why that
  line exists.
* A present-but-undecodable artifact is treated as a miss and recomputed, with one stderr warning.

And one asymmetry to know: `px run <graph>` is read-only by default. If a needed instance library is
missing it stops and tells you to rebuild instead of compiling anything; `--build` is the explicit
request. The node-caching driver never invokes cargo — that happens only in `px build` and
`px run --build`.

A graph binary run bare (not through `px run`) holds the same line by calling `insts::gate(graph)`
at startup: it refuses a graph whose instance libraries are not all on disk before the first node
cooks, names the missing keys, and prints the same `px build` command. The gate lives on the
driver / graph-exe side only — implementation libraries must not reach `px_cook`, or the gate
itself stops being free to edit (docs/invariants.md). Two tiers: the instance tier checks the
artifact file; "present but compiled against a different contract" is *not* caught at startup yet
— per-node `cached` still walks the loader handshake and refuses it, and a startup handshake needs
a loader entry point, so that half waits for the loader window. A caller may also pass named
libraries; each is checked through `source_hash`, which is the full `open()` handshake, so the
memoized entry the gate builds is reused by the first cooking node. The list must be exactly the
libraries this graph can reach: the gate refuses everything on it, so an unused-but-listed
library would turn "the run can start" into a false failure, while an unlisted one falls back to
today's mid-run refusal — named lists per graph exe are derived from the operators the binary
actually `cached`s, and the schema declarations they point at stay authoritative.

## Deliberately absent

* **No global graph state.** `begin` hands back a handle, so two graphs — or two tests running in
  parallel — cannot read each other's parameters.
* **No canvas.** `GraphSpec` has no width, height, or projection. Size and projection are
  parameters of the operators that produce a field (`field_params::Shape`), so "does changing the
  size recompute this node?" is answered by that node's parameter type: the ones with a `shape` field
  change key, and volumes, meshes, textures and NURBS do not.
* **No camera in a node's payload, in `GraphSpec`, or in a node key.** Cameras are ordinary data in a
  scene recipe and they are about *how to look*, not about *what a node computes*. (A scene document
  is a different kind of artifact and does carry its own camera list.)
* **No graph version.** There is no axis that exists only to represent "the graph changed"; the graph
  script's text reaches a key solely through `source_hash`, and only for operators implemented in the
  graph program. Editing an unrelated line of a graph that uses only declared operators does not
  invalidate a single artifact.
* **No runtime environment in a key**, and no key derived from a path, a timestamp, a pid, or a host
  name.
* **No eviction, no GC, no dirty flag on the node path.** "Dirty" is simply "the key is not on disk";
  nothing is deleted automatically, and files under `target/` may be removed at any time without
  changing any result.
* **No resume.** `finish` writes the manifest from scratch each run; there is no incremental graph
  state to reload.

## Graphs that do not go through `cached`

Besides the generated-asset writers above, two producers write into the same CAS on their own terms
rather than through `cached`:

* `bake_shader_graph()` (the `shaders` graph) scans `art/shaders/*.wgsl`, skips every file that has a
  `#define_import_path` (that marks a module, not an entry), and for each entry writes an artifact
  whose key is `shader_key(text, closure)` — blake3 over the tag `px_shader/v2`, `SHADER_VERSION`
  (= 1), the include-closure fingerprint, and the WGSL bytes. Every entry it writes to
  `target/pcg/shaders/manifest.json` has `hit = false` and `millis = 0`; it knows nothing about the
  previous run.
* The `scene` graph (`px_graphs/src/bin/scene.rs`) begins a graph but caches no nodes. It compiles
  `art/scene/<recipe>.toml` into one document, keys it with
  `scene_key(spec_json, member_keys)` — blake3 over `px_scene/v1`, `SCENE_SCHEMA` as `u32` LE, the
  document JSON, and each member key's hex text — writes the document, and merges its own entry into
  the scene manifest by node name.

## What the tests pin

| Gate | What it enforces |
|---|---|
| `px_graph/tests/keys.rs` | the same values give the same key however they were written (omitted field = default, comments and spacing are irrelevant); the interface hash, the implementation source fingerprint, an upstream, and a shape parameter each change the key; an unknown parameter name is rejected; a filter node has no `shape` axis to carry into its key; a shader key follows its include closure. It calls the *same* `node_key` that `cached` calls — there is no second key implementation to drift |
| `px_graph/tests/source_hash.rs` | the fingerprint mechanism itself: every fingerprinted crate's `build.rs` calls the shared `px_fingerprint` entry point, there is no `include!`, no hand-listed `include_str!` source list, no hand-written `VERSION`, and every implementation library exports `px_impl_lib!` |
| `px_graphs/tests/bare_value.rs` | a `Cooked::of` value is content-keyed (same content ⇒ same key, different content ⇒ different key), really works as an upstream, and makes the downstream node hit on a second run |
| `px_graphs/tests/local_op.rs` | a graph-side operator computes, stores, hits on the second run, takes the graph crate's `PX_SOURCE_HASH` as its identity, and differs from another such operator because the id is in the key |
| `px_graphs/tests/ops_load.rs` | every declared operator's symbol is actually loadable from its library, handshake included — the declaration ↔ implementation link is two strings, and the compiler cannot check it |
| `px_graphs/tests/crate_graph.rs` | the layering: graph programs depend on no `px_*_op` and no `px_*_alg`, implementation libraries link neither `px_graph` nor `px_cook`, `px_graph` links no operator, and the schemas link no operator |

See [operators.md](operators.md) for how an operator is declared and loaded, and
[programs.md](programs.md) for the `px` driver's verbs.
