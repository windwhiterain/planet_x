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

/// 入口 shader 的真本住在艺术内容里（`art/shaders/`），随场景产物走；
/// `assets/shaders/slot_*.wgsl` 只是「槽占位」（声明同样的绑定、画得出东西、
/// 一眼看得出不是成品）。两道门两边都要扫，否则真本会从「体积 / 校验」下面溜走。
pub const CONTENT_SHADER_ROOT: &str = "../art/shaders";

pub fn shader_files() -> Vec<std::path::PathBuf> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for root in [manifest.join(SHADER_ROOT), manifest.join(CONTENT_SHADER_ROOT)] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            panic!("找不到 shader 目录：{}", root.display());
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("wgsl") {
                files.push(path);
            }
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
             \x20   exposure: f32,\n\
             \x20   view_from_world: mat4x4<f32>,\n\
             \x20   clip_from_view: mat4x4<f32>,\n\
             \x20   viewport: vec4<f32>,\n\
             };\n\
             @group(0) @binding(0) var<uniform> view: ViewStub;\n",
        ),
        "bevy_pbr::mesh_view_bindings::depth_prepass_texture" => {
            Some("@group(0) @binding(20) var depth_prepass_texture: texture_depth_2d;\n")
        }
        // 自写表面材质读的是**同一个**光源 uniform（`lights`）：环境光与直接光的强度
        // 只有一份来源（相机的 `AmbientLight` 与场景里的 `DirectionalLight`），
        // 不再往材质 params 里抄一遍 —— 抄一遍就是"同一个值、两处维护"。
        "bevy_pbr::mesh_view_bindings::lights" => Some(
            "struct DirectionalLightStub {\n\
             \x20   color: vec4<f32>,\n\
             \x20   direction_to_light: vec3<f32>,\n\
             \x20   flags: u32,\n\
             \x20   num_cascades: u32,\n\
             \x20   depth_texture_base_index: u32,\n\
             };\n\
             struct LightsStub {\n\
             \x20   directional_lights: array<DirectionalLightStub, 1>,\n\
             \x20   ambient_color: vec4<f32>,\n\
             \x20   n_directional_lights: u32,\n\
             };\n\
             @group(0) @binding(1) var<uniform> lights: LightsStub;\n",
        ),
        "bevy_pbr::shadows::fetch_directional_shadow" => Some(
            "fn fetch_directional_shadow(\n\
             \x20   light_id: u32,\n\
             \x20   frag_position: vec4<f32>,\n\
             \x20   surface_normal: vec3<f32>,\n\
             \x20   view_z: f32,\n\
             \x20   frag_coord_xy: vec2<f32>,\n\
             ) -> f32 { return 1.0; }\n",
        ),
        // 点光源的影：cube shadow map（`fetch_directional_shadow` 的兄弟）。§60
        "bevy_pbr::shadows::fetch_point_shadow" => Some(
            "fn fetch_point_shadow(\n\
             \x20   light_id: u32,\n\
             \x20   frag_position: vec4<f32>,\n\
             \x20   surface_normal: vec3<f32>,\n\
             \x20   frag_coord_xy: vec2<f32>,\n\
             ) -> f32 { return 1.0; }\n",
        ),
        // 点光源的那份数据（`Lights` 里只有方向光）。字段名/次序照抄
        // `bevy_pbr::mesh_view_types::ClusteredLight`，桩里只需被解析，不必真的对。
        "bevy_pbr::mesh_view_bindings::clustered_lights" => Some(
            "struct ClusteredLightStub {\n\
             \x20   light_custom_data: vec4<f32>,\n\
             \x20   color_inverse_square_range: vec4<f32>,\n\
             \x20   position_radius: vec4<f32>,\n\
             \x20   flags: u32,\n\
             \x20   shadow_depth_bias: f32,\n\
             \x20   shadow_normal_bias: f32,\n\
             \x20   spot_light_tan_angle: f32,\n\
             \x20   soft_shadow_size: f32,\n\
             \x20   shadow_map_near_z: f32,\n\
             \x20   decal_index: u32,\n\
             \x20   range: f32,\n\
             };\n\
             struct ClusteredLightsStub { data: array<ClusteredLightStub, 64> };\n\
             @group(0) @binding(8) var<storage> clustered_lights: ClusteredLightsStub;\n",
        ),
        "bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT" => {
            Some("const POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT: u32 = 1u;\n")
        }
        "bevy_pbr::clustered_forward::view_fragment_cluster_index" => Some(
            "fn view_fragment_cluster_index(\n\
             \x20   frag_coord: vec2<f32>,\n\
             \x20   view_z: f32,\n\
             \x20   is_orthographic: bool,\n\
             ) -> u32 { return 0u; }\n",
        ),
        "bevy_pbr::clustered_forward::unpack_clusterable_object_index_ranges" => Some(
            "struct ClusterableObjectIndexRanges {\n\
             \x20   first_point_light_index_offset: u32,\n\
             \x20   first_spot_light_index_offset: u32,\n\
             \x20   first_reflection_probe_index_offset: u32,\n\
             \x20   first_irradiance_volume_index_offset: u32,\n\
             \x20   first_decal_index_offset: u32,\n\
             \x20   last_clusterable_index_offset: u32,\n\
             };\n\
             fn unpack_clusterable_object_index_ranges(\n\
             \x20   cluster_index: u32,\n\
             ) -> ClusterableObjectIndexRanges {\n\
             \x20   return ClusterableObjectIndexRanges(0u, 0u, 0u, 0u, 0u, 0u);\n\
             }\n",
        ),
        "bevy_pbr::clustered_forward::get_clusterable_object_id" => {
            Some("fn get_clusterable_object_id(index: u32) -> u32 { return index; }\n")
        }
        // shader 内的时钟（§61 的细节风读它）。运行时它在 group 0 binding 11，
        // 由渲染侧每帧写。⚠ 必须从 `mesh_view_bindings` 转出来的那个名字引
        // （Bevy 自己的 `pbr_functions.wgsl` 也是这么引的）：直接
        // `#import bevy_render::globals::globals` 在运行期报
        // "no definition in scope for identifier"（§60.3 实测踩过）。
        "bevy_pbr::mesh_view_bindings::globals" => Some(
            "struct GlobalsStub {\n\
             \x20   time: f32,\n\
             \x20   delta_time: f32,\n\
             \x20   frame_count: u32,\n\
             };\n\
             @group(0) @binding(11) var<uniform> globals: GlobalsStub;\n",
        ),
        "bevy_pbr::view_transformations::depth_ndc_to_view_z" => {
            Some("fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 { return -1.0 / max(ndc_depth, 1e-6); }\n")
        }
        _ => None,
    }
}

