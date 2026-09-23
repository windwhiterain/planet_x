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

// ⚠⚠ **径向律：参数空间里线性、世界空间里等比**（2026-09-25 用户口径，与 CPU 的
//   `px_volume_schema::volume::Shell` 是**同一条**）：`r = inner·(outer/inner)^u`。
//   角向格子的世界尺寸是 `r·Δθ`（∝ r），径向也 ∝ r ⇒ 格子在每个半径上是同一个形状
//   （线性径向在 r = 3 处给出 3:1 的"饼"）。
//   ⚠ 采样 O(1)：层号 = `floor(u·layers)`，非线性只活在下面这一对函数里。
//   改这里必须同时改 CPU 那一份 —— 两侧不一致的症状是"层错位"（画面上一圈圈台阶）。
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

// ------------------------------- R3 星场（统一稀疏格）------------------------------
//
// ⚠⚠ 五段索引**拼成一个** storage buffer（`chunk_start ‖ brick_slot ‖ brick_mask ‖
//   brick_sub ‖ sub_start`），段起点走 uniform：`sky_radiance` 这个入口已经有 5 个
//   storage 绑定（体数据 / 图 / 星表 / 索引 / 溢出计数），把五段拆成五个绑定就是 9 个 ——
//   越过 WebGPU 的 `maxStorageBuffersPerShaderStage = 8`（默认下限）。症状是"某些设备上
//   这个入口直接起不来"，而错误信息指不到"是星场那几个绑定"。
//
// ⚠ 逐条语义的真源是 `px_sparse::grid`（`GridMeta::decompose` / `Grid::brick_at` /
//   `Grid::cell_range`）：位移分解（BRICK = 8、CHUNK = 4）、"空则子全空"（块区间长度 0、
//   表项 EMPTY）、掩码位 + popcount 排名 → 子 CSR 区间。改那边必须改这里。
struct StarMeta {
    // (星数, dims.x, dims.y, dims.z)
    counts: vec4<u32>,
    // (细格边长, origin.x, origin.y, origin.z)
    space: vec4<f32>,
    // 五段索引里的前四段起点：chunk_start, brick_slot, brick_mask, brick_sub
    segments_a: vec4<u32>,
    // (sub_start 起点, 逐星阴影步数, 每体素最多吃几颗星, 未用)
    segments_b: vec4<u32>,
    // 星光球（发射那一档）：(查询半径, 软化半径, 增益, 未用)
    light: vec4<f32>,
    // 星点轮廓（天空那一档）：(核, 晕, 晕权重, 支持域)
    profile: vec4<f32>,
};

// 星表：每颗 `StarField::STRIDE = 8` 个 f32 —— `[x, y, z, 亮度, r, g, b, 未用]`。
@group(0) @binding(6) var<storage, read> star_table: array<f32>;
// 五段索引拼成的那一段（u32）。
@group(0) @binding(10) var<storage, read> star_index: array<u32>;
@group(0) @binding(11) var<uniform> star_meta: StarMeta;
// ⚠⚠ 溢出计数：WGSL 没有变长数组，天空那一档的待消费队列是**定长**的。真溢出时静默丢星是
//   最坏的错法（画面上少几颗，归因不到）⇒ 让它落在一个计数器上、由宿主当场报 Err。
@group(0) @binding(12) var<storage, read_write> star_overflow: array<atomic<u32>>;

// ⚠ 这几个常数就是 `px_sparse::grid` 的那几个（`BRICK` / `CHUNK` / `CHUNK_CELLS` /
//   `MASK_WORDS`）：细格 → (块, brick, 局部) 是移位与掩码，不是除法。
//   ⚠⚠ 块那一维写**除法**（`/ STAR_CHUNK_CELLS`）而不是字面移位：常数是 2 的幂 ⇒ 编译器
//   给出的就是移位，而手写 `>> 5` 只要数错一次就没有任何编译错 —— 实测写成 `>> 4`
//   （把 `CHUNK_CELLS` 当成 16）让块键在每个轴上都少一半，症状不是"报错"而是
//   **一颗星都查不到**：块区间读到别的块、`start == end` ⇒ 整块被判空。
const STAR_BRICK: u32 = 8u;
const STAR_CHUNK: u32 = 4u;
const STAR_CHUNK_CELLS: u32 = 32u;
const STAR_MASK_WORDS: u32 = 16u;
const STAR_EMPTY: u32 = 0xffffffffu;
// ⚠ 就是 `f32::EPSILON`（CPU 那一侧判"星落在原点"用的那个数）。
const STAR_EPSILON: f32 = 1.1920929e-7;

fn star_dims() -> vec3<i32> {
    return vec3<i32>(i32(star_meta.counts.y), i32(star_meta.counts.z), i32(star_meta.counts.w));
}

