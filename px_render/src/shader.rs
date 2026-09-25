use std::path::{Path, PathBuf};

use px_shader::ModuleTable;
use px_shader::assemble::Stubs;

pub fn roots() -> Vec<PathBuf> {
    px_shader::workspace_roots(workspace())
}

pub fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("px_render 必须住在 workspace 下")
}

pub fn modules() -> ModuleTable {
    try_modules().unwrap_or_else(|err| panic!("{err}"))
}

pub fn try_modules() -> Result<ModuleTable, String> {
    px_shader::module_sources(&roots())
}

pub fn assemble(entry: &str, modules: &ModuleTable, stubs: Stubs) -> String {
    let mut seen = Vec::new();
    px_shader::assemble::render_source(entry, modules, stubs, &mut seen)
}

pub fn source_of(name: &str) -> (String, PathBuf) {
    try_source_of(name).unwrap_or_else(|err| panic!("{err}"))
}

pub fn try_source_of(name: &str) -> Result<(String, PathBuf), String> {
    px_shader::workspace_source_of(workspace(), name)
}

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
    files.sort();
    Ok(files)
}

pub const CONTENT_SHADERS: [&str; 4] = [
    "surface.wgsl",
    "atmosphere.wgsl",
    "clouds.wgsl",
    "ring.wgsl",
];

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
                other => {
                    return Some((
                        binding.group,
                        binding.binding,
                        format!("{other:?}"),
                        "?".into(),
                    ));
                }
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
