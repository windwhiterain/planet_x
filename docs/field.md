# The field domain

A field is a scalar grid: a `width × height` block of `f32` plus a `Projection` that says where
each cell sits in the world. Anything that produces or consumes a scalar 2-D grid goes through this
domain — planet height maps, gas-giant band masks, cloud coverage, and the folded cube-sphere volume
grids that reuse the same layout.

The domain has three crates, split by what they are allowed to know:

| Crate | Kind | Holds |
|---|---|---|
| `px_field_schema` | rlib | the data (`Field`, `Projection`, `Shape`, `VolumeShape`), the parameter structs, the operator **declarations**, the payload codec, the row-band parallel helper. No algorithm. |
| `px_field_alg` | rlib | the algorithms both the preset library and the instance libraries link: the cell-for-cell remap path, the field-function interface, the lattice hash and noise vocabulary. |
| `px_field_op` | dylib | the nine operator **bodies**. Loaded at runtime by identity; it is not a cargo dependency of any graph program. |

A graph node names a declaration from the schema (`field::Fbm`), passes its parameters as an
ordinary Rust value, and gets a typed payload back. The declaration carries the id, the parameter
type, the input struct and the payload type; the body is fetched from the dylib by content identity.
See [model.md](model.md) for the two-stage build and the key rules.

## The payload

```rust
pub struct Field {
    pub width: u32,
    pub height: u32,
    pub data: Vec<f32>,        // row-major: data[y * width + x]
    pub projection: Projection, // = px_protocol::art::Domain
}
```

- **Cell addressing** is `Field::at(x, y)` / `Field::set(x, y, v)` with
  `index = y * width + x`. `x` runs along the row, `y` along the column; there is no other layout
  and no padding. Out-of-range indices panic through slice indexing.
- **Constructor checks only one thing**: `Field::with_projection` asserts
  `data.len() == width * height`. `Field::new` / `Field::filled` set the projection to `Equirect`;
  generators use `Shape::filled`.
- **Wire form** is one `F32` blob with shape `[height, width]` and little-endian `f32` payload,
  plus three manifest parameters: `width`, `height`, and `projection` (the frozen
  `Domain::code()`, see below). `Field::to_blob` / `from_blob` do the blob half;
  `px_field_schema::payload` does the manifest half and implements `Build` (`detail` /
  `encode` / `decode`). A node's reading is
  `"<w>×<h>｜值域 <min>..<max>｜均值 <mean>"` from `Field::stats()`, which computes the mean in
  `f64`.
- `payload::decode` takes the projection **from the payload's own manifest entry** and falls back to
  `Equirect` only when that entry is absent. A field payload therefore states its own domain:
  `AssetKind` is the renderer's notion of what a file on disk is, not a column of a graph cache
  payload.
- **Value domain**: nothing is enforced. Fields are plain floats and several operators add
  perturbations without clamping. The working convention across the graph is `[0, 1]` — `0` is a
  valley or no coverage, `1` is a peak or full coverage — and the noise generators land inside it by
  construction, but a graph is free to widen it (`elem` remap is not clamped either) and it is the
  consumers (mesh displacement, shaders) that interpret the range.

## Addressing beyond (x, y)

| Call | Meaning |
|---|---|
| `uv(x, y)` | texel-centre coordinates: `((x + 0.5) / width, (y + 0.5) / height)`, in `(0,1)²` |
| `direction(x, y)` | unit sphere direction of that cell, from the projection |
| `sample_uv(u, v)` | bilinear lookup at normalized coordinates |
| `sample_bilinear(x, y)` | bilinear in **texel** coordinates: `x` wraps (`rem_euclid`), `y` clamps |
| `sample_direction(d)` | projection dispatcher; `CubeMap` goes through the face-aware path, everything else through `uv_of` + `sample_uv` |
| `like(value)` | a constant field with **this** field's shape and projection |
| `filled_with(w, h, v, projection)` | the constant-field constructor |

`like` is the structural form of a rule: filtering operators do not take a shape, their output is
the upstream grid cell for cell.

