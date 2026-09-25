# Element-wise operators

A pointwise operator on a field — "multiply by a mask", "remap the range", "fill with a constant" —
is not worth a module, a declaration, a dylib and a key of its own. `px_elem` exists so that all of
them are **one** generic operator that monomorphises into any pointwise function, and can fuse a
chain of them into a single pass.

The authoring surface stays concrete: each function has its own parameter struct, so a wrong field
name or a missing upstream is a compile error rather than a runtime complaint. The implementation
is still compiled into a dylib and loaded at runtime, so changing one function's body rebuilds one
library and no graph binary.

## The pieces

```
px_elem/src/lib.rs        ElementFn (the author-side trait), ElemSpec/ElemFacts, the ELEM_SPECS
                          table, fill (the one loop), like (shape helper)
px_elem/src/specs.rs      the specification table — one line per function, the only list
px_elem/src/<fn>.rs       the authoring side: `struct <F>;` + params + input struct
px_elem/body/<fn>.rs      the actual per-cell arithmetic
px_graphs/src/elem.rs     the graph side: ElemOp<F> (the PxOp impl), content keying, loading
```

⚠ The split matters. `px_elem` deliberately does **not** know about the driver. If it did, the
driver chain would be linked into every instance library — measured at 15.0 MB per library versus
5.3 MB. So the `PxOp` implementation (`ElemOp<F>`) lives on the graph side, in `px_graphs`, and the
script-facing value is a **generated unit struct** (`elem::Constant`) produced by
`px_graphs/build.rs` into `OUT_DIR/elem_gen.rs`.

### `ElementFn`

```rust
pub trait ElementFn: 'static {
    type Params: Serialize + DeserializeOwned + Default + PxKeyed;
    type Inputs: PxInputs;
    const NAME: &'static str;                       // the operator id, e.g. "field.mix"
    const SOURCE: &'static str;                     // the body file
    const ROOTS: &'static [&'static str];           // crates the generated library links
    const BODY: &'static str;                       // the body template (key axis)
    const SYMBOL: &'static str;                     // how the body is exported
    fn shape(params: &Self::Params, inputs: &Self::Inputs) -> Shape;
}
```

`Params` and `Inputs` are associated types, so each function declares its own parameter struct and
its own upstream count. Fusion is expressed the same way: a fused function simply declares more
upstreams and does the whole chain inside one call.

`shape` receives both the parameters and the upstreams. Generator-style functions take their shape
from the parameters (`params.shape`); filter-style functions take it from an upstream
(`px_elem::like(inputs.<field>.value())`), which makes "the output has the same shape as its input"
true by construction rather than by convention.

### The specification table

One line per function, and the only place the set is enumerated:

```
Constant, ConstantParams, (),        "field.constant", source: "px_elem/body/constant.rs", roots: &[],                 shape: |p, _| p.shape;
Mix,      MixParams,      MixInput,  "field.mix",      source: "px_elem/body/mix.rs",      roots: &[],                 shape: |_, i| like(i.a.value());
Remap,    RemapParams,    RemapInput,"field.remap",    source: "px_elem/body/remap.rs",    roots: &["px_field_alg"],   shape: |_, i| like(i.field.value());
Fuse,     FuseParams,     FuseInput, "field.fuse",     source: "px_elem/body/fuse.rs",     roots: &["px_field_alg"],   shape: |_, i| like(i.a.value());
```

The columns are the graph-side type name, the parameter type, the input struct, the human-readable
operator id, and then the three facts the build needs: which file holds the body, which crates the
generated library must link, and how the shape is derived.

### Why the body files live outside `src/`

Cargo's incremental compilation works per crate. If the arithmetic lived in `px_elem/src/`, editing
one function would recompile `px_elem` — and every graph program that depends on it. Putting the
bodies in `px_elem/body/` takes them out of the crate's source fingerprint entirely, so editing a
body changes **only that instance's key** and rebuilds **only that instance's dylib**.

The body files are `include!`d into the generated library, so they use fully-qualified paths only,
carry no `#[cfg(test)]` code, and refer to nothing private to `px_elem`. Their comments must use
`//` and not `//!`: the generated file places them after the `use` lines, where an inner doc
comment is a compile error.

## The functions that exist

| Id | Type | Upstreams | Parameters (defaults) | Computes |
|---|---|---|---|---|
| `field.constant` | `Constant` | 0 | `shape` (`512×256`, `CubeMap`), `value` (0.5) | fills the whole field with one value |
| `field.mix` | `Mix` | 3 (`a`, `b`, `mask`) | `bias` (0.0) | `a·(1−w) + b·w` with `w = clamp(mask + bias, 0, 1)` |
| `field.remap` | `Remap` | 1 (`field`) | `in_min` (0.0), `in_max` (1.0), `out_min` (0.0), `out_max` (1.0), `smooth` (false), `gamma` (1.0) | normalises the input range, maps it to the output range, optionally smoothsteps, then applies gamma |
| `field.fuse` | `Fuse` | 3 (`a`, `b`, `mask`) | all `RemapParams` plus `bias` (0.0) | `remap` followed by `mix` in a single loop: one node, one artifact, the upstreams read once |

There is no separate loop per function. `px_elem::fill` is the only loop in this operator family —
each body supplies the per-cell expression and nothing else.

## Identity

An element instance is keyed by content, not by the graph that uses it:

```
declaration hash  ‖  roots roster  ‖  interface  ‖  normalised body template  ‖  body file bytes
```

Two axes that a hand-written instance would carry are deliberately **absent**:

- the **op id** — it is a label for readings and error messages. The specification's name lives in
  `px_elem` and is shared by every graph, so it is not a per-graph identity.
- the **graph program's own source fingerprint** — the body and everything it depends on live in
  the dylib, and the other axes already cover them. A graph program editing one character must not
  rotate an instance's key.

The consequence is the point of the design: **two graphs using the same specification get the same
key, the same library, and the same artifact.** Editing one body file rotates exactly that one key.

## Adding a function

1. Add `px_elem/src/<fn>.rs`: the marker struct, its `Params` struct (with `#[serde(default,
   deny_unknown_fields)]` and a `Default`), and its input struct if it takes upstreams.
2. Add `px_elem/body/<fn>.rs`: the per-cell arithmetic, fully-qualified imports, `//` comments.
3. Add one line to `px_elem/src/specs.rs`.
4. `cargo run -p px_graphs --bin px -- list` and then `build` — the new instance appears and is
   compiled. No graph binary is recompiled.
