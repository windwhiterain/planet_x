//! 探针的公共地基：一个进程一个 wgpu 设备、一个 shader 组装入口。
//!
//! `connect()` 原来住在 `px_render/tests/common/mod.rs`，被 15 个 `#[test]` 各调一次
//! （文档 §44.4 实测每次拿 adapter 4.24 s）。现在探针是一个进程跑完全部 check，
//! 设备只建一次，而且后端固定 DX12。

pub use px_render::shaders::assemble;

/// 材质绑定组 = **契约表里的那个数**（= Bevy 的 `MATERIAL_BIND_GROUP_INDEX`）。
/// 探针原来自己写死 2（那是老组装器替出来的数），而运行期是 3 —— 同一个 `#{MATERIAL_BIND_GROUP}`
/// 在两边组的不是同一份东西（`art/12-step0.md` §79 的「2/3 那颗雷」）。
pub use px_protocol::material::MATERIAL_BIND_GROUP;

/// 探针自己的 job/out 放在**哪一格**。**不是**材质组 + 1：材质组按契约是 **3**，
/// 而探针设备要的是 `wgpu::Limits::downlevel_defaults()`（`max_bind_groups = 4` ⇒ 合法只有 0..3），
/// 排到 4 会被 `create_shader_module` 当场拒：「group index 4 exceeds the max_bind_groups limit of 4」。
/// 探针不用 0/1/2 这三格（0 是 Bevy 的视图组，1/2 空着），所以自己的组就放 1。
/// 同样**不许写死数字**：`#{JOB_BIND_GROUP}` 由 [`job_group_source`] 替进那段 WGSL。
pub const JOB_BIND_GROUP: u32 = 1;

/// 探针 WGSL 里的组号占位（材质组那个占位由组装器替，见 `px_shader::assemble`）。
pub const JOB_BIND_GROUP_TOKEN: &str = "#{JOB_BIND_GROUP}";

/// 探针**只用契约里的一格贴图**：覆盖度 cube 那一格 —— `clouds.wgsl` 把它声明在第 5 格
/// （采样器在第 6 格，见 `TEXTURE_SLOTS` 的约定：贴图占奇数格、采样器占 +1）。
///
/// ⚠ 这两个数**不许在探针里再抄一遍**：探针原来自己写「贴图 1 / 采样器 2」，
/// 而 `76be114`（通用渲染 S0–S4）把材质贴图挪到了奇数格 ⇒ 探针的布局与 shader 声明对不上，
/// `gradient` 那一族 check 从此全红（实测：`group 2, binding 5 is not available in the pipeline layout`，
/// 见 `08-renderer.md` §81.6）。绑定的真源只有一处：`px_protocol::material`。
pub const COVERAGE_BINDING: u32 = 5;
pub const COVERAGE_SAMPLER_BINDING: u32 = COVERAGE_BINDING + 1;

/// 把探针自己那段 WGSL 里的 `#{JOB_BIND_GROUP}` 替成 [`JOB_BIND_GROUP`]。
pub fn job_group_source(source: &str) -> String {
    source.replace(JOB_BIND_GROUP_TOKEN, &JOB_BIND_GROUP.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探针的材质布局必须落在契约表里：这一格是 cube、采样器在 +1。
    /// 「探针是第四份手抄表」那个坑（§67.4 第 7 处）就是这条断言要拦的东西。
    #[test]
    fn the_probe_texture_slot_comes_from_the_contract() {
        assert_eq!(
            px_protocol::material::texture_slot_of(COVERAGE_BINDING),
            Some(px_protocol::material::TextureDimension::Cube),
            "覆盖度必须是契约表里的一个 cube 格"
        );
        assert_eq!(COVERAGE_SAMPLER_BINDING, COVERAGE_BINDING + 1);
        assert_ne!(
            MATERIAL_BIND_GROUP, JOB_BIND_GROUP,
            "材质组与探针自己的组不许撞"
        );
        assert!(
            JOB_BIND_GROUP < 4,
            "探针设备用 downlevel_defaults（max_bind_groups = 4）⇒ 合法组号只有 0..3"
        );
    }
}

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
