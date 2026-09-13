#import planet_x::common::{SUN_DIRECTION, shell_thickness}
#import planet_x::noise::{fbm_3, rotate_vector}
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::{view, depth_prepass_texture}
#import bevy_pbr::view_transformations::depth_ndc_to_view_z

struct CloudParams {
    orientation: vec4<f32>,
    tint: vec4<f32>,
    inner: f32,
    outer: f32,
    density: f32,
    coverage: f32,
    base: f32,
    top: f32,
    detail_scale: f32,
    detail_strength: f32,
    erode: f32,
    phase: f32,
    shadow: f32,
    steps: u32,
    sun_steps: u32,
    seed: u32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: CloudParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var coverage_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var coverage_sampler: sampler;

fn cloud_density(point: vec3<f32>) -> f32 {
    let local = rotate_vector(vec4<f32>(-params.orientation.xyz, params.orientation.w), point);
    let radius = length(local);
    if radius < params.inner || radius > params.outer {
        return 0.0;
    }
    let span = max(params.outer - params.inner, 1e-5);
    let altitude = (radius - params.inner) / span;

    let direction = local / max(radius, 1e-5);
    let mask = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0).r;
    let cover = smoothstep(
        0.0,
        0.45,
        clamp((mask - params.coverage) / max(1.0 - params.coverage, 1e-4), 0.0, 1.0),
    );
    if cover <= 0.0 {
        return 0.0;
    }

    let tower = fbm_3(direction * params.detail_scale * 0.35, 1.0, 3u, 2.0, 0.5, params.seed);
    let skin = fbm_3(
        direction * params.detail_scale * 1.70 * (1.0 + altitude * 0.50),
        1.0,
        2u,
        2.0,
        0.5,
        params.seed ^ 31u,
    );
    let noise = clamp(tower * 0.62 + skin * 0.38, 0.0, 1.0);

    let lobed = cover * (0.45 + 0.55 * noise);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, altitude);
    let shape = clamp(floor_here * under_top * lobed, 0.0, 1.0);
    return clamp((shape - params.erode) / max(1.0 - params.erode, 1e-4), 0.0, 1.0);
}

fn sun_transmittance(point: vec3<f32>) -> f32 {
    let sun = normalize(SUN_DIRECTION);
    let reach = shell_thickness(point, sun, params.inner, params.outer);
    if !reach.valid || reach.exit <= 0.0 {
        return 1.0;
    }
    let steps = max(params.sun_steps, 1u);
    let step = reach.exit / f32(steps);
    var optical = 0.0;
    var along = step * 0.5;
    for (var index = 0u; index < steps; index += 1u) {
        optical += cloud_density(point + sun * along);
        along += step;
    }
    let depth = optical * step * params.density;
    let thin = exp(-depth);
    let middle = exp(-depth * 0.5);
    let thick = exp(-depth * 0.25);
    let scattered = thin * 0.35 + middle * 0.35 + thick * 0.30;
    return mix(1.0, scattered, clamp(params.shadow, 0.0, 1.0));
}

fn phase_hg(cosine: f32, g: f32) -> f32 {
    let gg = g * g;
    let denominator = max(1.0 + gg - 2.0 * g * cosine, 1e-4);
    return (1.0 - gg) / (4.0 * 3.14159265 * pow(denominator, 1.5));
}

fn phase_forward(g: f32) -> f32 {
    let gap = max(1.0 - g, 1e-3);
    return (1.0 + g) / (4.0 * 3.14159265 * gap * gap);
}

fn scene_distance(fragment: vec2<f32>) -> f32 {
    let depth = textureLoad(depth_prepass_texture, vec2<i32>(fragment), 0);
    if depth <= 0.0 {
        return 1e9;
    }
    let scale = -depth_ndc_to_view_z(depth);
    let ndc = (fragment - view.viewport.xy) / view.viewport.zw * 2.0 - 1.0;
    let clip = view.clip_from_view;
    let scene = vec3<f32>(ndc.x / clip[0][0], ndc.y / clip[1][1], 1.0) * scale;
    return length(scene);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let camera = view.world_position.xyz;
    let away = in.world_position.xyz - camera;
    let distance = length(away);
    if distance <= 1e-5 {
        discard;
    }
    let ray = away / distance;

    let hit = shell_thickness(camera, ray, params.inner, params.outer);
    if !hit.valid {
        discard;
    }
    let chord = min(hit.exit, scene_distance(in.position.xy)) - hit.entry;
    if chord <= 0.0 {
        discard;
    }

    let steps = max(params.steps, 1u);
    let step = chord / f32(steps);
    let sun = normalize(SUN_DIRECTION);
    let phase = 0.60 + 0.40 * phase_hg(dot(-ray, sun), params.phase) / phase_forward(params.phase);

    var optical = 0.0;
    var scattered = 0.0;
    var along = hit.entry + step * 0.5;
    for (var index = 0u; index < steps; index += 1u) {
        let point = camera + ray * along;
        let density = cloud_density(point);
        if density > 0.0 {
            optical += density * step;
            scattered += density * sun_transmittance(point) * step;
        }
        along += step;
    }

    let alpha = 1.0 - exp(-optical * params.density);
    if alpha < 0.002 {
        discard;
    }
    let light = scattered / max(optical, 1e-6);
    let shade = mix(0.35, 1.05, clamp(light, 0.0, 1.0));
    return vec4<f32>(params.tint.rgb * shade * phase * alpha, alpha);
}
