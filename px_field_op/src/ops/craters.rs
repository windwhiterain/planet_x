use px_field_schema::field::Field;
use px_field_schema::ops::Craters;
use px_field_schema::params;
use px_graph_schema::Grid;

use crate::noise;

px_graph_schema::px_body! {
    Craters,
    |p, i, g| crate::ops::craters::eval(p, &[i.base.value()], g)
}

/// **在 `base` 上打坑**：一层一层地叠 —— 每层换个格点尺度，坑里往下压、坑缘往上抬。
///
/// ⚠ 剖面在 `d = radius` 处**两侧都是 0**（碗在坑缘归零、凸起从坑缘起算），所以层与层可以
///   直接相加而不会在坑边留下台阶。多层按 `gain` 加权、再**除以总权** ⇒ 层数不改变幅度量级。
///
/// ⚠ 算子**不钳制**输出：进来的地形本来就可能越界（比如上游是 `field.remap` 映到
///   `[-1, 2]`），"算出来必须落在 `[0,1]`"不是这一层替别人兜的底。
///   ⚠ 这一条是 `px_field_alg::Scale::map` 那条注释的同一族：**顺手加一个钳制就是静默换产物**。
pub fn eval(params: &params::CratersParams, inputs: &[&Field], grid: Grid) -> Field {
    let base = inputs[0];
    let mut field = base.clone();
    // `jitter > 1` 时 27 邻域不再保证找到最近点（见 `noise` 那一族的注释）⇒ 钳在 `[0,1]`。
    let jitter = params.jitter.clamp(0.0, 1.0);

    let mut amplitude = 1.0_f32;
    let mut frequency = params.frequency;
    let mut weight = 0.0_f32;
    for octave in 0..params.octaves {
        let seed = params.seed ^ octave.wrapping_mul(0x9e37_79b9);
        for y in 0..grid.height {
            for x in 0..grid.width {
                let distance = if params.spherical {
                    let direction = base.direction(x, y);
                    noise::worley_3(
                        [
                            direction[0] * frequency,
                            direction[1] * frequency,
                            direction[2] * frequency,
                        ],
                        seed,
                        jitter,
                    )
                } else {
                    let (u, v) = base.uv(x, y);
                    noise::worley_2([u * params.aspect * frequency, v * frequency], seed, jitter)
                };
                let delta = crater_delta(
                    distance,
                    params.radius,
                    params.rim,
                    params.depth,
                    params.height,
                );
                field.set(x, y, field.at(x, y) + delta * amplitude);
            }
        }
        weight += amplitude;
        amplitude *= params.gain;
        frequency *= params.lacunarity;
    }

    if weight > 0.0 {
        let data = field
            .data
            .iter()
            .zip(base.data.iter())
            .map(|(value, anchor)| anchor + (value - anchor) / weight)
            .collect();
        field.data = data;
    }
    field
}

/// **一个坑的剖面**（相对基底的增量）：坑内是 `t²` 的碗（`t = 1 - d / radius`），
/// 坑缘是 `radius .. radius + rim` 上的半正弦凸起。
///
/// 边界：`radius` / `rim` 为 0 或负时按一个极小值处理（不 panic、也不产生 NaN）；
/// 距离恰在 `radius` 上时**两项都是 0**（凹凸在这里交接）。
fn crater_delta(distance: f32, radius: f32, rim: f32, depth: f32, height: f32) -> f32 {
    let radius = radius.max(1e-3);
    let rim = rim.max(1e-3);
    let bowl = if distance < radius {
        let t = 1.0 - distance / radius;
        t * t
    } else {
        0.0
    };
    let ring = if distance >= radius {
        let t = ((distance - radius) / rim).clamp(0.0, 1.0);
        (std::f32::consts::PI * t).sin()
    } else {
        0.0
    };
    0.5 * height * ring - 0.5 * depth * bowl
}
