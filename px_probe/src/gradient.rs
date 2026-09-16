use crate::common::{
    JOB_BIND_GROUP, MATERIAL_BIND_GROUP, assemble, connect, job_group_source,
};
use crate::params::{CLOUD_BASE, CLOUD_TOP, CloudParams};

const POINTS: usize = 128;
const STEPS: usize = 5;
const CUBE_FACE: u32 = 256;
const KINK_FACTOR: f32 = 8.0;
const BLOCK: usize = 37;
const CANDIDATES: usize = 5;
const SWEEP: [f32; STEPS] = [8e-5, 4e-5, 2e-5, 1e-5, 5e-6];
const SIMPLE_SWEEP: [f32; STEPS] = [1e-2, 5e-3, 2.5e-3, 1.25e-3, 6.25e-4];
const SIMPLE_MARGIN: f32 = 1e-2;
const SIMPLE_INNER: f32 = 1.0;
const SIMPLE_OUTER: f32 = 1.5;
const SIMPLE_DETAIL: f32 = 2.0;
const SIMPLE_MASK: f32 = 156.0 / 255.0;
const PRODUCTION_MASK: f32 = 153.0 / 255.0;

#[derive(Clone, Copy)]
enum Mask {
    Varying,
    Constant(f32),
}

const PROBE: &str = r#"
struct Job {
    head: vec4<u32>,
    steps: array<vec4<f32>, 5>,
    points: array<vec4<f32>, 128>,
};

