#import planet_x::common::{SUN_DIRECTION, shell_thickness}
#import planet_x::noise::{fbm_3, fbm_3_grad, rotate_vector}
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
    bump: f32,
    seed: u32,
    ablate: u32,
    slope_scale: f32,
    taper: f32,
    coverage_gain: f32,
};

const ABLATE_NONE: u32 = 0u;
const ABLATE_SUN: u32 = 1u;
const ABLATE_NOISE: u32 = 2u;
const ABLATE_FETCH: u32 = 3u;
const ABLATE_DETAIL: u32 = 4u;
const ABLATE_SURFACE: u32 = 5u;
const ABLATE_NORMALS: u32 = 6u;
const ABLATE_ANALYTIC: u32 = 7u;
const SHADOW_GAIN: f32 = 4.0;
const SURFACE_STEPS: u32 = 56;
const SURFACE_LEVEL: f32 = 0.20;
const SURFACE_EPSILON: f32 = 0.05;

struct Medium {
    direction: vec3<f32>,
    altitude: f32,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> params: CloudParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var coverage_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var coverage_sampler: sampler;

fn to_local(point: vec3<f32>) -> vec3<f32> {
    return rotate_vector(vec4<f32>(-params.orientation.xyz, params.orientation.w), point);
}

fn to_world(vector: vec3<f32>) -> vec3<f32> {
    return rotate_vector(params.orientation, vector);
}

fn span() -> f32 {
    return max(params.outer - params.inner, 1e-5);
}

fn medium_of(point: vec3<f32>) -> Medium {
    let local = to_local(point);
    let radius = length(local);
    return Medium(local / max(radius, 1e-5), (radius - params.inner) / span());
}

fn coverage_of(direction: vec3<f32>) -> f32 {
    if params.ablate == ABLATE_FETCH {
        return 0.45;
    }
    let mask = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0).r;
    return smoothstep(
        0.0,
        0.45,
        clamp((mask - params.coverage) / max(1.0 - params.coverage, 1e-4), 0.0, 1.0),
    );
}

fn billows(direction: vec3<f32>, altitude: f32, with_skin: bool) -> f32 {
    if params.ablate == ABLATE_NOISE {
        return 0.55;
    }
    if params.ablate == ABLATE_ANALYTIC {
        return sampled_noise(direction, altitude, params.detail_scale, 1u, params.seed);
    }
    let tower = sampled_noise(direction, altitude, params.detail_scale * 0.35, 3u, params.seed);
    if !with_skin {
        return tower;
    }
    let skin = sampled_noise(
        direction,
        altitude,
        params.detail_scale * 1.70,
        2u,
        params.seed ^ 31u,
    );
    return clamp(tower * 0.62 + skin * 0.38, 0.0, 1.0);
}

fn sampled_noise(
    direction: vec3<f32>,
    altitude: f32,
    across: f32,
    octaves: u32,
    seed: u32,
) -> f32 {
    let along = across * span();
    return fbm_3(direction * (across + altitude * along), 1.0, octaves, 2.0, 0.5, seed);
}

fn shape_of(cover: f32, altitude: f32, noise: f32) -> f32 {
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let bias = footprint + noise - 1.0;
    let lobed = clamp(bias * params.coverage_gain, 0.0, 1.0);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, altitude);
    let shape = clamp(floor_here * under_top * lobed, 0.0, 1.0);
    return clamp((shape - params.erode) / max(1.0 - params.erode, 1e-4), 0.0, 1.0);
}

fn density_of(medium: Medium, cover: f32, with_skin: bool) -> f32 {
    return shape_of(cover, medium.altitude, billows(medium.direction, medium.altitude, with_skin));
}

fn cloud_field(point: vec3<f32>) -> f32 {
    let medium = medium_of(point);
    if medium.altitude < 0.0 || medium.altitude > 1.0 {
        return 0.0;
    }
    let cover = coverage_of(medium.direction);
    if cover <= 0.0 {
        return 0.0;
    }
    return density_of(medium, cover, true);
}

fn cloud_field_gradient(point: vec3<f32>) -> vec3<f32> {
    let step = SURFACE_EPSILON * span();
    let x = vec3<f32>(step, 0.0, 0.0);
    let y = vec3<f32>(0.0, step, 0.0);
    let z = vec3<f32>(0.0, 0.0, step);
    let scale = 1.0 / (2.0 * step);
    return vec3<f32>(
        cloud_field(point + x) - cloud_field(point - x),
        cloud_field(point + y) - cloud_field(point - y),
        cloud_field(point + z) - cloud_field(point - z),
    ) * scale;
}

