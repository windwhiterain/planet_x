# GPU access

Two layers sit under the cooking side of the pipeline:

- **`px_gpu`** — the windowless compute host. It owns exactly three things: create a device, run
  one compute dispatch, read the result back (`connect` and `dispatch_slots` in
  `px_gpu/src/lib.rs`).
- **`px_volume_gpu_op` and `px_nurbs_gpu_op`** — operator helper crates that ship WGSL inside the
  crate and drive those dispatches. The hot loop lives in WGSL; the orchestration (how far to
  subdivide, how to split parameters, how to assemble the result) lives in Rust
  (`tessellate_surface` / `tessellate_curve` in `px_nurbs_gpu_op/src/lib.rs`).

It is deliberately not `px_probe`: probes are a development tool and are absent from
`default-members`, so a production operator library must not depend on them
(`px_gpu/Cargo.toml`, its `[dependencies]`). The rendering host does **not** use this layer; it
has its own device in [renderer.md](renderer.md).

## One device per process

```rust
pub fn connect() -> Option<&'static Gpu>          // px_gpu/src/lib.rs
static GPU: OnceLock<Option<Gpu>>                 // px_gpu/src/lib.rs
```

`connect` builds the instance, adapter, device and queue on first call and returns the same
`&'static Gpu` afterwards. Device creation costs on the order of 0.3 s
(`connect` in `px_gpu/src/lib.rs`), and every operator call wants the same device, so there is
one. The failure is memoised too: once `build` returns `None`, every later `connect` in that
process also returns `None`.

`px_probe` re-exports this device rather than creating one of its own (the
`pub use px_gpu::{Gpu, connect}` re-export in `px_probe/src/common.rs`), so a probe and an
operator run on the same adapter and the same limits.

## Backend selection

| `WGPU_BACKEND` | Backends requested |
|---|---|
| unset, or anything unrecognised | `wgpu::Backends::VULKAN` |
| `dx12` | `wgpu::Backends::DX12` |
| `gl` or `gles` | `wgpu::Backends::GL` |

(`backends` in `px_gpu/src/lib.rs`.)

Vulkan is the default because it is the only backend the renderer compiles in
(the `wgpu` dependency line in `px_render/Cargo.toml`), and a device that can run the renderer is
the only device on which "the operator in the bake" and "the host that draws" are measuring the
same thing (`build` in `px_gpu/src/lib.rs`, the `required_features` / `required_limits` it asks
for). `px_pass` enables no backend of its own — its library names only `std` and `wgsl`, and
Vulkan appears solely in its dev-dependencies so its GPU tests can build a device (the `wgpu`
lines in `[dependencies]` and `[dev-dependencies]` of `px_pass/Cargo.toml`). The environment
variable exists as an escape hatch for a machine without Vulkan; it is an override, not a chooser.

The selector is data, so a bad override cannot silently degrade: if the requested backend yields no
adapter, `build` returns `None` and callers report `没有可用 GPU` as an `Err`
(e.g. the `connect()` guard in `march`, `px_volume_gpu_op/src/lib.rs`). Contrast `px_render::gpu`,
which treats a non-Vulkan backend as a hard failure and `exit(2)` (`refuse` in
`px_render/src/gpu.rs`).

## wgpu features

### Cargo features (`px_gpu/Cargo.toml`, the `wgpu` dependency)

```toml
wgpu = { version = "29", default-features = false, features = ["std", "vulkan", "wgsl"] }
```

- `default-features = false` — wgpu 29's default set pulls in backends this workspace has rejected
  (metal, gles, webgpu), so the set is named explicitly; it is the set `px_render` names
  (`px_render/Cargo.toml`, the `wgpu` dependency), and `px_pass` names the same features minus the
  backend, which only its dev-dependency needs (the `wgpu` lines in `[dependencies]` and
  `[dev-dependencies]` of `px_pass/Cargo.toml`).
- `vulkan` — the one backend that is compiled in.
- `wgsl` — the shader language the compute entries are written in.
- **`std` is a rule, not a detail.** With `default-features = false`, omitting `std` lets
  `wgpu-core` use its no-std synchronisation, and error scopes then get popped out of order across
  threads (`Mismatched pop_error_scope call: error scopes must be popped in reverse order`),
  which appears as randomly failing tests with a different count of failures per run. Never rely
  on another dependency's default features to merge in a feature this crate needs; name it.