@group(#{JOB_BIND_GROUP}) @binding(0) var<uniform> job: Job;
@group(#{JOB_BIND_GROUP}) @binding(1) var<storage, read_write> out: array<vec4<f32>>;

fn fd_axis(point: vec3<f32>, axis: u32, h: f32) -> f32 {
    let ahead = vec3<f32>(
        h * select(1.0, 0.0, axis != 0u),
        h * select(1.0, 0.0, axis != 1u),
        h * select(1.0, 0.0, axis != 2u),
    );
    return (
        cloud_field(point - ahead * 2.0)
            - 8.0 * cloud_field(point - ahead)
            + 8.0 * cloud_field(point + ahead)
            - cloud_field(point + ahead * 2.0)
    ) / (12.0 * h);
}

fn fd_gradient(point: vec3<f32>, h: f32) -> vec3<f32> {
    return vec3<f32>(
        fd_axis(point, 0u, h),
        fd_axis(point, 1u, h),
        fd_axis(point, 2u, h),
    );
}

fn probe_octave_margin(point: vec3<f32>, seed: u32, octaves: u32) -> f32 {
    var current = 1.0;
    var margin = 1e9;
    for (var octave = 0u; octave < octaves; octave += 1u) {
        let sample = gradient_noise_3_grad(point * current, seed ^ octave);
        margin = min(margin, min(sample.value, 1.0 - sample.value) * current);
        current *= 2.0;
    }
    return margin;
}

fn probe_lattice_margin(direction: vec3<f32>, altitude: f32) -> f32 {
    let across = params.detail_scale;
    let x = direction * (across + altitude * across * span());
    let local = x - floor(x);
    return min(
        min(local.x, 1.0 - local.x),
        min(min(local.y, 1.0 - local.y), min(local.z, 1.0 - local.z)),
    );
}

fn probe_noise_margin(direction: vec3<f32>, across: f32, seed: u32, altitude: f32, octaves: u32) -> f32 {
    return probe_octave_margin(direction * (across + altitude * across * span()), seed, octaves);
}

fn probe_noise_value(direction: vec3<f32>, altitude: f32) -> f32 {
    let tower = sampled_noise(direction, altitude, params.detail_scale * 0.35, 3u, params.seed);
    let skin = sampled_noise(direction, altitude, params.detail_scale * 1.70, 2u, params.seed ^ 31u);
    return tower * 0.62 + skin * 0.38;
}

fn probe_gate_margin(point: vec3<f32>, cover: f32) -> f32 {
    let medium = medium_of(point);
    let altitude = medium.altitude;
    let direction = medium.direction;
    var margin = min(altitude, 1.0 - altitude);
    let tower_across = params.detail_scale * 0.35;
    let skin_across = params.detail_scale * 1.70;
    margin = min(margin, probe_noise_margin(direction, tower_across, params.seed, altitude, 3u));
    margin = min(margin, probe_noise_margin(direction, skin_across, params.seed ^ 31u, altitude, 3u));
    if params.ablate == ABLATE_FETCH || params.ablate == ABLATE_NOISE {
        return margin;
    }
    let blended = probe_noise_value(direction, altitude);
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    margin = min(margin, footprint / max(params.coverage_gain, 1e-4));
    let lobed = clamp((footprint + blended - 1.0) * params.coverage_gain, 0.0, 1.0);
    margin = min(margin, min(lobed, 1.0 - lobed) / params.coverage_gain);
    let shape = shape_of(cover, altitude, blended);
    return min(margin, min(shape, 1.0 - shape) * max(1.0 - params.erode, 1e-4));
}

fn stencil_check_value(point: vec3<f32>) -> f32 {
    return point.x * point.x * point.x * point.x
        + 2.0 * point.y * point.y * point.y
        - point.z * point.z
        + 5.0 * point.x * point.y * point.z;
}

fn stencil_check_axis(point: vec3<f32>, axis: u32, h: f32) -> f32 {
    let ahead = vec3<f32>(
        h * select(1.0, 0.0, axis != 0u),
        h * select(1.0, 0.0, axis != 1u),
        h * select(1.0, 0.0, axis != 2u),
    );
    return (
        stencil_check_value(point - ahead * 2.0)
            - 8.0 * stencil_check_value(point - ahead)
            + 8.0 * stencil_check_value(point + ahead)
            - stencil_check_value(point + ahead * 2.0)
    ) / (12.0 * h);
}

fn simple_noise_value(point: vec3<f32>) -> f32 {
    let medium = medium_of(point);
    return sampled_noise(medium.direction, medium.altitude, params.detail_scale, 1u, params.seed);
}

fn simple_noise_fd_axis(point: vec3<f32>, axis: u32, h: f32) -> f32 {
    let ahead = vec3<f32>(
        h * select(1.0, 0.0, axis != 0u),
        h * select(1.0, 0.0, axis != 1u),
        h * select(1.0, 0.0, axis != 2u),
    );
    return (
        simple_noise_value(point - ahead * 2.0)
            - 8.0 * simple_noise_value(point - ahead)
            + 8.0 * simple_noise_value(point + ahead)
            - simple_noise_value(point + ahead * 2.0)
    ) / (12.0 * h);
}

fn simple_noise_fd(point: vec3<f32>, h: f32) -> vec3<f32> {
    return vec3<f32>(
        simple_noise_fd_axis(point, 0u, h),
        simple_noise_fd_axis(point, 1u, h),
        simple_noise_fd_axis(point, 2u, h),
    );
}

fn simple_noise_axis(direction: vec3<f32>, altitude: f32, radius: f32, kind: u32) -> vec3<f32> {
    let across = params.detail_scale;
    let sample = sampled_noise_along(direction, across, 1u, params.seed, altitude);
    let raw = sample.gradient;
    let along = across * span();
    let xi = across + altitude * along;
    let axial = dot(direction, raw);
    let tangential = raw - axial * direction;
    let radial = axial * direction;
    if kind == 0u {
        return xi * tangential + radius * across * radial;
    }
    if kind == 1u {
        return tangential - altitude * across * radial;
    }
    if kind == 2u {
        return across * tangential + radius * across * radial;
    }
    if kind == 3u {
        return xi * tangential + altitude * across * radial;
    }
    return xi * tangential + altitude * along * radial;
}

var<private> frozen_cover: f32;
var<private> frozen_altitude: f32;
var<private> frozen_noise: f32;

fn altitude_channel(point: vec3<f32>) -> f32 {
    let medium = medium_of(point);
    if medium.altitude < 0.0 || medium.altitude > 1.0 {
        return 0.0;
    }
    return shape_of(frozen_cover, medium.altitude, frozen_noise);
}

fn noise_channel(point: vec3<f32>) -> f32 {
    let medium = medium_of(point);
    let noise = probe_noise_value(medium.direction, medium.altitude);
    return shape_of(frozen_cover, frozen_altitude, noise);
}

fn cover_channel(point: vec3<f32>) -> f32 {
    let medium = medium_of(point);
    let cover = coverage_of(medium.direction);
    if cover <= 0.0 {
        return 0.0;
    }
    return shape_of(cover, frozen_altitude, frozen_noise);
}

fn channel_value(point: vec3<f32>, which: u32) -> f32 {
    if which == 0u {
        return altitude_channel(point);
    }
    if which == 1u {
        return noise_channel(point);
    }
    return cover_channel(point);
}

fn channel_fd_axis(point: vec3<f32>, axis: u32, h: f32, which: u32) -> f32 {
    let ahead = vec3<f32>(
        h * select(1.0, 0.0, axis != 0u),
        h * select(1.0, 0.0, axis != 1u),
        h * select(1.0, 0.0, axis != 2u),
    );
    return (
        channel_value(point - ahead * 2.0, which)
            - 8.0 * channel_value(point - ahead, which)
            + 8.0 * channel_value(point + ahead, which)
            - channel_value(point + ahead * 2.0, which)
    ) / (12.0 * h);
}

fn channel_fd(point: vec3<f32>, h: f32, which: u32) -> vec3<f32> {
    return vec3<f32>(
        channel_fd_axis(point, 0u, h, which),
        channel_fd_axis(point, 1u, h, which),
        channel_fd_axis(point, 2u, h, which),
    );
}

fn sampled_mask_axis(point: vec3<f32>, axis: u32, h: f32) -> f32 {
    let ahead = vec3<f32>(
        h * select(1.0, 0.0, axis != 0u),
        h * select(1.0, 0.0, axis != 1u),
        h * select(1.0, 0.0, axis != 2u),
    );
    let behind = textureSampleLevel(coverage_map, coverage_sampler, medium_of(point - ahead).direction, 0.0);
    let front = textureSampleLevel(coverage_map, coverage_sampler, medium_of(point + ahead).direction, 0.0);
    return (front.r - behind.r) / (2.0 * h);
}

fn sampled_mask_fd(point: vec3<f32>, h: f32) -> vec3<f32> {
    return vec3<f32>(
        sampled_mask_axis(point, 0u, h),
        sampled_mask_axis(point, 1u, h),
        sampled_mask_axis(point, 2u, h),
    );
}

@compute @workgroup_size(64)
fn gradient_probe(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= job.head.x) {
        return;
    }
    let point = job.points[index].xyz;
    let medium = medium_of(point);
    let cover = coverage_of(medium.direction);
    let analytic = cloud_field_gradient_analytic(point);
    let field = cloud_field(point);
    let row = index * 37u;
    out[row] = vec4<f32>(analytic, probe_gate_margin(point, cover));
    out[row + 1u] = vec4<f32>(field, cover, medium.altitude, length(analytic));
    for (var slot = 0u; slot < 5u; slot += 1u) {
        out[row + 2u + slot] = vec4<f32>(fd_gradient(point, job.steps[slot].x), 0.0);
    }
    let altitude = medium.altitude;
    let direction = medium.direction;
    let radius = max(length(to_local(point)), 1e-5);
    let noise = probe_noise_value(direction, altitude);
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let lobed = clamp((footprint + noise - 1.0) * params.coverage_gain, 0.0, 1.0);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, altitude);
    out[row + 7u] = vec4<f32>(radius, noise, floor_here, under_top);
    out[row + 8u] = vec4<f32>(footprint, lobed, floor_here * under_top * lobed, ceiling);
    for (var slot = 0u; slot < 5u; slot += 1u) {
        out[row + 9u + slot] = vec4<f32>(simple_noise_fd(point, job.steps[slot].x), 0.0);
    }
    out[row + 9u].w = probe_noise_margin(direction, params.detail_scale, params.seed, altitude, 1u);
    for (var kind = 0u; kind < 5u; kind += 1u) {
        out[row + 14u + kind] = vec4<f32>(simple_noise_axis(direction, altitude, radius, kind), 0.0);
    }
    let checked = vec3<f32>(
        stencil_check_axis(point, 0u, job.steps[0].x),
        stencil_check_axis(point, 1u, job.steps[0].x),
        stencil_check_axis(point, 2u, job.steps[0].x),
    );
    let exact = vec3<f32>(
        4.0 * point.x * point.x * point.x + 5.0 * point.y * point.z,
        6.0 * point.y * point.y + 5.0 * point.x * point.z,
        -2.0 * point.z + 5.0 * point.x * point.y,
    );
    out[row + 19u] = vec4<f32>(checked - exact, 0.0);
    out[row + 20u] = vec4<f32>(probe_lattice_margin(direction, altitude), noise, 0.0, 0.0);
    frozen_cover = cover;
    frozen_altitude = altitude;
    frozen_noise = noise;
    for (var which = 0u; which < 3u; which += 1u) {
        out[row + 21u + which] = vec4<f32>(channel_fd(point, job.steps[0].x, which), 0.0);
    }
    let cover_sample = coverage_gradient_of(direction);
    let billow = billows_along(direction, altitude, true);
    let partials = shape_of_partials(cover, altitude, billow.x);
    out[row + 24u] = vec4<f32>(partials.altitude * direction / span(), 0.0);
    out[row + 25u] = vec4<f32>((partials.noise / radius) * billow.yzw, 0.0);
    out[row + 26u] = vec4<f32>(
        (partials.cover / radius) * project_tangential(cover_sample.gba, direction),
        0.0,
    );
    out[row + 27u] = vec4<f32>(
        billow.x,
        noise,
        abs(billow.x - noise),
        abs(field - shape_of(cover, altitude, noise)),
    );
    let upper_live = select(
        0.0,
        params.top * params.detail_strength,
        ceiling > params.base + 0.02,
    );
    let candidate_noise = gate_open(floor_here * under_top * lobed)
        * floor_here
        * (
            under_top * params.coverage_gain * gate_open(lobed)
                + lobed * upper_live * slope_of_smoothstep(ceiling, ceiling + 0.20, altitude)
        )
        / max(1.0 - params.erode, 1e-4);
    out[row + 28u] = vec4<f32>((candidate_noise / radius) * billow.yzw, 0.0);
    let raw_baked = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0);
    let raw_normalized = clamp(
        (raw_baked.r - params.coverage) / max(1.0 - params.coverage, 1e-4),
        0.0,
        1.0,
    );
    let raw_slope = select(
        0.0,
        6.0 * raw_normalized * (1.0 - raw_normalized) / 0.45,
        raw_normalized > 0.0 && raw_normalized < 1.0,
    );
    let bad_cover = raw_slope
        * raw_baked.g
        / max(1.0 - params.coverage, 1e-4)
        * project_tangential(vec3<f32>(raw_baked.g, raw_baked.b, raw_baked.a), direction);
    out[row + 29u] = vec4<f32>((partials.cover / radius) * bad_cover, raw_normalized);
    for (var which = 0u; which < 3u; which += 1u) {
        out[row + 30u + which] = vec4<f32>(channel_fd(point, job.steps[1].x, which), 0.0);
    }
    let under_gate = gate_open(under_top);
    out[row + 33u] = vec4<f32>(
        abs(under_gate - 1.0) * abs(lobed * upper_live * slope_of_smoothstep(ceiling, ceiling + 0.20, altitude)),
        partials.cover,
        0.0,
        0.0,
    );
    out[row + 34u] = vec4<f32>(sampled_mask_fd(point, job.steps[0].x), 0.0);
    let cover_step = 1e-4;
    let numeric_cover = (shape_of(cover + cover_step, altitude, noise)
        - shape_of(cover - cover_step, altitude, noise)) / (2.0 * cover_step);
    out[row + 35u] = vec4<f32>(raw_baked.r, raw_baked.g, numeric_cover, 0.0);
    out[row + 36u] = vec4<f32>(sampled_mask_fd(point, job.steps[1].x), 0.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Job {
    head: [u32; 4],
    steps: [[f32; 4]; STEPS],
    points: [[f32; 4]; POINTS],
}

struct Row {
    analytic: [f32; 3],
    margin: f32,
    field: f32,
    cover: f32,
    altitude: f32,
    step: [[f32; 3]; STEPS],
    radius: f32,
    noise: f32,
    floor_here: f32,
    under_top: f32,
    footprint: f32,
    lobed: f32,
    shape: f32,
    ceiling: f32,
    noise_fd: [[f32; 3]; STEPS],
    noise_margin: f32,
    axis: [[f32; 3]; CANDIDATES],
    stencil_error: [f32; 3],
    lattice_margin: f32,
    channel: [[f32; 3]; 3],
    term: [[f32; 3]; 3],
    noise_pair: [f32; 4],
    noise_candidate: [f32; 3],
    channel_fine: [[f32; 3]; 3],
    cover_bad: [f32; 3],
    cover_normalized: f32,
    under_gate_gap: f32,
    cover_partial: f32,
    mask_fd: [f32; 3],
    raw_sample: [f32; 4],
    mask_fd_fine: [f32; 3],
}

impl Row {
    fn fd(&self, slot: usize) -> [f32; 3] {
        self.step[slot]
    }
}

fn params_bytes(params: &CloudParams) -> Vec<u8> {
    let mut buffer = encase::UniformBuffer::new(Vec::new());
    buffer.write(params).expect("写不进 params");
    buffer.into_inner()
}

fn production_params() -> CloudParams {
    CloudParams::new(CLOUD_BASE, CLOUD_TOP, 900.0)
}

fn simple_params() -> CloudParams {
    let mut params = CloudParams::new(SIMPLE_INNER, SIMPLE_OUTER, 900.0);
    params.coverage = 0.45;
    params.base = 1.20;
    params.top = 0.60;
    params.detail_scale = SIMPLE_DETAIL;
    params.detail_strength = 0.50;
    params.taper = 0.05;
    params.coverage_gain = 1.20;
    params
}

fn coverage_mask(direction: [f32; 3]) -> f32 {
    return 0.62 + 0.18 * (direction[0] * 0.6 + direction[1] * 0.5 + direction[2] * 0.62);
}

fn coverage_mask_gradient() -> [f32; 3] {
    return [0.108, 0.09, 0.1116];
}

fn half_from_f32(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;
    if exponent == 0xff {
        let payload = if mantissa == 0 { 0 } else { 0x0200 };
        return sign | 0x7c00 | payload;
    }
    let unbiased = exponent - 127 + 15;
    if unbiased >= 0x1f {
        return sign | 0x7c00;
    }
    if unbiased <= 0 {
        if unbiased < -10 {
            return sign;
        }
        let mantissa = mantissa | 0x0080_0000;
        let shift = (14 - unbiased) as u32;
        let mut half = (mantissa >> shift) as u16;
        if (mantissa >> (shift - 1)) & 1 == 1 {
            half += 1;
        }
        return sign | half;
    }
    let mut half = ((unbiased as u32) << 10) as u16 | (mantissa >> 13) as u16;
    if mantissa & 0x1000 != 0 {
        half += 1;
    }
    sign | half
}

fn coverage_cube(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mask: Mask,
) -> (wgpu::Texture, wgpu::TextureView) {
    let mut pixels = Vec::with_capacity((CUBE_FACE * CUBE_FACE * 6) as usize * 16);
    for face in 0..6u32 {
        for y in 0..CUBE_FACE {
            for x in 0..CUBE_FACE {
                let u = (x as f32 + 0.5) / CUBE_FACE as f32 * 2.0 - 1.0;
                let v = (y as f32 + 0.5) / CUBE_FACE as f32 * 2.0 - 1.0;
                let axis = match face {
                    0 => [1.0, -v, -u],
                    1 => [-1.0, -v, u],
                    2 => [u, 1.0, v],
                    3 => [u, -1.0, -v],
                    4 => [u, -v, 1.0],
                    _ => [-u, -v, -1.0],
                };
                let length =
                    (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
                let direction = [axis[0] / length, axis[1] / length, axis[2] / length];
                let (baked, slope) = match mask {
                    Mask::Varying => (coverage_mask(direction), coverage_mask_gradient()),
                    Mask::Constant(value) => (value, [0.0, 0.0, 0.0]),
                };
                for channel in [baked, slope[0], slope[1], slope[2]] {
                    pixels.extend_from_slice(&channel.to_le_bytes());
                }
            }
        }
    }
    let size = wgpu::Extent3d {
        width: CUBE_FACE,
        height: CUBE_FACE,
        depth_or_array_layers: 6,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("coverage"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(CUBE_FACE * 16),
            rows_per_image: Some(CUBE_FACE),
        },
        size,
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    });
    (texture, view)
}

fn probe(
    points: &[[f32; 3]],
    params: &CloudParams,
    sweep: [f32; STEPS],
    mask: Mask,
) -> Vec<Row> {
    let Some(gpu) = connect() else {
        return Vec::new();
    };
    let device = &gpu.device;
    println!("适配器：{:?}", gpu.adapter.get_info());

    let mut source = assemble("clouds.wgsl");
    source.push_str(&job_group_source(PROBE));
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cloud gradient probe"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });

    let bytes = params_bytes(params);
    let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&params_buffer, 0, &bytes);

    let (_texture, cube) = coverage_cube(device, &gpu.queue, mask);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let material = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("material"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let job_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("job"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let material_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("material"),
        layout: &material,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&cube),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let mut job = Job {
        head: [points.len() as u32, 0, 0, 0],
        steps: [[0.0; 4]; STEPS],
        points: [[0.0; 4]; POINTS],
    };
    for (slot, step) in job.steps.iter_mut().zip(sweep) {
        slot[0] = step;
    }
    for (slot, point) in job.points.iter_mut().zip(points) {
        *slot = [point[0], point[1], point[2], 0.0];
    }
    let job_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("job"),
        size: std::mem::size_of::<Job>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue
        .write_buffer(&job_buffer, 0, bytemuck::bytes_of(&job));

    let rows = POINTS * BLOCK;
    let out_size = (rows * 16) as u64;
    let out_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("out"),
        size: out_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: out_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let job_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("job"),
        layout: &job_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: job_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: out_buffer.as_entire_binding(),
            },
        ],
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("probe"),
        bind_group_layouts: &[None, None, None, Some(&material), Some(&job_layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("probe"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("gradient_probe"),
        compilation_options: Default::default(),
        cache: None,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("probe"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("probe"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(MATERIAL_BIND_GROUP, &material_group, &[]);
        pass.set_bind_group(JOB_BIND_GROUP, &job_group, &[]);
        pass.dispatch_workgroups((POINTS as u32) / 64, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&out_buffer, 0, &staging, 0, out_size);
    gpu.queue.submit([encoder.finish()]);

    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(120)),
        })
        .expect("等待 GPU 超时");
    receiver
        .recv()
        .expect("映射没有回调")
        .expect("映射缓冲区失败");
    let data = slice.get_mapped_range();
    let values: Vec<[f32; 4]> =
        bytemuck::allocation::pod_collect_to_vec(&data[..points.len() * BLOCK * 16]);
    drop(data);
    staging.unmap();

    values
        .chunks_exact(BLOCK)
        .take(POINTS)
        .map(|row| Row {
            analytic: [row[0][0], row[0][1], row[0][2]],
            margin: row[0][3],
            field: row[1][0],
            cover: row[1][1],
            altitude: row[1][2],
            step: [
                [row[2][0], row[2][1], row[2][2]],
                [row[3][0], row[3][1], row[3][2]],
                [row[4][0], row[4][1], row[4][2]],
                [row[5][0], row[5][1], row[5][2]],
                [row[6][0], row[6][1], row[6][2]],
            ],
            radius: row[7][0],
            noise: row[7][1],
            floor_here: row[7][2],
            under_top: row[7][3],
            footprint: row[8][0],
            lobed: row[8][1],
            shape: row[8][2],
            ceiling: row[8][3],
            noise_fd: [
                [row[9][0], row[9][1], row[9][2]],
                [row[10][0], row[10][1], row[10][2]],
                [row[11][0], row[11][1], row[11][2]],
                [row[12][0], row[12][1], row[12][2]],
                [row[13][0], row[13][1], row[13][2]],
            ],
            noise_margin: row[9][3],
            axis: [
                [row[14][0], row[14][1], row[14][2]],
                [row[15][0], row[15][1], row[15][2]],
                [row[16][0], row[16][1], row[16][2]],
                [row[17][0], row[17][1], row[17][2]],
                [row[18][0], row[18][1], row[18][2]],
            ],
            stencil_error: [row[19][0], row[19][1], row[19][2]],
            lattice_margin: row[20][0],
            channel: [
                [row[21][0], row[21][1], row[21][2]],
                [row[22][0], row[22][1], row[22][2]],
                [row[23][0], row[23][1], row[23][2]],
            ],
            term: [
                [row[24][0], row[24][1], row[24][2]],
                [row[25][0], row[25][1], row[25][2]],
                [row[26][0], row[26][1], row[26][2]],
            ],
            noise_pair: [row[27][0], row[27][1], row[27][2], row[27][3]],
            noise_candidate: [row[28][0], row[28][1], row[28][2]],
            channel_fine: [
                [row[30][0], row[30][1], row[30][2]],
                [row[31][0], row[31][1], row[31][2]],
                [row[32][0], row[32][1], row[32][2]],
            ],
            cover_bad: [row[29][0], row[29][1], row[29][2]],
            cover_normalized: row[29][3],
            under_gate_gap: row[33][0],
            cover_partial: row[33][1],
            mask_fd: [row[34][0], row[34][1], row[34][2]],
            raw_sample: [row[35][0], row[35][1], row[35][2], row[35][3]],
            mask_fd_fine: [row[36][0], row[36][1], row[36][2]],
        })
        .collect()
}

fn shell_points_between(inner: f32, outer: f32) -> Vec<[f32; 3]> {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f32 / (1u64 << 53) as f32
    };
    (0..POINTS)
        .map(|_| {
            let z = next() * 2.0 - 1.0;
            let phi = next() * std::f32::consts::TAU;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let radius = inner + (outer - inner) * next();
            [
                ring * phi.cos() * radius,
                z * radius,
                ring * phi.sin() * radius,
            ]
        })
        .collect()
}

