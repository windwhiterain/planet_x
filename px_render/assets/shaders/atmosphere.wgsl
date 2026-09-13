#import planet_x::common::SUN_DIRECTION
#import bevy_pbr::forward_io::VertexOutput

struct AtmosphereParams {
    inner: f32,
    outer: f32,
    density: f32,
    softness: f32,
    camera_x: f32,
    camera_y: f32,
    camera_z: f32,
    reserved: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: AtmosphereParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(in.world_normal);
    let surface = normal * params.outer;
    let camera = vec3<f32>(params.camera_x, params.camera_y, params.camera_z);
    let to_camera = normalize(camera - surface);
    let cosine = clamp(dot(normal, to_camera), 0.0, 1.0);
    let impact = params.outer * sqrt(max(1.0 - cosine * cosine, 0.0));

    var chord = 2.0 * params.outer * cosine;
    if impact < params.inner {
        chord = max(
            params.outer * cosine - sqrt(params.inner * params.inner - impact * impact),
            0.0,
        );
    }

    let ray = -to_camera;
    let sun = normalize(SUN_DIRECTION);
    let steps = 5;
    var sunlit = 0.0;
    for (var index = 0; index < steps; index += 1) {
        let along = (f32(index) + 0.5) / f32(steps) * chord;
        let point = surface + ray * along;
        let reach = dot(normalize(point), sun);
        sunlit += clamp((reach + 0.25) / 1.25, 0.0, 1.0);
    }
    sunlit /= f32(steps);

    let alpha = 1.0 - exp(-chord * params.density);
    return vec4<f32>(tint.rgb * (0.05 + 0.95 * sunlit) * alpha, alpha);
}


