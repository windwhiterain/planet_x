#import planet_x::common::shell_thickness
#import planet_x::light::{sun_light, view_z_of}
#import planet_x::noise::{fbm_3, fbm_3_grad, rotate_vector, NoiseSample}
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::{view, depth_prepass_texture, globals}
#import bevy_pbr::view_transformations::depth_ndc_to_view_z
#import bevy_pbr::shadows::{fetch_directional_shadow, fetch_point_shadow}

struct CloudParams {
    orientation: vec4<f32>,
    tint: vec4<f32>,
    inner: f32,
    outer: f32,
    density: f32,
    coverage: f32,
    base: f32,
    top: f32,
    detail_scale: f32,
    detail_strength: f32,
    erode: f32,
    phase: f32,
    shadow: f32,
    steps: u32,
    bump: f32,
    seed: u32,
    ablate: u32,
    slope_scale: f32,
    taper: f32,
    coverage_gain: f32,
    surface_level: f32,
    bound: u32,
    gradient: u32,
    /// 细节风：两层各一份**幅度**（方向单位，0 = 不动）。两层的时间尺度写在 shader 里
    /// 且故意不同 ⇒ 细的那层在粗的那层上滑动。§61
    wind: f32,
    wind_skin: f32,
};

const ABLATE_NONE: u32 = 0u;
const ABLATE_SUN: u32 = 1u;
const ABLATE_NOISE: u32 = 2u;
const ABLATE_FETCH: u32 = 3u;
const ABLATE_DETAIL: u32 = 4u;
const ABLATE_SURFACE: u32 = 5u;
const ABLATE_NORMALS: u32 = 6u;
const SHADOW_GAIN: f32 = 4.0;
/// 体积那条路的档位：场景参数 `gradient = 2` 选**软云档** —— 步进的**每一步**都拿这一步
/// 自己的法线算这一步的受光，再按前向透射率积分。别的取值都是老路（老路一个像素都不许变，
/// 所以开关只能走这个从没被体积路读过的参数）。
///
/// 旧版是"累积光深到 `SOFT_TAU` 才算法线、算一次"：那等于拿一个**累积量**开关一个逐像素属性，
/// 掠射的轮廓射线永远到不了阈值 ⇒ 整条射线回退成全亮 ⇒ 轮廓上一圈银边（§51.16）。
/// 现在没有这个开关了：只有每一步的受光，回退分支从构造上不存在。
const SOFT_GRADIENT: u32 = 2u;
/// 软档的密度凹重映射宽度：等值面**高度以上**这层宽度里把密度从 0 爬到 1。
/// `smoothstep` 在等值面上斜率为 0 ⇒ 边缘是最软的爬升；一个宽度内就饱和到 1 ⇒ 体内是实心团块。
/// 这正是"软而高不透明"的来源。等值面高度本身取 `surface_level`（硬表面那条路的同一个阈值），
/// 所以软档与硬表面是**同一个 cloud surface**：一个软爬升、一个硬切。
const SOFT_EDGE: f32 = 0.35;

struct Medium {
    direction: vec3<f32>,
    altitude: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: CloudParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var coverage_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var coverage_sampler: sampler;

fn to_local(point: vec3<f32>) -> vec3<f32> {
    return rotate_vector(vec4<f32>(-params.orientation.xyz, params.orientation.w), point);
}

fn to_world(vector: vec3<f32>) -> vec3<f32> {
    return rotate_vector(params.orientation, vector);
}

fn span() -> f32 {
    return max(params.outer - params.inner, 1e-5);
}

fn medium_of(point: vec3<f32>) -> Medium {
    let local = to_local(point);
    let radius = length(local);
    return Medium(local / max(radius, 1e-5), (radius - params.inner) / span());
}

fn coverage_of(direction: vec3<f32>) -> f32 {
    if params.ablate == ABLATE_FETCH {
        return 0.45;
    }
    let mask = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0).r;
    return smoothstep(
        0.0,
        0.45,
        clamp((mask - params.coverage) / max(1.0 - params.coverage, 1e-4), 0.0, 1.0),
    );
}

/// 细节噪声的凹重映射。原场的值压在低位、峰很窄 ⇒ 阈值以上只剩零星峰，离散步长会整个
/// 跨过去（硬表面的"擦面而过"）。开平方根把低位抬起来、峰变宽 ⇒ 阈值以上的覆盖变实。
/// `1.0` 是它的不动点 ⇒ `shape_of(cover, altitude, 1.0)` 那条上界一字不变（见 `cloud_field`）。
fn detail_curve(value: f32) -> f32 {
    return sqrt(value);
}