// 细格坐标 → (块键, brick 的块内局部键, 细格在 brick 内的局部键)。⚠ 调用前必须已判界。
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

// 细格坐标 → brick 下标（越界 / 空块 / 空 brick 都回 -1）。
// ⚠ "空则子全空"：块区间长度 0 ⇒ 整块跳过；brick 表项是 EMPTY ⇒ 整块跳过。
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

// 一个 brick 里某个细格的星区间。
// ⚠ 空细格回 `(0, 0)`：占用细格必然有 ≥ 1 项 ⇒ `start == end` 只可能是空（与 CPU 回
//   `None` 是同一件事），调用方的循环于是自然一次都不进。
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
    // ⚠⚠ `brick_sub[brick]` 是**读出来的值**（那个 brick 在子 CSR 里的起点），
    //   不是"brick_sub 这一段的下标" —— 写成 `segments_a.w + brick + rank` 就把起点
    //   当成了下标：症状是区间落在**别的 brick** 上（星数对不上，而且不报错）。
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

// 取 vec3 的第 i 个分量（显式分支：动态 vec 下标在语言间有差异，不碰它）。
fn pick3(v: vec3<f32>, i: u32) -> f32 {
    if (i == 0u) { return v.x; }
    if (i == 1u) { return v.y; }
    return v.z;
}

// 一颗星的角向轮廓（核 + 晕），自变量是 `sin θ`。
// ⚠ 与 CPU 的 `star_power` **同序**：先 `(sine/core)²` 再 `exp(-·)`，最后
//   `核项 + 晕权重 × 晕项`；两个 `max(1e-6)` 已经在宿主上夹进 uniform 了。
fn star_power(sine: f32) -> f32 {
    let core = sine / star_meta.profile.x;
    let halo = sine / star_meta.profile.y;
    return exp(-(core * core)) + star_meta.profile.z * exp(-(halo * halo));
}

// 星光球那一档的两张定长表（见 `star_light`）：`star_cand` 记**遍历次序**的前几名、
// `star_kept` 记稳定降序的前几名。
// ⚠⚠ 两套并存不是冗余：`brightest_near` **只在候选多于 keep 时**才排序 ⇒ 候选不多时
//   用的是**遍历次序**，而逐通道的求和是浮点加法 ⇒ 次序不同就不是逐位相同。
const STAR_KEEP_MAX: u32 = 32u;
var<private> star_cand: array<u32, STAR_KEEP_MAX + 1u>;
var<private> star_kept: array<u32, STAR_KEEP_MAX>;
var<private> star_cand_count: u32;
var<private> star_kept_count: u32;

// 把一颗星插进"亮度降序"的前 `keep` 名：插在第一个**严格更暗**的位置 ⇒ 亮度并列时后到的
// 排在后面（与 CPU 那次**稳定** `sort_by` + `truncate` 逐条同一规则）。
//
// ⚠⚠ `keep` 是**入参**（`starlight_max`），不是数组容量 `STAR_KEEP_MAX`：拿容量当上限
//   会让"前 2 名"变成"前 32 名"，症状是**两边亮度差几十倍**（实测 2849 对 70）——
//   而它看起来像"星光太亮"，不像"截断写错了"。
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

// ------------------------------- 步进入口 -------------------------------
// binding 4/5：与采样入口的 0..3 分开（同一模块里同一号只能有一种类型）。
// 0/1（体积 uniform 与体数据）两个入口同类型，直接复用。

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

// 待消费队列：CPU 那一侧把整条视线的候选**一次收齐**、按 (半径, sin) 排好再按半径消费；
// GPU 这一侧边走边收（候选表按视线放不下），但**次序逐条相同**：
//   * 层（半径的归属区间）按升序处理 ⇒ 后收的星半径只会更大 ⇒ 直接接在队尾；
//   * 层内按 CPU 的遍历次序稳定插入 (半径, sin) ⇒ 与 CPU 那次**稳定**排序逐位同序。
const STAR_PENDING_MAX: u32 = 32u;
var<private> star_pending_star: array<u32, STAR_PENDING_MAX>;
var<private> star_pending_radius: array<f32, STAR_PENDING_MAX>;
var<private> star_pending_sine: array<f32, STAR_PENDING_MAX>;
var<private> star_pending_count: u32;

// ⚠ `mark` = 这一层开始收之前的队尾：新星只在 `[mark, count)` 这一段里找插入位
//   （队列前面是更近的层，半径必然更小 ⇒ 队尾那一截才是本层）。
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

