@group(0) @binding(0) var px_source: texture_2d<f32>;
@group(0) @binding(1) var px_sampler: sampler;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(px_source, px_sampler, uv);
    return vec4<f32>(1.0 - color.rgb, color.a);
}
