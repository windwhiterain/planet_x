# The visual layer

How a picture is specified, compiled, and drawn: the scene recipe language, the material and shader
set, and the astronomical content that exists today. The renderer that consumes the compiled result
is in [renderer.md](renderer.md); shader assembly and reflection are in [shaders.md](shaders.md).

## The pipeline

```
art/scene/<name>.toml          a scene recipe: what is in the shot
        |  px_graphs --bin scene <name>
        v
a scene document (.pxart)      geometry + materials + textures + lights + cameras + pass table
        |  px_render --scene <artifact> --out <png>
        v
a PNG
```

A recipe names graph members for the things that are procedural (a height field, a mesh, a coverage
cubemap) and gives material parameters by name. The compiler reads the *baked artifact* of each
member; it does not re-cook anything. A scene document embeds the keys of its members, so its bytes
change when they do.

`--no-frame-graph` is a compatibility escape hatch: it emits a document with the frame-graph
sections empty. Its only purpose is to prove that an older artifact shape is still reproducible
byte for byte. It is not a second supported way to cook.

## The recipe language

Top level:

| Key | Meaning |
|---|---|
| `name` | the scene's name; becomes the artifact id |
| `ambient` | ambient light level |
| `cameras` | which camera table to use; `"review"` (the default) is the review set |
| `frame` | which frame graph to use from `art/frame/`; default is `default` |
| `skybox` | `"graph::node"` to use a cooked image as the skybox instead of the built-in star field |
| `skybox_graph` | the graph, when `skybox` names a bare node |
| `parts` | the list of parts |

Each `[[parts]]` entry:

| Key | Meaning |
|---|---|
| `id` | this part's name |
| `kind` | one of `planet`, `clouds`, `atmosphere`, `moon`, `light` — anything else is refused, and the error names the accepted set |
| `shader` | which shader member to use. Optional, because a `light` has no material |
| `graph` | the default graph for this part's members; cross-graph members are written `graph::node` |
| `primitive` | for `kind = "planet"`: `"icosphere"` uses the built-in sphere instead of the `mesh` member |
| `members` | named graph members, e.g. `height`, `mesh`, `palette`, `coverage`, `shader` |
| `params` | material parameters and structural keys, by name |

Parameter names are resolved against the target shader's reflected contract:

- names the **compiler** needs (radius, palette, light, ablation mode, …) are structural and are
  consumed by the compiler rather than forwarded;
- names in the shader's parameter table are forwarded with the type the shader declared — this is
  what makes "add a parameter" a change to WGSL and a recipe only, with no Rust edit;
- names in neither table are an error that lists both tables.

Packing is a completeness check performed at cook time, so a parameter the shader declares but
nobody supplies fails while cooking rather than producing an artifact that will be refused on load.

## Frame layer

`art/frame/default.toml` describes how the renderer draws one frame, and it is **shared by all
scenes** rather than copied into each: the frame is the shape of the renderer, not content. A scene
selects it with `frame = "…"`.

The order is: a depth prepass, then opaque geometry, then the skybox, then transparent geometry,
with content passes inserted, then a blit to the final target.

Frame-declared resources include the scene ping-pong colour pair, the scene depth, the point-light
shadow atlas and its three downsample levels, and a depth snapshot used by passes that must both
bind and write depth in different passes. Frame-declared materials (the skybox) are inlined into
the artifact, so editing `art/frame/*.wgsl` requires re-cooking.

Frame materials take parameters by **reference to the environment** rather than by value (for
example `brightness = "environment.skybox_brightness"`), so the frame description stays shared when
a scene changes its brightness.

## Point-light shadows

Point-light shadows use a **virtual, paged** shadow map. Each shadow-casting light gets a cube; each
cube face is one pass; within a face, pages are allocated to objects by their declared
`shadow_density` in texels per world unit. Resolution therefore does not depend on how far the
light is — a fixed-size per-light cube made distant suns blurry.

Three coarser levels are produced by downsampling, and a level's page is the **maximum** of its four
children. Geometry is rasterised only into its own level; the coarser levels are filled by the
downsample chain. An allocator that cannot fit the demand reports an error rather than silently
reducing precision.

Two resource pairs exist purely because wgpu forbids using one texture as a depth attachment and a
bound resource in the same pass: the shadow atlas has separate per-level textures, and the scene
depth is copied to a snapshot before it is sampled. A pass that would otherwise bind the atlas it
writes binds a 1×1 dummy instead.