fn shell_points() -> Vec<[f32; 3]> {
    shell_points_between(CLOUD_BASE, CLOUD_TOP)
}

fn simple_points() -> Vec<[f32; 3]> {
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f32 / (1u64 << 53) as f32
    };
    let mut points: Vec<[f32; 3]> = Vec::new();
    for altitude in [0.25_f32, 0.45, 0.65] {
        let radius = SIMPLE_INNER + (SIMPLE_OUTER - SIMPLE_INNER) * altitude;
        for direction in [
            [1.0_f32, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [-1.0, 0.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, -1.0],
        ] {
            points.push([
                direction[0] * radius,
                direction[1] * radius,
                direction[2] * radius,
            ]);
        }
    }
    while points.len() < POINTS {
        let z = next() * 1.6 - 0.8;
        let phi = next() * std::f32::consts::TAU;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        let altitude = 0.2 + 0.6 * next();
        let radius = SIMPLE_INNER + (SIMPLE_OUTER - SIMPLE_INNER) * altitude;
        points.push([
            ring * phi.cos() * radius,
            z * radius,
            ring * phi.sin() * radius,
        ]);
    }
    points.truncate(POINTS);
    points
}

fn distance(fd: &[f32; 3], analytic: &[f32; 3]) -> f32 {
    let mut worst = 0.0_f32;
    for axis in 0..3 {
        worst = worst.max((fd[axis] - analytic[axis]).abs());
    }
    worst
}

