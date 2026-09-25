# NURBS

Curves and surfaces as non-uniform rational B-splines: exact circles and spheres, evaluation and
its derivatives, knot insertion and degree elevation, and tessellation down to a mesh or a
polyline. A GPU evaluation path exists for the two tessellation operators.

## Why `f64`

The whole library computes in `f64`. `f32` cannot hold the weight `√2/2` that makes a rational
quadratic circle a circle: round it and the radius drifts by roughly `1e-4`. Representing conics
*exactly* is the reason NURBS is here at all, so the precision is not negotiable. Only the final
conversion to a payload — `MeshData`/`PolylineData`, whose wire format is `f32` — drops precision,
and that step is stated as manufacturing tolerance rather than treated as error.

## Where the pieces live

| Layer | Contents |
|---|---|
| `px_nurbs_schema` | the payload types (`Curve`, `Surface`, `PointData`), knot and basis-function utilities in `knot.rs`, the reusable half of the algorithms (`insert_knot`, `elevate_degree`, `sphere`, `circle`), assembly into `MeshData`/`PolylineData` in `mesh.rs`, and the operator declarations |
| `px_nurbs_op` | the implementations, built as a dylib: one body per operator plus the adaptive tessellator |
| `px_nurbs_gpu_op` | the GPU evaluation path: `surface.wgsl` / `curve.wgsl` plus dispatch and readback |

The reusable half lives in the schema crate rather than in the implementation because geometric
invariants — knot insertion and degree elevation must not move the geometry, and circles and
spheres are exact — are what the tests measure directly, and the declaring layer has to compile and
be testable without an implementation library present.

## Payloads

A `Curve` and a `Surface` carry control points, knots, weights and degree. Evaluation produces a
`PointData`: the point, optionally first derivatives, and for a surface the unit normal.

Tessellation produces **two different payloads**, and the split is deliberate. A curve tessellates
to a `PolylineData` — vertices plus line-segment indices. A surface tessellates to a `MeshData` —
vertices plus triangle indices, with per-vertex normals. A polyline is not a mesh: it has no
normals and no area, so the three things `MeshData` promises (one normal per vertex, indices in
threes, a meaningful triangle count) would all be false. Forcing a curve into a mesh can only be
done with degenerate zero-area triangles, which makes the triangle count a lie.

## Operators

| Id | Inputs | Parameters (defaults) | Produces |
|---|---|---|---|
| `nurbs.circle` | — | `radius` (1.0), `plane` (`"xy"`), `center` ([0,0,0]) | 9-point, 4-segment exact circle |
| `nurbs.sphere` | — | `radius` (1.0), `center` ([0,0,0]), `rings` (2) | exact sphere; each `rings` step is 90° of latitude |
| `nurbs.curve.eval` | `curve` | `u` (0.5), `v` (0.5), `tangent` (false) | point (and first derivative) at the parameter |
| `nurbs.curve.at` | `curve`, `point` | same | point, with the parameter supplied by an upstream `PointData` |
| `nurbs.curve.hodograph` | `curve` | same | tangent vector — the `tangent` flag's always-on form |
| `nurbs.surface.eval` | `surface` | `u`, `v`, `tangent` | point, two partials and the unit normal |
| `nurbs.surface.at` | `surface`, `point` | same | as above, with `(u, v)` from upstream |
| `nurbs.curve.insert` | `curve` | `t` (0.5), `times` (1), `along` | curve with extra control points; geometry unchanged |
| `nurbs.surface.insert` | `surface` | `t`, `times`, `along` (`"u"`/`"v"`) | surface with extra control points; geometry unchanged |
| `nurbs.curve.elevate` | `curve` | `degree` (3) | curve of higher degree; geometry unchanged |
| `nurbs.surface.elevate` | `surface` | `degree` | both directions raised; geometry unchanged |
| `nurbs.curve.tessellate` | `curve` | `tolerance` (1e-3), `depth` (6), `segments` (4) | `PolylineData` |
| `nurbs.surface.tessellate` | `surface` | `tolerance`, `depth`, `segments` | `MeshData`, with normals from the surface |
| `nurbs.surface.tessellate.gpu` | `surface` | `tolerance`, `depth`, `segments` | `MeshData`, evaluated on the GPU |
| `nurbs.curve.tessellate.gpu` | `curve` | `tolerance`, `depth`, `segments` | `PolylineData`, evaluated on the GPU |

`tolerance` is a chordal error bound in world units — the knob that trades vertex count against
looking round. `depth` caps recursion so a cusp cannot recurse forever, and `segments` is the
initial split of the parameter domain before refinement begins.

The `.at` family exists so that "where to evaluate" can be computed by another node rather than
written into a parameter file. The input structs (`CurveAtInput`, `SurfaceAtInput`) are part of the
interface, which is why they live next to the declarations.

### The GPU variants are separate operators

`…gpu` is not a flag on the CPU operator: it is a distinct operator with a distinct id, so the two
have different keys and their artifacts never overwrite each other. A graph picks one by naming it.

The GPU variants share the CPU variants' verdicts — chordal error, watertightness, closure — but
not necessarily their topology: the GPU curve tessellator bisects uniformly per level while the CPU
one refines adaptively per segment. If no usable device is present the GPU operators **fail**; they
do not fall back to the CPU. Choosing one is a declaration that the machine has a GPU.

## What the tests pin

- **Knot insertion and degree elevation do not move the geometry** — checked to a numeric
  tolerance rather than by comparing control points.
- **Circles and spheres are exact** — after tessellation every vertex lies on the radius, with the
  only deviation coming from `f64` rounding.
- **Tessellated surfaces are watertight and on the radius**, and Euler characteristic is checked.
- The analytic comparison against exact geometry lives in `px_nurbs_schema/tests/analytic.rs`, and
  the end-to-end path through the loading gate in `px_graphs/tests/nurbs_pipeline.rs`.

## Running the GPU path

`px_nurbs_gpu_op` embeds its WGSL with `include_str!` and dispatches through `px_gpu`. Its tests
create a real device, so they belong to the fast chain only because the crate is small; they do
require a GPU to pass.
