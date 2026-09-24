// NURBS 曲面在规则格点上的求值：位置 + 法线。
//
// 语义逐条对齐 `px_nurbs_schema`（CPU 那一侧是真源，改那边必须改这里）：
//
//   * `span_of` ↔ `knot::span`：最后一个 ≤ t 的节点；**右端点取最后一段**
//     （用 `degree` 那一段算出来的基函数全是 0 ⇒ 端点会求出原点）；
//   * `basis_of` ↔ `knot::basis`：逐层 Cox-de Boor。第 0 层那一格在局部第 `degree` 个
//     （窗口是 `[span−degree, span]`，右端点在闭区间上就是最后一格）；
//     值那一层用**上一层的值**，导数那一层也是用**上一层的值**乘 `level` 的系数；
//   * 混合与商法则 ↔ `Surface::patch`：`A00/A10/A01` 都是齐次量的加权和，
//     除权得到点与两个偏导，法线是 `∂u × ∂v` 归一化。
//
// ⚠ 次数上限 `MAXD` 是**编译期**的：WGSL 不许变长数组、也不许从函数返回数组。
//   超了在 Rust 那一侧当场拒（`MAX_DEGREE`），不会静默算错。
// ⚠ 宽度是 `MAXD + 2`：多出来的那一格是**恒为 0 的哨兵**，这样 `previous[j + 1]`
//   在 `j = degree` 上不必特判（CPU 那边是靠"行比窗口宽"达到同一件事）。

const MAXD: u32 = 8u;
const WIDTH: u32 = 10u;

struct Shape {
    nu: u32,
    nv: u32,
    p: u32,
    q: u32,
    out_u: u32,
    out_v: u32,
    u0: f32,
    v0: f32,
    u_step: f32,
    v_step: f32,
    u_offset: f32,
    v_offset: f32,
};

@group(0) @binding(0) var<uniform> shape: Shape;
// 齐次控制点：`(w·x, w·y, w·z, w)` 四格一个，行主序 `u * nv + v`。
@group(0) @binding(1) var<storage, read> hom: array<f32>;
@group(0) @binding(2) var<storage, read> knots_u: array<f32>;
@group(0) @binding(3) var<storage, read> knots_v: array<f32>;
@group(0) @binding(4) var<storage, read_write> positions: array<f32>;
@group(0) @binding(5) var<storage, read_write> normals: array<f32>;

fn knot_at(use_u: bool, index: u32) -> f32 {
    if (use_u) {
        return knots_u[index];
    }
    return knots_v[index];
}

// 参数 `t` 落在哪一段。
fn span_of(use_u: bool, degree: u32, count: u32, t: f32) -> u32 {
    if (t >= knot_at(use_u, count)) {
        return count - 1u;
    }
    if (t <= knot_at(use_u, degree)) {
        return degree;
    }
    var span = degree;
    for (var index = degree; index <= count - 1u; index = index + 1u) {
        if (knot_at(use_u, index) <= t) {
            span = index;
        }
    }
    return span;
}

// 那一层基函数（`values`）与它的一阶导（`slopes`），窗口 `[span−degree, span]` 按局部下标。
fn basis_of(
    use_u: bool,
    t: f32,
    span: u32,
    degree: u32,
    values: ptr<function, array<f32, 10>>,
    slopes: ptr<function, array<f32, 10>>,
) {
    for (var index = 0u; index < WIDTH; index = index + 1u) {
        (*values)[index] = 0.0;
        (*slopes)[index] = 0.0;
    }
    (*values)[degree] = 1.0;
    var previous: array<f32, 10>;
    for (var index = 0u; index < WIDTH; index = index + 1u) {
        previous[index] = (*values)[index];
    }
    let base = span - degree;
    for (var level = 1u; level <= degree; level = level + 1u) {
        for (var j = 0u; j <= degree; j = j + 1u) {
            let index = base + j;
            let low = knot_at(use_u, index);
            let high = knot_at(use_u, index + level);
            let low_next = knot_at(use_u, index + 1u);
            let high_next = knot_at(use_u, index + level + 1u);
            let left = select(0.0, 1.0 / (high - low), abs(high - low) > 1e-12);
            let right = select(
                0.0,
                1.0 / (high_next - low_next),
                abs(high_next - low_next) > 1e-12,
            );
            (*values)[j] = left * (t - low) * previous[j]
                + right * (high_next - t) * previous[j + 1u];
            (*slopes)[j] = f32(level) * (left * previous[j] - right * previous[j + 1u]);
        }
        for (var index = 0u; index < WIDTH; index = index + 1u) {
            previous[index] = (*values)[index];
        }
    }
}

@compute @workgroup_size(64)
fn surface_grid(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= shape.out_u * shape.out_v) {
        return;
    }
    // ⚠ **行主序 `i · out_v + j`**（`i` 沿 u、是外层）：装配那一侧
    //   （`px_nurbs_schema::mesh::grid_mesh`）用的正是这一条。反过来写（`j · out_u + i`）
    //   在正方形的格子上不会报错，只会把 uv 与顶点对调 —— 实测：17×17 的球面格点与
    //   CPU 对账最大偏差 4（半径是 2）。
    let i = id.x / shape.out_v;
    let j = id.x % shape.out_v;
    let u = shape.u0 + shape.u_step * (f32(i) + shape.u_offset);
    let v = shape.v0 + shape.v_step * (f32(j) + shape.v_offset);

    let span_u = span_of(true, shape.p, shape.nu, u);
    let span_v = span_of(false, shape.q, shape.nv, v);
    var values_u: array<f32, 10>;
    var slopes_u: array<f32, 10>;
    var values_v: array<f32, 10>;
    var slopes_v: array<f32, 10>;
    basis_of(true, u, span_u, shape.p, &values_u, &slopes_u);
    basis_of(false, v, span_v, shape.q, &values_v, &slopes_v);

    let base_u = span_u - shape.p;
    let base_v = span_v - shape.q;
    var a00 = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var a10 = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    var a01 = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    for (var a = 0u; a <= shape.p; a = a + 1u) {
        for (var b = 0u; b <= shape.q; b = b + 1u) {
            let at = ((base_u + a) * shape.nv + (base_v + b)) * 4u;
            let point = vec4<f32>(hom[at], hom[at + 1u], hom[at + 2u], hom[at + 3u]);
            a00 = a00 + (values_u[a] * values_v[b]) * point;
            a10 = a10 + (slopes_u[a] * values_v[b]) * point;
            a01 = a01 + (values_u[a] * slopes_v[b]) * point;
        }
    }

    let w = a00.w;
    let point = a00.xyz / w;
    let along_u = (a10.xyz - a10.w * point) / w;
    let along_v = (a01.xyz - a01.w * point) / w;
    let crossed = cross(along_u, along_v);
    let length_ = length(crossed);
    let normal = select(vec3<f32>(0.0, 1.0, 0.0), crossed / length_, length_ > 1e-12);

    let out = id.x * 3u;
    positions[out] = point.x;
    positions[out + 1u] = point.y;
    positions[out + 2u] = point.z;
    normals[out] = normal.x;
    normals[out + 1u] = normal.y;
    normals[out + 2u] = normal.z;
}