/// `detail_curve` 的斜率，解析梯度走链式法则时乘上去（`d sqrt(b)/db`）。
/// 表面上场 > τ ⇒ `sqrt(b) > τ / coverage_gain` ⇒ `b` 有正下界，这里没有奇点；
/// 底下那个 `max` 只是别让 0 附近的调用给出 inf。
fn detail_curve_slope(value: f32) -> f32 {
    return 0.5 / max(sqrt(value), 1e-4);
}

/// 细节风（§61）：两层细节的采样点各自搬一份，速度不同 ⇒ 粗/细细节互相搓动。
///
/// **时间从哪来**：Bevy 的 shader 内时钟 `globals.time`（开机以来的秒数），
/// 由渲染侧每帧自己写进 uniform —— **不经逻辑帧注入**（P31 把"按逻辑帧推进的自转"删掉，
/// 就是因为它同时毁掉可复现性与帧间一致性；这里要动的是着色细节，不是场景状态）。
///
/// 为什么用 `var<private>` 而不是把它们做成函数参数：`billows` 有 4 个调用者，
/// 其中一个是**探针的 compute 入点**，而探针的 bind group 0/1 是空的（`None`）
/// ⇒ 让 `billows` 自己去读 `globals` 会把探针打挂。私有变量按 invocation 一份，
/// 片段入点开头 `arm_wind()` 一次，探针不碰它 ⇒ 探针看到的风恒为 0（＝不动），
/// 于是"CPU 场 vs GPU 场"那条判据不受这个特性影响。
var<private> wind_tower: vec3<f32> = vec3<f32>(0.0);
var<private> wind_skin: vec3<f32> = vec3<f32>(0.0);

/// 风的方向（局部系，与覆盖度立方图同一个坐标系）。斜着推是为了让细节不沿经纬线滑动。
const WIND_AXIS: vec3<f32> = vec3<f32>(0.7746, 0.2582, -0.5774);

/// 两层各自的时间尺度（弧度/秒）。**故意不同**：同速就是整块平移，不同速才有"搓动"
/// （细的那层在粗的那层上滑过去）。这两个是**节奏**不是内容，所以写死在 shader 里；
/// 场景给的是幅度（`wind` / `wind_skin`）。
const WIND_RATE_TOWER: f32 = 0.13;
const WIND_RATE_SKIN: f32 = 0.37;

/// 片段入点的第一句：把这一刻的风偏置算好。`params.wind == 0` 时是零向量
/// ⇒ 与加这个特性之前逐位相同（缺省就是 0，见 §61.2）。
///
/// ⚠ 偏置是**有界的正弦**，不是"速度 × 时间"的线性漂移：线性漂移下采样点会一直往外走，
/// 挂上半小时之后常数项就盖过方向项 ⇒ 整颗球的细节被抹成一片（而且再也回不来）。
/// 正弦还有个好处：`t = 0` 时偏置为 0 ⇒ 第一帧与静态那张图逐位相同。
/// 两个正弦的周期不同（约 48 s 与 17 s）⇒ 合成起来不显周期。
fn arm_wind() {
    let clock = globals.time;
    wind_tower = WIND_AXIS * (params.wind * sin(WIND_RATE_TOWER * clock));
    wind_skin = WIND_AXIS * (params.wind_skin * sin(WIND_RATE_SKIN * clock));
}

fn billows(direction: vec3<f32>, altitude: f32, with_skin: bool) -> f32 {
    if params.ablate == ABLATE_NOISE {
        // 消融档把噪声钉成一个常数：跟着重映射走，否则"关噪声"那一档会顺带把场压低一截。
        return detail_curve(0.55);
    }
    let tower = sampled_noise(
        direction + wind_tower,
        altitude,
        params.detail_scale * 0.35,
        3u,
        params.seed,
    );
    if !with_skin {
        return detail_curve(clamp(tower, 0.0, 1.0));
    }
    let skin = sampled_noise(
        direction + wind_skin,
        altitude,
        params.detail_scale * 1.70,
        2u,
        params.seed ^ 31u,
    );
    return detail_curve(clamp(tower * 0.62 + skin * 0.38, 0.0, 1.0));
}

fn sampled_noise(
    direction: vec3<f32>,
    altitude: f32,
    across: f32,
    octaves: u32,
    seed: u32,
) -> f32 {
    let along = across * span();
    return fbm_3(direction * (across + altitude * along), 1.0, octaves, 2.0, 0.5, seed);
}

