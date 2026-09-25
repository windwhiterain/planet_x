# Backlog

Unresolved questions and known work. The rest of `docs/` describes current behaviour; this file is
the only place that does not.

Defects discovered while rewriting the documentation are recorded in [`FINDINGS.md`](../FINDINGS.md)
at the repository root, with a reproduction or a source trace for each.

## Awaiting a decision

**What is documented, and where does an idea go before it is true?** `docs/` states only what holds
today. An idea that cannot yet be written as a present-tense fact has no home other than this file.

## Known defects

### Any element operator on a volume field aborts the process

`px_elem::fill` unconditionally calls `out.direction(x, y)`. For `Projection::Volume` that reaches a
deliberate panic, and in the operator path the panic crosses a dylib boundary, which is fatal and
uncatchable. Reproduced with the `nebula` graph.

### `px run nebula` cannot start

`art/nebula/density_volume.toml` sets `res_ratio`, which is not a field of `DensityParams`. The
graph parses that file before cooking anything.

### `TextureData::encode` is broken for `Rgba8Srgb`

The blob shape is written as `bytes.len()/2` for both texture formats, but `Rgba8Srgb` has an
element size of 1, so the length self-check rejects every non-empty rgba8 texture. Latent today: the
one operator that emits a texture emits `Rgba16Float`.

### `mesh.cubesphere` silently ignores unknown parameters

Its parameter struct is the only one without `deny_unknown_fields`, so a typo in a recipe is
defaulted away instead of reported.

### `Shape::default()` is not a whole cube map

The default is `512 × 256` with a `CubeMap` projection, but a cube map requires `height = 6·width`.
Nothing validates the relation.

### `tangent_frame` has a discontinuity

The reference axis switches at `|direction[1]| > 0.99`, so `field.gradient` and `field.warp` inherit
a discontinuity ring that no existing test sweeps.

## Weakened gates

- GPU-backed tests in two default-member crates print a skip line and **pass** when no device is
  present, so the fast chain is green while those comparisons never ran.
- The source-fingerprint test omits the NURBS crates, leaving their build scripts and identity
  exports ungated.
- The operator-loading test claims to cover every declared operator and covers about two thirds.

## Unconsumed inputs

25 of the 44 scene recipes under `art/scene/` are referenced by no source file or script. They are
single-purpose comparison recipes from earlier measurements. They are inputs, so deleting them
changes no key, but a measurement someone wants to re-run may depend on one.
