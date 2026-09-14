use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Resource)]
pub struct ShaderLibraries(pub Vec<Handle<Shader>>);

fn declares_import_path(source: &str) -> bool {
    source
        .lines()
        .any(|line| line.trim_start().starts_with("#define_import_path"))
}

fn load_libraries(mut commands: Commands, assets: Res<AssetServer>) {
    let mut handles = Vec::new();
    let directory = std::path::Path::new(&crate::asset_root()).join("shaders");
    if let Ok(entries) = std::fs::read_dir(&directory) {
        let mut paths: Vec<std::path::PathBuf> =
            entries.flatten().map(|entry| entry.path()).collect();
        paths.sort();
        for path in paths {
            if path.extension().and_then(|value| value.to_str()) != Some("wgsl") {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !declares_import_path(&source) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            handles.push(assets.load(format!("shaders/{name}")));
            println!("shader 库已加载：{name}");
        }
    }
    if handles.is_empty() {
        eprintln!("⚠ 一个 shader 库都没加载到：{}", directory.display());
    }
    commands.insert_resource(ShaderLibraries(handles));
}

pub struct ShaderLibraryPlugin;

impl Plugin for ShaderLibraryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_libraries);
    }
}

// ---------------------------------------------------------------------------
// 按文本组装 shader 的助手。
//
// 原来住在 `px_render/tests/common/mod.rs`，被离线门（`tests/shaders.rs`、
// `tests/cloud_field.rs`）和 GPU 探针各用一份引子。现在只有一个实现：
// 探针走 `px_probe::common::assemble`，离线门走 `tests/common` 的再导出。
//
// ⚠️ 它比运行时宽松：这里把 `#import` 的整个模块递归展开，而 Bevy 只内联
//    `#import` 里点名的符号 —— 所以「测试能过」给不了「运行时能过」的保证（§46.4）。
// ---------------------------------------------------------------------------

pub const SHADER_ROOT: &str = "assets/shaders";

pub fn shader_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(SHADER_ROOT);
    let Ok(entries) = std::fs::read_dir(&root) else {
        panic!("找不到 shader 目录：{}", root.display());
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) == Some("wgsl") {
            files.push(path);
        }
    }
    files.sort();
    files
}

pub fn import_path_of(source: &str) -> Option<String> {
    source.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("#define_import_path ")
            .map(|rest| rest.trim().to_string())
    })
}

pub fn module_sources() -> HashMap<String, String> {
    let mut modules = HashMap::new();
    for path in shader_files() {
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        if let Some(name) = import_path_of(&source) {
            modules.insert(name, source);
        }
    }
    modules
}

pub fn bevy_stub(symbol: &str) -> Option<&'static str> {
    match symbol {
        "bevy_pbr::forward_io::VertexOutput" => Some(
            "struct VertexOutput {\n\
             \x20   @builtin(position) position: vec4<f32>,\n\
             \x20   @location(0) world_position: vec4<f32>,\n\
             \x20   @location(1) world_normal: vec3<f32>,\n\
             \x20   @location(2) uv: vec2<f32>,\n\
             };\n",
        ),
        "bevy_pbr::mesh_view_bindings::view" => Some(
            "struct ViewStub {\n\
             \x20   world_position: vec3<f32>,\n\
             \x20   view_from_world: mat4x4<f32>,\n\
             \x20   clip_from_view: mat4x4<f32>,\n\
             \x20   viewport: vec4<f32>,\n\
             };\n\
             @group(0) @binding(0) var<uniform> view: ViewStub;\n",
        ),
        "bevy_pbr::mesh_view_bindings::depth_prepass_texture" => {
            Some("@group(0) @binding(20) var depth_prepass_texture: texture_depth_2d;\n")
        }
        "bevy_pbr::view_transformations::depth_ndc_to_view_z" => {
            Some("fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 { return -1.0 / max(ndc_depth, 1e-6); }\n")
        }
        _ => None,
    }
}

pub fn expand(path: &str, modules: &HashMap<String, String>, seen: &mut Vec<String>) -> String {
    let import = path.trim();
    if let Some(stub) = bevy_stub(import) {
        return stub.to_string();
    }

    let module = if modules.contains_key(import) {
        import.to_string()
    } else {
        match import.rsplit_once("::") {
            Some((module, _)) => module.to_string(),
            None => return String::new(),
        }
    };

    let Some(source) = modules.get(&module) else {
        panic!("未知的 import：{import}");
    };
    if seen.iter().any(|entry| entry == &module) {
        return String::new();
    }
    seen.push(module);
    render_source(source, modules, seen)
}

pub fn render_source(
    source: &str,
    modules: &HashMap<String, String>,
    seen: &mut Vec<String>,
) -> String {
    let mut out = String::new();
    let mut imports: Vec<String> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("#import ") {
            imports.push(rest.trim().to_string());
            continue;
        }
        if trimmed.starts_with("#define_import_path") {
            continue;
        }
        out.push_str(&line.replace("#{MATERIAL_BIND_GROUP}", "2"));
        out.push('\n');
    }

    let mut prelude = String::new();
    for import in imports {
        if let Some((module, braces)) = import.split_once("::{") {
            for symbol in braces.trim_end_matches('}').split(',') {
                let symbol = symbol.trim();
                if symbol.is_empty() {
                    continue;
                }
                prelude.push_str(&expand(&format!("{module}::{symbol}"), modules, seen));
            }
            continue;
        }
        prelude.push_str(&expand(&import, modules, seen));
    }
    format!("{prelude}\n{out}")
}

pub fn assemble(name: &str) -> String {
    let modules = module_sources();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(SHADER_ROOT)
        .join(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读不了 {}：{error}", path.display()));
    let mut seen = Vec::new();
    render_source(&source, &modules, &mut seen)
}
