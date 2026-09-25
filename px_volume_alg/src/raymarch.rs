use px_field_schema::field::{CUBE_FACES, Field, Projection, art_direction_at, cube_face_of};
use px_sparse::StarField;
use px_volume_schema::params::sky::SkyParams;
use px_volume_schema::{TextureData, TextureFormat, VolumeData};

fn jitter_at(texel: u32, step: u32, seed: u32) -> f32 {
    let mut hash = texel
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(step.wrapping_mul(0x85eb_ca6b))
        .wrapping_add(seed.wrapping_mul(0x27d4_eb2d));
    hash ^= hash >> 15;
    hash = hash.wrapping_mul(0x2c1b_3c6d);
    hash ^= hash >> 12;
    (hash & 0xffff) as f32 / 65535.0
}

#[derive(Clone, Copy)]
struct StarHit {
    radius: f32,
    sine: f32,
    power: [f32; 3],
}

pub fn star_power(sine: f32, distance: f32, params: &SkyParams) -> f32 {
    let core = params.star_core.max(1e-6);
    let halo = params.star_halo.max(1e-6);
    let offset = sine * distance.max(1e-4);
    let core_term = (-(offset / core).powi(2)).exp();
    let halo_term = (-(offset / halo).powi(2)).exp();
    core_term + params.star_halo_gain * halo_term
}

pub fn star_support(params: &SkyParams) -> f32 {
    3.0 * params.star_core.max(params.star_halo).max(1e-6)
}

pub fn slab_candidate_counts(
    field: &StarField,
    direction: [f32; 3],
    enter: f32,
    exit: f32,
    support: f32,
) -> Vec<usize> {
    let cell = field.grid.meta.cell;
    let mut out = Vec::new();
    let mut t0 = (enter - cell).max(0.0);
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    while t0 <= exit {
        let t1 = t0 + cell;
        let half = (support + cell).max(cell);
        let angular = support / t1.max(cell);
        let centre = [direction[0] * t1, direction[1] * t1, direction[2] * t1];
        let low = [centre[0] - half, centre[1] - half, centre[2] - half];
        let high = [centre[0] + half, centre[1] + half, centre[2] + half];
        ranges.clear();
        field.for_each_cell_in(low, high, |_, range| ranges.push(range));
        let mut count = 0;
        for range in &ranges {
            for index in range.clone() {
                let star = field.star(index);
                let radius = (star.position[0] * star.position[0]
                    + star.position[1] * star.position[1]
                    + star.position[2] * star.position[2])
                    .sqrt();
                if radius < t0 || radius >= t1 || radius <= f32::EPSILON {
                    continue;
                }
                let cosine = (star.position[0] * direction[0]
                    + star.position[1] * direction[1]
                    + star.position[2] * direction[2])
                    / radius;
                if (1.0 - cosine * cosine).max(0.0).sqrt() <= angular {
                    count += 1;
                }
            }
        }
        out.push(count);
        t0 = t1;
    }
    out
}

