# Backlog

Unresolved questions and known work. The rest of `docs/` describes current behaviour; this file is
the only place that does not.

Confirmed defects and stale artefacts live in [`FINDINGS.md`](../FINDINGS.md) at the repository root,
with a reproduction or a source trace for each. An entry belongs here only while nothing about it is
true yet: once it is confirmed broken, it moves there.

## Batch window discipline

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

**How fine-grained invalidation should get.** `Graph::fetch` answers "is this exact key on disk", so a
node is either whole-and-cached or whole-and-recomputed. Cube maps, volumes and textures have natural
divisions (6 faces, face x layer blocks, mip levels) that could each be cached and computed
separately, which would let a change dirt one face instead of the artifact. Two shapes, and they are
not the same cost: a slot index carried in the cache identity persists and is readable by the render
host, but both halves of that live in crates every operator links (`Cooked` in `px_graph_schema`, the
path rule and whole-payload decode in `px_protocol`), so it rotates every implementation library and
forces the contract handshake to rebuild them all; a slot kept only inside a resident cook process
touches no protocol at all, but dies with the service and stays invisible to the renderer. Either way
a slot is its own key and its own artifact, so partial results depend on a partition rule each
domain's `Build` declares — not on the existing row-band machinery (below), which changes wall time
without changing what is stored.

## Known costs, not defects

Each of these is how the engine behaves today and why it costs what it costs. None is a bug.

**The fingerprint axis is crate-wide, so a signature edit rotates unrelated keys.** `interface_hash`
folds in `type_name::<Params>()`, so adding one field to a parameter struct changes that node's key —
correctly. But `source_hash` is the whole declaring crate's source fingerprint, and a schema crate
holds the declarations, the parameter structs and the domain data together, so the same edit also
rotates every other operator that crate declares. In a repair loop that repeatedly widens parameter
structs, this reads as "fix one node, dirty the whole graph". Splitting the declaration surface from
the data surface, or deriving the interface hash from a structural layout summary instead of a type
name, would move each of those consequences onto its own axis.

**Cost telemetry has no home that is safe to write to.** Measured timings must not reach a node key,
or recording a measurement would invalidate the thing it measured. There is nowhere to put them today:
the manifest is overwritten by every `finish()`, so it survives only one run; `Art.params` is a
`BTreeMap<String, f64>`, so a number stored there is indistinguishable from a tuning parameter; and
the per-instance `.json` sidecar (`px_cook::inst::sidecar_text`) describes compiled code rather than
cooked output, and nothing reads it back. A cost model needs a store that outlives a run and is keyed
by node key without feeding it.

**The hot element path does not use the parallel machinery.** `px_field_schema::parallel::rows` bands a
field over scoped threads with each thread writing a disjoint slice; it is used by two operators
(`field.fbm3`, `field.ridged3`), while every element operator goes through the serial nested loop in
`px_elem::fill`. So the path an editing loop touches most uses neither. A bit-exact gate for that
banding now lives in `px_field_schema/tests/row_bands.rs` (`tests` dirs are outside every fingerprint
roster, so adding it rotated no key). Routing `fill` through `rows` is the remaining step, and it is
not free: `px_elem/build.rs` fingerprints all of `src/`, so that edit rotates the whole element
instance-key family and must be counted before it is made.

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

**A node has no identity that outlives a run.** Node names are deliberately outside every key, and
`finish()` writes the manifest from this run alone, so a name-to-key index exists only for the last
execution (which is why the `scene` graph merges its own entries rather than relying on the driver).
Anything that wants to follow one named node across runs — comparing two bakes of the same design, or
attributing a violation to the node that caused it — needs an identity ledger beside the keys, not
derived from them.

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

## Gates shipped, and the one still missing

Three checks that used to be convention only now have a home in `tests/`, so none of them costs a
key rotation. Two are shipped and are described here as contracts rather than as pending work.

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
`collect_sources` walks only `src/` and `build.rs`, and `collect_tree` accepts only `.rs`/`.wgsl`, so
embedded data reaches the binary without reaching identity. The check is
`px_fingerprint/tests/roster.rs::every_embedded_file_is_in_a_roster_or_on_the_exception_list`. One
case is exempt: `px_protocol/src/lib.rs` embeds `../snapshots/protocol.snapshot.json`. Its coverage
is what exempts it — the only consumer is `protocol_hash()`, which enters `ProtocolId` and is
compared value-by-value by `Handshake::verify`, so two different snapshots refuse to communicate
rather than silently exchanging wrong content (`.gitattributes` marking that path `-text` is only a
precondition, keeping the bytes stable so the hash means the same thing everywhere). The exemption
carries its own expiry as a check: **if the snapshot gains a second consumer — anything that derives
payloads, artifacts or keys from it — the gate fails and the file belongs in a roster.** The gate
also refuses an embed whose path does not exist, and refuses a `.rs` file it cannot read.

**The element parallel banding has no bit-exact gate for the helper production uses.** Shipped as
`px_field_schema/tests/row_bands.rs`; still open is routing `px_elem::fill` through it, which needs a
key count first (see Known costs above). Recorded here so the remaining half is not mistaken for
done.

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
