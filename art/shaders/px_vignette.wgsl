//! 全屏 pass 入口：**与材质同一条绑定契约**（§79 的 C 案）。
//!
//! 第 0 格是这份 shader 自己声明的参数块，贴图从第 1 格起（采样器在 `+1`）。
//! 参数由**配方**按名字给（`art/passes/*.toml` 的 `params`），烘图时按这里的结构体打包。
//! `#{MATERIAL_BIND_GROUP}` 由组装器替成运行期那个数 —— 手写 2 或 3 就是第二个会漂开的真相。

struct PxVignetteParams {
    /// 1 = 迁移前那份写死的衰减；0 = 不压暗。
    strength: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PxVignetteParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var px_source: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var px_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(px_source, px_sampler, uv);
    let centered = uv - vec2<f32>(0.5, 0.5);
    let falloff = clamp(1.0 - dot(centered, centered) * 2.0 * params.strength, 0.0, 1.0);
    return vec4<f32>(color.rgb * falloff, color.a);
}
