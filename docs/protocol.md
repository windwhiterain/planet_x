# The protocol (`px_protocol`)

`px_protocol` is the data contract of the engine: the artifact wire format (`.pxart` /
`.pxstream`), the material-binding contract, the scene document, and the envelope the render host
and its clients speak. It is **types plus serde**, plus the file-level readers and writers around
them (path in, bytes out) and a few shape helpers (cube / octahedral mapping, volume extents),
FNV-1a, and the two-level sparse volume index. It holds no cryptographic hashing (CAS keys are
blake3, computed by callers), no GPU code, no scene or recipe logic, and no policy about where
artifacts live.

Two declaration families live here, with different audiences:

| Family | Modules | Boundary |
|---|---|---|
| artifact and document contract | `art`, `wire`, `stream`, `payload`, `material`, `scene`, `sparse`, `fnv` | px-scene ⇄ px-pass: what a cooked artifact means |
| host ⇄ client | `render`, `frame`, `client` | the render service and the tools driving it |

The host layer is here rather than in the host for a build reason: the snapshot test constructs
those shapes from the **real types**, so if they lived in the host, the protocol test build would
pull in wgpu / naga / winit. `tests/crate_graph.rs` enforces that the host is not a dependency of
this crate — runtime or dev.

Two different types are called `Frame`: `px_protocol::Frame` is the **stream** frame (five
variants, `stream.rs`), `px_protocol::frame::Frame` is the **host** envelope (four variants). They
are separate formats with separate encoders and no shared code.

| Module | Contents |
|---|---|
| `art` | `AssetKind`, `AssetManifest` / `ArtBundle`, `VolumeData`, `MeshData`, `PolylineData`, `TextureData` / `TextureShape` / `TextureFormat`, `Camera`, `Domain` + projection helpers, artifact diffing, artifact readers |
| `wire` | `DType`, `BlobHeader`, `Blob`, `WireError` — the blob encoding shared by everything |
| `stream` | `MAGIC`, `STREAM_VERSION`, `Frame`, `read_stream` / `write_stream`, `declared_protocol` |
| `payload` | `PayloadBundle` (the shape of a CAS file: manifest frame + blobs) and `payload_fingerprint` |
| `material` | `MATERIAL_BIND_GROUP`, `PARAMS_BINDING`, `TEXTURE_SLOTS`, `MAX_PARAMS_BYTES`, `PARAMS_ALIGN`, `ParamKind`, `ParamSlot`, `TextureSlot`, `MaterialLayout` |
| `scene` | `SceneSpec` and every type inside it, `SCENE_SCHEMA`, `SHADOW_FACE_BASIS`, `cas_path`, `read_scene` / `scene_bytes` / `write_scene` |
| `sparse` | `SparseVolume`: two-level occupancy plus linear `u16` quantization |
| `fnv` | `FNV_OFFSET`, `FNV_PRIME`, `fnv1a`, `fnv1a_bytes` |
| `render`, `frame`, `client` | request / response / report / lease shapes, the host envelope, the client half (lease → connect → handshake → request) |

`src/rows.rs` sits in the crate directory but is **not a module of the crate**: `lib.rs` leaves the
`pub mod rows;` line out, so nothing in that file is compiled.

## The blob encoding

Everything binary travels as a `Blob`: one JSON header line, a `\n`, then the raw payload.

```text
BlobHeader = {"dtype":"F32","shape":[2,2]}   ← compact JSON, one line
\n
<elems × dtype.elem_size() bytes>            ← little-endian, no padding, no alignment, no length field
```

| `DType` | element size |
|---|---|
| `U8` | 1 |
| `U16` | 2 |
| `F32`, `U32` | 4 |
| `F64` | 8 |

`BlobHeader::elems()` is the product of `shape`; `byte_len()` is `elems × elem_size()`. A shape
dimension of 0 makes the whole blob empty. `Blob::new` refuses any payload whose length is not
exactly `byte_len()`, which is what makes a blob self-describing and self-checking: the reader
never has to trust an outer length. `Blob::decode` splits on the **first** `\n`, parses the header,
and hands the rest to that same check, so a wrong length is `BadPayloadLength` and a missing header
line is `MissingHeader`. Payload bytes are never reordered:

