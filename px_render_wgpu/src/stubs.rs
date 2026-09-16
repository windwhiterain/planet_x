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

/// 视图变换表里**必须**与 Bevy 逐字同语义的那一个符号：`depth_ndc_to_view_z`。
///
/// ⚠ 这是我们与 Bevy 那张桩表的**第二处**语义差别（第一处是 [`POINT_SHADOW_STUB`]），
/// 而它不属于"先凑合"那一类：`px_shader::assemble::bevy_stub` 给的是
/// `-1.0 / max(ndc_depth, 1e-6)`，Bevy 的真本是
/// `-perspective_camera_near() / ndc_depth`
/// （`bevy_pbr-0.19.1/src/render/view_transformations.wgsl:172`），
/// 其中 `perspective_camera_near() = view.clip_from_view[3][2]`。两个式子**差一个 near**。
///
/// 差多少（本机实测：Vulkan／RTX 3060／960×640／`orbit-bare-nolight`）：
/// - 深度预通道在盘心写下 **0.045503**；真距离 = `0.1 / 0.045503` = **2.198**
///   （与相机到行星表面 3.15 − 1.0 ≈ 2.15 吻合），而 `-1/depth` 给的是 **21.98**；
/// - 大气里 `end = min(entry + 2·outer·cos, length(scene))` 于是**永远取前者**（截断不生效）：
///   弦长从 0.067 变成 2.2 ⇒ alpha 从 **≈0.034** 变成 **≈0.48**；
/// - 症状：一颗**被冲淡的灰蓝球**（对照图 `target/tmp/experiment-atm-real-depth.png`），
///   而不是 oracle 那种"薄薄一层纱 + 边缘一圈晕"。
///
/// ⚠ 它**跑得起来、也不报错** —— 建管线、出图、哈希全都有值，只有画面不对。
/// 这一条绊线就是为这种"看起来也能跑"的桩准备的，所以上面那几个数留在现场。
///
/// ⚠ 为什么写 `view.clip_from_view[3][2]` 而不是把 `0.1` 抄进来：near 是**相机矩阵**里的数，
/// 抄进 Rust 侧（或抄进这个函数体）就是 §66.1 那颗「同一条契约、两个数」的雷 ——
/// 换一台相机就漂开，而画面上只表现为"大气的厚薄不对"。
///
/// ⚠ 函数体引用了 `view`，所以它**只能**进那些同时 import 了 `view` 的 shader
/// （`atmosphere.wgsl` / `clouds.wgsl` 都是）。判据在
/// [`tests::the_depth_override_assembles_against_the_real_content`]：少一个声明，
/// naga 会当场拒 —— 那正是我们要的失败方式。
pub const DEPTH_NDC_TO_VIEW_Z: &str = "fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 {\n\
                                       \x20   return -view.clip_from_view[3][2] / ndc_depth;\n\
                                       }\n";

/// 这个宿主的桩表。函数指针（不是泛型、不是 trait）：组装器只有一份，见 `assemble::Stubs`。
pub fn stubs(symbol: &str) -> Option<&'static str> {
    match symbol {
        "bevy_pbr::shadows::fetch_point_shadow" => Some(POINT_SHADOW_STUB),
        "bevy_pbr::view_transformations::depth_ndc_to_view_z" => Some(DEPTH_NDC_TO_VIEW_Z),
        other => bevy_stub(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 桩表与 Bevy 那张的差别**逐符号点名**：差哪几个、每个为什么必须差。
    ///
    /// ⚠ 这条绊线的形状是改过的：原来它写的是"**只**差 `fetch_point_shadow` 一个符号"，
    /// 而那种"数个数"的断言在真的多出第二处差别时只会说"多了"，说不出**为什么**。
    /// 现在两处各自点名、各自写清后果，剩下的符号仍然**必须逐字相同** ——
    /// 多差一个都意味着两边的 group 0 开始漂开。
    #[test]
    fn the_table_differs_from_bevys_in_exactly_two_named_symbols() {
        // ① 影子采样：本表**自己拥有**这一格（`bevy_stub` 今天给的也是同一段占位文本，
        //    差别在**语义**：Bevy 宿主运行期拿到的是 Bevy 的真实现，而我们拿到的是这段
        //    占位 —— S3 换成采样我们自己的 cube 影子图）。绊线见下一条测试。
        let shadow = "bevy_pbr::shadows::fetch_point_shadow";
        assert_eq!(
            stubs(shadow),
            Some(POINT_SHADOW_STUB),
            "影子那一格必须由本表显式提供（不许漏下去让 `bevy_stub` 兜住）"
        );

        // ② 视图变换：near 必须从**相机矩阵**来，不能是 `1.0`。
        //    症状是一颗被冲淡的灰蓝球（数在 `DEPTH_NDC_TO_VIEW_Z` 的注释里）。
        let depth = "bevy_pbr::view_transformations::depth_ndc_to_view_z";
        assert_eq!(stubs(depth), Some(DEPTH_NDC_TO_VIEW_Z));
        assert_ne!(
            stubs(depth),
            bevy_stub(depth),
            "深度反投影**必须**与 Bevy 的表不同：Bevy 那份是 `-1.0 / ndc_depth`，少一个 near"
        );
        assert!(
            DEPTH_NDC_TO_VIEW_Z.contains("view.clip_from_view[3][2]"),
            "near 要从相机矩阵里取"
        );
        assert!(
            !DEPTH_NDC_TO_VIEW_Z.contains("0.1"),
            "near 不许抄成字面量（§66.1：同一条契约、两个数）"
        );

        // 其余符号：逐字相同。少一个都不行 —— 这几格是 group 0 的绑定号与结构体形状。
        let probes = [
            "bevy_pbr::forward_io::VertexOutput",
            "bevy_pbr::mesh_view_bindings::view",
            "bevy_pbr::mesh_view_bindings::lights",
            "bevy_pbr::mesh_view_bindings::depth_prepass_texture",
            "bevy_pbr::mesh_view_bindings::clustered_lights",
            "bevy_pbr::mesh_view_bindings::globals",
            "bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT",
        ];
        for symbol in probes {
            assert_eq!(
                stubs(symbol),
                bevy_stub(symbol),
                "{symbol} 在这个宿主里必须与 Bevy 那张表逐字相同（绑定号靠它保证）"
            );
        }
    }

    /// 覆盖掉的那一格**在真内容里组装得出、校验得过**。
    ///
    /// 为什么必须有这一条：函数体里引用了 `view`，而 `view` 是**另一条 import** 带进来的
    /// （`bevy_pbr::mesh_view_bindings::view`）。谁的 shader 只 import 了
    /// `depth_ndc_to_view_z` 而没 import `view`，组装出来的文本就少一个声明 ——
    /// naga 当场拒。这一条把"两处 import 的搭配"钉在真内容上，而不是靠人去记。
    #[test]
    fn the_depth_override_assembles_against_the_real_content() {
        let modules = crate::shader::modules();
        for name in ["atmosphere.wgsl", "clouds.wgsl"] {
            let (source, path) = crate::shader::source_of(name);
            let assembled = crate::shader::assemble(&source, &modules, stubs);
            assert!(
                assembled.contains("return -view.clip_from_view[3][2] / ndc_depth;"),
                "{name}（{}）没吃到那一格覆盖",
                path.display()
            );
            assert!(
                assembled.contains("var<uniform> view"),
                "{name}（{}）少了 `view` 的声明 —— 覆盖后的函数体会引用不到它",
                path.display()
            );
            crate::shader::validate(name, &assembled)
                .unwrap_or_else(|err| panic!("覆盖之后 {name} 组装不过：{err}"));
        }
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