fn gate_open(value: f32) -> f32 {
    return select(0.0, 1.0, value > 0.0 && value < 1.0);
}

fn slope_of_smoothstep(low: f32, high: f32, value: f32) -> f32 {
    let width = max(high - low, 1e-6);
    let t = clamp((value - low) / width, 0.0, 1.0);
    return select(0.0, 6.0 * t * (1.0 - t) / width, t > 0.0 && t < 1.0);
}

fn project_tangential(vector: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    return vector - dot(direction, vector) * direction;
}

struct ShapePartials {
    cover: f32,
    altitude: f32,
    noise: f32,
};

fn shape_of_partials(cover: f32, altitude: f32, noise: f32) -> ShapePartials {
    let height = clamp(altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let bias = footprint + noise - 1.0;
    let lobed = clamp(bias * params.coverage_gain, 0.0, 1.0);
    let floor_here = smoothstep(0.0, max(params.base, 1e-3), altitude);
    let ceiling = max(
        params.top * mix(1.0 - params.detail_strength, 1.0, noise),
        params.base + 0.02,
    );
    let under_top = 1.0 - smoothstep(ceiling, ceiling + 0.20, altitude);
    let raw = floor_here * under_top * lobed;
    let shape = clamp(raw, 0.0, 1.0);
    let erode_room = max(1.0 - params.erode, 1e-4);
    let live = gate_open(shape);
    let footprint_live = select(0.0, 1.0, footprint > 0.0);
    let lobe_live = gate_open(lobed);
    let under_live = gate_open(under_top);
    let ceiling_live = select(
        0.0,
        params.top * params.detail_strength,
        ceiling > params.base + 0.02,
    );

    let tall = floor_here * under_top * params.coverage_gain * footprint_live;
    let cover_partial = live * lobe_live * tall / erode_room;
    let shape_altitude = select(
        0.0,
        under_top * lobed * slope_of_smoothstep(0.0, max(params.base, 1e-3), altitude),
        altitude > 0.0 && altitude < max(params.base, 1e-3),
    ) - floor_here * lobed * slope_of_smoothstep(ceiling, ceiling + 0.20, altitude)
        - floor_here * under_top * params.coverage_gain * params.taper * 2.0 * height
            * footprint_live * lobe_live;
    let altitude_partial = live * shape_altitude / erode_room;
    let shape_noise = floor_here
        * (
            under_top * params.coverage_gain * lobe_live
                + lobed * ceiling_live * slope_of_smoothstep(ceiling, ceiling + 0.20, altitude)
        );
    let noise_partial = live * shape_noise / erode_room;

    return ShapePartials(
        select(0.0, cover_partial, footprint_live > 0.0),
        altitude_partial,
        noise_partial,
    );
}

fn sampled_noise_along(direction: vec3<f32>, across: f32, octaves: u32, seed: u32, altitude: f32) -> NoiseSample {
    return fbm_3_grad(
        direction * (across + altitude * across * span()),
        octaves,
        2.0,
        0.5,
        seed,
    );
}

fn billows_along(direction: vec3<f32>, altitude: f32, with_skin: bool) -> vec4<f32> {
    if params.ablate == ABLATE_NOISE {
        return vec4<f32>(0.55, vec3<f32>(0.0));
    }
    let radius = params.inner + altitude * span();
    if params.ablate == ABLATE_ANALYTIC {
        let single_across = params.detail_scale;
        let single = sampled_noise_along(direction, single_across, 1u, params.seed, altitude);
        let single_live = gate_open(single.value);
        let single_stretch = single_across + altitude * single_across * span();
        let single_tangential =
            single_live * single_stretch * project_tangential(single.gradient, direction);
        let single_radial = single_live * radius * single_across * dot(direction, single.gradient);
        return vec4<f32>(clamp(single.value, 0.0, 1.0), single_tangential + single_radial * direction);
    }
    let tower_across = params.detail_scale * 0.35;
    let tower = sampled_noise_along(direction, tower_across, 3u, params.seed, altitude);
    let tower_stretch = tower_across + altitude * tower_across * span();
    if !with_skin {
        let live = gate_open(tower.value);
        let tangential = live * tower_stretch * project_tangential(tower.gradient, direction);
        let radial = live * radius * tower_across * dot(direction, tower.gradient);
        return vec4<f32>(tower.value, tangential + radial * direction);
    }
    let skin_across = params.detail_scale * 1.70;
    let skin = sampled_noise_along(direction, skin_across, 2u, params.seed ^ 31u, altitude);
    let skin_stretch = skin_across + altitude * skin_across * span();
    let blended = tower.value * 0.62 + skin.value * 0.38;
    let live = gate_open(blended);
    let tangential = live
        * (tower_stretch * project_tangential(tower.gradient, direction) * 0.62
            + skin_stretch * project_tangential(skin.gradient, direction) * 0.38);
    let radial = live
        * radius
        * (tower_across * dot(direction, tower.gradient) * 0.62
            + skin_across * dot(direction, skin.gradient) * 0.38);
    return vec4<f32>(clamp(blended, 0.0, 1.0), tangential + radial * direction);
}

fn coverage_gradient_of(direction: vec3<f32>) -> vec4<f32> {
    if params.ablate == ABLATE_FETCH {
        return vec4<f32>(0.45, 0.0, 0.0, 0.0);
    }
    let baked = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0);
    let normalized = clamp(
        (baked.r - params.coverage) / max(1.0 - params.coverage, 1e-4),
        0.0,
        1.0,
    );
    let cover = smoothstep(0.0, 0.45, normalized);
    let slope = select(0.0, 6.0 * normalized * (1.0 - normalized) / 0.45, normalized > 0.0 && normalized < 1.0);
    return vec4<f32>(
        cover,
        slope * baked.g / max(1.0 - params.coverage, 1e-4) * project_tangential(vec3<f32>(baked.g, baked.b, baked.a), direction),
    );
}

