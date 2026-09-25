# Backlog

Unresolved questions and known work. The rest of `docs/` describes current behaviour; this file is
the only place that does not.

Confirmed defects and stale artefacts live in [`FINDINGS.md`](../FINDINGS.md) at the repository root,
with a reproduction or a source trace for each. An entry belongs here only while nothing about it is
true yet: once it is confirmed broken, it moves there.

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
separately, which would let a change dirt one face instead of the artifact. The shape would be an
optional slot index on `Cooked` plus a partition rule each domain's `Build` declares; payloads that
declare no partition keep using slot 0 and stay byte-for-byte what they are today. This is separate
from parallelism, which already exists (below).

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

**The hot element path is serial while two cold paths are parallel.** `px_field_schema::parallel::rows`
partitions a field into row bands over `available_parallelism()` scoped threads, each writing a
disjoint slice, and is used by exactly two operators: `field.fbm3` and `field.ridged3`. Every element
operator runs through the single nested loop in `px_elem::fill`, which is serial. So the operators an
editing loop touches most are the ones that do not use the machinery that exists. Routing `fill`
through `rows` is a wall-time change only if it stays bit-identical to the serial result, which is the
gate `rows` was written to and must be tested against before it is adopted more widely.

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
