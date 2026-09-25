use std::time::Instant;

pub const BACKEND_ASSERT: &str = "后端断言失败";

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

fn wanted() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY
        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES
        | wgpu::Features::FLOAT32_FILTERABLE
}

fn refuse(reason: String) -> ! {
    eprintln!("{BACKEND_ASSERT}：{reason}");
    std::process::exit(2);
}

pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

pub fn connect() -> Gpu {
    connect_with(instance(), None)
}

pub fn connect_with(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Gpu {
    let started = Instant::now();

    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: surface,
    })) {
        Ok(adapter) => adapter,
        Err(err) => refuse(format!("Vulkan 适配器拿不到：{err}")),
    };

    let info = adapter.get_info();
    if info.backend != wgpu::Backend::Vulkan {
        refuse(format!(
            "选到的后端是 {:?} 而不是 Vulkan（{}）：本渲染器只认 Vulkan",
            info.backend, info.name
        ));
    }

    let available = adapter.features();
    let (device, queue) =
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("px_render"),
            required_features: available & wanted(),
            required_limits: wgpu::Limits {
                max_sampled_textures_per_shader_stage: 32,
                ..wgpu::Limits::default()
            },
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        })) {
            Ok(pair) => pair,
            Err(err) => refuse(format!("Vulkan 设备建不出来：{err}")),
        };

    println!("后端：{:?}｜{}｜设备就绪", info.backend, info.name);
    println!(
        "GPU 时间戳能力：query={} encoders={} passes={}",
        available.contains(wgpu::Features::TIMESTAMP_QUERY),
        available.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS),
        available.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
    );
    println!("进程内：到设备就绪 {} ms", started.elapsed().as_millis());

    Gpu {
        instance,
        adapter,
        device,
        queue,
    }
}
