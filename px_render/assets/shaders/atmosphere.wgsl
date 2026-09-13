#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::forward_io::VertexOutput

struct AtmosphereParams {
    power: f32,
    intensity: f32,
    padding: vec2<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: AtmosphereParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(in.world_normal);
    let normal_view = normalize((view.view_from_world * vec4<f32>(normal, 0.0)).xyz);
    let rim = pow(1.0 - abs(normal_view.z), params.power);
    let sun = normalize(vec3<f32>(-4.2, 1.15, 2.35));
    let lit = 0.18 + 0.82 * max(dot(normal, sun), 0.0);
    return vec4<f32>(tint.rgb * rim * params.intensity * lit, rim);
}
