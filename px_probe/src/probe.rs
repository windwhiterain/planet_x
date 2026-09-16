use crate::common::{
    COVERAGE_BINDING, COVERAGE_SAMPLER_BINDING, JOB_BIND_GROUP, MATERIAL_BIND_GROUP, assemble,
    connect, job_group_source,
};
use crate::params::CloudParams;

pub const POINTS: usize = 512;
pub const STEPS: usize = 5;
pub const BLOCK: usize = 14;
pub const MASK_GRADIENT: [f32; 3] = [0.108, 0.09, 0.1116];
const CUBE_FACE: u32 = 64;
const WORKGROUP: u32 = 64;

const PROBE: &str = r#"
struct Job {
    head: vec4<u32>,
    steps: array<vec4<f32>, 5>,
    points: array<vec4<f32>, 512>,
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

@compute @workgroup_size(64)
fn field_probe(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= job.head.x) {
        return;
    }
    let point = job.points[index].xyz;
    let medium = medium_of(point);
    let cover = coverage_of(medium.direction);
    let row = index * 14u;
    let analytic = cloud_field_gradient_analytic(point);
    out[row] = vec4<f32>(analytic, length(analytic));
    out[row + 1u] = vec4<f32>(cloud_field(point), cover, medium.altitude, length(to_local(point)));
    out[row + 2u] = vec4<f32>(medium.direction, 0.0);
    for (var slot = 0u; slot < 5u; slot += 1u) {
        out[row + 3u + slot] = vec4<f32>(fd_gradient(point, job.steps[slot].x), 0.0);
    }
    let noise = billows(medium.direction, medium.altitude, true);
    let height = clamp(medium.altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let lobed = clamp((footprint + noise - 1.0) * params.coverage_gain, 0.0, 1.0);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), medium.altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, medium.altitude);
    out[row + 8u] = vec4<f32>(noise, footprint, lobed, floor_here);
    out[row + 9u] = vec4<f32>(
        ceiling,
        under_top,
        shape_of(cover, medium.altitude, noise),
        coverage_gradient_of(medium.direction).r,
    );
    let tower = sampled_noise(medium.direction, medium.altitude, params.detail_scale * 0.35, 3u, params.seed);
    let skin = sampled_noise(medium.direction, medium.altitude, params.detail_scale * 1.70, 2u, params.seed ^ 31u);
    let fixed = vec3<f32>(1.3, 2.7, -0.4);
    out[row + 10u] = vec4<f32>(
        tower,
        skin,
        fbm_3(fixed, 1.0, 3u, 2.0, 0.5, 7u),
        gradient_noise_3(fixed, 7u),
    );
    out[row + 11u] = vec4<f32>(
        sampled_noise(medium.direction, medium.altitude, params.detail_scale * 0.35, 1u, params.seed),
        sampled_noise(medium.direction, medium.altitude, params.detail_scale * 1.70, 1u, params.seed ^ 31u),
        0.0,
        0.0,
    );

    let radius = max(length(to_local(point)), 1e-5);
    let billow = billows_along(medium.direction, medium.altitude, true);
    let partials = shape_of_partials(cover, medium.altitude, billow.x);
    let baked = textureSampleLevel(coverage_map, coverage_sampler, medium.direction, 0.0);
    let normalized = clamp((baked.r - params.coverage) / max(1.0 - params.coverage, 1e-4), 0.0, 1.0);
    let slope = select(
        0.0,
        6.0 * normalized * (1.0 - normalized) / 0.45,
        normalized > 0.0 && normalized < 1.0,
    );
    let chain = slope / max(1.0 - params.coverage, 1e-4);
    let tangential = project_tangential(vec3<f32>(baked.g, baked.b, baked.a), medium.direction);
    out[row + 12u] = vec4<f32>(to_world((partials.cover / radius) * chain * tangential), baked.g);
    out[row + 13u] = vec4<f32>(to_world((partials.cover / radius) * chain * baked.g * tangential), chain);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Job {
    head: [u32; 4],
    steps: [[f32; 4]; STEPS],
    points: [[f32; 4]; POINTS],
}

#[derive(Clone, Copy)]
pub enum Mask {
    Constant(f32),
    Varying,
}

