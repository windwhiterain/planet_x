# The mesh domain

A mesh is a surface: positions, per-vertex normals, UVs, and triangles. The domain has two
operators, one that builds a displaced cube-sphere from a height field and one that extracts an
isosurface from a volume.

| Layer | Crate | What it owns |
|---|---|---|
| payload + declarations | `px_mesh_schema` | `MeshData` re-export, the parameter sets, the `px_op!` lines — no algorithm |
| operator bodies | `px_mesh_op` | a dylib: one `px_body!` per operator, loaded at runtime by identity |
| extractor | the `isosurface` crate | dense marching cubes (the only extractor used) |
| verdicts and instruments | `px_graphs/tests/cloud_proxy.rs`, `px_graphs/src/cloud_proxy.rs` | closure, containment, reproducibility, gradient bound |

There is no `px_mesh_schema/tests/`: the mesh verdicts live in `px_graphs`, because they need both the
volume bake and the mesh operator to run through the declaration path.

## The payload: `MeshData`

[px_protocol/src/art.rs](../px_protocol/src/art.rs#L352) defines four parallel arrays plus their blob
order:

```rust
pub const MESH_ATTRIBUTES: [&str; 4] = ["positions", "normals", "uvs", "indices"];
pub const MESH_POSITION: usize = 0;
pub const MESH_NORMAL:   usize = 1;
pub const MESH_UV:       usize = 2;
pub const MESH_INDEX:    usize = 3;

pub struct MeshData {
    pub positions: Vec<f32>,  // 3 per vertex
    pub normals:   Vec<f32>,  // 3 per vertex
    pub uvs:       Vec<f32>,  // 2 per vertex
    pub indices:   Vec<u32>,  // 3 per triangle
}
```

* `vertices()` = `positions.len() / 3`; `triangles()` = `indices.len() / 3`.
* `blobs()` encodes positions as `f32 [vertices, 3]`, normals as `f32 [vertices, 3]`, UVs as
  `f32 [vertices, 2]`, indices as `u32 [indices.len()]`.
* `from_blobs()` requires at least four blobs and rejects the payload unless
  `normals.len() == vertices · 3` and `uvs.len() == vertices · 2`.
* Readers walk triangles with `chunks_exact(3)`; `positions.len() % 3` and `indices.len() % 3` are
  **not** checked, so a trailing partial triangle would be silently dropped rather than refused. Write
  well-formed payloads.
* `Build::detail` for a mesh reports the vertex and triangle counts, and `encode` also writes
  `vertices` and `triangles` into the manifest parameters
  ([px_graph_schema/src/build.rs](../px_graph_schema/src/build.rs#L178)). `decode` ignores those two
  manifest entries and rebuilds from the blob headers, so the manifest counts are informational.
* Thin geometry uses `PolylineData`, not `MeshData`: a polyline has no normals, no area and no
  triangles, so forcing it into this payload would make `triangles()` lie.

The asset kind is `AssetKind::Mesh`. The renderer consumes meshes directly; it never reads a volume or
a star field.

## Operators

Declarations: [px_mesh_schema/src/ops.rs](../px_mesh_schema/src/ops.rs). Bodies:
`px_mesh_op/src/*.rs`.

| id | Inputs | Params | Payload |
|---|---|---|---|
| `mesh.cubesphere` | `CubeSphereInput { height: Cooked<Field> }` | `params::cubesphere::Params` | `MeshData` |
| `mesh.proxy` | `ProxyInput { volume: Cooked<VolumeData> }` | `params::proxy::Params` | `MeshData` |

### `mesh.cubesphere`

| Param | Default | Meaning |
|---|---|---|
| `subdivisions` | 128 | subdivisions per face axis; clamped to `[2, 512]` |
| `radius` | 1.0 | base radius before displacement |
| `displace` | 0.075 | displacement amplitude as a fraction of `radius` |
| `sea_level` | 0.52 | the height value that maps to exactly `radius` |
| `flat_sea` | false | clamp heights below `sea_level` up to `sea_level` (a flat ocean) |

The face size is **not** a parameter: it comes from the upstream field, `cube_face_size(field.width)`
for the UV atlas and `cube_cell_size(field.width)` for the audit readout.

### `mesh.proxy`

| Param | Default | Meaning |
|---|---|---|
| `level` | 0.0 | isosurface height, in the **stored** (normalised) field's units |
| `depth` | 6 | `2^depth + 1` samples per axis per face; clamped to `[1, 8]` |
| `weld` | 1e-4 | weld tolerance in world units; a non-positive value falls back to `1e-4` |
| `offset` | 0.0 | push every vertex `offset` world units along its outward normal; skipped when serialising if exactly `0`, so the default does not enter the key |

## The sphere mesh

`px_mesh_op/src/cubesphere.rs` builds `6 · (n+1)²` vertices (minus within-face duplicates) with
`n = subdivisions.clamp(2, 512)`.

**Subdivision.** Each face lays out an `(n+1) × (n+1)` parameter grid with `s = i/n`, `t = j/n`, and
`direction = cube_direction(face, s, t)`. Vertices are deduplicated per face by
`(face, quantize(direction))`, where `quantize` rounds each component to a multiple of `2^-18`
(`× 262144`).

**Displacement.**
`height = ((field.sample_direction(direction) − stats.min) / span).clamp(0, 1)`, where `stats` are the
upstream field's min/max and `span` is `max − min` (or `1.0` if the field is constant). For a cube-map
field the sample is bilinear in the face's `(s, t)`, and each of the four corner texels is resolved by
turning its direction back into a face — so a corner may come from the neighbouring face and the seam is
interpolated, not clamped. With `flat_sea` on, any height below `sea_level` becomes `sea_level`. The
vertex radius is

```text
lift = radius · (1 + displace · (shaped − sea_level))
```

so `shaped == sea_level` sits exactly on the sphere, and a `sea_level` above the field's low end pulls
the surface inside `radius`. UVs are the upstream field's cube-atlas coordinates
(`cube_atlas_uv(face, s, t, face_size, CUBE_GUTTER)`: three columns, two rows, a two-texel gutter),
so the mesh samples the same atlas layout the height field uses.

**Winding.** The first quad's corner triangle `(0,0), (1,0), (0,1)` gives a geometric normal, which is
dotted against the face-centre direction; if it points outward the quads are emitted as
`[p00, p10, p01] + [p01, p10, p11]`, otherwise reversed. Each face therefore follows the actual
displaced geometry instead of a fixed winding table.

**Normals.** Triangles accumulate their un-normalised cross product into each of their three vertices,
so the sum is area-weighted; each vertex normal is that sum normalised. A vertex whose sum is degenerate
falls back to the **radial** direction `normalize(position)` (and is counted in the audit as `zero`).

**Seams and poles.** Adjacent faces produce bit-identical directions along their shared edge, so their
positions coincide — but vertices are *not* index-welded across faces (each face's UVs live in its own
atlas cell, and a seam must be able to carry two different UVs). Instead the normals are welded: all
vertices whose quantised `normalize(position)` matches form a group, and members of a group receive the
normalised sum of their normals. That includes the eight cube corners, where three faces meet, so the
pole/corner shading is averaged rather than split.

**The audit line and the one hard failure.** Every build prints `vertices / triangles`, `face_size²`,
the cell size, the number of normal-weld **groups**, open edges, the inward count, the degenerate-normal
count and the worst `dot(normal, radial)`.
Open edges are reported, never refused (the closedness verdict lives in the tests). The one hard
failure is orientation: if any vertex's `dot(normal, radial)` is negative, the operator returns an
error naming the inward count and the minimum dot, and points at the two real causes — `displace` being
too large for this field, or the field containing detail finer than the mesh (a field with structure at
the mesh's own cell scale samples into spikes, and the winding derived from those spikes points
inward). Flipping the normals back is deliberately not done: that would hide self-intersecting geometry
rather than fix it.

## The isosurface path

`px_mesh_op/src/proxy.rs` reads a `VolumeData` through `px_volume_schema::VolumeGrid`, which does
trilinear interpolation **within a single face** and maps parameters to world points with the same
`point_of` used by the volume bake.

**Threshold and scale contract.** `cloud.coarse` stores `(field − tau) / scale`; `mesh.proxy` compares
that stored value against `level`. So `level = 0` means "the field equals `tau`", and a non-zero `level`
means "the field equals `tau + level · scale`". What is dropped by the normalisation is the field's own
units: the mesh operator never sees `tau` or `scale`, and because dense marching cubes only interpolates
between samples, multiplying the whole field by a positive constant does not move the isosurface — the
mesh is identical for every positive `scale`, only the stored magnitude changes. `scale` still matters
to the *key* (it is a parameter) and to any extractor that prunes on `|f|`. Pointing `level` below wall
value `−tau/scale` extracts nothing at all, which is reported as an empty proxy.

**Extraction**, per face:

1. `points = 2^depth + 1` samples per axis over the face's unit parameter cube.
2. A `ScalarSource` returns `field.sample(face, [u, v, altitude]) − level`; the extractor is the dense
   `MarchingCubes`. It is the dense one on purpose: the adaptive hashed octree's vertex positions follow
   hash-map iteration order (so two runs of the same parameters differ), and it cuts half a cell away
   from a domain wall, which leaves the two faces' cut polylines out of register. Dense marching cubes is
   bit-reproducible and puts a wall cut exactly on the shared 2D contour, which is what makes the six
   patches weld.
3. Each extracted local vertex `(u, v, altitude)` becomes a world position via
   `VolumeGrid::point` and a UV `[u, (face + v) / 6]`.
4. Vertices are welded against a hash grid of `weld`-sized cells: the vertex's cell is
   `floor(p / weld)` per axis, the 27 neighbouring cells are searched, and a candidate is merged only if
   the maximum absolute component difference is `<= weld`. Merging picks the closest candidate. The 27-cell
   sweep exists because two patches can differ by one ULP at a seam, which can land in the neighbouring
   cell.
5. Triangles are remapped through the welds and any triangle whose corners collided
   (`a == b || b == c || a == c`) is dropped.
6. Normals are area-weighted sums of triangle cross products. A degenerate sum falls back to the
   **negated parameter-space gradient** (central differences of `sample` in the face's own `(u, v,
   altitude)` with step `1/256`, no world-length scaling), and only then to `normalize(position)`.
7. `offset`, when non-zero, moves every vertex along an outward direction chosen in three steps: the
   vertex star (`|Σ cross|`) if it is well formed (`|Σ cross|² · 256 >= (Σ|cross|)²`), corrected by the
   sign of the mesh's signed volume so it follows the winding; otherwise the negated **world** gradient
   (central differences divided by the world distance between the two offset points, which is what makes
   it comparable across axes); otherwise the attribute normal. The positions array is the only thing
   changed — the attribute normals stay as computed, so `offset = 0` is bit-identical to not having the
   parameter.

**Audits and hard failures.** The build prints open edges, non-manifold edges, flipped directed edges,
the number of merged duplicate vertices and the number of gradient-fallback normals; open edges only
produce a second warning line. The single hard failure is an empty result: if no vertex survives, the
operator returns an error saying the volume contains no such isosurface, so the proxy is empty and
`tau`/`level` should be checked. Reading the volume also asserts `lanes == 1`, so a six-lane emission
volume cannot be fed in at all.

## Limits that are enforced rather than documented

| Where | Enforced |
|---|---|
| `mesh.cubesphere::subdivisions` | clamped to `[2, 512]`; the face grid is `(n+1)²` |
| `mesh.cubesphere` orientation | any vertex with `dot(normal, radial) < 0` is a hard error |
| `mesh.cubesphere` open edges | printed only — never an error |
| `mesh.cubesphere` parameters | `#[serde(default)]` **without** `deny_unknown_fields`: a mistyped key in `art/<graph>/<node>.toml` is silently ignored |
| `mesh.proxy::depth` | clamped to `[1, 8]` (`3 .. 257` samples per axis) |
| `mesh.proxy::weld` | `<= 0` falls back to `1e-4` |
| `mesh.proxy::offset` | `0` is skipped when serialising, so the parameter is absent from the canonical form unless it is non-zero and the default adds nothing to a node's key |
| `mesh.proxy` input | single-lane volume only: `VolumeData::at` asserts it |
| `mesh.proxy` empty result | hard error |
| `mesh.proxy` open edges | printed only; the closure verdict is in the tests |

## Verdicts

`px_graphs/tests/cloud_proxy.rs` drives both operators through the declaration path
(`PxOp::render` → runtime-loaded implementation dylib, so the dylib is exercised by the test itself) on
a synthetic coverage cube map, and asserts:

* `the_proxy_is_closed` — no open edges (a proxy with a hole would leak the hard surface).
* `the_mesh_op_is_reproducible` — positions, normals, UVs and indices are bit-identical across two runs
  (the cache is keyed by content).
* `the_proxy_encloses_the_coarse_field` — every test direction where the reference field crosses `tau`
  also has a proxy intersection, and the worst radial slack is smaller than one cell diagonal.
* `the_final_field_makes_a_tighter_proxy` — the `final` field is never higher than the `coarse` field at
  any node, and the resulting mesh is still closed.
* `the_gradient_bound_is_above_the_measured_gradient` — the measured `|∇coarse|` never exceeds
  `Params::scale`.

`px_graphs/src/cloud_proxy.rs` holds the instruments those verdicts use, statically (so they do not
depend on a dylib loading): deterministic scattered directions, a brute-force plus bisection root finder
for the reference field, Möller–Trumbore ray/mesh intersection, the cell-diagonal scale for a direction,
and the gradient-bound measurement (`measure_gradient_bound`).

## Real parameters

| File | Values |
|---|---|
| `art/planet/surface.toml` | `subdivisions = 160`, `radius = 1.0`, `displace = 0.075`, `sea_level = 0.520`, `flat_sea = true` |
| `art/desert/surface.toml` | `subdivisions = 160`, `radius = 1.0`, `displace = 0.085`, `sea_level = 0.520`, `flat_sea = false` |
| `art/moon/surface.toml` | `subdivisions = 160`, `radius = 1.0`, `displace = 0.045`, `sea_level = 0.0`, `flat_sea = false` |
| `art/clouds/coarse.toml` | `res = 65`, `layers = 65`, `inner = 1.01`, `outer = 1.06`, `tau = 0.20`, `scale = 240.0`, `reach = 1`, plus the cloud shape parameters |
| `art/clouds/proxy.toml` | `level = 0.0`, `depth = 5` (33 samples per axis per face), `weld = 1e-4` |

The cloud pair is the shipped isosurface chain: `cloud.coarse` bakes the coarse field into a shell of
`1.01 .. 1.06` world units around the planet, and `mesh.proxy` extracts the `level = 0` surface from it.
Its shape parameters must match the `clouds` slot of the scene recipe, or the proxy encloses a different
cloud than the one being rendered.
