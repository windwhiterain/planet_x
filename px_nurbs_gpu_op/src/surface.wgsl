
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