### Device features (`build` in `px_gpu/src/lib.rs`)

```rust
required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
```

The device feature is not optional where it is used: sampling an `f32` texture through a filtering
sampler requires `FLOAT32_FILTERABLE` at device creation, and the renderer's bind-group layouts
declare `TextureSampleType::Float { filterable: true }` with a filtering sampler
(`px_render/src/material.rs` and `px_pass/src/lib.rs`, the layout entries that declare it).
Requesting the feature is what makes that layout legal instead of a failure at pipeline or
bind-group creation.

Masking the request with `adapter.features()` means a device that does not advertise it still comes
up; no dispatch in this layer samples a texture at all — every binding is a uniform or a storage
buffer (`Binding` in `px_gpu/src/lib.rs`), and neither helper crate creates a wgpu texture or
sampler.

## Device creation parameters

`build` (`px_gpu/src/lib.rs`) uses:

| Parameter | Value | Why |
|---|---|---|
| instance | `InstanceDescriptor::new_without_display_handle()` | no window, no surface; there is no swapchain anywhere on this path |
| adapter | `PowerPreference::HighPerformance`, `force_fallback_adapter: false`, `compatible_surface: None` | the high-performance adapter; a fallback CPU adapter would silently change what the measurements mean |
| `required_features` | `adapter.features() & FLOAT32_FILTERABLE` | see above |
| `required_limits` | `adapter.limits()` | **the adapter's own limits, never `downlevel_defaults`.** The downlevel default caps a storage buffer at 128 MB, while a 128-face volume (6 × 128² × 128 layers × 6 channels × 4 B) is about 300 MB ⇒ a validation error, and a validation error that reaches wgpu's default handler panics (see the error model) |
| `experimental_features` | `ExperimentalFeatures::disabled()` | nothing here needs an experimental path |
| `memory_hints` | `MemoryHints::MemoryUsage` | the buffers are transient compute payloads |
| `trace` | `Trace::Off` | no tracing |

The rendering host takes the opposite approach for limits on purpose: it asks for
`Limits { max_sampled_textures_per_shader_stage: 32, ..Limits::default() }` because it must reason
about a specific bind-group count (`connect_with` in `px_render/src/gpu.rs`, its
`required_limits`). The compute host has no such constraint, so it takes whatever the adapter
offers.

## Error model

wgpu's default reaction to an uncaptured error is a **panic**, and a panic is fatal here:

> An operator implementation is loaded as a dylib (`px_volume_op/Cargo.toml` `crate-type =
> ["dylib"]`, `px_nurbs_gpu_op/Cargo.toml` `crate-type = ["dylib"]`). A Rust panic that unwinds out
> of a dylib is `Rust cannot catch foreign exceptions`, which aborts the whole bake process with
> no readable message. Therefore **no wgpu error may be allowed to reach the default handler**
> (`on_uncaptured_error` and the error scope in `px_gpu/src/lib.rs`).

Two mechanisms enforce that:

1. **An installed handler.** `device.on_uncaptured_error(..)` stores the formatted error in
   `LAST_ERROR` and prints it to stderr (`on_uncaptured_error` in `px_gpu/src/lib.rs`).
   `take_last_error() -> Option<String>` drains that slot (`take_last_error` in
   `px_gpu/src/lib.rs`) so a caller can turn "something went wrong on the device" into a message
   in its own `Err`.
2. **An error scope around the dispatch.** `dispatch_slots` pushes
   `ErrorFilter::Validation` before encoding and `pop()`s it after submitting
   (`dispatch_slots` in `px_gpu/src/lib.rs`, around the encode and submit); a validation error
   becomes `Err("px_gpu 校验出错：…")` instead of a panic. Readback failures (poll timeout,
   missing callback, map failure) are returned the same way (`dispatch_slots` in
   `px_gpu/src/lib.rs`, the readback loop).

Operators inherit this contract: their public functions return `Result<_, String>`, and a missing
device is an `Err`, not a fallback to CPU (`surface_grid` / `curve_points` in
`px_nurbs_gpu_op/src/lib.rs`, each with a `connect()` guard).

