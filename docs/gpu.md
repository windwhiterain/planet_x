# GPU access

Two layers sit under the cooking side of the pipeline:

- **`px_gpu`** — the windowless compute host. It owns exactly three things: create a device, run
  one compute dispatch, read the result back (`px_gpu/src/lib.rs:5`).
- **`px_volume_gpu_op` and `px_nurbs_gpu_op`** — operator helper crates that ship WGSL inside the
  crate and drive those dispatches. The hot loop lives in WGSL; the orchestration (how far to
  subdivide, how to split parameters, how to assemble the result) lives in Rust
  (`px_nurbs_gpu_op/src/lib.rs:5-6`).

It is deliberately not `px_probe`: probes are a development tool and are absent from
`default-members`, so a production operator library must not depend on them
(`px_gpu/src/lib.rs:3-5`). The rendering host does **not** use this layer; it has its own device
in [renderer.md](renderer.md).

## One device per process

```rust
pub fn connect() -> Option<&'static Gpu>          // px_gpu/src/lib.rs:96
static GPU: OnceLock<Option<Gpu>>                 // px_gpu/src/lib.rs:25
```

`connect` builds the instance, adapter, device and queue on first call and returns the same
`&'static Gpu` afterwards. Device creation costs on the order of 0.3 s
(`px_gpu/src/lib.rs:95`), and every operator call wants the same device, so there is one. The
failure is memoised too: once `build` returns `None`, every later `connect` in that process also
returns `None`.

`px_probe` re-exports this device rather than creating one of its own
(`px_probe/src/common.rs:108-110`), so a probe and an operator run on the same adapter and the
same limits.

## Backend selection

| `WGPU_BACKEND` | Backends requested |
|---|---|
| unset, or anything unrecognised | `wgpu::Backends::VULKAN` |
| `dx12` | `wgpu::Backends::DX12` |
| `gl` or `gles` | `wgpu::Backends::GL` |

(`backends`, `px_gpu/src/lib.rs:47-53`.)

Vulkan is the default because it is the only backend the renderer compiles in
(`px_render/Cargo.toml:13-17`), and a device that can run the renderer is the only device on which
"the operator in the bake" and "the host that draws" are measuring the same thing
(`px_gpu/src/lib.rs:39-46`). `px_pass` enables no backend of its own — its library names only
`std` and `wgsl`, and Vulkan appears solely in its dev-dependencies so its GPU tests can build a
device (`px_pass/Cargo.toml:10`, `:16`). The environment variable exists as an escape hatch for a
machine without Vulkan; it is an override, not a chooser.

The selector is data, so a bad override cannot silently degrade: if the requested backend yields no
adapter, `build` returns `None` and callers report `没有可用 GPU` as an `Err`
(e.g. `px_volume_gpu_op/src/lib.rs:228-230`). Contrast `px_render::gpu`, which treats a
non-Vulkan backend as a hard failure and `exit(2)` (`px_render/src/gpu.rs:76-81`).

## wgpu features

### Cargo features (`px_gpu/Cargo.toml:20`)

```toml
wgpu = { version = "29", default-features = false, features = ["std", "vulkan", "wgsl"] }
```

- `default-features = false` — wgpu 29's default set pulls in backends this workspace has rejected
  (metal, gles, webgpu), so the set is named explicitly; it is the set `px_render` names
  (`px_render/Cargo.toml:17`), and `px_pass` names the same features minus the backend, which only
  its dev-dependency needs (`px_pass/Cargo.toml:10`, `:16`).
- `vulkan` — the one backend that is compiled in.
- `wgsl` — the shader language the compute entries are written in.
- **`std` is a rule, not a detail.** With `default-features = false`, omitting `std` lets
  `wgpu-core` use its no-std synchronisation, and error scopes then get popped out of order across
  threads (`Mismatched pop_error_scope call: error scopes must be popped in reverse order`),
  which appears as randomly failing tests with a different count of failures per run. Never rely
  on another dependency's default features to merge in a feature this crate needs; name it.

### Device features (`px_gpu/src/lib.rs:69`)

```rust
required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
```

The device feature is not optional where it is used: sampling an `f32` texture through a filtering
sampler requires `FLOAT32_FILTERABLE` at device creation, and the renderer's bind-group layouts
declare `TextureSampleType::Float { filterable: true }` with a filtering sampler
(`px_render/src/material.rs:201-211`, `px_pass/src/lib.rs:2385-2404`). Requesting the feature is
what makes that layout legal instead of a failure at pipeline or bind-group creation.

Masking the request with `adapter.features()` means a device that does not advertise it still comes
up; no dispatch in this layer samples a texture at all — every binding is a uniform or a storage
buffer (`px_gpu/src/lib.rs:101-108`), and neither helper crate creates a wgpu texture or sampler.