pub fn level_of(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

pub fn quantised(value: f32) -> f32 {
    level_of(value) as f32 / 255.0
}

pub fn mask_of(direction: [f32; 3]) -> f32 {
    0.62 + 0.18 * (direction[0] * 0.6 + direction[1] * 0.5 + direction[2] * 0.62)
}

pub struct Row {
    pub analytic: [f32; 3],
    pub field: f32,
    pub cover: f32,
    pub altitude: f32,
    pub radius: f32,
    pub direction: [f32; 3],
    pub step: [[f32; 3]; STEPS],
    pub noise: f32,
    pub footprint: f32,
    pub lobed: f32,
    pub floor_here: f32,
    pub ceiling: f32,
    pub under_top: f32,
    pub shape: f32,
    pub analytic_cover: f32,
    pub tower: f32,
    pub skin: f32,
    pub fbm_fixed: f32,
    pub gradient_fixed: f32,
    pub tower_single: f32,
    pub skin_single: f32,
    pub new_axis: [f32; 3],
    pub old_axis: [f32; 3],
    pub baked_g: f32,
    pub chain: f32,
}

fn params_bytes(params: &CloudParams) -> Vec<u8> {
    let mut buffer = encase::UniformBuffer::new(Vec::new());
    buffer.write(params).expect("写不进 params");
    buffer.into_inner()
}

fn coverage_cube(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mask: Mask,
) -> (wgpu::Texture, wgpu::TextureView) {
    let mut pixels = Vec::with_capacity((CUBE_FACE * CUBE_FACE * 6) as usize * 4);
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
                let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
                let direction = [axis[0] / length, axis[1] / length, axis[2] / length];
                let baked = match mask {
                    Mask::Constant(value) => [value, 0.0, 0.0, 0.0],
                    Mask::Varying => [
                        mask_of(direction),
                        MASK_GRADIENT[0],
                        MASK_GRADIENT[1],
                        MASK_GRADIENT[2],
                    ],
                };
                pixels.extend_from_slice(&[
                    level_of(baked[0]),
                    level_of(baked[1]),
                    level_of(baked[2]),
                    level_of(baked[3]),
                ]);
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
        format: wgpu::TextureFormat::Rgba8Unorm,
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
            bytes_per_row: Some(CUBE_FACE * 4),
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

pub fn run(points: &[[f32; 3]], params: &CloudParams, sweep: [f32; STEPS], mask: Mask) -> Vec<Row> {
    let Some(gpu) = connect() else {
        return Vec::new();
    };
    let device = &gpu.device;
    println!("适配器：{:?}", gpu.adapter.get_info());

    let mut source = assemble("clouds.wgsl");
    source.push_str(&job_group_source(PROBE));
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cloud field probe"),
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
                binding: COVERAGE_BINDING,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::Cube,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: COVERAGE_SAMPLER_BINDING,
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
                binding: COVERAGE_BINDING,
                resource: wgpu::BindingResource::TextureView(&cube),
            },
            wgpu::BindGroupEntry {
                binding: COVERAGE_SAMPLER_BINDING,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    let mut job = Job {
        head: [points.len() as u32, 0, 0, 0],
        steps: [[0.0; 4]; STEPS],
        points: [[0.0; 4]; POINTS],
    };
    for (slot, value) in job.steps.iter_mut().zip(sweep) {
        slot[0] = value;
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

    let out_size = (POINTS * BLOCK * 16) as u64;
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
        // 5 格会超：探针设备是 downlevel_defaults（max_bind_groups = 4）⇒ 只能给 4 个布局。
        // 材质组按契约在 3；探针自己的 job/out 放 1（0 是 Bevy 的视图组，2 空着）。
        bind_group_layouts: &[None, Some(&job_layout), None, Some(&material)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("probe"),
        layout: Some(&layout),
        module: &module,
        entry_point: Some("field_probe"),
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
        pass.dispatch_workgroups((points.len() as u32).div_ceil(WORKGROUP), 1, 1);
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
        .map(|row| Row {
            analytic: [row[0][0], row[0][1], row[0][2]],
            field: row[1][0],
            cover: row[1][1],
            altitude: row[1][2],
            radius: row[1][3],
            direction: [row[2][0], row[2][1], row[2][2]],
            step: [
                [row[3][0], row[3][1], row[3][2]],
                [row[4][0], row[4][1], row[4][2]],
                [row[5][0], row[5][1], row[5][2]],
                [row[6][0], row[6][1], row[6][2]],
                [row[7][0], row[7][1], row[7][2]],
            ],
            noise: row[8][0],
            footprint: row[8][1],
            lobed: row[8][2],
            floor_here: row[8][3],
            ceiling: row[9][0],
            under_top: row[9][1],
            shape: row[9][2],
            analytic_cover: row[9][3],
            tower: row[10][0],
            skin: row[10][1],
            fbm_fixed: row[10][2],
            gradient_fixed: row[10][3],
            tower_single: row[11][0],
            skin_single: row[11][1],
            new_axis: [row[12][0], row[12][1], row[12][2]],
            baked_g: row[12][3],
            old_axis: [row[13][0], row[13][1], row[13][2]],
            chain: row[13][3],
        })
        .collect()
}
