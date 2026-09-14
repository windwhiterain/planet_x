mod common;

use bevy::render::render_resource::ShaderType;
use common::{assemble, connect};
use px_render::clouds::{CLOUD_BASE, CLOUD_TOP, CloudParams};

const POINTS: usize = 64;
const STEPS: usize = 5;
const CUBE_FACE: u32 = 64;
const KINK_FACTOR: f32 = 8.0;
const SWEEP: [f32; STEPS] = [8e-5, 4e-5, 2e-5, 1e-5, 5e-6];

const PROBE: &str = r#"
struct Job {
    head: vec4<u32>,
    steps: array<vec4<f32>, 5>,
    points: array<vec4<f32>, 64>,
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

fn probe_octave_margin(point: vec3<f32>, seed: u32, octaves: u32) -> f32 {
    var current = 1.0;
    var margin = 1e9;
    for (var octave = 0u; octave < octaves; octave += 1u) {
        let sample = gradient_noise_3_grad(point * current, seed ^ octave);
        let raw = sample.value * 2.0 - 1.0;
        margin = min(margin, min(abs(raw), abs(1.0 - raw)) * current);
        current *= 2.0;
    }
    return margin;
}

fn probe_noise_margin(direction: vec3<f32>, across: f32, seed: u32, altitude: f32) -> f32 {
    return probe_octave_margin(direction * (across + altitude * across * span()), seed, 3u);
}

fn probe_gate_margin(point: vec3<f32>, cover: f32) -> f32 {
    let medium = medium_of(point);
    let altitude = medium.altitude;
    let direction = medium.direction;
    var margin = min(altitude, 1.0 - altitude);
    let tower_across = params.detail_scale * 0.35;
    let skin_across = params.detail_scale * 1.70;
    margin = min(margin, probe_noise_margin(direction, tower_across, params.seed, altitude));
    margin = min(margin, probe_noise_margin(direction, skin_across, params.seed ^ 31u, altitude));
    if params.ablate == ABLATE_FETCH || params.ablate == ABLATE_NOISE {
        return margin;
    }
    let tower = sampled_noise_along(direction, tower_across, 3u, params.seed, altitude);
    let skin = sampled_noise_along(direction, skin_across, 2u, params.seed ^ 31u, altitude);
    let blended = tower.value * 0.62 + skin.value * 0.38;
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    margin = min(margin, footprint / max(params.coverage_gain, 1e-4));
    let lobed = clamp((footprint + blended - 1.0) * params.coverage_gain, 0.0, 1.0);
    margin = min(margin, min(lobed, 1.0 - lobed) / params.coverage_gain);
    let shape = shape_of(cover, altitude, blended);
    return min(margin, min(shape, 1.0 - shape) * max(1.0 - params.erode, 1e-4));
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
    let row = index * (2u + 5u);
    out[row] = vec4<f32>(analytic, probe_gate_margin(point, cover));
    out[row + 1u] = vec4<f32>(field, cover, medium.altitude, length(analytic));
    for (var slot = 0u; slot < 5u; slot += 1u) {
        out[row + 2u + slot] = vec4<f32>(fd_gradient(point, job.steps[slot].x), 0.0);
    }
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
}

impl Row {
    fn fd(&self, slot: usize) -> [f32; 3] {
        self.step[slot]
    }
}

fn params_bytes() -> Vec<u8> {
    let params = CloudParams::new(CLOUD_BASE, CLOUD_TOP, 900.0);
    let mut buffer = encase::UniformBuffer::new(Vec::new());
    buffer.write(&params).expect("写不进 params");
    buffer.into_inner()
}

fn coverage_mask(direction: [f32; 3]) -> f32 {
    return 0.62 + 0.18 * (direction[0] * 0.6 + direction[1] * 0.5 + direction[2] * 0.62);
}

fn coverage_cube(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView) {
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
                let length =
                    (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
                let direction = [axis[0] / length, axis[1] / length, axis[2] / length];
                let level = (coverage_mask(direction) * 255.0).round().clamp(0.0, 255.0) as u8;
                pixels.extend_from_slice(&[level, 0, 0, 255]);
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

fn probe(points: &[[f32; 3]]) -> Vec<Row> {
    let Some(gpu) = connect() else {
        return Vec::new();
    };
    let device = &gpu.device;
    println!("适配器：{:?}", gpu.adapter.get_info());

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
        head: [points.len() as u32, 0, 0, 0],
        steps: [[0.0; 4]; STEPS],
        points: [[0.0; 4]; POINTS],
    };
    for (slot, step) in job.steps.iter_mut().zip(SWEEP) {
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

    let rows = POINTS * (2 + STEPS);
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
            timeout: Some(std::time::Duration::from_secs(120)),
        })
        .expect("等待 GPU 超时");
    receiver
        .recv()
        .expect("映射没有回调")
        .expect("映射缓冲区失败");
    let data = slice.get_mapped_range();
    let values: Vec<[f32; 4]> =
        bytemuck::allocation::pod_collect_to_vec(&data[..points.len() * (2 + STEPS) * 16]);
    drop(data);
    staging.unmap();

    values
        .chunks_exact(2 + STEPS)
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
        })
        .collect()
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
            let radius = CLOUD_BASE + (CLOUD_TOP - CLOUD_BASE) * next();
            [
                ring * phi.cos() * radius,
                z * radius,
                ring * phi.sin() * radius,
            ]
        })
        .collect()
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

