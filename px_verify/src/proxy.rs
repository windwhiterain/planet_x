use px_field_schema::field::{Field, normalize, tangent_frame};
use px_volume_schema::{FieldKind, PATCHES, Params, direction_of};

use crate::cloud_field::CloudFieldParams;

pub fn from_volume(params: &Params) -> CloudFieldParams {
    CloudFieldParams {
        orientation: params.orientation,
        inner: params.inner,
        outer: params.outer,
        coverage: params.coverage,
        base: params.base,
        top: params.top,
        detail_scale: 16.0,
        detail_strength: 0.55,
        erode: params.erode,
        taper: params.taper,
        coverage_gain: params.coverage_gain,
        seed: 7,
    }
}

pub fn cover_at(cloud: &CloudFieldParams, coverage: &Field, direction: [f32; 3]) -> f32 {
    let local = cloud.to_local(direction);
    cloud.cover_from_mask(coverage.sample_direction(local))
}

pub fn coarse_at(cloud: &CloudFieldParams, cover: f32, altitude: f32) -> f32 {
    if altitude < 0.0 || altitude > 1.0 || cover <= 0.0 {
        return 0.0;
    }
    cloud.shape(cover, altitude, 1.0)
}

pub fn final_at(cloud: &CloudFieldParams, cover: f32, direction: [f32; 3], altitude: f32) -> f32 {
    if altitude < 0.0 || altitude > 1.0 || cover <= 0.0 {
        return 0.0;
    }
    let local = cloud.to_local(direction);
    cloud.shape(cover, altitude, cloud.billows(local, altitude))
}

pub fn field_at(
    cloud: &CloudFieldParams,
    params: &Params,
    cover: f32,
    direction: [f32; 3],
    altitude: f32,
) -> f32 {
    match params.field {
        FieldKind::Coarse => coarse_at(cloud, cover, altitude),
        FieldKind::Final => final_at(cloud, cover, direction, altitude),
    }
}

pub fn tangential_cell(params: &Params) -> f32 {
    let res = params.res.max(2);
    let step = 1.0 / (res - 1) as f32;
    let mut worst = 0.0_f32;
    for face in 0..PATCHES {
        for t in 0..=16 {
            for s in 0..=16 {
                let u = s as f32 / 16.0;
                let v = t as f32 / 16.0;
                let here = direction_of(face, u, v);
                for (du, dv) in [(step, 0.0), (0.0, step)] {
                    let there = direction_of(face, u + du, v + dv);
                    let far = ((there[0] - here[0]).powi(2)
                        + (there[1] - here[1]).powi(2)
                        + (there[2] - here[2]).powi(2))
                    .sqrt()
                        * params.outer;
                    worst = worst.max(far);
                }
            }
        }
    }
    worst
}

pub fn unit(vector: [f32; 3]) -> [f32; 3] {
    normalize(vector)
}

pub fn tangent(direction: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    tangent_frame(direction)
}
