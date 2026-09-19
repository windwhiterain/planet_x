//! 本宿主的桩表：**文本住在 `px_shader::host_stubs`**，这里只把它接成宿主的入口并**钉住它**。
//!
//! ⚠ **文本为什么搬走（S8-a）**：`px_probe` 把云 shader 编到自己的设备上，它要的正是
//! 「新宿主会编出哪份文本」；而本 crate **只有 bin target**（没有 `src/lib.rs`），
//! `px_probe` 依赖不到它。规则与 `assemble::HOST_VIEW_STUB` 当初搬进 `px_shader` 时是同一条：
//! 够不到就抄一份，而抄第二份 = §66.1 那颗「同一条契约、两处文本」的雷。
//! ⇒ 文本搬进共享叶子 crate，**判据留在宿主这一侧**（下面那些测试钉的是"宿主认下来的那张表"，
//! 那是宿主的事，不是叶子 crate 的事）。搬家是**纯文本位移**：组装出来的字一个都没变，
//! 由回归集（J1/J2/J3 + 逃生门六份文件字节）复测证明。
//!
//! 三处与 Bevy 那张表的差别，逐条见下面对应的测试。

pub use px_shader::assemble::HOST_VIEW_STUB as VIEW_STUB;
pub use px_shader::assemble::bevy_stub;
pub use px_shader::host_stubs::wgpu_host_stub as stubs;
pub use px_shader::host_stubs::{DEPTH_NDC_TO_VIEW_Z, POINT_SHADOW_STUB};

#[cfg(test)]
mod tests {
    use super::*;

    /// 桩表与 Bevy 那张的差别**逐符号点名**：差哪几个、每个为什么必须差。
    ///
    /// ⚠ 这条绊线的形状是改过的：原来它写的是"**只**差 `fetch_point_shadow` 一个符号"，
    /// 而那种"数个数"的断言在真的多出第二处差别时只会说"多了"，说不出**为什么**。
    /// 现在三处各自点名、各自写清后果，剩下的符号仍然**必须逐字相同** ——
    /// 多差一个都意味着两边的 group 0 开始漂开。
    #[test]
    fn the_table_differs_from_bevys_in_exactly_three_named_symbols() {
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

        // ③ `view`：本宿主**自己拥有**这一格。Bevy 的 `View` 有七十多个字段，那张近似表里
        //    只有五格；天空盒要的 `view_from_clip` 与 `world_from_view` 在近似表里**没有**，
        //    所以这一格必须由本表说了算。⚠ 差别只许是"末尾多两格"：
        //    前五格与绑定号必须逐字相同（否则内容 shader 按名字读到的是别的字节）。
        let view = "bevy_pbr::mesh_view_bindings::view";
        assert_eq!(stubs(view), Some(VIEW_STUB), "`view` 由本表显式提供");
        let bevy_view = bevy_stub(view).expect("Bevy 那张近似表也认这一格");
        assert_ne!(stubs(view), bevy_stub(view), "差别就是这一条存在的意义");
        let head = bevy_view.split("};\n").next().expect("结构体");
        let mine = VIEW_STUB.split("};\n").next().expect("结构体");
        assert!(
            mine.starts_with(head),
            "前五格必须与 Bevy 那张逐字相同（新字段只许追加）：\n{head}\n---\n{mine}"
        );
        for field in ["view_from_clip", "world_from_view"] {
            assert!(mine.contains(field), "缺了 {field}：{mine}");
            assert!(!head.contains(field), "Bevy 那张近似表里没有 {field}");
        }

        // 其余符号：逐字相同。少一个都不行 —— 这几格是 group 0 的绑定号与结构体形状。
        let probes = [
            "bevy_pbr::forward_io::VertexOutput",
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

    /// 影子那一个符号**由本表显式提供**，而且是**真实现**（§109）。
    ///
    /// ⚠ 这一条原来是"绊线"：桩换成真实现的那一天它会响，逼着换的人是有意识地换。
    /// 现在它响过了，于是它改成钉**真实现的那几处不许化简**：8 个采样、`flip_z`、
    /// 自带的两格声明、以及"它确实在采样"（而不是又退回一个常量）。
    #[test]
    fn the_point_shadow_symbol_is_owned_by_this_host() {
        let shadow = "bevy_pbr::shadows::fetch_point_shadow";
        assert_eq!(
            stubs(shadow),
            Some(POINT_SHADOW_STUB),
            "影子走的是本表里那一格（§109 起是真实现）"
        );
        assert!(
            !POINT_SHADOW_STUB.contains("return 1.0;"),
            "桩的痕迹（`return 1.0`）不许留在真实现里"
        );
        assert_eq!(
            POINT_SHADOW_STUB
                .matches("px_sample_shadow_cubemap_at_offset(")
                .count(),
            9,
            "Gaussian 那条路是**八个**采样（一次定义 + 八次调用）"
        );
        assert!(
            POINT_SHADOW_STUB.contains("textureSampleCompareLevel("),
            "比较采样必须走 Level 那一档：`fetch_point_shadow` 的调用点有非一致控制流\
             （`shadow_sampling.wgsl:319-323`），隐式 LOD 在那种地方是未定义行为"
        );
        assert!(
            POINT_SHADOW_STUB.contains("vec3<f32>(1.0, 1.0, -1.0)"),
            "采样坐标要 flip_z（cube 是左手 y-up）"
        );
        assert!(
            POINT_SHADOW_STUB.contains("@group(0) @binding(2)")
                && POINT_SHADOW_STUB.contains("@group(0) @binding(3)"),
            "这一格要**自带** cube array 与比较采样器两格的声明（内容 shader 只 import 它）"
        );
    }

    /// 退休的符号不许被认下来：离线门应当当场报「找不到」，而不是运行期才发现画面不对。
    #[test]
    fn retired_symbols_are_not_silently_accepted() {
        assert!(stubs("bevy_pbr::shadows::fetch_directional_shadow").is_none());
        assert!(stubs("bevy_pbr::mesh_view_bindings::directional_shadow_textures").is_none());
    }
}
