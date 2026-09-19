//! 设备地基：实例 / 适配器 / 设备。
//!
//! 与 `px_probe::common` 的差别只有一处，而那一处本身就是判据：**后端锁死 Vulkan**。
//!
//! - `tools/harness.ps1` 会给子进程设 `WGPU_BACKEND=vulkan`（那是给探针/老宿主用的），
//!   但**环境变量不许盖过这里的选择**：DX12 实测慢 2.3×、长尾 3.6×（§104 第 9 条），
//!   同一个 exe 在两种后端下出两种性能读数，而判据是逐字节的 —— 后端必须是代码里的常数。
//! - 拿不到 Vulkan 就 **exit(2)** 并在 stderr 打固定前缀 [`BACKEND_ASSERT`]：
//!   `tools/harness.ps1` 的 `$HarnessFailPattern` 认这个串，不认就要等满超时才失败。

use std::time::Instant;

/// `tools/harness.ps1` 的 fail-fast 认的固定串。改这个串 = 让闸门失效。
pub const BACKEND_ASSERT: &str = "后端断言失败";

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// 我们要的能力。**要不到的就不开**（§104 第 12 条：时间戳在有些队列族上不可用，
/// 那只该让 `gpu_ms` 退成 null，不该让进程起不来）。
fn wanted() -> wgpu::Features {
    wgpu::Features::TIMESTAMP_QUERY
        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
        | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES
        | wgpu::Features::FLOAT32_FILTERABLE
}

/// 后端不对 / 拿不到设备：这是**硬失败**，不是"降级跑跑看"。
fn refuse(reason: String) -> ! {
    eprintln!("{BACKEND_ASSERT}：{reason}");
    std::process::exit(2);
}

/// 建实例（**只有它**）。分开是为了预览窗口：`Surface` 属于**建它的那个实例**，
/// 所以窗口那条路的次序必须是"实例 → surface → 适配器/设备"，而不是"设备 → surface"。
pub fn instance() -> wgpu::Instance {
    wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    })
}

/// 建实例 → 适配器 → 设备，并把就绪读数打出来。
///
/// 打出来的顺序就是外部仪器打戳的顺序（`target/research/startup-probe.ps1` 那种：
/// 它按**日志行到达时间**打戳，一行代码都不改产品）：
/// 含 `后端` 的那一行 = "适配器 + 设备就绪"这个里程碑，与 §94 那张表同一个边界。
/// ⚠ 边界换一格，读数就不能与历史比（§104 第 4 条同一条教训）。
pub fn connect() -> Gpu {
    connect_with(instance(), None)
}

/// [`connect`] 的后半段，多一格：**要画到哪张 surface 上**。
///
/// ⚠ 只有预览窗口会传 `Some`，而那一格不是可选的：`compatible_surface` 缺省是 `None`，
/// 于是选适配器时**不考虑**它能不能画到那张交换链上 —— 多 GPU 的机器上完全可能选到一张
/// 画不出来的卡，症状是"窗口开了、里面全黑"，而那一刻的错误离病因已经很远。
/// 离线那几条路传 `None`（它们的层次里根本没有交换链，§104 第 13 条）。
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
    let (device, queue) = match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("px_render"),
        required_features: available & wanted(),
        required_limits: wgpu::Limits::default(),
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
    })) {
        Ok(pair) => pair,
        Err(err) => refuse(format!("Vulkan 设备建不出来：{err}")),
    };

    // ⚠ 这一行要在**设备建好之后**打：外部仪器拿它当"设备就绪"的里程碑。
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
