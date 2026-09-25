# The volume domain

A volume is a three-dimensional scalar field stored on a **cubical-sphere grid** and read along
view rays. The grid is the storage; the marching is the consumer. The two halves live in different
crates, and only one of them is allowed to integrate light.

| Layer | Crate | What it owns |
|---|---|---|
| payload + declarations | `px_volume_schema` | `VolumeData`, the parameter sets, the `px_op!` lines — no algorithm |
| operator bodies | `px_volume_op` | a dylib: `px_body!` per operator, loaded at runtime by identity |
| CPU algorithms | `px_volume_alg` | coarse bake, density bake, star bake, and a marching **draft used as a test oracle** |
| GPU implementation | `px_volume_gpu_op` + [src/sampler.wgsl](../px_volume_gpu_op/src/sampler.wgsl) | per-voxel emission bake and the single implementation of marching |
| reference field | `px_verify::proxy` | the one mapping from `cloud.coarse` parameters to a field sample |

## The payload: `VolumeData`

[px_protocol/src/art.rs](../px_protocol/src/art.rs#L409) defines it:

```rust
pub struct VolumeData {
    pub res: u32,        // samples per face axis (s and t)
    pub layers: u32,     // radial samples
    pub inner: f32,      // radius of layer 0
    pub outer: f32,      // radius of layer layers-1
    pub lanes: u32,      // channels per cell: 1 = density, 6 = emission
    pub data: Vec<f32>,
}
```

The data order is `data[((((face * layers + layer) * res + t) * res + s) * lanes) + lane]`
(`VolumeData::lane_slot` is the only place the per-cell stride is written). `s`/`t` are the in-face
parameters, `layer` is the radial index, `face` is `0..6`.

**Why lanes are interleaved rather than planar.** A marching sample reads the eight corner cells of
one point and needs every channel at each corner. Interleaved, the six channels of a cell sit in one
cache line, so one gather of eight corners serves both the emission and the extinction channel
(`gather_at` in the shader is called twice with `lane` and `lane + 3` against the same corner
indices). A structure-of-arrays layout would need six passes over the same grid.

Which lane holds what:

| Payload | `lanes` | Layout |
|---|---|---|
| density volume (`cloud.density`), coarse volume (`cloud.coarse`) | 1 | the stored scalar |
| emission volume (`cloud.emission`) | 6 | `[emit R, emit G, emit B, σ_R, σ_G, σ_B]` |

`VolumeData` has a hand-written `Default` (it yields `lanes = 1`, because a zero-lane volume has no
meaning). The accessors split by contract:

* `at(face, layer, t, s)` is the single-lane reader and **hard-asserts `lanes == 1`**.
* `lane_slot` / `emission_at` are the per-channel readers (`emission_at` asserts the channel is in
  range).
* `expect_single()` is a hard assert that single-lane consumers call as their entry gate.
* `samples()` = `6 · res · layers · res`.

The six-lane contract is asserted at the producers and at the single-lane consumers, but not at the
marcher: `raymarch_sky` reads channel `lane` and `lane + 3` with a stride that is fixed at six (the host
passes `LANES = 6` as `volume.shape.z`), so a single-lane volume handed to it would be read as if it were
six-lane. The graph is what keeps that from happening — `sky.nebula`'s input is `cloud.emission`'s
payload.

**On the wire** the volume is one `f32` blob whose shape is `[6, layers, res, res]`, plus a fifth
dimension `lanes` **only when `lanes > 1`** (`VolumeData::blobs`). A four-dimensional shape therefore
means one lane. `from_blob` reads the optional fifth dimension, defaults to 1, and rejects the payload
if `data.len() != samples · lanes`. The manifest carries `res`, `layers`, `inner`, `outer`
([px_graph_schema/src/build.rs](../px_graph_schema/src/build.rs#L139)); on decode, `res`/`layers` come
from the blob shape and `inner`/`outer` from the manifest. The asset kind is `AssetKind::Volume`,
which is deliberately distinct from `AssetKind::VoxelField` (a volume *as a field*, i.e. the
`Domain::Volume` layout used upstream in the graph) — a volume is a four-dimensional payload, a voxel
field is a two-dimensional row-major grid.

## Parameter space

The six faces are the cube faces of `px_protocol::art::cube_direction` / `cube_face_of` (the two are
exact inverses; `px_protocol/tests/cube.rs` pins the round trip). `px_volume_schema::PATCHES` is
`CUBE_FACES` = 6. The face number is passed **explicitly**, never folded into the coordinates: adjacent
faces agree bit-for-bit on their shared edge, which is what lets six patches be welded into one closed
mesh and what keeps a straight ray from seeing a step at a face seam.

The radial law lives in [px_volume_schema/src/volume.rs](../px_volume_schema/src/volume.rs#L34):

```text
r(u) = inner · (outer/inner)^u          // geometric in world space, linear in parameter space
u(r) = ln(r/inner) / ln(outer/inner)
dr/du = r · ln(outer/inner)             // Shell::stretch_of
```

`u` is linear in everything (`layer = floor(u · layers)`), and the non-linearity exists only in this
pair of functions, which keeps sampling O(1). The reason for the geometric law is isotropy: an angular
cell measures `r · Δθ` in the world, so the radial cell must also scale with `r`; a linear ladder gives
a 3:1 "pie slice" at `r = 3`. `inner <= 0` degenerates to the linear law (no ratio is defined).

`point_of(face, [u, v, altitude], inner, outer)` is the world mapping, and `VolumeGrid` (the
interpolating sampler used by the isosurface operator) does trilinear reads **within one face**.
`inner`/`outer` are a property of the volume, not of the grid shape: per-cell operators work in
`[0,1]³` voxel coordinates and never need them, while marching and density baking do.

## Grid shape

The grid is `6 · res² · layers` cells. `res` and `layers` are independent parameters, and that is
structural: tying them together (e.g. `layers = res/2`) makes the cell count cubic, which at
`res = 128+` does not finish and at `256` fails to allocate. The two volume bakes have separate
defaults (`cloud.coarse` 65/65, `cloud.density` 64/64).

## Operators

Declarations: [px_volume_schema/src/ops.rs](../px_volume_schema/src/ops.rs). Bodies:
`px_volume_op/src/*.rs`.

| id | Inputs | Params | Payload | Runs on |
|---|---|---|---|---|
| `sky.stars` | `StarsInput { volume }` | `params::stars::StarsParams` | `StarField` | CPU |
| `cloud.coarse` | `CloudCoarseInput { coverage }` | `params::Params` | `VolumeData` (1 lane) | CPU |
| `cloud.density` | `DensityInput { density }` | `params::density::DensityParams` | `VolumeData` (1 lane) | CPU |
| `cloud.emission` | `EmissionInput { volume, stars }` | `params::emission::EmissionParams` | `VolumeData` (6 lanes) | GPU |
| `sky.nebula` | `SkyInput { volume, stars }` | `params::sky::SkyParams` | `TextureData` (cube, `Rgba16Float`) | GPU |

`sky.stars` is not a field operator: its payload is a point table plus a uniform sparse grid, and the
renderer never reads it. The dependency direction is `density → stars → emission → sky`: stars
reject-sample the density volume so that gas and stars share their large-scale distribution, and emission
reads the star field to light the gas. A star field cannot be built from the emission volume — that
would close the cycle, and the star query indexes a single channel while the emission volume is six
interleaved lanes.

### `cloud.coarse` — `params::Params`

Bakes the field the isosurface extractor wants: the stored value is `(field − tau) / scale`.

| Param | Default | Meaning |
|---|---|---|
| `field` | `coarse` | `coarse` = `shape_of(cover, altitude, 1.0)` (the noise upper bound, structurally enclosing the real field); `final` = the shader's `cloud_field` value |
| `res` | 65 | samples per face axis (`res × res` rays per face) |
| `layers` | 65 | radial layers |
| `inner`, `outer` | 1.01, 1.06 | shell radii |
| `tau` | 0.20 | the hard-surface threshold (the shader's `surface_level`) |
| `scale` | 240.0 | `L`: the gradient bound used for normalisation. It must not be too small (a pruned extractor's test `|f| < size·√3` only holds when `|∇f| ≤ 1`); the dense extractor in use is interpolation-only, so `L` changes the stored magnitude and not the mesh |
| `reach` | 1 | conservative dilation radius in cells (0 = off). The tangential neighbourhood is taken as a maximum over directions |
| `coverage`, `base`, `top`, `taper`, `coverage_gain`, `erode`, `orientation` | 0.35, 0.06, 0.62, 0.45, 2.6, 0.0, `[0,0,0,1]` | the cloud shape parameters, which must match the `clouds` slot of the scene recipe |

The bake (in `px_volume_alg`) walks each face node, takes the tangential maximum of coverage, evaluates
the reference field from `px_verify::proxy`, subtracts `tau`, multiplies by `1/scale`, and then dilates
along the radius while leaving the two wall layers untouched (moving them would push the isosurface
onto the domain wall and open the proxy). `FieldKind` participates in the key.

### `cloud.density` — `params::density::DensityParams`

A three-dimensional field (`Domain::Volume`) becomes a **steppable density volume**. This is a distinct
operator from `cloud.coarse`, not another mode of it: the values mean different things (density itself
versus `(field − tau)/L`), so the parameter sets differ.

| Param | Default | Meaning |
|---|---|---|
| `res` | 64 | samples per face axis (absolute, not a ratio of anything) |
| `layers` | 64 | radial layers, independent of `res` |
| `inner`, `outer` | 1.0, 1.6 | shell radii — the only place the world scale of this grid is defined |
| `reach` | 1 | radial conservative dilation in cells (0 = off) |

`shape_of()` returns `(max(res,2), max(layers,2))`. When the upstream field has the same `res`/`layers`
the bake is a pure **layout move** (row-major `[face][layer][t][s]` upstream, `[face, layer, t, s]`
here, no interpolation); otherwise it resamples trilinearly in voxel coordinates with the face offset
included in the row index. Either way the operator body then applies a hard gate: values below
`DENSITY_GATE = 0.02` become exactly `0`, and everything above is rescaled linearly into `[0,1]`. That
gate is what makes "vacuum" literally zero instead of a thin haze that would still be lit and still
extinct.

The bake itself (in `px_volume_alg`) applies two things per column: the radial dilation (max over
`reach` neighbours, wall layers untouched) and the `shell_wall` fade — a smooth window of
`SHELL_WALL_FADE = 0.14` of the radial span at each end, applied **after** the dilation so the
conservative pass cannot refill the wall. The fade is part of the density-volume semantics and is
shared by the bake and the verdicts.

### `cloud.emission` — `params::emission::EmissionParams`

Turns a density volume plus a star field into per-voxel emission and extinction — the "material" that
marching integrates. Lighting is computed per voxel here and never inside the marching loop: a shadow
ray depends on the point, not on the view ray, so computing it per voxel costs
`voxels × shadow_steps` while computing it while marching would cost `rays × steps × shadow_steps`.

| Param | Default | Meaning |
|---|---|---|
| `light` | `[0.3, 0.5, 0.8]` | direction of the ionising source (world space, normalised) |
| `light_radius` | 0.25 | the source's radius as a fraction of the shell (`0` = centre, `1` = outer wall) |
| `shadow_steps` | 24 | steps taken from each voxel towards the light point |
| `shadow_gain` | 1.6 | shadow density: `exp(−τ · shadow_gain)` |
| `emission_power`, `emission_gain` | 2.2, 1.0 | main emission: `d^power · gain`, a neutral scalar |
| `glow_gain`, `glow_power`, `glow_threshold` | 0.0, 1.0, 0.0 | the low-power "core glow": `max(0, d − threshold)^power · gain`; below the threshold it is exactly zero, which is a gate (by location) rather than a curve (by value) |
| `glow_tint`, `scatter_tint` | `[1,1,1]`, `[1,1,1]` | per-channel weights multiplied into the emission lanes; `glow_tint` multiplies the main term, the core glow and the scattered starlight alike, so it is the channel mix of *the light the nebula scatters*; `scatter_tint` is the diagnostic split (`[1,0,0]` scatter vs `[0,0,1]` starlight, paired with `SkyParams::star_tint`) |
| `extinction` | `[1.6, 2.4, 3.4]` | per-channel extinction `σ` (dust reddens because blue is eaten first) |
| `extinction_power` | 1.0 | `σ = d^power · extinction[channel]` |
| `dust_bias`, `dust_threshold` | 2.0, 0.35 | the extra dust term: `max(0, d − threshold) · bias` added to every extinction channel |
| `starlight_gain` | 0.0 | 0 = off. Gains the starlight scattered by gas |
| `starlight_soft` | 0.05 | softening radius of the starlight irradiance: `brightness / (d² + soft²)` |
| `starlight_radius` | 0.20 | how far a voxel looks for stars (a physical knob, independent of `sky.stars`' storage `cell`) |
| `starlight_steps` | 12 | shadow steps per candidate star |
| `starlight_max` | 8 | how many stars a voxel may keep, by brightness (0 = no cap) |

Per voxel: read `d`; march `shadow_steps` towards the light **point** accumulating world optical depth;
`lit = exp(−τ · shadow_gain) · (light_radius/d)²`; collect the brightest `starlight_max` stars within
`starlight_radius` and add a **colourless** `exp(−τ_star · shadow_gain) · brightness/(d² + soft²)` per
star (star colour deliberately does not enter this term — it is the channel mix of `glow_tint` that
decides what scattered light looks like). Output lanes:

```text
lane 0..2 = (main + glow + d^emission_power · starlight_gain · star_lit) · glow_tint · scatter_tint
lane 3..5 = d^extinction_power · extinction[channel] + max(0, d − dust_threshold) · dust_bias
```

The star term is added to the same shape as the main emission (`d^emission_power`), so scattering only
happens where there is gas. With `starlight_gain > 0`, a `starlight_max` above `STAR_KEEP_MAX = 32` is
refused before dispatch: the shader's "top few" table is fixed-size, and silently truncating it would
look like a slightly different gas brightness.

### `sky.stars` — `params::stars::StarsParams`

A batch of world-space point lights plus a uniform sparse grid (`px_sparse::StarField`). Positions are
uniform **by volume** in the shell (`r³` interpolation), directions uniform by solid angle, brightness
a power law `(1/draw)^power` with a ceiling.

| Param | Default | Meaning |
|---|---|---|
| `count`, `seed` | 180000, 60613 | how many field stars, and the deterministic hash seed |
| `inner`, `outer` | 1.0, 3.0 | the shell the stars live in; intentionally the same numbers as the density volume's shell |
| `cell` | 0.05 | storage resolution: the fine-cell edge length of the sparse grid |
| `brightness_power`, `max_brightness` | 1.1, 64.0 | the brightness power law and its ceiling |
| `min_apparent` | 0.0 | apparent-brightness cut: stars with `brightness · (inner/r)²` below it are dropped at generation (a magnitude cut, hence "dense near, sparse far") |
| `cluster_count`, `cluster`, `cluster_radius`, `cluster_gain`, `cluster_tint` | 4, `[0.50, 0.70, 1.80]`, 0.35, 1.2, `[0.72, 0.86, 1.0]` | a deliberate bright cluster embedded in the gas (a world-space point, not a direction) |
| `clump_count`, `clump_radius`, `clump_share` | 0, 0.25, 0.0 | statistical clumping: a share of the field stars is placed around random clump centres |
| `star_tint` | `[1,1,1]` | per-star colour written into the table; feeds the **direct** term only |
| `sky_biased`, `sky_frequency`, `sky_octaves`, `sky_lacunarity`, `sky_gain`, `sky_seed`, `sky_zonal`, `sky_contrast` | false, 0.35, 3, 2.0, 0.5, 9173, 0.9, 2.0 | large-scale bias by rejection sampling against an fbm of the same family as the gas envelope: accept with probability `noise^sky_contrast` |
| `gas_biased`, `gas_contrast` | false, 1.5 | exact correlation: reject-sample against the **real density volume** with probability `clamp((d/mean), 0, 1)^gas_contrast` |
| `gas_floor` | 0.0 | density floor relative to the mean: cells below it keep no stars at all |
| `gas_depth` | 0.0 | push stars outward with `((r − inner)/(outer − inner))^gas_depth`, so more gas stands between the camera (inside the shell) and the star |

`min_apparent` is evaluated on geometry and brightness only. The `gas_*` knobs do read the density
volume — the operator takes `cloud.density` as an input, and the body calls `expect_single()` on it, so
a six-lane emission volume cannot be fed in.

### `sky.nebula` — `params::sky::SkyParams`

Integrates the emission volume along each view ray and emits **one cube texture**, not three fields:
per-channel extinction makes the three channels genuinely different, so one operator producing one
texture is the right shape. The camera is the shell centre, so the chord through the shell differs per
direction and depth is free.

| Param | Default | Meaning |
|---|---|---|
| `face` | 256 | per-face resolution of the output cube (`face × face × 6`) |
| `steps` | 96 | ray steps; also the unit of the skip path's sample budget (`steps · 16`) |
| `jitter` | 1.0 | offsets each sample inside its cell by a per-texel, per-step hash, turning step banding into noise. `0.0` selects the plain cell sample verbatim |
| `star_gain` | 1.0 | multiplies direct starlight; anchored on the inner wall, i.e. `(inner/r)²` |
| `star_tint` | `[1,1,1]` | per-channel weights for direct starlight (pairs with `scatter_tint` for the red/blue diagnostic split) |
| `star_core` | 0.0029 | core radius of a star as a **world length** (not a pixel or a texel count) |
| `star_halo`, `star_halo_gain` | 0.010, 0.035 | halo world radius and its weight |
| `background` | `[0,0,0]` | the sky floor added through the final transmittance |

Output is a `TextureData`: `face × face × 6` layers, 1 mip, `Rgba16Float`, linear and unclamped (the
exposure decision belongs to the renderer). The three channels are integrated separately, each
collecting its own star candidates because transmittance differs per channel.

## The occupancy mask (empty-space skipping)

[px_volume_gpu_op/src/occupancy.rs](../px_volume_gpu_op/src/occupancy.rs) derives a read-only index
from an **already baked emission volume**. It is an acceleration structure, not a storage format: the
volume stays dense, the artifact bytes, cache key and manifest are unchanged, and the index can be
rebuilt at any time. It does not compute any radiance.

### Two levels, one predicate

```text
face → coarse block 8³ → fine block 2³ → cell
```

`COARSE = 8`, `FINE = 2`, `FINE_PER_AXIS = 4`, `WORDS_PER_BLOCK = FINE_PER_AXIS³ / 32 = 2`.

The predicate is **exactness, not a threshold**: a cell is empty iff all six lane values are exactly
`0.0`; a block is empty iff none of its cells is non-empty. `block_axes(res, layers)` is
`[ceil(res/8), ceil(layers/8), ceil(res/8)]` (s, radial, t), and the block's linear index inside a face
is `(ct · blocks_l + cl) · blocks_s + cs`; a sub-block's slot is `(sub_l · 4 + sub_t) · 4 + sub_s`.
These are the same orderings as `px_protocol::sparse`'s two-level occupancy, and the same constants.

### Internal storage vs upload layout

Two different layouts exist, and the writer, the Rust reader and the shader each use exactly one:

| | Layout |
|---|---|
| internal, `sidecar` | one `u32` per block (values 0/1), length `6 · blocks_per_face` |
| internal, `words` | `WORDS_PER_BLOCK = 2` `u32` per block, bit = sub-block slot |
| upload, `upload_words()` | `[L1 bits (one bit per block)][L2 mask (2 words per block)]` |

* `upload_words` packs `sidecar` **one bit at a time**: block `i` lives in bit `i % 32` of word `i / 32`.
  The L1 section is `l1_words() = ceil(blocks / 32)` words rounded **up to a multiple of 4**, and the L2
  section starts at exactly that word offset.
* `at_cell(face, s, t, radius, inner, outer)` reads the internal layout: `sidecar[packed]` for L1, then
  `words[packed · 2 + sub / 32]` bit `sub % 32` for L2.
* the WGSL `occupancy_class(point)` reads the upload layout: `occupancy_words[packed / 32]` bit
  `packed % 32` for L1, then `occupancy_words[l1_words + packed · 2 + sub / 32]` bit `sub % 32` for L2,
  where `l1_words` arrives in `OccupancyUniform::scalars[2]`.

Because L2 fills exactly two `u32`s per block, L1 cannot be folded into "word 0 of each block": it has
its own section. The word grid also has to be a multiple of 4 so that every L1 word starts on a `u32`
boundary. `WORDS_PER_BLOCK` is computed on both sides from `FINE_PER_AXIS` rather than written as a
literal.

### Class values and the skip algorithm

`at` / `at_cell` and `occupancy_class` all return the same three-way class:

| Class | Meaning |
|---|---|
| 0 | the coarse block is empty → the whole block can be dropped |
| 1 | the block is live but this `2³` sub-block is empty |
| 2 | this sub-block contains a non-empty cell |

Both the Rust and the WGSL side locate a cell with the same integer rules: `cs = min(u32(s·res), res-1)`,
`ct = min(u32(t·res), res-1)`, `cl = min(u32(altitude·(layers-1)), layers-1)`, where `altitude` comes
from the geometric radial law. The coordinates are the volume grid's own (`px_volume_schema::direction_of`
/ `cube_direction`), reached from a world direction through `grid_coords_of` — the inverse of
`cube_direction` (the same mapping as `px_protocol::art::cube_face_of`).

The shader's loop in `march_radiance` walks the ray along coarse blocks:

1. Enter at the layer boundary containing `enter`: `radius = layer_radius(enter_layer)` — this is the
   only thing the skip mode changes about the starting point; it moves no sample.
2. Ask `occupancy_class(direction · radius)`.
   * **Class 0** → jump. `next_layer = min((block + 1) · COARSE, max_layer)`, `boundary =
     clamp(layer_radius(next_layer), enter, outer)`, and the cursor moves to
     `max(boundary · (1 + 1e-5) + 1e-6, radius + step)`, `current = next_layer`. The overshoot is not
     cosmetic: `layer_radius` is a `pow`, so `layer_index(boundary)` can land one ulp below
     `next_layer` and send the cursor back to the block it just skipped, which makes the cursor spin in
     place until `radius >= outer` and blacks out every ray near a block seam.
   * **Class 1 or 2** → the block is worth sampling. `samples = clamp(block_high − block_low, 1, 2048)`
     layers, charged to a budget of `steps · 16` units (16 units per sample, so fractional sample
     counts accumulate across blocks); when the budget cannot afford one sample the loop breaks, the
     same way the dense path ends when its steps run out. If the sub-block test at the block's low layer
     classifies as 1, the block is dropped as a whole without sampling.
   * Otherwise sample each layer of the block at its **radial midpoint**:
     `sample_distance = (layer_radius(l) + layer_radius(l+1)) / 2` with
     `step = layer_radius(l+1) − layer_radius(l)`, then `radiance += T · emit · step`,
     `T ·= exp(−σ · step)`, and consume stars whose radius has been passed. Stop when `T < 1e-4`.
3. `i` advances by the number of layers consumed; the loop ends on `i >= steps`, `radius >= outer`, or
   `T < 1e-4`.

With skipping disabled (`skip = 0`) `occupancy_class` returns 2 immediately and the same loop takes the
dense branch: `here = (i + 0.5)/steps`, `distance = shell_radius(here)`,
`step = shell_radius((i+1)/steps) − shell_radius(i/steps)`, and `i` advances by one.

### Why skipping is (almost) an identity

The predicate is exact zero, so trilinear reconstruction over cells that are all exactly zero is exactly
zero, and `exp(−0) = 1`: a skipped block contributes no emission and no extinction. This is the argument
the code states, and the pairing verdict
`the_skip_matches_the_dense_march_on_a_blocky_volume` (same WGSL, same `steps`, `skip` flipped) checks it
per texel with a `2e-2` tolerance; the residual recorded in the code is `0.0142`.

That residual is attributed to the two paths being *different quadratures of the same integral*: in live
blocks the skip path samples layer midpoints of that block, the dense path samples
`shell_radius((i+0.5) · du)`. Two further facts belong with the argument:

* the sampling stencil reads the sample's own cell and the next cell up along each axis, so a sample in
  the last cell of an empty block can still have a non-zero true value when that next cell lies in a live
  block. The mask is **not** dilated to cover that one-cell border; nothing in the predicate compensates
  for it.
* the empty predicate is on all six lanes, so "empty" is the same statement the marching makes when it
  reads emission and extinction.

### Environment switches and readings

Two switches are read in `raymarch_sky` only. Both test **presence**, not value:

| Variable | Effect |
|---|---|
| `PX_SKIP_REPORT` | prints one line: `res`, `layers`, blocks per face, total blocks, live blocks and their percentage |
| `PX_SKIP_OFF` | drops the mask and dispatches with `skip = 0`, i.e. the dense march through the same shader |

They change no artifact, and they are not part of any cache key: to measure both settings, vary a
parameter (for example `steps`) as well, or the second run is a cache hit.

## The ray marcher

Everything that integrates along a ray is in [sampler.wgsl](../px_volume_gpu_op/src/sampler.wgsl) and is
dispatched from [px_volume_gpu_op/src/lib.rs](../px_volume_gpu_op/src/lib.rs). Entry points:

| Entry | Purpose |
|---|---|
| `sample_points` | samples one lane at arbitrary world points (the point-by-point GPU↔CPU verdict) |
| `march` | one channel of a cube map, no grading |
| `sky_radiance` | all three channels, ungraded |
| `bin_luma` | log-spaced luma histogram (input quantiles for the response curve) |
| `tone_of`, `hue_of_keys`, `grade_pixels` | the grading stages |
| `bake_emission` | the per-voxel emission/extinction bake |

### Sampling

`sample_volume(point, lane)`:

1. Early-out to `0.0` outside `[inner − tol, outer + tol]`, with `tol = max(|outer − inner|, 1) · 1e-5`
   (the relative tolerance exists because the first and last layers sit exactly on `inner`/`outer` and
   `f32` puts the sum a hair outside; a strict comparison would read both shell walls as zero).
2. Direction → face, in-face parameters; radius → altitude → layer, with the `1e-3` snap that keeps a
   sample that is within a thousandth of a layer on that layer.
3. **All eight corners are resolved across faces**: for a corner cell the shader rebuilds the cell-centre
   direction, maps it back with `cube_face_of(cube_direction(face, s, t))` and takes the cell it lands
   on. Corners are never wrapped inside the current face; doing that makes a sample step across a face
   edge and is exactly the cause of a full-height seam in the output cube map.

### Integration (dense)

Per step `i ∈ [0, steps)`: `here = (i + 0.5)/steps`, `distance = shell_radius(here)`,
`step = shell_radius((i+1)/steps) − shell_radius(i/steps)`; then

```text
radiance      += transmittance · emit · step
transmittance *= exp(−σ · step)
```

and afterwards stars whose radius is `<= distance` are added with the **current** transmittance
(`transmittance · brightness · tint · PSF(sin θ, r) · star_gain · (inner/r)²`), so a near star is not
attenuated by the whole shell while a far one is. The step is in parameter space and therefore geometric
in world space — the same isotropy argument as the grid. `transmittance < 1e-4` stops the ray
(`stopped`), which **discards** the remaining candidates rather than paying them with a floor
transmittance; if the loop instead runs to the end, the remaining candidates are collected and added with
the final transmittance, then `transmittance · background` is added.

### Skipping and the star queue

With the mask enabled the loop becomes the block walk described above and the samples in live blocks move
to layer midpoints. Star candidates are collected per radial slab (a band of one fine grid cell) and
consumed in ascending radius order; the queue is fixed-size (`STAR_PENDING_MAX = 64`), and an overflow
increments a counter the host reads back and turns into an `Err` — silently dropping stars would only look
like slightly dimmer gas. Direct starlight is a world-space PSF expressed in world lengths
(`star_power`: `exp(−(offset/core)²) + halo_gain · exp(−(offset/halo)²)`, `offset = sin θ · distance`), so
a star's angular size follows `radius/distance` and a brighter star's visible disc is larger; the
support radius is `3 · max(core, halo)` in world units and is divided by the slab radius to test
candidates. Irradiance is `(inner/r)²`, anchored on the inner wall.

The GPU marcher reads the jitter from `scalars[0]` and applies it **inside** the occupancy sum: the
offset moves the sample within its own layer's `[low, high)` span, so it can never leave the cell
the mask classified. `jitter = 0.0` selects the plain cell sample verbatim rather than adding a zero
offset, so turning jitter off reproduces the earlier bytes exactly.

### The whole sky

`raymarch_sky` is three GPU dispatches plus a readback:

1. `sky_radiance` — three channels, ungraded.
2. `bin_luma` + `anchors_from_radiance` — measures the input quantiles `p10/p50/p85/p99` from a 512-bin
   log2 histogram over `[2^-16, 2^4]` and uses them as the response curve's input anchors; the output
   anchors are constants fitted from the reference image. Anchors are forced strictly increasing with a 6%
   minimum gap, because equal anchors make a segment's slope divide by zero.
3. `grade_pixels` — brightness response first (`tone(l)/l`, log-log piecewise interpolation between the
   anchors with exponential extrapolation at both ends and a C¹ soft shoulder asymptotically approaching
   `0.95`), then the per-texel hue ramp keyed on **that texel's own luma** at
   `GRADE_STRENGTH = 0.95`, with the luma preserved exactly and pure black passed through unchanged.

The result is packed as `Rgba16Float` with `alpha = 1`. `tone` and `ramp_hue` in WGSL mirror
`px_volume_alg::raymarch::tone` / `ramp_hue`; the anchors, ramp table and shoulder/ceiling travel in the
`SkyUniform` so the shader holds no tuned constant of its own.

### Emission bake

`bake_emission` is one compute dispatch over voxels. A single-lane density volume is widened to six lanes
on the host (cell `i`'s value goes to lane 0, the rest zero) because the shader's sampler carries the lane
count as the constant 6. Counterpart of `px_volume_alg::bake_emission`, cross-checked per voxel. The host
refuses `starlight_gain > 0` with `starlight_max > 32`.

## The CPU copy, and what it may be used for

`px_volume_alg` contains a complete CPU version of the pipeline: `eval_sampled` / `coarse_with` (coarse
bake), `bake_density`, `sample_world`, `bake_stars`, `bake_emission`, `raymarch_channel`,
`raymarch_sky`, plus the grading constants that the GPU side also reads.

* **Operators that legitimately run on the CPU**: `cloud.coarse` (`eval_sampled`), `cloud.density`
  (`bake_density`), `sky.stars` (`bake_stars`). These are bakes of fields and point sets, not light
  transport.
* **The CPU marching copy is a draft and an oracle.** `px_volume_op` routes `cloud.emission` and
  `sky.nebula` to `px_volume_gpu_op`, and nothing else calls `raymarch_channel` / `raymarch_sky`: they are
  used by the GPU↔CPU verdicts inside `px_volume_gpu_op`, by `px_volume_alg`'s own tests, and by the
  `star_probe` instrument as a reference. There is no operator, graph or graph program that marches on the
  CPU, and no feature is allowed to exist only there. `raymarch_sky` on the GPU side returns an `Err` when
  no device is available rather than falling back.
* **The shader is the specification.** Where the CPU copy and `sampler.wgsl` disagree, the WGSL is the
  truth and the CPU copy is the bug; the verdicts in `px_volume_gpu_op` compare the two point by point and
  texel by texel under tight tolerances. Two GPUs would be worse than one: bake and render use the same
  backend.

## A real chain

`px_graphs/src/bin/nebula.rs` builds two graphs from `art/nebula/*.toml` and `art/nebulasky/*.toml`:

```text
graph "nebula"      field.fbm3 / ridged3 / warp3 → cloud.density → sky.stars → cloud.emission
                                                                                    │
graph "nebulasky"                            sky.nebula ← (the same star field handle)
```

* The two graphs must be separate: the volume chain's fields live on the `Domain::Volume` grid while the
  sky's live on a cube map. `VolumeShape::of` rejects a cube map as a volume grid.
* The star field is defined **once** (`art/nebulasky/stars.toml`, read by the script and handed to both
  nodes) so that both graphs key the same node: two copies of those parameters would produce two star
  fields, and "the gas is lit by a different set of stars than the sky shows" is not an error, only a
  subtly wrong picture.
* `art/nebula/density_volume.toml` carries `cloud.density`'s parameters; the script reads `layers` from it
  and uses the same value for the upstream `field.*` shape, so "the volume and the field are the same
  grid" is visible in the script instead of being implied by a global resolution.
* `art/nebulasky/sky.toml` carries `sky.nebula`'s parameters, and their values are the shipped ones:
  `face = 1024`, `steps = 96`, `star_core = 0.006` (a world length), `star_gain = 0.10`,
  `star_tint = [0.12, 0.30, 1.0]`, and in `art/nebula/emission.toml`
  `light_radius = 0.28`, `shadow_steps = 48`, `shadow_gain = 150.0`, `emission_power = 4.5`,
  `emission_gain = 0.0`, `glow_gain = 0.0`, `glow_tint = [1.0, 0.03, 0.03]`,
  `starlight_gain = 0.30`, `starlight_radius = 0.4`, `starlight_soft = 0.02`,
  `starlight_steps = 24`, `starlight_max = 16`, `extinction = [20, 20, 20]`,
  `dust_bias = 1.7`, `dust_threshold = 0.38`.

The program runs end to end. An element operator on a `Domain::Volume` field takes its direction from
a single probe (`px_elem::fill`, `Field::direction_probe`) rather than per cell, because a volume cell
has no direction, and `art/nebula/density_volume.toml` names its in-face grid `res`.

Its `--face` switch is parsed and printed but drives nothing: the sky resolution is `sky.toml`'s `face`,
and the volume resolution is `--shape` (with `--layers` overriding the layer count from
`density_volume.toml`).