fn shape_of(cover: f32, altitude: f32, noise: f32) -> f32 {
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let bias = footprint + noise - 1.0;
    let lobed = clamp(bias * params.coverage_gain, 0.0, 1.0);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, altitude);
    let shape = clamp(floor_here * under_top * lobed, 0.0, 1.0);
    return clamp((shape - params.erode) / max(1.0 - params.erode, 1e-4), 0.0, 1.0);
}

fn density_of(medium: Medium, cover: f32, with_skin: bool) -> f32 {
    return shape_of(cover, medium.altitude, billows(medium.direction, medium.altitude, with_skin));
}

fn cloud_field(point: vec3<f32>) -> f32 {
    let medium = medium_of(point);
    if medium.altitude < 0.0 || medium.altitude > 1.0 {
        return 0.0;
    }
    let cover = coverage_of(medium.direction);
    if cover <= 0.0 {
        return 0.0;
    }
    // 保守上界早退：`shape_of` 对 noise 单调非降，而 billows 夹在 [0,1] ⇒
    // `shape_of(cover, altitude, 1.0)` 就是精确上界。上界都不够阈值 ⇒ 硬表面那几步
    // 永远打不中这里，返回 0.0 与算出来的值逐位一致（体积那条路不读 cloud_field）。
    if params.bound != 0u && shape_of(cover, medium.altitude, 1.0) <= params.surface_level {
        return 0.0;
    }
    return density_of(medium, cover, true);
}

fn gate_open(value: f32) -> f32 {
    return select(0.0, 1.0, value > 0.0 && value < 1.0);
}

fn slope_of_smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let width = max(high - low, 1e-6);
    let t = clamp((value - low) / width, 0.0, 1.0);
    return select(0.0, 6.0 * t * (1.0 - t) / width, t > 0.0 && t < 1.0);
}

fn project_tangential(vector: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    return vector - dot(direction, vector) * direction;
}

struct ShapePartials {
    cover: f32,
    altitude: f32,
    noise: f32,
};

fn shape_of_partials(cover: f32, altitude: f32, noise: f32) -> ShapePartials {
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let bias = footprint + noise - 1.0;
    let lobed = clamp(bias * params.coverage_gain, 0.0, 1.0);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, altitude);
    let raw = floor_here * under_top * lobed;
    let shape = clamp(raw, 0.0, 1.0);
    let erode_room = max(1.0 - params.erode, 1e-4);
    let live = gate_open(shape);
    let footprint_live = select(0.0, 1.0, footprint > 0.0);
    let lobe_live = gate_open(lobed);
    let ceiling_live = select(
        0.0,
        params.top * params.detail_strength,
        ceiling > params.base + 0.02,
    );

    let tall = floor_here * under_top * params.coverage_gain * footprint_live;
    let cover_partial = live * lobe_live * tall / erode_room;
    let shape_altitude = select(
        0.0,
        under_top * lobed * slope_of_smoothstep(0.0, max(params.base, 1e-3), altitude),
        altitude > 0.0 && altitude < max(params.base, 1e-3),
    ) - floor_here * lobed * slope_of_smoothstep(ceiling, ceiling + 0.20, altitude)
        - floor_here * under_top * params.coverage_gain * params.taper * 2.0 * height
            * footprint_live * lobe_live;
    let altitude_partial = live * shape_altitude / erode_room;
    let shape_noise = floor_here
        * (
            under_top * params.coverage_gain * lobe_live
                + lobed * ceiling_live * slope_of_smoothstep(ceiling, ceiling + 0.20, altitude)
        );
    let noise_partial = live * shape_noise / erode_room;

    return ShapePartials(
        select(0.0, cover_partial, footprint_live > 0.0),
        altitude_partial,
        noise_partial,
    );
}

