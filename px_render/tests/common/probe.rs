use crate::common::{assemble, connect};
use px_render::clouds::CloudParams;

pub const POINTS: usize = 512;
pub const STEPS: usize = 5;
pub const BLOCK: usize = 12;
const CUBE_FACE: u32 = 64;
const WORKGROUP: u32 = 64;

const PROBE: &str = r#"
struct Job {
    head: vec4<u32>,
    steps: array<vec4<f32>, 5>,
    points: array<vec4<f32>, 512>,
};

@group(3) @binding(0) var<uniform> job: Job;
@group(3) @binding(1) var<storage, read_write> out: array<vec4<f32>>;

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
    let row = index * 12u;
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
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Job {
    head: [u32; 4],
    steps: [[f32; 4]; STEPS],
    points: [[f32; 4]; POINTS],
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
}

fn params_bytes(params: &CloudParams) -> Vec<u8> {
    let mut buffer = encase::UniformBuffer::new(Vec::new());
    buffer.write(params).expect("写不进 params");
    buffer.into_inner()
}

fn coverage_cube(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mask: f32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let level = (mask * 255.0).round().clamp(0.0, 255.0) as u8;
    let mut pixels = Vec::with_capacity((CUBE_FACE * CUBE_FACE * 6) as usize * 4);
    for _ in 0..CUBE_FACE * CUBE_FACE * 6 {
        pixels.extend_from_slice(&[level, 0, 0, 255]);
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

pub fn run(points: &[[f32; 3]], params: &CloudParams, sweep: [f32; STEPS], mask: f32) -> Vec<Row> {
    let Some(gpu) = connect() else {
        return Vec::new();
    };
    let device = &gpu.device;
    println!("适配器：{:?}", gpu.adapter.get_info());

    let mut source = assemble("clouds.wgsl");
    source.push_str(PROBE);
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
        bind_group_layouts: &[None, None, Some(&material), Some(&job_layout)],
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
        pass.set_bind_group(2, &material_group, &[]);
        pass.set_bind_group(3, &job_group, &[]);
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
        })
        .collect()
}
