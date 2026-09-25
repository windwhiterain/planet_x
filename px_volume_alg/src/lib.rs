//! See docs/volume.md

pub mod density;
pub mod emission;
#[allow(dead_code)]
pub mod field_fn;
pub mod half;
pub mod raymarch;
pub mod stars;

pub use density::{bake_density, sample_world};
pub use emission::{bake_emission, emit_from_field};
pub use raymarch::{
    GRADE_STRENGTH, TONE_LIMITS, ramp_hue_at, raymarch_channel, raymarch_sky, sample_volume,
    tone_at,
};
pub use stars::{bake_stars, stars_near};

use field_fn::{FieldFn, SampleField};
use px_field_schema::field::{Field, tangent_frame};
use px_verify::proxy;
use px_volume_schema::params::{self, FieldKind};
use px_volume_schema::{PATCHES, VolumeData, direction_of};

fn normalize(vector: [f32; 3]) -> [f32; 3] {
    let length = (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt();
    if length <= f32::EPSILON {
        return [0.0, 1.0, 0.0];
    }
    [vector[0] / length, vector[1] / length, vector[2] / length]
}

pub fn bake<F: FieldFn>(params: &params::Params, cover: &F) -> VolumeData {
    let cloud = proxy::from_volume(params);
    let at = |direction: [f32; 3]| cover.cover(&cloud, direction);
    let res = params.res.max(2);
    let layers = params.layers.max(2);
    let inv_scale = 1.0
        / if params.scale > 0.0 {
            params.scale
        } else {
            1.0
        };

    let steps = 4 * params.reach as i64;
    let radius = params.reach as f32 * proxy::tangential_cell(params);
    let mut node_cover = vec![0.0_f32; (PATCHES * res * res) as usize];
    for face in 0..PATCHES {
        for t in 0..res {
            for s in 0..res {
                let direction = direction_of(
                    face,
                    s as f32 / (res - 1) as f32,
                    t as f32 / (res - 1) as f32,
                );
                let mut best = at(direction);
                if radius > 0.0 {
                    let (east, north) = tangent_frame(direction);
                    for far in -steps..=steps {
                        for side in -steps..=steps {
                            if side * side + far * far > steps * steps {
                                continue;
                            }
                            let across = radius * side as f32 / steps as f32;
                            let along = radius * far as f32 / steps as f32;
                            let neighbour = normalize([
                                direction[0] + east[0] * across + north[0] * along,
                                direction[1] + east[1] * across + north[1] * along,
                                direction[2] + east[2] * across + north[2] * along,
                            ]);
                            best = best.max(at(neighbour));
                        }
                    }
                }
                node_cover[((face * res + t) * res + s) as usize] = best;
            }
        }
    }

    let mut data = vec![0.0_f32; (PATCHES * layers * res * res) as usize];
    for face in 0..PATCHES {
        for layer in 0..layers {
            let altitude = layer as f32 / (layers - 1) as f32;
            for t in 0..res {
                for s in 0..res {
                    let cover = node_cover[((face * res + t) * res + s) as usize];
                    let value = match params.field {
                        FieldKind::Coarse => proxy::coarse_at(&cloud, cover, altitude),
                        FieldKind::Final => {
                            let direction = direction_of(
                                face,
                                s as f32 / (res - 1) as f32,
                                t as f32 / (res - 1) as f32,
                            );
                            proxy::final_at(&cloud, cover, direction, altitude)
                        }
                    };
                    let slot = (((face * layers + layer) * res + t) * res + s) as usize;
                    data[slot] = (value - params.tau) * inv_scale;
                }
            }
        }
        let reach = params.reach as usize;
        if reach > 0 && layers as usize > 2 * reach + 1 {
            let mut filtered = vec![0.0_f32; (layers * res * res) as usize];
            for layer in 0..layers as usize {
                let low = layer.saturating_sub(reach);
                let high = (layer + reach).min(layers as usize - 1);
                for t in 0..res as usize {
                    for s in 0..res as usize {
                        let mut best = f32::MIN;
                        for other in low..=high {
                            let slot = (((face * layers + other as u32) * res + t as u32) * res
                                + s as u32) as usize;
                            best = best.max(data[slot]);
                        }
                        filtered[(layer * res as usize + t) * res as usize + s] = best;
                    }
                }
            }
            for layer in 1..layers as usize - 1 {
                for t in 0..res as usize {
                    for s in 0..res as usize {
                        let slot = (((face * layers + layer as u32) * res + t as u32) * res
                            + s as u32) as usize;
                        data[slot] = filtered[(layer * res as usize + t) * res as usize + s];
                    }
                }
            }
        }
    }
    VolumeData {
        res,
        layers,
        inner: params.inner,
        outer: params.outer,
        lanes: 1,
        data,
    }
}

pub fn eval_sampled(params: &params::Params, coverage: &Field) -> VolumeData {
    bake(params, &SampleField { field: coverage })
}

pub fn coarse_with<F: FieldFn>(params: &params::Params, coverage: &Field, cover: &F) -> VolumeData {
    let _ = coverage;
    bake(params, cover)
}