## Projections

`Projection` is `px_protocol::art::Domain`. Its names and codes are frozen: the code is what a
stored payload carries, so renumbering would read old artifacts as another domain.

| Variant | `name()` | `code()` | Grid layout | uv → direction |
|---|---|---|---|---|
| `Equirect` | `equirect` | 0 | any `width × height` | `phi = u·2π`, `theta = v·π`, `dir = (sinθ·cosφ, cosθ, sinθ·sinφ)` (y up) |
| `Octahedral` | `octahedral` | 1 | square | octahedral map with y-up swizzle |
| `Cube` | `cube` | 2 | atlas: `cell = width / 3`, `face_size = cell − 2·2`, `face = (y/cell)·3 + x/cell` | cell-local index minus a 2-texel gutter, divided by `face_size` |
| `CubeMap` | `cubemap` | 3 | `width` = face size, `height = 6 × width`; face `= y / width` | `s = (x + 0.5)/width`, `t = ((y % width) + 0.5)/width` |
| `Volume` | `volume` | 4 | `width = res`, `height = res · layers · 6` | not a sphere: the third dimension is folded into `height` |

Inverses: `uv_of` maps a direction back to `(u, v)` per projection (`Equirect` via `acos`/`atan2`,
`Cube` via `cube_atlas_uv`, `CubeMap` via `[s, (face + t)/6]`). A volume cell is not a direction:
`direction_at` **panics** for `Domain::Volume`, and `direction_at_opt` is the same function returning
`None` instead. Per-cell loops that may meet a volume **must** use the `_opt` form — the panic
crosses an operator dylib boundary uncatchably and aborts the process. `uv_of` for a volume returns
the in-face `(s, t)`.

## CubeMap

`cube_map_extent(face_size)` returns `(face_size, face_size * 6)`; that pair is the shape of a cube
map, and `px_graphs` uses it (`const FACE: u32 = 256` ⇒ `256 × 1536`).

- **Index → (face, t, s).** `face = y / width`, `s = (x + 0.5)/width`,
  `t = ((y % width) + 0.5)/width`. The cell is addressed as
  `field.at(x, face * width + row_in_face)`: the six faces are stacked along `y`, face `f` owning
  rows `f·width .. (f+1)·width`.
- **Direction → face.** `cube_face_of` picks the face from the dominant axis of `(x, y, z)`
  (ties resolve x, then y, then z) and the side from that component's sign; `a / major` and
  `b / major` become `s` and `t`, clamped to `[0, 1]`.
- **Face → direction.** With `a = 2s − 1` and `b = 2t − 1`, `cube_direction` is:

  | face | direction (before normalising) |
  |---|---|
  | 0 | `[1, −b, −a]` |
  | 1 | `[−1, −b, a]` |
  | 2 | `[a, 1, b]` |
  | 3 | `[a, −1, −b]` |
  | 4 | `[a, −b, 1]` |
  | 5 | `[−a, −b, −1]` |

- **The six faces meet exactly.** `cube_direction` gives the same direction on a shared edge, so two
  adjacent faces sample the same point of the sphere there and no per-face offset is needed for the
  seam to close. Cell centres on either side of an edge are about one texel apart; the seam test
  asserts the gap stays under three texels.
- **Sampling crosses faces.** `Field::sample_cube_map` resolves the face, converts to continuous
  face coordinates, and interpolates by re-deriving the four corner directions through
  `cube_direction` and truncating each to a texel — a bilinear filter that is valid across an edge by
  construction.
- **The face-size relation is validated at the payload boundary.** `Shape::check` refuses a
  `CubeMap` whose `height != 6 * width`, naming both values and the required one. A truncated height
  would otherwise be read as "every row past face 0 is face 0" (`direction_at` clamps the face
  index) — silent corruption rather than a panic.

## Shape is a parameter

```rust
pub struct Shape { pub width: u32, pub height: u32, pub projection: Projection }
```

There is no global canvas and no resolution in the graph's `GraphSpec`; whoever produces a field
states its shape in its own parameters. Consequences that are load-bearing:

- **Generators carry a `shape` field; filtering operators do not.** Changing a generator's shape
  therefore changes its key and everything downstream of it, and it cannot change the key of an
  operator whose parameter table has no `shape` column. The rule is structural, not a per-domain
  flag.
- `Shape` gets a `HashField`; the projection enters the key **by its `code()`**, alongside the two
  extents, with field names included as usual. Two shapes that differ only in projection are
  different keys.
- `Shape::filled(value)` makes a constant field; `Shape::direction(x, y)` and
  `Shape::volume_shape()` are the other two uses.
- The default is `512 × 256` with `CubeMap`, which is vestigial from an equirectangular era
  (exactly 2:1) and is **not** a valid cube map. It is harmless in practice: every graph binary that
  produces a field sets its shape explicitly, and `Shape::check` refuses the invalid combination at
  the payload boundary, so the
  default only shows up when a parameter file is missing and the script does not override it.

Parameters arrive as ordinary Rust values: `node_params(&graph, "<node>")` reads
`art/<graph>/<node>.toml` (a missing file means `Default`), and the script may override any field
before handing the value to `cached`. The key holds the canonical JSON of the **effective** value, so
the parameters in the key and the parameters the operator sees are the same value by construction.

`art/planet/` is the worked example:

| File | Node | Domain |
|---|---|---|
| `continents.toml` | `continents` (`field.fbm`) | field |
| `mountains.toml` | `mountains` (`field.ridged`) | field |
| `terrain.toml` | `terrain` (`elem::Mix`) | element |
| `height.toml` | `height` (`elem::Remap`) | element |
| `weight.toml` | `weight` (`elem::Constant`) | element |
| `surface.toml` | `surface` (`mesh::CubeSphere`) | mesh |

`continents.toml` sets `frequency = 0.55, octaves = 6, lacunarity = 2.0, gain = 0.5, seed = 7,
aspect = 2.0`; `mountains.toml` sets `frequency = 1.9, octaves = 5, lacunarity = 2.0, gain = 0.55,
seed = 21, aspect = 2.0, sharpness = 1.8`. Neither file mentions `shape` or `spherical`: the script
supplies `Shape { width: 780, height: 520, projection: Cube }` and leaves `spherical = true`
(the default). No parameter file under `art/` sets a `shape`; sizes are chosen in the graph script.

## Volume fields

The volume domain reuses this data layout instead of inventing a second one: a volume grid is a
plain field whose third dimension is folded into `height`,
`height = res · layers · 6`, laid out as `[face][layer][t][s]` with row
`face · res · layers + layer · res + t`. `VolumeShape { res, layers }` is **derived** from the shape
parameters (`res = width`, `layers = height / (res·6)`), and `VolumeShape::of` returns `None` when
the domain is not `Volume` or the rows do not divide into whole layers.

The coordinate authorities are `px_field_schema::volume`:

- `local_voxel_of(shape, face, x, y)` → `(s, t, altitude)` in grid space, using integer division for
  the layer (a float round-trip can jump a layer) and the **face's own row origin** (a bare
  `y % (res·layers)` would read every face at face 0);
- `voxel_of(shape, face, x, y)` → the 3-D noise-space point `cube_direction(face, s, t) · radius`
  with `radius = SHELL_FRONT + altitude` and `SHELL_FRONT = 2/π`, chosen so that one face's arc
  length (`(π/2)·FRONT = 1`) has the same scale as the radial span;
- `slot_of` / `row_of` / `in_face_row` / `row_within_layer` / `layer_of` for row arithmetic;
- `VolumeShape::matches(field)` is the single predicate for "is this field a volume grid of this
  shape" (domain **and** both extents).

The volume domain itself (marching, occupancy, emission) is documented with that domain; what matters
here is that a volume grid is a field, so the cell-for-cell vocabulary applies to it unchanged.

## The operators