/// 软档每一步要的两个形状偏导：∂shape/∂altitude 与 ∂shape/∂cover。
/// 只搬 `shape_of_partials` 里的这两项、其余逐字同源 —— 硬表面那条路仍然用它自己那份，一行不动。
/// **噪声那一项不要**：它要的是 5 八度噪声的梯度（`billows_along`），那才是每步都算会付不起的东西。
fn soft_shape_axes(cover: f32, altitude: f32, noise: f32) -> vec2<f32> {
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let lobed = clamp((footprint + noise - 1.0) * params.coverage_gain, 0.0, 1.0);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, altitude);
    let shape = clamp(floor_here * under_top * lobed, 0.0, 1.0);
    let erode_room = max(1.0 - params.erode, 1e-4);
    let live = gate_open(shape);
    let footprint_live = select(0.0, 1.0, footprint > 0.0);
    let lobe_live = gate_open(lobed);
    let cover_partial = live * lobe_live * floor_here * under_top * params.coverage_gain
        * footprint_live / erode_room;
    let shape_altitude = select(
        0.0,
        under_top * lobed * slope_of_smoothstep(0.0, max(params.base, 1e-3), altitude),
        altitude > 0.0 && altitude < max(params.base, 1e-3),
    ) - floor_here * lobed * slope_of_smoothstep(ceiling, ceiling + 0.20, altitude)
        - floor_here * under_top * params.coverage_gain * params.taper * 2.0 * height
            * footprint_live * lobe_live;
    return vec2<f32>(live * shape_altitude / erode_room, cover_partial);
}

/// 软档**每一步**的法线 = **径向解析项 + 覆盖度切向项**。
/// 径向那支是解析的（`soft_shape_axes` 的 x 分量 × `direction / span`），切向那支就是
/// `coverage_and_slope` 白拿的 `.gba` ⇒ 与 `cloud_field_gradient_analytic` 同源、同约定，
/// 只是省掉了 `∂shape/∂noise · ∇noise`（细节噪声的切向梯度不在那三张 slope 图里）。
/// 代价：0 次额外噪声采样、0 次额外立方图采样。
fn soft_step_normal(
    direction: vec3<f32>,
    altitude: f32,
    cover: f32,
    cover_axis: vec3<f32>,
    noise: f32,
) -> vec3<f32> {
    let axes = soft_shape_axes(cover, altitude, noise);
    let radius = max(params.inner + altitude * span(), 1e-5);
    let gradient = axes.x * (direction / span()) + (axes.y / radius) * cover_axis;
    let length_squared = dot(gradient, gradient);
    if length_squared <= 1e-14 {
        // 退化点（`live` 把偏导清零的体内等值面）退回径向朝外：它仍是连续场里的一个值，
        // 比"没有法线"那种开关安全（后者正是银边的来源）。
        return direction;
    }
    // 密度往外降 ⇒ 朝外的法线是梯度的反向（与硬表面那条路同一约定）。
    return -gradient * inverseSqrt(length_squared);
}

fn sampled_noise_along(direction: vec3<f32>, across: f32, octaves: u32, seed: u32, altitude: f32) -> NoiseSample {
    return fbm_3_grad(
        direction * (across + altitude * across * span()),
        octaves,
        2.0,
        0.5,
        seed,
    );
}

fn billows_along(direction: vec3<f32>, altitude: f32, with_skin: bool) -> vec4<f32> {
    if params.ablate == ABLATE_NOISE {
        return vec4<f32>(detail_curve(0.55), vec3<f32>(0.0));
    }
    let radius = params.inner + altitude * span();
    let tower_across = params.detail_scale * 0.35;
    // 风偏置与 `billows` 那一份**必须逐位相同**：值和它的解析梯度是同一次采样的两面，
    // 偏置错开一点，法线就和密度错开（在风里表现为"边缘的明暗追不上轮廓"）。
    let tower = sampled_noise_along(
        direction + wind_tower,
        tower_across,
        3u,
        params.seed,
        altitude,
    );
    let tower_stretch = tower_across + altitude * tower_across * span();
    if !with_skin {
        let clamped = clamp(tower.value, 0.0, 1.0);
        let live = gate_open(clamped);
        let tangential = live * tower_stretch * project_tangential(tower.gradient, direction);
        let radial = live * radius * tower_across * dot(direction, tower.gradient);
        return vec4<f32>(
            detail_curve(clamped),
            (tangential + radial * direction) * detail_curve_slope(clamped),
        );
    }
    let skin_across = params.detail_scale * 1.70;
    let skin = sampled_noise_along(
        direction + wind_skin,
        skin_across,
        2u,
        params.seed ^ 31u,
        altitude,
    );
    let skin_stretch = skin_across + altitude * skin_across * span();
    let blended = clamp(tower.value * 0.62 + skin.value * 0.38, 0.0, 1.0);
    let live = gate_open(blended);
    let tangential = live
        * (tower_stretch * project_tangential(tower.gradient, direction) * 0.62
            + skin_stretch * project_tangential(skin.gradient, direction) * 0.38);
    let radial = live
        * radius
        * (tower_across * dot(direction, tower.gradient) * 0.62
            + skin_across * dot(direction, skin.gradient) * 0.38);
    // 链式法则：值是 `detail_curve(blended)` ⇒ 梯度要乘它的斜率。`live` 已经把被 clamp
    // 夹住的两端（b ≤ 0 / b ≥ 1）清零，剩下的中间段才乘这个因子。
    return vec4<f32>(
        detail_curve(blended),
        (tangential + radial * direction) * detail_curve_slope(blended),
    );
}

