# The renderer

Two subjects: the wgpu host that draws pixels (`px_render`) and the pass executor (`px_pass`) that
performs the draws. The scene-recipe language that feeds them is in [art.md](art.md); shader assembly
and reflection are in [shaders.md](shaders.md).

## The host's entry points

`px_render` is one binary with several modes over the same renderer. The render paths consume a
compiled scene artifact and generate no content. The one place a recipe is read is the `--edit` panel,
and it reads it only to enumerate the graphs the recipe references.

| Mode | Command shape | What it is for |
|---|---|---|
| offline shot | `px_render --offline --scene <artifact> --out <png>` | one image in this process: no window, no service, no protocol, no surface. The verdict images come from here |
| client request | `px_render --scene <artifact> --out <png> [--report r.json] [--autostart]` | hands one request to a running service over the socket. Without `--offline`, this is what `--scene --out` means, and this is how the instruments drive the host |
| resident service | `px_render --serve [--port N] [--pcg-root DIR]` | a long-lived process answering requests, plus a lease file so a client can find it |
| preview window | `px_render --view [--scene <artifact>] [--cam Y,P,D] [--shot PNG] [--novsync] [--edit <recipe>] [--ui-shot PNG] [--image-hash]` | resident window, redrawn when the camera, the scene, or the size changes |
| device probe | `px_render --device [--shot PNG]` | builds instance, adapter and device, then optionally writes one solid-colour image |
| shader gate | `px_render --shaders` | assembles the content shaders against the host's stub table and validates them; no GPU |
| image diff | `px_render --diff A.png B.png` | compares two PNG files pixel by pixel; no GPU |
| window calls | `px_render --show --scene <artifact> [--shot PNG]`, `--where`, `--place Y,P,D` | push a scene to a running window, ask its camera, or place it; none of these renders in this process |

`--shot` never opens a window. Given alone, or with `--device`, it writes the solid-colour clear,
through the same PNG path every other shot uses — that is the windowless writer. In the window path it
writes the window's **first** frame. `--ui-shot` writes what a person sees: the frame plus the panel,
read back from the swapchain. `--image-hash` is a window log line: each frame prints the sha16 of the
read-back bytes and a wall-clock timestamp.

`--sheet` renders the review contact sheet from the camera table carried in the artifact. Cameras go
in row-major order, `columns` comes from the request (the command line defaults it to 4, and 0 is
floored to 1), the sheet is `(cell width × columns) × (cell height × ceil(cameras / columns))`, and
each camera's projection uses the **cell's** aspect, not the sheet's. `--cam` with `--sheet` is refused
locally: the camera table and `--cam` are two sources that would drift apart.

`--view`, `--show`, `--where` and `--place` are four different things, and only one of them may be
given at a time. Each is refused together with `--serve` (two resident paths cannot be one process)
and with `--offline`. `--show` does not render: it writes `target/viewer-scene.json` and the window
picks the scene up. `--where` and `--place` exchange one request and one reply with the window's camera
through the same files and wait up to 3 s for the answer.

A flag that belongs to one path is normally refused with exit 64 when it is given to another, rather
than silently ignored:

- `--stats`, `--time` and `--spans` are offline readings, and on the client path they are refused:
  the service draws one frame per request, so there is no frame sequence to measure there.
