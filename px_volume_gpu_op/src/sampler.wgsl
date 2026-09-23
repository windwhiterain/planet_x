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


// ⚠⚠ 一维派发有个**硬顶**：`max_compute_workgroups_per_dimension = 65535`（WebGPU 规范常数）。
//   天穹 face 1024 要 6 * 1024^2 / 64 = 98304 个工作组 ⇒ 一维派发必然越界（实测：烘图报
//   wgpu 校验错，panic 穿过算子的 dylib 边界 ⇒ 整个烘焙进程 abort，且没有任何可读信息）。
//   ⇒ x 方向切在 65535，余数走 y 维；这里按同一把尺子把 (x, y) 摊平。
const WG_X: u32 = 65535u;
const WG_SIZE: u32 = 64u;

fn flat_index_of(id: vec3<u32>) -> u32 {
    return id.x + id.y * WG_X * WG_SIZE;
}

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
    let index = flat_index_of(id);
    let count = arrayLength(&out);
    if (index >= count) {
        return;
    }
    let point = vec3<f32>(points[index * 3u], points[index * 3u + 1u], points[index * 3u + 2u]);
    out[index] = sample_volume(point, volume.shape.w);
}

// ------------------------------- 步进入口 -------------------------------
// binding 4/5：与采样入口的 0..3 分开（同一模块里同一号只能有一种类型）。
// 0/1（体积 uniform 与体数据）两个入口同类型，直接复用。

struct Sky {
    counts: vec4<u32>,        // (steps, face, channel, 未用)
    scalars: vec4<f32>,       // (jitter, star_gain, star_floor, enter)
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

// 单通道步进：每条视线从 enter 走到 outer，中点取样的黎曼和。
// 布局与星点查表同一套：row = 面 * face_size + y。
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
    let outer = volume.extent.y;
    let h = (outer - enter) / f32(steps);
    var transmittance = 1.0;
    var radiance = 0.0;
    for (var i = 0u; i < steps; i = i + 1u) {
        let distance = enter + (f32(i) + 0.5) * h;
        let point = direction * distance;
        let emit = sample_volume(point, lane);
        let sigma = sample_volume(point, lane + 3u);
        radiance = radiance + transmittance * emit * h;
        transmittance = transmittance * exp(-sigma * h);
    }
    // 星点与背景：乘透射率 —— 被前面的气遮住、被尘埃染红（与 CPU 同一口径）。
    radiance = radiance + transmittance * star_level(direction) * sky.scalars.y;
    radiance = radiance + transmittance * channel_of(sky.background, lane);
    image[index] = radiance;
}

// 星点：方向 -> 立方贴图格 -> 扣地板并归一化。face_size = 0 表示没有星图。
// 取法与 CPU 的 star_level 逐条对齐（含 min 截断与 height = face_size * 6）。
@group(0) @binding(6) var<storage, read> stars: array<f32>;

fn channel_of(v: vec4<f32>, lane: u32) -> f32 {
    if (lane == 0u) { return v.x; }
    if (lane == 1u) { return v.y; }
    if (lane == 2u) { return v.z; }
    return v.w;
}

fn star_level(direction: vec3<f32>) -> f32 {
    let face_size = sky.counts.w;
    if (face_size == 0u) {
        return 0.0;
    }
    let mapped = cube_face_of(direction);
    let face = u32(mapped.x);
    let x = min(u32(mapped.y * f32(face_size)), face_size - 1u);
    let height = face_size * 6u;
    let y = min(face * face_size + u32(mapped.z * f32(face_size)), height - 1u);
    let raw = stars[y * face_size + x];
    let floor_value = sky.scalars.z;
    if (raw <= floor_value) {
        return 0.0;
    }
    return (raw - floor_value) / max(1.0 - floor_value, 1e-4);
}

// 取 vec4 的第 i 个分量（显式分支：动态 vec 下标在语言间有差异，不碰它）。
fn pick(v: vec4<f32>, i: u32) -> f32 {
    if (i == 0u) { return v.x; }
    if (i == 1u) { return v.y; }
    if (i == 2u) { return v.z; }
    return v.w;
}

// 响应曲线：log-log 分段线性（锚点间插值，两端按最外一段的斜率幂外推）+ 指数软肩。
// 与 CPU 的 tone 逐条对齐；三个常量（锚点 x2、肩/上限）全部走 uniform，不在这里复制。
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

// 档位色相：按档位键的亮度取段；段间过渡**收窄到段间的 20%**（线性混色会让大量像素停在
// 混色带上 —— 暖沙到蓝河的中点就是"薰衣草"）。与 CPU 的 ramp_hue 逐条对齐。
// 档位表走 uniform（ramp_luma / ramp_hue_0..3），WGSL 里不复制。
// 注意：WGSL 的保留字比 Rust 多 —— 第一版这里用 base_hue 之前叫过 from，直接编不过。
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

// 每格一次：键就是这一格自己的亮度（没有邻域平均 —— 烘焙离线，判据直接来自采样本身）。
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

// 每格一次分级：先亮度响应、后色相斜坡，逐格亮度守恒；纯黑原样出去。
// 与 CPU 的 grade_pixel + raymarch_sky 的装配逐条对齐。
fn luma_of(rgb: vec3<f32>) -> f32 {
    return rgb.x * 0.2126 + rgb.y * 0.7152 + rgb.z * 0.0722;
}

fn grade_pixel(rgb: vec3<f32>) -> vec3<f32> {
    let measured = luma_of(rgb);
    if (measured <= 1e-6) {
        return rgb;
    }
    // 注意：WGSL 的保留字比 Rust 多 —— target / from / to 这些都不能当标识符。
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

// 整条天空：三条通道各积一遍（入口名避开模块级的 sky uniform）（逐通道消光不同 => 透过率也不同），再分级。
// image 每格 3 个 f32（分级就地覆盖）。
@compute @workgroup_size(64)
fn sky_grade(@builtin(global_invocation_id) id: vec3<u32>) {
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
    let outer = volume.extent.y;
    let h = (outer - enter) / f32(steps);
    let star = star_level(direction) * sky.scalars.y;

    var rgb = vec3<f32>(0.0, 0.0, 0.0);
    for (var c = 0u; c < 3u; c = c + 1u) {
        var transmittance = 1.0;
        var radiance = 0.0;
        for (var i = 0u; i < steps; i = i + 1u) {
            let point = direction * (enter + (f32(i) + 0.5) * h);
            let emit = sample_volume(point, c);
            let sigma = sample_volume(point, c + 3u);
            radiance = radiance + transmittance * emit * h;
            transmittance = transmittance * exp(-sigma * h);
        }
        // 星点与底色：乘透射率（被前面的气遮住、被尘埃染红），与 CPU 同一口径。
        let channel_background = channel_of(sky.background, c);
        radiance = radiance + transmittance * star + transmittance * channel_background;
        if (c == 0u) { rgb.x = max(radiance, 0.0); }
        if (c == 1u) { rgb.y = max(radiance, 0.0); }
        if (c == 2u) { rgb.z = max(radiance, 0.0); }
    }

    // 先亮度响应（tone(l)/l 保色相），后色相斜坡（档位常数在输出域量的，顺序不能反）。
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
