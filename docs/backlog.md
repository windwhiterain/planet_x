# Open decisions

Unresolved questions and known work. Nothing here describes current behaviour; the rest of
`docs/` does that.

## Awaiting a decision

**Where do unresolved design questions and ideas live now?** The rewrite removed the dated round
notes and the previous backlog. Ideas that cannot yet be stated as a present-tense fact need a home
that is clearly not a state document — either this file, or an explicitly separated part of the
repository. Until that is settled, record them here.

## Known defects

Found while rewriting the documentation by reading against the code. Each was reproduced or traced
to the source.

### Any element operator on a volume field aborts the process

`px_elem::fill` unconditionally calls `out.direction(x, y)`. For `Projection::Volume` that reaches a
deliberate panic in `px_protocol::art` (`direction_at`). In the operator path the panic crosses a
dylib boundary, which is fatal and uncatchable:

```
panicked at px_protocol/src/art.rs:816
体网格（Domain::Volume）没有「一个方向」这回事
fatal runtime error: Rust cannot catch foreign exceptions, aborting
```

Reproduced with the `nebula` graph, whose node after `warped` is an element `remap` over a volume
field. Any element operator applied to a volume field hits this.

### `px run nebula` cannot start

`art/nebula/density_volume.toml` sets `res_ratio`, which is not a field of `DensityParams`
(that struct denies unknown fields). The graph parses that file before cooking anything:

```
unknown field `res_ratio`, expected one of `res`, `layers`, `inner`, `outer`, `reach`
```

### `TextureData::encode` is broken for `Rgba8Srgb`

The blob shape is written as `bytes.len()/2` for both texture formats, but `Rgba8Srgb` has an
element size of 1, so the length self-check rejects every non-empty rgba8 texture. Only
`rgba16_float` works, and the test covers only that format. Latent: the one operator that emits a
texture emits `Rgba16Float`.

### `mesh.cubesphere` silently ignores unknown parameters

Its parameter struct is the only one without `deny_unknown_fields`, so a typo in a recipe is
defaulted away instead of reported.

### `Shape::default()` is not a whole cube map

The default is `512 × 256` with a `CubeMap` projection, but a cube map requires `height = 6·width`.
Nothing validates the relation, so a node that does not set its own shape produces a field whose
rows all land on one face.

### `tangent_frame` has a discontinuity

The reference axis switches at `|direction[1]| > 0.99`, so `east` jumps on that ring and
`field.gradient` and `field.warp` inherit the discontinuity. Existing seam tests only sweep
mid-latitudes.

## Weakened gates

- The source-fingerprint test omits the NURBS crates from its list, so their build scripts and
  identity exports are not gated.
- The operator-loading test claims to cover every declared operator and covers about two thirds of
  them.

## Comments that contradict the code

A substantial number of comments across the source cite a deleted renderer, removed types, and
numbered sections that no longer exist, and several state the opposite of the code beneath them —
for example a backend default of DX12 where the code defaults to Vulkan, an empty pass list
described as "main pass only" where it draws nothing, and a group layout described as having two
empty groups where one is populated. These are worth a sweep, but note the cost: the crates that
participate in keys require a full key rotation, which means rebuilding every operator library and
re-cooking every graph.