```text
F32 → f32::from_le_bytes per 4 bytes        U32 → u32::from_le_bytes per 4 bytes
```

`Blob::from_f32` / `from_u32` are the constructors used by payload encoders; `from_f32` also
`debug_assert!`s that `shape` holds exactly the values given.

### `float_roundtrip` is not optional

Every crate that parses these documents enables `serde_json`'s `float_roundtrip` feature, and each
manifest names it explicitly instead of relying on Cargo feature unification. The reason is that
the default float parser is not correctly rounded: parsing and re-serializing can change the last
bit of an `f64`. Artifact identity is byte-derived, so a value that does not survive a read-write
round trip produces a different key for unchanged content. With the feature on, read-then-write is
byte-identity, which is the property the whole cache rests on.

## The frame stream

```text
PXST                       magic, 4 ASCII bytes
u32 (little-endian)        STREAM_VERSION = 1
[frame]*                   each frame: u32 LE length, then `length` payload bytes
```

A frame payload starts with a one-byte tag:

| First byte | Payload | JSON tag |
|---|---|---|
| `B` | `Blob` = header line + `\n` + bytes | — |
| `J` | compact JSON with a `"frame"` tag | `protocol`, `art`, `scene`, `refused` |

`Frame` variants (`px_protocol::Frame`):

| Variant | Payload |
|---|---|
| `Protocol(ProtocolId)` | handshake identity of the writer |
| `Art(ArtBundle)` | the manifest: asset ids, `params`, blob headers, payload fingerprints |
| `Scene(SceneSpec)` | the scene document (see below) |
| `Blob(Blob)` | one payload block |
| `Refused(String)` | refusal reason; the JSON key is `reason` |

Readers: `write_stream` / `read_stream` (whole stream), `write_frame` / `read_frame` (one frame at a
time, `Ok(None)` at end of input), `declared_protocol(path)` (the first `Protocol` frame in a file).

### Failure behaviour

| Input | Result |
|---|---|
| fewer than 4 bytes at the start | `WireError::Io` — `read_exact` fails before the magic can be compared |
| 4 bytes, not `PXST` | `WireError::BadMagic` |
| magic ok, version bytes truncated | `Io` |
| version ≠ 1 | `WireError::BadStreamVersion(v)` |
| input ends exactly at a frame boundary | clean end |
| 1–3 trailing bytes where a length prefix should start | clean end — treated as end of stream, not an error |
| complete length prefix, payload shorter than declared | `Io` |
| payload starts with neither `B` nor `J` | `TruncatedFrame` |
| `B`, header line missing | `MissingHeader` |
| `B`, header not JSON / payload UTF-8 invalid | `Json` |
| `B`, declared length ≠ actual byte count | `BadPayloadLength { expected, actual }` |
| `J`, unknown `"frame"` tag or malformed JSON | `Json` |

Truncation is deliberately not a silent end: only a clean read boundary ends a stream. The
container's own version (`STREAM_VERSION`) and the protocol's (`SCHEMA_VERSION`) are independent;
a stream with an unsupported container version is refused before any frame is decoded.

## Payloads and artifact shape

A cooked artifact is `ArtBundle { assets: Vec<AssetManifest> }`; almost every artifact has exactly
one asset.

```text
AssetManifest { id: String, params: BTreeMap<String, f64>, blobs: Vec<BlobHeader>, fingerprint: u64 }
```

- `id` is the node name (the graph node that produced it). Diffing pairs assets by `id`.
- `params` is the manifest: everything about the payload that is not expressible in the blob
  header. Values are `f64` only; integer facts (counts, sizes, format codes) are stored as integral
  `f64` and read back with explicit checks.
