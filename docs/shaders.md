# Shaders: assembly, import closure, reflection, host stubs

How a WGSL entry shader becomes a self-contained text the host can hand to
`create_shader_module`, how its identity is computed, and what is read back out of it.

The whole layer lives in `px_shader`, a leaf crate. It depends on `px_protocol` (types only) and on
`naga` — no `wgpu`, no host crate. That is what lets the cook side, the host, and the probes share
one set of rules instead of writing three that drift apart.

## Where shaders live

Two roots, one convention (`px_shader::workspace_roots`):

| Root | Holds | Marker |
|---|---|---|
| `<workspace>/art/shaders/lib` | library modules | each file declares `#define_import_path <name>` |
| `<workspace>/art/shaders` | entry shaders | none may declare `#define_import_path` |

Both roots are scanned **one level deep** (`wgsl_files` does not recurse), so the `lib/` subtree is
not collected a second time by the entry root, and every `.wgsl` is collected exactly once.

`workspace_modules` builds the **module table** — `ModuleTable = BTreeMap<String, String>`, module
name to full file source. The name comes from the file's `#define_import_path` line, never from the
file path. Two files declaring the same module name is an error: a silent first-wins would mean the
key is computed from one copy while the other is assembled.

Entry lookup is by **file name** across both roots (`workspace_source_of`): exactly one match, or
`Err`. Zero matches and two-or-more matches are both failures, so a duplicated name can never mean
"the probes test one copy and the host runs the other".

The table today is `planet_x::common`, `planet_x::light`, `planet_x::noise`; the entry root holds
`atmosphere`, `blit`, `clouds`, `gasgiant`, `px_grade`, `px_vignette`, `ring`, `shadow_downsample`,
`surface`. `bake_shader_graph` bakes **every** entry it finds there (the test for "is this an entry"
is exactly "has no `#define_import_path`"). `px_render::shader::CONTENT_SHADERS` names the four the
offline gate covers: `surface.wgsl`, `atmosphere.wgsl`, `clouds.wgsl`, `ring.wgsl`.

`art/frame/*.wgsl` is a third, separate place — see the last section.

## `#import`: syntax and resolution

An import is a whole line: `#import <clause>`. The clause is kept verbatim, including any braces
and dots. Three shapes are in use:

```wgsl
#import planet_x::light::sun_light
#import planet_x::noise::{fbm_3, fbm_3_grad, rotate_vector, NoiseSample}
#import bevy_pbr::mesh_view_bindings::{view, depth_prepass_texture, globals}
```

Resolution (`module_of`) is: an exact whole-string match against the module table wins; otherwise
the **longest `::` prefix** that is a module name wins. So `planet_x::noise::{…}` and
`planet_x::light::sun_light` both land on their module, and a clause that names the module itself
(`#import planet_x::common`) resolves the same way — the resolver has exactly one rule.

A clause that resolves to nothing is an **external symbol**. It is recorded verbatim as the clause
string and never split: `{view}` and `{view, lights}` are different, because *which names were
asked for* is part of the shader's identity. External symbols are not an error by themselves.

## Assembly

`px_shader::assemble::render_source(source, modules, stubs, seen)` produces the text the host hands
to wgpu:

1. `#import` and `#define_import_path` lines are dropped from the body.
2. Imports are collected in order. A `::{…}` clause is split on commas and each `module::symbol` is
   expanded separately.
3. `expand` answers from the stub table if it can — the stub text is inlined once, deduplicated
   through `seen`. Otherwise the clause resolves to a module and the **entire module** is rendered
   recursively, again deduplicated. Stub dedup is load-bearing: a symbol imported by both a library
   module and the entry (for example `view`) would otherwise be inlined twice and redefine itself.
4. Every retained line has `#{MATERIAL_BIND_GROUP}` replaced by
   `px_protocol::material::MATERIAL_BIND_GROUP` (= 3, the material bind group). Line content is
   otherwise untouched.
5. The result is `prelude + "\n" + body`: all expanded imports first, then the entry body's own
   lines, in their original order.

Failure modes are deliberate. A clause containing `::` that resolves to neither a stub nor a module
is a hard error. A bare word with no `::` that is not a module expands to nothing, silently.

Because import lines leave the product, **changing the name of an imported symbol can leave the
assembled bytes identical while the closure fingerprint changes**. The two readings are separate and
are pinned separately (see the last section).

