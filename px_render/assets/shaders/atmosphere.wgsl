#import planet_x::common::{SUN_DIRECTION, shell_thickness}
#import bevy_pbr::forward_io::VertexOutput

struct AtmosphereParams {
    inner: f32,
    outer: f32,
    density: f32,
    softness: f32,
    camera_x: f32,
    camera_y: f32,
    camera_z: f32,
    screen_x: f32,
    screen_y: f32,
    padding_a: f32,
    padding_b: f32,
    padding_c: f32,
    forward: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    reserved: vec4<f32>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: AtmosphereParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> tint: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let fragment = in.position.xy;
    let ndc = vec2<f32>(
        (fragment.x / params.screen_x) * 2.0 - 1.0,
        1.0 - (fragment.y / params.screen_y) * 2.0,
    );
    let camera = vec3<f32>(params.camera_x, params.camera_y, params.camera_z);
    let ray = normalize(params.forward.xyz + params.right.xyz * ndc.x + params.up.xyz * ndc.y);

    let hit = shell_thickness(camera, ray, params.inner, params.outer);
    if !hit.valid {
        discard;
    }

    let thickness = hit.exit - hit.entry;
    let steps = 4;
    var optical = 0.0;
    for (var index = 0; index < steps; index += 1) {
        let along = mix(hit.entry, hit.exit, (f32(index) + 0.5) / f32(steps));
        let point = camera + ray * along;
        let altitude = (length(point) - params.inner) / max(params.outer - params.inner, 1e-4);
        optical += exp(-max(altitude, 0.0) * 4.0) * thickness / f32(steps);
    }
    let alpha = 1.0 - exp(-optical * params.density);
    let middle = camera + ray * ((hit.entry + hit.exit) * 0.5);
    let height = clamp(length(middle) / max(params.outer, 1e-4), 0.0, 1.0);
    let facing = max(dot(ray, normalize(SUN_DIRECTION)), 0.0);
    let soft = pow(1.0 - height, params.softness);
    let lit = 0.02 + 0.98 * pow(facing, 0.65) * (0.30 + 0.70 * soft);
    return vec4<f32>(tint.rgb * lit, alpha);
}





