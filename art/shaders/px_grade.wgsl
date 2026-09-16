//! 全屏 pass 入口：**与材质同一条绑定契约**（§79 的 C 案）。
//!
//! 第 0 格是这份 shader 自己声明的参数块，贴图从第 1 格起（采样器在 `+1`）。
//! 参数由**配方**按名字给（`art/passes/*.toml` 的 `params`），烘图时按这里的结构体打包。
//! `#{MATERIAL_BIND_GROUP}` 由组装器替成运行期那个数 —— 手写 2 或 3 就是第二个会漂开的真相。

struct PxGradeParams {
    /// 0 = 原样，1 = 完全反相。
    strength: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: PxGradeParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var px_source: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var px_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(px_source, px_sampler, uv);
    let inverted = vec3<f32>(1.0 - color.rgb);
    return vec4<f32>(mix(color.rgb, inverted, params.strength), color.a);
}
