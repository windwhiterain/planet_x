#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::forward_io::VertexOutput

struct AtmosphereParams {
    inner: f32,
    outer: f32,
    density: f32,
    softness: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: AtmosphereParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let camera = view.world_position.xyz;
    let ray = normalize(in.world_position.xyz - camera);

    let along = dot(camera, ray);
    let outer_term = dot(camera, camera) - params.outer * params.outer;
    let outer_disc = along * along - outer_term;
    if outer_disc < 0.0 {
        discard;
    }
    let outer_root = sqrt(outer_disc);
    let entry = max(-along - outer_root, 0.0);
    var exit = -along + outer_root;

    let inner_term = dot(camera, camera) - params.inner * params.inner;
    let inner_disc = along * along - inner_term;
    if inner_disc > 0.0 {
        let near = -along - sqrt(inner_disc);
        if near > 0.0 {
            exit = min(exit, near);
        }
    }

    let thickness = max(exit - entry, 0.0);
    let alpha = 1.0 - exp(-thickness * params.density);
    let normal = normalize(in.world_normal);
    let sun = normalize(vec3<f32>(-4.2, 1.15, 2.35));
    let lit = 0.12 + 0.88 * pow(max(dot(normal, sun), 0.0), params.softness);
    return vec4<f32>(tint.rgb * alpha * lit, alpha);
}
