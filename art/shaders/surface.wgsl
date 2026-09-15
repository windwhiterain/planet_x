#import planet_x::light::{sun_light, view_z_of}
#import planet_x::noise::rotate_vector
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::{view, lights}
#import bevy_pbr::shadows::{fetch_directional_shadow, fetch_point_shadow}

// 行星表面的自写材质（§39.6 阶段 3/4 的那一步，口径与实测见 `06-clouds.md` §59）。
//
// 为什么必须自写：`StandardMaterial` 的两条光照路径都没有"只压直接光"的位置 ——
// `diffuse_occlusion` 只吃间接光，直接光那一项在 `apply_pbr_lighting` 里直接乘了阴影贴图
// 的结果，外面拿不到。所以要往地表投云影，只能自己拥有 surface shader。
//
// 换来的是两件事可以分开算：
//   ① 直接光 × 阴影贴图（山自己投的影、环投在行星上的影）—— 归 Bevy 的 shadow map；
//   ② 直接光 × 云影 —— 归这张云覆盖度立方图，自成本文件里的一条解析近似。
// 间接光（环境光）两项都不乘：云在天上挡的是太阳，不是天光（§39.6 阶段 4 的口径）。
struct SurfaceParams {
    /// 行星的**世界朝向**（`SYSTEM_TILT × spin`），与云材质拿的是同一个四元数：
    /// 覆盖度立方图烘在未倾斜的局部系里，查它之前要把方向转回去。
    orientation: vec4<f32>,
    /// glow（emissive）的强度。没有 glow 贴图时是 0 —— 那样乘上兜底白图也不发光。
    emissive: vec4<f32>,
    /// 云壳的绝对内/外半径（× 行星半径之后的数，与云材质同一口径）。
    inner: f32,
    outer: f32,
    /// 云覆盖度的阈值：与 `clouds.wgsl` 的 `coverage_of` 逐字同一个口径，
    /// 否则"云在哪里"这件事会有两份答案。
    coverage: f32,
    /// 云影强度：0 = 关（`cloud_shadow` 场景参数）。
    shadow: f32,
    /// 「指定高度」h：太阳光线打在云带上哪个归一化海拔上（0 = 云底、1 = 云顶）。
    height: f32,
    /// 那一个采样点的覆盖度 → 光学深度的增益。
    gain: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SurfaceParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var albedo_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var albedo_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var glow_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var glow_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var coverage_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var coverage_sampler: sampler;

const PI: f32 = 3.141592653589793;
/// 半影：沿"太阳切向"那根轴在方向空间里张开多少（一个方向单位 ≈ 57°）。
/// 三个 tap 取平均 ⇒ 云影的边缘是一条软带而不是一条硬边。
const CLOUD_PENUMBRA: f32 = 0.045;

fn to_local(direction: vec3<f32>) -> vec3<f32> {
    return rotate_vector(vec4<f32>(-params.orientation.xyz, params.orientation.w), direction);
}

/// 覆盖度立方图的一次采样 → [0,1] 的覆盖度。算式与 `clouds.wgsl::coverage_of` 同源。
fn coverage_lookup(direction: vec3<f32>) -> f32 {
    let mask = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0).r;
    return smoothstep(
        0.0,
        0.45,
        clamp((mask - params.coverage) / max(1.0 - params.coverage, 1e-4), 0.0, 1.0),
    );
}

/// 云影：从这一片地表沿太阳方向穿云带，落在**指定高度 h** 那一层上的方向查覆盖度。
///
/// 解析近似（不再沿光线 march）：解 `|p + s·t| = r` 的二次式拿到穿过的步长 `t`，
/// 方向那一步只搬**切向分量** —— `s` 的径向那份只把半径抬到云带上（那正是"指定高度"的意思），
/// 方向由 `(s − (s·d)d)·t` 决定。于是一次查询就是"这方向抬头看，云有多厚"。
///
/// `sun` 是从**这一片地表**指向光源的单位向量：太阳换成点光源之后它逐片元不同（§60），
/// 而这个二次式对"每片元一个方向"本来就是对的 —— 云带仍是绕行星的那颗球。
fn cloud_shadow(world_position: vec3<f32>, sun: vec3<f32>) -> f32 {
    if params.shadow <= 0.0 {
        return 1.0;
    }
    let radius = params.inner + clamp(params.height, 0.0, 1.0) * max(params.outer - params.inner, 1e-5);
    let along = dot(world_position, sun);
    let disc = along * along + radius * radius - dot(world_position, world_position);
    if disc <= 0.0 {
        // 这条光线够不到那一层云（太阳在地平线下 / 高度取在壳外）：没有云影。
        return 1.0;
    }
    let travel = -along + sqrt(disc);
    let direction = normalize(world_position);
    let tangential = sun - dot(sun, direction) * direction;
    let probe = normalize(direction + tangential * travel);

    var cover = coverage_lookup(to_local(probe));
    let spread = length(tangential);
    if spread > 1e-4 {
        // 半影只沿太阳切向张开：太阳的正切向就是"影子边缘该往哪边拉"的方向。
        let side = tangential / spread;
        cover = (
            cover
            + coverage_lookup(to_local(normalize(probe + side * CLOUD_PENUMBRA)))
            + coverage_lookup(to_local(normalize(probe - side * CLOUD_PENUMBRA)))
        ) * (1.0 / 3.0);
    }
    let transmittance = exp(-cover * max(params.gain, 0.0));
    return 1.0 - clamp(params.shadow, 0.0, 1.0) * (1.0 - transmittance);
}