The assembler inlines whole modules; it does not inline only the named symbols. That is fine here
because the same function is the runtime assembler: `px_render::shader::assemble` and
`px_probe::common::assemble` call this function with the same roots, the same lookup rule, and the
same stub table. The text that is validated and reflected is the text that is compiled.

## The import closure and its fingerprint

A `Closure` is the **reachable** modules (name to source) plus the unresolved external clauses.
Reachable only: an unimported module is not in the closure, so editing it changes no key and forces
no re-cook.

`Closure::fingerprint()` is FNV-1a 64 (offset `0xcbf29ce484222325`, prime `0x100000001b3`):

- the namespace string `px_shader/closure/v1`;
- then, in `BTreeMap` order, each module: `chunk(name)` followed by `chunk(source)`;
- then, in `BTreeSet` order, each external clause: `chunk(clause)`.

`chunk` writes the length as a little-endian `u64` and then the bytes. The length prefix is what
stops `("ab", "c")` from colliding with `("a", "bc")`; the sorted containers are what make the
result independent of the order the `#import` lines were written in.

`modules_fingerprint()` is the same construction over the whole table (namespace
`px_shader/modules/v1`) and ignores reachability. It has no caller outside its own test today.

**Where it is recorded.** `px_graph::write_shader` writes into the artifact's manifest params:
`closure_hi` and `closure_lo` (the two 32-bit halves of the value, carried as `f64` — lossless,
because `f64` represents every `u32` exactly), plus `closure_modules`, `closure_externals`,
`wgsl_bytes`, `schema_bytes`, and `shader_version`.

**Where it is compared.** `px_render::art::load_shader` recomputes `px_shader::closure(source,
modules)` from the artifact's WGSL against the modules on disk and compares it with the recorded
value. A mismatch is refused outright, with both fingerprints and the re-cook commands; a missing
value is also refused (an artifact that never recorded a closure is not treated as zero). This
catches the case the key cannot: the library was edited, nothing was re-cooked, so the scene still
points at the old artifact while assembly reads the new library.

**The key.** `px_graph::shader_key(text, closure)` is

```
blake3("px_shader/v2" ‖ SHADER_VERSION.to_le_bytes() ‖ closure.fingerprint().to_le_bytes() ‖ text)
```

Identity is entry text plus closure fingerprint plus `SHADER_VERSION`. External symbols enter only
by name; their implementations are covered by the version number instead.

## `SHADER_VERSION`

`px_graph::SHADER_VERSION: u32 = 1`. It is an input to every shader key and is recorded both as the
manifest entry's `op_version` and as the `shader_version` param. It is the manual dial for everything
that is not in the bytes, and there are exactly two such things:

- **External symbol implementations.** `#import bevy_pbr::…` clauses enter the fingerprint by name
  only, so swapping what a symbol expands to changes the assembled text and therefore the key — but
  a change to the host contract that is not visible in the stub text is not covered by any hash.
- **Reflection rules.** `px_shader::reflect` and the `px_protocol::material` table decide what a
  given text reflects to. The descriptor is baked with the artifact, so changing the rules without
  changing the key leaves two contracts under one key. The loader catches it; the key should change
  first.

Bump it for either. The fingerprint namespaces (`px_shader/closure/v1`, `px_shader/modules/v1`) and
the key prefix (`px_shader/v2`) play the same role for the hashing rules themselves: changing what is
hashed means changing the namespace, so an old fingerprint never claims to still hold.

## Reflection with naga

`px_shader::reflect::reflect_assembled(assembled, name)` requires **assembled** text — an entry with
`#import` still in it does not parse. It parses with naga's WGSL front end and validates with all
validation flags and capabilities, then walks the module's global variables:

- Only bindings in group `MATERIAL_BIND_GROUP` (= 3) are considered.
- **Binding 0** (`PARAMS_BINDING`) is the params block. It must be `var<uniform>` and must be a
  struct; every member must be named. Member kinds: 4-byte scalar (`f32` / `i32` / `u32`),
  `vec3<f32>`, `vec4<f32>`. `vec2` is rejected (no artifact value can express it), non-4-byte scalars
  are rejected, and any other member type is rejected. The block size is the struct's span rounded up
  to `PARAMS_ALIGN` (16) and must not exceed `MAX_PARAMS_BYTES` (4096). A shader with no params block
  is an error.
- **Images** map to `TextureDimension`: `texture_2d` to `D2`, `texture_cube` to `Cube`,
  `texture_depth_2d_array` to `D2Array`; any other dimension is rejected. The depth flag comes from
  naga's `ImageClass`, never from the variable's name.