## Materials and shaders

The shader set under `art/shaders/`:

| Shader | Renders | Parameters |
|---|---|---|
| `surface.wgsl` | rocky or icy planet surfaces | `orientation`, `emissive`, `inner`, `outer`, `coverage`, `shadow`, `height`, `gain` |
| `gasgiant.wgsl` | banded gas giants | `orientation`, `albedo`, `absorption`, `wrap`, `thickness`, `limb`, `band_gain`, `sun_gain`, `ambient`, `band_contrast`, `hemi_depth`, `haze`, `detail_scale`, `detail_strength`, `belt_tint`, `zone_tint`, `polar_tint`, `filament_gain` |
| `clouds.wgsl` | volumetric cloud shells | `orientation`, `tint`, `inner`, `outer`, `density`, `coverage`, `base`, `top`, `detail_scale`, `detail_strength`, `erode`, `phase`, `shadow`, `steps`, `bump`, `seed`, `ablate`, `slope_scale`, `taper`, `coverage_gain`, `surface_level`, `bound`, `gradient`, `wind`, `wind_skin` |
| `atmosphere.wgsl` | limb haze and atmospheric shells | `inner`, `outer`, `density`, `softness`, `tint` |
| `ring.wgsl` | planetary rings | `tint`, `ambient` |
| `shadow_downsample.wgsl` | shadow-map pyramid reduction | `level`, `light`, `face`, `page_size`, `origin_x`, `origin_y` |
| `blit.wgsl` | final transfer to the output | `gain` |
| `px_grade.wgsl` | colour grading pass | `strength` |
| `px_vignette.wgsl` | vignette pass | `strength` |

`art/shaders/lib/` holds the shared libraries imported by these: noise, lighting, and common
utilities. The import closure is part of each shader's identity — see [shaders.md](shaders.md).

Frame-owned shaders live under `art/frame/` instead: `skybox.wgsl` and the two vertex shaders
(`vertex_mesh.wgsl`, `vertex_sky.wgsl`).

## How a part becomes objects

A `planet` part is the scene's subject and is required: it carries the height field, the mesh, and
the palette. Depending on its members it produces a displaced sphere with its material, a generated
colour-ramp texture, and a coverage cubemap.

A `clouds` part adds a volumetric shell around the planet, using a coverage cubemap from the graph
and a baked volume. An `atmosphere` part adds a thin scattering shell. A `moon` part is a second
body with its own built-in sphere, position, radius, subdivisions and spin — its geometry is a
primitive rather than a graph member, so it needs no bake of its own. A `light` part adds a point
light with position, colour, intensity, and whether it casts shadows.

Rings are not a part kind: a planet opts in with the `rings` parameter, which generates the ring
mesh and band texture in the compiler.

## Scenes that exist

The scenes in `art/scene/` fall into groups. The ones that carry the content:

| Scene | Depicts |
|---|---|
| `orbit.toml` | the reference planet with clouds and atmosphere |
| `orbit-soft.toml` | the soft-cloud variant of the same shot |
| `orbit-bare.toml` | the planet and atmosphere without clouds |
| `orbit-gasgiant.toml` | the banded gas giant with a moon |
| `orbit-uranus.toml` | the same gas-giant graph with an ice-giant palette |
| `orbit-moon.toml` | the cratered moon, with a second light standing in for earthshine |
| `orbit-rings.toml` | the only scene with rings enabled |
| `orbit-surface.toml` | the surface detail study |
| `orbit-proxy.toml`, `orbit-proxy-fine.toml`, `orbit-proxy-fine-bound.toml` | cloud-mesh proxy variants |
| `nebula.toml` | a nebula seen from inside, skybox supplied by a cooked graph |

The remaining files in that directory are single-purpose comparison and probe recipes used to
isolate one parameter at a time (sun distance, shadow on/off, resolution, cloud extinction). They
are inputs to specific measurements rather than content.

## Pass recipes

`art/passes/` holds standalone pass tables used to exercise the pass executor and the grade path:
`identity`, `invert`, `invert_vignette`, `grade_half`, `compute`, `scratch`, `none`, and
`bad_entry` (a deliberate negative case).

## Reference images

`art/reference/` holds images of what the result should look like. They are targets, not verdicts:
nothing hashes them, nothing tests against them, and deleting one changes no key. They are read by a
person when deciding where to take the look next.
