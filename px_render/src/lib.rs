pub mod art_cache;
pub mod digest;
pub mod material;
pub mod mesh;
pub mod passes;
pub mod reflect;
pub mod scene;
pub mod shaders;
pub mod slots;

use bevy::prelude::*;

/// 「重建世界时只动这些」的标记（§13 的三条结构规则：相机与灯是另外两类）。
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
        .find(|path| path.join("shaders/common.wgsl").exists())
        .unwrap_or_else(|| std::path::PathBuf::from("px_render/assets"))
        .to_string_lossy()
        .to_string()
}