fn coverage_gradient_of(direction: vec3<f32>) -> vec4<f32> {
    if params.ablate == ABLATE_FETCH {
        return vec4<f32>(0.45, 0.0, 0.0, 0.0);
    }
    let baked = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0);
    let normalized = clamp(
        (baked.r - params.coverage) / max(1.0 - params.coverage, 1e-4),
        0.0,
        1.0,
    );
    let cover = smoothstep(0.0, 0.45, normalized);
    let slope = select(0.0, 6.0 * normalized * (1.0 - normalized) / 0.45, normalized > 0.0 && normalized < 1.0);
    return vec4<f32>(
        cover,
        slope / max(1.0 - params.coverage, 1e-4) * project_tangential(vec3<f32>(baked.g, baked.b, baked.a), direction),
    );
}

/// 一次立方图采样同时给出**覆盖度**与它的**切向梯度**（`.gba` = `slope_x/y/z`，即 `mixed`
/// 场的三轴梯度）。软档每步都要这两样：覆盖度决定密度，切向梯度是那一步法线的一半。
/// `.r` 那条算式与 `coverage_of` 逐字相同 ⇒ 密度场一个 bit 都不变，白拿的只是 `.gba`。
fn coverage_and_slope(direction: vec3<f32>) -> vec4<f32> {
    if params.ablate == ABLATE_FETCH {
        return vec4<f32>(0.45, 0.0, 0.0, 0.0);
    }
    let baked = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0);
    let normalized = clamp(
        (baked.r - params.coverage) / max(1.0 - params.coverage, 1e-4),
        0.0,
        1.0,
    );
    let cover = smoothstep(0.0, 0.45, normalized);
    let slope = select(0.0, 6.0 * normalized * (1.0 - normalized) / 0.45, normalized > 0.0 && normalized < 1.0);
    return vec4<f32>(
        cover,
        slope / max(1.0 - params.coverage, 1e-4) * project_tangential(vec3<f32>(baked.g, baked.b, baked.a), direction),
    );
}

fn kink_margin(medium: Medium, direction: vec3<f32>) -> f32 {
    var margin = min(medium.altitude, 1.0 - medium.altitude);
    if params.ablate == ABLATE_FETCH {
        return margin;
    }
    let baked = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0).r;
    margin = min(margin, abs(baked - params.coverage) / max(1.0 - params.coverage, 1e-4) / 0.45);
    if params.ablate == ABLATE_NOISE {
        return margin;
    }
    let cover = coverage_of(direction);
    let noise = billows(direction, medium.altitude, true);
    let height = clamp(medium.altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let lobed = clamp((footprint + noise - 1.0) * params.coverage_gain, 0.0, 1.0);
    margin = min(margin, footprint / max(params.coverage_gain, 1e-4));
    margin = min(margin, min(lobed, 1.0 - lobed) / params.coverage_gain);
    let shape = shape_of(cover, medium.altitude, noise);
    return min(margin, min(shape, 1.0 - shape) * max(1.0 - params.erode, 1e-4));
}

fn cloud_field_gradient_analytic(point: vec3<f32>) -> vec3<f32> {
    let local = to_local(point);
    let medium = medium_of(point);
    let altitude = medium.altitude;
    if !(altitude >= 0.0 && altitude <= 1.0) {
        return vec3<f32>(0.0);
    }
    let direction = medium.direction;
    let cover = coverage_gradient_of(direction);
    if cover.r <= 0.0 {
        return vec3<f32>(0.0);
    }
    let radius = max(length(local), 1e-5);
    let billow = billows_along(direction, altitude, true);
    let partials = shape_of_partials(cover.r, altitude, billow.x);
    let altitude_axis = direction / span();
    let cover_axis = project_tangential(cover.gba, direction);
    let noise_axis = project_tangential(billow.yzw, direction)
        + dot(direction, billow.yzw) * direction;
    let gradient = partials.altitude * altitude_axis
        + (partials.noise / radius) * noise_axis
        + (partials.cover / radius) * cover_axis;
    return to_world(gradient);
}

