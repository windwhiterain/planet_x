//! **裸 wgpu 宿主自己的桩表** —— 这就是它的 group 0 契约。
//!
//! 与 Bevy 宿主的关系（§103.1）：
//!
//! - Bevy 宿主那条路，`#import bevy_pbr::*` 在**运行期**由 naga_oil 拿 Bevy 自己的
//!   `bevy_pbr` 兑现；今天那张 `bevy_stub` 只服务"离线把文本拼出来"（离线门与反射）。
//! - 这个宿主**没有 naga_oil**，所以桩表**就是运行期真正用的那一份**：
//!   组装出来的文本直接喂给 `create_shader_module`。
//!
//! 因此这里只覆盖**三**个符号：`fetch_point_shadow`、`depth_ndc_to_view_z`、`view`。
//! 其余（`lights` / `clustered_lights` / `globals` / `depth_prepass_texture` /
//! `VertexOutput` / `POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT`）
//! **原样转给 Bevy 那张表** —— 于是绑定号与字段次序**由构造保证**与 Bevy 一致，
//! 而不是靠两边各抄一遍再祈祷它们不漂（§104 第 1 条：绑定号会改像素，省下的每一步
//! 都是判据上的噪声）。§65 记的那次"cube 从第 1 格挪到第 5 格就差 22–33 个像素"至今没归因。
//!
//! ⚠ §109 起 `fetch_point_shadow` 那一格是**真实现**（不再是"先让文本解析得过去"的桩）：
//! 它逐句抄 Bevy 的 Gaussian 采样路，并**自带** binding 2/3 两格的声明
//! （内容 shader 只 import 这个符号，而 Bevy 那边那两格是另一条 import 顺带带进来的）。

/// 3D 顶点输出。字段与 `bevy_pbr::forward_io::VertexOutput` 同形：
/// 内容 shader 按 `position` / `world_position` / `world_normal` / `uv` 四个 location 取。
pub use px_shader::assemble::bevy_stub;

/// 本宿主 `view` 的声明：**与 Bevy 那张近似表差在末尾两格逆矩阵**（§135）。
///
/// ⚠ 文本**住在 `px_shader::assemble`**（[`px_shader::assemble::HOST_VIEW_STUB`]），不是这里 ——
/// 因为烘图侧也要反射帧材质（`art/frame/skybox.wgsl` 引 `view.view_from_clip`），
/// 而烘图侧依赖不到这个 crate（`px_render_wgpu` 拖着整棵 wgpu 树）。文本只有一份，
/// 这里只是**把它认下来**。抄第二份 = §66.1 那颗「同一条契约、两个数字」的雷。
pub use px_shader::assemble::HOST_VIEW_STUB as VIEW_STUB;