fn magnitude(values: &[f32; 3]) -> f32 {
    (values[0] * values[0] + values[1] * values[1] + values[2] * values[2]).sqrt()
}

fn median(values: &mut [f32]) -> f32 {
    values.sort_by(|one, two| one.partial_cmp(two).expect("误差里出现了 NaN"));
    values[values.len() / 2]
}

fn best_of(rows: &[Row], step: usize) -> (Vec<f32>, Vec<f32>) {
    let mut errors = Vec::new();
    let mut outside = Vec::new();
    for row in rows {
        let error = distance(&row.fd(step), &row.analytic);
        if row.field > 1e-3 && row.margin > KINK_FACTOR * SWEEP[step] {
            errors.push(error);
        } else {
            outside.push(error);
        }
    }
    (errors, outside)
}

fn the_probe_harness_runs_the_production_shader_headless() {
    let points = shell_points();
    let rows = probe(&points, &production_params(), SWEEP, Mask::Varying);
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let live = rows.iter().filter(|row| row.field > 1e-4).count();
    assert!(
        live > POINTS / 4,
        "只有 {live} 个采样点落在云里，探针没测到场"
    );

    let (mut coarse, _) = best_of(&rows, 0);
    let (mut fine, _) = best_of(&rows, 1);
    assert!(
        coarse.len() >= 4,
        "探针能用的点只有 {} 个，这个测试没在测东西",
        coarse.len()
    );
    let coarse_median = median(&mut coarse);
    let fine_median = median(&mut fine);
    println!(
        "壳内 {live} / {}；差商中位偏差 h={:e} 时 {coarse_median:e}，h={:e} 时 {fine_median:e}",
        rows.len(),
        SWEEP[0],
        SWEEP[1],
    );
    println!("探针能跑、能回读、能落进壳里 —— 梯度对不对由 field_dual 的 arbiter 断言，这里不断言收敛");
}