fn sun_shadow(point: vec3<f32>, sun: vec3<f32>, reach: f32) -> f32 {
    if reach <= 0.0 || params.ablate == ABLATE_SUN {
        return 1.0;
    }
    let cover = coverage_of(medium_of(point + sun * reach * 0.5).direction);
    let depth = cover * params.shadow * SHADOW_GAIN;
    let thin = exp(-depth);
    let middle = exp(-depth * 0.5);
    let thick = exp(-depth * 0.25);
    return thin * 0.35 + middle * 0.35 + thick * 0.30;
}

fn detail_shading(direction: vec3<f32>, sun: vec3<f32>) -> f32 {
    if params.ablate == ABLATE_DETAIL || params.bump <= 0.0 {
        return 1.0;
    }
    let sample = fbm_3_grad(direction * params.detail_scale * 0.35, 3u, 2.0, 0.5, params.seed);
    let up = direction - sample.gradient * params.bump;
    let length_squared = dot(up, up);
    if length_squared <= 1e-8 {
        return 1.0;
    }
    let normal = to_world(up * inverseSqrt(length_squared));
    return mix(0.45, 1.0, clamp(dot(normal, sun) + 0.5, 0.0, 1.0));
}

fn phase_hg(cosine: f32, g: f32) -> f32 {
    let gg = g * g;
    let denominator = max(1.0 + gg - 2.0 * g * cosine, 1e-4);
    return (1.0 - gg) / (4.0 * 3.14159265 * pow(denominator, 1.5));
}

fn phase_forward(g: f32) -> f32 {
    let gap = max(1.0 - g, 1e-3);
    return (1.0 + g) / (4.0 * 3.14159265 * gap * gap);
}

fn scene_distance(fragment: vec2<f32>) -> f32 {
    let depth = textureLoad(depth_prepass_texture, vec2<i32>(fragment), 0);
    if depth <= 0.0 {
        return 1e9;
    }
    let scale = -depth_ndc_to_view_z(depth);
    let ndc = (fragment - view.viewport.xy) / view.viewport.zw * 2.0 - 1.0;
    let clip = view.clip_from_view;
    let scene = vec3<f32>(ndc.x / clip[0][0], ndc.y / clip[1][1], 1.0) * scale;
    return length(scene);
}

