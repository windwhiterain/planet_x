use px_sparse::grid::{CHUNK_CELLS, GridMeta};
use px_sparse::{Star, StarField};

use crate::raymarch::star_falloff;
use px_volume_schema::params::stars::StarsParams;

fn hash(seed: u32, index: u32) -> u32 {
    let mut h = index.wrapping_mul(0x9e37_79b9) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

fn unit24(hash: u32) -> f32 {
    (hash >> 8) as f32 * (1.0 / 16_777_216.0)
}

fn brightness(hash: u32, params: &StarsParams) -> f32 {
    let draw = unit24(hash).max(1e-3);
    (1.0 / draw)
        .powf(params.brightness_power)
        .min(params.max_brightness.max(1.0))
}

fn direction_of(u: f32, v: f32) -> [f32; 3] {
    let z = 2.0 * u - 1.0;
    let phi = std::f32::consts::TAU * v;
    let ring = (1.0 - z * z).max(0.0).sqrt();
    [ring * phi.cos(), ring * phi.sin(), z]
}

fn radius_of(u: f32, params: &StarsParams) -> f32 {
    let inner = params.inner.max(0.0);
    let outer = params.outer.max(inner);
    let cube = inner * inner * inner + u * (outer * outer * outer - inner * inner * inner);
    cube.max(0.0).cbrt()
}

fn sky_density(direction: [f32; 3], params: &StarsParams) -> f32 {
    let settings = px_field_schema::noise::FbmSettings {
        frequency: params.sky_frequency,
        octaves: params.sky_octaves,
        lacunarity: params.sky_lacunarity,
        gain: params.sky_gain,
        seed: params.sky_seed,
    };
    let point = [direction[0], direction[1] * params.sky_zonal, direction[2]];
    let mut total = 0.0_f32;
    let mut amplitude = 1.0_f32;
    let mut normalization = 0.0_f32;
    let mut frequency = settings.frequency;
    for octave in 0..settings.octaves {
        total += amplitude
            * px_field_alg::noise::faded_gradient_noise_3(
                [
                    point[0] * frequency,
                    point[1] * frequency,
                    point[2] * frequency,
                ],
                settings.seed ^ octave,
            );
        normalization += amplitude;
        amplitude *= settings.gain;
        frequency *= settings.lacunarity;
    }
    let value = if normalization > 0.0 {
        total / normalization
    } else {
        0.0
    };
    value.clamp(-1.0, 1.0) * 0.5 + 0.5
}

fn grid_meta(params: &StarsParams, reach: f32) -> GridMeta {
    let cell = params.cell.max(1e-4);
    let half = reach + 2.0 * cell;
    let raw = (2.0 * half / cell).ceil().max(1.0) as u32;
    let dims = raw.div_ceil(CHUNK_CELLS) * CHUNK_CELLS;
    GridMeta {
        cell,
        origin: [-(dims as f32) * cell * 0.5; 3],
        dims: [dims, dims, dims],
    }
}

pub fn bake_stars(
    params: &StarsParams,
    density: Option<&px_volume_schema::VolumeData>,
) -> Result<StarField, String> {
    let seed = params.seed;
    let gas_mean = density
        .map(|volume| {
            let sum: f64 = volume.data.iter().map(|value| *value as f64).sum();
            (sum / volume.data.len().max(1) as f64) as f32
        })
        .filter(|mean| *mean > 1e-9);
    let count = params.count as usize;
    let cluster_count = params.cluster_count as usize;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(count + cluster_count);
    let mut values: Vec<f32> = Vec::with_capacity(count + cluster_count);
    let mut tints: Vec<[f32; 3]> = Vec::with_capacity(count + cluster_count);

    let clump_count = params.clump_count as usize;
    let mut clumps: Vec<[f32; 3]> = Vec::with_capacity(clump_count);
    for index in 0..clump_count {
        let h0 = hash(seed ^ 0x1b87_3f21, index as u32);
        let h1 = hash(seed ^ 0x2f6a_1d43, index as u32);
        let h2 = hash(seed ^ 0x3c9e_5b17, index as u32);
        let r = radius_of(unit24(h0), params);
        let direction = direction_of(unit24(h1), unit24(h2));
        clumps.push([direction[0] * r, direction[1] * r, direction[2] * r]);
    }

    for index in 0..count {
        let h0 = hash(seed, index as u32);
        let h1 = hash(seed ^ 0x9e37_79b9, index as u32);
        let h2 = hash(seed ^ 0x85eb_ca6b, index as u32);
        let value = brightness(hash(seed ^ 0x51ed_270b, index as u32), params);
        let clumped = clump_count > 0
            && params.clump_share > 0.0
            && unit24(hash(seed ^ 0x6d2b_79f5, index as u32)) < params.clump_share;
        let position = if clumped {
            let pick = unit24(hash(seed ^ 0x4a1c_9e37, index as u32));
            let centre = clumps[((pick * clump_count as f32) as usize).min(clump_count - 1)];
            let spread = params.clump_radius.max(0.0)
                * unit24(hash(seed ^ 0x7f4a_2c61, index as u32)).cbrt();
            let direction = direction_of(
                unit24(hash(seed ^ 0x58d3_1b9f, index as u32)),
                unit24(hash(seed ^ 0x93e6_4d27, index as u32)),
            );
            [
                centre[0] + direction[0] * spread,
                centre[1] + direction[1] * spread,
                centre[2] + direction[2] * spread,
            ]
        } else {
            let r = radius_of(unit24(h0), params);
            let direction = direction_of(unit24(h1), unit24(h2));
            [direction[0] * r, direction[1] * r, direction[2] * r]
        };
        let r = (position[0] * position[0] + position[1] * position[1] + position[2] * position[2])
            .sqrt()
            .max(1e-6);
        if params.gas_biased {
            if let (Some(volume), Some(mean)) = (density, gas_mean) {
                let here = crate::density::sample_world(volume, position).max(0.0);
                let ratio = here / mean;
                let floor = params.gas_floor.clamp(0.0, 0.999);
                let over = ((ratio - floor) / (1.0 - floor)).clamp(0.0, 1.0);
                let span = (params.outer - params.inner).max(1e-4);
                let outward = ((r - params.inner) / span).clamp(0.0, 1.0);
                let chance = over.powf(params.gas_contrast) * outward.powf(params.gas_depth);
                if unit24(hash(seed ^ 0x77c1_5a3d, index as u32)) >= chance {
                    continue;
                }
            }
        }
        if params.sky_biased && params.sky_contrast > 0.0 {
            let direction = [position[0] / r, position[1] / r, position[2] / r];
            let chance = sky_density(direction, params).powf(params.sky_contrast);
            if unit24(hash(seed ^ 0x2b9f_41c7, index as u32)) >= chance {
                continue;
            }
        }
        if value * star_falloff(r, params.inner) < params.min_apparent {
            continue;
        }
        positions.push(position);
        values.push(value);
        tints.push(params.star_tint);
    }

    if cluster_count > 0 {
        let n = cluster_count as f32;
        let golden = 2.399_963_2_f32;
        for index in 0..cluster_count {
            let h = hash(seed ^ 0x2545_f491, index as u32);
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / n;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = golden * index as f32;
            let spread = params.cluster_radius.max(0.0)
                * unit24(hash(seed ^ 0x27d4_eb2f, index as u32)).cbrt();
            positions.push([
                params.cluster[0] + ring * phi.cos() * spread,
                params.cluster[1] + ring * phi.sin() * spread,
                params.cluster[2] + z * spread,
            ]);
            values.push(brightness(h, params) * params.cluster_gain);
            tints.push(params.cluster_tint);
        }
    }

    let mut reach = params.outer.abs().max(params.inner.abs());
    for position in &positions {
        for axis in 0..3 {
            reach = reach.max(position[axis].abs());
        }
    }
    let meta = grid_meta(params, reach);
    let field = StarField::build(meta, &positions, &values, &tints)?;
    Ok(field)
}

pub fn stars_near(field: &StarField, point: [f32; 3], radius: f32) -> Vec<Star> {
    let mut out = Vec::new();
    if radius <= 0.0 {
        return out;
    }
    let low = [point[0] - radius, point[1] - radius, point[2] - radius];
    let high = [point[0] + radius, point[1] + radius, point[2] + radius];
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    field.for_each_cell_in(low, high, |_, range| ranges.push(range));
    for range in ranges {
        for index in range {
            let star = field.star(index);
            let delta = [
                star.position[0] - point[0],
                star.position[1] - point[1],
                star.position[2] - point[2],
            ];
            if delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2] <= radius * radius {
                out.push(star);
            }
        }
    }
    out
}

