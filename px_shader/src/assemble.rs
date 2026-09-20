//! 组装：把一份入口 WGSL 加上它 `#import` 的东西，变成一份能交给 naga 的完整文本。
//!
//! 为什么住在叶子 crate：**烘图侧也要组装**（烘 shader 产物时要反射出 schema descriptor，
//! 见 `px_graph::write_shader`），而 `px_render` 拖着 bevy 进不去。
//! ⚠ 这一句里的 `px_render` 是**已删的 Bevy 宿主**（§154）；§157 起同一个名字归 wgpu 宿主，
//! 而那条理由对**今天那支**同样成立（它拖整棵 wgpu 树）。
//!
//! ⚠ 它比运行期宽松：这里把 `#import` 的整个模块递归展开，而 Bevy（naga_oil）只内联
//! `#import` 里点名的符号 —— 所以「离线门能过」给不了「运行期能过」的保证（§46.4）。
//! 有一条是必须对齐的：`#{MATERIAL_BIND_GROUP}` 替成**运行期那个数**
//! （[`px_protocol::material::MATERIAL_BIND_GROUP`]，Bevy 的 `MATERIAL_BIND_GROUP_INDEX`），
//! 否则两边组的不是同一份东西（§75 记的「2/3 那颗雷」）。

use px_protocol::material::MATERIAL_BIND_GROUP;

use crate::ModuleTable;

/// 外部符号（`bevy_pbr::…`）的**桩表**：由**宿主**提供，组装器自己一个符号都不认识。
///
/// 为什么是**函数指针**而不是泛型/trait（§103.1 原稿写的是 `&dyn Stubs`）：
/// 这是"能动态的就动态、减少单态化时间"那一条（§100 的用户口径）——
/// 两个宿主对同一批 `#import bevy_pbr::*` 的兑现方式不同（Bevy 宿主运行期由 naga_oil 用
/// Bevy 自己的实现兑现；裸 wgpu 宿主没有 naga_oil，必须自己兑），但组装器只有一份：
/// 它不该为"宿主是谁"单态化出两份代码，也不该为一次间接调用养一张 vtable。
///
/// ⚠ 桩表的**内容**是宿主的判据来源：谁多认一个已经退休的符号（比如平行光的
/// `fetch_directional_shadow`），离线门就该报「找不到这个符号」，而不是运行期才发现画面不对。
pub type Stubs = fn(&str) -> Option<&'static str>;

