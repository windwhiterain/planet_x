use px_protocol::material::MATERIAL_BIND_GROUP;

use crate::ModuleTable;

pub type Stubs = fn(&str) -> Option<&'static str>;

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
        "bevy_pbr::mesh_view_bindings::lights" => Some(
            "struct LightsStub {\n\
             \x20   ambient_color: vec4<f32>,\n\
             \x20   n_point_lights: vec4<u32>,\n\
             };\n\
             @group(0) @binding(1) var<uniform> lights: LightsStub;\n",
        ),
        "bevy_pbr::shadows::fetch_point_shadow" => Some(
            "fn fetch_point_shadow(\n\
             \x20   light_id: u32,\n\
             \x20   frag_position: vec4<f32>,\n\
             \x20   surface_normal: vec3<f32>,\n\
             \x20   frag_coord_xy: vec2<f32>,\n\
             \x20   ) -> f32 { return 1.0; }\n",
        ),
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
             \x20   ) -> u32 { return 0u; }\n",
        ),
        "bevy_pbr::clustered_forward::unpack_clusterable_object_index_ranges" => Some(
            "struct ClusterableObjectIndexRanges {\n\
             \x20   first_point_light_index_offset: u32,\n\
             \x20   first_spot_light_index_offset: u32,\n\
             \x20   first_reflection_probe_index_offset: u32,\n\
             \x20   first_irradiance_volume_index_offset: u32,\n\
             \x20   first_decal_index_offset: u32,\n\
             \x20   last_clusterable_index_offset: u32,\n\
             \x20   };\n\
             fn unpack_clusterable_object_index_ranges(\n\
             \x20   cluster_index: u32,\n\
             \x20   ) -> ClusterableObjectIndexRanges {\n\
             \x20   return ClusterableObjectIndexRanges(0u, 0u, 0u, 0u, 0u, 0u);\n\
             \x20   }\n",
        ),
        "bevy_pbr::clustered_forward::get_clusterable_object_id" => {
            Some("fn get_clusterable_object_id(index: u32) -> u32 { return index; }\n")
        }
        "bevy_pbr::mesh_view_bindings::globals" => Some(
            "struct GlobalsStub {\n\
             \x20   time: f32,\n\
             \x20   delta_time: f32,\n\
             \x20   frame_count: u32,\n\
             \x20   };\n\
             @group(0) @binding(11) var<uniform> globals: GlobalsStub;\n",
        ),
        "bevy_pbr::view_transformations::depth_ndc_to_view_z" => Some(
            "fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 { return -1.0 / max(ndc_depth, 1e-6); }\n",
        ),
        _ => None,
    }
}

pub const HOST_VIEW_STUB: &str = "struct ViewStub {\n\
                                   \x20   world_position: vec3<f32>,\n\
                                   \x20   exposure: f32,\n\
                                   \x20   view_from_world: mat4x4<f32>,\n\
                                   \x20   clip_from_view: mat4x4<f32>,\n\
                                   \x20   viewport: vec4<f32>,\n\
                                   \x20   view_from_clip: mat4x4<f32>,\n\
                                   \x20   world_from_view: mat4x4<f32>,\n\
                                   };\n\
                                   @group(0) @binding(0) var<uniform> view: ViewStub;\n";

pub fn expand(import: &str, modules: &ModuleTable, stubs: Stubs, seen: &mut Vec<String>) -> String {
    let import = import.trim();
    if let Some(stub) = stubs(import) {
        if seen.iter().any(|entry| entry == import) {
            return String::new();
        }
        seen.push(import.to_string());
        return stub.to_string();
    }

    let Some(module) = crate::module_of(import, modules) else {
        if import.contains("::") {
            panic!("未知的 import：{import}");
        }
        return String::new();
    };

    if seen.iter().any(|entry| entry == module) {
        return String::new();
    }
    let source = modules
        .get(module)
        .expect("module_of 给出来的名字一定在表里");
    seen.push(module.to_string());
    render_source(source, modules, stubs, seen)
}

