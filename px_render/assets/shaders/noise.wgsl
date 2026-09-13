#define_import_path planet_x::noise

var<private> GRADIENTS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(1.0, 1.0, 0.0),
    vec3<f32>(-1.0, 1.0, 0.0),
    vec3<f32>(1.0, -1.0, 0.0),
    vec3<f32>(-1.0, -1.0, 0.0),
    vec3<f32>(1.0, 0.0, 1.0),
    vec3<f32>(-1.0, 0.0, 1.0),
    vec3<f32>(1.0, 0.0, -1.0),
    vec3<f32>(-1.0, 0.0, -1.0),
    vec3<f32>(0.0, 1.0, 1.0),
    vec3<f32>(0.0, -1.0, 1.0),
    vec3<f32>(0.0, 1.0, -1.0),
    vec3<f32>(0.0, -1.0, -1.0),
);

fn easier(value: f32) -> f32 {
    return value * value * (3.0 - 2.0 * value);
}

fn lattice_3(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    var hash = (bitcast<u32>(x) * 0x27d4eb2du)
        ^ (bitcast<u32>(y) * 0x165667b1u)
        ^ (bitcast<u32>(z) * 0x9e3779b9u)
        ^ seed;
    hash ^= hash >> 15u;
    hash = hash * 0x2c1b3c6du;
    hash ^= hash >> 12u;
    hash = hash * 0x297a2d39u;
    hash ^= hash >> 15u;
    return hash;
}

fn gradient_noise_3(point: vec3<f32>, seed: u32) -> f32 {
    let base = floor(point);
    let cell = vec3<i32>(base);
    let local = point - base;
    let weight = vec3<f32>(easier(local.x), easier(local.y), easier(local.z));

    var total = 0.0;
    for (var corner = 0u; corner < 8u; corner += 1u) {
        let step = vec3<i32>(
            i32(corner & 1u),
            i32((corner >> 1u) & 1u),
            i32((corner >> 2u) & 1u),
        );
        let index = lattice_3(cell.x + step.x, cell.y + step.y, cell.z + step.z, seed) % 12u;
        let gradient = GRADIENTS[index];
        let offset = vec3<f32>(step);
        let axis = local - offset;
        let dot = gradient.x * axis.x + gradient.y * axis.y + gradient.z * axis.z;
        let blend = select(vec3<f32>(1.0) - weight, weight, step == vec3<i32>(1));
        total += dot * blend.x * blend.y * blend.z;
    }
    return clamp(total * 0.9 + 0.5, 0.0, 1.0);
}

fn fbm_3(
    point: vec3<f32>,
    frequency: f32,
    octaves: u32,
    lacunarity: f32,
    gain: f32,
    seed: u32,
) -> f32 {
    var total = 0.0;
    var amplitude = 1.0;
    var normalization = 0.0;
    var current = frequency;
    for (var octave = 0u; octave < octaves; octave += 1u) {
        total += amplitude * gradient_noise_3(point * current, seed ^ octave);
        normalization += amplitude;
        amplitude *= gain;
        current *= lacunarity;
    }
    if normalization > 0.0 {
        return total / normalization;
    }
    return 0.0;
}

fn rotate_vector(quaternion: vec4<f32>, vector: vec3<f32>) -> vec3<f32> {
    let first = cross(quaternion.xyz, vector);
    let second = cross(quaternion.xyz, first);
    return vector + 2.0 * (quaternion.w * first + second);
}