/// 绝对栅格上的第 `index` 个采样点：由壳的入射点**逐次相加**走出来 —— 与全量 march 的
/// 加法次序逐位一致。换成 `entry + f32(index) * stride` 末位会不同，阈值附近就翻面。
fn grid_point(camera: vec3<f32>, ray: vec3<f32>, entry: f32, stride: f32, index: u32) -> vec3<f32> {
    var along = entry;
    for (var step = 0u; step < index; step += 1u) {
        along += stride;
    }
    return camera + ray * along;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // 细节风的第一句：这一帧这一刻的两层风偏置（§61）。`wind == 0` 时是零向量。
    arm_wind();
    if params.ablate == ABLATE_NORMALS || params.ablate == ABLATE_SURFACE {
        let camera = view.world_position.xyz;
        let away = in.world_position.xyz - camera;
        let distance = length(away);
        if distance <= 1e-5 {
            discard;
        }
        let ray = away / distance;
        let shell = shell_thickness(camera, ray, params.inner, params.outer);
        if !shell.valid || shell.exit <= shell.entry {
            discard;
        }

        let surface_steps = max(params.steps, 1u);
        let stride = (shell.exit - shell.entry) / f32(surface_steps);

        // 代理几何的 fragment 只决定**从哪个下标起步**：栅格仍锚在壳的入射点上（绝对），
        // 采到的点与全量 march 是逐位同一批 ⇒ 外观逐字节不变。落点夹在两格之间就取 ceil
        // （它之后含的第一个采样）。球壳场景的 fragment 落在入射点上 ⇒ 起点恒为 0。
        let landing = clamp(ceil((distance - shell.entry) / stride), 0.0, f32(surface_steps - 1u));
        let start = u32(landing);

        var along = shell.entry;
        for (var index = 0u; index < start; index += 1u) {
            along += stride;
        }

        var found = false;
        var surface_point = camera + ray * shell.entry;
        if cloud_field(camera + ray * along) > params.surface_level {
            // 落点已经在云里（代理允许在内、切穿云）：往相机那侧（往回）找入射面。
            // 第一个 ≤ 阈值的下一个下标就是入射面 —— 往回补的这批点与全量 march 同批。
            var walk = start;
            loop {
                if walk == 0u {
                    break;
                }
                if cloud_field(grid_point(camera, ray, shell.entry, stride, walk - 1u)) <= params.surface_level {
                    break;
                }
                walk -= 1u;
            }
            surface_point = grid_point(camera, ray, shell.entry, stride, walk);
            found = true;
        } else {
            var walk = start;
            loop {
                if walk + 1u >= surface_steps {
                    break;
                }
                walk += 1u;
                along += stride;
                if cloud_field(camera + ray * along) > params.surface_level {
                    surface_point = camera + ray * along;
                    found = true;
                    break;
                }
            }
        }
        if !found {
            discard;
        }

        // 法线是这段里唯一读梯度的东西（§43：找面靠步进，梯度只用来算法线）。
        // `gradient == 0` 时**整个调用都不发**，否则量到的是"发了再覆盖"的代价。
        var normal = vec3<f32>(0.0, 1.0, 0.0);
        if params.gradient != 0u {
            let gradient = cloud_field_gradient_analytic(surface_point);
            let length_squared = dot(gradient, gradient);
            if length_squared <= 1e-14 {
                discard;
            }
            normal = -gradient * inverseSqrt(length_squared);
        }
        if params.ablate == ABLATE_NORMALS {
            return vec4<f32>(normal * 0.5 + vec3<f32>(0.5), 1.0);
        }
        let sun = sun_light(in.world_position.xyz, in.position.xy).direction;
        let lit = clamp(dot(normal, sun), 0.0, 1.0);
        return vec4<f32>(params.tint.rgb * (0.14 + 0.86 * lit), 1.0);
    }

    let camera = view.world_position.xyz;
    let away = in.world_position.xyz - camera;
    let distance = length(away);
    if distance <= 1e-5 {
        discard;
    }
    let ray = away / distance;

    let hit = shell_thickness(camera, ray, params.inner, params.outer);
    if !hit.valid {
        discard;
    }
    let chord = min(hit.exit, scene_distance(in.position.xy)) - hit.entry;
    if chord <= 0.0 {
        discard;
    }

    let ceiling_steps = f32(max(params.steps, 16u));
    let stride = max(span() * 0.045, 1e-5);
    let steps = u32(clamp(chord / stride, 16.0, ceiling_steps));
    let step = chord / f32(steps);

    // 太阳由场景那盏灯说了算（§60）：点光源的方向**每个着色点都不一样**，所以这里
    // 现算一次，整条步进共用（云壳只有 0.05 个半径厚，壳内方向变化可以忽略）。
    let principal = sun_light(in.world_position.xyz, in.position.xy);
    let sun = principal.direction;
    let middle = camera + ray * (hit.entry + chord * 0.5);
    let reach = shell_thickness(middle, sun, params.inner, params.outer);
    let sun_reach = select(0.0, reach.exit, reach.valid && reach.exit > 0.0);
    let opaque = 6.0 / max(params.density, 1e-4);

    // 软云档（`gradient = 2`）：没有"有没有法线"这个开关了 —— 每一步都算这一步自己的受光，
    // 再按前向透射率积分（见下面的循环）。所以掠射轮廓上不可能再出现着色台阶。
    let soft = params.gradient == SOFT_GRADIENT;

    let phase = 0.60 + 0.40 * phase_hg(dot(-ray, sun), params.phase) / phase_forward(params.phase);
    let detail = detail_shading(medium_of(camera + ray * hit.entry).direction, sun);

    var optical = 0.0;
    var scattered = 0.0;
    // 软档的前向积分量：`transmittance` 是"这一步之前还剩下多少可见"，`lit_sum` 是
    // Σ 可见比例 × 这一步的受光。两者之比就是这条可见链上受光的透射率加权平均 ⇒
    // 埋得多深由透射率表达（旧的 `(1−密度)` 那条极性反转的公式与"拿像素 alpha 当
    // shadow_depth"的耦合一起退休）。
    var transmittance = 1.0;
    var lit_sum = 0.0;
    var along = hit.entry + step * 0.5;
    for (var index = 0u; index < steps; index += 1u) {
        let point = camera + ray * along;
        along += step;
        let medium = medium_of(point);
        if medium.altitude < 0.0 || medium.altitude > 1.0 {
            continue;
        }
        var cover = 0.0;
        var cover_axis = vec3<f32>(0.0);
        if soft {
            let slot = coverage_and_slope(medium.direction);
            cover = slot.x;
            cover_axis = slot.yzw;
        } else {
            cover = coverage_of(medium.direction);
        }
        if cover <= 0.0 {
            continue;
        }
        // 软档的保守上界早退：与 `cloud_field` 里那个 `bound`（§51.7）是同一条论证 ——
        // `shape_of` 对 noise 单调非降、`billows` 夹在 [0,1] ⇒ 上界够不着等值面 ⇒
        // 下面那次 `smoothstep` 恰为 0 ⇒ 这一步的 `visible` 是 0、透射率一字不变
        // ⇒ 整段跳过与算出来**逐位相同**，省掉 `billows` ＋ 那一步的法线 ＋ 一次 shadow map 采样。
        if soft && params.bound != 0u && shape_of(cover, medium.altitude, 1.0) <= params.surface_level {
            continue;
        }
        let noise = billows(medium.direction, medium.altitude, true);
        let field_density = shape_of(cover, medium.altitude, noise);
        if field_density <= 0.0 {
            continue;
        }
        var density = field_density;
        if soft {
            // 等值面取硬表面那条路的同一个阈值：轮廓是软的（斜率为 0 的爬升），体内是实的。
            density = smoothstep(params.surface_level, params.surface_level + SOFT_EDGE, density);
            // 这一步自己的可见比例（前向）：它同时是这一步散射的权重与下一步的透射率。
            let visible = 1.0 - exp(-density * step * params.density);
            let normal = soft_step_normal(
                medium.direction,
                medium.altitude,
                cover,
                cover_axis,
                noise,
            );
            let facing = clamp(dot(normal, sun), 0.0, 1.0);
            // 这一步的受光 = 硬表面同一条 Lambert（全亮 1.0 / 全暗 0.14）；`shadow` 是自阴影
            // 强度旋钮（0 = 关）。密度不再额外乘一次：它已经通过 `visible` 进了权重，再乘就是双计。
            let lit = mix(1.0, 0.14 + 0.86 * facing, clamp(params.shadow, 0.0, 1.0));
            // 「别人投在云上的影」：山尖 / 环挡住的太阳，由 Bevy 的阴影贴图说了算（§59.1）。
            // **自己的自阴影不在这里** —— 那一条是上面那句 `facing`（每步法线的 N·L）。
            // 影子是 cube 还是级联由**灯的种类**决定（§60）：点光源查 cube，方向光查级联。
            // 灯没开影子时 `principal.shadow_maps == 0` ⇒ 一次采样都不发（输出仍是 1.0）。
            // ⚠ 名字不能叫 `cast`：WGSL 的保留字。
            var cast_shadow = 1.0;
            if facing > 0.0 && principal.shadow_maps != 0u {
                if principal.point == 1u {
                    cast_shadow = fetch_point_shadow(
                        principal.shadow_id,
                        vec4<f32>(point, 1.0),
                        normal,
                        in.position.xy,
                    );
                } else {
                    cast_shadow = fetch_directional_shadow(
                        principal.shadow_id,
                        vec4<f32>(point, 1.0),
                        normal,
                        view_z_of(point),
                        in.position.xy,
                    );
                }
            }
            lit_sum += transmittance * visible * lit * cast_shadow;
            transmittance *= 1.0 - visible;
            optical += density * step;
        } else {
            optical += density * step;
            scattered += density * sun_shadow(point, sun, sun_reach) * step;
        }
        if optical > opaque {
            break;
        }
    }

    let alpha = 1.0 - exp(-optical * params.density);
    if alpha < 0.002 {
        discard;
    }
    let average = scattered / max(optical, 1e-6);
    // 老路：三阶透射率平均出来的是"没被遮挡的比例"，再压进 [0.35, 1.05]。
    let legacy_shade = mix(0.35, 1.05, clamp(average, 0.0, 1.0));
    // 软档：`lit_sum / alpha` 是透射率加权平均受光（权重和 = 1 − T = alpha）⇒ 值域就是
    // 硬表面那条 Lambert 的 [0.14, 1.0]：全不透时逐字收敛到硬表面，薄处既不会被抬亮、
    // 体内也不会被压到中性 0.5（(D) 那条体型反转随之消失）。
    let soft_shade = clamp(lit_sum / max(alpha, 1e-6), 0.0, 1.0);
    let shade = select(legacy_shade, soft_shade, soft);
    // 老路的 detail（噪声 bump，最暗 0.45）与 phase（背散射时 ~0.62）只在老路乘 ——
    // 那两项正是老体积云发灰的原因，软档要能在全不透时收敛到硬表面。
    let veil = select(detail * phase, 1.0, soft);
    return vec4<f32>(params.tint.rgb * shade * veil * alpha, alpha);
}