fn the_analytic_gradient_keeps_the_kink_convention() {
    let mut points = shell_points();
    for radius in [CLOUD_BASE, CLOUD_TOP] {
        for direction in [[1.0_f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            points.push([
                direction[0] * radius,
                direction[1] * radius,
                direction[2] * radius,
            ]);
        }
    }
    points.truncate(POINTS);
    let rows = probe(&points, &production_params(), SWEEP, Mask::Varying);
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), points.len(), "回读的点数不对");

    let mut zeroed = 0_usize;
    let mut live = 0_usize;
    let mut worst_flat = 0.0_f32;
    for row in &rows {
        if row.field > 1e-4 {
            live += 1;
            continue;
        }
        assert_eq!(
            magnitude(&row.analytic),
            0.0,
            "场值 {:e} 被门归零了，解析梯度却是 {:?}",
            row.field,
            row.analytic,
        );
        zeroed += 1;
        worst_flat = worst_flat.max(magnitude(&row.analytic));
    }

    println!("壳内点 {live} 个；被门归零且解析梯度为零 {zeroed} 个（最大 |解析梯度| {worst_flat:e}）");
    assert!(zeroed > 0, "这组点里没有一个被门归零，测试没测到约定");
    assert!(live > 0, "这组点里没有一个落在云里，测试没测到场");
}

const CANDIDATE_NAMES: [&str; CANDIDATES] = [
    "xi*P_t*g + r*A*(d.g)*d",
    "P_t*g - a*A*(d.g)*d",
    "A*P_t*g + r*A*(d.g)*d",
    "xi*P_t*g + a*A*(d.g)*d",
    "xi*P_t*g + a*along*(d.g)*d",
];

fn relative_error(predicted: &[f32; 3], oracle: &[f32; 3]) -> f32 {
    let mut worst = 0.0_f32;
    let mut scale = 0.0_f32;
    for axis in 0..3 {
        worst = worst.max((predicted[axis] - oracle[axis]).abs());
        scale = scale.max(oracle[axis].abs());
    }
    if scale <= 1e-6 {
        return 0.0;
    }
    worst / scale
}

fn noise_oracle(row: &Row, slot: usize) -> [f32; 3] {
    [
        row.radius * row.noise_fd[slot][0],
        row.radius * row.noise_fd[slot][1],
        row.radius * row.noise_fd[slot][2],
    ]
}

fn measured_rows(rows: &[Row], sweep: [f32; STEPS], factor: f32) -> Vec<&Row> {
    rows.iter()
        .filter(|row| {
            row.field > 1e-3
                && row.margin > factor * sweep[0]
                && row.noise_margin > factor * sweep[0]
        })
        .collect()
}

fn simple_rows(rows: &[Row]) -> Vec<&Row> {
    measured_rows(rows, SIMPLE_SWEEP, KINK_FACTOR)
}

fn the_five_point_stencil_reproduces_a_known_derivative() {
    let points = simple_points();
    let rows = probe(&points, &simple_params(), SIMPLE_SWEEP, Mask::Constant(SIMPLE_MASK));
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let mut worst = 0.0_f32;
    for row in &rows {
        for axis in 0..3 {
            worst = worst.max(row.stencil_error[axis].abs());
        }
    }
    println!(
        "五点模板 (f(-2h) - 8f(-h) + 8f(+h) - f(+2h)) / 12h 对四次多项式的最大偏差：{worst:e}（h={:e}）",
        SIMPLE_SWEEP[0],
    );
    assert!(
        worst < 1e-3,
        "五点模板没有复现已知导数（最大偏差 {worst:e}）⇒ 差分符号错了，先修 oracle"
    );
}

fn the_simplified_field_keeps_every_gate_open() {
    let points = simple_points();
    let rows = probe(&points, &simple_params(), SIMPLE_SWEEP, Mask::Constant(SIMPLE_MASK));
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let params = simple_params();
    let mut floor_low = f32::MAX;
    let mut floor_high = f32::MIN;
    let mut footprint_low = f32::MAX;
    let mut lobed_low = f32::MAX;
    let mut lobed_high = f32::MIN;
    let mut shape_low = f32::MAX;
    let mut shape_high = f32::MIN;
    let mut noise_low = f32::MAX;
    let mut noise_high = f32::MIN;
    let mut margin_low = f32::MAX;
    let mut noise_margin_low = f32::MAX;
    let mut tally = 0_usize;

    for row in simple_rows(&rows) {
        tally += 1;
        assert!(
            row.floor_here > 0.05 && row.floor_here < 0.95,
            "floor_here = {:e} 夹住了（altitude {:e}）",
            row.floor_here,
            row.altitude,
        );
        assert_eq!(
            row.under_top, 1.0,
            "这一组配置里 under_top 必须正好是 1（ceiling {:e}）",
            row.ceiling,
        );
        assert_eq!(
            row.ceiling,
            params.base + 0.02,
            "ceiling 的 max 应当正好由 base + 0.02 胜出",
        );
        assert!(
            row.footprint > SIMPLE_MARGIN,
            "footprint = {:e} 贴住了 max(..., 0)",
            row.footprint,
        );
        assert!(
            row.lobed > SIMPLE_MARGIN && row.lobed < 1.0 - SIMPLE_MARGIN,
            "lobed = {:e} 贴住了 clamp(..., 0, 1)",
            row.lobed,
        );
        assert!(
            row.shape > SIMPLE_MARGIN && row.shape < 1.0 - SIMPLE_MARGIN,
            "shape = {:e} 贴住了 clamp(..., 0, 1)",
            row.shape,
        );
        assert!(
            row.noise > SIMPLE_MARGIN && row.noise < 1.0 - SIMPLE_MARGIN,
            "噪声值 {:e} 贴住了 billows 的 clamp",
            row.noise,
        );
        floor_low = floor_low.min(row.floor_here);
        floor_high = floor_high.max(row.floor_here);
        footprint_low = footprint_low.min(row.footprint);
        lobed_low = lobed_low.min(row.lobed);
        lobed_high = lobed_high.max(row.lobed);
        shape_low = shape_low.min(row.shape);
        shape_high = shape_high.max(row.shape);
        noise_low = noise_low.min(row.noise);
        noise_high = noise_high.max(row.noise);
        margin_low = margin_low.min(row.margin);
        noise_margin_low = noise_margin_low.min(row.noise_margin);
    }

    println!(
        "可用点 {tally} / {}；floor_here [{floor_low:e}, {floor_high:e}]；\
         footprint 最小 {footprint_low:e}；lobed [{lobed_low:e}, {lobed_high:e}]；\
         shape [{shape_low:e}, {shape_high:e}]；噪声 [{noise_low:e}, {noise_high:e}]；\
         门限最小值 {margin_low:e}，噪声夹持门限最小值 {noise_margin_low:e}，最大步长 {:e}",
        rows.len(),
        SIMPLE_SWEEP[0],
    );
    assert!(
        tally >= 24,
        "只有 {tally} 个点所有门都开着，这个配置证明不了什么"
    );
    assert!(
        margin_low > KINK_FACTOR * SIMPLE_SWEEP[0],
        "最紧的门限 {margin_low:e} 还不够大"
    );
}

