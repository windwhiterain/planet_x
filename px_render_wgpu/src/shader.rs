//! 装载与组装：把一份入口 shader 变成能直接喂给 `create_shader_module` 的自足 WGSL。
//!
//! 与 Bevy 宿主那半**共用同一份解析规则**（`px_shader`：模块发现、`#import` 解析、闭包指纹）——
//! 拆开写两份的话，"这份 shader 到底是什么"会出现第二个答案，而键与画面就再也对不上（§17.1）。
//! 差别只有一处，也就是桩表，见 [`crate::stubs`]。
//!
//! ⚠ 这里有个 Bevy 宿主没有的**简化**：裸 wgpu 的 `create_shader_module` 是**同步**的，
//! 所以不需要 `slots.rs` 那套"排队 / 编辑中 / 重试"的状态机（§104 第 5 条）。
//! 但"坏管线当场拒、不静默出缺材质的图"这条判据一个字都不许少。

use std::path::{Path, PathBuf};

use px_shader::ModuleTable;
use px_shader::assemble::Stubs;

/// 入口 shader 的内容根（`art/shaders`）与库根（`<asset_root>/shaders`）。
///
/// ⚠ **库根今天指在 `px_render/assets/shaders`**：那是本仓的既有约定
/// （`px_shader::workspace_roots`），库的内容住在那里，与"谁来渲染"无关。
/// S8 删掉 Bevy 宿主时，这批库要跟着搬到新家 —— 那一档的第一件事就是这里，
/// 所以**只留这一个函数**当落点，别在别处再写一遍路径。
pub fn roots() -> Vec<PathBuf> {
    px_shader::workspace_roots(workspace())
}

/// 工作区根：`px_render_wgpu` 的上一级。与 `px_render::shaders` 的取法同源（`CARGO_MANIFEST_DIR`），
/// 不猜当前目录 —— 服务是**别的进程**按绝对路径调起来的，当前目录不归我们管。
pub fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_render_wgpu 必须住在 workspace 下")
}

pub fn modules() -> ModuleTable {
    px_shader::module_sources(&roots()).unwrap_or_else(|err| panic!("{err}"))
}

/// 一份入口 shader 的**自足** WGSL：`#import` 全展开、`#{MATERIAL_BIND_GROUP}` 已替。
///
/// ⚠ 两件事必须都做到，否则 `naga` 当场拒（§104 第 6 条）：
/// 入口文本里的 `#{MATERIAL_BIND_GROUP}` 不是合法 WGSL；而 `px_pass` 拿到的文本
/// 必须是自足的（执行器把 WGSL 直接交给 wgpu，不跑组装器）。
pub fn assemble(entry: &str, modules: &ModuleTable, stubs: Stubs) -> String {
    let mut seen = Vec::new();
    px_shader::assemble::render_source(entry, modules, stubs, &mut seen)
}

/// 名字 → 入口文本。两处都找：内容（`art/shaders`）与库（`assets/shaders`）。
/// 与 `px_render::shaders::shader_source_of` 同口径，**重名要报错**。
pub fn source_of(name: &str) -> (String, PathBuf) {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidates = [manifest.join("../art/shaders"), manifest.join("assets/shaders")];
    let mut found: Vec<PathBuf> = candidates
        .iter()
        .map(|root| root.join(name))
        .filter(|path| path.exists())
        .collect();
    match found.len() {
        1 => {
            let path = found.remove(0);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("读不了 {}：{err}", path.display()));
            (text, path)
        }
        0 => panic!("哪里都找不到 shader '{name}'（找过 {}）", manifest.display()),
        _ => panic!(
            "shader '{name}' 在两处都有：{} —— 名字必须唯一，否则测的是这一份、用的是那一份",
            found
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(" / ")
        ),
    }
}

/// 内容 shader（入口）的名字。判据只认这几个 —— 目录里多出来的东西不该悄悄进判据。
pub const CONTENT_SHADERS: [&str; 4] = ["surface.wgsl", "atmosphere.wgsl", "clouds.wgsl", "ring.wgsl"];

/// `naga` 解析 + 校验。**返回**错误而不是 panic：调用方决定这是"门"还是"探针"。
pub fn validate(name: &str, assembled: &str) -> Result<naga::Module, String> {
    let module = naga::front::wgsl::parse_str(assembled)
        .map_err(|err| format!("{name} 解析失败：{}", err.emit_to_string(assembled)))?;
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .map_err(|err| format!("{name} 校验失败：{err:?}"))?;
    Ok(module)
}

/// 组装出来的文本里，**每一个全局变量的 (group, binding)** —— 按组号、格号排序。
///
/// 为什么要反射而不是在 Rust 侧抄一份常量：group 0 的号是**契约**，
/// 而契约的真本是 shader 自己（§85 的 C 案把这条推到了材质那边，这里同一句话）。
/// 抄一份常量进 Rust 就是 §66.1 那颗「同一条契约、两个数字」的雷 ——
/// 绑定号一旦漂开，画面会变而任何门都不会响（§65 记的那 22–33 个像素）。
pub fn bindings(module: &naga::Module) -> Vec<(u32, u32, String, String)> {
    let mut rows: Vec<(u32, u32, String, String)> = module
        .global_variables
        .iter()
        .filter_map(|(_, global)| {
            let binding = global.binding?;
            let space = match global.space {
                naga::AddressSpace::Uniform => "uniform",
                naga::AddressSpace::Storage { .. } => "storage",
                naga::AddressSpace::Handle => "handle",
                other => return Some((binding.group, binding.binding, format!("{other:?}"), "?".into())),
            };
            Some((
                binding.group,
                binding.binding,
                space.to_string(),
                global.name.clone().unwrap_or_else(|| "?".to_string()),
            ))
        })
        .collect();
    rows.sort();
    rows
}