fn gather_stars(
    field: &StarField,
    direction: [f32; 3],
    enter: f32,
    exit: f32,
    support: f32,
) -> Vec<StarHit> {
    let mut hits: Vec<StarHit> = Vec::new();
    if field.count() == 0 || support <= 0.0 || exit <= enter {
        return hits;
    }
    let cell = field.grid.meta.cell;
    let mut t0 = (enter - cell).max(0.0);
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    while t0 <= exit {
        let t1 = t0 + cell;
        let half = (support + cell).max(cell);
        let angular = support / t1.max(cell);
        let centre = [direction[0] * t1, direction[1] * t1, direction[2] * t1];
        let low = [centre[0] - half, centre[1] - half, centre[2] - half];
        let high = [centre[0] + half, centre[1] + half, centre[2] + half];
        ranges.clear();
        field.for_each_cell_in(low, high, |_, range| ranges.push(range));
        for range in &ranges {
            for index in range.clone() {
                let star = field.star(index);
                let radius = (star.position[0] * star.position[0]
                    + star.position[1] * star.position[1]
                    + star.position[2] * star.position[2])
                    .sqrt();
                if radius < t0 || radius >= t1 || radius <= f32::EPSILON {
                    continue;
                }
                let cosine = (star.position[0] * direction[0]
                    + star.position[1] * direction[1]
                    + star.position[2] * direction[2])
                    / radius;
                let sine = (1.0 - cosine * cosine).max(0.0).sqrt();
                if sine > angular {
                    continue;
                }
                hits.push(StarHit {
                    radius,
                    sine,
                    power: [
                        star.brightness * star.tint[0],
                        star.brightness * star.tint[1],
                        star.brightness * star.tint[2],
                    ],
                });
            }
        }
        t0 = t1;
    }
    hits.sort_by(|a, b| {
        a.radius
            .partial_cmp(&b.radius)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.sine
                    .partial_cmp(&b.sine)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    hits
}

#[derive(Clone, Copy)]
struct Sample {
    corners: [usize; 8],
    weights: [f32; 8],
    inside: bool,
}

impl Sample {
    fn below_shell(volume: &VolumeData, point: [f32; 3]) -> Self {
        let _ = (volume, point);
        Self {
            corners: [0; 8],
            weights: [0.0; 8],
            inside: false,
        }
    }

    #[inline]
    fn gather(&self, data: &[f32], lane: usize) -> f32 {
        if !self.inside {
            return 0.0;
        }
        let mut total = 0.0_f32;
        for index in 0..8 {
            total += self.weights[index] * data[self.corners[index] + lane];
        }
        total
    }
}

#[inline]
fn snap(fraction: f32) -> f32 {
    if fraction < 1e-4 {
        0.0
    } else if fraction > 1.0 - 1e-4 {
        1.0
    } else {
        fraction
    }
}

pub fn sample_volume(volume: &VolumeData, point: [f32; 3], lane: usize) -> f32 {
    sample_at(volume, point).gather(&volume.data, lane)
}

fn sample_at(volume: &VolumeData, point: [f32; 3]) -> Sample {
    let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
    let span = volume.outer - volume.inner;
    let tolerance = span.abs().max(1.0) * 1e-5;
    if radius < volume.inner - tolerance
        || radius > volume.outer + tolerance
        || span.abs() <= f32::EPSILON
    {
        return Sample::below_shell(volume, point);
    }
    let direction = [point[0] / radius, point[1] / radius, point[2] / radius];
    let (face, s, t) = cube_face_of(direction);
    let res = volume.res.max(2);
    let layers = volume.layers.max(2);
    let last_layer = layers - 1;
    let altitude =
        px_volume_schema::volume::Shell::new(volume.inner, volume.outer).altitude_of(radius);
    let sz = altitude * last_layer as f32;
    let nearest = sz.round();
    let layer0 = if (sz - nearest).abs() < 1e-3 {
        nearest
    } else {
        sz.floor()
    };
    let tz = snap(sz - layer0);
    let sx = s * res as f32 - 0.5;
    let sy = t * res as f32 - 0.5;
    let x0 = sx.floor();
    let y0 = sy.floor();
    let tx = snap(sx - x0);
    let ty = snap(sy - y0);
    let layer_at = |step: f32| (layer0 + step).clamp(0.0, last_layer as f32) as u32;
    let (la, lb) = (layer_at(0.0), layer_at(1.0));

    let corner_slot = |cell_s: f32, cell_t: f32, layer: u32| -> usize {
        let s = (cell_s + 0.5) / res as f32;
        let t = (cell_t + 0.5) / res as f32;
        let (nf, ns, nt) = cube_face_of(px_field_schema::field::cube_direction(face, s, t));
        let cs = ((ns * res as f32) as u32).min(res - 1);
        let ct = ((nt * res as f32) as u32).min(res - 1);
        ((((nf * layers + layer.min(last_layer)) * res + ct) * res + cs) * 6) as usize
    };
    let corners = [
        corner_slot(x0, y0, la),
        corner_slot(x0 + 1.0, y0, la),
        corner_slot(x0, y0 + 1.0, la),
        corner_slot(x0 + 1.0, y0 + 1.0, la),
        corner_slot(x0, y0, lb),
        corner_slot(x0 + 1.0, y0, lb),
        corner_slot(x0, y0 + 1.0, lb),
        corner_slot(x0 + 1.0, y0 + 1.0, lb),
    ];
    let (wx, wy) = (1.0 - tx, 1.0 - ty);
    let weights = [
        wx * wy * (1.0 - tz),
        tx * wy * (1.0 - tz),
        wx * ty * (1.0 - tz),
        tx * ty * (1.0 - tz),
        wx * wy * tz,
        tx * wy * tz,
        wx * ty * tz,
        tx * ty * tz,
    ];
    Sample {
        corners,
        weights,
        inside: true,
    }
}

pub fn star_falloff(radius: f32, inner: f32) -> f32 {
    let reference = inner.max(1e-4);
    let distance = radius.max(1e-4);
    (reference / distance) * (reference / distance)
}

fn march_channel(
    emission: &VolumeData,
    stars: Option<&StarField>,
    params: &SkyParams,
    channel: usize,
    direction: [f32; 3],
    texel: u32,
) -> f32 {
    let enter = emission.inner;
    let exit = emission.outer;
    let steps = params.steps.max(1);
    let shell = px_volume_schema::volume::Shell::new(enter, exit);
    let du = 1.0 / steps as f32;
    let seed = 0x51ed_270b_u32.wrapping_add(channel as u32);
    let sigma_lane = 3 + channel.min(2);
    let lane = channel.min(2);

    let hits = match stars {
        Some(field) if params.star_gain > 0.0 => {
            gather_stars(field, direction, enter, exit, star_support(params))
        }
        _ => Vec::new(),
    };
    let mut next_hit = 0_usize;

    let mut transmittance = 1.0_f32;
    let mut radiance = 0.0_f32;
    for index in 0..steps {
        let offset = if params.jitter > 0.0 {
            jitter_at(texel, index, seed) * params.jitter
        } else {
            0.5
        };
        let distance = shell.radius_of((index as f32 + offset) * du);
        let step = shell.radius_of((index + 1) as f32 * du) - shell.radius_of(index as f32 * du);
        let point = [
            direction[0] * distance,
            direction[1] * distance,
            direction[2] * distance,
        ];
        let sample = sample_at(emission, point);
        let emit = sample.gather(&emission.data, lane);
        let sigma = sample.gather(&emission.data, sigma_lane);
        if emit > 0.0 {
            radiance += transmittance * emit * step;
        }
        if sigma > 0.0 {
            transmittance *= (-sigma * step).exp();
        }
        while next_hit < hits.len() && hits[next_hit].radius <= distance {
            let hit = &hits[next_hit];
            radiance += transmittance
                * hit.power[lane]
                * star_power(hit.sine, hit.radius, params)
                * params.star_gain
                * params.star_tint[lane]
                * star_falloff(hit.radius, enter);
            next_hit += 1;
        }
        if transmittance < 1e-4 {
            next_hit = hits.len();
            break;
        }
    }

    while next_hit < hits.len() {
        let hit = &hits[next_hit];
        radiance += transmittance
            * hit.power[lane]
            * star_power(hit.sine, hit.radius, params)
            * params.star_gain
            * params.star_tint[lane]
            * star_falloff(hit.radius, enter);
        next_hit += 1;
    }

    radiance + transmittance * params.background[lane]
}

pub fn raymarch_channel(
    emission: &VolumeData,
    stars: Option<&StarField>,
    params: &SkyParams,
    channel: usize,
) -> Result<Field, String> {
    let face = params.face.max(1);
    let height = face * CUBE_FACES;
    let data =
        px_field_schema::parallel::rows(face as usize, height as usize, |first, count, out| {
            for row in 0..count {
                let y = (first + row) as u32;
                let base = row * face as usize;
                for x in 0..face {
                    let direction = art_direction_at(Projection::CubeMap, face, height, x, y);
                    let texel = direction[0].to_bits()
                        ^ direction[1].to_bits().rotate_left(11)
                        ^ direction[2].to_bits().rotate_left(22);
                    out[base + x as usize] =
                        march_channel(emission, stars, params, channel, direction, texel);
                }
            }
        })?;
    Ok(Field::with_projection(
        face,
        height,
        data,
        Projection::CubeMap,
    ))
}

pub const RAMP_LUMA: [f32; 4] = [0.028, 0.034, 0.058, 0.35];
pub const RAMP_HUE: [[f32; 3]; 4] = [
    [1.00, 0.25, 0.55],
    [1.00, 0.42, 0.58],
    [1.00, 1.14, 2.30],
    [1.00, 0.95, 1.05],
];
pub const GRADE_STRENGTH: f32 = 0.95;

pub const TONE_IN: [f32; 4] = [0.002672, 0.014921, 0.041914, 0.110530];
pub const TONE_OUT: [f32; 4] = [0.0051, 0.0171, 0.0746, 0.2489];
const TONE_SHOULDER: f32 = 0.72;
const TONE_CEIL: f32 = 0.95;

pub fn tone_at(l: f32) -> f32 {
    tone(l)
}

pub const TONE_LIMITS: [f32; 2] = [TONE_SHOULDER, TONE_CEIL];

fn tone(l: f32) -> f32 {
    if l <= 0.0 {
        return 0.0;
    }
    let x = l.ln();
    let y = if l <= TONE_IN[0] {
        let slope = (TONE_OUT[1] / TONE_OUT[0]).ln() / (TONE_IN[1] / TONE_IN[0]).ln();
        (TONE_OUT[0].ln() + (x - TONE_IN[0].ln()) * slope).exp()
    } else if l >= TONE_IN[3] {
        let slope = (TONE_OUT[3] / TONE_OUT[2]).ln() / (TONE_IN[3] / TONE_IN[2]).ln();
        (TONE_OUT[3].ln() + (x - TONE_IN[3].ln()) * slope).exp()
    } else {
        let mut value = TONE_OUT[0];
        for stop in 0..3 {
            if l < TONE_IN[stop + 1] {
                let w = (x - TONE_IN[stop].ln()) / (TONE_IN[stop + 1].ln() - TONE_IN[stop].ln());
                value =
                    (TONE_OUT[stop].ln() + w * (TONE_OUT[stop + 1] / TONE_OUT[stop]).ln()).exp();
                break;
            }
        }
        value
    };
    if y <= TONE_SHOULDER {
        y
    } else {
        let span = TONE_CEIL - TONE_SHOULDER;
        TONE_SHOULDER + span * (1.0 - (-(y - TONE_SHOULDER) / span).exp())
    }
}

pub fn ramp_hue_at(key: f32) -> [f32; 3] {
    ramp_hue(key)
}

fn ramp_hue(key: f32) -> [f32; 3] {
    if key >= RAMP_LUMA[3] {
        return RAMP_HUE[3];
    }
    for stop in 0..3 {
        let (lo, hi) = (RAMP_LUMA[stop], RAMP_LUMA[stop + 1]);
        if key < hi {
            let raw = ((key / lo).ln() / (hi / lo).ln()).clamp(0.0, 1.0);
            let shaped = ((raw - 0.4) / 0.2).clamp(0.0, 1.0);
            let w = shaped * shaped * (3.0 - 2.0 * shaped);
            let mut target = [0.0_f32; 3];
            for channel in 0..3 {
                target[channel] = RAMP_HUE[stop][channel]
                    + (RAMP_HUE[stop + 1][channel] - RAMP_HUE[stop][channel]) * w;
            }
            return target;
        }
    }
    RAMP_HUE[3]
}

fn grade_pixel(rgb: [f32; 3]) -> [f32; 3] {
    let luma = |color: &[f32; 3]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
    let measured = luma(&rgb);
    if measured <= 1e-6 {
        return rgb;
    }
    let target = ramp_hue(measured);
    let scale = measured / luma(&target);
    let mut out = rgb;
    for channel in 0..3 {
        out[channel] += (target[channel] * scale - rgb[channel]) * GRADE_STRENGTH;
        out[channel] = out[channel].max(0.0);
    }
    out
}

pub fn raymarch_sky(
    emission: &VolumeData,
    stars: &StarField,
    sky_params: &SkyParams,
) -> Result<TextureData, String> {
    let mut planes = Vec::with_capacity(3);
    for channel in 0..3 {
        let field = raymarch_channel(emission, Some(stars), sky_params, channel)?;
        planes.push(field.data);
    }

    let line = |value: f32| -> f32 { value.max(0.0) };
    let texels = planes[0].len();
    let mut graded = vec![0.0_f32; texels * 3];
    for index in 0..texels {
        let rgb = [
            planes[0][index].max(0.0),
            planes[1][index].max(0.0),
            planes[2][index].max(0.0),
        ];
        let l = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        let response = if l > 1e-9 { tone(l) / l } else { 0.0 };
        let rgb = [rgb[0] * response, rgb[1] * response, rgb[2] * response];
        let out = grade_pixel(rgb);
        graded[index * 3] = line(out[0]);
        graded[index * 3 + 1] = line(out[1]);
        graded[index * 3 + 2] = line(out[2]);
    }

    let face = sky_params.face.max(1);
    let mut bytes = Vec::with_capacity(texels * 8);
    for index in 0..texels {
        for channel in 0..3 {
            bytes.extend_from_slice(
                &crate::half::half_from_f32(graded[index * 3 + channel]).to_le_bytes(),
            );
        }
        bytes.extend_from_slice(&crate::half::half_from_f32(1.0).to_le_bytes());
    }

    Ok(TextureData::new(
        face,
        face,
        CUBE_FACES,
        1,
        TextureFormat::Rgba16Float,
        bytes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The banding is fallible now, so a test that only wants the field says so once here.
    fn channel_field(
        emission: &VolumeData,
        stars: Option<&StarField>,
        params: &SkyParams,
        channel: usize,
    ) -> Field {
        raymarch_channel(emission, stars, params, channel).expect("测试夹具的行带不 panic")
    }
    use px_field_schema::field::Field;

    fn uniform(res: u32, layers: u32, emit: f32, alpha: f32) -> VolumeData {
        uniform_channels(res, layers, emit, [alpha, alpha, alpha])
    }

    fn uniform_channels(res: u32, layers: u32, emit: f32, alpha: [f32; 3]) -> VolumeData {
        let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res * 6) as usize];
        for chunk in data.chunks_mut(6) {
            chunk[0] = emit;
            chunk[1] = emit;
            chunk[2] = emit;
            chunk[3] = alpha[0];
            chunk[4] = alpha[1];
            chunk[5] = alpha[2];
        }
        VolumeData {
            lanes: 1,
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data,
        }
    }

    #[test]
    fn sampling_stays_continuous_across_a_face_edge() {
        let (res, layers) = (16_u32, 8_u32);
        let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res * 6) as usize];
        let mut slot = 0_usize;
        for face in 0..CUBE_FACES {
            for _layer in 0..layers {
                for t in 0..res {
                    for s in 0..res {
                        let direction = px_volume_schema::direction_of(
                            face,
                            s as f32 / (res - 1) as f32,
                            t as f32 / (res - 1) as f32,
                        );
                        let value = 0.2
                            + 0.15
                                * (7.0 * direction[0]).sin()
                                * (7.0 * direction[1]).sin()
                                * (7.0 * direction[2]).sin();
                        for lane in 0..6 {
                            data[slot + lane] = value;
                        }
                        slot += 6;
                    }
                }
            }
        }
        let volume = VolumeData {
            lanes: 1,
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data,
        };
        let t = 0.5_f32;
        let half = 0.5 / res as f32;
        let radius = 1.5_f32;
        let point = |direction: [f32; 3]| {
            [
                direction[0] * radius,
                direction[1] * radius,
                direction[2] * radius,
            ]
        };
        let left = point(px_field_schema::field::cube_direction(0, half, t));
        let right = point(px_field_schema::field::cube_direction(4, 1.0 - half, t));
        let a = sample_at(&volume, left).gather(&volume.data, 0);
        let b = sample_at(&volume, right).gather(&volume.data, 0);
        assert!(
            (a - b).abs() < 0.05,
            "棱两侧的采样值差了 {:.4}（{a:.4} 对 {b:.4}）—— 场在棱上是断的",
            (a - b).abs()
        );
    }

    fn params() -> SkyParams {
        SkyParams {
            face: 8,
            steps: 96,
            jitter: 1.0,
            background: [0.0, 0.0, 0.0],
            ..Default::default()
        }
    }

    #[test]
    fn an_optically_thin_shell_gives_emission_times_the_chord() {
        let volume = uniform(8, 6, 0.5, 0.0);
        let field = channel_field(&volume, None, &params(), 0);
        let expected = 0.5 * (volume.outer - volume.inner);
        let mut worst = 0.0_f32;
        for value in &field.data {
            worst = worst.max((value - expected).abs());
        }
        assert!(
            worst < 1e-3,
            "光学薄时应当是 {expected}（发射 × 弦长），最大偏差 {worst}"
        );
    }

    #[test]
    fn more_extinction_gives_a_dimmer_result() {
        let thin = channel_field(&uniform(8, 6, 0.5, 0.0), None, &params(), 0);
        let thick = channel_field(&uniform(8, 6, 0.5, 4.0), None, &params(), 0);
        let mean = |field: &Field| -> f64 {
            field.data.iter().map(|v| *v as f64).sum::<f64>() / field.data.len() as f64
        };
        let (thin_mean, thick_mean) = (mean(&thin), mean(&thick));
        assert!(
            thick_mean < thin_mean,
            "消光变大必须变暗：{thick_mean:.4} vs {thin_mean:.4}"
        );
        assert!(thick_mean > 0.0, "再浓也该有一点光透出来");
    }

    #[test]
    fn an_empty_volume_shows_only_the_background() {
        let volume = uniform(8, 4, 0.0, 0.0);
        let sky = SkyParams {
            background: [0.02, 0.03, 0.04],
            ..params()
        };
        let field = channel_field(&volume, None, &sky, 0);
        let mut worst = 0.0_f32;
        for value in &field.data {
            worst = worst.max((value - 0.02).abs());
        }
        assert!(worst < 1e-4, "空体积应当只剩背景 0.02，最大偏差 {worst}");
    }

    fn star_shell(count: usize, radius: f32, brightness: f32) -> StarField {
        let block = px_sparse::grid::CHUNK_CELLS;
        let cell = 0.25_f32;
        let half = radius + 2.0 * cell;
        let dims = (((2.0 * half) / cell).ceil() as u32).div_ceil(block) * block;
        let meta = px_sparse::GridMeta {
            cell,
            origin: [-(dims as f32) * cell * 0.5; 3],
            dims: [dims, dims, dims],
        };
        let golden = 2.399_963_2_f32;
        let mut positions: Vec<[f32; 3]> = Vec::with_capacity(count);
        for index in 0..count {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = golden * index as f32;
            positions.push([
                ring * phi.cos() * radius,
                ring * phi.sin() * radius,
                z * radius,
            ]);
        }
        let values = vec![brightness; count];
        let tints = vec![[1.0, 1.0, 1.0]; count];
        StarField::build(meta, &positions, &values, &tints).expect("造星壳")
    }

    #[test]
    fn a_star_behind_extinction_is_dimmer() {
        let stars = star_shell(512, 2.0, 1.0);
        let clear = channel_field(&uniform(8, 4, 0.0, 0.0), Some(&stars), &params(), 0);
        let dusty = channel_field(&uniform(8, 4, 0.0, 3.0), Some(&stars), &params(), 0);
        let mean = |field: &Field| -> f64 {
            field.data.iter().map(|v| *v as f64).sum::<f64>() / field.data.len() as f64
        };
        assert!(
            mean(&dusty) < mean(&clear) * 0.5,
            "尘埃必须把星压暗：{} vs {}",
            mean(&dusty),
            mean(&clear)
        );
    }

    #[test]
    fn the_star_light_is_uniform_across_a_face() {
        let params = SkyParams {
            face: 128,
            steps: 4,
            jitter: 0.0,
            star_gain: 1.0,
            star_halo_gain: 0.0,
            ..Default::default()
        };
        let vacuum = uniform(8, 4, 0.0, 0.0);
        let stars = star_shell(20000, 2.0, 8.0);
        let sky = channel_field(&vacuum, Some(&stars), &params, 0);
        let face = params.face;
        let mut lit = [0.0_f64; 3];
        let mut area = [0.0_f64; 3];
        let mut all = 0.0_f64;
        for face_index in 0..CUBE_FACES {
            for y in 0..face {
                for x in 0..face {
                    let s = (x as f32 + 0.5) / face as f32;
                    let t = (y as f32 + 0.5) / face as f32;
                    let a = s * 2.0 - 1.0;
                    let b = t * 2.0 - 1.0;
                    let r = (a * a + b * b).sqrt();
                    let band = if r < 0.5 {
                        0
                    } else if r < 0.9 {
                        1
                    } else {
                        2
                    };
                    let value = sky.at(x, face_index * face + y) as f64;
                    area[band] += 1.0;
                    if value > 0.05 {
                        lit[band] += 1.0;
                    }
                    all += value;
                }
            }
        }
        assert!(all > 0.0, "一颗星都没画出来");
        let centre = lit[0] / area[0];
        let corner = lit[2] / area[2];
        assert!(
            centre > 0.0 && corner > 0.0,
            "有的环带一颗星都没有（面心 {centre:.5} / 面角 {corner:.5}）"
        );
        let ratio = corner / centre;
        assert!(
            (0.9..1.1).contains(&ratio),
            "面角/面心的星点密度比是 {ratio:.3}（旧版实测 1.58）—— 星又被位置影响了"
        );
    }

    #[test]
    fn the_same_parameters_give_the_same_sky() {
        let volume = uniform(8, 5, 0.4, 1.1);
        let one = channel_field(&volume, None, &params(), 0);
        let two = channel_field(&volume, None, &params(), 0);
        assert_eq!(one.data, two.data);
    }

    #[test]
    fn per_channel_extinction_makes_blue_darker_than_red() {
        let volume = uniform_channels(8, 5, 0.5, [0.2, 1.2, 2.6]);
        let red = channel_field(&volume, None, &params(), 0);
        let green = channel_field(&volume, None, &params(), 1);
        let blue = channel_field(&volume, None, &params(), 2);
        let mean = |field: &Field| -> f64 {
            field.data.iter().map(|v| *v as f64).sum::<f64>() / field.data.len() as f64
        };
        let (r, g, b) = (mean(&red), mean(&green), mean(&blue));
        assert!(
            r > g && g > b,
            "消光越大该越暗，实际 R {r:.4} / G {g:.4} / B {b:.4}（分不开说明读错了通道）"
        );
    }

    #[test]
    fn emission_and_extinction_are_sampled_at_the_same_place() {
        let res = 8;
        let layers = 6;
        let mut volume = uniform(res, layers, 0.0, 0.0);
        for face in 0..CUBE_FACES {
            for layer in 0..layers {
                for t in 0..res {
                    for s in 0..res {
                        let slot = (((face * layers + layer) * res + t) * res + s) as usize;
                        if layer < layers / 2 {
                            volume.data[slot * 6] = 0.3;
                        } else {
                            volume.data[slot * 6 + 3] = 0.3;
                            volume.data[slot * 6 + 4] = 0.3;
                            volume.data[slot * 6 + 5] = 0.3;
                        }
                    }
                }
            }
        }
        let field = channel_field(&volume, None, &params(), 0);
        let chord = volume.outer - volume.inner;
        let mut worst_high = 0.0_f32;
        let mut saw_dimmer = false;
        for value in &field.data {
            worst_high = worst_high.max(*value - 0.3 * chord);
            if *value < 0.3 * chord * 0.99 {
                saw_dimmer = true;
            }
        }
        assert!(worst_high < 1e-3, "不该超过光学薄上界，超出 {worst_high}");
        assert!(saw_dimmer, "外层有消光 ⇒ 必须有一部分被吃掉");
    }

    #[test]
    fn grading_keeps_the_luma_and_heads_for_the_band_hue() {
        let luma = |color: &[f32; 3]| color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
        assert_eq!(grade_pixel([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        for rgb in [
            [0.004, 0.002, 0.003],
            [0.01, 0.01, 0.01],
            [0.05, 0.03, 0.06],
            [0.2, 0.16, 0.22],
            [1.5, 0.9, 1.2],
        ] {
            let out = grade_pixel(rgb);
            let drift = (luma(&out) - luma(&rgb)).abs();
            assert!(
                drift <= 1e-6 + luma(&rgb) * 1e-4,
                "{rgb:?} 分级之后亮度漂了 {drift}"
            );
        }
        let dark = grade_pixel([0.006, 0.006, 0.006]);
        assert!(
            dark[1] / dark[0] < 0.4,
            "暗档的绿没压下去（{:.3}）",
            dark[1] / dark[0]
        );
        assert!(
            dark[2] / dark[0] < 0.75,
            "暗档的蓝没压过绿（{:.3}）",
            dark[2] / dark[0]
        );
        let bright = grade_pixel([0.2, 0.2, 0.2]);
        assert!(
            bright[2] / bright[0] > 1.0,
            "亮档的蓝没高过红（{:.3}）",
            bright[2] / bright[0]
        );
        assert!(
            bright[1] / bright[0] > 0.7,
            "亮档的绿没抬起来（{:.3}）",
            bright[1] / bright[0]
        );
        let a = grade_pixel([0.05, 0.05, 0.05]);
        let b = grade_pixel([0.0796, 0.0318, 0.1432]);
        assert!(
            (luma(&a) - luma(&b)).abs() < 1e-4,
            "配平写错了：两格亮度不同（{:.5} vs {:.5}）",
            luma(&a),
            luma(&b)
        );
        assert!(
            (a[2] / a[0] - b[2] / b[0]).abs() < 0.1,
            "等亮度的两格档位不一致（{:.2} vs {:.2}）—— 键漂了",
            a[2] / a[0],
            b[2] / b[0]
        );

        assert_eq!(tone(0.0), 0.0);
        let mut previous = 0.0_f32;
        for step in 0..200_usize {
            let l = step as f32 * 0.01;
            let y = tone(l);
            assert!(y >= previous, "tone 不单调（{l} 处 {y} < {previous}）");
            assert!(y <= TONE_CEIL, "tone 撞顶（{l} 处 {y}）");
            previous = y;
        }
    }
}