pub fn render_source(
    source: &str,
    modules: &ModuleTable,
    stubs: Stubs,
    seen: &mut Vec<String>,
) -> String {
    let mut out = String::new();
    let mut imports: Vec<String> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(crate::IMPORT_PREFIX) {
            imports.push(rest.trim().to_string());
            continue;
        }
        if trimmed.starts_with(crate::MODULE_PREFIX) {
            continue;
        }
        out.push_str(&line.replace("#{MATERIAL_BIND_GROUP}", &MATERIAL_BIND_GROUP.to_string()));
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
                prelude.push_str(&expand(
                    &format!("{module}::{symbol}"),
                    modules,
                    stubs,
                    seen,
                ));
            }
            continue;
        }
        prelude.push_str(&expand(&import, modules, stubs, seen));
    }
    format!("{prelude}\n{out}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bind_group_placeholder_becomes_the_runtime_number() {
        let modules = ModuleTable::new();
        let mut seen = Vec::new();
        let text = render_source(
            "@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: f32;\n",
            &modules,
            bevy_stub,
            &mut seen,
        );
        assert_eq!(
            text.trim(),
            format!("@group({MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: f32;")
        );
        assert_eq!(
            MATERIAL_BIND_GROUP, 3,
            "Bevy 的 MATERIAL_BIND_GROUP_INDEX 是 3；改这个数就等于换了一套绑定"
        );
    }

    #[test]
    fn an_import_is_inlined_once_and_stubs_are_deduplicated() {
        let mut modules = ModuleTable::new();
        modules.insert(
            "planet_x::light".to_string(),
            "#define_import_path planet_x::light\n#import bevy_pbr::mesh_view_bindings::view\nfn sun() -> f32 { return view.exposure; }\n".to_string(),
        );
        let mut seen = Vec::new();
        let text = render_source(
            "#import planet_x::light::sun\n#import bevy_pbr::mesh_view_bindings::view\nfn f() -> f32 { return sun(); }\n",
            &modules,
            bevy_stub,
            &mut seen,
        );
        assert_eq!(
            text.matches("var<uniform> view").count(),
            1,
            "同一个桩内联两遍就是重定义：{text}"
        );
        assert_eq!(text.matches("fn sun()").count(), 1);
    }

    #[test]
    fn an_unknown_import_is_an_error_and_a_bare_name_is_not() {
        let modules = ModuleTable::new();
        let mut seen = Vec::new();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_source("#import nobody::knows\n", &modules, bevy_stub, &mut seen)
        }));
        assert!(
            caught.is_err(),
            "带 :: 的未知 import 必须报错，不许静默丢掉"
        );
    }

    #[test]
    fn the_stub_table_comes_from_the_host() {
        fn empty(_: &str) -> Option<&'static str> {
            None
        }
        let modules = ModuleTable::new();
        let mut seen = Vec::new();
        let text = render_source(
            "#import bevy_pbr::mesh_view_bindings::view\nfn f() -> f32 { return view.exposure; }\n",
            &modules,
            bevy_stub,
            &mut seen,
        );
        assert!(
            text.contains("var<uniform> view"),
            "Bevy 那张表认这个符号：{text}"
        );

        let mut other_seen = Vec::new();
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            render_source(
                "#import bevy_pbr::mesh_view_bindings::view\n",
                &modules,
                empty,
                &mut other_seen,
            )
        }));
        assert!(
            caught.is_err(),
            "换一张空表，同一个符号就解不开了 —— 说明认符号的是**传进来的那张表**，\
             不是组装器里写死的一份"
        );
    }

    #[test]
    fn the_host_view_stub_is_bevys_five_fields_plus_two_inverses() {
        let bevy = bevy_stub("bevy_pbr::mesh_view_bindings::view").expect("Bevy 那张表认这个符号");
        let head = bevy.split("};\n").next().expect("Bevy 那份是个结构体");
        let mine = HOST_VIEW_STUB
            .split("};\n")
            .next()
            .expect("这一份也是结构体");
        assert!(
            mine.starts_with(head),
            "前五格必须与 Bevy 那张近似表逐字相同（新字段只许追加在末尾）：\n{head}\n---\n{mine}"
        );
        for field in [
            "view_from_clip: mat4x4<f32>",
            "world_from_view: mat4x4<f32>",
        ] {
            assert!(mine.contains(field), "缺了 {field}：\n{mine}");
            assert!(
                !head.contains(field),
                "Bevy 那张近似表里本来没有 {field} —— 有的话这一格就不必由本工程自己声明"
            );
        }
        assert!(
            HOST_VIEW_STUB.contains("@group(0) @binding(0) var<uniform> view"),
            "绑定号必须还是 0（绑定号会改像素）"
        );
    }
}