/// Disney 的 Burley 漫反射因子（含 `1/π`），逐字对齐 `bevy_pbr::pbr_lighting::Fd_Burley`。
///
/// 为什么照抄而不是用 Lambert：自写材质的判据是"和换材质之前的图并排、差异带有多大"，
/// 直接光的余弦项贴着 Bevy 那一份，差异就只剩高光那一项（我们主动丢掉的东西）。
fn diffuse_burley(roughness: f32, ldotv: f32, ndotl: f32, ndotv: f32) -> f32 {
    let ldot_h_squared = 0.5 + 0.5 * ldotv;
    let f90 = 0.5 + 2.0 * roughness * ldot_h_squared;
    let light_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - ndotl, 5.0);
    let view_scatter = 1.0 + (f90 - 1.0) * pow(1.0 - ndotv, 5.0);
    return light_scatter * view_scatter * (1.0 / PI);
}

/// 环境光那一路的 DFG 近似（`bevy_pbr::pbr_lighting::F_AB` / `EnvBRDFApprox` 逐字一份）。
/// 不照抄的话暗面的亮度会差出好几倍 —— 环境光是这颗行星暗面唯一的照明，它错一点全看得出来。
fn f_ab(perceptual_roughness: f32, ndotv: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = perceptual_roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * ndotv)) * r.x + r.y;
    return max(vec2<f32>(-1.04, 1.04) * a004 + r.zw, vec2<f32>(0.00005));
}

fn env_brdf_approx(f0: vec3<f32>, ab: vec2<f32>) -> vec3<f32> {
    return f0 * ab.x + ab.y;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let albedo = textureSample(albedo_texture, albedo_sampler, in.uv).rgb;
    let normal = normalize(in.world_normal);
    let camera = view.world_position.xyz;
    let view_vector = normalize(camera - in.world_position.xyz);

    // 太阳由场景那盏灯说了算（§60）：点光源时 `sun` 逐片元不同、`principal.color` 已经
    // 含强度与距离衰减；方向光是同一套代码的另一个分支（`color` 就是照度）。
    let principal = sun_light(in.world_position.xyz, in.position.xy);
    let sun = principal.direction;
    let facing = clamp(dot(normal, sun), 0.0, 1.0);
    let ndotv = max(dot(normal, view_vector), 0.0001);

    // ① 别人的影（shadow map）。灯没开影子时（`shadow_maps == 0`）连采样都不发 ——
    //    没开阴影的场景里那张贴图是兜底的 1×1，读它只会白花时间。
    //    ⚠ 名字不能叫 `cast`：WGSL 的保留字。
    var cast_shadow = 1.0;
    if facing > 0.0 && principal.shadow_maps != 0u {
        if principal.point == 1u {
            cast_shadow = fetch_point_shadow(
                principal.shadow_id,
                in.world_position,
                normal,
                in.position.xy,
            );
        } else {
            cast_shadow = fetch_directional_shadow(
                principal.shadow_id,
                in.world_position,
                normal,
                view_z_of(in.world_position.xyz),
                in.position.xy,
            );
        }
    }

    // ② 云影（覆盖度立方图 + 指定高度）。0.88 是换材质之前 `StandardMaterial` 的
    //    perceptual_roughness ⇒ 内建那套把 0.88² 当 roughness 用。
    let cloud = cloud_shadow(in.world_position.xyz, sun);
    let roughness = 0.88 * 0.88;
    let direct = albedo
        * (principal.color * diffuse_burley(roughness, dot(sun, view_vector), facing, ndotv))
        * facing
        * cast_shadow
        * cloud;
    // 环境光：漫反射那一支 ＋ 介电高光那一支（`F0 = 0.08 × reflectance(0.5) = 0.04`），
    // 与 `bevy_pbr::ambient::ambient_light` 同一套算式。**不乘云影**：
    // 云挡的是太阳，不是天光。三支都不乘 `cloud`/`cast_shadow` 的只有这一支。
    let f0 = vec3<f32>(0.04);
    let ambient_specular = env_brdf_approx(f0, f_ab(0.88, ndotv));
    let ambient = (env_brdf_approx(albedo, f_ab(1.0, ndotv)) + ambient_specular)
        * lights.ambient_color.rgb;    let glow = params.emissive.rgb * textureSample(glow_texture, glow_sampler, in.uv).rgb;
    // ⚠ **曝光**：Bevy 把 `view.exposure` 乘在 `apply_pbr_lighting` 的总光里（`pbr_functions.wgsl`
    // 的 864 行），自写材质不乘就会亮三个数量级（EV100 = 9.7 ⇒ 曝光 ≈ 1e-3）。
    // 自写材质里只有这一个是"跟 Bevy 对齐"的硬要求。
    return vec4<f32>(view.exposure * (ambient + direct + glow), 1.0);
}
