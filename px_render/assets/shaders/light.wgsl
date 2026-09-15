#define_import_path planet_x::light

// 「场景里的那盏太阳」。**不许再写死方向**（§60）：太阳是哪种灯由场景说了算，
// 这里只负责把 Bevy 的光源数据翻成着色要用的那一份。
//
// 为什么点光源要看 `clustered_lights`：`Lights` 那份 uniform 里**只有方向光**（`directional_lights`），
// 点光源住在聚类缓冲 `clustered_lights.data` 里。
// ⚠ 但**只在那一格里直接取第 0 个**，不走 `view_fragment_cluster_index` 那三跳（§64：那条路在远距离
//    上会让大气的采样点查不到灯、退回兜底，画面上留下一条硬台阶）。

#import bevy_pbr::mesh_view_bindings::{view, lights, clustered_lights}
#import bevy_pbr::mesh_view_types::POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT
// ⚠ `#import` 是**按行**解析的（离线门那份如此；Bevy 的 naga_oil 两种都吃），花括号里那几个
//    符号写在同一行 —— 折行的话后面那几行会被离线门当成源码，报"expected global item"。
// ⚠ 模块名是 `clustered_forward`（文件 `clustered_forward.wgsl`），**不是** `clustering`：
//    写错的话离线门照样过（它按我写的字符串给桩），运行期那条管线会永远停在
//    "import 还没到"⇒ 出图等管线超时、画面只有星空（§60.3 实测踩过）。
#import bevy_pbr::clustered_forward::{get_clusterable_object_id, unpack_clusterable_object_index_ranges, view_fragment_cluster_index}

struct SunLight {
    /// 从着色点指向光源（单位向量）。点光源是现算的 ⇒ **每个着色点都不一样**。
    direction: vec3<f32>,
    /// 光源在世界里的位置。方向光给 `direction * FAR_LIGHT`，"往光源走"两种光同一个公式。
    position: vec3<f32>,
    /// **已经乘过强度与距离衰减**的颜色：点光源 = `color × 强度 × 1/d² × range 衰减`，
    /// 方向光 = `lights.directional_lights[0].color`（照度）。
    color: vec3<f32>,
    /// 1 = 点光源（影子查 cube），0 = 方向光（影子查级联）。
    point: u32,
    /// 这盏灯的 shadow map 开着没有（0 = 没开 ⇒ 连采样都不发）。
    shadow_maps: u32,
    /// 影子那张图的 id：点光源 = clusterable object id，方向光 = 灯的下标。
    shadow_id: u32,
};

/// 方向光在这个模型里就是"放在无穷远处的点光源"。取 1e5 是因为它远到让 1/d² 可以忽略，
/// 又远不到让 f32 掉精度。
const FAR_LIGHT: f32 = 1.0e5;

/// 距离衰减：逐字抄 `bevy_pbr::lighting::getDistanceAttenuation`（含 range 的平滑落零）。
/// 抄而不是引，是因为它只有三行 —— 引进来要多一个 `bevy_pbr::lighting` 的桩，
/// 而那一份抄错了会立刻在"受光面亮度对不对"上露出来（§60.2 的判据就是量它）。
fn range_attenuation(distance_squared: f32, inverse_range_squared: f32) -> f32 {
    let factor = distance_squared * inverse_range_squared;
    // ⚠ 变量别叫 `smooth` —— 和 `cast` 一样是 WGSL 保留字（`smoothstep` 的前缀不算数）。
    let falloff = clamp(1.0 - factor * factor, 0.0, 1.0);
    return falloff * falloff / max(distance_squared, 0.0001);
}

/// 视空间 z（级联阴影要用它挑级联）。与 `bevy_pbr::shadows` 里那条算法同一份。
fn view_z_of(point: vec3<f32>) -> f32 {
    return dot(
        vec4<f32>(
            view.view_from_world[0].z,
            view.view_from_world[1].z,
            view.view_from_world[2].z,
            view.view_from_world[3].z,
        ),
        vec4<f32>(point, 1.0),
    );
}

/// 正交相机（Bevy 那边也是这么判的：`pbr_input.is_orthographic`）。
fn is_orthographic() -> bool {
    return view.clip_from_view[3][3] == 0.0;
}

/// 取"主光"：**直接问场景里那盏点光源**（`clustered_lights.data[0]`），照不到才退回第 0 盏方向光。
///
/// 于是"太阳是点光源还是平行光"是**场景**的事，shader 一个常量都不写死。
/// ⚠ **不问 cluster**（§64 实测）：`view_fragment_cluster_index` 那条路按片元所在的格子取灯，
///    而"我们的采样点落在哪个格子/哪一层 z 切片"是 Bevy 聚类网格的内部细节 —— 相机拉到远距离
///    （viewer 里 `--place …,14`）时，大气沿 chord 的 5 个采样点**有一半查不到灯**，退回兜底，
///    于是画面上沿网格边界出现一条**硬台阶**（左右两半的失败率 4% 对 57%）。
///    这个渲染器只摆一盏灯 ⇒"最近的一盏"就是"这一盏"，直接取它既省一整套格子计算、又不受视口影响。
///    真要多灯累加/灯间遮挡，得先把灯自己的数据喂进来（§60.5 记着这条）。
/// ⚠ `frag_coord` 只为不动四个调用点而留着。
fn sun_light(point: vec3<f32>, frag_coord: vec2<f32>) -> SunLight {
    let data = &clustered_lights.data[0];

    var light: SunLight;
    // 第 0 格"没被写过"（uniform 数组的默认值 = 全 0）⇒ 这一帧没有点光源，退回第 0 盏方向光
    // （老场景、探针、只摆了平行光的场景）。
    // ⚠ **不能拿 `position_radius.w` 当"有没有灯"**：那格不是 range（实测对这盏灯读到 0），
    //    拿它判会把有点光源的场景也推进兜底 —— §64.4 那次"整幅受光变了"就是这么来的。
    //    用颜色判"写没写过"：全零点光源本来就贡献 0，退兜底不会丢东西。
    if all((*data).color_inverse_square_range.rgb == vec3<f32>(0.0)) {
        // ⚠ 一盏方向光都没有时 `directional_lights[0]` 是全零，`normalize` 会算出 NaN ⇒
        //   这里按 `n_directional_lights` 选一个安全的方向（朝 +Z，且颜色为 0 ⇒ 画面全黑而不是花屏）。
        let directional = &lights.directional_lights[0];
        let available = lights.n_directional_lights > 0u;
        light.direction = select(
            vec3<f32>(0.0, 0.0, 1.0),
            normalize((*directional).direction_to_light),
            available,
        );
        light.position = light.direction * FAR_LIGHT;
        light.color = select(vec3<f32>(0.0), (*directional).color.rgb, available);
        light.point = 0u;
        light.shadow_maps = select(0u, (*directional).num_cascades, available);
        light.shadow_id = 0u;
        return light;
    }

    let offset = (*data).position_radius.xyz - point;
    let distance_squared = dot(offset, offset);
    light.direction = normalize(offset);
    light.position = (*data).position_radius.xyz;
    light.color = (*data).color_inverse_square_range.rgb
        * range_attenuation(distance_squared, (*data).color_inverse_square_range.w);
    light.point = 1u;
    light.shadow_maps = select(
        0u,
        1u,
        ((*data).flags & POINT_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u,
    );
    light.shadow_id = 0u;
    return light;
}