Ten declarations live in `px_field_schema/src/ops.rs`; nine have a body in `px_field_op`. Each
declaration is one line of the form `px_op! { Name, "id", "lib", Params, Inputs, Payload }`, and the
body is the matching `px_body!` line at the top of its file. The symbol name is
`<library>__<declaration>`, assembled at compile time on both sides.

| id | declaration | upstream inputs | computes | output range |
|---|---|---|---|---|
| `field.fbm` | `Fbm` | none | fractal Brownian noise: weighted, normalised octave sum | `[0, 1]` |
| `field.ridged` | `Ridged` | none | fractal noise of `(1 − \|2x − 1\|)^sharpness` per octave | `[0, 1]` |
| `field.fbm3` | `Fbm3` | none | the same fbm, sampled at **volume** voxel coordinates | `[0, 1]` |
| `field.ridged3` | `Ridged3` | none | the same ridges, sampled at volume voxel coordinates | `[0, 1]` |
| `field.gradient` | `Gradient` | `field` | tangential gradient of the upstream on the sphere, one world component | unbounded |
| `field.warp` | `Warp` | `field`, `offset` | resamples `field` after pushing each cell's direction along its tangent frame by amounts read from `offset` | within `field`'s range |
| `field.warp3` | `Warp3` | `field`, `offset_a`, `offset_b`, `offset_c` | resamples a volume grid after shifting each voxel coordinate by the three mean-centred offset fields | within `field`'s range |
| `field.craters` | `Craters` | `base` | adds a cell-distance crater profile to the upstream | upstream ± the profile |
| `field.stamps` | `Stamps` | `base` | stamps craters with random radius/age, excavating older relief, density masked by the upstream | upstream + the stamps |
| `field.remap/inst` | `FieldRemap` | `input` | **declaration only** — a graph-side field function is the implementation | `[0, 1]` by the function's own contract |

Parameters and their defaults, read from `px_field_schema/src/params.rs`:

| id | parameters (`name = default`) |
|---|---|
| `field.fbm` | `shape = 512×256 CubeMap`, `frequency = 4.0`, `octaves = 6`, `lacunarity = 2.0`, `gain = 0.5`, `seed = 7`, `aspect = 2.0`, `spherical = true`, `zonal = 1.0` |
| `field.ridged` | `shape = 512×256 CubeMap`, `frequency = 9.0`, `octaves = 5`, `lacunarity = 2.0`, `gain = 0.5`, `seed = 21`, `aspect = 2.0`, `sharpness = 1.6`, `spherical = true` |
| `field.fbm3` | `shape = 512×256 CubeMap` (must be overridden with a volume shape), `frequency = 3.0`, `octaves = 6`, `lacunarity = 2.0`, `gain = 0.5`, `seed = 7`, `zonal = 1.0` |
| `field.ridged3` | `shape = 512×256 CubeMap` (must be overridden with a volume shape), `frequency = 6.0`, `octaves = 5`, `lacunarity = 2.1`, `gain = 0.55`, `seed = 21`, `sharpness = 2.0`, `zonal = 1.0` |
| `field.gradient` | `component = 0`, `epsilon = 0.0` |
| `field.warp` | `strength = 0.40`, `lateral = 0.50`, `probe = 0.07` |
| `field.warp3` | `strength = 0.35`, `axial = 1.0` |
| `field.craters` | `frequency = 6.0`, `octaves = 3`, `lacunarity = 2.15`, `gain = 0.55`, `jitter = 0.85`, `seed = 31`, `aspect = 2.0`, `spherical = true`, `radius = 0.62`, `rim = 0.18`, `depth = 0.35`, `height = 0.16` |
| `field.stamps` | `frequency = 6.0`, `octaves = 3`, `lacunarity = 2.4`, `gain = 0.7`, `jitter = 0.9`, `seed = 31`, `aspect = 2.0`, `spherical = true`, `max_radius = 0.55`, `min_radius = 0.12`, `power = 2.2`, `depth = 0.22`, `height = 0.22`, `rim = 0.35`, `excavate = 0.85`, `degrade = 0.45`, `mask_lo = 0.0`, `mask_hi = 1.0` |
| `field.remap/inst` | `gain = 0.65`, `bias = 0.0`, `bands = 8.0` |

