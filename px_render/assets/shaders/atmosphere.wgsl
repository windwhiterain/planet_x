#import planet_x::common::{SUN_DIRECTION, shell_thickness}
#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::forward_io::VertexOutput

struct AtmosphereParams {
    inner: f32,
    outer: f32,
    density: f32,
    softness: f32,
    camera: vec3<f32>,
    padding: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: AtmosphereParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let camera = params.camera;
    let position = normalize(in.world_normal) * params.outer;
    let ray = normalize(position - camera);
    let hit = shell_thickness(camera, ray, params.inner, params.outer);
    if !hit.valid {
        discard;
    }

    let thickness = hit.exit - hit.entry;
    let alpha = 1.0 - exp(-thickness * params.density);
    let normal = normalize(in.world_normal);
    let sun = normalize(SUN_DIRECTION);
    let lit = 0.16 + 0.84 * pow(max(dot(normal, sun), 0.0), params.softness);
    return vec4<f32>(tint.rgb * lit, alpha);
}








