
struct Volume {
    shape: vec4<u32>,
    extent: vec4<f32>,
};


const WG_X: u32 = 65535u;
const WG_SIZE: u32 = 64u;

fn flat_index_of(id: vec3<u32>) -> u32 {
    return id.x + id.y * WG_X * WG_SIZE;
}

struct Occupancy {
    extra: vec4<u32>,
    scalars: vec4<f32>,
};

@group(0) @binding(0) var<uniform> volume: Volume;
@group(0) @binding(1) var<storage, read> data: array<f32>;
@group(0) @binding(2) var<storage, read> points: array<f32>;
@group(0) @binding(3) var<storage, read_write> out: array<f32>;
@group(0) @binding(13) var<uniform> occupancy: Occupancy;
@group(0) @binding(14) var<storage, read> occupancy_words: array<u32>;

fn cube_direction(face: u32, s: f32, t: f32) -> vec3<f32> {
    let a = s * 2.0 - 1.0;
    let b = t * 2.0 - 1.0;
    var d = vec3<f32>(1.0, -b, -a);
    switch face % 6u {
        case 0u: { d = vec3<f32>(1.0, -b, -a); }
        case 1u: { d = vec3<f32>(-1.0, -b, a); }
        case 2u: { d = vec3<f32>(a, 1.0, b); }
        case 3u: { d = vec3<f32>(a, -1.0, -b); }
        case 4u: { d = vec3<f32>(a, -b, 1.0); }
        default: { d = vec3<f32>(-a, -b, -1.0); }
    }
    let length = sqrt(dot(d, d));
    if (length <= 1e-7) {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return d / length;
}

fn cube_face_of(d: vec3<f32>) -> vec3<f32> {
    let ax = abs(d.x);
    let ay = abs(d.y);
    let az = abs(d.z);
    var face = 5u;
    if (ax >= ay && ax >= az) {
        face = select(1u, 0u, d.x > 0.0);
    } else if (ay >= az) {
        face = select(3u, 2u, d.y > 0.0);
    } else {
        face = select(5u, 4u, d.z > 0.0);
    }
    var major = az;
    if (face == 0u || face == 1u) {
        major = ax;
    } else if (face == 2u || face == 3u) {
        major = ay;
    }
    major = max(major, 1e-7);
    var a = -d.x;
    var b = -d.y;
    switch face {
        case 0u: { a = -d.z; b = -d.y; }
        case 1u: { a = d.z; b = -d.y; }
        case 2u: { a = d.x; b = d.z; }
        case 3u: { a = d.x; b = -d.z; }
        case 4u: { a = d.x; b = -d.y; }
        default: { a = -d.x; b = -d.y; }
    }
    let s = clamp((a / major) * 0.5 + 0.5, 0.0, 1.0);
    let t = clamp((b / major) * 0.5 + 0.5, 0.0, 1.0);
    return vec3<f32>(f32(face), s, t);
}

fn snap(fraction: f32) -> f32 {
    if (fraction < 1e-4) {
        return 0.0;
    }
    if (fraction > 1.0 - 1e-4) {
        return 1.0;
    }
    return fraction;
}

fn corner_slot(face: u32, cell_s: f32, cell_t: f32, layer: u32, res: u32, layers: u32) -> u32 {
    let s = (cell_s + 0.5) / f32(res);
    let t = (cell_t + 0.5) / f32(res);
    let mapped = cube_face_of(cube_direction(face, s, t));
    let nf = u32(mapped.x);
    let cs = min(u32(mapped.y * f32(res)), res - 1u);
    let ct = min(u32(mapped.z * f32(res)), res - 1u);
    return (((nf * layers + min(layer, layers - 1u)) * res + ct) * res + cs) * volume.shape.z;
}

fn gather_at(corners: array<u32, 8>, weights: array<f32, 8>, lane: u32) -> f32 {
    var total = 0.0;
    for (var i = 0u; i < 8u; i = i + 1u) {
        total = total + weights[i] * data[corners[i] + lane];
    }
    return total;
}

const COARSE: u32 = 8u;
const FINE: u32 = 2u;
const FINE_PER_AXIS: u32 = 4u;
const WORDS_PER_BLOCK: u32 = FINE_PER_AXIS * FINE_PER_AXIS * FINE_PER_AXIS / 32u;

fn occupancy_bit(word: u32, bit: u32) -> bool {
    return (word & (1u << bit)) != 0u;
}

fn layer_index(radius: f32) -> u32 {
    let layers = max(volume.shape.y, 1u);
    let altitude = shell_altitude(radius);
    return min(u32(altitude * f32(layers - 1u)), layers - 1u);
}

fn layer_radius(cl: u32) -> f32 {
    let layers = max(volume.shape.y, 2u);
    return shell_radius(f32(min(cl, layers - 1u)) / f32(layers - 1u));
}

fn grid_coords_of(direction: vec3<f32>) -> vec3<f32> {
    let ax = abs(direction.x);
    let ay = abs(direction.y);
    let az = abs(direction.z);
    var face = 5u;
    var major = az;
    var a = -direction.x;
    var b = -direction.y;
    if (ax >= ay && ax >= az) {
        major = ax;
        if (direction.x > 0.0) {
            face = 0u;
            a = -direction.z;
            b = -direction.y;
        } else {
            face = 1u;
            a = direction.z;
            b = -direction.y;
        }
    } else if (ay >= az) {
        major = ay;
        if (direction.y > 0.0) {
            face = 2u;
            a = direction.x;
            b = direction.z;
        } else {
            face = 3u;
            a = direction.x;
            b = -direction.z;
        }
    } else if (direction.z > 0.0) {
        face = 4u;
        a = direction.x;
        b = -direction.y;
    }
    major = max(major, 1e-30);
    let u = clamp((a / major) * 0.5 + 0.5, 0.0, 1.0);
    let v = clamp((b / major) * 0.5 + 0.5, 0.0, 1.0);
    return vec3<f32>(f32(face), u, v);
}

fn occupancy_class(point: vec3<f32>) -> u32 {
    if (occupancy.extra.w == 0u) {
        return 2u;
    }
    let res = occupancy.extra.x;
    let layers = occupancy.extra.y;
    let radius = sqrt(dot(point, point));
    let direction = point / max(radius, 1e-30);
    let mapped = grid_coords_of(direction);
    let face = u32(mapped.x);
    let cs = min(u32(mapped.y * f32(res)), res - 1u);
    let ct = min(u32(mapped.z * f32(res)), res - 1u);
    let cl = layer_index(radius);
    let blocks_s = (res + COARSE - 1u) / COARSE;
    let blocks_l = (layers + COARSE - 1u) / COARSE;
    let blocks_t = (res + COARSE - 1u) / COARSE;
    let bs = min(cs / COARSE, blocks_s - 1u);
    let bl = min(cl / COARSE, blocks_l - 1u);
    let bt = min(ct / COARSE, blocks_t - 1u);
    let packed = face * occupancy.extra.z + (bt * blocks_l + bl) * blocks_s + bs;
    let l1_word = occupancy_words[packed / 32u];
    if (!occupancy_bit(l1_word, packed % 32u)) {
        return 0u;
    }
    let sub = ((cl % COARSE) / FINE) * FINE_PER_AXIS * FINE_PER_AXIS
        + ((ct % COARSE) / FINE) * FINE_PER_AXIS
        + ((cs % COARSE) / FINE);
    let base = u32(occupancy.scalars.z) + packed * WORDS_PER_BLOCK + sub / 32u;
    return select(1u, 2u, occupancy_bit(occupancy_words[base], sub % 32u));
}

fn shell_radius(u: f32) -> f32 {
    let inner = volume.extent.x;
    let outer = volume.extent.y;
    if (inner <= 0.0) {
        return inner + (outer - inner) * clamp(u, 0.0, 1.0);
    }
    return inner * pow(outer / inner, clamp(u, 0.0, 1.0));
}

fn shell_altitude(radius: f32) -> f32 {
    let inner = volume.extent.x;
    let outer = volume.extent.y;
    if (inner <= 0.0) {
        let span = outer - inner;
        if (abs(span) <= 1e-30) {
            return 0.0;
        }
        return clamp((radius - inner) / span, 0.0, 1.0);
    }
    let ratio = log(outer / inner);
    if (abs(ratio) <= 1e-30) {
        return 0.0;
    }
    return clamp(log(max(radius, 1e-30) / inner) / ratio, 0.0, 1.0);
}

fn sample_volume(point: vec3<f32>, lane: u32) -> f32 {
    let res = volume.shape.x;
    let layers = volume.shape.y;
    let inner = volume.extent.x;
    let outer = volume.extent.y;
    let radius = sqrt(dot(point, point));
    let span = outer - inner;
    let tolerance = max(abs(span), 1.0) * 1e-5;
    if (radius < inner - tolerance || radius > outer + tolerance || abs(span) <= 1e-7) {
        return 0.0;
    }
    let direction = point / radius;
    let mapped = cube_face_of(direction);
    let face = u32(mapped.x);
    let s = mapped.y;
    let t = mapped.z;

    let last_layer = layers - 1u;
    let altitude = shell_altitude(radius);
    let sz = altitude * f32(last_layer);
    let nearest = round(sz);
    var layer0 = floor(sz);
    if (abs(sz - nearest) < 1e-3) {
        layer0 = nearest;
    }
    let tz = snap(sz - layer0);
    let sx = s * f32(res) - 0.5;
    let sy = t * f32(res) - 0.5;
    let x0 = floor(sx);
    let y0 = floor(sy);
    let tx = snap(sx - x0);
    let ty = snap(sy - y0);
    let la = u32(clamp(layer0, 0.0, f32(last_layer)));
    let lb = u32(clamp(layer0 + 1.0, 0.0, f32(last_layer)));

    var corners: array<u32, 8>;
    corners[0] = corner_slot(face, x0, y0, la, res, layers);
    corners[1] = corner_slot(face, x0 + 1.0, y0, la, res, layers);
    corners[2] = corner_slot(face, x0, y0 + 1.0, la, res, layers);
    corners[3] = corner_slot(face, x0 + 1.0, y0 + 1.0, la, res, layers);
    corners[4] = corner_slot(face, x0, y0, lb, res, layers);
    corners[5] = corner_slot(face, x0 + 1.0, y0, lb, res, layers);
    corners[6] = corner_slot(face, x0, y0 + 1.0, lb, res, layers);
    corners[7] = corner_slot(face, x0 + 1.0, y0 + 1.0, lb, res, layers);

    let wx = 1.0 - tx;
    let wy = 1.0 - ty;
    let wz = 1.0 - tz;
    let weights = array<f32, 8>(
        wx * wy * wz,
        tx * wy * wz,
        wx * ty * wz,
        tx * ty * wz,
        wx * wy * tz,
        tx * wy * tz,
        wx * ty * tz,
        tx * ty * tz,
    );
    return gather_at(corners, weights, lane);
}

@compute @workgroup_size(64)
fn sample_points(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let count = arrayLength(&out);
    if (index >= count) {
        return;
    }
    let point = vec3<f32>(points[index * 3u], points[index * 3u + 1u], points[index * 3u + 2u]);
    out[index] = sample_volume(point, volume.shape.w);
}

struct StarMeta {
    counts: vec4<u32>,
    space: vec4<f32>,
    segments_a: vec4<u32>,
    segments_b: vec4<u32>,
    light: vec4<f32>,
    profile: vec4<f32>,
};

@group(0) @binding(6) var<storage, read> star_table: array<f32>;
@group(0) @binding(10) var<storage, read> star_index: array<u32>;
@group(0) @binding(11) var<uniform> star_meta: StarMeta;
@group(0) @binding(12) var<storage, read_write> star_overflow: array<atomic<u32>>;

const STAR_BRICK: u32 = 8u;
const STAR_CHUNK: u32 = 4u;
const STAR_CHUNK_CELLS: u32 = 32u;
const STAR_MASK_WORDS: u32 = 16u;
const STAR_EMPTY: u32 = 0xffffffffu;
const STAR_EPSILON: f32 = 1.1920929e-7;

fn star_dims() -> vec3<i32> {
    return vec3<i32>(i32(star_meta.counts.y), i32(star_meta.counts.z), i32(star_meta.counts.w));
}

fn star_decompose(cell: vec3<i32>) -> vec3<u32> {
    let v = vec3<u32>(cell);
    let chunk_dims = vec3<u32>(
        star_meta.counts.y / STAR_CHUNK_CELLS,
        star_meta.counts.z / STAR_CHUNK_CELLS,
        star_meta.counts.w / STAR_CHUNK_CELLS,
    );
    return vec3<u32>(
        (((v.z / STAR_CHUNK_CELLS) * chunk_dims.y + (v.y / STAR_CHUNK_CELLS)) * chunk_dims.x)
            + (v.x / STAR_CHUNK_CELLS),
        ((((v.z >> 3u) & 3u) * STAR_CHUNK + ((v.y >> 3u) & 3u)) * STAR_CHUNK) + ((v.x >> 3u) & 3u),
        (((v.z & 7u) * STAR_BRICK + (v.y & 7u)) * STAR_BRICK) + (v.x & 7u),
    );
}

fn star_brick_at(cell: vec3<i32>) -> i32 {
    if (any(cell < vec3<i32>(0)) || any(cell >= star_dims())) {
        return -1;
    }
    let key = star_decompose(cell);
    let start = star_index[star_meta.segments_a.x + key.x];
    let end = star_index[star_meta.segments_a.x + key.x + 1u];
    if (start == end) {
        return -1;
    }
    let slot = star_index[star_meta.segments_a.y + start + key.y];
    if (slot == STAR_EMPTY) {
        return -1;
    }
    return i32(slot);
}

fn star_cell_range(brick: u32, local: u32) -> vec2<u32> {
    let base = star_meta.segments_a.z + brick * STAR_MASK_WORDS;
    let word = local / 32u;
    let flag = 1u << (local % 32u);
    let bits = star_index[base + word];
    if ((bits & flag) == 0u) {
        return vec2<u32>(0u, 0u);
    }
    var rank = 0u;
    for (var i = 0u; i < word; i = i + 1u) {
        rank = rank + countOneBits(star_index[base + i]);
    }
    rank = rank + countOneBits(bits & (flag - 1u));
    let sub = star_index[star_meta.segments_a.w + brick];
    let at = sub + rank;
    return vec2<u32>(
        star_index[star_meta.segments_b.x + at],
        star_index[star_meta.segments_b.x + at + 1u],
    );
}

fn star_position(star: u32) -> vec3<f32> {
    let at = star * 8u;
    return vec3<f32>(star_table[at], star_table[at + 1u], star_table[at + 2u]);
}

fn star_brightness(star: u32) -> f32 {
    return star_table[star * 8u + 3u];
}

fn star_tint(star: u32) -> vec3<f32> {
    let at = star * 8u + 4u;
    return vec3<f32>(star_table[at], star_table[at + 1u], star_table[at + 2u]);
}

fn pick3(v: vec3<f32>, i: u32) -> f32 {
    if (i == 0u) { return v.x; }
    if (i == 1u) { return v.y; }
    return v.z;
}

fn star_power(sine: f32, distance: f32) -> f32 {
    let offset = sine * max(distance, 1e-4);
    let core = offset / star_meta.profile.x;
    let halo = offset / star_meta.profile.y;
    return exp(-(core * core)) + star_meta.profile.z * exp(-(halo * halo));
}

const STAR_KEEP_MAX: u32 = 32u;
var<private> star_cand: array<u32, STAR_KEEP_MAX + 1u>;
var<private> star_kept: array<u32, STAR_KEEP_MAX>;
var<private> star_cand_count: u32;
var<private> star_kept_count: u32;

fn star_keep_insert(star: u32, keep: u32) {
    let brightness = star_brightness(star);
    var at = 0u;
    while (at < star_kept_count && star_brightness(star_kept[at]) >= brightness) {
        at = at + 1u;
    }
    if (at >= keep) {
        return;
    }
    var top = star_kept_count;
    if (top >= keep) {
        top = keep - 1u;
    }
    var i = top;
    while (i > at) {
        star_kept[i] = star_kept[i - 1u];
        i = i - 1u;
    }
    star_kept[at] = star;
    if (star_kept_count < keep) {
        star_kept_count = star_kept_count + 1u;
    }
}


struct Sky {
    counts: vec4<u32>,        // (steps, face, channel, 未用)
    scalars: vec4<f32>,       // (jitter, 未用, 未用, enter) —— 星点的旋钮走 star_meta
    background: vec4<f32>,
    tone_in: vec4<f32>,
    tone_out: vec4<f32>,
    ramp_luma: vec4<f32>,
    ramp_hue_0: vec4<f32>,
    ramp_hue_1: vec4<f32>,
    ramp_hue_2: vec4<f32>,
    ramp_hue_3: vec4<f32>,
    limits: vec4<f32>,        // (shoulder, ceil, 未用, 未用)
};

@group(0) @binding(4) var<uniform> sky: Sky;
@group(0) @binding(5) var<storage, read_write> image: array<f32>;

const STAR_PENDING_MAX: u32 = 64u;
var<private> star_pending_star: array<u32, STAR_PENDING_MAX>;
var<private> star_pending_radius: array<f32, STAR_PENDING_MAX>;
var<private> star_pending_sine: array<f32, STAR_PENDING_MAX>;
var<private> star_pending_count: u32;

fn star_pending_insert(mark: u32, star: u32, radius: f32, sine: f32) {
    if (star_pending_count >= STAR_PENDING_MAX) {
        atomicAdd(&star_overflow[0], 1u);
        return;
    }
    var at = mark;
    while (at < star_pending_count
        && (star_pending_radius[at] < radius
            || (star_pending_radius[at] == radius && star_pending_sine[at] <= sine))) {
        at = at + 1u;
    }
    var i = star_pending_count;
    while (i > at) {
        star_pending_radius[i] = star_pending_radius[i - 1u];
        star_pending_sine[i] = star_pending_sine[i - 1u];
        star_pending_star[i] = star_pending_star[i - 1u];
        i = i - 1u;
    }
    star_pending_radius[at] = radius;
    star_pending_sine[at] = sine;
    star_pending_star[at] = star;
    star_pending_count = star_pending_count + 1u;
}

fn star_slab_collect(direction: vec3<f32>, t0: f32, t1: f32, support: f32) {
    let mark = star_pending_count;
    let cell = star_meta.space.x;
    let origin = star_meta.space.yzw;
    let half = max(support + cell, cell);
    let angular = support / max(t1, 1e-4);
    let centre = direction * t1;
    let edge = star_dims() - vec3<i32>(1);
    let lo = clamp(vec3<i32>(floor((centre - half - origin) / cell)), vec3<i32>(0), edge);
    let hi = clamp(vec3<i32>(floor((centre + half - origin) / cell)), vec3<i32>(0), edge);
    for (var z = lo.z; z <= hi.z; z = z + 1i) {
        for (var y = lo.y; y <= hi.y; y = y + 1i) {
            for (var x = lo.x; x <= hi.x; x = x + 1i) {
                let at = vec3<i32>(x, y, z);
                let brick = star_brick_at(at);
                if (brick < 0) {
                    continue;
                }
                let range = star_cell_range(u32(brick), star_decompose(at).z);
                for (var star = range.x; star < range.y; star = star + 1u) {
                    let p = star_position(star);
                    let radius = length(p);
                    if (radius < t0 || radius >= t1 || radius <= STAR_EPSILON) {
                        continue;
                    }
                    let cosine = dot(p, direction) / radius;
                    let sine = sqrt(max(1.0 - cosine * cosine, 0.0));
                    if (sine > angular) {
                        continue;
                    }
                    star_pending_insert(mark, star, radius, sine);
                }
            }
        }
    }
}

fn march_radiance(direction: vec3<f32>, lane: u32, steps: u32, enter: f32, background: f32) -> f32 {
    let outer = volume.extent.y;
    let cell = star_meta.space.x;
    let gain = star_meta.light.z;
    let support = star_meta.profile.w;
    let star_on = star_meta.counts.x > 0u && gain > 0.0 && support > 0.0 && outer > enter;
    star_pending_count = 0u;
    var slab = max(enter - cell, 0.0);
    var transmittance = 1.0;
    var radiance = 0.0;
    var stopped = false;
    let du = 1.0 / f32(steps);
    var budget = steps * 16u;
    var current = 0u;
    let max_layer = volume.shape.y - 1u;
    let enter_layer = min(u32(shell_altitude(enter) * f32(max_layer) + 1e-6), max_layer);
    var radius = select(clamp(enter, shell_radius(0.0), shell_radius(1.0)), layer_radius(enter_layer), occupancy.extra.w != 0u);
    var i = 0u;
    if (occupancy.extra.w != 0u) {
        current = enter_layer;
    }
    loop {
        if (i >= steps || radius >= outer || transmittance < 1e-4) {
            if (transmittance < 1e-4) {
                stopped = true;
            }
            break;
        }
        var here = (f32(i) + 0.5) * du;
        var distance = shell_radius(here);
        var step = shell_radius(f32(i + 1u) * du) - shell_radius(f32(i) * du);
        var samples = 1u;
        var block = 0u;
        if (occupancy.extra.w != 0u) {
            block = current / COARSE;
            let state = occupancy_class(direction * radius);
            if (state == 0u) {
                let next_layer = min((block + 1u) * COARSE, max_layer);
                let boundary = clamp(layer_radius(next_layer), enter, outer);
                let past = boundary * (1.0 + 1e-5) + 1e-6;
                radius = max(past, radius + step);
                current = next_layer;
                continue;
            }
            let block_low = block * COARSE;
            let block_high = min(block_low + COARSE, max_layer);
            samples = clamp(block_high - block_low, 1u, 2048u);
            if (samples * 16u > budget) {
                samples = budget / 16u;
            }
            if (samples == 0u) {
                break; // 预算见底：与密集档"步数用完"同一种收尾（剩下的气不再积）。
            }
            budget = budget - samples * 16u;
            if (occupancy_class(direction * (layer_radius(block_low) + 0.5 * step)) == 1u) {
                i = i + samples;
                current = block_low + COARSE;
                continue;
            }
            var local = 0u;
            loop {
                if (local >= samples) {
                    i = i + samples;
                    current = block_low + COARSE;
                    break;
                }
                let layer = min(block_low + local, max_layer);
                let low = layer_radius(layer);
                let high = layer_radius(min(layer + 1u, max_layer));
                let sample_distance = (low + high) * 0.5;
                let layer_step = max(high - low, 1e-9);
                let sample_point = direction * sample_distance;
                let sample_emit = sample_volume(sample_point, lane);
                let sample_sigma = sample_volume(sample_point, lane + 3u);
                if (star_on) {
                    while (slab <= outer && slab <= sample_distance) {
                        star_slab_collect(direction, slab, slab + cell, support);
                        slab = slab + cell;
                    }
                }
                radiance = radiance + transmittance * sample_emit * layer_step;
                transmittance = transmittance * exp(-sample_sigma * layer_step);
                if (star_on) {
                    var taken = 0u;
                    while (taken < star_pending_count && star_pending_radius[taken] <= sample_distance) {
                        let star = star_pending_star[taken];
                        let power = star_brightness(star) * pick3(star_tint(star), lane);
                        let falloff = (enter / max(star_pending_radius[taken], 1e-4));
                        radiance = radiance + ((transmittance * power) * star_power(star_pending_sine[taken], star_pending_radius[taken])) * gain * (falloff * falloff);
                        taken = taken + 1u;
                    }
                    if (taken > 0u) {
                        var left = 0u;
                        while (taken + left < star_pending_count) {
                            star_pending_radius[left] = star_pending_radius[taken + left];
                            star_pending_sine[left] = star_pending_sine[taken + left];
                            star_pending_star[left] = star_pending_star[taken + left];
                            left = left + 1u;
                        }
                        star_pending_count = star_pending_count - taken;
                    }
                }
                if (transmittance < 1e-4) {
                    stopped = true;
                    break;
                }
                local = local + 1u;
            }
            if (stopped) {
                break;
            }
            continue;
        }
        if (star_on) {
            while (slab <= outer && slab <= distance) {
                star_slab_collect(direction, slab, slab + cell, support);
                slab = slab + cell;
            }
        }
        let point = direction * distance;
        let emit = sample_volume(point, lane);
        let sigma = sample_volume(point, lane + 3u);
        radiance = radiance + transmittance * emit * step;
        transmittance = transmittance * exp(-sigma * step);
        if (star_on) {
            var taken = 0u;
            while (taken < star_pending_count && star_pending_radius[taken] <= distance) {
                let star = star_pending_star[taken];
                let power = star_brightness(star) * pick3(star_tint(star), lane);
                let falloff = (enter / max(star_pending_radius[taken], 1e-4));
                radiance = radiance + ((transmittance * power) * star_power(star_pending_sine[taken], star_pending_radius[taken])) * gain * (falloff * falloff);
                taken = taken + 1u;
            }
            if (taken > 0u) {
                var left = 0u;
                while (taken + left < star_pending_count) {
                    star_pending_radius[left] = star_pending_radius[taken + left];
                    star_pending_sine[left] = star_pending_sine[taken + left];
                    star_pending_star[left] = star_pending_star[taken + left];
                    left = left + 1u;
                }
                star_pending_count = star_pending_count - taken;
            }
        }
        if (transmittance < 1e-4) {
            stopped = true;
            break;
        }
        i = i + 1u;
    }
    if (star_on && !stopped) {
        while (slab <= outer) {
            star_slab_collect(direction, slab, slab + cell, support);
            slab = slab + cell;
        }
        for (var i = 0u; i < star_pending_count; i = i + 1u) {
            let star = star_pending_star[i];
            let power = star_brightness(star) * pick3(star_tint(star), lane);
            let falloff = (enter / max(star_pending_radius[i], 1e-4));
            radiance = radiance + ((transmittance * power) * star_power(star_pending_sine[i], star_pending_radius[i])) * gain * (falloff * falloff);
        }
    }
    return radiance + transmittance * background;
}

@compute @workgroup_size(64)
fn march(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let count = arrayLength(&image);
    if (index >= count) {
        return;
    }
    let face_size = sky.counts.y;
    let row = index / face_size;
    let x = index % face_size;
    let face = row / face_size;
    let y = row % face_size;
    let s = (f32(x) + 0.5) / f32(face_size);
    let t = (f32(y) + 0.5) / f32(face_size);
    let direction = cube_direction(face, s, t);
    let steps = sky.counts.x;
    let lane = sky.counts.z;
    let enter = sky.scalars.w;
    image[index] = march_radiance(direction, lane, steps, enter, pick3(sky.background.xyz, lane));
}

fn pick(v: vec4<f32>, i: u32) -> f32 {
    if (i == 0u) { return v.x; }
    if (i == 1u) { return v.y; }
    if (i == 2u) { return v.z; }
    return v.w;
}

fn tone(l: f32, tone_in: vec4<f32>, tone_out: vec4<f32>) -> f32 {
    if (l <= 0.0) {
        return 0.0;
    }
    let x = log(l);
    var y = tone_out.x;
    if (l <= tone_in.x) {
        let slope = log(tone_out.y / tone_out.x) / log(tone_in.y / tone_in.x);
        y = exp(log(tone_out.x) + (x - log(tone_in.x)) * slope);
    } else if (l >= tone_in.w) {
        let slope = log(tone_out.w / tone_out.z) / log(tone_in.w / tone_in.z);
        y = exp(log(tone_out.w) + (x - log(tone_in.w)) * slope);
    } else {
        for (var stop = 0u; stop < 3u; stop = stop + 1u) {
            let hi = pick(tone_in, stop + 1u);
            if (l < hi) {
                let lo = pick(tone_in, stop);
                let lo_out = pick(tone_out, stop);
                let w = (x - log(lo)) / (log(hi) - log(lo));
                y = exp(log(lo_out) + w * log(pick(tone_out, stop + 1u) / lo_out));
                break;
            }
        }
    }
    let shoulder = sky.limits.x;
    if (y <= shoulder) {
        return y;
    }
    let span = sky.limits.y - shoulder;
    return shoulder + span * (1.0 - exp(-(y - shoulder) / span));
}

@compute @workgroup_size(64)
fn tone_of(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let count = arrayLength(&image);
    if (index >= count) {
        return;
    }
    image[index] = tone(image[index], sky.tone_in, sky.tone_out);
}

fn hue_of(stop: u32) -> vec3<f32> {
    if (stop == 0u) { return sky.ramp_hue_0.xyz; }
    if (stop == 1u) { return sky.ramp_hue_1.xyz; }
    if (stop == 2u) { return sky.ramp_hue_2.xyz; }
    return sky.ramp_hue_3.xyz;
}

fn ramp_hue(key: f32) -> vec3<f32> {
    if (key >= sky.ramp_luma.w) {
        return sky.ramp_hue_3.xyz;
    }
    for (var stop = 0u; stop < 3u; stop = stop + 1u) {
        let lo = pick(sky.ramp_luma, stop);
        let hi = pick(sky.ramp_luma, stop + 1u);
        if (key < hi) {
            let raw = clamp(log(key / lo) / log(hi / lo), 0.0, 1.0);
            let shaped = clamp((raw - 0.4) / 0.2, 0.0, 1.0);
            let w = shaped * shaped * (3.0 - 2.0 * shaped);
            let base_hue = hue_of(stop);
            let next_hue = hue_of(stop + 1u);
            return base_hue + (next_hue - base_hue) * w;
        }
    }
    return sky.ramp_hue_3.xyz;
}

@compute @workgroup_size(64)
fn hue_of_keys(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let count = arrayLength(&image) / 3u;
    if (index >= count) {
        return;
    }
    let hue = ramp_hue(image[index * 3u]);
    image[index * 3u] = hue.x;
    image[index * 3u + 1u] = hue.y;
    image[index * 3u + 2u] = hue.z;
}

fn luma_of(rgb: vec3<f32>) -> f32 {
    return rgb.x * 0.2126 + rgb.y * 0.7152 + rgb.z * 0.0722;
}

fn grade_pixel(rgb: vec3<f32>) -> vec3<f32> {
    let measured = luma_of(rgb);
    if (measured <= 1e-6) {
        return rgb;
    }
    let band_hue = ramp_hue(measured);
    let band_luma = luma_of(band_hue);
    let scale = measured / max(band_luma, 1e-9);
    let strength = sky.limits.z;
    var out = rgb;
    out.x = max(out.x + (band_hue.x * scale - rgb.x) * strength, 0.0);
    out.y = max(out.y + (band_hue.y * scale - rgb.y) * strength, 0.0);
    out.z = max(out.z + (band_hue.z * scale - rgb.z) * strength, 0.0);
    return out;
}

@compute @workgroup_size(64)
fn sky_radiance(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let count = arrayLength(&image) / 3u;
    if (index >= count) {
        return;
    }
    let face_size = sky.counts.y;
    let row = index / face_size;
    let x = index % face_size;
    let face = row / face_size;
    let y = row % face_size;
    let s = (f32(x) + 0.5) / f32(face_size);
    let t = (f32(y) + 0.5) / f32(face_size);
    let direction = cube_direction(face, s, t);
    let steps = sky.counts.x;
    let enter = sky.scalars.w;

    var rgb = vec3<f32>(0.0, 0.0, 0.0);
    for (var c = 0u; c < 3u; c = c + 1u) {
        let radiance = march_radiance(direction, c, steps, enter, pick3(sky.background.xyz, c));
        if (c == 0u) { rgb.x = max(radiance, 0.0); }
        if (c == 1u) { rgb.y = max(radiance, 0.0); }
        if (c == 2u) { rgb.z = max(radiance, 0.0); }
    }

    image[index * 3u] = rgb.x;
    image[index * 3u + 1u] = rgb.y;
    image[index * 3u + 2u] = rgb.z;
}

@compute @workgroup_size(64)
fn grade_pixels(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let count = arrayLength(&image) / 3u;
    if (index >= count) {
        return;
    }
    let rgb = vec3<f32>(image[index * 3u], image[index * 3u + 1u], image[index * 3u + 2u]);
    let l = luma_of(rgb);
    var response = 0.0;
    if (l > 1e-9) {
        response = tone(l, sky.tone_in, sky.tone_out) / l;
    }
    let graded = grade_pixel(rgb * response);
    image[index * 3u] = graded.x;
    image[index * 3u + 1u] = graded.y;
    image[index * 3u + 2u] = graded.z;
}

const BIN_COUNT: u32 = 512u;
const LOG_MIN: f32 = -16.0;
const LOG_MAX: f32 = 4.0;

@group(0) @binding(7) var<storage, read_write> histogram: array<atomic<u32>>;

@compute @workgroup_size(64)
fn bin_luma(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let count = arrayLength(&image) / 3u;
    if (index >= count) {
        return;
    }
    let l = luma_of(vec3<f32>(image[index * 3u], image[index * 3u + 1u], image[index * 3u + 2u]));
    if (l <= 0.0) {
        atomicAdd(&histogram[0u], 1u);
        return;
    }
    let position = (log2(l) - LOG_MIN) / (LOG_MAX - LOG_MIN) * f32(BIN_COUNT);
    let bin = u32(clamp(floor(position), 0.0, f32(BIN_COUNT) - 1.0));
    atomicAdd(&histogram[bin], 1u);
}


struct EmissionUniform {
    params: vec4<f32>,      // (light_radius_ratio, shadow_gain, emission_power, emission_gain)
    light: vec4<f32>,       // (方向 xyz, 未用)
    glow: vec4<f32>,        // (glow_gain, glow_power, glow_threshold, 未用)
    glow_tint: vec4<f32>,   // 散射的通道配比（用户 2026-09-25：散射走红）
    scatter_tint: vec4<f32>,// 分色诊断的配比（`[1,1,1]` = 不染色）
    extinction: vec4<f32>,  // (r, g, b, extinction_power)
    dust: vec4<f32>,        // (dust_bias, dust_threshold, 未用, 未用)
    counts: vec4<u32>,      // (res, layers, shadow_steps, 未用)
};

@group(0) @binding(8) var<uniform> emission: EmissionUniform;
@group(0) @binding(9) var<storage, read_write> emitted: array<f32>;

fn emission_world(point: vec3<f32>) -> f32 {
    return max(sample_volume(point, 0u), 0.0);
}

fn star_visible(position: vec3<f32>, star: u32, soft2: f32, steps: u32) -> f32 {
    let to_star = star_position(star) - position;
    let distance2 = dot(to_star, to_star);
    let distance = max(sqrt(distance2), 1e-4);
    let away = to_star / distance;
    let through = distance / f32(steps);
    var tau = 0.0;
    for (var i = 1u; i <= steps; i = i + 1u) {
        tau = tau + emission_world(position + away * (f32(i) * through)) * through;
    }
    let falloff = star_brightness(star) / (distance2 + soft2);
    return exp(-tau * emission.params.y) * falloff;
}

fn star_light(position: vec3<f32>) -> vec3<f32> {
    var lit = vec3<f32>(0.0, 0.0, 0.0);
    let radius = star_meta.light.x;
    if (star_meta.counts.x == 0u || star_meta.light.z <= 0.0 || radius <= 0.0) {
        return lit;
    }
    let keep = min(star_meta.segments_b.z, STAR_KEEP_MAX);
    let steps = max(star_meta.segments_b.y, 1u);
    let soft2 = star_meta.light.y * star_meta.light.y;
    let cell = star_meta.space.x;
    let origin = star_meta.space.yzw;
    let edge = star_dims() - vec3<i32>(1);
    let lo = clamp(vec3<i32>(floor((position - radius - origin) / cell)), vec3<i32>(0), edge);
    let hi = clamp(vec3<i32>(floor((position + radius - origin) / cell)), vec3<i32>(0), edge);
    star_cand_count = 0u;
    star_kept_count = 0u;
    for (var z = lo.z; z <= hi.z; z = z + 1i) {
        for (var y = lo.y; y <= hi.y; y = y + 1i) {
            for (var x = lo.x; x <= hi.x; x = x + 1i) {
                let at = vec3<i32>(x, y, z);
                let brick = star_brick_at(at);
                if (brick < 0) {
                    continue;
                }
                let range = star_cell_range(u32(brick), star_decompose(at).z);
                for (var star = range.x; star < range.y; star = star + 1u) {
                    let delta = star_position(star) - position;
                    if (dot(delta, delta) > radius * radius) {
                        continue;
                    }
                    if (keep == 0u) {
                        lit = lit + vec3<f32>(star_visible(position, star, soft2, steps));
                    } else {
                        if (star_cand_count <= keep) {
                            star_cand[star_cand_count] = star;
                        }
                        star_cand_count = star_cand_count + 1u;
                        star_keep_insert(star, keep);
                    }
                }
            }
        }
    }
    if (keep == 0u) {
        return lit;
    }
    if (star_cand_count <= keep) {
        for (var i = 0u; i < star_cand_count; i = i + 1u) {
            lit = lit + vec3<f32>(star_visible(position, star_cand[i], soft2, steps));
        }
    } else {
        for (var i = 0u; i < star_kept_count; i = i + 1u) {
            lit = lit + vec3<f32>(star_visible(position, star_kept[i], soft2, steps));
        }
    }
    return lit;
}

@compute @workgroup_size(64)
fn bake_emission(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = flat_index_of(id);
    let voxels = arrayLength(&emitted) / 6u;
    if (index >= voxels) {
        return;
    }
    let res = emission.counts.x;
    let layers = emission.counts.y;
    let s = index % res;
    let t = (index / res) % res;
    let layer = (index / (res * res)) % layers;
    let face = index / (res * res * layers);
    let inner = volume.extent.x;
    let outer = volume.extent.y;
    let span = outer - inner;
    let altitude = f32(layer) / f32(max(layers, 2u) - 1u);
    let radius = shell_radius(altitude);
    let direction = cube_direction(face, (f32(s) + 0.5) / f32(res), (f32(t) + 0.5) / f32(res));
    let position = direction * radius;
    let d = emission_world(position);

    let light_direction = normalize(emission.light.xyz);
    let light_radius = shell_radius(clamp(emission.params.x, 0.0, 1.0));
    let light_position = light_direction * light_radius;
    let to_light = light_position - position;
    let light_distance = max(length(to_light), 1e-4);
    let toward = to_light / light_distance;
    let steps = emission.counts.z;
    let through = light_distance / f32(steps);
    var optical_depth = 0.0;
    for (var i = 1u; i <= steps; i = i + 1u) {
        let probe = position + toward * (f32(i) * through);
        optical_depth = optical_depth + emission_world(probe) * through;
    }
    let reach = clamp(light_radius / light_distance, 0.0, 1.0);
    let lit = exp(-optical_depth * emission.params.y) * (reach * reach);

    let star_lit = star_light(position);

    let main = pow(d, emission.params.z) * emission.params.w * lit;
    let above = max(d - emission.glow.z, 0.0);
    let glow = pow(above, emission.glow.y) * emission.glow.x * lit;
    let star_emit = pow(d, emission.params.z) * star_meta.light.z;
    let base = pow(d, emission.extinction.w);
    let dust = max(d - emission.dust.y, 0.0) * emission.dust.x;

    let at = index * 6u;
    let tint = emission.glow_tint.xyz;
    let scatter = emission.scatter_tint.xyz;
    emitted[at + 0u] = (main + glow + star_emit * star_lit.x) * tint.x * scatter.x;
    emitted[at + 1u] = (main + glow + star_emit * star_lit.y) * tint.y * scatter.y;
    emitted[at + 2u] = (main + glow + star_emit * star_lit.z) * tint.z * scatter.z;
    emitted[at + 3u] = base * emission.extinction.x + dust;
    emitted[at + 4u] = base * emission.extinction.y + dust;
    emitted[at + 5u] = base * emission.extinction.z + dust;
}