- `--report` is produced by the service (digests, grid differences and placeholder counts are all the
  server's); the offline path points at `--stats` instead.
- `--edit` and `--image-hash` live in the window and are refused without `--view`. `--ui-shot` is read
  only by the window path.
- `--pcg-root` names the CAS root of the process that resolves members, so it is read by the resident
  render processes (`--serve`, `--view`). A client that was given it reports that the content is
  resolved by the serving process' own root.

Exit codes are part of that interface: `0` success, `1` a render, IO or bring-up failure, `2` a
backend assertion (the message starts with the fixed prefix `gpu::BACKEND_ASSERT`, which is how the
harness fails fast instead of waiting out a timeout), and `64` a usage or capability refusal — an
unknown flag, a flag that does not apply to the chosen path, a missing `--out`, or a device feature
the requested reading needs.

## What loading an artifact validates

An artifact is not trusted blindly. On load the host checks:

- **the scene schema** against `SCENE_SCHEMA` (3).
- **the import closure fingerprint** of every shader in the artifact against the modules on disk. A
  mismatch means the artifact was assembled from a different set of shader text, and is refused with
  the commands needed to re-cook it.
- **the schema descriptor** baked into the artifact against the reflection rules in force, compared as
  canonical JSON. This catches "same key, two contracts", which would otherwise silently mis-bind
  parameters.
- **parameter completeness and types**, in both directions: a parameter the shader declares but the
  artifact does not supply, and an extra parameter the shader does not declare, are both refused.
- **textures** against their declared shape: `rgba8_srgb` must carry `U8` and `rgba16_float` must
  carry `U16`, the payload length must equal the whole mip chain, and a texture whose layer count does
  not match the dimension of the slot it is bound to is refused too.
- **geometry**: the only primitive is `icosphere`, taking a `radius` and an integer `subdivisions` in
  `1..=64`. Everything else comes from a mesh member.

`SHADER_VERSION` is not part of this: it is an input of the **bake-time** shader key, so a change to
the reflection rules changes the key of everything baked afterwards. Enforcing the same rule at load
time is the schema-descriptor comparison above.

The skybox texture's sampler is `Sampler::clamped()` — the skybox is not a material slot, and
`Sampler::clamped()` is not `Sampler::default()`: they differ on the u axis (`ClampToEdge` against
`Repeat`), and cube-face edge texels land on that difference.

## Materials and bind groups

A material's parameters are packed into a uniform block whose layout is derived from the reflected
descriptor, not from a hand-written table. The rules that matter when extending anything:

- **Slots are append-only.** A new texture slot goes after every existing one, and a new parameter
  after every existing parameter. Moving an existing slot silently repoints an existing shader's
  texture at a different binding.
- **Packing is defined once, in `MaterialLayout::pack`.** It is the only place that turns a named
  parameter map into the uniform bytes (through `write_value`). The recipe side has a separate mapper,
  `px_scene::contract::coerce_value`, which turns a TOML value into the artifact's `Value` according
  to the declared `ParamKind`. Both read the same declared kinds; neither is a second packing rule.
- **The layout does not depend on the instance.** It is a fixed superset, and empty slots are bound
  with a fallback texture, so a shader that declares a slot always has something bound there.

Bind groups, by index:

| Group | Contents |
|---|---|
| 0 | the frame: `view` (0,0, which also carries `exposure`), `lights` (0,1), the shadow atlas and its comparison sampler (0,2) and (0,3), the shadow page table (0,4), the page offsets (0,5), the shadow face basis (0,6), clustered lights (0,8), `globals` (0,11), the depth-prepass texture (0,20), the mesh-instance array (0,21), and the three coarser shadow atlases (0,22)–(0,24) |
| 1 | the geometry pass' own parameters at binding 0 — the vertex stage's `PassView` (`view_proj` plus the page rectangle) |
| 2 | empty |
| 3 | the material: the parameter block at binding 0, and the texture slots of the contract table, with each sampler one binding above its texture |

Group 3 is `MATERIAL_BIND_GROUP`, a fixed number that the assembled content shaders already contain.
Group indices are positions in the pipeline-layout array, so the empty group 2 is not an accident to be
tidied away.

Pass shaders declare the same `var<uniform> params` block as material shaders and take textures from
the slots their own contract declares. The executor builds its bind group from the slot list the
caller hands it, so no second copy of the contract table exists.

## The pass executor

`px_pass` executes a pass table against a set of render targets. It is deliberately not part of the
host: it knows about wgpu resources and pipelines, and nothing about scenes, materials-by-name, or
content.

Three kinds exist:

- **fullscreen**: one shader and one entry point supplied by the pass, the executor's own full-screen
  triangle for the vertex stage, a parameter block packed from that shader's declared struct, and
  `reads` mapped onto the texture slots the shader declares. The pass must declare no draws, its
  parameter block must be a non-zero multiple of the layout alignment, and it must have as many slots
  as reads.
- **geometry**: the vertex stage is the document's `vertex_shader` / `vertex_entry`; the fragment stage
  belongs to the material of each draw, so the pass' own `shader` and `entry` columns must be empty.
  `reads` and `slots` must be empty as well — the host resolves material textures into group 3 and the
  executor must not carry an unverified declaration of them. A parameter block is allowed, and it is
  what a per-face view or a per-page rectangle rides in.
- **copy**: exactly one read and one write, no attachments, no shader, no entry, no parameters, no
  draws. It opens no render pass at all — it records a single `copy_texture_to_texture`, and both ends
  must agree on format and size rule, with opposite usages (`copy_src` and `copy_dst`).

**Compute passes are refused.** The executor has fullscreen, geometry and copy, and no compute path: a
declared but unimplemented kind is refused where it is written rather than skipped silently.

An empty pass table draws **nothing**. `Plan::check` runs first; execution then returns early saying
the pass table is empty, and the host synthesises no main pass. Such a table also serialises to the
same bytes as a document with no pass section at all, so the two spellings are byte-identical on disk
while behaving the same way: nothing is drawn.

Pipelines are created on demand and cached by the executor under a key that contains everything the
pipeline is built from:

- fullscreen: `fullscreen|<fnv1a(shader)>|<colour format>|<entry>|<reads>|<layout key>|<render states>`
- geometry: `geometry|<fnv1a(vertex wgsl)>|<fnv1a(fragment wgsl)>|<vertex entry>|<fragment entry>|<vertex layout or "procedural">|<colour format>|<render states>|<group layout ids>`

Every pipeline is created with `cache: None`: there is no wgpu `PipelineCache` in the process, so
content hashing is the caller's job and those keys are the whole of the caching. The layout key covers
the group, the parameter binding and the binding and dimension of every slot; the render states cover
the states the pipeline actually reads.

`px_pass` exports `VERTEX_ENTRY = "px_fullscreen_vertex"`: the fullscreen vertex stage is fixed and
used by every fullscreen pipeline. The fullscreen **fragment** entry comes from the pass. The crate
also exports a `FRAGMENT_ENTRY = "fs_main"` constant, which no execution path reads; the host's
material path has an entry name of its own.

## The frame graph

A scene document carries, beyond objects and materials:

- **targets** — intermediate resources declared once with a format, a size rule, a layer count and a
  usage list, then referenced by name from passes.
- **passes** — an ordered list; array order *is* execution order.
- **frame-owned materials** — materials that belong to this renderer rather than to swappable
  content (the skybox is the example). Their text is inlined in full because changing one requires
  re-cooking.
- **material instances** — a new name for an existing material, resolved through its `base`. The frame
  compiler emits this list empty: a per-face or per-page view travels in the pass' own parameters and
  viewport instead. The host still implements the instance path (one name resolves to exactly one set
  of groups, and the face comes from the pass that uses the name), so that contract is live code with
  no producer today.

The frame graph itself is one shared definition, `art/frame/<name>.toml`, selected by name from the
scene recipe and defaulting to `default`. It is the renderer's shape, not content: it declares the
frame's resources and its pass order (depth prepass, the point-shadow passes for each level, a depth
copy, opaque, sky, transparent, blit, with content passes inserted before the blit). It is baked into
the artifact's resources and pass table, so the host never reads it.

