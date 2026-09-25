#define_import_path planet_x::light

// 「场景里的那盏太阳」。**不许再写死方向**：太阳是哪种灯由场景说了算，
// 这里只负责把 Bevy 的光源数据翻成着色要用的那一份。
//
// 太阳 = 场景里那盏**点光源**（`spawn_lights` 摆的）：它住在聚类缓冲 `clustered_lights.data` 里
// （`Lights` 那份 uniform 只有方向光）。
// ⚠ 但**只取第 0 格，不走 `view_fragment_cluster_index`**：那条路按格子/z 切片取灯，
//    相机拉远时大气的采样点会有一半"查不到灯"，画面上留下一条硬台阶）。
// ⚠ **没有方向光兜底**：宇宙里没有平行光，没有点光源就是"这一帧没有光"（颜色 0 ⇒ 全黑）。
//    多光源以后再说。

#import bevy_pbr::mesh_view_bindings::clustered_lights
#import bevy_pbr::mesh_view_bindings::lights
#import bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
// ⚠ `#import` 是**按行**解析的（离线门那份如此；Bevy 的 naga_oil 两种都吃），花括号里那几个
//    符号写在同一行 —— 折行的话后面那几行会被离线门当成源码，报"expected global item"。
// ⚠ 模块名是 `clustered_forward`（文件 `clustered_forward.wgsl`），**不是** `clustering`：
//    写错的话离线门照样过（它按我写的字符串给桩），运行期那条管线会永远停在
//    "import 还没到"⇒ 出图等管线超时、画面只有星空。
//    —— 这份 shader 现在**不再引它**了（不查聚类），那两条坑留给以后要用的人。

struct SunLight {
    /// 从着色点指向光源（单位向量）。点光源是现算的 ⇒ **每个着色点都不一样**。
    direction: vec3<f32>,
    /// 光源在世界里的位置。
    position: vec3<f32>,
    /// **已经乘过强度与距离衰减**的颜色：`color × 强度 × 1/d² × range 衰减`。
    /// 没有光时是 0（⇒ 着色结果全黑，不再退回任何"平行光"）。
    color: vec3<f32>,
    /// 1 = 点光源（影子查 cube），0 = **没有光**（`color` 也是 0）。
    point: u32,
    /// 这盏灯的 shadow map 开着没有（0 = 没开 ⇒ 连采样都不发）。
    shadow_maps: u32,
    /// 影子那张图的 id：目前恒为 0（只摆一盏灯；多光源时这里要改成真 id）。
    shadow_id: u32,
};

/// 距离衰减：逐字抄 `bevy_pbr::lighting::getDistanceAttenuation`（含 range 的平滑落零）。
/// 抄而不是引，是因为它只有三行 —— 引进来要多一个 `bevy_pbr::lighting` 的桩，
/// 而那一份抄错了会立刻在"受光面亮度对不对"上露出来（判据就是量它）。
fn range_attenuation(distance_squared: f32, inverse_range_squared: f32) -> f32 {
    let factor = distance_squared * inverse_range_squared;
    // ⚠ 变量别叫 `smooth` —— 和 `cast` 一样是 WGSL 保留字（`smoothstep` 的前缀不算数）。
    let falloff = clamp(1.0 - factor * factor, 0.0, 1.0);
    return falloff * falloff / max(distance_squared, 0.0001);
}

/// **场景里有几盏点光源**（2026-09-20 多光源：这一格加在 `Lights` uniform 里）。
///
/// ⚠ 为什么需要它：`clustered_lights.data` 是**定长**数组（长度反射自 shader），
///   着色器没有"写到第几格"的记号（约定是"判颜色非零"）—— 逐灯求和要靠这一格才知道边界。
fn light_count() -> u32 {
    return lights.n_point_lights.x;
}

/// 取**第 `index` 盏**点光源（`sun_light` 就是第 0 盏）。
///
/// ⚠ **不问 cluster**（实测）：`view_fragment_cluster_index` 那条路按片元所在的格子取灯，
///    而"我们的采样点落在哪个格子/哪一层 z 切片"是 Bevy 聚类网格的内部细节 —— 相机拉到远距离
///    （viewer 里 `--place …,14`）时，大气沿 chord 的 5 个采样点**有一半查不到灯**，
///    于是画面上沿网格边界出现一条**硬台阶**（左右两半的失败率 4% 对 57%）。
///    这个渲染器摆的灯是**配方里那几盏**（通常就一盏太阳）⇒ 直接按序取，既省一整套格子计算、
///    又不受视口影响。
/// ⚠ **没有兜底**（用户口径）：宇宙里没有平行光，没被写过的格子（uniform 数组默认值 = 全 0）
///    就是"这一盏不存在" —— 颜色 0、`point = 0`，它那一份贡献是 0。
/// ⚠ `frag_coord` 只为不动调用点而留着（取灯不再需要片元坐标）。
fn point_light(index: u32, point: vec3<f32>) -> SunLight {
    let data = &clustered_lights.data[index];
    // ⚠ 判"这格写没写过"**只能看颜色**：`position_radius.w` 不是 range（实测对这盏灯读到 0），
    //    拿它当阈值会把有点光源的场景也判成"没光"。
    let lit = !all((*data).color_inverse_square_range.rgb == vec3<f32>(0.0));

    let offset = (*data).position_radius.xyz - point;
    let distance_squared = dot(offset, offset);

    var light: SunLight;
    // 没有光时给一个**定值方向**：`normalize(0 - point)` 在 `point` 恰好为 0 时是 NaN。
    light.direction = select(vec3<f32>(0.0, 0.0, 1.0), normalize(offset), lit);
    light.position = (*data).position_radius.xyz;
    light.color = select(
        vec3<f32>(0.0),
        (*data).color_inverse_square_range.rgb
            * range_attenuation(distance_squared, (*data).color_inverse_square_range.w),
        lit,
    );
    light.point = select(0u, 1u, lit);
    light.shadow_maps = select(
        0u,
        1u,
        lit && ((*data).flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u,
    );
    // ⚠ 影子那张 cube 的层号 = **这一盏灯的序号**（`group0` 按灯序 × 6 面摆 pass，
    //    `px_render::render` 里 `light * SHADOW_CUBE_FACES + face`）⇒ 索引就是 id。
    light.shadow_id = index;
    return light;
}

/// 取"主光"：**第 0 盏**（配方里第一盏，通常就是那颗太阳）。
///
/// ⚠ 单光源场景里它与从前逐位相同；多光源场景里"主光"只用于那些**按太阳定义**的项
///   （边缘散射、包裹漫反射的方向、云影的太阳方向），**漫反射那一支要走逐灯求和**
///   （`surface.wgsl` 里的循环就是这件事）。
fn sun_light(point: vec3<f32>, frag_coord: vec2<f32>) -> SunLight {
    return point_light(0u, point);
}
