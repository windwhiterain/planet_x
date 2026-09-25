# Backlog

Unresolved questions and known work. The rest of `docs/` describes current behaviour; this file is
the only place that does not.

Confirmed defects and stale artefacts live in [`FINDINGS.md`](../FINDINGS.md) at the repository root,
with a reproduction or a source trace for each. An entry belongs here only while nothing about it is
true yet: once it is confirmed broken, it moves there.

## Batch window discipline

**The toolchain gate guards the driver's entrances, not the loader.** `px run` refuses a plan whose
present instance libraries were compiled by another build (FINDINGS.md §13), and `px build` redoes
them. A graph executable started directly — double-clicked, or run from `target/debug/` — resolves its
instance libraries by `PROFILE` alone and loads whatever is there, so the gate can be stepped around by
not using the driver. Moving the check into `px_graph_schema`'s loading entry point would cover every
caller, at the cost of rotating the whole instance-key family, because that crate is in each root's
closure. Whether that price is worth paying depends on whether direct invocation is a path anyone uses
for a measurement; until it is, the driver entrances are where the answer is enforced.

**What a key-rotation batch window may and may not do.** The batch items only deliver "one
rotation, one rebuild, one re-cook" if the build environment stays frozen for the window:
one profile, one `TARGET`, one `RUSTFLAGS`, and no workspace rebuild by anyone (the toolchain axis
of an instance key rotates on it — docs/operators.md, instance-key section). The window's validity
is not a promise, it is a reading: a `px list` invocation before opening and before closing leaves
two key columns; they must be byte-identical, and the most recent run's `manifest.json` key column
(the **node-key** side, complementary to `px list`'s instance-library key column) is the second
half of that check. A window whose columns differ is void —
none of its numbers enters a ledger.

## Awaiting a decision

**Where does an idea go before it is true?** `docs/` states only what holds today. An idea that
cannot yet be written as a present-tense fact has no home other than this file.

**Which side owns the resident cook process.** Every graph run pays for a GPU device
(`px_gpu::connect`, a `OnceLock<&'static Gpu>`), re-opens every operator library
(`px_graph_schema::ops::open`, whose table is a leaked `OnceLock<Mutex<HashMap>>`), and — when the
panel edits a parameter — spawns a subprocess that may compile an instance dylib before the parent
reads the artifact back. The render side already has a long-lived service (`px_render --serve` with a
lease file); the cook side has no counterpart. Adding one needs a second `Cache` implementation, and
that forces a choice the crate graph currently settles the other way: `px_render`'s dependencies are
`px_protocol`, `px_shader` and `px_pass` only, so the host holds no key primitives, no `Cache` trait
and no payload decoders, which is what keeps "the host never generates anything" true. Either the
service lives on the cook side (host unchanged, device cost paid by the service) or the contract layer
moves into the host. Nothing is decided until someone measures what the process boundaries actually
cost.

**Whether to erase operator types.** The declaration surface is 32 preset operators (field 10,
volume 5, mesh 2, nurbs 15) over a handful of `Body` shapes (inputs times payload domain), plus 4
element specs sharing one cell-closure shape, 3 instance recipes whose bodies are already template
strings, and zero shipped `px_local_op!` users. A per-declaration shim table keeps static checking
but rotates every implementation roster the moment the `px_op!` / `px_body!` expansion changes; a
`&dyn` body erases the interface (today a hash of three type names), rotating every node and
instance key and deleting the compiler's input, output and parameter enforcement; skipping costs
nothing today, because no recurring cost of static dispatch has been measured. The project starts if
and only if someone measures such a cost larger than one full rotation plus re-bake plus rewriting
about five gates.

## Known costs, not defects

Each of these is how the engine behaves today and why it costs what it costs. None is a bug.

**How fine-grained invalidation should get.** `Graph::fetch` answers "is this exact key on disk", so a
node is either whole-and-cached or whole-and-recomputed. Cube maps, volumes and textures have natural
divisions (6 faces, face x layer blocks, mip levels) that could each be cached and computed separately,
which would let a change dirty one face instead of the artifact. A slot is its own key and its own
artifact, so partial results depend on a partition rule each domain's `Build` declares — not on the
existing row-band machinery (below), which changes wall time without changing what is stored.

**The cost of a partition is decided by where its rule lives, not by whether a slot index also enters
the key.** Every payload's `Build` implementation sits either in the contract crate or in a domain
crate, and both are inside rosters:

| payload | `Build` lives in | so a partition rule change rotates |
|---|---|---|
| `VolumeData`, `TextureData`, `MeshData`, `PolylineData` | `px_graph_schema/src/build.rs` | every node key and every instance key (the contract crate is in every roster) |
| `Field` | `px_field_schema/src/payload.rs` | every instance key (that crate is in each instance root's closure) and every field node key |
| `Curve`, `Surface`, `PointData` | `px_nurbs_schema/src/payload.rs` | the nurbs family |
| `StarField` | `px_sparse/src/stars.rs` | the volume family |

Measured, and it is the sharpest form of the rule: **a single comment line added to
`px_field_schema/src/payload.rs` rotated all seven planned instance keys** (`1d2c7973609a` →
`7b9be8384554`, `e59875d8f0f3` → `b8f42ee44cad`, …), declaration hashes included, and the same edit was
visible to an `px list` binary that predated it — the roster is walked at run time
(`px_cook/src/inst.rs::key`), so no rebuild is needed to see a key move.

The one genuinely zero-key shape is the reader: **the render host now consumes slots.** `px_render`'s
read path used to take `assets.first()` and the first blob, which made a partitioned artifact
unreadable rather than wrong; it now pairs each `AssetManifest` with its own blobs positionally,
validates every slot, and reports each slot's `id`, shape and fingerprint, with a gate that refuses
damage in a slot past the first. `px_render` is in no roster, no instance closure, and no dependency
list, so that step rotates nothing.

What remains undecided is whether any **producer** should partition, and the readings say the ceiling
is low: `clouds` spends 46,128 ms with 90.7% of it in two nodes, of which `coarse_fine` (25,007 ms,
24.76 MB, a face-major `6 x layers x res x res` volume) divides cleanly by six while `proxy_fine`
(16,852 ms, a 263k-vertex mesh) has no axis at all — its attribute blobs change together and a spatial
split would re-index. `sky.nebula` (48 MB) has one mip level, so splitting by mip buys nothing there.
So the producer side starts only if a face-level edit becomes a real workflow, or if a measured benefit
exceeds one full-family rotation plus re-bake; a slot kept inside a resident cook process stays
unattractive because that service is itself not being built (see the cost entry below).

**The fingerprint axis is crate-wide, so a signature edit rotates unrelated keys.** `interface_hash`
folds in `type_name::<Params>()`, so adding one field to a parameter struct changes that node's key —
correctly. What `source_hash` covers depends on where the implementation lives (graph.md's axis table): a
preset contributes its own dylib's fingerprint, a `px_local_op!` the graph crate's, and an element or
recorded instance the **instance content key** — which folds in that instance's `decl_hash` and the
algorithm roots' roster. A schema crate holds declarations, parameter structs and domain data together,
so one widened struct still rotates every operator that crate declares. In a repair loop that repeatedly
widens parameter structs, this reads as "fix one node, dirty the whole graph": the axis such an edit
belongs on is the instance's `decl_hash`/interface, and the crate-wide source fingerprint is what makes
it wider than that.

**Cost telemetry writes to a ledger that is not part of identity, and the first machine reader is
still to be written.** `Graph::finish()` appends one run to `target/pcg/<graph>/metrics.jsonl`: a
header line (`seq`, `graph`, `started`, `node_count`) followed by one line per node (`seq`, `node`,
`key`, `hit`, `cook_millis`, `bytes`). `seq` counts runs per graph and is read back from the most recent
line that carries one, so it orders the file even when two runs start in the same second; `started` is
the human-readable anchor only. The file rotates to `metrics.jsonl.1` past 256 KB or 4096 lines, and a
write that does not go through is reported rather than swallowed. Damage older than that most recent
readable line is outside what this routine reports: the scan stops at the first line that yields a `seq`,
and when none does the run is recorded as `seq` 1. Nothing under `target/` is in any roster, so recording
a measurement cannot invalidate what it measured. What is missing is the consumer: no command reads the
ledger yet, so a cost model is still a manual exercise over the JSONL.

**A resident cook service is not worth building today; the reading and the reason are below.** The
idea was a long-lived process holding the operator libraries and the `Graph`, so repeated cooks stop
paying per-process costs. Measured on one machine (RTX 3060 Laptop / Vulkan, `planet`, warm page
cache):

| reading | value |
|---|---|
| `planet`, first bake in a fresh process (cold page cache) | 5592 ms wall |
| `planet`, next three bakes | 673 / 683 / 685 ms wall |
| `planet` re-cooking every node (`PX_PCG_FRESH=1`, libraries invoked) | 3641 ms wall, of which **2943 ms is the nodes' own `cook_millis`** |
| all-hit run: process floor that a resident service could remove at best | ~680 ms |
| `px_probe cook_boundary`: 5 operator libraries, first open vs resident | 2638.7 ms vs **0.017 ms** |
| the same 5 libraries in a fresh child process | **4.9 ms** |
| `cook_boundary`: device cold / warm, spawn overhead | 4082 ms / ~0.000 ms / 47–54 ms |

Three things follow, and they point the same way. **The 2.6 s is page-cache cold, not a per-run
cost**: in a child process with the images already cached, the same five libraries load in 4.9 ms, so
consecutive bakes never paid it — the second `planet` bake is 673 ms, and the ledger shows every node
a hit with `cook_millis: 0`. **The floor a resident service could actually remove is ~680 ms of
process start**, and of that the part a service could hold warm is the device (~310–360 ms measured
per run, not the idle 4082 ms outlier), which only helps if execution also moves into the service.
**And execution cannot move**: `px_graph_schema::ops::Body<O>` is
`fn(&O::Params, &O::Inputs) -> Result<O::Payload, String>` and `px_body!` pins those three types, so a
server that runs operators must statically link every schema crate — the dependency direction the
library-loading design exists to avoid. The alternative is a typed-shim-per-declaration layer, i.e.
type erasure for the whole operator surface.

For the loop this was meant to serve, the shell is not the cost: a panel-shaped cycle timed by
polling the real process stages measures **128.5 s (render-dominated)**, while the cook-side shell is
0.68 s and cook itself is ~2.9 s. Writing the ledger (§cost telemetry, above) is the half that was
worth having; the service is the half that is not, until someone wants type erasure for its own sake.

**A scene document cannot point a material at content.** `ParamKind` covers `F32`, `I32`, `U32`,
`Vec3`, `Vec4`, and the matching `Value` covers a number, a string, a triple and a quad. There is no
texture slot, matrix or fixed-length array, so changing which texture a material samples means
re-cooking the scene document rather than pushing a parameter. Preview-versus-final quality therefore
cannot be expressed as a parameter either: resolution lives inside each producing operator's `Shape`
parameter, so switching precision rotates the keys of the whole upstream chain.

**Operator failures reach an agent as prose.** The cook path reports through `Result<_, String>` with
human-oriented messages, and some carry the fix command inside the text (`ops::hint`). Two
consequences: a caller cannot tell one failure kind from another without matching strings, and a panic
still crosses a dylib boundary uncaught where it is not wrapped — the parallel helper itself falls
back to `expect("行带线程不该 panic")` on a joined worker. A stable set of error codes alongside the
display text would let a caller branch on the failure instead of reading it.

**A node name is outside every key, and its history now lives in two files.** Node names are
deliberately outside every key, and `finish()` writes the manifest from this run alone, so the
name-to-key index covers only the last execution (which is why the `scene` graph merges its own entries
rather than relying on the driver). What is new is that a run is no longer described only by that index:
`finish()` also appends one line per node to `target/pcg/<graph>/metrics.jsonl`, carrying the node name,
its key, whether it was a hit, its milliseconds and its bytes, under a per-graph run number. Together the
two give the minimal surface for following one named node across runs — comparing two bakes of the same
design, or attributing a violation to the node that caused it. What does not exist is a reader that
treats them as one volume: the ledger is appended and never queried, so the comparison is still done by
hand over the JSONL.

**Which asset id a shared artifact should carry.** Several node names can
legitimately share one key and one CAS slot, and the first writer fixes the
`id` embedded in that artifact (a hit no longer re-writes it). No reader
consumes it — `PayloadBundle::from_bytes` recovers params and blobs only,
`Build::decode` takes the caller's node name, and the render host reads
`params`, blob headers and the fingerprint, which is opaque to it — so sharing
is safe today. What is undecided is whether the field should keep existing:
either give it meaning (which would need the bundle to carry it through decode)
or drop it from the manifest frame.

## Repair vocabulary for the executor's repair loop (designed, not built)

The repair loop assigned to the thin dirty set has two traps that come from the content-defined
cache itself, not from any scheduler:

* A fixed `fix_hint` replayed from the second round is a cache hit — the same action on the same
inputs produces the same key, so the loop "learns" nothing while everything looks self-consistent.
* A learned policy whose reward counts a replayed hit as success drifts toward exactly that
cheapest-looking path.

The shape that avoids both: every declared `repair_op` carries a `perturb` note (how to vary within
that knob's safe range so the next attempt hashes differently) and a `force_dirty` set of rule ids
that may borrow the approved thin dirty set to force a miss without touching inputs. A declaration
without either is not a replay candidate. The learned policy's reward counts only runs that reach
`f.render` as attempts; a replayed hit is neither a success nor a cost. Scope: `gate_ready` (the
startup gate) and the thin dirty set are separate homes — the vocabulary does not belong in either
file.

## Cache format with mutual references (designed, not built)

`Graph::fetch` answers "is this exact key on disk", so the invalidation unit is the whole node artifact.
A payload whose natural parts are faces, layers or mip levels could instead be cached as parts and
referenced from the parent, and then a change to one face re-cooks one face while the parent's own work
becomes assembly. None of this is a new mechanism: the frame already carries
`ArtBundle.assets` with `AssetManifest { id, params, blobs, fingerprint }` followed by positional
`Frame::Blob`s, and a scene document already references other artifacts by key through
`Member { graph, node, key }` and `cas_path`. The design moves that one reference rule inside an asset,
rather than inventing a second one.

**Frame shape.** One new field, `AssetManifest.sub: Vec<SubEntry>`, where a `SubEntry` is
`{ kind, key, fingerprint, blobs }`: `key` is the child's own 64-hex content key (resolved through
`cas_path`, the same rule `Member` uses), `fingerprint` is the child's own fingerprint so a consumer can
tell which child moved without reading it, and `blobs` names which of the asset's blobs belong to this
child. Blob ownership stays an index partition of the asset's one ordered `blobs` list, with a load-time
invariant that the children's index sets partition `0..blobs.len()` exactly — the alternative, giving
each `SubEntry` its own `Vec<BlobHeader>`, duplicates the ordering and breaks the positional pairing the
reader relies on. `kind` is a closed enum (`Face`, `Mip`, `Layer`, and a named form deferred to a later
milestone), because a free string would put the partition semantics back into prose. **The frame does
not nest:** a child appears only as a key and is an ordinary artifact in the CAS, which may itself carry
children, so the structure is a DAG reached through the cache, walked with a visited set and a depth cap
(a cycle is refused).

**Key axis.** A child is an ordinary node, so `child_key = H(op, interface, source_hash,
canonical_params including its partition selector, inputs)`, and the parent folds its children's keys
exactly as it folds upstream keys: `parent_key = H(op, interface, source_hash, canonical_params, inputs,
children sorted by (kind, key))`. Two consequences are worth stating. Folding child keys does not defeat
partial invalidation, because the expensive per-part work is each child's own cook, which still hits;
what the parent redoes is assembly. And child keys cannot be left out of the parent key: doing so would
let a fresh parent with stale children and a stale parent with fresh children share one key, which breaks
the rule that a key covers its output.

**What the parent keeps.** Keeping the merged payload beside the references doubles the bytes for the
case that matters (`coarse_fine` is 24.76 MB, so six 4.13 MB faces plus the merged blob is about 49 MB)
and still re-encodes the whole merged blob on a one-face change. The parent therefore holds references
only, and the merged form is assembled by the consumer — which is the ground the reader already stands
on.

**Store ordering.** Children are written first and the parent last. A crash then leaves orphaned
children, which are garbage; the reverse order can leave a parent that references children which do not
exist, which is a poisoned entry whose key is legitimate and whose load must fail. Garbage has a
collector and corruption does not. Note that `store` writes with a plain `std::fs::write` (no fsync, no
temporary file and rename), so a torn parent is possible today; closure verification turns that from
silent garbage into a refused load, which is a net improvement rather than a new risk.

**Closure verification and collection.** At load, every `SubEntry.key` must resolve and the child's
recorded fingerprint must match; a mismatch is refused by name rather than silently recomputed, because
recomputing would disguise a broken reference as an ordinary miss. The live set for collection is the
transitive closure of the manifest's keys through those references, and a parent that cannot be read
must have its children **kept** — the walk fails closed, never open. Today's `collect_garbage` sweeps
only `target/pcg/inst/` and, with `--deep`, `target/jit/`; it never sweeps the artifact CAS, so the
invariant is currently vacuous and becomes load-bearing the moment an artifact collector exists.

**The reader is already in place.** `px_render`'s read path pairs each `AssetManifest` with its own blobs
positionally, validates every slot, and reports each slot's id, shape and fingerprint, with a gate that
refuses damage in a slot past the first. The old path read `assets.first()` and the first blob, which
made a partitioned artifact unreadable rather than wrong. References add one step to that reader —
resolving each child through `cas_path` and validating it as its own slot — and cost no keys, since
`px_render` is in no roster, no instance closure and no dependency list.

**When this starts.** Three triggers, any one sufficient: a real per-face or per-layer editing workflow;
a consumer on the pane or edit side that diffs child fingerprints, which is closer now that the reader
exposes them; or readings in which the benefit exceeds one full-family rotation plus re-bake. The ceiling
those readings put on it: `clouds` spends 46,128 ms with 90.7% of it in two nodes, `coarse_fine`
(25,007 ms, 24.76 MB, face-major `6 x layers x res x res`) divides by six, `proxy_fine` (16,852 ms,
263k-vertex mesh) has no axis without re-indexing, and `sky.nebula` (48 MB) has one mip level, so
splitting by mip buys nothing there.

**Scope.** The first milestone is volume and texture by face, a references-only parent, per-child
consumption in the host, and a single `Face` kind. The second is nested closures, the `Mip`, `Layer` and
named kinds, and the closure-aware live set. The third is every domain: a field's natural partition is
row bands, which the parallel row helper already exploits to change wall time without changing what is
stored, so its benefit is the smallest; a mesh has no axis that does not re-index its vertices, so it may
never be worth doing. A contract field is what makes the first milestone a batch-window job: measured, a
single field added to `AssetManifest` rotates every instance key and every node key, because
`px_protocol` and `px_graph_schema` sit in every roster.

No per-face or per-layer editing workflow is known on any node kind today, so the format stays designed
and the three trigger conditions above carry the launch. When such a workflow, or a consumer that diffs
child fingerprints, appears, the first milestone's window can be opened following the shape above.

## Gates shipped

Three checks that used to be convention only now have a home in `tests/`, so none of them costs a
key rotation. All three are shipped and are described here as contracts rather than as pending work.

**No payload-producing source may read an input its key cannot see.** A payload that depends on an
environment variable, a process id, a wall clock or an unseeded RNG breaks "same key ⇒ same bytes"
silently: the second run overwrites the first in the CAS and the manifest stays self-consistent. The
rule is [invariants.md](invariants.md)'s; the check is
`px_fingerprint/tests/roster.rs::no_payload_source_reads_an_input_its_key_cannot_see`. Its scan
covers what production actually cooks repeatedly — `px_*_op/src`, `px_*_alg/src`,
**`px_elem/body/*.rs`**, **`art/inst/*.rs`** — because the instance bodies sit outside every
crate's `src/` (that placement is what makes "edit one body, rotate one key" work), so scanning
only `src/` would watch the least likely place to fail. The deny list is a list of literals, not a
phrase like "non-deterministic"; an exemption names one call site (file plus the literal argument)
and the mechanism that makes it safe. Two shapes the literal list deliberately does not reach, and
which therefore remain convention: **HashMap iteration order that reaches output** (it needs a
value-to-output trace, not a call site; today's `px_mesh_op` HashMaps only feed order-independent
values), and **reads of files outside the key** (a path's contents are not a call site either).

**An `include_str!`/`include_bytes!` target must be inside a roster or on a named exception list.**
The collector accepts `.rs` files by declaration and `.wgsl` by extension, so any other file type
reaches the binary without reaching identity. The check is
`px_fingerprint/tests/roster.rs::every_embedded_file_is_in_a_roster_or_on_the_exception_list`. One
case is exempt: `px_protocol/src/lib.rs` embeds `../snapshots/protocol.snapshot.json`. Its coverage
is what exempts it — the only consumer is `protocol_hash()`, which enters `ProtocolId` and is
compared value-by-value by `Handshake::verify`, so two different snapshots refuse to communicate
rather than silently exchanging wrong content (`.gitattributes` marking that path `-text` is only a
precondition, keeping the bytes stable so the hash means the same thing everywhere). The exemption
carries its own expiry as a check: **if the snapshot gains a second consumer — anything that derives
payloads, artifacts or keys from it — the gate fails and the file belongs in a roster.** The gate
also refuses an embed whose path does not exist, and refuses a `.rs` file it cannot read.

**The element parallel banding has a bit-exact gate at the junction, not only at the helper.**
`px_elem::fill` is the one loop in the element family and it bands rows through
`px_field_schema::parallel::rows`; `px_elem/tests/fill_bands.rs` drives it against the serial loop it
replaced and compares **bits**, including a `Volume` field, whose direction probe must happen once
rather than per cell. `px_field_schema/tests/row_bands.rs` still pins the helper itself and
`px_graphs/tests/elem.rs` the operator path. All three live in `tests/` and rotate no key.

## Unconsumed inputs

34 of the 44 scene recipes under `art/scene/` are referenced by no `.rs` or `.ps1` in the tree:

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
