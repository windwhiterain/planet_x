use px_field_schema::field::Field;
use px_field_schema::ops::Craters;
use px_field_schema::params;

use crate::noise;

px_graph_schema::px_body! {
    Craters,
    |p, i| crate::ops::craters::eval(p, &[i.base.value()])
}

pub fn eval(params: &params::CratersParams, inputs: &[&Field]) -> Field {
    let base = inputs[0];
    let mut field = base.clone();
    let jitter = params.jitter.clamp(0.0, 1.0);

    let mut amplitude = 1.0_f32;
    let mut frequency = params.frequency;
    let mut weight = 0.0_f32;
    for octave in 0..params.octaves {
        let seed = params.seed ^ octave.wrapping_mul(0x9e37_79b9);
        for y in 0..base.height {
            for x in 0..base.width {
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