fn kink_margin(medium: Medium, direction: vec3<f32>) -> f32 {
    var margin = min(medium.altitude, 1.0 - medium.altitude);
    if params.ablate == ABLATE_FETCH {
        return margin;
    }
    let baked = textureSampleLevel(coverage_map, coverage_sampler, direction, 0.0).r;
    margin = min(margin, abs(baked - params.coverage) / max(1.0 - params.coverage, 1e-4) / 0.45);
    if params.ablate == ABLATE_NOISE {
        return margin;
    }
    let cover = coverage_of(direction);
    let noise = billows(direction, medium.altitude, true);
    let height = clamp(medium.altitude, 0.0, 1.0);
    let footprint = max(cover - params.taper * height * height, 0.0);
    let lobed = clamp((footprint + noise - 1.0) * params.coverage_gain, 0.0, 1.0);
    margin = min(margin, footprint / max(params.coverage_gain, 1e-4));
    margin = min(margin, min(lobed, 1.0 - lobed) / params.coverage_gain);
    let shape = shape_of(cover, medium.altitude, noise);
    return min(margin, min(shape, 1.0 - shape) * max(1.0 - params.erode, 1e-4));
}

fn cloud_field_gradient_analytic(point: vec3<f32>) -> vec3<f32> {
    let local = to_local(point);
    let medium = medium_of(point);
    let altitude = medium.altitude;
    if !(altitude >= 0.0 && altitude <= 1.0) {
        return vec3<f32>(0.0);
    }
    let direction = medium.direction;
    let cover = coverage_gradient_of(direction);
    if cover.r <= 0.0 {
        return vec3<f32>(0.0);
    }
    let radius = max(length(local), 1e-5);
    let billow = billows_along(direction, altitude, true);
    let partials = shape_of_partials(cover.r, altitude, billow.x);
    let altitude_axis = direction / span();
    let cover_axis = project_tangential(cover.gba, direction);
    let noise_axis = project_tangential(billow.yzw, direction)
        + dot(direction, billow.yzw) * direction;
    let gradient = partials.altitude * altitude_axis
        + (partials.noise / radius) * noise_axis
        + (partials.cover / radius) * cover_axis;
    return to_world(gradient);
}

