// 天空体的**跨面三线性采样**（GPU 侧）。语义与 `px_volume_alg::raymarch::sample_at`
// 逐条对齐 —— 真源在那边，改那边必须改这里；规格见 `lib.rs` 的"移植规格"一节。
//
// ⚠⚠ 八个角**必须逐个按方向反查相邻面**（`corner_slot`）：从前 CPU 那份把八个角困在本面里
//   （wrap + 面号写死）⇒ 视线跨过面棱时采样值跳一下 ⇒ 立方贴图上那道通高的竖缝。
//   搬 GPU 时这一步是**同一个坑**。

struct Volume {
    // res, layers, lanes, lane —— 只读参数用 vec4 对齐（uniform 的 16 字节规矩）。
    shape: vec4<u32>,
    // inner, outer, 未用, 未用
    extent: vec4<f32>,
};

@group(0) @binding(0) var<uniform> volume: Volume;
@group(0) @binding(1) var<storage, read> data: array<f32>;
@group(0) @binding(2) var<storage, read> points: array<f32>;
@group(0) @binding(3) var<storage, read_write> out: array<f32>;

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

// 方向 → (面, s, t)。面序与 `cube_direction` 的表一致。
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

// 一个角在 data 里的**起始下标**（含通道步长，但还没加 lane）。
// ⚠ 这里就是"跨面"那一步：格心方向反查相邻面，再落到那一面的格子上。
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

fn sample_volume(point: vec3<f32>, lane: u32) -> f32 {
    let res = volume.shape.x;
    let layers = volume.shape.y;
    let inner = volume.extent.x;
    let outer = volume.extent.y;
    let radius = sqrt(dot(point, point));
    let span = outer - inner;
    // ⚠ 边界**闭 + 相对容差**（与 CPU 同一口径）：体网格第一层/最后一层正落在 inner/outer 上，
    //   而 f32 上那个和会偏 1e-7 ⇒ 严格比较会把壳的两壁读成 0。
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
    let altitude = clamp((radius - inner) / span, 0.0, 1.0);
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
    let count = arrayLength(&out);
    if (id.x >= count) {
        return;
    }
    let point = vec3<f32>(points[id.x * 3u], points[id.x * 3u + 1u], points[id.x * 3u + 2u]);
    out[id.x] = sample_volume(point, volume.shape.w);
}
