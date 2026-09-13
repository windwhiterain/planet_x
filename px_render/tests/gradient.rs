mod common;

use bevy::render::render_resource::ShaderType;
use common::{assemble, connect};
use px_render::clouds::{CLOUD_BASE, CLOUD_TOP, CloudParams};

const POINTS: usize = 64;
const WORKGROUP: u32 = 64;
const CUBE_FACE: u32 = 4;
const COVERAGE_VALUE: f32 = 0.62;

const PROBE: &str = r#"
struct Job {
    count: u32,
    step: f32,
    _pad: vec2<u32>,
    points: array<vec4<f32>, 64>,
};

@group(3) @binding(0) var<uniform> job: Job;
@group(3) @binding(1) var<storage, read_write> out: array<vec4<f32>>;

fn fd_gradient(point: vec3<f32>, h: f32) -> vec3<f32> {
    let x = vec3<f32>(h, 0.0, 0.0);
    let y = vec3<f32>(0.0, h, 0.0);
    let z = vec3<f32>(0.0, 0.0, h);
    return vec3<f32>(
        cloud_field(point + x) - cloud_field(point - x),
        cloud_field(point + y) - cloud_field(point - y),
        cloud_field(point + z) - cloud_field(point - z),
    ) / (2.0 * h);
}

@compute @workgroup_size(64)
fn gradient_probe(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= job.count) {
        return;
    }
    let point = job.points[index].xyz;
    out[index] = vec4<f32>(fd_gradient(point, job.step), cloud_field(point));
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Job {
    count: u32,
    step: f32,
    pad: [u32; 2],
    points: [[f32; 4]; POINTS],
}

fn params_bytes() -> Vec<u8> {
    let params = CloudParams::new(CLOUD_BASE, CLOUD_TOP, 900.0);
    let mut buffer = encase::UniformBuffer::new(Vec::new());
    buffer.write(&params).expect("写不进 params");
    buffer.into_inner()
}

fn coverage_cube(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView) {
    let texels = (CUBE_FACE * CUBE_FACE * 6) as usize;
    let mut pixels = Vec::with_capacity(texels * 4);
    for _ in 0..texels {
        let level = (COVERAGE_VALUE * 255.0).round() as u8;
        pixels.extend_from_slice(&[level, 128, 128, 255]);
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

fn probe(points: &[[f32; 3]], step: f32) -> Vec<[f32; 4]> {
    let Some(gpu) = connect() else {
        return Vec::new();
    };
    let device = &gpu.device;

    let mut source = assemble("clouds.wgsl");
    source.push_str(PROBE);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cloud gradient probe"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });

    let params = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("params"),
        size: params_bytes().len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    gpu.queue.write_buffer(&params, 0, &params_bytes());

    let (_texture, cube) = coverage_cube(device, &gpu.queue);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("empty"),
        entries: &[],
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
                resource: params.as_entire_binding(),
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
        count: points.len() as u32,
        step,
        pad: [0, 0],
        points: [[0.0; 4]; POINTS],
    };
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

    let out_size = (POINTS * 16) as u64;
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
        pass.set_bind_group(2, &material_group, &[]);
        pass.set_bind_group(3, &job_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
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
            timeout: Some(std::time::Duration::from_secs(60)),
        })
        .expect("等待 GPU 超时");
    receiver
        .recv()
        .expect("映射没有回调")
        .expect("映射缓冲区失败");
    let data = slice.get_mapped_range();
    let values: Vec<[f32; 4]> =
        bytemuck::allocation::pod_collect_to_vec(&data[..points.len() * 16]);
    drop(data);
    staging.unmap();
    values
}

fn shell_points() -> Vec<[f32; 3]> {
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
            let radius = f32::midpoint(CLOUD_BASE, CLOUD_TOP);
            [ring * phi.cos() * radius, z * radius, ring * phi.sin() * radius]
        })
        .collect()
}

#[test]
fn the_probe_harness_runs_the_production_shader_headless() {
    let points = shell_points();
    let coarse = probe(&points, 4e-3);
    if coarse.is_empty() {
        eprintln!("跳过：没有可用的 wgpu 适配器");
        return;
    }
    assert_eq!(coarse.len(), POINTS, "回读的点数不对");

    let live = coarse.iter().filter(|value| value[3] > 1e-4).count();
    assert!(
        live > POINTS / 4,
        "只有 {live} 个采样点落在云里，探针没测到场"
    );

    let fine = probe(&points, 2e-3);
    let mut coarse_change = 0.0_f32;
    let mut fine_change = 0.0_f32;
    for index in 0..POINTS {
        for axis in 0..3 {
            coarse_change = coarse_change.max(
                (fine[index][axis] - coarse[index][axis]).abs(),
            );
        }
    }

    let finer = probe(&points, 1e-3);
    for index in 0..POINTS {
        for axis in 0..3 {
            fine_change = fine_change.max((finer[index][axis] - fine[index][axis]).abs());
        }
    }

    println!("步长 4e-3→2e-3 最大变化 {coarse_change:e}，2e-3→1e-3 最大变化 {fine_change:e}");
    assert!(
        fine_change < coarse_change,
        "步长减半后差分没有收敛（{coarse_change} → {fine_change}），探针本身不可信"
    );
}
