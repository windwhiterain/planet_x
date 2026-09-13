use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SHADER_ROOT: &str = "assets/shaders";

fn shader_files() -> Vec<PathBuf> {
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

fn import_path_of(source: &str) -> Option<String> {
    source.lines().find_map(|line| {
        let line = line.trim();
        line.strip_prefix("#define_import_path ")
            .map(|rest| rest.trim().to_string())
    })
}

fn module_sources() -> HashMap<String, String> {
    let mut modules = HashMap::new();
    for path in shader_files() {
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        if let Some(name) = import_path_of(&source) {
            modules.insert(name, source);
        }
    }
    modules
}

fn bevy_stub(symbol: &str) -> Option<&'static str> {
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

fn expand(path: &str, modules: &HashMap<String, String>, seen: &mut Vec<String>) -> String {
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

fn render_source(source: &str, modules: &HashMap<String, String>, seen: &mut Vec<String>) -> String {
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

#[test]
fn every_shader_parses_and_validates() {
    let modules = module_sources();
    let mut checked = 0_usize;

    for path in shader_files() {
        if import_path_of(&std::fs::read_to_string(&path).expect("读不了 shader")).is_some()
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("common"))
                .unwrap_or(false)
        {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        if import_path_of(&source).is_some() {
            continue;
        }
        let mut seen = Vec::new();
        let assembled = render_source(&source, &modules, &mut seen);
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let module = naga::front::wgsl::parse_str(&assembled).unwrap_or_else(|error| {
            panic!(
                "{name} 解析失败：\n{}\n---- 组装后的源码 ----\n{assembled}",
                error.emit_to_string(&assembled)
            )
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator.validate(&module).unwrap_or_else(|error| {
            panic!("{name} 校验失败：{error:?}");
        });
        checked += 1;
    }

    assert!(checked > 0, "一个 shader 都没检查到");
    println!("shader 校验通过：{checked} 个");
}

#[test]
fn the_checker_catches_a_broken_shader() {
    let broken = "fn bad() -> f32 { return vec3<f32>(1.0, 2.0, 3.0); }";
    let parsed = naga::front::wgsl::parse_str(broken);
    let rejected = match parsed {
        Err(_) => true,
        Ok(module) => {
            let mut validator = naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            );
            validator.validate(&module).is_err()
        }
    };
    assert!(rejected, "校验器必须能拒绝类型错误的 shader");
}

#[test]
fn an_unknown_import_is_not_silently_ignored() {
    let modules = module_sources();
    let mut seen = Vec::new();
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        expand("planet_x::missing::thing", &modules, &mut seen)
    }))
    .is_err();
    assert!(caught, "未知 import 必须报错，不能静默略过");
}