## Shots and reports

A shot request produces images plus a structured `Report`. The report carries the protocol
`schema_version` and `protocol_hash`, the job name, the size of the last image, the wall clock, and one
`ShotReport` per image. A `ShotReport` has exactly these fields:

| Field | Meaning |
|---|---|
| `scene`, `label`, `out` | the artifact path, the audit text, the PNG path |
| `width`, `height`, `bytes`, `sha256` | the written image's size and digest |
| `placeholder_px` | pure magenta pixels (`r > 250 && g < 8 && b > 250`) — the placeholder shader's fingerprint |
| `bright_px` | pixels with `r + g + b > 72` |
| `diff_vs_ref_grid` | the grid difference against the reference: over a 16×10 grid, the sum of the absolute differences of each cell's `r + g + b` sum |
| `declared_clouds` | the artifact's `expects` list contains `clouds` |
| `has_cloud`, `verdict` | the availability verdict |

Grid cells are addressed with integer division only: row `y * 10 / height`, column `x * 16 / width`.
There is no artifact key in a shot report; a key lives on the performance report.

The availability rules: the reference image is the **first shot of the batch**, so a batch is meant to
put the cloudless variant first. `has_cloud` holds when the artifact declares clouds, there is no
magenta placeholder pixel, and either the image *is* the reference and has at least one bright pixel,
or its grid difference is at least 1% of the reference's total `r + g + b` (with a floor of 1, so an
all-black reference cannot make the test vacuous). Any placeholder pixel short-circuits the verdict to
"this image does not count", because a placeholder shader says nothing about the content at all.

