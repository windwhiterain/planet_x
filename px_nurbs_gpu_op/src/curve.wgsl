
const MAXD: u32 = 8u;
const WIDTH: u32 = 10u;

struct Shape {
    count: u32,
    p: u32,
    out_n: u32,
    u0: f32,
    u_step: f32,
    u_offset: f32,
};

@group(0) @binding(0) var<uniform> shape: Shape;
@group(0) @binding(1) var<storage, read> hom: array<f32>;
@group(0) @binding(2) var<storage, read> knots: array<f32>;
@group(0) @binding(3) var<storage, read_write> points: array<f32>;

fn span_of(degree: u32, count: u32, t: f32) -> u32 {
    if (t >= knots[count]) {
        return count - 1u;
    }
    if (t <= knots[degree]) {
        return degree;
    }
    var span = degree;
    for (var index = degree; index <= count - 1u; index = index + 1u) {
        if (knots[index] <= t) {
            span = index;
        }
    }
    return span;
}

fn basis_of(t: f32, span: u32, degree: u32, values: ptr<function, array<f32, 10>>) {
    for (var index = 0u; index < WIDTH; index = index + 1u) {
        (*values)[index] = 0.0;
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
            let low = knots[index];
            let high = knots[index + level];
            let low_next = knots[index + 1u];
            let high_next = knots[index + level + 1u];
            let left = select(0.0, 1.0 / (high - low), abs(high - low) > 1e-12);
            let right = select(
                0.0,
                1.0 / (high_next - low_next),
                abs(high_next - low_next) > 1e-12,
            );
            (*values)[j] = left * (t - low) * previous[j]
                + right * (high_next - t) * previous[j + 1u];
        }
        for (var index = 0u; index < WIDTH; index = index + 1u) {
            previous[index] = (*values)[index];
        }
    }
}

@compute @workgroup_size(64)
fn curve_points(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= shape.out_n) {
        return;
    }
    let t = shape.u0 + shape.u_step * (f32(id.x) + shape.u_offset);
    let span = span_of(shape.p, shape.count, t);
    var values: array<f32, 10>;
    basis_of(t, span, shape.p, &values);
    let base = span - shape.p;
    var sum = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    for (var a = 0u; a <= shape.p; a = a + 1u) {
        let at = (base + a) * 4u;
        let point = vec4<f32>(hom[at], hom[at + 1u], hom[at + 2u], hom[at + 3u]);
        sum = sum + values[a] * point;
    }
    let out = id.x * 3u;
    points[out] = sum.x / sum.w;
    points[out + 1u] = sum.y / sum.w;
    points[out + 2u] = sum.z / sum.w;
}
