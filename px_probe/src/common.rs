use std::path::Path;

pub fn assemble(name: &str) -> String {
    let workspace = workspace_root();
    let modules = px_shader::workspace_modules(workspace).unwrap_or_else(|err| panic!("{err}"));
    let (source, _path) =
        px_shader::workspace_source_of(workspace, name).unwrap_or_else(|err| panic!("{err}"));
    let mut seen = Vec::new();
    px_shader::assemble::render_source(
        &source,
        &modules,
        px_shader::host_stubs::wgpu_host_stub,
        &mut seen,
    )
}

pub fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_probe 必须住在 workspace 下")
}

pub use px_protocol::material::MATERIAL_BIND_GROUP;

pub const JOB_BIND_GROUP: u32 = 1;

pub const JOB_BIND_GROUP_TOKEN: &str = "#{JOB_BIND_GROUP}";

pub const COVERAGE_BINDING: u32 = 5;
pub const COVERAGE_SAMPLER_BINDING: u32 = COVERAGE_BINDING + 1;

pub fn job_group_source(source: &str) -> String {
    source.replace(JOB_BIND_GROUP_TOKEN, &JOB_BIND_GROUP.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

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

pub use px_gpu::{Gpu, connect};

pub fn require_gpu() -> &'static Gpu {
    match connect() {
        Some(gpu) => gpu,
        None => {
            eprintln!(
                "拿不到 wgpu 适配器（后端选择见 px_gpu，`WGPU_BACKEND` 可覆盖）。\
                 探针不在无 GPU 的机器上静默通过。"
            );
            std::process::exit(2);
        }
    }
}

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
