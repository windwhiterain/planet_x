use px_field_schema::field::{CUBE_FACES, cube_direction};
use px_sparse::StarField;
use px_volume_schema::VolumeData;
use px_volume_schema::params::density::DensityParams;
use px_volume_schema::params::emission::EmissionParams;

use crate::density::{bake_density, sample_world};
use crate::stars::brightest_near;

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [v[0] / length, v[1] / length, v[2] / length]
}

pub fn bake_emission(
    density: &VolumeData,
    stars: &StarField,
    params: &EmissionParams,
) -> Result<VolumeData, String> {
    let res = density.res.max(2);
    let layers = density.layers.max(2);
    let shell = px_volume_schema::volume::Shell::new(density.inner, density.outer);
    let light_direction = normalize(params.light);
    let light_radius = shell.radius_of(params.light_radius.clamp(0.0, 1.0));
    let light_position = [
        light_direction[0] * light_radius,
        light_direction[1] * light_radius,
        light_direction[2] * light_radius,
    ];

    let steps = params.shadow_steps.max(1);

    let star_reach = params.starlight_radius.max(1e-4);
    let star_soft2 = (params.starlight_soft.max(1e-4)).powi(2);
    let star_steps = params.starlight_steps.max(1);
    let star_keep = params.starlight_max as usize;

    let width = res as usize * 6;
    let height = (CUBE_FACES * layers * res) as usize;
    let data = px_field_schema::parallel::rows(width, height, |first, count, out| {
        for row in 0..count {
            let row_index = (first + row) as u32;
            let face = row_index / (layers * res);
            let layer = (row_index % (layers * res)) / res;
            let t = row_index % res;
            let altitude = layer as f32 / (layers - 1) as f32;
            let radius = shell.radius_of(altitude);
            let s_t = (t as f32 + 0.5) / res as f32;
            let mut candidates: Vec<px_sparse::Star> = Vec::new();
            for s in 0..res {
                let s_s = (s as f32 + 0.5) / res as f32;
                let direction = cube_direction(face, s_s, s_t);
                let position = [
                    direction[0] * radius,
                    direction[1] * radius,
                    direction[2] * radius,
                ];

                let d = sample_world(density, position).max(0.0);

                let to_light = [
                    light_position[0] - position[0],
                    light_position[1] - position[1],
                    light_position[2] - position[2],
                ];
                let light_distance = (to_light[0] * to_light[0]
                    + to_light[1] * to_light[1]
                    + to_light[2] * to_light[2])
                    .sqrt()
                    .max(1e-4);
                let through = light_distance / steps as f32;
                let mut optical_depth = 0.0_f32;
                for step_index in 1..=steps {
                    let far = step_index as f32 * through;
                    let probe = [
                        position[0] + to_light[0] / light_distance * far,
                        position[1] + to_light[1] / light_distance * far,
                        position[2] + to_light[2] / light_distance * far,
                    ];
                    optical_depth += sample_world(density, probe) * through;
                }
                let reach = (light_radius / light_distance).clamp(0.0, 1.0);
                let lit = (-optical_depth * params.shadow_gain).exp() * reach * reach;

                let mut star_lit = [0.0_f32; 3];
                if params.starlight_gain > 0.0 {
                    brightest_near(stars, position, star_reach, star_keep, &mut candidates);
                    for star in &candidates {
                        let to_star = [
                            star.position[0] - position[0],
                            star.position[1] - position[1],
                            star.position[2] - position[2],
                        ];
                        let distance2 = to_star[0] * to_star[0]
                            + to_star[1] * to_star[1]
                            + to_star[2] * to_star[2];
                        let distance = distance2.sqrt().max(1e-4);
                        let away = [
                            to_star[0] / distance,
                            to_star[1] / distance,
                            to_star[2] / distance,
                        ];
                        let through = distance / star_steps as f32;
                        let mut tau = 0.0_f32;
                        for step_index in 1..=star_steps {
                            let far = step_index as f32 * through;
                            let probe = [
                                position[0] + away[0] * far,
                                position[1] + away[1] * far,
                                position[2] + away[2] * far,
                            ];
                            tau += sample_world(density, probe) * through;
                        }
                        let falloff = star.brightness / (distance2 + star_soft2);
                        let visible = (-tau * params.shadow_gain).exp() * falloff;
                        for channel in 0..3 {
                            star_lit[channel] += visible;
                        }
                    }
                }

                let main = d.powf(params.emission_power) * params.emission_gain * lit;
                let above = (d - params.glow_threshold).max(0.0);
                let glow = above.powf(params.glow_power) * params.glow_gain * lit;

                let base = d.powf(params.extinction_power);
                let dust = ((d - params.dust_threshold).max(0.0)) * params.dust_bias;

                let star_emit = d.powf(params.emission_power) * params.starlight_gain;

                let at = row * width + s as usize * 6;
                for channel in 0..3 {
                    out[at + channel] = (main + glow + star_emit * star_lit[channel])
                        * params.glow_tint[channel]
                        * params.scatter_tint[channel];
                    out[at + 3 + channel] = base * params.extinction[channel] + dust;
                }
            }
        }
    })?;

    Ok(VolumeData {
        lanes: 6,
        res,
        layers,
        inner: density.inner,
        outer: density.outer,
        data,
    })
}

