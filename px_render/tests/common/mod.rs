use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const SHADER_ROOT: &str = "assets/shaders";

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

pub fn connect() -> Option<Gpu> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("headless probe"),
        required_features: adapter.features() & wgpu::Features::FLOAT32_FILTERABLE,
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

pub fn shader_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(SHADER_ROOT);
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(SHADER_ROOT)
        .join(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读不了 {}：{error}", path.display()));
    let mut seen = Vec::new();
    render_source(&source, &modules, &mut seen)
}
