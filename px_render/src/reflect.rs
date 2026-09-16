//! 材质契约在**渲染侧**的这一层：组装 + 反射缓存。
//!
//! 表与类型收在 `px_protocol::material`（§74.3 的契约收口），反射本身收在叶子 crate
//! `px_shader::reflect`（烘图侧也要用同一份）。这里只留两件渲染侧的事：
//!
//! 1. 把**入口文本**组装成可反射的完整 WGSL（`crate::shaders::render_source`，含 bevy 桩）；
//! 2. 缓存：反射是**纯函数**（WGSL 文本 → 契约），而 WGSL 的版本号就是它的内容键
//!    （键 = 内容，§17.1）⇒ 同一版永远反射出同一份契约。缓存键加**库指纹**：库文件换了内容时，
//!    即使入口 shader 一个字没动，也必须重新反射。
//!
//! ⚠ 库指纹的算口径只有一份，住在 `px_shader`（`modules_fingerprint`）：烘图侧把**闭包**
//! 指纹算进 shader 产物键（§52.3），这里算的是**整张模块表**。两处都不许自己搓 FNV ——
//! 各搓一份的结果是「键说没变、反射说变了」这种谁也说不清的分歧。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub use px_protocol::material::{
    MATERIAL_BIND_GROUP, MAX_PARAMS_BYTES, MaterialLayout, PARAMS_ALIGN, PARAMS_BINDING, ParamKind,
    ParamSlot, TEXTURE_SLOTS, TextureDimension, TextureSlot,
};

fn library() -> &'static (px_shader::ModuleTable, u64) {
    static LIBRARY: OnceLock<(px_shader::ModuleTable, u64)> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let modules = crate::shaders::module_sources();
        let fingerprint = px_shader::modules_fingerprint(&modules);
        (modules, fingerprint)
    })
}

type Cache = Mutex<HashMap<(u64, u64), Arc<MaterialLayout>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 「这一版 WGSL 的材质契约是什么」。`version` = 产物成员的内容键前 16 位（`slots::version_of`）。
pub fn layout_of(version: u64, name: &str, source: &str) -> Result<Arc<MaterialLayout>, String> {
    let (modules, library_hash) = library();
    let key = (version, *library_hash);
    if let Some(layout) = cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .get(&key)
    {
        return Ok(layout.clone());
    }
    let mut seen = Vec::new();
    let assembled = crate::shaders::render_source(source, modules, &mut seen);
    let layout = Arc::new(px_shader::reflect::reflect_assembled(&assembled, name)?);
    cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(key, layout.clone());
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一版**必须**反射出同一份契约（缓存的意义就在这里），而且反射来自那份文本本身。
    /// ⚠ 这里只钉**名字与类型**，不钉偏移：偏移是 shader 的事，改布局是作者的权利（§75 的 W4）。
    #[test]
    fn the_layout_comes_from_the_shader_text_and_is_cached_per_version() {
        let path = crate::shaders::shader_source_of("clouds.wgsl");
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        let first = layout_of(0x1234, "clouds.wgsl", &source).expect("反射");
        let second = layout_of(0x1234, "clouds.wgsl", &source).expect("反射");
        assert!(Arc::ptr_eq(&first, &second), "同一版走缓存，不重算");
        assert_eq!(first.param("steps").expect("steps 在").kind, ParamKind::U32);
        assert_eq!(first.param("tint").expect("tint 在").kind, ParamKind::Vec4);
        assert_eq!(
            first.texture(5).expect("第 5 格").dimension,
            TextureDimension::Cube
        );
        assert!(
            first.params_bytes <= MAX_PARAMS_BYTES,
            "参数块不许超过表里的上限"
        );
    }

    /// 版号不同就是另一条缓存条目（内容键换了一位 = 另一份契约）。
    #[test]
    fn a_different_version_is_a_different_entry() {
        let path = crate::shaders::shader_source_of("atmosphere.wgsl");
        let source = std::fs::read_to_string(&path).expect("读不了 shader");
        let one = layout_of(1, "atmosphere.wgsl", &source).expect("反射");
        let two = layout_of(2, "atmosphere.wgsl", &source).expect("反射");
        assert!(!Arc::ptr_eq(&one, &two), "不同版号不许共用同一条缓存");
        assert_eq!(one, two, "但内容一样 ⇒ 契约一样");
    }
}