- **Samplers** are collected by binding.
- Anything else in the group is rejected — the group holds the params block, textures, and samplers.

The slots are then checked against `px_protocol::material::TEXTURE_SLOTS` (odd bindings 1..23, with a
declared dimension each). A texture on a slot outside that list is rejected; a texture whose
dimension disagrees with the slot is rejected, with one relaxation in one direction only: a `D2` slot
also accepts `D2Array` (a downsampling pass binds the previous level's depth atlas to a 2D slot).
Each texture must have a sampler at `binding + 1`; there is no matching requirement in the other
direction, so a sampler on a slot with no texture is not rejected here.

The result is `MaterialLayout { params, params_bytes, textures }`, textures sorted by binding. The
types themselves live in `px_protocol::material` because that crate's dependencies are pinned to
serde, so naga cannot live there; `px_shader::reflect` is where naga is allowed to be.

`reflect::entry_points(assembled, name)` is separate: it parses (no validation) and returns
`(name, stage)` for each entry point in declaration order, with unknown stages reported as a
descriptive string rather than a panic. It exists because reflection reads only the params block and
the texture slots and therefore cannot see whether a recipe's `entry` name points at anything.

**Consumers.** Cook side: `px_graph::write_shader` bakes the descriptor; `px_scene::frame::bake_material`
reflects frame materials. Host side: `px_render::art::reflect_source` (assemble, validate, reflect)
serves both content shaders and frame materials, and is the one place that produces the text handed
to the GPU.

**Descriptor check.** `MaterialLayout::to_json()` is written as a second blob beside the WGSL. On
load, the freshly reflected descriptor must be byte-identical to the recorded one, otherwise the load
is refused: same WGSL, different reflection rules, different contract. `MaterialLayout::pack` turns
parameter values into bytes and refuses a missing, extra, or wrongly typed name; `too_big` bounds the
result.

## The host stub table

```rust
pub type Stubs = fn(&str) -> Option<&'static str>;
```

A plain function pointer. The assembler itself knows no symbol; which `bevy_pbr::…` clauses can be
answered is entirely the caller's table. There are two tables.

**`assemble::bevy_stub`** — twelve minimal declarations, laid out after Bevy's WGSL: `VertexOutput`,
`view` (five fields), `lights`, `depth_prepass_texture`, `clustered_lights`, `globals`,
`POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT`, and three `clustered_forward::*` helpers that return
constants (no `#import` in `art/` asks for those three). Two entries are placeholders:
`fetch_point_shadow` returns `1.0`, and `depth_ndc_to_view_z` is `-1.0 / max(ndc_depth, 1e-6)`.

**`host_stubs::wgpu_host_stub`** — `bevy_stub` plus exactly three overrides:

| Symbol | Text |
|---|---|
| `bevy_pbr::mesh_view_bindings::view` | `HOST_VIEW_STUB` |
| `bevy_pbr::view_transformations::depth_ndc_to_view_z` | `-view.clip_from_view[3][2] / ndc_depth` |
| `bevy_pbr::shadows::fetch_point_shadow` | `POINT_SHADOW_STUB` |

`HOST_VIEW_STUB` is a contract, not an approximation: Bevy's five fields in the same order at the
same offsets, then `view_from_clip` and `world_from_view` appended at the end, still at binding 0.
Both inverses are computed on the host side and read out of the uniform; the shader never inverts a
matrix and never uses an analytic inverse. `DEPTH_NDC_TO_VIEW_Z` takes the near plane from the
camera matrix (`view.clip_from_view[3][2]`) rather than a literal, so a different camera cannot drift
away from it. `POINT_SHADOW_STUB` is a real implementation: eight D3D MSAA sample positions with
eight Gaussian coefficients, a virtual shadow page table (two grids — virtual cells and physical
atlas pages — read separately), one depth atlas per level, per-level kernel radius derived from the
pixel's footprint in light space, and a manual reverse-Z compare (`depth < stored`) done by
`textureLoad`, since a page's internal texel is not reachable through the comparison sampler. It
carries its own declarations for the bindings it needs, because a content shader imports only this
one symbol.

The `bevy_pbr::` prefix is kept on purpose: it is a **provenance pointer** — it names the upstream
file each declaration was laid out after — not a claim of dependency. Renaming it would replace the
one pointer to the origin with a false claim of ownership, and it would rotate every shader key in
the workspace, because external clauses are hashed by name.

Two rules hold the table together:

- **A symbol the table does not answer is an offline error.** For example
  `bevy_pbr::shadows::fetch_directional_shadow` and
  `bevy_pbr::mesh_view_bindings::directional_shadow_textures` both return `None`, so an offline gate
  reports an unknown import instead of the mistake reaching the GPU.
- **The probe must compile the text the host compiles.** `px_probe::common::assemble` uses
  `workspace_roots` + `workspace_source_of` + `wgpu_host_stub`; `px_render::shader::assemble` uses
  the same three; `px_render::stubs` merely re-exports the same text and keeps the tests that pin it.
  The probe cannot depend on the host crate — the host has no library target — so this identity is
  maintained at the level of the text, and it must stay text: a probe assembling with the placeholder
  table would measure a shader nobody executes (`clouds.wgsl` imports both `fetch_point_shadow` and
  `depth_ndc_to_view_z`).

Cook-side assembly also uses the host table: `px_graph::write_shader` → `shader_schema` assembles
and reflects with `wgpu_host_stub`, and `px_scene::frame::frame_stubs` overrides the `view` slot with
`HOST_VIEW_STUB` and falls back to `wgpu_host_stub`.

## What the tests pin byte-for-byte

`px_shader`'s `the_four_entry_shaders_assemble_to_these_bytes_under_the_wgpu_host_table` pins four
content entries under the host stub table:

| Entry | Assembled bytes | FNV-1a of assembled text | Closure fingerprint |
|---|---|---|---|
| `atmosphere.wgsl` | 9528 | `b22207bbe62eb0fa` | `7c31cc927eabed01` |
| `clouds.wgsl` | 71282 | `dc87a63951832035` | `26d97a072b269596` |
| `ring.wgsl` | 32321 | `1127586ad87a3b72` | `8c39408dfc5e5e7b` |
| `surface.wgsl` | 45290 | `9bf5f88d031e3f69` | `8865fd2aed6d4b64` |

The byte count is what goes to `create_shader_module`. The FNV column is plain FNV-1a over the
assembled bytes (no namespace, no length prefixes), so it is a different quantity from the closure
fingerprint. The closure column is the value that enters `shader_key`, and it moves when an external
symbol's *name* changes even if the assembled bytes do not. `ring.wgsl` is the negative control: it
imports one external symbol (`VertexOutput`), whose text is identical in both tables.

`the_workspace_convention_points_at_the_real_shader_roots` pins the convention itself: each `.wgsl`
collected once, every library file carrying `#define_import_path`, no entry file carrying one, the
three library modules present, `clouds.wgsl` resolving to `art/shaders/clouds.wgsl` and containing
`#import planet_x::light::sun_light`, and an unknown name returning `Err`.

The GPU-free offline gate is `px_render --shaders`: it assembles and naga-validates the four content
shaders with the host stub table, prints each assembled size and every global variable's
`(group, binding)`, and exits non-zero if any of them fails (see [verdicts.md](verdicts.md)). For
content shaders that table is the group 0 contract; it is read from the shader rather than restated
in Rust, because the shader is the only source for it.

## Frame shaders (`art/frame/`)

`art/frame/vertex_mesh.wgsl` (mesh vertex stage), `art/frame/vertex_sky.wgsl` (full-screen sky vertex
stage; three vertices derived from `vertex_index`, only `@builtin(position)` out) and
`art/frame/skybox.wgsl` (frame fragment stage that rebuilds the ray direction from the viewport and
samples the cube map; its only import is `bevy_pbr::mesh_view_bindings::{view}`) are the frame's own
stages. They are **not** in `art/shaders`, are not in the module table, and never enter the CAS as
content: they belong to the frame recipe (`art/frame/<name>.toml`, default `default.toml`) and are
named there by path.

They are still assembled and reflected — at cook time, by `px_scene::frame::bake_material`, using
`frame_stubs`. Four things are checked, all at cook time:

- the recipe's parameter sources are known to the WGSL;
- the WGSL declares no parameter the recipe fails to supply;
- the declared type matches the supplied value;
- the recipe's `entry` really names a **fragment** entry point of that text — reflection cannot see
  entry names, and the error lists the entry points that do exist.

The vertex stages are inlined as full text into the document, so they have no member name to refer to;
`px_render::shader::watch_files` adds `art/frame` to the hot-reload scan anyway, and which slot
actually needs reloading is decided by comparing assembled text byte for byte, not by file timestamps.