Details that the code makes load-bearing:

- **`spherical` selects the sampling space, and with it the noise kernel.** `spherical = true`
  samples at the cell's unit direction with the 3-D gradient noise (`faded_gradient_noise_3`, twelve
  Perlin-style gradients) or the 3-D Worley cell distance; `spherical = false` samples the planar
  `(u · aspect, v)` with the 2-D value-noise/Worley kernels. `aspect` is ignored when spherical, so
  the two modes are two different noise fields, not two parameterisations of one.
- **`zonal` (`field.fbm`)** multiplies the direction's y component before sampling: the noise then
  varies `zonal` times faster in latitude than in longitude, which is what makes features stretch
  **along** longitude. `1.0` is exactly the unmodified line, point for point.
- **`zonal` (`field.fbm3` / `field.ridged3`)** multiplies the whole voxel point, which leaves the
  direction untouched and scales the radius: structure is stretched along the radial direction.
- **`sharpness`** is the exponent on `1 − |2x − 1|`: larger means thinner ridges (`1.0` is a triangle
  wave).
- **`field.gradient`** `component` is clamped with `min(2)`. `epsilon = 0` means "derive it from the
  grid": `(π/2) / width` for `CubeMap`, `(π/2) / (width / 4)` otherwise. The output is the chosen
  world-axis component of the tangential gradient of the upstream, i.e. a slope in upstream units per
  radian — so it is not confined to `[0, 1]` and its sign is meaningful (it is what normal/slope
  consumers read).
- **`field.warp`** computes `east`/`north` from `tangent_frame(direction)`, reads
  `first = offset.sample_direction(direction) − 0.5` and, after stepping `probe` along `east`,
  `second = offset.sample_direction(probed) − 0.5`. It then displaces
  `direction + east·(first·strength) + north·(second·lateral·strength)`, normalises, and samples
  `field` there. `strength` and `probe` are displacements in unit-direction space, so their unit is
  an angle, and `tangent_frame` provides the basis without any branch: it is the rotation that takes the south pole to `direction`, applied to the x axis. The only degenerate direction is the south pole itself, which no cube-map texel centre reaches.
- **`field.craters`** takes the upstream as its first input and **adds** to it. Per octave
  (seed `seed ^ octave·0x9e37_79b9`) it takes the cell distance in grid units, then applies one
  profile: inside `radius` a bowl `(1 − d/radius)²` scaled by `−0.5·depth`, from `radius` to
  `radius + rim` a half-sine ring scaled by `+0.5·height`. Both terms are zero at `d = radius`, so
  layers can be summed. Each layer is weighted by `amplitude` and the accumulated perturbation is
  divided by the total weight, so the number of octaves does not change the magnitude.
- **`field.stamps`** is the same intent with a different placement rule: one stamp per lattice cell
  (position jittered by `jitter ∈ [0,1]`, radius drawn from the power law
  `r = min + (max − min)·(1 − u)^power`, age and a mask coin from a second hash), then every cell
  sorts the reachable stamps by age and excavates oldest to newest. A bowl first removes existing
  relief (`sum · (1 − excavate · bowl)`) and then subtracts its own depth; the rim adds
  `height · depth · radius · sin(π t)` over `radius .. radius(1 + rim)`. `excavate` is the
  "dig out, do not just add" knob and `degrade` ages older stamps. `depth` and `height` are
  ratios — depth per radius, rim height per depth — so their value-unit magnitude scales with
  radius. The mask is decided per **whole stamp**, sampling the upstream at the stamp centre between
  `mask_lo` and `mask_hi` (if `mask_hi <= mask_lo` the range falls back to `[0, 1]`).
- **These clamps are preconditions, not taste**: `jitter` in `craters` and `stamps` is clamped to
  `[0, 1]` because a feature point outside its own cell breaks the 27-cell search; `stamps` clamps
  `max_radius` to `1/(1 + rim)` so a footprint is at most one cell (again the search precondition),
  `min_radius` into `[1e-3, max_radius]`, `power` to at least `0.05`, and `excavate` / `degrade` to
  `[0, 1]`; `field.warp3` clamps `axial` to `[0, 1]`. No other parameter is clamped, and neither
  `craters` nor `stamps` clamps its output.
