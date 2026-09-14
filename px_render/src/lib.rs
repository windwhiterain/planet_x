pub mod art_cache;
pub mod atmosphere;
pub mod clouds;
pub mod planet;
pub mod shaders;

use bevy::prelude::*;

#[derive(Component)]
pub struct ScenePart;

#[derive(Resource)]
pub struct Canvas {
    pub size: (u32, u32),
    pub target: Handle<Image>,
}

#[derive(Component)]
pub struct OrbitCamera;

pub fn asset_root() -> String {
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("px_render/assets"));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut cursor = exe.parent().map(|path| path.to_path_buf());
        while let Some(directory) = cursor {
            candidates.push(directory.join("px_render/assets"));
            cursor = directory.parent().map(|path| path.to_path_buf());
        }
    }
    candidates
        .into_iter()
        .find(|path| path.join("shaders/atmosphere.wgsl").exists())
        .unwrap_or_else(|| std::path::PathBuf::from("px_render/assets"))
        .to_string_lossy()
        .to_string()
}
