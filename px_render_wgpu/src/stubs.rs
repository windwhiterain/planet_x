//! **裸 wgpu 宿主自己的桩表** —— 这就是它的 group 0 契约。
//!
//! 与 Bevy 宿主的关系（§103.1）：
//!
//! - Bevy 宿主那条路，`#import bevy_pbr::*` 在**运行期**由 naga_oil 拿 Bevy 自己的
//!   `bevy_pbr` 兑现；今天那张 `bevy_stub` 只服务"离线把文本拼出来"（离线门与反射）。
//! - 这个宿主**没有 naga_oil**，所以桩表**就是运行期真正用的那一份**：
//!   组装出来的文本直接喂给 `create_shader_module`。
//!
//! 因此这里只覆盖**一个**符号：`fetch_point_shadow`。
//! 其余（`view` / `lights` / `clustered_lights` / `globals` / `depth_prepass_texture` /
//! `VertexOutput` / `depth_ndc_to_view_z` / `POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT`）
//! **原样转给 Bevy 那张表** —— 于是绑定号与字段次序**由构造保证**与 Bevy 一致，
//! 而不是靠两边各抄一遍再祈祷它们不漂（§104 第 1 条：绑定号会改像素，省下的每一步
//! 都是判据上的噪声）。§65 记的那次"cube 从第 1 格挪到第 5 格就差 22–33 个像素"至今没归因。

/// 3D 顶点输出。字段与 `bevy_pbr::forward_io::VertexOutput` 同形：
/// 内容 shader 按 `position` / `world_position` / `world_normal` / `uv` 四个 location 取。
pub use px_shader::assemble::bevy_stub;

/// 点光 cube 影子：**S3 换成真实现**（采样我们自己的 cube 影子图）。
///
/// 今天这一份是"先让文本解析得过去"的桩，**故意**摆在最显眼的地方：
/// 它是这个宿主与 Bevy 宿主在 group 0 上**唯一**的语义差别，
/// 也是 §104 第 2 条那个"全局唯一没有现成参考的活"。
///
/// 形状（参数个数与名字）必须与 Bevy 的 `fetch_point_shadow` 相同 —— 内容 shader
/// 是按那个签名调的，签名改了这个符号就解不开了。
pub const POINT_SHADOW_STUB: &str = "fn fetch_point_shadow(\n\
                                     \x20   light_id: u32,\n\
                                     \x20   frag_position: vec4<f32>,\n\
                                     \x20   surface_normal: vec3<f32>,\n\
                                     \x20   frag_coord_xy: vec2<f32>,\n\
                                     \x20   ) -> f32 { return 1.0; }\n";

/// 这个宿主的桩表。函数指针（不是泛型、不是 trait）：组装器只有一份，见 `assemble::Stubs`。
pub fn stubs(symbol: &str) -> Option<&'static str> {
    match symbol {
        "bevy_pbr::shadows::fetch_point_shadow" => Some(POINT_SHADOW_STUB),
        other => bevy_stub(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 桩表与 Bevy 那张只差一个符号 —— 多差一个都意味着两边的 group 0 开始漂开。
    #[test]
    fn only_the_point_shadow_differs_from_the_bevy_table() {
        let probes = [
            "bevy_pbr::forward_io::VertexOutput",
            "bevy_pbr::mesh_view_bindings::view",
            "bevy_pbr::mesh_view_bindings::lights",
            "bevy_pbr::mesh_view_bindings::depth_prepass_texture",
            "bevy_pbr::mesh_view_bindings::clustered_lights",
            "bevy_pbr::mesh_view_bindings::globals",
            "bevy_pbr::view_transformations::depth_ndc_to_view_z",
            "bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT",
        ];
        for symbol in probes {
            assert_eq!(
                stubs(symbol),
                bevy_stub(symbol),
                "{symbol} 在这个宿主里必须与 Bevy 那张表逐字相同（绑定号靠它保证）"
            );
        }
        let shadow = "bevy_pbr::shadows::fetch_point_shadow";
        assert!(stubs(shadow).is_some(), "影子那一个符号必须解得开");
    }

    /// 影子那一个符号**由本表显式提供**（不是漏下去让 `bevy_stub` 兜住的）。
    ///
    /// ⚠ 这是一条**绊线**：S3 会把 [`POINT_SHADOW_STUB`] 换成真实现（采样我们自己的
    /// cube 影子图），那时这条断言会响 —— 那正是它存在的意义：
    /// 换实现必须是有意识的一步，而不是某次顺手改动。
    #[test]
    fn the_point_shadow_symbol_is_owned_by_this_host() {
        let shadow = "bevy_pbr::shadows::fetch_point_shadow";
        assert_eq!(
            stubs(shadow),
            Some(POINT_SHADOW_STUB),
            "影子走的是本表里那一格（S1 阶段它与 Bevy 那张同形；S3 换成真实现）"
        );
    }

    /// 退休的符号不许被认下来：离线门应当当场报「找不到」，而不是运行期才发现画面不对。
    #[test]
    fn retired_symbols_are_not_silently_accepted() {
        assert!(stubs("bevy_pbr::shadows::fetch_directional_shadow").is_none());
        assert!(stubs("bevy_pbr::mesh_view_bindings::directional_shadow_textures").is_none());
    }
}