- **`field.fbm3` / `field.ridged3` require a volume shape** and panic when
  `params.shape.volume_shape()` is `None`; both iterate rows in parallel bands and reconstruct the
  field as `Field::with_projection(res, res·layers·6, data, projection)`. Their `zonal` semantics are
  the radial stretch above.
- **`field.warp3`** needs four fields of the same volume shape (it asserts `matches` on each) and
  takes its output shape from the first one. It subtracts each offset field's mean (otherwise a
  constant 0.5 offset translates the whole volume by half a cell), weights the three axes
  `[axial, axial, 1]`, shifts the **in-face** voxel coordinates (`local_voxel_of`), and resamples with
  a trilinear kernel that clamps at the edges and snaps near-integer fractional coordinates — the
  snapping is what keeps an exact cell-centre sample from blending two neighbours by `1e-7`.

## Field remap instances

`FieldRemap` (`field.remap/inst`) is the declaration reused by **generic instances**: it has no body
in `px_field_op`, so nothing exports the symbol its declaration line implies and using the
declaration as a value would fail at load time with a reported error. Its implementation is a Rust
function written on the graph side (`art/inst/*.rs`), compiled into a content-addressed instance
library and loaded by content key.

Two instances are registered today, in `px_graphs/src/inst_recipe.rs`:

| op id | type | field function | body |
|---|---|---|---|
| `field.remap/waves` | `Waves` | `art/inst/waves.rs` | `px_field_alg::remap_with(&px_field_alg::identity(), p, 1.0, i.input.value(), ARG)` |
| `field.remap/latbands` | `LatBands` | `art/inst/latbands.rs` | same |

Both declare `roots = ["px_field_alg"]`, so **an instance links exactly the algorithm crate** — the
primitives a field function may use are the exports of `px_field_alg` and nothing else. The instance
key covers the toolchain, the declaration crate's source fingerprint, the alg roster hash, the
interface hash, the normalised body template and the bytes of the field-function file; the human
readable op id is a label for readings and errors, not an axis of the key, so two graphs using the
same function share one library and one artifact.

The field-function contract (`px_field_alg::field_fn::FieldFn`) is
`(&RemapParams, upstream: f32, uv: [f32; 2], direction: [f32; 3]) -> f32`:

- `upstream` is the upstream cell's value **already normalised and clamped by the shared ruler**;
- `uv` is the texel-centre coordinate (the same numbers as `Field::uv`), usable directly in `sin`,
  distances and noise;
- `direction` is the cell's unit sphere direction — the only way to write a latitude band, a polar
  cap or a crater-shaped mask, because under `CubeMap` the `v` axis walks across the six stacked
  faces rather than across latitude;
- the return value must lie in `[0, 1]`; that is the function's own promise (the `clamp` at the end
  of `art/inst/*.rs`), not something the shared path does for it.

## The shared algorithm half

`px_field_alg` exists so that the preset library and the instance libraries compute the same thing
from the same parameters. Its single loop is `map_grid`:

```
for every cell of the upstream grid:
    t = scale.normalize(upstream.at(x, y))          // clamp to [in_min, in_max] → [0,1], optional smooth
    v = cell.value(params, t, uv(x, y), direction(x, y))
    out.set(x, y, bend(scale.map(v), gamma))
```

- The output is `upstream.like(0.0)`: **same shape and projection as the upstream**, always.
- `Scale { in_min, in_max, out_min, out_max, smooth }` is the ruler. `normalize` clamps to
  `[0, 1]`, applies `3t² − 2t³` when `smooth`, and returns `0` for a degenerate span
  (`in_max == in_min`) — `0`, not `1`. `map` scales to `[out_min, out_max]` and **does not clamp**;
  a graph that wants `[0, 1]` says so with its own bounds.