fn sun_shadow(point: vec3<f32>, reach: f32) -> f32 {
    if reach <= 0.0 || params.ablate == ABLATE_SUN {
        return 1.0;
    }
    let sun = normalize(SUN_DIRECTION);
    let cover = coverage_of(medium_of(point + sun * reach * 0.5).direction);
    let depth = cover * params.shadow * SHADOW_GAIN;
    let thin = exp(-depth);
    let middle = exp(-depth * 0.5);
    let thick = exp(-depth * 0.25);
    return thin * 0.35 + middle * 0.35 + thick * 0.30;
}

fn detail_shading(direction: vec3<f32>) -> f32 {
    if params.ablate == ABLATE_DETAIL || params.bump <= 0.0 {
        return 1.0;
    }
    let sample = fbm_3_grad(direction * params.detail_scale * 0.35, 3u, 2.0, 0.5, params.seed);
    let up = direction - sample.gradient * params.bump;
    let length_squared = dot(up, up);
    if length_squared <= 1e-8 {
        return 1.0;
    }
    let normal = to_world(up * inverseSqrt(length_squared));
    return mix(0.45, 1.0, clamp(dot(normal, normalize(SUN_DIRECTION)) + 0.5, 0.0, 1.0));
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
    if params.ablate == ABLATE_NORMALS || params.ablate == ABLATE_SURFACE {
        let camera = view.world_position.xyz;
        let away = in.world_position.xyz - camera;
        let distance = length(away);
        if distance <= 1e-5 {
            discard;
        }
        let ray = away / distance;
        let shell = shell_thickness(camera, ray, params.inner, params.outer);
        if !shell.valid || shell.exit <= shell.entry {
            discard;
        }

        let stride = (shell.exit - shell.entry) / f32(SURFACE_STEPS);
        var along = shell.entry;
        var found = false;
        var surface_point = camera + ray * shell.entry;
        for (var index = 0u; index < SURFACE_STEPS; index += 1u) {
            let point = camera + ray * along;
            if cloud_field(point) > SURFACE_LEVEL {
                surface_point = point;
                found = true;
                break;
            }
            along += stride;
        }
        if !found {
            discard;
        }

        let gradient = cloud_field_gradient(surface_point);
        let length_squared = dot(gradient, gradient);
        if length_squared <= 1e-14 {
            discard;
        }
        let normal = -gradient * inverseSqrt(length_squared);
        if params.ablate == ABLATE_NORMALS {
            return vec4<f32>(normal * 0.5 + vec3<f32>(0.5), 1.0);
        }
        let sun = normalize(SUN_DIRECTION);
        let lit = clamp(dot(normal, sun), 0.0, 1.0);
        return vec4<f32>(params.tint.rgb * (0.14 + 0.86 * lit), 1.0);
    }

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

    let ceiling_steps = f32(max(params.steps, 16u));
    let stride = max(span() * 0.045, 1e-5);
    let steps = u32(clamp(chord / stride, 16.0, ceiling_steps));
    let step = chord / f32(steps);

    let sun = normalize(SUN_DIRECTION);
    let middle = camera + ray * (hit.entry + chord * 0.5);
    let reach = shell_thickness(middle, sun, params.inner, params.outer);
    let sun_reach = select(0.0, reach.exit, reach.valid && reach.exit > 0.0);
    let opaque = 6.0 / max(params.density, 1e-4);

    let phase = 0.60 + 0.40 * phase_hg(dot(-ray, sun), params.phase) / phase_forward(params.phase);
    let detail = detail_shading(medium_of(camera + ray * hit.entry).direction);

    var optical = 0.0;
    var scattered = 0.0;
    var along = hit.entry + step * 0.5;
    for (var index = 0u; index < steps; index += 1u) {
        let point = camera + ray * along;
        along += step;
        let medium = medium_of(point);
        if medium.altitude < 0.0 || medium.altitude > 1.0 {
            continue;
        }
        let cover = coverage_of(medium.direction);
        if cover <= 0.0 {
            continue;
        }
        let density = density_of(medium, cover, true);
        if density <= 0.0 {
            continue;
        }
        optical += density * step;
        scattered += density * sun_shadow(point, sun_reach) * step;
        if optical > opaque {
            break;
        }
    }

    let alpha = 1.0 - exp(-optical * params.density);
    if alpha < 0.002 {
        discard;
    }
    let light = scattered / max(optical, 1e-6);
    let shade = mix(0.35, 1.05, clamp(light, 0.0, 1.0));
    return vec4<f32>(params.tint.rgb * shade * detail * phase * alpha, alpha);
}