pub fn brightest_near(
    field: &StarField,
    point: [f32; 3],
    radius: f32,
    keep: usize,
    out: &mut Vec<Star>,
) {
    out.clear();
    if radius <= 0.0 {
        return;
    }
    let low = [point[0] - radius, point[1] - radius, point[2] - radius];
    let high = [point[0] + radius, point[1] + radius, point[2] + radius];
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    field.for_each_cell_in(low, high, |_, range| ranges.push(range));
    for range in ranges {
        for index in range {
            let star = field.star(index);
            let delta = [
                star.position[0] - point[0],
                star.position[1] - point[1],
                star.position[2] - point[2],
            ];
            if delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2] <= radius * radius {
                out.push(star);
            }
        }
    }
    if keep > 0 && out.len() > keep {
        out.sort_by(|a, b| {
            b.brightness
                .partial_cmp(&a.brightness)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.truncate(keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> StarsParams {
        StarsParams {
            count: 4096,
            ..Default::default()
        }
    }

    #[test]
    fn the_same_parameters_give_the_same_stars() {
        let one = bake_stars(&params(), None).expect("烘星");
        let two = bake_stars(&params(), None).expect("烘星");
        assert_eq!(one.stars, two.stars);
        assert_eq!(one.grid, two.grid);
    }

    #[test]
    fn every_star_sits_in_the_cell_that_claims_it() {
        let field = bake_stars(&params(), None).expect("烘星");
        let mut seen = 0;
        field.grid.for_each_cell(|cell, range| {
            for index in range {
                let star = field.star(index);
                let here = field.grid.meta.cell_of(star.position);
                assert_eq!(here, cell, "第 {index} 颗星不在它被登记的那一格");
                seen += 1;
            }
        });
        assert_eq!(seen, field.count(), "有星没被任何一格登记");
    }

    #[test]
    fn a_single_star_is_found_where_it_sits() {
        let block = CHUNK_CELLS;
        let meta = GridMeta {
            cell: 0.5,
            origin: [-8.0; 3],
            dims: [block, block, block],
        };
        let field = StarField::build(meta, &[[1.5, 0.0, 0.0]], &[8.0], &[[1.0, 1.0, 1.0]])
            .expect("造一颗星");
        assert_eq!(field.count(), 1);
        assert_eq!(stars_near(&field, [1.625, 0.0, 0.0], 0.2).len(), 1);
        assert_eq!(stars_near(&field, [0.0, 0.0, 0.0], 0.2).len(), 0);
        let mut out = Vec::new();
        brightest_near(&field, [1.625, 0.0, 0.0], 0.2, 8, &mut out);
        assert_eq!(out.len(), 1, "最亮的那几颗里必须有它");
        assert_eq!(out[0].brightness, 8.0);
    }

    #[test]
    fn the_neighbourhood_query_matches_brute_force() {
        let field = bake_stars(&params(), None).expect("烘星");
        let radius = field.grid.meta.cell * 2.0;
        let probes = [
            [0.0, 0.0, 0.0],
            [1.5, 0.0, 0.0],
            [0.0, 2.5, 0.0],
            [1.0, 1.0, 1.4],
            [2.9, -0.3, 0.2],
        ];
        for point in probes {
            let mut fast: Vec<f32> = stars_near(&field, point, radius)
                .iter()
                .map(|star| star.brightness)
                .collect();
            let mut slow: Vec<f32> = (0..field.count())
                .map(|index| field.star(index))
                .filter(|star| {
                    let d = [
                        star.position[0] - point[0],
                        star.position[1] - point[1],
                        star.position[2] - point[2],
                    ];
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() <= radius
                })
                .map(|star| star.brightness)
                .collect();
            fast.sort_by(|a, b| a.partial_cmp(b).expect("没有 NaN"));
            slow.sort_by(|a, b| a.partial_cmp(b).expect("没有 NaN"));
            assert_eq!(fast, slow, "点 {point:?} 的邻域与暴力结果不一致");
        }
    }

    #[test]
    fn the_stars_fill_the_shell_by_volume() {
        let params = StarsParams {
            count: 20000,
            ..params()
        };
        let field = bake_stars(&params, None).expect("烘星");
        let mut radii: Vec<f32> = (0..field.count())
            .map(|index| {
                let p = field.star(index).position;
                (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
            })
            .collect();
        radii.sort_by(|a, b| a.partial_cmp(b).expect("没有 NaN"));
        let first = radii[0];
        let last = radii[radii.len() - 1];
        assert!(
            first >= params.inner - 1e-3 && last <= params.outer + 1e-3,
            "有星跑到壳外：{first}..{last}（壳是 {}..{}）",
            params.inner,
            params.outer
        );
        let median = radii[radii.len() / 2];
        let expected = ((params.inner.powi(3) + params.outer.powi(3)) * 0.5).cbrt();
        assert!(
            (median - expected).abs() < 0.05 * expected,
            "中位半径 {median:.3} 偏离体积中位 {expected:.3} —— 不是按体积均匀"
        );
    }

    #[test]
    fn the_star_density_is_the_same_everywhere() {
        let field = bake_stars(
            &StarsParams {
                count: 60_000,
                ..params()
            },
            None,
        )
        .expect("烘星");
        let radius = field.grid.meta.cell * 2.0;
        let mut counts: Vec<f64> = Vec::new();
        let golden = 2.399_963_2_f32;
        for index in 0..300 {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / 300.0;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = golden * index as f32;
            let point = [ring * phi.cos() * 2.0, ring * phi.sin() * 2.0, z * 2.0];
            counts.push(stars_near(&field, point, radius).len() as f64);
        }
        let mean = counts.iter().sum::<f64>() / counts.len() as f64;
        assert!(mean > 2.0, "平均每球只有 {mean:.2} 颗星，判不出均匀性");
        let spread =
            counts.iter().map(|value| (value - mean).abs()).sum::<f64>() / counts.len() as f64;
        let expected = 3.0 / mean.sqrt();
        assert!(
            spread / mean < expected,
            "每球的星数相对起伏 {:.3} 超过泊松上界 {expected:.3} —— 位置不均匀",
            spread / mean
        );
    }
}