/// **Bevy 宿主**（`px_render`）与烘图侧（`px_graph`）用的那张表：把 `bevy_pbr::*` 替成
/// **最小声明**，只让离线文本解析得过去，**不是**运行期真正用的实现。
///
/// ⚠ **S8-c 标注：上面的"Bevy 宿主（`px_render`）"已经不在了**（§154 删了那个 crate）。
/// 这张表今天**还有真实用户**，所以留着 —— 烘图侧（`px_graph` 的 shader 路、`px_scene::frame`
/// 烘帧材质）与 `px_render::stubs` 那条"影子那一格必须由本表显式提供"的对照判据都在用它。
/// ⇒ 读这一行时把"两个用户"读成"**烘图侧 + 唯一那支宿主的对照判据**"。
/// ⚠ **§157（2026-09-19）：`px_render` 这个名字换过手** —— 上面"Bevy 宿主（`px_render`）"里
/// 那个名字指**已删的 Bevy 宿主**，而同一段里的 `px_render::stubs` 指**现在的 wgpu 宿主**
/// （它改名前叫 `px_render_wgpu`）。两个所指靠日期切：§100–§156 的旧义 / §157 起的新义。
/// ⚠ 名字里的 `bevy_pbr::` **按 §155 保留**：那是**出处指针**，不是过时的名字（改名会换掉所有产物键）。
///
/// ⚠ 运行期归 naga_oil 按 Bevy 自己的 `bevy_pbr` 兑现；这张表只服务"离线把文本拼出来"
/// 这一件事（`px_shader::reflect` 与烘图侧）。
/// 裸 wgpu 宿主（`px_render`）**不传这张**，它传自己那份 —— 文本住在
/// [`crate::host_stubs`]（S8-a 从宿主 crate 搬过来，理由见那个模块的头），
/// 含真的 cube 影子实现。
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
        // 自写表面材质读的是**同一个**光源 uniform（`lights`）里的**环境光**：强度只有一份来源
        // （相机的 `AmbientLight`），不再往材质 params 里抄一遍 —— 抄一遍就是"同一个值、两处维护"。
        // ⚠ 桩里**只留 `ambient_color`**：平行光已经从渲染器里删掉了（§64.9），谁再想读
        //    `directional_lights` 就该在离线门上直接报错，而不是运行期才发现画面不对。
        "bevy_pbr::mesh_view_bindings::lights" => Some(
            "struct LightsStub {\n\
             \x20   ambient_color: vec4<f32>,\n\
             \x20   n_point_lights: vec4<u32>,\n\
             };\n\
             @group(0) @binding(1) var<uniform> lights: LightsStub;\n",
        ),
        // 点光源的影：cube shadow map。§60
        // ⚠ 平行光那一支（`fetch_directional_shadow`）已经退休（§64.9：宇宙里没有平行光），
        //    桩也删了 —— 谁再引它，离线门直接报"找不到这个符号"。
        "bevy_pbr::shadows::fetch_point_shadow" => Some(
            "fn fetch_point_shadow(\n\
             \x20   light_id: u32,\n\
             \x20   frag_position: vec4<f32>,\n\
             \x20   surface_normal: vec3<f32>,\n\
             \x20   frag_coord_xy: vec2<f32>,\n\
             \x20   ) -> f32 { return 1.0; }\n",
        ),
        // 点光源的那份数据（`Lights` 里只有方向光）。字段名/次序照抄
        // `bevy_pbr::mesh_view_types::ClusteredLight`，桩里只需被解析，不必真的对。
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
        // shader 内的时钟（§61 的细节风读它）。运行时它在 group 0 binding 11，
        // 由渲染侧每帧写。⚠ 必须从 `mesh_view_bindings` 转出来的那个名字引
        // （Bevy 自己的 `pbr_functions.wgsl` 也是这么引的）：直接
        // `#import bevy_render::globals::globals` 在运行期报
        // "no definition in scope for identifier"（§60.3 实测踩过）。
        "bevy_pbr::mesh_view_bindings::globals" => Some(
            "struct GlobalsStub {\n\
             \x20   time: f32,\n\
             \x20   delta_time: f32,\n\
             \x20   frame_count: u32,\n\
             \x20   };\n\
             @group(0) @binding(11) var<uniform> globals: GlobalsStub;\n",
        ),
        "bevy_pbr::view_transformations::depth_ndc_to_view_z" => {
            Some("fn depth_ndc_to_view_z(ndc_depth: f32) -> f32 { return -1.0 / max(ndc_depth, 1e-6); }\n")
        }
        _ => None,
    }
}

/// **本工程自己**的 `view` 声明 —— 裸 wgpu 宿主运行期真正兑现的那一份。
///
/// 与 [`bevy_stub`] 的关系：那张表是"离线把 `bevy_pbr::*` 拼出来"的**近似**（Bevy 真正的 `View`
/// 有七十多个字段，那张表里只有五格）。这一份不是近似，它是**契约**：宿主侧那个
/// `#[repr(C)]` 的 `ViewUniform` 必须与它逐字对应，而两份的对账在宿主的 `cargo test` 里
/// （`px_render::group0` 的布局判据，偏移/大小/成员名逐格比）。
///
/// ⚠ 为什么文本住在**共享的叶子 crate**、而不是宿主自己的桩表里：帧材质
/// （`art/frame/skybox.wgsl`）是**宿主自有**的 WGSL，而它必须在**烘图时**被反射
/// （参数块的三档校验在烘图时就做，见 `px_scene::frame`）；烘图侧够不到宿主 crate ——
/// `px_render` 拖着整棵 wgpu 树（§100：烘图侧要快）。所以这一格文本只有**一处**：
/// 宿主的桩表返回它，烘图侧的桩表（`bevy_stub` + 这一格覆盖）也返回它。
/// 抄成两份就是 §66.1 那颗「同一条契约、两个数字」的雷：漂开的那天，烘图侧校验过的参数
/// 与宿主反射出来的布局不是同一份东西，而画面上只表现为"某几个像素不一样"。
///
/// ⚠ 最后两格（`view_from_clip` / `world_from_view`）是**追加在末尾**的，为的是天空盒
/// 片元阶段重建视线方向（Bevy 的 `skybox.wgsl` 走的就是这两条逆矩阵，不是插值下来的
/// 裁剪坐标）：追加 ⇒ 前面五格的偏移一个都不动，已经烘好的内容 shader 按名字读，读到的是
/// 同一格。⚠ 两条逆矩阵都由宿主用**逐位移植的通用逆**（`px_render::mat4::inverse`）算，
/// **不许**在 shader 里求逆、也不许换成解析逆（§110.1.1 实测：解析逆差 1–2 ulp）。
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

