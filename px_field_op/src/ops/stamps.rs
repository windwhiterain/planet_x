use px_field_schema::field::Field;
use px_field_schema::ops::Stamps;
use px_field_schema::params;

use crate::noise;

px_graph_schema::px_body! {
    Stamps,
    |p, i| crate::ops::stamps::eval(p, &[i.base.value()])
}

#[derive(Clone, Copy)]
struct Stamp {
    centre: [f32; 3],
    radius: f32,
    age: u32,
    coin: f32,
}

pub fn eval(params: &params::StampsParams, inputs: &[&Field]) -> Field {
    let base = inputs[0];
    let mut field = base.clone();
    let jitter = params.jitter.clamp(0.0, 1.0);
    let rim = params.rim.max(1e-3);
    let max_radius = params.max_radius.clamp(1e-3, 1.0 / (1.0 + rim));
    let min_radius = params.min_radius.clamp(1e-3, max_radius);
    let power = params.power.max(0.05);
    let excavate = params.excavate.clamp(0.0, 1.0);
    let degrade = params.degrade.clamp(0.0, 1.0);

    let mut frequency = params.frequency;
    let mut amplitude = 1.0_f32;
    let mut weight = 0.0_f32;
    let mut candidates: Vec<Stamp> = Vec::with_capacity(27);
    for octave in 0..params.octaves {
        let seed = params.seed ^ octave.wrapping_mul(0x9e37_79b9);
        for y in 0..base.height {
            for x in 0..base.width {
                let point = grid_point(params, base, x, y, frequency);
                let cell = [point[0].floor(), point[1].floor(), point[2].floor()];

                candidates.clear();
                for offset in noise::neighbours(params.spherical) {
                    {
                        {
                            let at = [
                                cell[0] + offset[0] as f32,
                                cell[1] + offset[1] as f32,
                                cell[2] + offset[2] as f32,
                            ];
                            let Some(stamp) =
                                stamp_of(at, seed, jitter, max_radius, min_radius, power)
                            else {
                                continue;
                            };
                            let footprint = stamp.radius * (1.0 + rim);
                            if grid_distance(params, point, stamp.centre) >= footprint {
                                continue;
                            }
                            if !passes_mask(params, base, stamp.centre, frequency, stamp.coin) {
                                continue;
                            }
                            candidates.push(stamp);
                        }
                    }
                }
                if candidates.is_empty() {
                    continue;
                }
                candidates.sort_unstable_by_key(|stamp| stamp.age);
                let oldest = candidates[0].age;
                let newest = candidates[candidates.len() - 1].age;
                let ages = (newest.saturating_sub(oldest)) as f32;
                let mut sum = 0.0_f32;
                for stamp in &candidates {
                    let freshness = if ages > 0.0 {
                        1.0 - degrade * ((newest - stamp.age) as f32 / ages)
                    } else {
                        1.0
                    };
                    sum = excavate_stamp(
                        sum,
                        grid_distance(params, point, stamp.centre),
                        stamp.radius,
                        rim,
                        params.depth,
                        params.height,
                        excavate,
                        freshness,
                    );
                }
                field.set(x, y, base.at(x, y) + sum * amplitude);
            }
        }
        weight += amplitude;
        amplitude *= params.gain;
        frequency *= params.lacunarity;
    }

    if weight > 0.0 && (weight - 1.0).abs() > 1e-6 {
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

fn grid_point(
    params: &params::StampsParams,
    base: &Field,
    x: u32,
    y: u32,
    frequency: f32,
) -> [f32; 3] {
    if params.spherical {
        let direction = base.direction(x, y);
        [
            direction[0] * frequency,
            direction[1] * frequency,
            direction[2] * frequency,
        ]
    } else {
        let (u, v) = base.uv(x, y);
        [u * params.aspect * frequency, v * frequency, 0.0]
    }
}

fn grid_distance(params: &params::StampsParams, one: [f32; 3], two: [f32; 3]) -> f32 {
    if params.spherical {
        euclid([one[0] - two[0], one[1] - two[1], one[2] - two[2]])
    } else {
        euclid2([one[0] - two[0], one[1] - two[1]])
    }
}

fn passes_mask(
    params: &params::StampsParams,
    base: &Field,
    centre: [f32; 3],
    frequency: f32,
    coin: f32,
) -> bool {
    let (lo, hi) = if params.mask_hi > params.mask_lo {
        (params.mask_lo, params.mask_hi)
    } else {
        (0.0, 1.0)
    };
    let mask = if params.spherical {
        base.sample_direction(normalize([centre[0], centre[1], centre[2]]))
    } else {
        base.sample_uv(
            centre[0] / (params.aspect * frequency),
            centre[1] / frequency,
        )
    };
    let chance = ((mask - lo) / (hi - lo)).clamp(0.0, 1.0);
    coin <= chance
}

#[allow(clippy::too_many_arguments)]
fn excavate_stamp(
    sum: f32,
    distance: f32,
    radius: f32,
    rim: f32,
    depth: f32,
    height: f32,
    excavate: f32,
    freshness: f32,
) -> f32 {
    let radius = radius.max(1e-4);
    let depth = depth.max(0.0) * radius * freshness;
    if distance < radius {
        let t = 1.0 - distance / radius;
        let bowl = t * t;
        let cleared = sum * (1.0 - excavate * bowl);
        cleared - depth * bowl
    } else {
        let rim_width = rim * radius;
        if distance >= radius + rim_width {
            return sum;
        }
        let t = (distance - radius) / rim_width;
        sum + height.max(0.0) * depth * (std::f32::consts::PI * t).sin()
    }
}

fn stamp_of(
    cell: [f32; 3],
    seed: u32,
    jitter: f32,
    max_radius: f32,
    min_radius: f32,
    power: f32,
) -> Option<Stamp> {
    let key = [cell[0] as i32, cell[1] as i32, cell[2] as i32];
    let geometry = noise::cell_hash(seed, key);
    let fate = noise::cell_hash(seed ^ 0x51ed_270b, key);
    let unit = |hash: u32, shift: u32| ((hash >> shift) & 0x3ff) as f32 / 1023.0;
    let centre = [
        cell[0] + 0.5 + (unit(geometry, 0) - 0.5) * jitter,
        cell[1] + 0.5 + (unit(geometry, 10) - 0.5) * jitter,
        cell[2] + 0.5 + (unit(geometry, 20) - 0.5) * jitter,
    ];
    let u = unit(geometry, 4);
    let radius = (min_radius + (max_radius - min_radius) * (1.0 - u).powf(power))
        .clamp(min_radius, max_radius);
    Some(Stamp {
        centre,
        radius,
        age: fate,
        coin: unit(fate, 20),
    })
}

fn euclid(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn euclid2(v: [f32; 2]) -> f32 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = euclid(v);
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stamp_is_a_pure_function_of_its_cell() {
        let once = stamp_of([3.0, -2.0, 7.0], 91, 0.9, 0.5, 0.1, 2.2).expect("有印章");
        let twice = stamp_of([3.0, -2.0, 7.0], 91, 0.9, 0.5, 0.1, 2.2).expect("有印章");
        assert_eq!(once.centre, twice.centre);
        assert_eq!(once.radius, twice.radius);
        assert_eq!(once.age, twice.age);
        assert_eq!(once.coin, twice.coin);
        let other = stamp_of([3.0, -2.0, 8.0], 91, 0.9, 0.5, 0.1, 2.2).expect("有印章");
        assert!(other.radius != once.radius || other.age != once.age);
    }

    #[test]
    fn the_sizes_follow_the_power_law() {
        let mut radii = Vec::new();
        for x in 0..18 {
            for y in 0..18 {
                for z in 0..3 {
                    let stamp = stamp_of([x as f32, y as f32, z as f32], 7, 0.9, 0.55, 0.12, 2.4)
                        .expect("有印章");
                    radii.push(stamp.radius);
                }
            }
        }
        let count = radii.len() as f32;
        let mean = radii.iter().sum::<f32>() / count;
        let small = radii.iter().filter(|radius| **radius < mean).count() as f32;
        assert!(
            small / count > 0.6,
            "小于平均半径的印章只有 {:.2}（幂律没起作用？）",
            small / count
        );
        assert!(
            radii
                .iter()
                .all(|radius| *radius >= 0.12 - 1e-6 && *radius <= 0.55 + 1e-6),
            "半径必须落在 [min_radius, max_radius] 里"
        );
    }

    #[test]
    fn a_young_bowl_clears_the_older_relief_inside_it() {
        let cleared = excavate_stamp(0.5, 0.0, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0);
        assert!(
            (cleared + 0.2).abs() < 1e-6,
            "应为 -0.2（清掉 0.5、落底 -0.2）：{cleared}"
        );
        let kept = excavate_stamp(0.5, 0.0, 1.0, 0.3, 0.2, 0.4, 0.0, 1.0);
        assert!((kept - 0.3).abs() < 1e-6, "不清除时应为 0.3：{kept}");
        let half = excavate_stamp(0.5, 0.5, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0);
        assert!(
            (half - (0.5 * 0.75 - 0.05)).abs() < 1e-6,
            "t=0.5 ⇒ bowl=0.25：{half}"
        );
    }

    #[test]
    fn the_rim_is_only_a_fraction_of_the_depth() {
        let rim_peak = excavate_stamp(0.0, 1.15, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0);
        let expected = 0.4 * 0.2 * 1.0;
        assert!(
            (rim_peak - expected).abs() < 1e-6,
            "缘峰应为 {expected}：{rim_peak}"
        );
        assert!(rim_peak < 0.2, "缘高必须明显小于坑深（0.2）");
        assert_eq!(
            excavate_stamp(0.11, 1.31, 1.0, 0.3, 0.2, 0.4, 1.0, 1.0),
            0.11
        );
    }
}
