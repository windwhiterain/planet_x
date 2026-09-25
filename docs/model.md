# The system model

One run of this engine goes through four stages. This document describes what happens at each,
and which crate owns it.

## 1. A graph program is a Rust binary

There is no runtime DAG, no graph file, and no interpreter. A graph *is* a `main()` that calls
operators in order, with caching in between:

```rust
let clusters  = cached(&graph, "clusters",  field::Fbm,      node_params(&graph, "clusters")?,  ())?;
let mountains = cached(&graph, "mountains", field::Ridged,   node_params(&graph, "mountains")?, ())?;
let carved    = cached(&graph, "carved",    field::Warp,     node_params(&graph, "carved")?,
                       field::FieldPairInput { field: clusters, offset: mountains })?;
```

`cached` is the only cache entry point in the whole engine:

```rust
cached<O: PxOp>(cache: &dyn Cache, node: &str, f: O, params: O::Params, inputs: O::Inputs)
    -> Result<Cooked<O::Payload>, String>
```

It reads the node's key, returns a hit if the artifact is already in the CAS, and otherwise calls
the operator and records the result. The operator `f` is a value — usually a unit struct like
`field::Fbm` — and its type carries everything else: input shape, output payload, parameter type,
and the string id that goes into the key.

Three things are therefore enforced by the **compiler**, not by a schema check at runtime:

| | Enforced by | What a mistake looks like |
|---|---|---|
| input count and shape | `O::Inputs` (an associated type) | `expected MixInput, found …` |
| output domain | `O::Payload` | feeding a volume to something that wants a field |
| parameter type | `O::Params` (a Rust value, not parsed TOML) | a wrong field name is a compile error |

Each node also has a parameter file, `art/<graph>/<node>.toml`, read by `node_params`. Parameters
are ordinary Rust values; the TOML is the editable form of them.

## 2. Keys: identity plus content

Every node and every compiled instance is identified by a key derived from content. Both keys mix
the same two kinds of ingredient, but they are **not the same key** and they do not have the same
axes.

A **node key** (what `cached` computes for a graph node) is:

```
px_graph_schema::node_key(OpId { id, interface, source_hash }, params_json, inputs)
```

where `source_hash` is the implementation's fingerprint, and where it comes from depends on where the
implementation lives: a `px_*_op` preset reads it at runtime from its dylib's identity symbol, a
`px_local_op!` takes the graph crate's own fingerprint at compile time and loads no library at all, and
an element or recorded instance reads the **instance content key** from its instance library (the
declaration fingerprint, the toolchain hash, each algorithm root's roster, the interface, the normalised
body template and the body bytes). So a node key covers the operator id, the interface shape, the
implementation's source, the parameters canonicalised to JSON, and whatever the inputs contribute
through their own keys.

⚠ The **toolchain hash has no axis of its own in a node key**, but it is not absent from one either: for
a node backed by an instance it sits inside `source_hash`, because that is the instance content key,
which folds the toolchain in. Neither the node name, the graph name, the parameter directory, the
environment nor the artifact bytes are in a node key — with the one further qualification that a
`px_local_op!` node's `source_hash` is its graph crate's fingerprint.

An **instance key** (what names a compiled dylib) is a different set, and two things a node key has
are deliberately absent from it:

```
px_inst/v1  ‖  toolchain hash  ‖  decl_hash  ‖  each root's roster hash  ‖  interface
            ‖  normalised body template  ‖  source file bytes
```

- **no operator id** — the id is a label for readings and error messages. The specification's name
  lives in its own crate and is shared, so it is not per-instance identity. Leaving it out is what
  makes two graphs that use the same specification land on the same key, library, and artifact.
- **no graph-program source fingerprint** — the body and its dependencies are inside the dylib, and
  the other axes already cover them. Editing a graph program must not rotate an instance key.

`px_fingerprint` computes a source fingerprint by hashing the **full text** of the Rust files a
crate's module tree declares, every `.wgsl` under its `src/`, its `build.rs`, and the same for every
reachable runtime or build path dependency (a dev-dependency is not compiled into the artifact, so
it is not folded in). That has a consequence worth internalizing: **editing a comment changes the
key.** The key is an identity, not a summary of behaviour.

The whole text means the whole text: a `#[cfg(test)]` module inside a file that is in a roster is part
of that file's bytes, so **editing a test module in `src/` rotates the key** even though the module is
never compiled into the artifact. The exclusion the rosters make is by *path*, not by `cfg`: a file
under a crate-root `tests/` is not walked, and a `mod tests` inside `src/` is. Measured:
`px_volume_gpu_op`'s reconciliation fixtures live in `src/lib.rs`, and giving one of their calls an
`.expect(…)` rotated that library's fingerprint.

Because the source fingerprint is read from the *loaded library* rather than baked into the graph
program, a rebuilt implementation is visible without recompiling the graph — which is what makes
"changed the implementation but hit the old artifact" impossible **once the library has been rebuilt**. A
library whose sources are newer than it is only warned about (`px_graph_schema::ops`' staleness check on
load), never refused, so until it is rebuilt a stale implementation answers for its key.

Artifacts live in a content-addressed CAS under `target/pcg/ab/<2 hex>/<full hash>.pxart`.

## 3. Two stages: code, then data

Generic instances are *code*. You cannot produce them from a running binary, so the work is split
into two stages with two already-compiled binaries:

```
stage 1  (px build)
    px_graphs/src/inst_recipe.rs   a data table: op id / declaration / type name / roots /
                                   source file / body
        |
        v  px_graphs/build.rs validates the table and asks px_decls for type-level facts
        v  -> OUT_DIR/insts_gen.rs: `pub struct <T>;` + impl PxOp/InstNode
        v  px build generates one crate per instance and compiles it
    target/pcg/inst/<key>.dll      (flat; one library per instance key, plus a <key>.json sidecar)

stage 2  (a graph binary)
    the graph `include!`s the generated types, so it uses the types stage 1 produced,
    and loads the implementations from the dlls by key at runtime
```

The consequences are deliberate:

- The interface is fixed when the graph program is compiled; the implementation arrives later.
- Changing an operator body changes **one** instance key, so exactly one dylib is rebuilt and no
  graph binary is recompiled. This is the property the whole design exists to protect.
- `cargo build` never invokes cargo and never compiles instances. Only `px build` and
  `px run --build` do.

The `px` driver owns the three verbs: `list` (plan), `build` (compile what is missing, optionally
garbage-collect), and `run` (both stages in order).

## 4. Rendering

`px_render` is a bare-wgpu host. It never parses a scene recipe and never generates anything; it
consumes `.pxart` artifacts. The scene recipe (`art/scene/*.toml`) is compiled by the `scene`
graph into a low-level scene document that carries geometry, materials, textures, lights, cameras,
and the pass table. A scene document embeds the keys of its members, so it changes when they do.

The host has three faces over the same renderer: an offline one-shot (`--offline --scene … --out
…`), a resident service, and a preview window with a parameter panel. The panel writes edits to a
session copy of the parameters and re-cooks through a subprocess — the host never writes to
`art/` behind your back.

## 5. What is deliberately absent

- **No runtime graph editing.** The topology is Rust code. Making it data would cost the
  compile-time type checking that catches most mistakes before anything runs.
- **No canvas.** There is no global resolution; every operator takes its shape as a parameter.
- **No CPU rendering path.** Volume marching exists once, in WGSL. A CPU implementation exists in
  `px_volume_alg` as a draft and an oracle for tests, but nothing renders through it.