A second, related rule lives in the helpers: **a dispatch dimension must stay within the device
limits** — the x axis of a workgroup grid is capped at 65535, so a large grid is split along y
(`workgroups` in `px_volume_gpu_op/src/lib.rs` and in `px_nurbs_gpu_op/src/lib.rs`). Exceeding it
is a validation error, and a validation error is the abort above.

## The dispatch helper

```rust
pub enum Binding<'a> { Uniform(&'a [u8]), Storage(&'a [u8]), Write(&'a [u8]) }
pub struct Slot<'a> { pub binding: u32, pub value: Binding<'a> }

pub fn dispatch(gpu: &Gpu, wgsl: &str, entry: &str,
                bindings: &[Binding<'_>], workgroups: (u32, u32, u32))
    -> Result<Vec<Vec<u8>>, String>
pub fn dispatch_slots(gpu: &Gpu, wgsl: &str, entry: &str,
                      slots: &[Slot<'_>], workgroups: (u32, u32, u32))
    -> Result<Vec<Vec<u8>>, String>
```

- `dispatch` numbers the bindings `0..n` in declaration order; `dispatch_slots` takes explicit
  binding numbers. The explicit form exists because `@group(0) @binding(n)` is module-level: one
  module cannot declare two different types at the same `n`, so entries that share one module use
  non-overlapping numbers and wgpu's pipeline layout covers only the bindings the entry actually
  references (`dispatch_slots` in `px_gpu/src/lib.rs`, the layout built from its `entries`).
  `march` is the worked example: bindings 0, 1, 4, 5, 6, 10, 11, 12, 13, 14, 15 (the `march`
  dispatch in `px_volume_gpu_op/src/lib.rs`).
- **Every buffer length must be a multiple of 4**; the helper says so immediately rather than
  letting the driver report it somewhere unrelated (the `bytes.len() % 4` check in
  `dispatch_slots`, `px_gpu/src/lib.rs`).
- Per binding, in order: a shader module is created, one buffer per slot is created with the usage
  the binding kind implies (`UNIFORM`/read-only `STORAGE`/read-write `STORAGE`, all with
  `COPY_DST`, writes also `COPY_SRC`), the bytes are written with `queue.write_buffer`, and a
  bind-group layout and bind group are built from those slots — an entry's `binding` field is
  always the **slot number**, never a positional index (`dispatch_slots` in `px_gpu/src/lib.rs`,
  `slots[index].binding`).
- One compute pass is encoded, `dispatch_workgroups` is called, then each `Write` buffer is copied
  into its **own** staging buffer (`MAP_READ | COPY_DST`) before a single `queue.submit`
  (`dispatch_slots` in `px_gpu/src/lib.rs`, the compute pass and the single submit). Giving each
  readback its own staging buffer costs one round of synchronisation for the whole set instead of
  one per buffer.
- Readback polls before it waits on the mapping callback, then copies only the declared byte count
  out and unmaps (`dispatch_slots` in `px_gpu/src/lib.rs`, the readback loop). The result vector
  is in `Write` declaration order.
- **Nothing is cached.** Every call creates a shader module, a bind-group layout, a pipeline layout
  and a compute pipeline (`cache: None`). A dispatch is a whole-bake step, not a frame loop, so the
  helper stays small; the caching that matters on the render path lives in `px_pass` instead.

On top of the helper, each crate has the same grid arithmetic: 64 threads per workgroup, x capped
at 65535, the remainder folded into y (`workgroups` in `px_volume_gpu_op/src/lib.rs`,
`workgroups` in `px_nurbs_gpu_op/src/lib.rs`). The two copies are identical; a volume dispatch for
`face` 1024 is 98304 workgroups, which is why the split exists.

## How the shaders are embedded

WGSL is source code of the crate, not an asset:

| Crate | Shader | How it ships |
|---|---|---|
| `px_volume_gpu_op` | `src/sampler.wgsl` (cross-face trilinear sampling, occupancy skip, marching, toning, star field, emission bake) | `pub const SAMPLER_WGSL: &str = include_str!("sampler.wgsl")` (`px_volume_gpu_op/src/lib.rs`) |
| `px_volume_gpu_op` | the repack kernel | inline `pub const REPACK_WGSL: &str = r#"…"#` (`px_volume_gpu_op/src/lib.rs`) |
| `px_nurbs_gpu_op` | `src/surface.wgsl`, `src/curve.wgsl` | `include_str!` (`px_nurbs_gpu_op/src/lib.rs`) |