// 一层（半径 ∈ `[t0, t1)`）里**支持域内**的星收进队列。
// ⚠ 层的 AABB 与 CPU 的 `gather_stars` 逐字相同（层里**最远**半径上的支持域再放一格 ——
//   格是方的、锥是圆的），逐格下探的次序也相同（z → y → x，两端夹回格内）⇒ 层内次序逐条同。
fn star_slab_collect(direction: vec3<f32>, t0: f32, t1: f32, support: f32) {
    let mark = star_pending_count;
    let cell = star_meta.space.x;
    let origin = star_meta.space.yzw;
    let half = max(t1 * support + cell, cell);
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
                    // ⚠ 半径的**归属**：一颗星只属于它自己那一层（这一条是"每颗只收一次"的全部）。
                    if (radius < t0 || radius >= t1 || radius <= STAR_EPSILON) {
                        continue;
                    }
                    let cosine = dot(p, direction) / radius;
                    let sine = sqrt(max(1.0 - cosine * cosine, 0.0));
                    if (sine > support) {
                        continue;
                    }
                    star_pending_insert(mark, star, radius, sine);
                }
            }
        }
    }
}

// 一条视线的**辐射**（一条通道）：步进 + 按半径消费星候选 + 底色。
// ⚠ 与 `px_volume_alg::raymarch::march_channel` 逐条对齐：
//   * 星摆在**透过率更新之后**（"这一步之前的吸收算完了，星的光从那里过来"）；
//   * `T < 1e-4` 时 CPU 把游标推到末尾 ⇒ 剩下的星**整批丢掉**（不是用 `T` 兜底）；
//   * 走完全程时，剩下的星（半径超出最后一个采样点的、**以及还没收到的那几层**）
//     才用最后的透过率兜底。
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
    // ⚠⚠ **步长在参数空间里固定**（`u` 均匀 ⇒ 世界等比，与 CPU 同一口径）。
    //   世界长度每步都要算（光学深度是世界的量），而它与抖动无关 ⇒ 期望值不变。
    let du = 1.0 / f32(steps);
    for (var i = 0u; i < steps; i = i + 1u) {
        // ⚠ 采样点取格子中点（`offset = 0.5`）：与 CPU 的 `jitter = 0` 那一档逐字一致
        //   （抖动是 CPU 那一侧的另一档，这里不掺 —— 掺了就不是同一条视线了）。
        let here = (f32(i) + 0.5) * du;
        let distance = shell_radius(here);
        let step = shell_radius(f32(i + 1u) * du) - shell_radius(f32(i) * du);
        if (star_on) {
            // 半径不超过这一步的层全部收下来（多收无害：消费那一条按半径判）。
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
            // ⚠ 每颗星用它**自己那一步**的透过率：近处的星不被整层气遮住、远处的被前面的气吃掉。
            var taken = 0u;
            while (taken < star_pending_count && star_pending_radius[taken] <= distance) {
                let star = star_pending_star[taken];
                let power = star_brightness(star) * pick3(star_tint(star), lane);
                radiance = radiance + ((transmittance * power) * star_power(star_pending_sine[taken])) * gain;
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
    }
    if (star_on && !stopped) {
        while (slab <= outer) {
            star_slab_collect(direction, slab, slab + cell, support);
            slab = slab + cell;
        }
        for (var i = 0u; i < star_pending_count; i = i + 1u) {
            let star = star_pending_star[i];
            let power = star_brightness(star) * pick3(star_tint(star), lane);
            radiance = radiance + ((transmittance * power) * star_power(star_pending_sine[i])) * gain;
        }
    }
    return radiance + transmittance * background;
}

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
    image[index] = march_radiance(direction, lane, steps, enter, pick3(sky.background.xyz, lane));
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

// 整条天空的**辐射**（未分级）：三条通道各积一遍（入口名避开模块级的 sky uniform）（逐通道消光不同 => 透过率也不同），再分级。
// ⚠ 三条通道各自**重收一遍**星候选（候选是按通道的透过率消费的）—— CPU 那一侧同样是
//   `raymarch_sky` 调三遍 `raymarch_channel`，每一遍自己 gather 一次。
// image 每格 3 个 f32（分级就地覆盖）。
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

// 分级（就地）：先亮度响应（tone(l)/l 保色相），后色相斜坡。
// ⚠ 锚点由宿主在烘焙时**量出来**（见 bin_luma）：档位常数在输出域量与响应之后对齐，
//   顺序反了整片色偏。
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

// 亮度直方图（对数分箱）：宿主据此在**烘焙时**量出响应曲线的输入锚点 ——
// 锚点的定义就是"本次烘焙的输入分位 -> 参考的输出分位"，量出来才与分辨率解耦。
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

// ======================= 发射烘焙（逐体素，compute）=======================
//
// ⚠ 为什么搬这里：它是体积链上**最贵**的一处（每体素一次 `shadow_steps` 步的阴影行进），
//   而且逐体素完全独立 —— 没有比它更像 compute 的东西。
// ⚠ 星光那一档（`starlight_*`）是**同一类量**（与看它的视线无关）⇒ 也在这里按体素算完：
//   算式与 CPU 的"星光照气体"那一段逐条对齐。

struct EmissionUniform {
    params: vec4<f32>,      // (light_radius_ratio, shadow_gain, emission_power, emission_gain)
    light: vec4<f32>,       // (方向 xyz, 未用)
    glow: vec4<f32>,        // (glow_gain, glow_power, glow_threshold, 未用)
    glow_tint: vec4<f32>,
    extinction: vec4<f32>,  // (r, g, b, extinction_power)
    dust: vec4<f32>,        // (dust_bias, dust_threshold, 未用, 未用)
    counts: vec4<u32>,      // (res, layers, shadow_steps, 未用)
};

@group(0) @binding(8) var<uniform> emission: EmissionUniform;
@group(0) @binding(9) var<storage, read_write> emitted: array<f32>;

fn emission_world(point: vec3<f32>) -> f32 {
    return max(sample_volume(point, 0u), 0.0);
}

// 一颗星落在这一点上的**辐照 × 它自己的色**：朝星走 `steps` 步算光深，再
// `exp(-τ × shadow_gain) × 亮度 / (d² + soft²)`。常数与 CPU 那一趟同一个来路
// （`soft2` 在宿主上按 `starlight_soft.max(1e-4)²` 夹好）。
fn star_visible(position: vec3<f32>, star: u32, soft2: f32, steps: u32) -> vec3<f32> {
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
    let visible = exp(-tau * emission.params.y) * falloff;
    return visible * star_tint(star);
}

// **星光照气体**：球查询（半径 `starlight_radius`）里亮度前 `starlight_max` 颗 + 逐星遮挡。
// ⚠⚠ 与 CPU 的 `brightest_near` 逐条对齐，包括那条最容易漏的次序规则：CPU **只在候选多于
//   `keep` 时**才排序 ⇒ 候选不多时用的是**遍历次序**；而逐通道的求和是浮点加法
//   ⇒ 次序不同就不是逐位相同。所以这里两套表并存（见上面的注释）。
fn star_light(position: vec3<f32>) -> vec3<f32> {
    var lit = vec3<f32>(0.0, 0.0, 0.0);
    let radius = star_meta.light.x;
    if (star_meta.counts.x == 0u || star_meta.light.z <= 0.0 || radius <= 0.0) {
        return lit;
    }
    // ⚠ 宿主已经把 `starlight_max > STAR_KEEP_MAX` 挡在派发之前；这里再夹一次纯粹是
    //   为了定长表的**下标**不会越界。
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
                        // `keep = 0` = CPU 那一侧的"不封顶"：不排序 ⇒ 直接按遍历次序累加。
                        lit = lit + star_visible(position, star, soft2, steps);
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
            lit = lit + star_visible(position, star_cand[i], soft2, steps);
        }
    } else {
        for (var i = 0u; i < star_kept_count; i = i + 1u) {
            lit = lit + star_visible(position, star_kept[i], soft2, steps);
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

    // ---- 单方向光的遮挡 ----
    let light_direction = normalize(emission.light.xyz);
    let light_radius = shell_radius(clamp(emission.params.x, 0.0, 1.0));
    let steps = emission.counts.z;
    let total = max(outer - light_radius, 1e-4);
    let step = total / f32(steps);
    var optical_depth = 0.0;
    for (var i = 1u; i <= steps; i = i + 1u) {
        let probe = position + light_direction * (f32(i) * step);
        optical_depth = optical_depth + emission_world(probe) * step;
    }
    let lit = exp(-optical_depth * emission.params.y);

    // ---- 星光照气体：R3 星场里附近最亮的几颗 + 逐星遮挡 ----
    let star_lit = star_light(position);

    let main = pow(d, emission.params.z) * emission.params.w * lit;
    let above = max(d - emission.glow.z, 0.0);
    let glow = pow(above, emission.glow.y) * emission.glow.x * lit;
    // 星光那一笔与主发射**同形状**（只在有气的地方亮），颜色走星自己的色温。
    let star_emit = pow(d, emission.params.z) * star_meta.light.z;
    let base = pow(d, emission.extinction.w);
    let dust = max(d - emission.dust.y, 0.0) * emission.dust.x;

    let at = index * 6u;
    let tint = emission.glow_tint.xyz;
    emitted[at + 0u] = main + glow * tint.x + star_emit * star_lit.x;
    emitted[at + 1u] = main + glow * tint.y + star_emit * star_lit.y;
    emitted[at + 2u] = main + glow * tint.z + star_emit * star_lit.z;
    emitted[at + 3u] = base * emission.extinction.x + dust;
    emitted[at + 4u] = base * emission.extinction.y + dust;
    emitted[at + 5u] = base * emission.extinction.z + dust;
}