## Device creation parameters

`build` (`px_gpu/src/lib.rs:55-93`) uses:

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
about a specific bind-group count (`px_render/src/gpu.rs:88-98`). The compute host has no such
constraint, so it takes whatever the adapter offers.

## Error model

wgpu's default reaction to an uncaptured error is a **panic**, and a panic is fatal here:

> An operator implementation is loaded as a dylib (`px_volume_op/Cargo.toml` `crate-type =
> ["dylib"]`, `px_nurbs_gpu_op/Cargo.toml` `crate-type = ["dylib"]`). A Rust panic that unwinds out
> of a dylib is `Rust cannot catch foreign exceptions`, which aborts the whole bake process with
> no readable message. Therefore **no wgpu error may be allowed to reach the default handler**
> (`px_gpu/src/lib.rs:27-32`).

Two mechanisms enforce that:

1. **An installed handler.** `device.on_uncaptured_error(..)` stores the formatted error in
   `LAST_ERROR` and prints it to stderr (`px_gpu/src/lib.rs:80-86`).
   `take_last_error() -> Option<String>` drains that slot (`px_gpu/src/lib.rs:35-37`) so a caller
   can turn "something went wrong on the device" into a message in its own `Err`.
2. **An error scope around the dispatch.** `dispatch_slots` pushes
   `ErrorFilter::Validation` before encoding and `pop()`s it after submitting
   (`px_gpu/src/lib.rs:283`, `:303-305`); a validation error becomes
   `Err("px_gpu 校验出错：…")` instead of a panic. Readback failures (poll timeout, missing
   callback, map failure) are returned the same way (`px_gpu/src/lib.rs:314-323`).

Operators inherit this contract: their public functions return `Result<_, String>`, and a missing
device is an `Err`, not a fallback to CPU (`px_nurbs_gpu_op/src/lib.rs:11-13`).

A second, related rule lives in the helpers: **a dispatch dimension must stay within the device
limits** — the x axis of a workgroup grid is capped at 65535, so a large grid is split along y
(`px_volume_gpu_op/src/lib.rs:214-224`, `px_nurbs_gpu_op/src/lib.rs:57-62`). Exceeding it is a
validation error, and a validation error is the abort above.

## The dispatch helper

```rust
pub enum Binding<'a> { Uniform(&'a [u8]), Storage(&'a [u8]), Write(&'a [u8]) }   // :101
pub struct Slot<'a> { pub binding: u32, pub value: Binding<'a> }                 // :116

pub fn dispatch(gpu: &Gpu, wgsl: &str, entry: &str,
                bindings: &[Binding<'_>], workgroups: (u32, u32, u32))
    -> Result<Vec<Vec<u8>>, String>                                              // :125
pub fn dispatch_slots(gpu: &Gpu, wgsl: &str, entry: &str,
                      slots: &[Slot<'_>], workgroups: (u32, u32, u32))
    -> Result<Vec<Vec<u8>>, String>                                              // :148
```

- `dispatch` numbers the bindings `0..n` in declaration order; `dispatch_slots` takes explicit
  binding numbers. The explicit form exists because `@group(0) @binding(n)` is module-level: one
  module cannot declare two different types at the same `n`, so entries that share one module use
  non-overlapping numbers and wgpu's pipeline layout covers only the bindings the entry actually
  references (`px_gpu/src/lib.rs:110-119`). `march` is the worked example: bindings
  0, 1, 4, 5, 6, 10, 11, 12, 13, 14, 15 (`px_volume_gpu_op/src/lib.rs:1012-1063`).
- **Every buffer length must be a multiple of 4**; the helper says so immediately rather than
  letting the driver report it somewhere unrelated (`px_gpu/src/lib.rs:155-166`).
- Per binding, in order: a shader module is created, one buffer per slot is created with the usage
  the binding kind implies (`UNIFORM`/read-only `STORAGE`/read-write `STORAGE`, all with
  `COPY_DST`, writes also `COPY_SRC`), the bytes are written with `queue.write_buffer`, and a
  bind-group layout and bind group are built from those slots — an entry's `binding` field is
  always the **slot number**, never a positional index (`px_gpu/src/lib.rs:168-248`).
- One compute pass is encoded, `dispatch_workgroups` is called, then each `Write` buffer is copied
  into its **own** staging buffer (`MAP_READ | COPY_DST`) before a single `queue.submit`
  (`px_gpu/src/lib.rs:268-301`). Giving each readback its own staging buffer costs one round of
  synchronisation for the whole set instead of one per buffer.
- Readback polls before it waits on the mapping callback, then copies only the declared byte count
  out and unmaps (`px_gpu/src/lib.rs:307-334`). The result vector is in `Write` declaration order.