#[test]
fn the_probe_harness_runs_the_production_shader_headless() {
    let points = shell_points();
    let rows = probe(&points);
    if rows.is_empty() {
        eprintln!("跳过：没有可用的 wgpu 适配器");
        return;
    }
    assert_eq!(rows.len(), POINTS, "回读的点数不对");

    let live = rows.iter().filter(|row| row.field > 1e-4).count();
    assert!(
        live > POINTS / 4,
        "只有 {live} 个采样点落在云里，探针没测到场"
    );

    let (mut coarse, _) = best_of(&rows, 0);
    let (mut fine, _) = best_of(&rows, 1);
    assert!(coarse.len() >= 4, "探针能用的点只有 {} 个", coarse.len());

    let coarse_median = median(&mut coarse);
    let fine_median = median(&mut fine);
    println!(
        "壳内 {live} / {}；差商对解析梯度的中位偏差：h={:e} 时 {coarse_median:e}，h={:e} 时 {fine_median:e}",
        rows.len(),
        SWEEP[0],
        SWEEP[1],
    );
    assert!(
        fine_median < coarse_median,
        "步长减半后差商没有更靠近解析梯度（{coarse_median:e} → {fine_median:e}），探针不可信"
    );
}

#[test]
fn the_analytic_gradient_matches_central_differences() {
    let points = shell_points();
    let rows = probe(&points);
    if rows.is_empty() {
        eprintln!("跳过：没有可用的 wgpu 适配器");
        return;
    }
    assert_eq!(rows.len(), points.len(), "回读的点数不对");

    let live = rows.iter().filter(|row| row.field > 1e-3).count();
    let mut margins: Vec<f32> = rows
        .iter()
        .filter(|row| row.field > 1e-3)
        .map(|row| row.margin)
        .collect();
    margins.sort_by(|one, two| one.partial_cmp(two).expect("NaN"));
    println!(
        "场里 {live} 个点；margin 中位 {:e} 最大 {:e}",
        margins.get(margins.len() / 2).copied().unwrap_or(0.0),
        margins.last().copied().unwrap_or(0.0),
    );

    let mut medians = Vec::new();
    let mut maxima = Vec::new();
    let mut raw_maxima = Vec::new();
    let mut survivors = Vec::new();
    for (slot, &step) in SWEEP.iter().enumerate() {
        let (mut kept, outside) = best_of(&rows, slot);
        raw_maxima.push(
            rows.iter()
                .fold(0.0_f32, |worst, row| worst.max(distance(&row.fd(slot), &row.analytic))),
        );
        println!(
            "步长 {step:e}：门槛 {:.4}，保留 {} 个（剔除 {} 个，剔除里最大偏差 {:e}）",
            KINK_FACTOR * step,
            kept.len(),
            outside.len(),
            outside.iter().fold(0.0_f32, |worst, value| worst.max(*value)),
        );
        assert!(
            kept.len() >= 4,
            "步长 {step:e} 下只有 {} 个点可用，这个测试没在测东西",
            kept.len(),
        );
        survivors.push(kept.len());
        maxima.push(kept.iter().fold(0.0_f32, |worst, value| worst.max(*value)));
        medians.push(median(&mut kept));
    }

    for (index, window) in medians.windows(2).enumerate() {
        println!(
            "中位偏差 {:.3e} → {:.3e}（比值 {:.3}，步长 {:.1e} → {:.1e}）",
            window[0],
            window[1],
            window[1] / window[0],
            SWEEP[index],
            SWEEP[index + 1],
        );
    }
    println!("保留点数 {survivors:?}");
    println!("保留点最大偏差 {maxima:?}");
    println!("全部点最大偏差 {raw_maxima:?}");

    for (index, window) in medians.windows(2).enumerate() {
        assert!(
            window[1] < window[0] * 0.7,
            "步长从 {:.1e} 降到 {:.1e}，中位偏差没有跟着缩（{:.3e} → {:.3e}）⇒ 是公式错，不是步长太大",
            SWEEP[index],
            SWEEP[index + 1],
            window[0],
            window[1],
        );
    }

    let finest = maxima[maxima.len() - 1];
    assert!(
        finest < 1e-3,
        "最细步长 {:.1e} 上最大偏差仍是 {finest:e} ⇒ 差的不是步长",
        SWEEP[SWEEP.len() - 1],
    );
}

#[test]
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
    let rows = probe(&points);
    if rows.is_empty() {
        eprintln!("跳过：没有可用的 wgpu 适配器");
        return;
    }
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