- `identity()` is `[0, 1] → [0, 1]` with `smooth = true`, which is the ruler the field-remap
  instances use: the upstream is smoothstepped before the field function sees it, and the three
  instance parameters (`gain` / `bias` / `bands`) never change the value-range convention.
- `remap_with` is the entry the generated instance libraries call; it is `map_grid` with an explicit
  ruler. The `gamma` argument is `1.0` for those instances because `RemapParams` has no
  non-linearity column; `bend` itself is shared with the element domain, where `gamma` is a
  parameter: `bend(value, gamma)` returns the value unchanged when `gamma` is not positive or is
  `≈ 1`, and otherwise maps a value `<= 0` to `0`.
- `Cell` is the "how is this cell computed" hole; `FieldFn` is the graph-side name for it, and every
  `FieldFn` is automatically a `Cell`. `Upstream` is the input hole; `Sampled { field }` is the
  implementation that treats a field as a function (`upstream ↦ upstream`).
- `map_grid` skips `uv` / `direction` for `Projection::Volume` and passes `(0,0)` / `+Y` instead:
  a volume cell has no single direction and a cell-wise value mapping never needs one.

The preset generators and kernels do **not** go through `map_grid` — they are their own loops over
their own sampling spaces. The element domain's functions (`elem::Remap`, `elem::Fuse`) do use the
same `Scale` and the same `map_grid`, which is why the ruler lives in the algorithm crate rather
than in either implementation.

## Noise primitives and determinism

`px_field_alg::noise` is the vocabulary **both** operators and instances may use:

| Primitive | What it gives |
|---|---|
| `lattice3(x, y, z, seed) -> u32` | the 32-bit integer lattice hash (wrapping multiplies and shifts, no floats) |
| `cell_hash(seed, cell) -> u32` | the same hash keyed by a cell, the "one cell, one number" entry |
| `unit(hash, channel) -> f32` | the hash's `channel % 4`-th 8-bit field mapped to `[0, 1)` by `/255` |
| `faded_gradient_noise_3(point, seed)` | 3-D gradient noise from twelve Perlin-style gradients and the `3t² − 2t³` weights, mapped to `[0, 1]`; generic over `Scalar`, so the arbiter's dual numbers reuse it |
| `value_noise3(point, seed)` | 3-D value noise: corner hashes plus the same smooth weights; bounded and aperiodic, flat on cell faces |
| `smoothstep(edge0, edge1, x)` | the `3t² − 2t³` transition, degenerating to a step when the edges coincide |
| `NEIGHBOURS_3` / `NEIGHBOURS_2` / `neighbours(spherical)` | 27 cells (3-D) or 9 cells (planar, `z = 0`), each including the centre |
| `cell_of` / `cell_centre` | point → cell index, cell index → cell centre |

The preset library adds noise the instances cannot see (`px_field_op::noise`): the 2-D lattice hash
and 2-D value noise, the 2-D/3-D `fbm` / `ridged` octave sums, the 3-D `fbm_3` / `ridged_3` sums and
the 2-D/3-D Worley cell distance with its hashed feature points. The split is real: planar
`field.fbm` / `field.ridged` use the 2-D value noise, their spherical mode uses the 3-D gradient
noise, and `field.craters` / `field.stamps` use Worley in their planar mode and 3-D Worley
(spherical) otherwise. Instances get the primitives they were given and re-derive nothing.

The determinism rule is a single sentence: **a value depends only on an integer cell and a seed.**
Concretely:

- hashing is integer arithmetic (`wrapping_mul`, xor-shifts) over `i32` cell coordinates and a `u32`
  seed; floats enter only after the hash is taken;
- every random draw comes from an explicitly derived seed — per octave `seed ^ octave` in the noise
  generators and `seed ^ octave·0x9e37_79b9` in `craters` / `stamps`, and a second, differently
  mixed hash where two independent draws are needed (`stamps` uses `seed ^ 0x51ed_270b` for age and
  coin so that "young" and "stamped" are not correlated);
