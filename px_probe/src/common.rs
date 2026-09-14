//! 探针的公共地基：一个进程一个 wgpu 设备、一个 shader 组装入口。
//!
//! `connect()` 原来住在 `px_render/tests/common/mod.rs`，被 15 个 `#[test]` 各调一次
//! （文档 §44.4 实测每次拿 adapter 4.24 s）。现在探针是一个进程跑完全部 check，
//! 设备只建一次，而且后端固定 DX12。

pub use px_render::shaders::assemble;

use std::sync::OnceLock;

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

/// 默认 DX12。本机实测：DX12 枚举 0.27 s，Vulkan 2.8 s
/// （`target/viewer.err` 里 Vulkan loader 在挨个找不存在的 layer JSON）。
/// `WGPU_BACKEND=vulkan|gl` 可以覆盖 —— 探针脚本本来也设了 dx12，这样就一致了。
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
        label: Some("px_probe"),
        required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
        required_limits: wgpu::Limits::downlevel_defaults(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    }))
    .ok()?;
    let info = adapter.get_info();
    println!(
        "探针设备：{}（{:?}，后端 {:?}）",
        info.name, info.device_type, info.backend
    );
    Some(Gpu {
        instance,
        adapter,
        device,
        queue,
    })
}

/// 进程内唯一的设备：第一次调用建，之后都是同一份。
pub fn connect() -> Option<&'static Gpu> {
    GPU.get_or_init(build).as_ref()
}

/// 拿不到设备就直接失败。「没有 GPU」绝不能读成「探针通过」——
/// 这是 §46.3 里唯一判据被静默跳过的那个洞。
pub fn require_gpu() -> &'static Gpu {
    match connect() {
        Some(gpu) => gpu,
        None => {
            eprintln!(
                "拿不到 wgpu 适配器（请求后端 {:?}）。探针不在无 GPU 的机器上静默通过。",
                backends()
            );
            std::process::exit(2);
        }
    }
}

/// 逐个跑 check：断言本身一字不改，靠 catch_unwind 兜；有失败就 exit 1。
pub fn run_checks(title: &str, checks: Vec<(&'static str, fn())>) {
    println!("== {title}：{} 个 check ==", checks.len());
    let mut failed = 0_usize;
    for (name, check) in checks {
        match std::panic::catch_unwind(check) {
            Ok(()) => println!("✓ {name}"),
            Err(_) => {
                failed += 1;
                println!("✗ {name}");
            }
        }
    }
    if failed > 0 {
        eprintln!("{title}：{failed} 个 check 失败");
        std::process::exit(1);
    }
    println!("{title}：全部通过");
}