/// 点光 cube 影子：**真实现**（§109）。
///
/// 逐句抄 `bevy_pbr-0.19.1/src/render/shadows.wgsl:19-69`（`fetch_point_shadow`）与
/// `shadow_sampling.wgsl` 那条 **Gaussian** 路（`ShadowFilteringMethod` 的缺省档）：
/// `sample_shadow_cubemap`（`:517-539`）→ `sample_shadow_cubemap_gaussian`（`:423-460`）
/// → `sample_shadow_cubemap_at_offset`（`:382-396`）→ `sample_shadow_cubemap_hardware`
/// （`:324-341`）。以及 `bevy_render-0.19.1/src/maths.wgsl:80-87` 的 `orthonormalize`。
///
/// ⚠ 三处**不许化简**：
/// 1. `depth` 走"最大绝对轴"那条推导（`zw = -major × light_custom_data.xy +
///    light_custom_data.zw`）—— 那个 `light_custom_data` 是**宿主**按
///    `perspective_inverse_reverse_rh(π/2, 1, near)` 的 z/w 两轴算出来的
///    （`group0::light_of`），不是这里随手推的。
/// 2. 采样坐标要 `flip_z`：cube 是**左手 y-up**，Bevy 的世界是右手（`shadows.wgsl:17`、
///    `:52-68` 那两处注释）。
/// 3. Gaussian 那 8 个点是 **D3D 的 8×MSAA 位置**配 8 个高斯系数（`shadow_sampling.wgsl:70-102`），
///    基向量是 `orthonormalize(normalize(light_local)) × 0.003 × distance_to_light`
///    —— 三个数一个都不许"看起来差不多"。
///
/// ⚠ 它**自带两格的声明**（binding 2 的 cube array 与 binding 3 的比较采样器）：
/// 内容 shader 只 import 这个符号，而 Bevy 那边这两格是 `mesh_view_bindings` 那份
/// import 顺带带进来的。本宿主没有 naga_oil，所以"顺带"这件事必须写出来 ——
/// 而绑定号仍然只有一处（[`crate::group0::POINT_SHADOW_TEXTURES_BINDING`] /
/// [`crate::group0::POINT_SHADOW_SAMPLER_BINDING`]），由 `group0` 那条反射判据钉住。
///
/// ⚠ 用到的两个符号（`clustered_lights` 与 `light_id` 的下标语义）来自**别的 import**：
/// `surface.wgsl` 引了 `clustered_lights`，所以这里直接用；谁哪天写一支只引
/// `fetch_point_shadow` 的 shader，组装会当场报"找不到 `clustered_lights`"——
/// 那正是我们要的失败方式（同 [`DEPTH_NDC_TO_VIEW_Z`] 那条判据）。
pub const POINT_SHADOW_STUB: &str = "\
@group(0) @binding(2) var point_shadow_textures: texture_depth_cube_array;\n\
@group(0) @binding(3) var point_shadow_textures_comparison_sampler: sampler_comparison;\n\
\n\
// `bevy_render::maths::copysign`（`maths.wgsl:66-68`）：把 b 的符号位抄到 a 上。\n\
//\n\
// ⚠ 它**不是内建** —— 是 Bevy 自己定义的一个函数（正因为 naga 那条链上 `copysign`\n\
// 不是人人都有；本宿主的 naga 29.0.4 的 WGSL 前端里也没有它，`parse/conv.rs` 的\n\
// `map_standard_fun` 那张表里查不到）。Bevy 那句注释写着为什么非它不可：\n\
// `copysign allows proper handling of negative zero to match the rust implementation of\n\
// orthonormalize` —— `-0.0` 上它给 -1.0，而 `select(1.0, -1.0, z < 0.0)` 给 1.0，\n\
// 那是**两个数**，而这两个数会让基向量翻个方向。照抄，一个字都不改。\n\
fn copysign(a: f32, b: f32) -> f32 {\n\
\x20   return bitcast<f32>((bitcast<u32>(a) & 0x7FFFFFFF) | (bitcast<u32>(b) & 0x80000000));\n\
}\n\
\n\
// `bevy_render::maths::orthonormalize`（`maths.wgsl:75-87`）：把一个方向铺成一组正交基。\n\
fn orthonormalize(z_basis: vec3<f32>) -> mat3x3<f32> {\n\
\x20   let sign = copysign(1.0, z_basis.z);\n\
\x20   let a = -1.0 / (sign + z_basis.z);\n\
\x20   let b = z_basis.x * z_basis.y * a;\n\
\x20   let x_basis = vec3<f32>(1.0 + sign * z_basis.x * z_basis.x * a, sign * b, -sign * z_basis.x);\n\
\x20   let y_basis = vec3<f32>(b, sign + z_basis.y * z_basis.y * a, -z_basis.y);\n\
\x20   return mat3x3<f32>(x_basis, y_basis, z_basis);\n\
}\n\
\n\
const PX_POINT_SHADOW_SCALE: f32 = 0.003;\n\
\n\
// D3D 那 8 个 MSAA 位置与对应的高斯系数（`shadow_sampling.wgsl:79-102`）。\n\
const PX_D3D_SAMPLE_POINT_POSITIONS: array<vec2<f32>, 8> = array<vec2<f32>, 8>(\n\
\x20   vec2<f32>( 0.125, -0.375),\n\
\x20   vec2<f32>(-0.125,  0.375),\n\
\x20   vec2<f32>( 0.625,  0.125),\n\
\x20   vec2<f32>(-0.375, -0.625),\n\
\x20   vec2<f32>(-0.625,  0.625),\n\
\x20   vec2<f32>(-0.875, -0.125),\n\
\x20   vec2<f32>( 0.375,  0.875),\n\
\x20   vec2<f32>( 0.875, -0.875),\n\
);\n\
const PX_D3D_SAMPLE_POINT_COEFFS: array<f32, 8> = array<f32, 8>(\n\
\x20   0.157112, 0.157112, 0.138651, 0.130251, 0.114946, 0.114946, 0.107982, 0.079001,\n\
);\n\
\n\
fn px_sample_shadow_cubemap_at_offset(\n\
\x20   position: vec2<f32>,\n\
\x20   coeff: f32,\n\
\x20   x_basis: vec3<f32>,\n\
\x20   y_basis: vec3<f32>,\n\
\x20   light_local: vec3<f32>,\n\
\x20   depth: f32,\n\
\x20   light_id: u32,\n\
) -> f32 {\n\
\x20   return textureSampleCompareLevel(\n\
\x20       point_shadow_textures,\n\
\x20       point_shadow_textures_comparison_sampler,\n\
\x20       light_local + position.x * x_basis + position.y * y_basis,\n\
\x20       i32(light_id),\n\
\x20       depth,\n\
\x20   ) * coeff;\n\
}\n\
\n\
fn fetch_point_shadow(\n\
\x20   light_id: u32,\n\
\x20   frag_position: vec4<f32>,\n\
\x20   surface_normal: vec3<f32>,\n\
\x20   frag_coord_xy: vec2<f32>,\n\
) -> f32 {\n\
\x20   let light = &clustered_lights.data[light_id];\n\
\x20   let surface_to_light = (*light).position_radius.xyz - frag_position.xyz;\n\
\x20   let surface_to_light_abs = abs(surface_to_light);\n\
\x20   let distance_to_light = max(\n\
\x20       surface_to_light_abs.x,\n\
\x20       max(surface_to_light_abs.y, surface_to_light_abs.z),\n\
\x20   );\n\
\x20   let normal_offset = (*light).shadow_normal_bias * distance_to_light * surface_normal.xyz;\n\
\x20   let depth_offset = (*light).shadow_depth_bias * normalize(surface_to_light.xyz);\n\
\x20   let offset_position = frag_position.xyz + normal_offset + depth_offset;\n\
\x20   let frag_ls = offset_position.xyz - (*light).position_radius.xyz;\n\
\x20   let abs_position_ls = abs(frag_ls);\n\
\x20   let major_axis_magnitude = max(\n\
\x20       abs_position_ls.x,\n\
\x20       max(abs_position_ls.y, abs_position_ls.z),\n\
\x20   );\n\
\x20   let zw = -major_axis_magnitude * (*light).light_custom_data.xy\n\
\x20       + (*light).light_custom_data.zw;\n\
\x20   let depth = zw.x / zw.y;\n\
\x20   let light_local = frag_ls * vec3<f32>(1.0, 1.0, -1.0);\n\
\x20   let basis = orthonormalize(normalize(light_local))\n\
\x20       * PX_POINT_SHADOW_SCALE * distance_to_light;\n\
\x20   var sum: f32 = 0.0;\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[0], PX_D3D_SAMPLE_POINT_COEFFS[0],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[1], PX_D3D_SAMPLE_POINT_COEFFS[1],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[2], PX_D3D_SAMPLE_POINT_COEFFS[2],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[3], PX_D3D_SAMPLE_POINT_COEFFS[3],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[4], PX_D3D_SAMPLE_POINT_COEFFS[4],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[5], PX_D3D_SAMPLE_POINT_COEFFS[5],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[6], PX_D3D_SAMPLE_POINT_COEFFS[6],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   sum += px_sample_shadow_cubemap_at_offset(\n\
\x20       PX_D3D_SAMPLE_POINT_POSITIONS[7], PX_D3D_SAMPLE_POINT_COEFFS[7],\n\
\x20       basis[0], basis[1], light_local, depth, light_id);\n\
\x20   return sum;\n\
}\n";

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
        "bevy_pbr::mesh_view_bindings::view" => Some(VIEW_STUB),
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