- **Nothing is cached.** Every call creates a shader module, a bind-group layout, a pipeline layout
  and a compute pipeline (`cache: None`). A dispatch is a whole-bake step, not a frame loop, so the
  helper stays small; the caching that matters on the render path lives in `px_pass` instead.

On top of the helper, each crate has the same grid arithmetic: 64 threads per workgroup, x capped
at 65535, the remainder folded into y (`workgroups`, `px_volume_gpu_op/src/lib.rs:218`,
`px_nurbs_gpu_op/src/lib.rs:56`). The two copies are identical; a volume dispatch for `face` 1024
is 98304 workgroups, which is why the split exists.

## How the shaders are embedded

WGSL is source code of the crate, not an asset:

| Crate | Shader | How it ships |
|---|---|---|
| `px_volume_gpu_op` | `src/sampler.wgsl` (cross-face trilinear sampling, occupancy skip, marching, toning, star field, emission bake) | `pub const SAMPLER_WGSL: &str = include_str!("sampler.wgsl")` (`:70`) |
| `px_volume_gpu_op` | the repack kernel | inline `pub const REPACK_WGSL: &str = r#"…"#` (`:183`) |
| `px_nurbs_gpu_op` | `src/surface.wgsl`, `src/curve.wgsl` | `include_str!` (`:30`, `:33`) |

`sampler.wgsl` is one module with eight compute entry points — `sample_points`, `march`,
`tone_of`, `hue_of_keys`, `sky_radiance`, `grade_pixels`, `bin_luma`, `bake_emission` — and it
declares every binding at module scope, reusing numbers so that no two types collide. That is why
the dispatch helper needs explicit slot numbers.

Because the `.wgsl` files live in `src/`, they are part of the crate's source fingerprint: the
fingerprint walks `.rs` and `.wgsl` under `src/` (`px_fingerprint/src/lib.rs:248`, `:358`), the
implementation libraries compute it in `build.rs`, and it is one of the inputs of an operator's
identity. Editing a line of WGSL changes the operator's key (`px_nurbs_gpu_op/build.rs:1-8`,
`px_nurbs_gpu_op/src/lib.rs:422-426`).

## What the helpers add

**`px_volume_gpu_op`** — the volume data layout and the geometry kernels are a single formula
shared by both languages; the module is explicit that the Rust side and the WGSL side must agree
element by element (`px_volume_gpu_op/src/lib.rs:1-13`), and a GPU↔CPU test pins the flat index
(`:268-299`). It also packs the occupancy index that lets the marching entry skip empty coarse
blocks: the index is derived from an already baked emission volume, the parameters and the mask are
produced as one pair and bound together, and the same WGSL runs both with and without it, so the
A/B comparison uses one implementation (`:86-139`, `:103-106`).

**`px_nurbs_gpu_op`** — evaluates surface grids (position + normal) and curve points, with a
degree cap that matches the fixed-size basis table in the shader (`MAX_DEGREE = 8`, `:36`), a
per-dispatch point cap that stops a tiny tolerance from asking for a many-gigabyte buffer
(`MAX_POINTS = 4_000_000`, `:42`), and uniform subdivision to a chord tolerance assembled through
the same mesh/polyline code the CPU path uses (`:257-307`, `:348-385`). It is itself a `dylib` and
exports its identity via `px_impl_lib!()` (`:420-426`).

## Tests

`px_gpu` has one test: a compute round-trip that writes `x * 2` and asserts the readback, skipped
with a message when `connect()` returns `None` — the covered chain is buffer → bind group →
dispatch → staging → map, which is the part whose failure would look like "the operator returns
zeros" (`px_gpu/src/lib.rs:341-382`). The helper crates test GPU against CPU point by point and
against analytic targets, under the same rule: no device means skip with a message, never a silent
pass and never a failure (`px_nurbs_gpu_op/src/tests.rs:1-4`).

## Environment variables

| Variable | Read by | Effect |
|---|---|---|
| `WGPU_BACKEND` | `px_gpu/src/lib.rs:48` | backend override, `dx12` / `gl` / `gles`, default Vulkan |
| `PX_SKIP_REPORT` | `px_volume_gpu_op/src/lib.rs:2031` | print one line of occupancy-block statistics (live blocks vs total) |
| `PX_SKIP_OFF` | `px_volume_gpu_op/src/lib.rs:2050` | run the same dispatch with the occupancy index disabled, for the A/B comparison |

`PX_SKIP_OFF` carries a caveat: the cook cache key does not include environment variables, so an
A/B run must also change a parameter (for example `steps`) or the second run is served from the
cache and measures disk read time instead.