fn the_noise_term_coefficient_matches_a_single_octave_oracle() {
    let points = simple_points();
    let rows = probe(&points, &simple_params(), SIMPLE_SWEEP, Mask::Constant(SIMPLE_MASK));
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let usable = simple_rows(&rows);
    println!(
        "单项噪声对拍：可用点 {} / {}，A = {:e}，span = {:e}，单八度，seed {}",
        usable.len(),
        rows.len(),
        SIMPLE_DETAIL,
        SIMPLE_OUTER - SIMPLE_INNER,
        simple_params().seed,
    );
    assert!(usable.len() >= 16, "可用点只有 {} 个", usable.len());

    let mut medians = [[0.0_f32; STEPS]; CANDIDATES];
    let mut maxima = [[0.0_f32; STEPS]; CANDIDATES];
    for (kind, name) in CANDIDATE_NAMES.iter().enumerate() {
        for (slot, step) in SIMPLE_SWEEP.iter().enumerate() {
            let mut errors: Vec<f32> = usable
                .iter()
                .map(|row| relative_error(&row.axis[kind], &noise_oracle(row, slot)))
                .collect();
            medians[kind][slot] = median(&mut errors);
            maxima[kind][slot] = errors.iter().fold(0.0_f32, |worst, value| worst.max(*value));
        }
        println!(
            "候选 {kind}（{name}）：中位 {:?}，最大 {:?}",
            medians[kind].map(|value| format!("{value:.3e}")),
            maxima[kind].map(|value| format!("{value:.3e}")),
        );
    }

    let derived = medians[0][1];
    assert!(
        derived < 1e-3,
        "导出式对中心差分的相对中位偏差 {derived:e}，太大了"
    );
    for slot in 0..STEPS - 1 {
        assert!(
            maxima[0][slot + 1] < maxima[0][slot] * 0.7,
            "导出式最坏点偏差没有随步长缩小（{:e} → {:e}）⇒ 残差不是 oracle 的截断",
            maxima[0][slot],
            maxima[0][slot + 1],
        );
    }
    for kind in 1..CANDIDATES {
        assert!(
            medians[kind][1] > derived * 10.0,
            "候选 {kind}（{}）的中位偏差 {:e} 和导出式 {:e} 分不开",
            CANDIDATE_NAMES[kind],
            medians[kind][1],
            derived,
        );
    }
    let lattice_low = usable
        .iter()
        .fold(f32::MAX, |worst, row| worst.min(row.lattice_margin));
    println!("可用点到最近晶格面的最小距离（x 单位）：{lattice_low:e}");
}

fn the_f16_coverage_bake_cannot_resolve_the_production_stencil() {
    let travel = magnitude(&coverage_mask_gradient()) * 4.0 * SWEEP[0] / CLOUD_BASE;
    let mut step = 0.5_f32;
    let mut threshold = step;
    while half_from_f32(0.5) != half_from_f32(0.5 - step) {
        threshold = step;
        step *= 0.5;
    }
    println!(
        "掩码在 4h 模板跨度上的行程 {travel:e}，f16 在 0.5 附近的舍入阈值 {threshold:e}"
    );
    assert!(
        travel < threshold,
        "f16 的舍入阈值 {threshold:e} 没有盖过掩码行程 {travel:e} ⇒ 生产那张 f16 覆盖度图在这套步长下量不出覆盖度梯度，这个结论不成立"
    );
}

