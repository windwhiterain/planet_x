// 环：一张带 alpha 的环带贴图，**不受光**（原来是 Bevy 的 `StandardMaterial { unlit: true }`）。
// 迁移前它走的是内建材质；通用渲染之后所有材质都是同一份自写材质（一份 WGSL + 一个参数块），
// 所以这条不发光的路也要有自己的 shader —— 渲染器里没有"内建材质"这第二档了。
//
// 绑定按约定（`px_render::reflect`）：0 = 参数块、1/2 = 2D 贴图 + 采样器。
#import bevy_pbr::forward_io::VertexOutput

struct RingParams {
    tint: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: RingParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var ring_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var ring_sampler: sampler;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(ring_texture, ring_sampler, in.uv);
    let alpha = color.a * params.tint.a;
    return vec4<f32>(color.rgb * params.tint.rgb, alpha);
}