Images are read back from an `Rgba8UnormSrgb` render target of the host's own; the swapchain is never
read, because it only guarantees `Bgra8Unorm(Srgb)` with `RENDER_ATTACHMENT`, and a BGRA buffer fed to
an RGBA PNG swaps red and blue. Readback pads each row to `COPY_BYTES_PER_ROW_ALIGNMENT` and strips the
padding afterwards. PNGs are written as `DynamicImage::to_rgb8()`, which drops alpha.

`--diff A.png B.png` compares two **PNG files** pixel by pixel and reports the difference per region:
differing pixel count, maximum and mean channel difference, the bounding box, and whether the
silhouette's pixels are identical. Both readings are positional: the background colour and the
silhouette are defined by the *first* argument. The tool names both paths with their side and never
guesses which one is the reference.

## The wire protocol

Client and server exchange four frame kinds — `protocol`, `request`, `response`, `refused` — as JSON
text prefixed with a `J` byte, inside an envelope of a `u32` little-endian length followed by that many
payload bytes (the length counts the `J`). The handshake exchanges a `ProtocolId`: the protocol schema
version, the FNV-1a hash of the protocol snapshot, and the git revision. A client talks to a peer only
if the lease and the peer agree on the schema version and the hash.

The service lease is `target/render-server.json`; it holds the pid, port, protocol hash, git revision
and executable path, and a client starts a server only when `--autostart` is given. The preview window
uses a different mechanism: `target/viewer-scene.json` carries the pushed scene, `target/viewer.json`
is the window's heartbeat (refreshed once a second; a lease older than 5 s means no window is running),
and `target/viewer-camera.json` carries the camera reply. Those three files are the whole interface
between the window and the `--show` / `--where` / `--place` entries; the window never goes through the
service protocol.

## Profiling

`--spans <warmup>,<measure>,<rounds>` reports per-pass GPU timestamps. All three numbers are mandatory:
how many frames are warm-up, how many are measured, and how many interleaved rounds — together they say
what conditions the reading was taken under. Two values are refused. The flag is offline-only, is
mutually exclusive with `--sheet` (the timestamp slots are laid out for a single image), and is refused
when the device lacks `TIMESTAMP_QUERY`, `TIMESTAMP_QUERY_INSIDE_ENCODERS` or
`TIMESTAMP_QUERY_INSIDE_PASSES`, rather than degrading into a reading with different boundaries. Each
document is opened once, and every round then draws all documents in turn, so warm-up frames and
interleaving are part of the instrument. What it reports is a per-pass encoder-level span set, and it
is explicitly not `gpu_ms`.

The host renders on demand: one request draws one frame and replies. It has no frame loop and no
per-frame sampling, so a request shape that needs those is refused up front with the reason instead of
timing out. The instruments in `tools/` drive the host as a client over the service.

## The parameter panel

`px_render --view --edit <recipe>` puts an editable panel over the preview.

- The panel lists every graph the recipe references, every node in those graphs, and every scalar leaf
  in `art/<graph>/*.toml` — including a file in that directory that the graph does not reference. Such
  a file is listed, saved and cooked, and the picture does not move; the cook output says that no node
  was recomputed.
- Edits are written to a **session copy** under `target/pcg/edit/`, never to `art/`. Save copies the
  changed files back to `art/`, and only the changed ones.
- Cooking runs `px run` as a **subprocess**: once per graph with `--store target/pcg/edit` in the order
  the recipe references them, then once more as `px run scene <recipe>` **without** `--store`. The
  scene graph's parameter directory is `art/scene/` and is not part of the session copy, so pointing it
  at the copy would silently fall back to defaults. Output is streamed to the panel unchanged.
- Which recipe is being edited is resolved by comparing the artifact's computed CAS path against the
  `target/pcg/scene/manifest.json` entries, and failing that by scanning `art/scene/*.toml` for the
  file whose `name` field equals the artifact's **file stem**. Without `--edit`, the panel derives the
  recipe the same way for the scene the window is showing. The artifact's file name alone is never
  used as a recipe name: a CAS file is named after a content key.
- The panel is drawn onto the swapchain after the frame bytes are uploaded and before the swapchain is
  presented, as a second pass in the same command buffer. It therefore cannot appear in `--shot`
  output and cannot change an image hash.
