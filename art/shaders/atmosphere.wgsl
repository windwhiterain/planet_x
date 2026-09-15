#import planet_x::light::sun_light
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::{view, depth_prepass_texture}
#import bevy_pbr::view_transformations::depth_ndc_to_view_z

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
    let normal = normalize(in.world_normal);
    let surface = normal * params.outer;
    let camera = view.world_position.xyz;
    let to_camera = normalize(camera - surface);
    let cosine = clamp(dot(normal, to_camera), 0.0, 1.0);

    let entry = distance(camera, surface);
    var end = entry + 2.0 * params.outer * cosine;

    let depth = textureLoad(depth_prepass_texture, vec2<i32>(in.position.xy), 0);
    if depth > 0.0 {
        let scale = -depth_ndc_to_view_z(depth);
        let ndc = (in.position.xy - view.viewport.xy) / view.viewport.zw * 2.0 - 1.0;
        let clip = view.clip_from_view;
        let scene = vec3<f32>(ndc.x / clip[0][0], ndc.y / clip[1][1], 1.0) * scale;
        end = min(end, length(scene));
    }

    let chord = max(end - entry, 0.0);
    let ray = -to_camera;
    // 太阳由场景那盏灯说了算（§60）：点光源时方向随位置变，所以逐采样点现取（5 步而已）。
    let steps = 5;
    var sunlit = 0.0;
    for (var index = 0; index < steps; index += 1) {
        let along = (f32(index) + 0.5) / f32(steps) * chord;
        let point = surface + ray * along;
        let sun = sun_light(point, in.position.xy).direction;
        sunlit += clamp((dot(normalize(point), sun) + 0.25) / 1.25, 0.0, 1.0);
    }
    sunlit /= f32(steps);

    let alpha = 1.0 - exp(-chord * params.density);
    return vec4<f32>(tint.rgb * (0.05 + 0.95 * sunlit) * alpha, alpha);
}