/// 展开一条 `#import`：外部符号走宿主的桩表，本仓模块走文本内联（去重）。
pub fn expand(import: &str, modules: &ModuleTable, stubs: Stubs, seen: &mut Vec<String>) -> String {
    let import = import.trim();
    if let Some(stub) = stubs(import) {
        // ⚠ 桩也要去重：同一个 `bevy_pbr::*` 符号可能被**入口 shader** 与**库模块**
        // 各 import 一次（`planet_x::light` 与 `surface.wgsl` 都要 `view`），
        // 不去重就会把同一份 `ViewStub` / `var<uniform> view` 内联两遍 ⇒ 重定义。
        if seen.iter().any(|entry| entry == import) {
            return String::new();
        }
        seen.push(import.to_string());
        return stub.to_string();
    }

    // 模块名怎么认（整串是模块名 / 最长前缀）只有一条规则（`crate::module_of`）：
    // 这里宽松地递归展开，运行期由 naga_oil 按同一批模块名解析（§46.4 的差别只在
    // 「内联整个模块」还是「只内联点名的符号」，模块名本身不许有两套口径）。
    let Some(module) = crate::module_of(import, modules) else {
        if import.contains("::") {
            panic!("未知的 import：{import}");
        }
        // 没有 `::` 又不是模块名 ⇒ 什么也展开不出来（老行为，不报错）。
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

/// 入口 / 模块 → 可解析的完整 WGSL。
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
                prelude.push_str(&expand(&format!("{module}::{symbol}"), modules, stubs, seen));
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
        assert!(caught.is_err(), "带 :: 的未知 import 必须报错，不许静默丢掉");
    }

    /// 桩表是**宿主给的参数**：同一个符号，两张表给两份文本。
    /// 这条是 §103.1 的判据 —— 组装器里不许再有"bevy 那一份"的暗默认。
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
        assert!(text.contains("var<uniform> view"), "Bevy 那张表认这个符号：{text}");

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

    /// 本宿主那份 `view` 与 Bevy 那张近似表的**关系**：同一格绑定号、同样的前五格，
    /// 末尾多两格逆矩阵。
    ///
    /// 为什么这算一条判据：两条路（宿主运行期 / 烘图侧反射帧材质）共用这一格文本，而
    /// "共用"只有在它确实是"Bevy 那五格 + 两格"时才有意义 —— 谁把顺序改了、或者把新字段
    /// 插在中间，前面五格的偏移就动了，而**内容 shader 是按名字读的**（读到的还是同一格，
    /// 所以画面不会立刻错，只会某天开始错）。
    #[test]
    fn the_host_view_stub_is_bevys_five_fields_plus_two_inverses() {
        let bevy = bevy_stub("bevy_pbr::mesh_view_bindings::view").expect("Bevy 那张表认这个符号");
        let head = bevy.split("};\n").next().expect("Bevy 那份是个结构体");
        let mine = HOST_VIEW_STUB.split("};\n").next().expect("这一份也是结构体");
        assert!(
            mine.starts_with(head),
            "前五格必须与 Bevy 那张近似表逐字相同（新字段只许追加在末尾）：\n{head}\n---\n{mine}"
        );
        for field in ["view_from_clip: mat4x4<f32>", "world_from_view: mat4x4<f32>"] {
            assert!(mine.contains(field), "缺了 {field}：\n{mine}");
            assert!(
                !head.contains(field),
                "Bevy 那张近似表里本来没有 {field} —— 有的话这一格就不必由本工程自己声明"
            );
        }
        assert!(
            HOST_VIEW_STUB.contains("@group(0) @binding(0) var<uniform> view"),
            "绑定号必须还是 0（§104 第 1 条：绑定号会改像素）"
        );
    }
}