pub fn emit_from_field(
    density_params: &DensityParams,
    emission_params: &EmissionParams,
    stars: &StarField,
    density_field: &px_field_schema::field::Field,
) -> Result<VolumeData, String> {
    let density = bake_density(density_params, density_field)?;
    bake_emission(&density, stars, emission_params)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The banding is fallible now, so a test that only wants the value says so once here.
    fn baked(density: &VolumeData, stars: &StarField, params: &EmissionParams) -> VolumeData {
        bake_emission(density, stars, params).expect("测试夹具的行带不 panic")
    }
    use px_field_schema::field::{Field, Projection};
    use px_field_schema::volume::VolumeShape;

    fn flat_density(res: u32, layers: u32, value: f32) -> VolumeData {
        VolumeData {
            lanes: 1,
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data: vec![value; (CUBE_FACES * layers * res * res) as usize],
        }
    }

    fn no_stars() -> StarField {
        StarField::build(empty_meta(), &[], &[], &[]).expect("造空星场")
    }

    fn one_star(brightness: f32) -> StarField {
        StarField::build(
            empty_meta(),
            &[[1.5, 0.0, 0.0]],
            &[brightness],
            &[[1.0, 1.0, 1.0]],
        )
        .expect("造一颗星")
    }

    fn empty_meta() -> px_sparse::GridMeta {
        let block = px_sparse::grid::CHUNK_CELLS;
        let dims = 3 * block;
        px_sparse::GridMeta {
            cell: 0.5,
            origin: [-3.0; 3],
            dims: [dims, dims, dims],
        }
    }

    #[test]
    fn every_voxel_centre_lands_inside_the_shell() {
        let density = flat_density(8, 4, 0.5);
        let shell = px_volume_schema::volume::Shell::new(density.inner, density.outer);
        let mut worst = 0.0_f32;
        for face in 0..CUBE_FACES {
            for layer in 0..density.layers {
                let altitude = layer as f32 / (density.layers - 1) as f32;
                let radius = shell.radius_of(altitude);
                assert!(
                    (density.inner..=density.outer).contains(&radius),
                    "层 {layer} 的半径 {radius} 跑到壳外了（壳是 {}..{}）",
                    density.inner,
                    density.outer
                );
                for t in 0..density.res {
                    for s in 0..density.res {
                        let direction = cube_direction(
                            face,
                            (s as f32 + 0.5) / density.res as f32,
                            (t as f32 + 0.5) / density.res as f32,
                        );
                        let point = [
                            direction[0] * radius,
                            direction[1] * radius,
                            direction[2] * radius,
                        ];
                        let read = sample_world(&density, point);
                        worst = worst.max((read - 0.5).abs());
                    }
                }
            }
        }
        assert!(worst < 1e-4, "格心读到的密度应当是 0.5，最大偏差 {worst}");
    }

    #[test]
    fn the_face_pointing_at_the_light_is_much_brighter_than_the_one_pointing_away() {
        let density = flat_density(16, 8, 0.6);
        let params = EmissionParams {
            light: [1.0, 0.0, 0.0],
            light_radius: 0.1,
            shadow_steps: 32,
            shadow_gain: 3.0,
            ..Default::default()
        };
        let emission = baked(&density, &no_stars(), &params);
        let emit_of = |face: u32| -> f32 {
            let layer = density.layers - 1;
            let mid = density.res / 2;
            emission.data[((((face * density.layers + layer) * density.res + mid) * density.res
                + mid)
                * 6) as usize]
        };
        let facing = emit_of(0);
        let away = emit_of(1);
        assert!(
            facing > away * 3.0,
            "正对光源 {facing:.5} 应当远亮于背对 {away:.5}（差不到 3 倍说明遮挡没生效）"
        );
        assert!(away > 0.0, "背光那一侧还应当有自发光的底（不是全黑）");
    }

    #[test]
    fn extinction_grows_with_density() {
        let thin = baked(
            &flat_density(8, 6, 0.15),
            &no_stars(),
            &EmissionParams {
                shadow_gain: 0.0,
                ..Default::default()
            },
        );
        let thick = baked(
            &flat_density(8, 6, 0.85),
            &no_stars(),
            &EmissionParams {
                shadow_gain: 0.0,
                ..Default::default()
            },
        );
        let mean_alpha = |volume: &VolumeData| -> f64 {
            volume.data.chunks(4).map(|c| c[3] as f64).sum::<f64>()
                / volume.data.len().max(4) as f64
                * 6.0
        };
        assert!(
            mean_alpha(&thick) > mean_alpha(&thin),
            "浓的地方消光必须更大：{} vs {}",
            mean_alpha(&thick),
            mean_alpha(&thin)
        );
    }

    #[test]
    fn the_extinction_channels_differ() {
        let params = EmissionParams {
            extinction: [1.0, 2.0, 3.0],
            dust_bias: 0.0,
            shadow_gain: 0.0,
            ..Default::default()
        };
        let density = flat_density(8, 4, 0.5);
        let emission = baked(&density, &no_stars(), &params);
        assert!(
            emission.data[3] > 1e-6,
            "格心该读到密度 0.5，σ_R 却是 {}（密度没读到）",
            emission.data[3]
        );
        let alpha = emission.data[5];
        assert!(
            (alpha - 1.5).abs() < 1e-3,
            "0.5 密度 × B 通道系数 3 应当是 1.5，实际 {alpha}"
        );
        assert!(
            emission.data[3] < emission.data[4] && emission.data[4] < emission.data[5],
            "三个消光通道必须逐格不同：{:?}",
            &emission.data[3..6]
        );
    }

    #[test]
    fn a_field_of_the_wrong_shape_is_rejected() {
        let field = Field::filled_with(4, 4, 0.0, Projection::Volume);
        let shape = VolumeShape { res: 4, layers: 3 };
        assert_ne!(field.height, shape.height());
        assert!(
            emit_from_field(
                &DensityParams::default(),
                &EmissionParams::default(),
                &no_stars(),
                &field
            )
            .is_err(),
            "形状对不上的场必须被拒"
        );
    }

    #[test]
    fn starlight_needs_both_a_star_and_gas() {
        let params = EmissionParams {
            starlight_gain: 4.0,
            starlight_radius: 0.5,
            starlight_steps: 4,
            shadow_gain: 0.0,
            ..Default::default()
        };
        let emission_of = |brightness: f32| -> [f64; 3] {
            let volume = baked(&flat_density(8, 4, 0.6), &one_star(brightness), &params);
            let sum = |lane: usize| -> f64 {
                volume
                    .data
                    .chunks(6)
                    .map(|chunk| chunk[lane] as f64)
                    .sum::<f64>()
            };
            [sum(0), sum(1), sum(2)]
        };
        let off = emission_of(0.0);
        let on = emission_of(8.0);
        assert!(
            on[0] > off[0] * 1.5,
            "点亮一颗星应当让气体明显更亮：{:.3} vs {:.3}",
            on[0],
            off[0]
        );
        let brighter = emission_of(16.0);
        assert!(
            (brighter[0] - off[0]) > 1.8 * (on[0] - off[0]),
            "星光那一笔必须跟着星的亮度线性涨"
        );
    }

    #[test]
    fn starlight_vanishes_without_gas() {
        let params = EmissionParams {
            starlight_gain: 4.0,
            starlight_radius: 0.5,
            starlight_steps: 4,
            shadow_gain: 0.0,
            ..Default::default()
        };
        let vacuum = baked(&flat_density(8, 4, 0.0), &one_star(32.0), &params);
        for value in &vacuum.data {
            assert_eq!(*value, 0.0, "真空里不该有星光照出来的光");
        }
    }
}
