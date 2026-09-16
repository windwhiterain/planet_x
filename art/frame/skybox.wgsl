// 天空盒片元阶段（帧自有；§131/§132 那一单元）。
//
// ⚠ **这一份不进 `art/shaders/`、不进 CAS**：它不是内容（内容住在 CAS 里、由文档的成员键指过来），
//    而是**帧自己的元阶段** —— 和 `vertex_sky.wgsl` / `vertex_mesh.wgsl` 一样住在帧图旁边。
//    "为了让 sky 跑起来而在宿主里自造一份天空盒 shader"是被明确否掉的做法：那是把内容塞进宿主。
//
// 逐字抄 Bevy `bevy_core_pipeline-0.19.1/src/skybox/skybox.wgsl`：
//   - 方向重建 `coords_to_ray_direction`（`:19-46`）+ `bevy_pbr` 的
//     `coords_to_viewport_uv`（`bevy_pbr-0.19.1/src/render/utils.wgsl:42-44`）；
//   - 立方图采样与亮度（`:74-81`）：`textureSample(skybox, skybox_sampler, dir * vec3(1.0, 1.0, -1.0))`
//     后 `rgb * brightness`，**alpha 原样返回**。
//
// ⚠ 方向重建的**算术路径**必须与 Bevy 相同，不能"等价地换一条"：
//    Bevy 走的是 `in.position.xy`（**片元坐标**）+ `view.viewport` + **逆矩阵**
//    （`view_from_clip`、`world_from_view`），**不是**插值下来的裁剪坐标。
//    两条路算同一个方向、浮点不同 ⇒ 逐位判据下整幅背景差 ±1，而画面看起来完全正确。
//
// ⚠ **内容 vs 策略**（这一栏不许混）：
//   - 内容（从文档来）：`brightness` 与立方图 —— 前者由帧材质表声明来源
//     （`environment.skybox_brightness`）、在**烘图时**打包进参数块；后者是环境里那一份
//     `environment.skybox`（一个 cube 成员），由宿主解析成 GPU 句柄。**WGSL 里一个内容字面量都不许出现。**
//   - 策略（住在这里/配方里）：方向重建、`position` 的 z 取 0.0（无限 reverse-Z 的远平面）、
//     `depth_write = false` / `compare = greater_equal` / `cull = none`（后三条已经在
//     `art/frame/default.toml` 的 `sky` 那一条里）、以及"画在 opaque 之后 transparent 之前"。
//
// ⚠ 采样器那一格**由宿主从产物那侧建**（`px_render_wgpu::material::sampler_of`），
//    因为 oracle 那条路就是"图用它自己带的采样器"（`px_render/src/art_cache.rs:477`）；
//    而这份星图产物**不带采样器字段** ⇒ 取的是**装载方显式传的那一个**
//    （oracle 传的是 `Sampler::clamped()`，`px_render/src/scene.rs:294`）。
//    详见 `sampler_of` 的注释：那里记着"Bevy 的缺省过滤是 Nearest、本工程是 Linear"这个陷阱。

#import bevy_pbr::mesh_view_bindings::{view}

struct SkyboxParams {
    /// 天空盒亮度倍率。**来源**是 `environment.skybox_brightness`（烘图时按反射布局打包成值）。
    brightness: f32,
};

// ⚠ 格位不是随手挑的：帧材质与内容材质**共用同一份材质契约表**
//    （`px_protocol::material::TEXTURE_SLOTS`：立方图只许占 5 / 7 / 21 / 23，
//    第 1 格是**2D** 那一档）。反射器（`px_shader::reflect`）按那张表逐格校验，
//    立方图写在第 1 格会被当场拒 —— 而宿主复用的正是材质那条绑定组构造
//    （12 格超集 + 空槽绑白图），换一套格位就等于同一个东西两份契约。
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: SkyboxParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var skybox: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var skybox_sampler: sampler;

struct SkyVertexOutput {
    @builtin(position) position: vec4<f32>,
};

/// `bevy_pbr-0.19.1/src/render/utils.wgsl:42-44`：`(position - viewport.xy) / viewport.zw`。
fn coords_to_viewport_uv(position: vec2<f32>, viewport: vec4<f32>) -> vec2<f32> {
    return (position - viewport.xy) / viewport.zw;
}

/// `bevy_core_pipeline-0.19.1/src/skybox/skybox.wgsl:19-46` —— 逐行相同。
///
/// ⚠ 它用的是**近裁剪面**上的那个点（`w = 1.0` 的齐次坐标送进逆矩阵），不是远平面：
///    无限 reverse-Z 的远平面在世界系里是无穷远，拿它算方向会退化。
fn coords_to_ray_direction(position: vec2<f32>, viewport: vec4<f32>) -> vec3<f32> {
    let view_position_homogeneous = view.view_from_clip * vec4(
        coords_to_viewport_uv(position, viewport) * vec2(2.0, -2.0) + vec2(-1.0, 1.0),
        1.0,
        1.0,
    );

    // 视图空间里相机在原点 ⇒ 片元位置的方向就是射线方向。
    var view_ray_direction = view_position_homogeneous.xyz / view_position_homogeneous.w;
    view_ray_direction = (view.world_from_view * vec4(view_ray_direction, 0.0)).xyz;

    // ⚠ `w = 0.0`：这是**方向**不是位置，于是矩阵里的平移被忽略。
    //    本帧的 `transform` 是单位阵（天空盒没有旋转）⇒ 这一乘是恒等；
    //    留着它是因为 Bevy 那一行在这儿，去掉就是"等价地换一条路径"。
    let ray_direction = view_ray_direction;

    return normalize(ray_direction);
}

@fragment
fn fragment(in: SkyVertexOutput) -> @location(0) vec4<f32> {
    let ray_direction = coords_to_ray_direction(in.position.xy, view.viewport);

    // 立方图是左手系 ⇒ z 取反（`skybox.wgsl:78-79`）。
    let out = textureSample(skybox, skybox_sampler, ray_direction * vec3(1.0, 1.0, -1.0));
    return vec4<f32>(out.rgb * params.brightness, out.a);
}
