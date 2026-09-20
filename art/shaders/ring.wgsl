// 环：一张带 alpha 的环带贴图 —— **受光，也接受别人的影**。
//
// ⚠ 2026-09-20 改（用户："星环没接受影子吗"）。在此之前这一份是**不受光**的
//   （迁移前是 Bevy 的 `StandardMaterial { unlit: true }`，照搬过来时只把"贴图 × tint"抄下了）：
//     · 行星挡在太阳与环之间时，环**照样亮**（真实画面里那是横贯环的一条暗带）；
//     · 卫星投在环上的影同样没有；
//     · 环的亮度与太阳角度、与"看不看得见"完全无关。
//   实测图：`target/probe-gg-rings.png`（改前）—— 环在行星两侧一样亮。
//
// 改法**只加一件半**（不去重做环的光度学）：
//   ① **遮挡**：采一次点光 shadow map（与 `surface.wgsl` / `gasgiant.wgsl` 同一支函数）。
//      环是**薄片**，它的透过率本来就烘在环带贴图里，所以这里只需要"太阳有没有被挡"这一个因子；
//   ② 影里**不是全黑**：留一档 `ambient`（星光/其它天体的散射），
//      `light = mix(ambient, 1.0, shadow)`。
//   ⚠ 故意**不加 N·L**：薄环的观感不是漫反射面（同一束视线穿过多少颗粒与入射角基本无关），
//     硬套 N·L 会在侧视时把整条环压黑 —— 而"侧视最亮"正是参考图（土星）那一张的样子。
//
// 绑定按约定（`px_render::reflect`）：0 = 参数块、1/2 = 2D 贴图 + 采样器。
#import bevy_pbr::forward_io::VertexOutput
#import planet_x::light::sun_light
#import bevy_pbr::shadows::fetch_point_shadow

struct RingParams {
    tint: vec4<f32>,
    /// 影里的亮度下限（`0` = 影里全黑，`1` = 完全不接受影）。
    ambient: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: RingParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var ring_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var ring_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(ring_texture, ring_sampler, in.uv);

    // 太阳有没有被挡（行星 / 卫星）。灯没开影子时（`shadow_maps == 0`）连采样都不发 ——
    // 那时环照旧是全亮的（与改之前的行为一致）。
    let principal = sun_light(in.world_position.xyz, in.position.xy);
    var shadow = 1.0;
    if principal.shadow_maps != 0u {
        shadow = fetch_point_shadow(
            principal.shadow_id,
            in.world_position,
            in.world_normal,
            in.position.xy,
        );
    }
    let light = mix(params.ambient, 1.0, shadow);

    let alpha = color.a * params.tint.a;
    return vec4<f32>(color.rgb * params.tint.rgb * light, alpha);
}