`sampler.wgsl` is one module with eight compute entry points — `sample_points`, `march`,
`tone_of`, `hue_of_keys`, `sky_radiance`, `grade_pixels`, `bin_luma`, `bake_emission` — and it
declares every binding at module scope, reusing numbers so that no two types collide. That is why
the dispatch helper needs explicit slot numbers.

Because the `.wgsl` files live in `src/`, they are part of the crate's source fingerprint: every
`.wgsl` under `src/` is collected whether or not a Rust declaration names it, the implementation
libraries compute that fingerprint in `build.rs`, and it is one of the inputs of an operator's
identity. Editing a line of WGSL changes the operator's key.

## What the helpers add

**`px_volume_gpu_op`** — the volume data layout and the geometry kernels are a single formula
shared by both languages; the Rust side and the WGSL side must agree element by element, and that
agreement is pinned by a test rather than asserted by a comment: `flat_index` against the
shader's own index arithmetic, under `the_gpu_agrees_with_the_cpu_on_the_flat_index`
(`px_volume_gpu_op/src/lib.rs`). The nearest statement of the same agreement that survives in
the source is `SkyUniform::to_bytes`'s doc, which is the layout `sampler.wgsl`'s `Sky` mirrors.
Docs here once carried pre-strip line-number anchors, so every coordinate in this file is now
file + symbol. It also packs the occupancy index that lets the marching entry skip empty coarse
blocks: the index
is derived from an already baked emission volume, the parameters and the mask are produced as one
pair and bound together, and the same WGSL runs both with and without it, so the A/B comparison
uses one implementation (`raymarch_sky` in `px_volume_gpu_op/src/lib.rs`, where
`Occupancy::from_flat` and `OccupancyUniform::packed` produce the pair that is bound together).

**`px_nurbs_gpu_op`** — evaluates surface grids (position + normal) and curve points, with a
degree cap that matches the fixed-size basis table in the shader (`MAX_DEGREE` in
`px_nurbs_gpu_op/src/lib.rs`), a per-dispatch point cap that stops a tiny tolerance from asking
for a many-gigabyte buffer (`MAX_POINTS` in `px_nurbs_gpu_op/src/lib.rs`), and uniform
subdivision to a chord tolerance assembled through the same mesh/polyline code the CPU path uses
(`tessellate_surface` / `tessellate_curve` in `px_nurbs_gpu_op/src/lib.rs`). It is itself a
`dylib` and exports its identity via `px_impl_lib!()` (the `px_impl_lib!()` call in
`px_nurbs_gpu_op/src/lib.rs`).

## Tests

`px_gpu` has one test: a compute round-trip that writes `x * 2` and asserts the readback — the
covered chain is buffer → bind group → dispatch → staging → map, which is the part whose failure
would look like "the operator returns zeros".

Every GPU-backed check **requires a device** rather than skipping. `px_gpu::require_gpu` is the one
place that decides; it panics with `this check requires a working GPU` and the underlying error.
The helper crates test GPU against CPU point by point and against analytic targets under the same
rule. A machine without a GPU therefore fails these tests instead of reporting them green.

## Environment variables

| Variable | Read by | Effect |
|---|---|---|
| `WGPU_BACKEND` | `backends` in `px_gpu/src/lib.rs` | backend override, `dx12` / `gl` / `gles`, default Vulkan |
| `PX_SKIP_REPORT` | `raymarch_sky` in `px_volume_gpu_op/src/lib.rs` | print one line of occupancy-block statistics (live blocks vs total) |
| `PX_SKIP_OFF` | `raymarch_sky` in `px_volume_gpu_op/src/lib.rs` | run the same dispatch with the occupancy index disabled, for the A/B comparison |

`PX_SKIP_OFF` carries a caveat: the cook cache key does not include environment variables, so an
A/B run must also change a parameter (for example `steps`) or the second run is served from the
cache and measures disk read time instead.