pub fn expand(path: &str, modules: &HashMap<String, String>, seen: &mut Vec<String>) -> String {
    let import = path.trim();
    if let Some(stub) = bevy_stub(import) {
        // ⚠ 桩也要去重：同一个 `bevy_pbr::*` 符号可能被**入口 shader** 与**库模块**
        // 各 import 一次（`planet_x::light` 与 `surface.wgsl` 都要 `view`），
        // 不去重就会把同一份 `ViewStub` / `var<uniform> view` 内联两遍 ⇒ 重定义。
        if seen.iter().any(|entry| entry == import) {
            return String::new();
        }
        seen.push(import.to_string());
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

/// 按文件名找 shader 的真本。两处都找：`assets/shaders`（槽占位与 shader 库）
/// 与 `art/shaders`（入口 shader 的内容）。
/// ⚠ 重名要报错，不能先到先得：`px_probe` 的梯度仲裁者是**唯一**判据，
/// 让它悄悄拿到占位就等于把判据换成了空壳。
pub fn shader_source_of(name: &str) -> std::path::PathBuf {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found: Vec<std::path::PathBuf> = Vec::new();
    for root in [manifest.join(SHADER_ROOT), manifest.join(CONTENT_SHADER_ROOT)] {
        let path = root.join(name);
        if path.exists() {
            found.push(path);
        }
    }
    match found.len() {
        1 => found.remove(0),
        0 => panic!(
            "哪里都找不到 shader '{name}'（找过 {SHADER_ROOT} 与 {CONTENT_SHADER_ROOT}）"
        ),
        _ => {
            let places = found
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(" / ");
            panic!("shader '{name}' 在两处都有：{places} —— 名字必须唯一，否则门与探针会各测一份")
        }
    }
}

pub fn assemble(name: &str) -> String {
    let modules = module_sources();
    let path = shader_source_of(name);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读不了 {}：{error}", path.display()));
    let mut seen = Vec::new();
    render_source(&source, &modules, &mut seen)
}
