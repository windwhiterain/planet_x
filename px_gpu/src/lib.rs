//! See docs/gpu.md

use std::sync::OnceLock;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn take_last_error() -> Option<String> {
    LAST_ERROR.lock().ok().and_then(|mut slot| slot.take())
}

fn backends() -> wgpu::Backends {
    match std::env::var("WGPU_BACKEND").as_deref() {
        Ok("dx12") => wgpu::Backends::DX12,
        Ok("gl") | Ok("gles") => wgpu::Backends::GL,
        _ => wgpu::Backends::VULKAN,
    }
}

fn build() -> Option<Gpu> {
    let descriptor = wgpu::InstanceDescriptor {
        backends: backends(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    };
    let instance = wgpu::Instance::new(descriptor);
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("px_gpu"),
        required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
        required_limits: adapter.limits(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    }))
    .ok()?;
    device.on_uncaptured_error(std::sync::Arc::new(|error| {
        let text = format!("{error}");
        eprintln!("px_gpu 未捕获错误：{text}");
        if let Ok(mut slot) = LAST_ERROR.lock() {
            *slot = Some(text);
        }
    }));
    Some(Gpu {
        instance,
        adapter,
        device,
        queue,
    })
}

pub fn connect() -> Option<&'static Gpu> {
    GPU.get_or_init(build).as_ref()
}

pub fn require_gpu<T>(result: Result<T, String>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("this check requires a working GPU: {error}"),
    }
}

pub enum Binding<'a> {
    Uniform(&'a [u8]),
    Storage(&'a [u8]),
    Write(&'a [u8]),
}

pub struct Slot<'a> {
    pub binding: u32,
    pub value: Binding<'a>,
}

pub fn dispatch(
    gpu: &Gpu,
    wgsl: &str,
    entry: &str,
    bindings: &[Binding<'_>],
    workgroups: (u32, u32, u32),
) -> Result<Vec<Vec<u8>>, String> {
    let slots: Vec<Slot<'_>> = bindings
        .iter()
        .enumerate()
        .map(|(index, binding)| Slot {
            binding: index as u32,
            value: match binding {
                Binding::Uniform(bytes) => Binding::Uniform(bytes),
                Binding::Storage(bytes) => Binding::Storage(bytes),
                Binding::Write(bytes) => Binding::Write(bytes),
            },
        })
        .collect();
    dispatch_slots(gpu, wgsl, entry, &slots, workgroups)
}

pub fn dispatch_slots(
    gpu: &Gpu,
    wgsl: &str,
    entry: &str,
    slots: &[Slot<'_>],
    workgroups: (u32, u32, u32),
) -> Result<Vec<Vec<u8>>, String> {
    for (index, slot) in slots.iter().enumerate() {
        let binding = &slot.value;
        let bytes = match binding {
            Binding::Uniform(bytes) | Binding::Storage(bytes) | Binding::Write(bytes) => *bytes,
        };
        if bytes.len() % 4 != 0 {
            return Err(format!(
                "px_gpu 第 {index} 个绑定是 {} 字节，不是 4 的倍数",
                bytes.len()
            ));
        }
    }

    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("px_gpu"),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });

    let mut entries = Vec::with_capacity(slots.len());
    let mut buffers = Vec::with_capacity(slots.len());
    let mut readback = Vec::new();
    for slot in slots.iter() {
        let index = slot.binding as usize;
        let binding = &slot.value;
        let (ty, usage, bytes) = match binding {
            Binding::Uniform(bytes) => (
                wgpu::BufferBindingType::Uniform,
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                *bytes,
            ),
            Binding::Storage(bytes) => (
                wgpu::BufferBindingType::Storage { read_only: true },
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                *bytes,
            ),
            Binding::Write(bytes) => (
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::COPY_SRC,
                *bytes,
            ),
        };
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: index as u32,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_gpu"),
            size: bytes.len().max(4) as u64,
            usage,
            mapped_at_creation: false,
        });
        if !bytes.is_empty() {
            gpu.queue.write_buffer(&buffer, 0, bytes);
        }
        buffers.push(buffer);
        if matches!(binding, Binding::Write(_)) {
            readback.push((buffers.len() - 1, bytes.len()));
        }
    }

    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("px_gpu"),
            entries: &entries,
        });
    let bindings_ref: Vec<wgpu::BindGroupEntry> = buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| wgpu::BindGroupEntry {
            binding: slots[index].binding,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("px_gpu"),
        layout: &layout,
        entries: &bindings_ref,
    });
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("px_gpu"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("px_gpu"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

    let mut staging = Vec::with_capacity(readback.len());
    for (_, size) in &readback {
        staging.push(gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_gpu::readback"),
            size: (*size).max(4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }

    let scope = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("px_gpu"),
        });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("px_gpu"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
    }
    for (slot, (index, size)) in readback.iter().enumerate() {
        encoder.copy_buffer_to_buffer(&buffers[*index], 0, &staging[slot], 0, *size as u64);
    }
    gpu.queue.submit(Some(encoder.finish()));

    if let Some(error) = pollster::block_on(scope.pop()) {
        return Err(format!("px_gpu 校验出错：{error}"));
    }

    for buffer in &staging {
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .map_err(|err| format!("px_gpu 回读 poll 失败：{err}"))?;
        receiver
            .recv()
            .map_err(|err| format!("px_gpu 回读回调没回来：{err}"))?
            .map_err(|err| format!("px_gpu 回读映射失败：{err}"))?;
    }

    let mut out = Vec::with_capacity(staging.len());
    for (slot, (_, size)) in readback.iter().enumerate() {
        let view = staging[slot].slice(..);
        out.push(view.get_mapped_range()[..*size].to_vec());
    }
    for buffer in &staging {
        buffer.unmap();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compute_pass_round_trips() {
        let gpu = connect().expect("this check requires a working GPU");
        let wgsl = r#"
@group(0) @binding(0) var<storage, read> src: array<u32>;
@group(0) @binding(1) var<storage, read_write> dst: array<u32>;

@compute @workgroup_size(64)
fn double(@builtin(global_invocation_id) id: vec3<u32>) {
    dst[id.x] = src[id.x] * 2u;
}
"#;
        let mut src = Vec::new();
        for value in 0..64_u32 {
            src.extend_from_slice(&value.to_le_bytes());
        }
        let zero = vec![0_u8; 64 * 4];
        let out = dispatch(
            gpu,
            wgsl,
            "double",
            &[Binding::Storage(&src), Binding::Write(&zero)],
            (1, 1, 1),
        )
        .expect("派发");
        assert_eq!(out.len(), 1, "应当回读一份");
        for value in 0..64_u32 {
            let at = value as usize * 4;
            let got = u32::from_le_bytes(out[0][at..at + 4].try_into().unwrap());
            assert_eq!(got, value * 2, "第 {value} 项");
        }
        println!("px_gpu：{} 项往返正确", 64);
    }
}
