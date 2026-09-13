use bevy::prelude::*;

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