fn the_coverage_term_is_measurable_and_its_scalar_is_wrong() {
    let points = shell_points();
    let rows = probe(&points, &production_params(), SWEEP, Mask::Varying);
    assert!(
        !rows.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let mut live = 0_usize;
    let mut under_gap_worst = 0.0_f32;
    let mut normalized_low = f32::MAX;
    let mut normalized_high = f32::MIN;
    let mut shader: Vec<f32> = Vec::new();
    let mut candidate: Vec<f32> = Vec::new();
    let mut ratio: Vec<f32> = Vec::new();
    let mut oracle: Vec<f32> = Vec::new();
    let mut sampler: Vec<f32> = Vec::new();
    let gradient = coverage_mask_gradient();
    for (row, point) in rows.iter().zip(points.iter()) {
        under_gap_worst = under_gap_worst.max(row.under_gate_gap);
        if row.field <= 1e-3 || row.margin <= KINK_FACTOR * SWEEP[0] {
            continue;
        }
        if row.cover_partial.abs() <= 1e-3 {
            continue;
        }
        if row.cover_normalized <= 0.02 || row.cover_normalized >= 0.43 {
            continue;
        }
        live += 1;
        normalized_low = normalized_low.min(row.cover_normalized);
        normalized_high = normalized_high.max(row.cover_normalized);
        let radius = magnitude(point);
        let direction = [point[0] / radius, point[1] / radius, point[2] / radius];
        let dot =
            gradient[0] * direction[0] + gradient[1] * direction[1] + gradient[2] * direction[2];
        let expected = [
            (gradient[0] - dot * direction[0]) / radius,
            (gradient[1] - dot * direction[1]) / radius,
            (gradient[2] - dot * direction[2]) / radius,
        ];
        oracle.push(relative_error(&row.mask_fd, &expected));
        sampler.push(relative_error(&row.mask_fd_fine, &row.mask_fd));
        if live <= 8 {
            println!(
                "点 {live}：oracle 掩码偏差 {:e}；预期 {:?}，实测 {:?}",
                oracle[live - 1], expected, row.mask_fd,
            );
        }
        let mut first = 0.0_f32;
        let mut second = 0.0_f32;
        let mut scale = 0.0_f32;
        for axis in 0..3 {
            first = first.max((row.term[2][axis] - row.channel[2][axis]).abs());
            second = second.max((row.cover_bad[axis] - row.channel[2][axis]).abs());
            scale = scale.max(row.channel[2][axis].abs());
        }
        let scale = scale.max(1e-6);
        shader.push(first / scale);
        candidate.push(second / scale);
        if live <= 8 {
            println!(
                "点 {live}：oracle {:e}，从零重算 {:e}，现式 {:e}；ρ_c 解析 {:e} 数值 {:e}（比 {:e}）",
                oracle[live - 1],
                candidate[live - 1],
                shader[live - 1],
                row.cover_partial,
                row.raw_sample[2],
                row.cover_partial / row.raw_sample[2],
            );
        }
        let here = magnitude(&row.term[2]);
        let there = magnitude(&row.cover_bad);
        if there > 1e-6 {
            ratio.push(here / there);
        }
    }
    println!(
        "覆盖度项：cover 偏导非零的点 {live} / {}，normalized 范围 [{normalized_low:e}, {normalized_high:e}]，under_live 冗余性最大出入 {under_gap_worst:e}",
        rows.len(),
    );
    assert!(live >= 8, "cover 偏导非零的点只有 {live} 个，这个测试没在测覆盖度项");
    let shader_median = median(&mut shader);
    let bad_median = median(&mut candidate);
    let oracle_median = median(&mut oracle);
    let oracle_worst = oracle.iter().fold(0.0_f32, |worst, value| worst.max(*value));
    let shader_worst = shader.iter().fold(0.0_f32, |worst, value| worst.max(*value));
    let bad_worst = candidate.iter().fold(0.0_f32, |worst, value| worst.max(*value));
    let mut ratio_sorted = ratio;
    let ratio_median = median(&mut ratio_sorted);
    println!(
        "差分 oracle 自己的掩码梯度相对偏差：中位 {oracle_median:e}，最大 {oracle_worst:e}（双线性插值对解析梯度的偏差，这就是 oracle 的分辨极限）"
    );
    println!(
        "现式 ③ 相对通道差商：中位 {shader_median:e}，最大 {shader_worst:e}；带 baked.g 的旧式：中位 {bad_median:e}，最大 {bad_worst:e}",
    );
    println!(
        "现式 ③ 与从零重算的模长之比中位 {ratio_median:e}（baked.g = {:e}）",
        coverage_mask_gradient()[0]
    );
    assert!(
        under_gap_worst == 0.0,
        "under_live 不是冗余的：最大出入 {under_gap_worst:e} ⇒ 它是有用的门"
    );
    assert!(
        bad_median > 0.5,
        "带着 baked.g 标量的旧式 ③ 相对通道差商只有 {bad_median:e}，没有明显错"
    );
    assert!(
        shader_median < bad_median * 0.5,
        "现式 ③ 并没有比带 baked.g 的旧式更贴近通道差商（{:e} 对 {:e}）⇒ 第四处不是这个标量",
        shader_median,
        bad_median,
    );
    assert!(
        (ratio_median - 1.0 / coverage_mask_gradient()[0]).abs() < 0.1,
        "旧式 ③ 与现式 ③ 的模长之比 {ratio_median:e} 不等于 1/baked.g {:e} ⇒ 结构判断有误",
        1.0 / coverage_mask_gradient()[0],
    );
    let sampler_median = median(&mut sampler);
    println!(
        "掩码差商在 h 与 h/2 之间的相对差（采样器自证）：中位 {sampler_median:e} 最大 {:e}",
        sampler.iter().fold(0.0_f32, |worst, value| worst.max(*value)),
    );
    println!(
        "现式 ③ 仍差 {shader_median:e}（中位），而差分 oracle 的分辨极限是 {oracle_median:e} 中位 / {oracle_worst:e} 最大：双线性插值在每个纹素里的斜率与解析梯度不同，这一步的残差就是它，不是公式"
    );
}

fn the_residual_is_attributed_to_one_channel() {
    let simple = probe(&simple_points(), &simple_params(), SIMPLE_SWEEP, Mask::Constant(SIMPLE_MASK));
    assert!(
        !simple.is_empty(),
        "探针没拿到数据（设备/管线失败）——不要把它读成通过"
    );
    assert_eq!(simple.len(), POINTS, "回读的点数不对");
    let production = probe(&shell_points(), &production_params(), SWEEP, Mask::Varying);
    assert_eq!(production.len(), POINTS, "回读的点数不对");
    let flat = probe(&shell_points(), &production_params(), SWEEP, Mask::Constant(PRODUCTION_MASK));
    assert_eq!(flat.len(), POINTS, "回读的点数不对");

    attribute("简化", &simple, SIMPLE_SWEEP, KINK_FACTOR);
    attribute("生产", &production, SWEEP, KINK_FACTOR);
    attribute("生产·定覆盖度", &flat, SWEEP, KINK_FACTOR);
    attribute("生产·定覆盖度·严门限", &flat, SWEEP, 20.0 * KINK_FACTOR);
    let mut near_params = production_params();
    near_params.inner = 0.01;
    near_params.outer = 0.06;
    let near = probe(
        &shell_points_between(0.01, 0.06),
        &near_params,
        SWEEP,
        Mask::Constant(PRODUCTION_MASK),
    );
    assert_eq!(near.len(), POINTS, "回读的点数不对");
    attribute("生产·小半径·定覆盖度", &near, SWEEP, KINK_FACTOR);
}

fn attribute(label: &str, rows: &[Row], sweep: [f32; STEPS], factor: f32) {
    let usable = measured_rows(rows, sweep, factor);
    println!(
        "[{label}] 逐通道归因：可用点 {} / {}，步长 {:e}",
        usable.len(),
        rows.len(),
        sweep[0],
    );
    assert!(usable.len() >= 8, "[{label}] 可用点只有 {} 个", usable.len());

    let names = ["高度 ①", "噪声 ②", "覆盖 ③"];
    let mut mirror_worst = 0.0_f32;
    let mut split_worst = 0.0_f32;
    let mut pair_worst = 0.0_f32;
    let mut face_worst = 0.0_f32;
    let mut chan_medians = [[0.0_f32; 3]; 3];
    let mut chan_maxima = [[0.0_f32; 3]; 3];
    for which in 0..3 {
        let mut residuals: Vec<f32> = usable
            .iter()
            .map(|row| {
                let mut error = 0.0_f32;
                for axis in 0..3 {
                    error = error
                        .max((row.term[which][axis] - row.channel[which][axis]).abs());
                }
                error
            })
            .collect();
        chan_medians[which][0] = median(&mut residuals);
        chan_maxima[which][0] = residuals
            .iter()
            .fold(0.0_f32, |worst, value| worst.max(*value));
        let worst_row = usable
            .iter()
            .max_by(|one, two| {
                let mut first = 0.0_f32;
                let mut second = 0.0_f32;
                for axis in 0..3 {
                    first = first
                        .max((one.term[which][axis] - one.channel[which][axis]).abs());
                    second = second
                        .max((two.term[which][axis] - two.channel[which][axis]).abs());
                }
                first.partial_cmp(&second).expect("残差里出现了 NaN")
            })
            .expect("上面刚断言过非空");
        let mut relative: Vec<f32> = usable
            .iter()
            .map(|row| {
                let mut error = 0.0_f32;
                let mut scale = 1e-9_f32;
                for axis in 0..3 {
                    error = error
                        .max((row.term[which][axis] - row.channel[which][axis]).abs());
                    scale = scale.max(row.channel[which][axis].abs());
                }
                error / scale
            })
            .collect();
        println!(
            "[{label}] {which}（{}）解析项对通道差商的偏差：中位 {:e}，最大 {:e}，相对中位 {:e}",
            names[which], chan_medians[which][0], chan_maxima[which][0], median(&mut relative),
        );
        println!(
            "[{label}] {which} 最坏点：解析 {:?} 差商 {:?}（半径 {:e}，altitude {:e}）",
            worst_row.term[which], worst_row.channel[which], worst_row.radius, worst_row.altitude,
        );
        let mut finer: Vec<f32> = usable
            .iter()
            .map(|row| {
                let mut error = 0.0_f32;
                for axis in 0..3 {
                    error = error
                        .max((row.term[which][axis] - row.channel_fine[which][axis]).abs());
                }
                error
            })
            .collect();
        let finer_median = median(&mut finer);
        println!(
            "[{label}] {which} 步长减半后（{:e}）中位偏差 {:e}，与粗步长之比 {:.3}",
            sweep[1],
            finer_median,
            finer_median / chan_medians[which][0],
        );
    }
    for row in &usable {
        let mut mirrored = [0.0_f32; 3];
        let mut split = [0.0_f32; 3];
        for axis in 0..3 {
            mirrored[axis] = row.term[0][axis] + row.term[1][axis] + row.term[2][axis];
            split[axis] = row.channel[0][axis] + row.channel[1][axis] + row.channel[2][axis];
            mirror_worst = mirror_worst.max((mirrored[axis] - row.analytic[axis]).abs());
            split_worst = split_worst.max((split[axis] - row.fd(0)[axis]).abs());
        }
        pair_worst = pair_worst.max(row.noise_pair[2]);
        face_worst = face_worst.max(row.noise_pair[3]);
    }
    println!("[{label}] 解析路径的噪声值对值路径噪声值的最大出入 {pair_worst:e}");
    println!("[{label}] 探针自己算的场值对 cloud_field 的最大出入 {face_worst:e}");
    println!("[{label}] 三项之和偏离 shader 自己返回值的最大出入 {mirror_worst:e}");
    assert!(
        pair_worst < 1e-6,
        "[{label}] 解析路径的噪声值和值路径的噪声值差 {pair_worst:e} ⇒ 两边量的不是同一个场（同一函数的两条代码路径只该差 1 ULP）"
    );
    assert!(
        face_worst == 0.0,
        "[{label}] 探针算的场值和 cloud_field 差 {face_worst:e} ⇒ 差商 oracle 不是这条值路径"
    );
    assert!(
        mirror_worst < 1e-5 * (1.0 + usable.iter().fold(0.0_f32, |worst, row| {
            worst.max(magnitude(&row.analytic))
        })),
        "[{label}] 探针里复算的三项加起来和 shader 返回值差 {mirror_worst:e} ⇒ 拆项没抄对，归因无效"
    );

    let mut worst = 0_usize;
    for which in 0..3 {
        if chan_maxima[which][0] > chan_maxima[worst][0] {
            worst = which;
        }
    }
    println!(
        "[{label}] 最大残差落在通道 {worst}（{}）：{:e}，另外两个是 {:e} 和 {:e}",
        names[worst],
        chan_maxima[worst][0],
        chan_maxima[(worst + 1) % 3][0],
        chan_maxima[(worst + 2) % 3][0],
    );
    let total: f32 = usable
        .iter()
        .map(|row| distance(&row.fd(0), &row.analytic))
        .fold(0.0_f32, |worst, value| worst.max(value));
    println!("[{label}] 整场最大残差 {total:e}");
    let mut candidate_residuals: Vec<f32> = usable
        .iter()
        .map(|row| {
            let mut error = 0.0_f32;
            for axis in 0..3 {
                error = error
                    .max((row.noise_candidate[axis] - row.channel[1][axis]).abs());
            }
            error
        })
        .collect();
    let candidate_median = median(&mut candidate_residuals);
    let candidate_worst = candidate_residuals
        .iter()
        .fold(0.0_f32, |worst, value| worst.max(*value));
    println!(
        "[{label}] 从零重算的噪声项（ceiling 项取正号）对通道差商：中位 {:e}，最大 {:e}（现式 {:e} / {:e}）",
        candidate_median, candidate_worst, chan_medians[1][0], chan_maxima[1][0],
    );
    assert!(
        candidate_median < 1e-3 || chan_medians[1][0] <= candidate_median * 1.5,
        "[{label}] 从零重算的噪声项中位残差 {candidate_median:e}，shader 现式 {:e}：现式没有比从零推导的版本更好 ⇒ 噪声项还有别的错",
        chan_medians[1][0],
    );
    println!(
        "[{label}] 三条通道差商之和和整场差商的最大出入 {split_worst:e}（差商在有限步长下不是恒等式；生产配置里门限中位 8.7e-3 小于场在模板跨度上的变化 |∇|·4h≈6e-2，折点会被穿过，再加上覆盖度通道的采样器噪声，这一项不作为断言）"
    );
    let simplified = label == "简化";
    if simplified {
        assert!(
            split_worst < 0.1 * chan_maxima[worst][0],
            "[{label}] 三条通道差商之和和整场差商差 {split_worst:e}，和最大通道残差 {:e} 同量级 ⇒ 通道分解不成立，归因无效",
            chan_maxima[worst][0],
        );
    }
}


/// 全部 check，按「先便宜后贵」排。原来这些是 `#[test]`，现在由 bin 逐个跑。
pub fn checks() -> Vec<(&'static str, fn())> {
    vec![
        (
            "the_f16_coverage_bake_cannot_resolve_the_production_stencil",
            the_f16_coverage_bake_cannot_resolve_the_production_stencil,
        ),
        (
            "the_probe_harness_runs_the_production_shader_headless",
            the_probe_harness_runs_the_production_shader_headless,
        ),
        (
            "the_analytic_gradient_keeps_the_kink_convention",
            the_analytic_gradient_keeps_the_kink_convention,
        ),
        (
            "the_five_point_stencil_reproduces_a_known_derivative",
            the_five_point_stencil_reproduces_a_known_derivative,
        ),
        (
            "the_simplified_field_keeps_every_gate_open",
            the_simplified_field_keeps_every_gate_open,
        ),
        (
            "the_noise_term_coefficient_matches_a_single_octave_oracle",
            the_noise_term_coefficient_matches_a_single_octave_oracle,
        ),
        (
            "the_coverage_term_is_measurable_and_its_scalar_is_wrong",
            the_coverage_term_is_measurable_and_its_scalar_is_wrong,
        ),
        (
            "the_residual_is_attributed_to_one_channel",
            the_residual_is_attributed_to_one_channel,
        ),
    ]
}