- independent numbers are taken from **different bit fields** of that hash (the 8-bit channels of
  `unit`, or four 10-bit fields in `field.stamps`), never by rescaling one field;
- there is no global RNG, no lookup table on disk, and no dependence on thread identity, wall-clock
  time, or iteration order;
- row-band parallelism (`px_field_schema::parallel::rows`, used by `field.fbm3`, `field.ridged3`,
  `px_elem::fill` and the volume banding) splits by rows, so each thread writes a disjoint slice and
  every cell's arithmetic is identical to the serial order — the parallel result is bit-for-bit the
  serial one. Nothing accumulates across cells. The helper is fallible: a band worker's panic has no
  other route back to its caller, and left to unwind it would reach the dylib boundary and abort the
  process, so `rows` catches it (the scope re-panics at teardown even after a joined handle, which is
  why catching the join result alone is not enough) and returns it as an error the operator body
  propagates. Callers that cannot fail — tests, probes — say `.expect(…)` at the call site.
  Changing that return type is a window operation: it rotates the keys of every crate that calls the
  helper, so the re-cook belongs after the last source edit of the window (docs/backlog.md, batch
  window discipline).

That is what makes "same parameters, same upstream ⇒ same bytes" true, and therefore what makes a
cache key over parameters and upstream keys sound.

## Deliberately not in this domain

- **No canvas and no global resolution.** Shape is a parameter of whichever operator produces the
  field (see [model.md](model.md)). Filtering operators have no shape column at all.
- **No cell-wise (pointwise) operators.** The element functions — Rust paths
  `px_graphs::elem::{Constant, Mix, Remap, Fuse}`, operator ids `field.constant`, `field.mix`,
  `field.remap`, `field.fuse` — live in `px_elem` and do not appear in
  `px_field_schema/src/ops.rs`. A function of `(value, uv, direction)` needs no knowledge of a
  domain, and several of them can fuse into one node and one loop. What stays here is what is
  genuinely spatial: noise sources, spatial kernels, and anything that must **resample an
  upstream**.
- **No interpolation where the layout already matches.** Filtering is a cell-for-cell map of the
  upstream grid (`Field::like`), reading the same index; interpolation appears only where the sample
  point is genuinely off-grid: `field.warp` / `field.warp3`, cube-map sampling, `sample_uv` /
  `sample_direction`, and the stamp mask lookup. A resolution change is not an operator; a field is
  generated at the resolution it is wanted at.
- **No third dimension on `Field`.** `layers` is not a field: the volume grid folds it into
  `height`, which is what lets the same cell-wise vocabulary work on volume grids without a second
  asset type or a second codec.
- **No output-range enforcement.** Operators clamp only where a documented precondition requires it
  (the search neighbourhoods of `craters` / `stamps`, `axial`, the gradient component, and `bend`);
  `Scale::map` deliberately does not clamp, so "the output must be in `[0, 1]`" stays a promise of
  the graph (or of the field function), and adding a clamp somewhere would silently change artifacts.
- **No state.** A field is a value; operators take values and return values, and `cached` is the
  only thing that remembers anything.

## Where to look

| Question | File |
|---|---|
| the payload, addressing, sampling, cube-map filtering | `px_field_schema/src/field.rs` |
| how a stored field is encoded and decoded | `px_field_schema/src/payload.rs` |
| operator ids, inputs, libraries | `px_field_schema/src/ops.rs` |
| every parameter and its default | `px_field_schema/src/params.rs` |
| volume row arithmetic and voxel coordinates | `px_field_schema/src/volume.rs` |
| row-band parallelism | `px_field_schema/src/parallel.rs` |
| the shared remap loop and ruler | `px_field_alg/src/remap.rs` |
| the field-function interface | `px_field_alg/src/field_fn.rs` |
| hash, noise and neighbourhood vocabulary | `px_field_alg/src/noise.rs` |
| one file per operator body | `px_field_op/src/ops/*.rs` |
| cube-map seam and face-reachability tests | `px_field_schema/tests/cube_map_sampling.rs` |
