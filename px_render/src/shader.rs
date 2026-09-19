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

/// 入口 shader 的内容根（`art/shaders`）与库根（`art/shaders/lib`）。
///
/// ⚠ **两个根都归 `px_shader::workspace_roots` 说了算**（S8-a 之后库搬去 `art/shaders/lib/`）：
/// 这个函数只是把"本 crate 住在工作区里的哪一格"喂进去，**不再自己拼路径** ——
/// 宿主、烘图侧、探针三方对"库在哪"只能有一个答案。
pub fn roots() -> Vec<PathBuf> {
    px_shader::workspace_roots(workspace())
}

/// 工作区根：本 crate（§157 起叫 `px_render`，改名前是 `px_render_wgpu`）的上一级。
/// 与 `px_render::shaders`（**已删的 Bevy 宿主**的模块，§154）的取法同源（`CARGO_MANIFEST_DIR`），
/// 不猜当前目录 —— 服务是**别的进程**按绝对路径调起来的，当前目录不归我们管。
pub fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_render 必须住在 workspace 下")
}

pub fn modules() -> ModuleTable {
    try_modules().unwrap_or_else(|err| panic!("{err}"))
}

/// 同 [`modules`]，但**读不出来就返回 `Err`**（理由与 [`try_source_of`] 同一条：
/// 热重载那条路要能"这一份不重载并说清为什么"，而不是把窗口整个带走）。
pub fn try_modules() -> Result<ModuleTable, String> {
    px_shader::module_sources(&roots())
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

/// 名字 → 入口文本。规则住在 `px_shader::workspace_source_of`：在 `workspace_roots`
/// 那两个根下按文件名找，**重名要报错**（不许先到先得 —— 那会让两边各测一份）。
pub fn source_of(name: &str) -> (String, PathBuf) {
    try_source_of(name).unwrap_or_else(|err| panic!("{err}"))
}

/// 同 [`source_of`]，但**找不到 / 重名都返回 `Err`**。
///
/// ⚠ 为什么要两个入口：装载期那条路的规矩是"文档说不通就当场拒"（panic 也是拒），
/// 而**热重载**那条路要能"这一份不重载，并说清为什么不"—— 一个 panicking 的取法
/// 会把"盘上少了一个文件"变成"窗口整个没了"。取法本身只有一份（这里），
/// 两个入口只差"拿不到时怎么办"。
///
/// ⚠ 查找规则本身**不住在这里**（S8-a）：`px_probe` 也要按同一条规则找入口，而它依赖不到
/// 本 crate（本 crate 只有 bin target）⇒ 规则搬进共享叶子 crate，这里只剩"喂工作区根"。
pub fn try_source_of(name: &str) -> Result<(String, PathBuf), String> {
    px_shader::workspace_source_of(workspace(), name)
}

/// 盘上**所有可能被热重载**的 `.wgsl`：两个 shader 根 + 帧图那一档（`art/frame/*.wgsl`）。
///
/// ⚠ 为什么是"扫目录"而不是"从文档推一张文件清单"：文档里记的是**成员**（`graph`/`node`）
/// 与**内联全文**，库文件（`planet_x::*`）在文档里根本没有名字 —— 扫目录是唯一不靠猜的取法。
/// 而"哪几份真的变了"由**组装后逐字比较**给出（那是读数，不是推断），所以扫宽一点不危险，
/// 只会多报一条"这个文件变了，但没有哪一槽的文本跟着变"。
///
/// ⚠ `art/frame/` 不是 shader 库根（`px_shader::workspace_roots` 里没有它）：它是**帧自己的**那几个
/// 阶段住的地方（`vertex_mesh.wgsl` / `vertex_sky.wgsl` / 帧材质的入口）。热重载要看它，
/// 因为判据说的是"改一个 `.wgsl` 存盘"；而那几个文件**今天只有帧材质那一份有来源**
/// （顶点阶段在文档里是内联全文、没有名字），见 `Session::shader_slots` 那段。
pub fn watch_files() -> Result<Vec<PathBuf>, String> {
    let mut roots = roots();
    roots.push(workspace().join("art").join("frame"));
    let mut files: Vec<PathBuf> = Vec::new();
    for root in &roots {
        let entries = std::fs::read_dir(root)
            .map_err(|err| format!("读不了 shader 目录 {}：{err}", root.display()))?;
        for entry in entries {
            let path = entry
                .map_err(|err| format!("读不了 {} 里的一项：{err}", root.display()))?
                .path();
            if path.extension().is_some_and(|ext| ext == "wgsl") {
                files.push(path);
            }
        }
    }
    // 排序 ⇒ 读数与 `read_dir` 的顺序无关（与 `px_shader::wgsl_files` 同一条纪律）。
    files.sort();
    Ok(files)
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