- `blobs` is the **headers only**; the bytes follow as `Blob` frames or as CAS payload.
- `fingerprint` is FNV-1a over the payload bytes with the id mixed in first; `0` means "not
  recorded". The diff uses it to distinguish "same params, different values" from "unchanged".

`AssetKind` names the kinds of payload. It is a plain Rust enum (serde names are the variant names;
there is no `rename_all`), and **no artifact field carries it today** — no writer in the workspace
writes a kind, and the readers (`px_render`'s mesh and texture loaders) recover the payload type
from the manifest params plus the blob count and shape. The enum is still the closed list of kinds,
and the snapshot test matches on it exhaustively, so adding a variant does not compile until that
test is updated.

| `AssetKind` | Payload type | Shape on the wire |
|---|---|---|
| `Field2D`, `OctahedralField`, `CubeField`, `CubeMap` | `px_field_schema::Field` | one `F32` blob `[height, width]`; the projection is the manifest param `projection` (a `Domain` code) |
| `Mesh` | `MeshData` (here) | four blobs in this order: `positions` `F32 [n,3]`, `normals` `F32 [n,3]`, `uvs` `F32 [n,2]`, `indices` `U32 [3·triangles]` |
| `Instances` | — | no payload type, no producer and no reader in the workspace |
| `Volume` | `VolumeData` (here) | one `F32` blob `[6, layers, res, res]` (a fifth dimension `lanes` appears only when `lanes > 1`); `res` / `layers` / `inner` / `outer` are manifest params |
| `VoxelField` | `Field` with `Domain::Volume` | one `F32` blob `[res·layers·6, res]`: a whole 3D volume folded into one 2D grid, the third dimension in `height` |
| `Scene` | `SceneSpec` | a `scene` frame (JSON text), not a blob |
| `Shader` | WGSL text + `MaterialLayout` | two `U8` blobs: `[0]` the WGSL entry text, `[1]` the schema descriptor as canonical JSON |
| `Texture` | `TextureData` (here) | one blob whose dtype follows the format (`U8` for `rgba8_srgb`, `U16` for `rgba16_float`); the whole mip chain, bytes unmodified; shape is in manifest params |
| `StarField` | `px_sparse::StarField` | `F32` blob `[n, 8]` (position xyz, brightness, rgb tint, one unused) plus five `U32` index blobs; grid parameters are manifest params |

`Volume` and `VoxelField` are separate kinds on purpose: both are `F32`, but one is a 3D grid in
cube-sphere parameter space with the radius in the manifest and the other is an ordinary 2D field
that happens to describe a volume. Folding them together would make the domain unrecoverable from
the payload, and the domain is what decides where a cell sits in the world.

### `VolumeData`

```text
VolumeData { res, layers, inner, outer, lanes, data }
```

- Layout for `lanes == 1`: `data[((face·layers + layer)·res + t)·res + s]`, `6·layers·res·res` values,
  `layer 0` at `inner` and `layers−1` at `outer`.
- For `lanes > 1` the data is interleaved per voxel: `lane_slot(voxel, lane) = voxel·lanes + lane`.
  `lanes` is 1 for density and 6 for emission (`[emission R,G,B, σ_R,σ_G,σ_B]`).
- `lanes` is encoded in the **blob shape** (fifth dimension), not in the manifest, so that "how many
  channels" is checkable against the byte count. A 4-dimension shape means one channel; old
  single-channel payloads stay readable.
- Single-channel-only readers go through `expect_single()` and `at()`, both of which assert on a
  multi-channel volume rather than silently reading the wrong stride. `emission_at(voxel, channel)`
  is the multi-channel accessor.
- `Default` is a one-channel empty volume (`lanes = 1`), not `lanes = 0`.

### `MeshData` and `PolylineData`

`MeshData` is four parallel arrays (`positions`, `normals`, `uvs`, `indices`); `from_blobs` requires
at least four blobs and checks `normals.len() == 3·vertices` and `uvs.len() == 2·vertices`, so a
truncated or mis-ordered set of blocks is refused instead of producing a mesh with the wrong
attribute stride.

`PolylineData` is `positions` `F32 [n,3]` plus `indices` `U32 [2·segments]` — segments, not
triangles. It is not a degenerate mesh: it has no normals and no area, so `MeshData` cannot
represent it honestly. It is not an `AssetKind`; it is the curve operator's payload.

### `TextureData`

```text
TextureData { width, height, layers, levels, format, bytes }
```

- `layers` is 1 (2D) or 6 (cube). `levels` counts mip levels including the finest.
- `format` is `rgba8_srgb` (code 0, 4 bytes/texel, sRGB sampling) or `rgba16_float` (code 1,
  8 bytes/texel, linear). The codes are what the manifest stores.
- `chain_bytes()` is the sum over the mip chain, halving each dimension per level and clamping at 1.
  `TextureData::new` asserts `bytes.len() == chain_bytes()`, so a short mip chain or a wrong depth
  fails at construction.
- `TextureShape::from_params` requires integral `width` / `height` / `layers` / `levels`,
  non-zero dimensions, `layers ∈ {1, 6}` and `levels ≥ 1`, and an unknown `format` code is an error.

### Frames, projections and extents

`Domain` codes are frozen and travel in the manifest param `projection`:

| `Domain` | code |
|---|---|
| `Equirect` | 0 |
| `Octahedral` | 1 |
| `Cube` | 2 |
| `CubeMap` | 3 |
| `Volume` | 4 |

Changing a code reinterprets every existing artifact, so the numbers are part of the format.
`Domain::Volume` has no "one direction" per cell: `direction_at` panics for it, and the world-point
mapping lives with the volume shape (`px_field_schema::volume`). `uv_of` returns the face-local
`(s, t)` for that domain, because the radial layer comes from the row number.

Geometry helpers shared by both sides: `CUBE_FACES = 6`, `CUBE_COLUMNS = 3`, `CUBE_GUTTER = 2`;
face order is `+X −X +Y −Y +Z −Z`; `cube_cell_size(w) = w/3`, `cube_face_size(w) = cell − 2·gutter`,
`cube_map_extent(f) = (f, 6f)`, `volume_extent(res, layers) = (res, res·layers·6)` with
`volume_layers(height, res)` as its inverse. `direction_at` / `uv_of` / `cube_direction` /
`cube_face_of` / `octahedral_direction*` are the shared parameterizations; artifacts depend on them,
so they are as frozen as the format itself.

### `PayloadBundle` and the CAS path

`PayloadBundle { params, blobs }` is the shape of one file in the CAS. `to_bytes(id)` writes a
manifest frame followed by one `Blob` frame per blob; `from_bytes` requires a manifest frame and at
least one blob frame, and returns params plus blobs **without** a type — the caller knows the type
statically. `payload_fingerprint(id, blobs)` is FNV-1a seeded with the id, then fed every payload
byte.

The path rule is also here, because both sides must agree on it:

```text
cas_path(root, key) = root/ab/<first 2 hex>/<key>.pxart     key must be exactly 64 hex digits
Member::resolve(root) = cas_path(root, key), error if the file is absent
```

## The material contract

The material's parameter block and its texture slots have one source: this module. Reflection
(`px_shader::reflect`, naga-based) produces a `MaterialLayout` from the shader text; `pack` turns
artifact values into bytes using that layout.

| Constant | Value | Meaning |
|---|---|---|
| `MATERIAL_BIND_GROUP` | 3 | the bind group index the host uses for materials |
| `PARAMS_BINDING` | 0 | binding of the `var<uniform>` parameter struct |
| `PARAMS_ALIGN` | 16 | alignment of the parameter block |
| `MAX_PARAMS_BYTES` | 4096 | upper bound on the parameter block |

`TEXTURE_SLOTS` is a fixed superset: a texture occupies an odd binding, its sampler occupies
`binding + 1`, and slots that a material does not use are still bound (to the fallback texture /
sampler) so that any declared slot resolves. The layout cannot depend on the instance, so the table
may only grow **by appending**.

| Dimension | Bindings |
|---|---|
| `D2` (8 slots) | 1, 3, 9, 11, 13, 15, 17, 19 |
| `Cube` (4 slots) | 5, 7, 21, 23 |

`TextureDimension::layers()` gives 1 for `D2` and 6 for `Cube` and `D2Array`. Emptying or moving an
existing slot would silently point an existing shader's texture at a different binding, which is why
the first four entries stay where they are.

### `ParamKind` ↔ `Value`

`Value` is untagged JSON with four forms: `Num(f64)`, `Text(String)`, `Triple([f32; 3])`,
`Quad([f32; 4])`. The **shader** decides which form a parameter may take; this is the only place
where the two vocabularies are related:

| `ParamKind` | accepted `Value` | bytes |
|---|---|---|
| `F32` | `Num` (cast to `f32`) | 4 |
| `U32` | `Num` with zero fraction, `0 ≤ n ≤ u32::MAX` | 4 |
| `I32` | `Num` with zero fraction, `i32::MIN ≤ n ≤ i32::MAX` | 4 |
| `Vec3` | `Triple` | 12 |
| `Vec4` | `Quad` | 16 |

`Text` is never accepted by any kind: a value that no shader declaration can consume is a mistake,
not a fallback. `MaterialLayout::pack` reports all three failure classes by name — a parameter the
artifact supplies but the shader does not declare, one the shader declares but the artifact omits,
and a type mismatch. Values are written little-endian at the offsets the layout declares; the buffer
is `params_bytes` long and any byte the layout does not cover stays zero.

`MaterialLayout { params: Vec<ParamSlot>, params_bytes, textures: Vec<TextureSlot> }` is also the
**schema descriptor** baked next to the shader: `to_json()` is compact, newline-free, and stable
field order, so the same layout always serializes to the same text. The loader re-reflects the
shader from disk and compares against the descriptor; a mismatch means the artifact's contract was
produced by a different reflection rule, and it is refused.

`SHADER_VERSION` is **not defined here**. It lives in `px_graph` (`px_graph::SHADER_VERSION`,
currently 1) and is mixed into the shader key. Because the descriptor is baked with the artifact,
changing the reflection rules or the tables in this module without bumping `SHADER_VERSION` would
leave two contracts under one key; the load-time descriptor check catches it, but the correct
order is: change the rule, bump `SHADER_VERSION`, re-cook.

## The scene document

A scene artifact is a manifest frame plus a `scene` frame. `SceneSpec` is a general rendering
document — no notion of "planet", "clouds" or "atmosphere" exists in it; the renderer resolves
nothing by content kind.

| Field | Type | Notes |
|---|---|---|
| `schema` | `u32` | must equal `SCENE_SCHEMA` (3) |
| `name` | `String` | also the manifest id of its artifact |
| `environment` | `Environment` | `ambient`, `skybox: Option<Member>`, `skybox_brightness` (default 900) |
| `cameras` | `Vec<Camera>` | review cameras (`direction` in local space, `distance` in planet radii, `tag`) |
| `expects` | `Vec<String>` | labels the content expects; the renderer only copies them into the report |
| `resources` | `Vec<PassResource>` | intermediate targets: `name`, `format`, `size`, `layers`, `usage` |
| `passes` | `Vec<PassSpec>` | execution order is array order |
| `lights` | `Vec<Light>` | `id`, `kind` (point / spot / directional), position, direction, color, intensity, `range`, angles, `shadows` |
| `shadow` | `Option<ShadowPlan>` | the virtual shadow-map page table: `table`, `light_offsets`, `atlas_side`, `layers`, `faces` |
| `objects` | `Vec<Object>` | `id`, `geometry`, `material`, `transform`, `cast_shadow`, `shadow_density` |
| `frame_materials` | `Vec<FrameMaterial>` | materials owned by the frame: name, inline WGSL, entry, params |
| `material_instances` | `Vec<MaterialInstance>` | generated `name` → `base` aliases, so one material can appear under several names |

Supporting types: `Member { graph, node, key }` is a reference to another artifact — the names are
for humans and error messages, the 64-hex `key` fetches the bytes. `Geometry` is either
`Mesh { member, bounding_radius }` or `Primitive { name, params, bounding_radius }`, where
`bounding_radius` is measured in local space; an object that casts a shadow with a positive
`shadow_density` must have one (`px-scene` refuses to build a frame for a caster without a radius,
since the page allocation is computed from it).
`Material` is `shader: Member`, `params`, `textures: BTreeMap<role, TextureRef>`, `alpha`, `cull`,
`depth_bias`; `TextureRef` is `binding`, `member`, `sampler`. `PassSpec` carries `kind`, an optional
pass-level `shader`, `label`, `entry`, `reads`, `writes`, `params`, `draws`, inline
`vertex_shader` / `vertex_entry`, the opaque `render` state text, `depth_target`, `cube_face` and
`viewport`. Every type declared in `scene.rs` (structs and enums alike) carries
`deny_unknown_fields`, so an unknown JSON field anywhere in the document is an error — silently
ignoring a field means "drew less and reported success". `Camera` comes from `art.rs` and is the one
exception: it ignores unknown fields. Material `params` is a free name→value map by design,
validated against the shader's descriptor rather than against a Rust list.

### What `check()` enforces

`SceneSpec::check` is the document's own gate; `scene_bytes` and `write_scene` call it, so a
malformed document never reaches disk.

- `schema == SCENE_SCHEMA`; at least one object.
- Object ids non-empty and unique; `shadow_density` finite and `≥ 0`; rotation quaternion of
  non-zero finite length; every texture `binding` present in `TEXTURE_SLOTS`.
- Light ids unique; light intensity finite and `≥ 0`.
- Resource names non-empty and unique, not `view`; `format` and `size` non-empty; `layers ≥ 1`.
- Pass `kind` ∈ `fullscreen` / `geometry` / `copy` / `compute`; a `fullscreen` pass must have a
  shader, a `geometry` or `copy` pass must not (the fragment stage of a geometry pass belongs to the
  material); a pass with a shader must name an entry point; at most one entry in `writes`; an empty
  `writes` is allowed for `fullscreen` and `geometry` (a pass may write only depth) but not for
  `copy` or `compute`; pass labels unique.
- `cube_face.face < 6`, and a pass with `cube_face` must also name a `depth_target`.
- Every `reads` / `writes` name resolves to a declared resource (`view` excepted), and no pass may
  read and write the same declared resource.
- If there are passes, at least one of them must write `view`.
- Frame materials: name non-empty and unique, not colliding with an object id, with non-empty
  inline shader text and entry point; every frame material and every generated material instance
  must be referenced by some draw. A draw's `material` must resolve to an object id, a frame
  material or an instance name.
- Material instances: names unique across objects, frame materials and instances; `base` must be an
  object id or a frame material name.

What it does not do: it never checks that a geometry name exists (there is no geometry table in the
document; the host resolves names), and it does not parse the `render` state text or the `depth=`
pairing — that parser exists once, in `px_pass`.

### Framing and members

`scene_bytes(spec, fingerprint)` produces the whole file: first an `Art` frame with
`id = spec.name`, `params = {schema, objects, lights}` (counts), empty `blobs`, and the caller's
`fingerprint`; then the `Scene` frame. The fingerprint comes from the caller because this crate
carries no hash beyond FNV-1a and the reference implementation hashes the canonical scene JSON.
`write_scene` is `scene_bytes` plus a write, and returns the byte count. `read_scene` returns the
first `Scene` frame of a file.

A scene document embeds **keys** rather than paths or names alone. Because identity is content, the
document then changes exactly when one of its members changes; `px_graph::scene_key` hashes the
document JSON together with all member keys, so a swapped member cannot be mistaken for the same
scene. `SceneSpec::members()` lists them (object geometry, material shaders, textures, skybox,
pass shaders). `Member::resolve` fails loudly when a key is not in the CAS, which is the intended
failure: a scene that points at a missing artifact must not render.

## The host ⇄ client layer

`Request` / `Response` / `Report` (with `ShotReport`, `PerfReport`, `Waits`, `GpuMs`, `Compare`,
`Pair`, `ErrorBar`, `GpuSample`), `Job` (`Shots` / `Perf{windows,drop}` / `Stable{frames}`),
`Scene` (`World` / `Artifact` / `Sequence` of `Shot`), `View` (`cam`, `sheet`, `columns`), `Lease`
and `ClientError` are declared in `render.rs`; the envelope is in `frame.rs`:

```text
u32 (little-endian) length
`J` + JSON {"frame":"protocol"|"request"|"response"|"refused", …}
```

`Refused` carries `reason` — the in-memory variant is a bare `String`, and the key asymmetry is part
of the format. There is no blob frame on this channel. `read_frame` returns `Ok(None)` only when the
length prefix cannot be read at all; a short payload is an error, so a half-written message is never
mistaken for "no message".

`ProtocolId { schema_version, protocol_hash, git_rev }` is built by `ProtocolId::local()`, where
`git_rev` is the short `HEAD` recorded at build time by `build.rs` (`unknown` if git is
unavailable). `Handshake { id, pid, exe }` adds the process id and executable path.
`Handshake::verify` compares **only** `schema_version` and then `protocol_hash`; a mismatch is a
`HandshakeError` naming both values.

The client (`client.rs`) reads a lease, connects, handshakes, then sends one request and reads one
response:

| Constant / path | Value |
|---|---|
| `LEASE_PATH` | `target/render-server.json` |
| connect timeout | 400 ms |
| read / write timeout | 180 s |
| startup timeout (autostart) | 90 s |

The lease is `{pid, port, protocol_hash, git_rev, exe}`; `Lease::matches` compares `protocol_hash`
and `git_rev` against the local `ProtocolId`, so a server left running from a different build is
skipped rather than talked to. `connect_or_start` spawns `current_exe --serve`, redirecting its
output to `target/render-server.log`, and polls the lease until the startup timeout expires.

## Versioning and the handshake

| Counter | Value | Scope |
|---|---|---|
| `SCHEMA_VERSION` | 12 | the protocol as a whole; travels in `ProtocolId` and in every report |
| `SCENE_SCHEMA` | 3 | the scene document only; checked by `SceneSpec::check` |
| `STREAM_VERSION` | 1 | the container after the magic; checked before any frame is decoded |

`protocol_hash()` is the FNV-1a-64 of the protocol snapshot text; `protocol_hash_hex()` renders it as
16 lowercase hex digits. It is a change detector for this side's declared shapes, not a security
measure, and it is what a running peer is checked against before anything else is exchanged.

The snapshot is `px_protocol/snapshots/protocol.snapshot.json`, produced by `canonical()` in
`tests/snapshot.rs` from the **real types** (never a hand-written fixture) and compared text-for-text
by `protocol_snapshot_is_current`. It is pretty-printed with sorted keys and a trailing newline, so
it is machine-written only. Regenerate it with:

```powershell
PX_UPDATE_SNAPSHOT=1 cargo test -p px_protocol
```

It records: `ProtocolId`, `Handshake`, `game::sim::WorldView` (the economy view's JSON shape, whose
type lives in `game`), `ArtBundle`, the asset-kind name list, a `VolumeData` blob shape, the whole
`render` family (`Request`, `Response`, `Report`, the three `Job` forms, `Scene` in all three forms,
`Lease`, `ShotReport`, `PerfReport`, `Pair`), `SceneSpec`, both `PassSpec` shapes, `BlobHeader` plus
a payload length, the frozen stream frame-kind list, `stream::MAGIC`, `stream::STREAM_VERSION` and
`client::LEASE_PATH`.

Two entries in that snapshot are **name lists, not live enumerations**, and both are compared as
text:

- `art::AssetKind.kinds` is built by the test's own match; it lists ten names and does not include
  `voxel_field` or `star_field`. Adding a variant still fails to compile until the match is
  extended, but adding the name to that list is a separate, manual step.
- `stream::Frame.kinds` is a fixed eight-name list (`protocol`, `world`, `art`, `scene`, `blob`,
  `request`, `response`, `refused`) that intentionally does not match the current five-variant
  `stream::Frame` plus four-variant `frame::Frame` split. It is a frozen identifier record; editing
  it changes `protocol_hash` for no protocol change.

Handshake consequences, in one line: **any change to the declared shapes changes the snapshot,
which changes `protocol_hash`, which makes a running peer with the old shape refuse the connection**
(and `Lease::matches` skips it before a socket is even opened). A semantic change that does not alter
any shape must bump `SCHEMA_VERSION` by hand; a shape change updates the snapshot by hand. Both are
intentional manual steps.

## What the tests pin

| Test | Asserts |
|---|---|
| `tests/snapshot.rs` | the snapshot text equals the canonical text built from the real types; `protocol_hash` is non-zero, 16 hex digits and deterministic |
| `tests/crate_graph.rs` | the dependency gates below |
| `tests/roundtrip.rs` | stream read → write is byte-identical; a foreign magic is `BadMagic`; handshake mismatches in schema version and hash are rejected; blob length and dtype are checked; `f32` payloads round-trip exactly |
| `tests/cube.rs`, `tests/cubemap.rs`, `tests/octahedral.rs`, `tests/domain.rs` | direction ↔ face/UV inversion, face-centre axes, atlas gutter behaviour and cube-map row ownership, octahedral round-trip error bounds — a wrong projection here warps everything downstream while the picture only "looks odd" |
| in-crate `art.rs` | diff classification order (fingerprint beats params beats blob headers), manifest prefix reading, camera normalization |
| in-crate `material.rs` | packing honours declared offsets, the three error classes, descriptor JSON stability, table shape (odd bindings, eight 2D + four cube, no binding 25) |
| in-crate `scene.rs` | unknown fields refused, defaults stay out of old documents (key order and absent keys), geometry and frame-material shapes round-trip, write → read → write is byte-identical |
| in-crate `sparse.rs` | per-cell dense → sparse round trip, no stored zeros, canonical encoding, out-of-range reads are 0 |
| in-crate `frame.rs` | host envelope bytes (`J`, JSON tag, length prefix), `refused` carries `reason`, stream end is `None`, truncated payload is an error, unknown prefix/tag refused |

### The crate-graph gate

`tests/crate_graph.rs` reads manifests as TOML (it does no reachability analysis — only directly
declared dependency names in `[dependencies]`, `[dev-dependencies]` and `[build-dependencies]`) and
asserts three things:

1. **`px_protocol`'s runtime dependency set is exactly `{serde, serde_json}`.** Set equality, not a
   subset: adding a dependency fails, and so does dropping one. The crate must stay frozen and tiny,
   because every consumer compiles it.
2. **`px_protocol`'s dev-dependencies do not contain the host.** The snapshot test constructs
   cross-process shapes from the real types; taking a shortcut through the host would compile
   wgpu / naga / winit into protocol tests. The shapes therefore live in this crate rather than in a
   satellite crate invented to dodge the dev edge.
3. **No consumer depends on the simulation.** The forbidden-edge table is
   `(px_render, px_sim)` and `(px_web, px_sim)`, checked in all three dependency sections of every
   workspace member's manifest: renderer and simulation may only meet through this protocol. Each
   edge carries a subject: `Present` (the package exists today, so not finding it is a failure — a
   renamed or deleted host would otherwise leave the edge unguarded) or `Reserved` (the package does
   not exist yet; not finding it is normal, but it is on the books). The test also asserts that at
   least three manifests were scanned and that `px_protocol` is a workspace member, since a gate
   that is not applied to anything looks exactly like a gate that passes.
