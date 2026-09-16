use bevy::prelude::*;
use std::path::PathBuf;

#[derive(Resource)]
pub struct ShaderLibraries(pub Vec<Handle<Shader>>);

/// 「这一份是不是模块」只有一条判据（`#define_import_path`），实现住在 `px_shader`。
fn declares_import_path(source: &str) -> bool {
    px_shader::import_path_of(source).is_some()
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
// 按文本组装 shader：**实现住在 `px_shader::assemble`**（烘图侧也要组装：烘 shader 产物时
// 要反射出 schema descriptor）。这里只把本仓的路径约定与它接起来。
//
// ⚠️ 它比运行时宽松：那里把 `#import` 的整个模块递归展开，而 Bevy 只内联
//    `#import` 里点名的符号 —— 所以「测试能过」给不了「运行时能过」的保证（§46.4）。
//    `#{MATERIAL_BIND_GROUP}` 两边都替成 `px_protocol::material::MATERIAL_BIND_GROUP`
//    （= Bevy 的 3），这一条必须一致（§75 的「2/3 那颗雷」）。
// ---------------------------------------------------------------------------

pub use px_shader::assemble::{bevy_stub, expand, render_source};

pub const SHADER_ROOT: &str = "assets/shaders";

/// 入口 shader 的真本住在艺术内容里（`art/shaders/`），随场景产物走；
/// `assets/shaders/slot_*.wgsl` 只是「槽占位」（声明同样的绑定、画得出东西、
/// 一眼看得出不是成品）。两道门两边都要扫，否则真本会从「体积 / 校验」下面溜走。
pub const CONTENT_SHADER_ROOT: &str = "../art/shaders";

/// 两个根：`[0]` 库（`asset_root()/shaders`，与 `load_libraries` 扫的是同一个目录）、
/// `[1]` 入口（`<workspace>/art/shaders`）。**约定只有一份**，住在 `px_shader`。
///
/// ⚠ 库这一侧必须跟 `load_libraries` 用同一个 `asset_root()`：装载时对账的闭包就是
/// naga_oil 真去组装的那些模块（`scene::preload_shaders` 的闸门），两边指到不同目录
/// 就等于拿另一批文件在核对。
pub fn shader_roots() -> Vec<PathBuf> {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace = manifest.parent().expect("px_render 必须住在 workspace 下");
    px_shader::roots(workspace, std::path::Path::new(&crate::asset_root()))
}

/// 两个根下的全部 `.wgsl`（离线门按它扫「每个 shader 都要解析 + 校验」）。
pub fn shader_files() -> Vec<PathBuf> {
    px_shader::wgsl_files(&shader_roots()).unwrap_or_else(|err| panic!("{err}"))
}

pub fn import_path_of(source: &str) -> Option<String> {
    px_shader::import_path_of(source).map(str::to_string)
}

pub fn module_sources() -> px_shader::ModuleTable {
    px_shader::module_sources(&shader_roots()).unwrap_or_else(|err| panic!("{err}"))
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
