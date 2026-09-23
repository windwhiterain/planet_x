//! **无窗口 GPU 计算的最小宿主**。
//!
//! ⚠ 为什么单独一个 crate（而不是把 `px_probe` 当依赖）：`px_probe` 是**探针工具**
//!   （`default-members` 里都没有它），而烘图侧的算子要调它 —— 生产 op 依赖开发工具是一条
//!   反过来的边。这里只放"起设备 + 跑一遍计算 + 回读"这三件事，`px_probe` 将来可以并到它上面。
//!
//! ⚠ **键这一侧不受影响**（GPU 产物的计算不确定性不破坏缓存语义）：
//!   `px_cook::cached` 的节点键 = 算子身份（含实现库的源码指纹）+ 参数 + 画布 + **输入键**
//!   （`px_cook/src/lib.rs:134`），**不看产物内容**。产物字节只在落 CAS 时算 blake3 当文件名
//!   ⇒ 同一台机器同输入命中不重算；换机器/驱动时内容不同而键相同，各自在本地 CAS 里
//!   重算自己那一份，不会混。**唯一失去的是"跨机器 .pxart 逐字节相同"。**
//!
//! ⚠ 后端默认 DX12：本机实测 DX12 枚举 0.27 s、Vulkan 2.8 s（Vulkan loader 在挨个找不存在的
//!   layer JSON）。`WGPU_BACKEND=vulkan|gl` 可覆盖 —— 与 `px_probe` 同一口径。

use std::sync::OnceLock;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

fn backends() -> wgpu::Backends {
    match std::env::var("WGPU_BACKEND").as_deref() {
        Ok("vulkan") => wgpu::Backends::VULKAN,
        Ok("gl") | Ok("gles") => wgpu::Backends::GL,
        _ => wgpu::Backends::DX12,
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
        // ⚠ `downlevel_defaults`：这一档的 storage buffer 上限比默认低，够放星点与体积；
        //   要求更高时**在调用处**按需申请，别在这里悄悄抬高所有算子的门槛。
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    }))
    .ok()?;
    Some(Gpu {
        instance,
        adapter,
        device,
        queue,
    })
}

/// 进程内唯一的设备：第一次调用建，之后都是同一份（设备创建本身要 0.3 s 量级）。
pub fn connect() -> Option<&'static Gpu> {
    GPU.get_or_init(build).as_ref()
}

/// 一次派发要绑的东西，**顺序就是 WGSL 里的 `@binding(n)`**。
pub enum Binding<'a> {
    /// 只读参数（`var<uniform>`）。
    Uniform(&'a [u8]),
    /// 只读数据（`var<storage, read>`）。
    Storage(&'a [u8]),
    /// 要回读的缓冲区（`var<storage, read_write>`）。
    Write(&'a [u8]),
}

/// 跑一遍计算着色器；回读**所有 `Binding::Write`**，顺序同声明顺序。
///
/// ⚠ 每段字节的长度必须是 4 的倍数（`write_buffer` 的要求）—— 不满足时当场说清，
///   而不是让驱动在别处报一个指不到这里的错。
pub fn dispatch(
    gpu: &Gpu,
    wgsl: &str,
    entry: &str,
    bindings: &[Binding<'_>],
    workgroups: (u32, u32, u32),
) -> Result<Vec<Vec<u8>>, String> {
    for (index, binding) in bindings.iter().enumerate() {
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

    let mut entries = Vec::with_capacity(bindings.len());
    let mut buffers = Vec::with_capacity(bindings.len());
    let mut readback = Vec::new();
    for (index, binding) in bindings.iter().enumerate() {
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
            readback.push((index, bytes.len()));
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
            binding: index as u32,
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
            // ⚠ wgpu 29 的 immediate（取代 push constant）：这里不用。
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

    // ⚠ 回读用**各自的**暂存缓冲：一张图几 MB，几份一起读比逐份 `poll` 省一整轮同步。
    let mut staging = Vec::with_capacity(readback.len());
    for (_, size) in &readback {
        staging.push(gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("px_gpu::readback"),
            size: (*size).max(4) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }

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

    for buffer in &staging {
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        // ⚠ 先 poll 再收：设备要跑起来回调才会来。
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

    /// **冒烟**：设备起得来 + 一趟 compute 派发能把结果原样读回来。
    ///
    /// ⚠ 这一条判的不是"某个着色器算得对不对"，而是**宿主这条链**：建缓冲 → 绑组 →
    ///   派发 → 暂存回读 → 映射。少了哪一步，症状都是"算子在烘焙里给出全 0 或旧数据"，
    ///   而归因不到这里。⚠ 没有可用设备时**跳过**（不是失败）：CI 机器未必有 GPU。
    #[test]
    fn a_compute_pass_round_trips() {
        let Some(gpu) = connect() else {
            println!("px_gpu：没有可用设备，跳过冒烟");
            return;
        };
        // 64 个工作项，各自写 x*2；workgroup_size(64) ⇒ 一个工作组。
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
