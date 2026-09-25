pub use px_shader::assemble::HOST_VIEW_STUB as VIEW_STUB;
pub use px_shader::assemble::bevy_stub;
pub use px_shader::host_stubs::wgpu_host_stub as stubs;
pub use px_shader::host_stubs::{DEPTH_NDC_TO_VIEW_Z, POINT_SHADOW_STUB};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_differs_from_bevys_in_exactly_three_named_symbols() {
        let shadow = "bevy_pbr::shadows::fetch_point_shadow";
        assert_eq!(
            stubs(shadow),
            Some(POINT_SHADOW_STUB),
            "影子那一格必须由本表显式提供（不许漏下去让 `bevy_stub` 兜住）"
        );

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
            "near 不许抄成字面量：同一条契约、两个数"
        );

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

    #[test]
    fn the_point_shadow_symbol_is_owned_by_this_host() {
        let shadow = "bevy_pbr::shadows::fetch_point_shadow";
        assert_eq!(
            stubs(shadow),
            Some(POINT_SHADOW_STUB),
            "影子走的是本表里那一格"
        );
        assert!(
            POINT_SHADOW_STUB.contains("depth < stored"),
            "手动比较那一条（reverse-Z：影里是 `depth < stored`）不许丢"
        );
        assert!(
            POINT_SHADOW_STUB.contains("return sum;"),
            "它必须把八个采样加起来返回，而不是变成一个常量"
        );
        assert_eq!(
            POINT_SHADOW_STUB
                .matches("px_sample_shadow_at_offset(")
                .count(),
            9,
            "Gaussian 那条路是**八个**采样（一次定义 + 八次调用）"
        );
        assert!(
            POINT_SHADOW_STUB.contains("textureLoad("),
            "虚拟影图改**手动比较**：一页里的某一格只有 `textureLoad` 取得到，\
             比较采样器那一档取不到\"这一页里的这一格\""
        );
        assert!(
            !POINT_SHADOW_STUB.contains("px_shadow_face_uv("),
            "六面的朝向**不许**在着色器里再写一遍（用户裁决）：它从文档的 \
             `px_shadow_faces` 读 —— 这条契约从前有三份转写而且漂开过（4/5 面朝向反了）"
        );
        assert!(
            POINT_SHADOW_STUB.contains("px_shadow_faces["),
            "面选择与 UV 必须走文档给的那张基表"
        );
        assert!(
            !POINT_SHADOW_STUB.contains("vec3<f32>(1.0, 1.0, -1.0)"),
            "采样方向**不许** flip_z：我们的层不是 cube 采样，是渲染器按世界空间六面相机\
             画出来的（翻了就是按镜像找面）"
        );
        assert!(
            POINT_SHADOW_STUB.contains("@group(0) @binding(2)")
                && POINT_SHADOW_STUB.contains("@group(0) @binding(3)"),
            "这一格要**自带** cube array 与比较采样器两格的声明（内容 shader 只 import 它）"
        );
    }

    #[test]
    fn retired_symbols_are_not_silently_accepted() {
        assert!(stubs("bevy_pbr::shadows::fetch_directional_shadow").is_none());
        assert!(stubs("bevy_pbr::mesh_view_bindings::directional_shadow_textures").is_none());
    }
}
