#import planet_x::light::sun_light
#import planet_x::noise::rotate_vector
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::{view, lights}
#import bevy_pbr::shadows::fetch_point_shadow

// **气态巨行星的次表面散射材质**（2026-09-20，用户口径：「气态巨星应当用次表面散射」）。
//
// 为什么不是云：云那一套（`clouds.wgsl` 的软档）算的是**一个不透明的壳**里"哪一步先撞到
// 等值面" —— 它的形状来自体积的硬边界（团块、缝隙、卷曲的边）。气态巨行星没有那个边界：
// 它是一颗**整球的气体**，我们看到的每一条带都是"光从这一柱气体里散射回来后"的颜色。
// 拿云的壳去画它，得到的是"一圈圈贴在球上的云带"（实测那版就是：带边像塑料环）。
//
// 所以这一份算的是**光穿过气体再出来**：
//
//   ① 穿透深度 `depth`：这一柱气体有多厚 —— 由条带场（`band_map`，图烘出来的那张）
//      给"这一带多厚"，再乘上视线掠过角（越靠边缘视线穿得越厚）；
//   ② 每通道吸收（Beer–Lambert）：`exp(-σ·depth)`，蓝光吸得比红光多 ⇒ 深带偏暖偏暗、
//      亮带偏白 —— 这是"次表面"最认得出的那一笔（也是土星那条奶黄/棕的分界从哪来）；
//   ③ 包裹漫反射：气体是半透的 ⇒ 照亮的那一侧**越过几何晨昏线**（`wrap`），
//      于是明暗交界是一条很宽的软带，而不是一条刀切的黑边；
//   ④ 边缘散射：视线与球面相切那圈穿过的气体最厚 ⇒ 边缘自己发亮（limb glow）。
//
// ⚠ 条带场是**图烘出来的产物**（`gasgiant` 图 → `field_cube` → 立方贴图），烘在**未倾斜的
//   局部系**里：查它之前要把世界方向按 `orientation` 转回去（与覆盖度立方图同一口径）。
struct GasParams {
    /// 行星的世界朝向（`SYSTEM_TILT × spin`）。
    orientation: vec4<f32>,
    /// 气体本体的基色（亮带那一档的颜色）。
    albedo: vec4<f32>,
    /// **每通道吸收系数**（线性 RGB）。越大越暗、越偏它自己的颜色：
    /// 典型给法 = 红最小、蓝最大 ⇒ 厚的地方偏奶黄/棕。
    absorption: vec4<f32>,
    /// 包裹量（translucency）：`0` = 严格 Lambert（硬晨昏线），越大越过晨昏线越多。
    wrap: f32,
    /// 光学厚度的整体尺度（1 = 常规气态巨行星；越小越像被照亮的球）。
    thickness: f32,
    /// 边缘散射增益（limb glow）。
    limb: f32,
    /// 条带对厚度的调制强度（`0` = 看不见带，`1` = 全强度）。
    band_gain: f32,
    /// 太阳项的整体增益（对齐曝光后的亮度）。
    sun_gain: f32,
    /// 环境光增益（暗面唯一的光源）。
    ambient: f32,
    /// 条带的对比（把条带场往 0/1 推多少；`1` = 原样）。
    band_contrast: f32,
    /// **半球厚度差**：光学厚度乘 `1 + hemi_depth · 纬度`（纬度在行星自己的系里）。
    /// ⚠ 参考图上"一半奶黄、一半青白"这件事在这个模型里就是**一边的气柱更厚**：
    ///   厚 ⇒ 蓝光被吃得多 ⇒ 偏暖偏暗；薄 ⇒ 偏白偏亮。（别去动 `albedo` —— 那是全球的。）
    hemi_depth: f32,
    /// **高纬霾**（冷色散射层）：`|纬度|` 越大越强，把一层淡蓝散射进视线。
    /// 参考图（土星的南半球、天王星的极区）那种"发青"的来源 —— 它与吸收是两件事：
    /// 吸收让颜色偏暖，霾往回收一点蓝。
    haze: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: GasParams;
// 格子与 `TEXTURE_SLOTS` 的表一致：1/3 是 2D（这一份不用），**5/7 是 cube**。
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var band_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var band_sampler: sampler;

const PI: f32 = 3.141592653589793;

/// 把世界方向转回条带图那个**未倾斜的局部系**（条带图烘在未倾斜的系里；纬度也按这个系算）。
fn to_local(direction: vec3<f32>) -> vec3<f32> {
    return rotate_vector(vec4<f32>(-params.orientation.xyz, params.orientation.w), direction);
}

/// 在局部系里采一次条带场。
fn band_of(local: vec3<f32>) -> f32 {
    return textureSampleLevel(band_map, band_sampler, local, 0.0).r;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // ⚠ **用几何球面的法线，不用网格法线**：气态巨行星的可见面是**光球层**（一团气体的
    //   等密度面），它就是一个球 —— 而网格法线带着几何的位移（这一颗本体今天是内建球，
    //   但就算挂上别的高细分网格，这条也不该变）。用网格法线的话，任何起伏都会从
    //   条带采样与 N·L 里透出来（实测：挂 `planet` 图那张位移网格时，画面上会出现
    //   几块"海岸线"——那其实是那张图的陆地）。
    //   ⚠ 行星摆在世界原点（变换只有旋转）⇒ `normalize(world_position)` 就是球面法线。
    let normal = normalize(in.world_position.xyz);
    let camera = view.world_position.xyz;
    let view_vector = normalize(camera - in.world_position.xyz);
    let ndotv = max(dot(normal, view_vector), 0.0001);

    // 光源由场景那盏灯说了算（与 surface.wgsl 同一条：宇宙里没有平行光兜底）。
    let principal = sun_light(in.world_position.xyz, in.position.xy);
    let sun = principal.direction;
    let ndotl = dot(normal, sun);

    // ---- ① 这一柱气体有多厚 ----------------------------------------------------
    // 条带场：0 = 深带（浓、光走得深）、1 = 亮带（高云、光很快散射回来）。
    // ⚠ `band_contrast` 把场往两端推：图那边给的是**软**的带（低对比、宽过渡），
    //   这里按这一颗行星的观感再收一次，而不是回去改图（改图要重烘）。
    // 行星自己的系：条带查表与"纬度"都在这里算（不然后者的纬度是世界的、会跟着自转跑）。
    let local = to_local(normal);
    let latitude = clamp(local.y, -1.0, 1.0);
    var band = clamp(band_of(local), 0.0, 1.0);
    band = clamp((band - 0.5) * params.band_contrast + 0.5, 0.0, 1.0);
    // 视线越斜，穿过的那一柱越长（`grazing`：正对 = 0、边缘 = 1）。
    let grazing = 1.0 - abs(ndotv);
    // 深度 = 厚度尺度 × 带的浓淡 × 掠射加长。
    let depth = params.thickness
        * (1.0 - params.band_gain * (band - 0.5) * 2.0)
        * (1.0 + params.limb * grazing)
        * (1.0 + params.hemi_depth * latitude);

    // ---- ② 每通道吸收：气体自己把光吃掉一部分，吃多少随波长 ----
    let transmittance = exp(-max(params.absorption.rgb, vec3<f32>(0.0)) * max(depth, 0.0));
    let gas = params.albedo.rgb * transmittance;

    // ---- ③ 包裹漫反射：半透 ⇒ 照亮的一侧越过晨昏线 ----
    // ① 别人的影（shadow map）：环挡住的太阳。⚠ 只挡**直射**那一支；天光（ambient）不该被挡。
    //    灯没开影子时（`shadow_maps == 0`）连采样都不发（与 `surface.wgsl` 同一条口径）。
    var cast_shadow = 1.0;
    if ndotl + params.wrap > 0.0 && principal.shadow_maps != 0u {
        cast_shadow = fetch_point_shadow(
            principal.shadow_id,
            in.world_position,
            normal,
            in.position.xy,
        );
    }
    let wrapped = clamp((ndotl + params.wrap) / (1.0 + params.wrap), 0.0, 1.0);
    let direct = gas * (principal.color * wrapped * (1.0 / PI) * params.sun_gain) * cast_shadow;

    // ---- ④ 边缘散射：切向那一圈穿过的气体最厚，自己发亮 ----
    // ⚠ 只加在**受光的那半边**（`ndotl + wrap·0.5 > 0`）：背光的边缘不该发光，
    //   否则整颗球会像一圈霓虹灯。
    let lit = clamp((ndotl + params.wrap * 0.5) * 4.0, 0.0, 1.0);
    let rim = gas * (principal.color * params.limb * pow(grazing, 4.0) * lit) * cast_shadow;

    // ---- ⑤ 高纬霾：一层偏蓝的散射（`|纬度|` 越大越强）----
    // ⚠ 它是**散射**不是吸收：吸收只会把颜色往暖里带（蓝被吃掉），霾反过来补一点蓝。
    //   参考图上土星南半球那一片青灰、天王星整颗的淡青，都要靠这一笔（或 `absorption` 整体换档）。
    let polar = smoothstep(0.28, 1.0, abs(latitude));
    let haze = params.haze * polar * gas * (principal.color * wrapped * (1.0 / PI)) * vec3<f32>(0.62, 0.78, 1.0);

    // ---- 环境光：暗面那一半只有天光（与 surface.wgsl 同一条口径：不乘任何遮挡） ----
    let ambient = gas * (lights.ambient_color.rgb * params.ambient);

    return vec4<f32>(view.exposure * (direct + rim + haze + ambient), 1.0);
}
