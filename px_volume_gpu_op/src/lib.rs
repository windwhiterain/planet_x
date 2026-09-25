//! See docs/volume.md

pub const SAMPLER_WGSL: &str = include_str!("sampler.wgsl");

use px_gpu::{Binding, connect, dispatch};

pub mod occupancy;
pub use occupancy::Occupancy;

pub const LANES: usize = 6;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OccupancyUniform {
    pub extra: [u32; 4],
    pub scalars: [f32; 4],
}

impl OccupancyUniform {
    pub fn none() -> Self {
        Self {
            extra: [0, 0, 0, 0],
            scalars: [0.0; 4],
        }
    }

    pub fn packed(occupancy: &Occupancy, inner: f32, outer: f32) -> (Self, Vec<u8>) {
        let ratio = if inner > 0.0 {
            (outer / inner).ln()
        } else {
            0.0
        };
        let words = occupancy.upload_words();
        (
            Self {
                extra: [
                    occupancy.res,
                    occupancy.layers,
                    occupancy.blocks_per_face(),
                    1,
                ],
                scalars: [ratio, 0.0, occupancy.l1_words() as f32, 0.0],
            },
            u32_bytes(&words),
        )
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        for value in self.extra {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.scalars {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }
}

fn occupancy_bytes(
    occupancy: Option<&Occupancy>,
    res: u32,
    layers: u32,
    inner: f32,
    outer: f32,
) -> (OccupancyUniform, Vec<u8>) {
    match occupancy {
        Some(occupancy) => OccupancyUniform::packed(occupancy, inner, outer),
        None => (
            OccupancyUniform {
                extra: [res, layers, 0, 0],
                scalars: [0.0; 4],
            },
            u32_bytes(&[]),
        ),
    }
}

pub fn flat_index(
    res: u32,
    layers: u32,
    face: u32,
    layer: u32,
    t: u32,
    s: u32,
    lane: u32,
) -> usize {
    (((((face * layers + layer) * res + t) * res + s) * LANES as u32) + lane) as usize
}

pub const REPACK_WGSL: &str = r#"
struct Shape {
    res: u32,
    layers: u32,
    lanes: u32,
    faces: u32,
};

@group(0) @binding(0) var<uniform> shape: Shape;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;

@compute @workgroup_size(64)
fn repack(@builtin(global_invocation_id) id: vec3<u32>) {
    let cells = shape.faces * shape.layers * shape.res * shape.res;
    if (id.x >= cells * shape.lanes) {
        return;
    }
    let lane = id.x % shape.lanes;
    let cell = id.x / shape.lanes;
    let s = cell % shape.res;
    let t = (cell / shape.res) % shape.res;
    let layer = (cell / (shape.res * shape.res)) % shape.layers;
    let face = cell / (shape.res * shape.res * shape.layers);
    let at = (((face * shape.layers + layer) * shape.res + t) * shape.res + s) * shape.lanes + lane;
    dst[id.x] = src[at];
}
"#;

pub fn workgroups(threads: usize) -> (u32, u32, u32) {
    const WG_X: usize = 65535;
    let blocks = threads.div_ceil(64).max(1);
    let x = blocks.min(WG_X);
    let y = blocks.div_ceil(WG_X);
    (x as u32, y as u32, 1)
}

pub fn repack(res: u32, layers: u32, faces: u32, data: &[f32]) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let shape = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        faces.to_le_bytes(),
    ]
    .concat();
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let src = bytes(data);
    let out = dispatch(
        gpu,
        REPACK_WGSL,
        "repack",
        &[
            Binding::Uniform(&shape),
            Binding::Storage(&src),
            Binding::Write(&vec![0_u8; src.len()]),
        ],
        workgroups(data.len()),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gpu_agrees_with_the_cpu_on_the_flat_index() {
        let (res, layers, faces) = (4_u32, 3_u32, 6_u32);
        let cells = (faces * layers * res * res) as usize;
        let data: Vec<f32> = (0..cells * LANES).map(|index| index as f32).collect();
        let gpu_side = px_gpu::require_gpu(repack(res, layers, faces, &data));
        assert_eq!(gpu_side.len(), data.len(), "长度");

        let mut index = 0_usize;
        for face in 0..faces {
            for layer in 0..layers {
                for t in 0..res {
                    for s in 0..res {
                        for lane in 0..LANES as u32 {
                            let want = flat_index(res, layers, face, layer, t, s, lane) as f32;
                            assert_eq!(
                                gpu_side[index], want,
                                "第 {index} 项：GPU 给了 {}，CPU 期望 {want}（face {face} 层 {layer} t {t} s {s} 通道 {lane}）",
                                gpu_side[index]
                            );
                            index += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(index, data.len(), "必须逐格都对过");
    }
}

pub fn sample_points(
    res: u32,
    layers: u32,
    inner: f32,
    outer: f32,
    data: &[f32],
    points: &[[f32; 3]],
    lane: u32,
) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let uniform = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        lane.to_le_bytes(),
        inner.to_le_bytes(),
        outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let flat: Vec<f32> = points.iter().flat_map(|p| p.iter().copied()).collect();
    let output = vec![0_u8; points.len() * 4];
    let out = dispatch(
        gpu,
        SAMPLER_WGSL,
        "sample_points",
        &[
            Binding::Uniform(&uniform),
            Binding::Storage(&bytes(data)),
            Binding::Storage(&bytes(&flat)),
            Binding::Write(&output),
        ],
        workgroups(points.len()),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod sampler_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    fn smooth_volume(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for face in 0..6_u32 {
            for layer in 0..layers {
                let altitude = layer as f32 / (layers - 1).max(1) as f32;
                let radius = inner + (outer - inner) * altitude;
                for t in 0..res {
                    for s in 0..res {
                        let d = px_volume_schema::direction_of(
                            face,
                            (s as f32 + 0.5) / res as f32,
                            (t as f32 + 0.5) / res as f32,
                        );
                        let value = 0.3
                            + 0.2 * (5.0 * d[0]).sin() * (5.0 * d[1]).sin() * (5.0 * d[2]).sin()
                            + 0.05 * (radius - inner);
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = value * radius;
                        }
                    }
                }
            }
        }
        VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        }
    }

    #[test]
    fn the_gpu_sampler_agrees_with_the_cpu_point_by_point() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let volume = smooth_volume(res, layers, inner, outer);
        let mut points: Vec<[f32; 3]> = Vec::new();
        for face in 0..6_u32 {
            for &s in &[0.0_f32, 0.13, 0.5, 0.87, 1.0] {
                for &t in &[0.0_f32, 0.13, 0.5, 0.87, 1.0] {
                    for &radius in &[inner + 0.01, 1.5, outer - 0.01] {
                        let d = px_volume_schema::direction_of(face, s, t);
                        points.push([d[0] * radius, d[1] * radius, d[2] * radius]);
                    }
                }
            }
        }
        let gpu_side = px_gpu::require_gpu(sample_points(
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &points,
            0,
        ));
        assert_eq!(gpu_side.len(), points.len(), "点数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0;
        for (index, point) in points.iter().enumerate() {
            let want = px_volume_alg::sample_volume(&volume, *point, 0);
            let got = gpu_side[index];
            let diff = (got - want).abs();
            if diff > worst {
                worst = diff;
                worst_at = index;
            }
        }
        assert!(
            worst < 2e-3,
            "最大偏差 {worst:.6} 在第 {worst_at} 个点（GPU {} 对 CPU {}）—— 两侧的采样语义不一致",
            gpu_side[worst_at],
            px_volume_alg::sample_volume(&volume, points[worst_at], 0)
        );
        println!("px_volume_gpu_op：{} 个点最大偏差 {worst:.6}", points.len());
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
/// Lane-packed scalars: `scalars[0]` is the step jitter in `[0, 1]`, `scalars[3]` is the ray entry
/// radius. [`SkyUniform::to_bytes`] is the layout `sampler.wgsl`'s `Sky` mirrors.
pub struct SkyUniform {
    pub counts: [u32; 4],
    pub scalars: [f32; 4],
    pub background: [f32; 4],
    pub tone_in: [f32; 4],
    pub tone_out: [f32; 4],
    pub ramp_luma: [f32; 4],
    pub ramp_hue: [[f32; 4]; 4],
    pub limits: [f32; 4],
}

impl SkyUniform {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(std::mem::size_of::<Self>());
        for value in self.counts {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.scalars {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for group in [self.background, self.tone_in, self.tone_out, self.ramp_luma] {
            for value in group {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for hue in self.ramp_hue {
            for value in hue {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        for value in self.limits {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }
}

#[cfg(test)]
mod uniform_tests {
    use super::*;

    #[test]
    fn the_sky_uniform_layout_is_pinned() {
        let value = SkyUniform {
            counts: [96, 1024, 0, 0],
            scalars: [1.0, 0.024, 0.4, 1.0],
            background: [0.0006, 0.0004, 0.0005, 0.0],
            tone_in: [0.002672, 0.014921, 0.041914, 0.110530],
            tone_out: [0.0051, 0.0171, 0.0746, 0.2489],
            ramp_luma: [0.028, 0.034, 0.12, 0.35],
            limits: [0.72, 0.95, 0.0, 0.0],
            ramp_hue: [
                [1.0, 0.24, 0.41, 0.0],
                [1.0, 0.42, 0.58, 0.0],
                [1.0, 1.14, 2.30, 0.0],
                [1.0, 0.95, 1.05, 0.0],
            ],
        };
        let bytes = value.to_bytes();
        assert_eq!(
            bytes.len(),
            std::mem::size_of::<SkyUniform>(),
            "逐字段导出应当正好等于内存布局"
        );
        assert_eq!(
            bytes.len() % 16,
            0,
            "uniform 块必须是 16 的倍数，实际 {}",
            bytes.len()
        );
        assert_eq!(
            bytes.len(),
            176,
            "字段变了就要同步 WGSL 的 struct（现为 16*7+64）"
        );
    }

    #[test]
    fn the_star_meta_layout_is_pinned() {
        let value = StarMeta {
            counts: [7, 32, 32, 48],
            space: [0.05, -0.8, -0.8, -1.2],
            segments_a: [0, 12, 44, 900],
            segments_b: [1200, 12, 8, 0],
            light: [0.2, 0.05, 1.6, 0.0],
            profile: [0.0029, 0.01, 0.035, 0.03],
        };
        let bytes = value.to_bytes();
        assert_eq!(
            bytes.len(),
            std::mem::size_of::<StarMeta>(),
            "逐字段导出应当正好等于内存布局"
        );
        assert_eq!(bytes.len() % 16, 0, "uniform 块必须是 16 的倍数");
        assert_eq!(
            bytes.len(),
            96,
            "字段变了就要同步 WGSL 的 struct StarMeta（现为 6 个 vec4）"
        );
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 48);
        assert_eq!(f32::from_le_bytes(bytes[16..20].try_into().unwrap()), 0.05);
        assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 0);
        assert_eq!(
            f32::from_le_bytes(bytes[80..84].try_into().unwrap()),
            0.0029
        );
    }
}

pub struct StarGrid {
    pub index: Vec<u32>,
    pub starts: [u32; 5],
    pub cell: f32,
    pub origin: [f32; 3],
    pub dims: [u32; 3],
    pub count: u32,
}

impl StarGrid {
    pub fn of(field: &px_sparse::StarField) -> Self {
        let grid = &field.grid;
        let mut index: Vec<u32> = Vec::with_capacity(
            grid.chunk_start.len()
                + grid.brick_slot.len()
                + grid.brick_mask.len()
                + grid.brick_sub.len()
                + grid.sub_start.len(),
        );
        let mut starts = [0_u32; 5];
        for (segment, values) in [
            &grid.chunk_start,
            &grid.brick_slot,
            &grid.brick_mask,
            &grid.brick_sub,
            &grid.sub_start,
        ]
        .into_iter()
        .enumerate()
        {
            starts[segment] = index.len() as u32;
            index.extend_from_slice(values);
        }
        Self {
            index,
            starts,
            cell: grid.meta.cell,
            origin: grid.meta.origin,
            dims: grid.meta.dims,
            count: field.count() as u32,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct StarMeta {
    pub counts: [u32; 4],
    pub space: [f32; 4],
    pub segments_a: [u32; 4],
    pub segments_b: [u32; 4],
    pub light: [f32; 4],
    pub profile: [f32; 4],
}

impl StarMeta {
    fn of(grid: &StarGrid) -> Self {
        Self {
            counts: [grid.count, grid.dims[0], grid.dims[1], grid.dims[2]],
            space: [grid.cell, grid.origin[0], grid.origin[1], grid.origin[2]],
            segments_a: [
                grid.starts[0],
                grid.starts[1],
                grid.starts[2],
                grid.starts[3],
            ],
            segments_b: [grid.starts[4], 0, 0, 0],
            light: [0.0; 4],
            profile: [0.0; 4],
        }
    }

    pub fn sky(grid: &StarGrid, params: &px_volume_schema::params::sky::SkyParams) -> Self {
        Self {
            segments_b: [grid.starts[4], 0, 0, 0],
            light: [0.0, 0.0, params.star_gain, 0.0],
            profile: [
                params.star_core.max(1e-6),
                params.star_halo.max(1e-6),
                params.star_halo_gain,
                px_volume_alg::raymarch::star_support(params),
            ],
            ..Self::of(grid)
        }
    }

    pub fn emission(
        grid: &StarGrid,
        params: &px_volume_schema::params::emission::EmissionParams,
    ) -> Self {
        Self {
            segments_b: [
                grid.starts[4],
                params.starlight_steps.max(1),
                params.starlight_max,
                0,
            ],
            light: [
                params.starlight_radius.max(1e-4),
                params.starlight_soft.max(1e-4),
                params.starlight_gain,
                0.0,
            ],
            ..Self::of(grid)
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(std::mem::size_of::<Self>());
        for value in self.counts {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.space {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.segments_a {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.segments_b {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.light {
            out.extend_from_slice(&value.to_le_bytes());
        }
        for value in self.profile {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }
}

pub const STAR_KEEP_MAX: u32 = 32;

#[derive(Default, Clone, Copy)]
pub struct MarchExtras<'a> {
    pub star_grid: Option<&'a StarGrid>,
    pub star_table: &'a [f32],
    pub star_meta: StarMeta,
    pub background: [f32; 3],
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    if values.is_empty() {
        return vec![0_u8; 4];
    }
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    if values.is_empty() {
        return vec![0_u8; 4];
    }
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub struct StarSlots {
    pub table: Vec<u8>,
    pub index: Vec<u8>,
    pub meta: Vec<u8>,
}

impl StarSlots {
    pub fn of(extras: &MarchExtras<'_>) -> Self {
        match extras.star_grid {
            Some(grid) => Self {
                table: f32_bytes(extras.star_table),
                index: u32_bytes(&grid.index),
                meta: extras.star_meta.to_bytes(),
            },
            None => Self {
                table: f32_bytes(&[]),
                index: u32_bytes(&[]),
                meta: StarMeta::default().to_bytes(),
            },
        }
    }
}

fn check_star_overflow(readback: &[u8]) -> Result<(), String> {
    let count = u32::from_le_bytes(readback[0..4].try_into().unwrap());
    if count > 0 {
        return Err(format!(
            "星候选的定长队列溢出 {count} 次（`STAR_PENDING_MAX` 装不下某一层里的星）"
        ));
    }
    Ok(())
}

pub fn tone_of(
    values: &[f32],
    tone_in: [f32; 4],
    tone_out: [f32; 4],
    limits: [f32; 2],
) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let sky = SkyUniform {
        counts: [0; 4],
        scalars: [0.0; 4],
        background: [0.0; 4],
        tone_in,
        tone_out,
        ramp_luma: [0.0; 4],
        ramp_hue: [[0.0; 4]; 4],
        limits: [limits[0], limits[1], 0.0, 0.0],
    };
    let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "tone_of",
        &[
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&bytes),
            },
        ],
        workgroups(values.len()),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod tone_tests {
    use super::*;

    #[test]
    fn the_gpu_tone_matches_the_cpu() {
        let mut values: Vec<f32> = Vec::new();
        let mut l = 1e-5_f32;
        while l < 2.0 {
            values.push(l);
            values.push(l * 1.0007);
            l *= 1.13;
        }
        for extra in [0.0_f32, 1e-9, 0.72, 0.95, 1.0, 5.0] {
            values.push(extra);
        }
        let anchors_in = px_volume_alg::raymarch::TONE_IN;
        let anchors_out = px_volume_alg::raymarch::TONE_OUT;
        let limits = px_volume_alg::TONE_LIMITS;
        let gpu_side = px_gpu::require_gpu(tone_of(&values, anchors_in, anchors_out, limits));
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, value) in values.iter().enumerate() {
            let want = px_volume_alg::tone_at(*value);
            let got = gpu_side[index];
            let diff = (got - want).abs() / want.abs().max(1e-3);
            if diff > worst {
                worst = diff;
                worst_at = index;
            }
        }
        assert!(
            worst < 3e-3,
            "响应曲线最大相对偏差 {worst:.6} 在 luma {}（GPU {} 对 CPU {}）",
            values[worst_at],
            gpu_side[worst_at],
            px_volume_alg::tone_at(values[worst_at])
        );
        println!(
            "px_volume_gpu_op：响应曲线最大相对偏差 {worst:.6}（{} 点）",
            values.len()
        );
    }
}

pub fn march(
    face: u32,
    steps: u32,
    lane: u32,
    jitter: f32,
    enter: f32,
    res: u32,
    layers: u32,
    inner: f32,
    outer: f32,
    data: &[f32],
    extras: &MarchExtras<'_>,
    occupancy: Option<&Occupancy>,
) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let volume_uniform = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        0_u32.to_le_bytes(),
        inner.to_le_bytes(),
        outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let (occupancy_uniform, occupancy_words) =
        occupancy_bytes(occupancy, res, layers, inner, outer);
    let skip = if occupancy.is_some() { 1 } else { 0 };
    let sky = SkyUniform {
        counts: [steps, face, lane, skip],
        scalars: [jitter.clamp(0.0, 1.0), 0.0, 0.0, enter],
        background: [
            extras.background[0],
            extras.background[1],
            extras.background[2],
            0.0,
        ],
        tone_in: [0.0; 4],
        tone_out: [0.0; 4],
        ramp_luma: [0.0; 4],
        ramp_hue: [[0.0; 4]; 4],
        limits: [0.0; 4],
    };
    let texels = (face * face * 6) as usize;
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let stars = StarSlots::of(extras);
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "march",
        &[
            px_gpu::Slot {
                binding: 0,
                value: Binding::Uniform(&volume_uniform),
            },
            px_gpu::Slot {
                binding: 1,
                value: Binding::Storage(&bytes(data)),
            },
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&vec![0_u8; texels * 4]),
            },
            px_gpu::Slot {
                binding: 6,
                value: Binding::Storage(&stars.table),
            },
            px_gpu::Slot {
                binding: 10,
                value: Binding::Storage(&stars.index),
            },
            px_gpu::Slot {
                binding: 11,
                value: Binding::Uniform(&stars.meta),
            },
            px_gpu::Slot {
                binding: 12,
                value: Binding::Write(&vec![0_u8; 4]),
            },
            px_gpu::Slot {
                binding: 13,
                value: Binding::Uniform(&occupancy_uniform.to_bytes()),
            },
            px_gpu::Slot {
                binding: 14,
                value: Binding::Storage(&occupancy_words),
            },
            px_gpu::Slot {
                binding: 15,
                value: Binding::Write(&vec![0_u8; 16 * 4]),
            },
        ],
        workgroups(texels),
    )?;
    {
        let probe: Vec<f32> = out[2]
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect();
        println!(
            "march 探针：w={} 进入 {} 次 / 调用 {} 次 / L1 活 {} 次 / 进块 {} 次",
            probe[6], probe[7], probe[4], probe[5], probe[3]
        );
    }
    check_star_overflow(&out[1])?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

#[cfg(test)]
mod march_tests {
    use super::*;

    #[test]
    fn the_march_converges_to_the_analytic_solution() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let data = vec![1.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        let gpu_side = px_gpu::require_gpu(march(
            8,
            256,
            0,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &data,
            &MarchExtras::default(),
            None,
        ));
        assert_eq!(gpu_side.len(), 8 * 8 * 6, "texel 数");
        println!(
            "步进读数（前 6 个 / 均值）：{:?} / {}",
            &gpu_side[..6],
            gpu_side.iter().sum::<f32>() / gpu_side.len() as f32
        );
        let want = 1.0 - (-1.0_f32).exp();
        let worst = gpu_side
            .iter()
            .map(|value| (value - want).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            worst < 2e-3,
            "步进结果应当收敛到解析解 {want:.6}，最大偏差 {worst:.6}"
        );
        println!("px_volume_gpu_op：步进 {worst:.6} 偏差（解析解 {want:.6}）");
    }
}

#[cfg(test)]
mod crosscheck_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    fn varying_volume(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for face in 0..6_u32 {
            for layer in 0..layers {
                let altitude = layer as f32 / (layers - 1).max(1) as f32;
                let _radius = inner + (outer - inner) * altitude;
                for t in 0..res {
                    for s in 0..res {
                        let d = px_volume_schema::direction_of(
                            face,
                            (s as f32 + 0.5) / res as f32,
                            (t as f32 + 0.5) / res as f32,
                        );
                        let wave = (3.0 * d[0]).sin() * (4.0 * d[1]).cos() * (5.0 * d[2]).sin();
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = match lane {
                                0..=2 => (0.4 + 0.3 * wave).max(0.0) * (1.0 + 0.1 * lane as f32),
                                _ => (0.2 + 0.5 * (wave * 0.5 + 0.5)).max(0.0),
                            };
                        }
                    }
                }
            }
        }
        VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        }
    }

    #[test]
    fn the_gpu_march_matches_the_cpu_channel_on_a_real_volume() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let volume = varying_volume(res, layers, inner, outer);
        let face = 8_u32;
        let steps = 64_u32;
        let params = px_volume_schema::params::sky::SkyParams {
            face,
            steps,
            jitter: 0.0,
            ..Default::default()
        };
        let reference = px_volume_alg::raymarch_channel(&volume, None, &params, 0);
        let gpu_side = px_gpu::require_gpu(march(
            face,
            steps,
            0,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &MarchExtras::default(),
            None,
        ));
        assert_eq!(gpu_side.len(), reference.data.len(), "texel 数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, value) in gpu_side.iter().enumerate() {
            let want = reference.data[index];
            if (value - want).abs() > worst {
                worst = (value - want).abs();
                worst_at = index;
            }
        }
        assert!(
            worst < 5e-3,
            "最大偏差 {worst:.6} 在第 {worst_at} 个 texel（GPU {} 对 CPU {}）",
            gpu_side[worst_at],
            reference.data[worst_at]
        );
        println!(
            "px_volume_gpu_op：真体积对账最大偏差 {worst:.6}（{} 个 texel）",
            gpu_side.len()
        );
    }

    fn blocky_volume(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for face in 0..6_u32 {
            for layer in 8..16_u32.min(layers) {
                for t in 0..8_u32.min(res) {
                    for s in 0..8_u32.min(res) {
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = match lane {
                                0..=2 => 0.6 + 0.1 * lane as f32,
                                _ => 0.3,
                            };
                        }
                    }
                }
            }
        }
        VolumeData {
            res,
            layers,
            inner,
            outer,
            lanes: 6,
            data,
        }
    }

    #[test]
    fn the_skip_matches_the_dense_march_on_a_blocky_volume() {
        let (res, layers) = (16_u32, 24_u32);
        let (inner, outer) = (1.0_f32, 3.0_f32);
        let volume = blocky_volume(res, layers, inner, outer);
        let occupancy = Occupancy::from_flat(res, layers, LANES, &volume.data);
        let marked = occupancy
            .sidecar()
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum::<usize>();
        assert!(
            marked < occupancy.sidecar().len() * 32 / 2,
            "夹具必须是**大部分空**的（存活 {marked} / {} 位）",
            occupancy.sidecar().len() * 32
        );
        let (face, steps) = (8_u32, 96_u32);
        let dense = march(
            face,
            steps,
            0,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &MarchExtras::default(),
            None,
        );
        let skipped = march(
            face,
            steps,
            0,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &MarchExtras::default(),
            Some(&occupancy),
        );
        let (dense, skipped) = match (dense, skipped) {
            (Ok(dense), Ok(skipped)) => (dense, skipped),
            (Err(err), _) | (_, Err(err)) => {
                panic!(
                    "步进派发失败（没有 GPU 与着色器编不过都走到这里，先看上一行的 px_gpu 报错）：{err}"
                )
            }
        };
        assert_eq!(dense.len(), skipped.len(), "texel 数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        let mut dense_mean = 0.0_f64;
        for (index, value) in dense.iter().enumerate() {
            dense_mean += *value as f64;
            if (value - skipped[index]).abs() > worst {
                worst = (value - skipped[index]).abs();
                worst_at = index;
            }
        }
        dense_mean /= dense.len() as f64;
        assert!(dense_mean > 0.0, "夹具一片黑 ⇒ 这条判据什么都没测");
        assert!(
            worst < 2e-2,
            "空跳与密集步进的最大偏差 {worst:.6}（第 {worst_at} 个 texel：{} 对 {}）—— \
             空块不空，或者有内容的块里样本错位",
            dense[worst_at],
            skipped[worst_at]
        );
        println!(
            "px_volume_gpu_op：空跳对账最大偏差 {worst:.6}（{} 个 texel，均值 {dense_mean:.5}）",
            dense.len()
        );
    }
}

#[cfg(test)]
fn empty_star_field() -> px_sparse::StarField {
    let block = px_sparse::CHUNK_CELLS;
    px_sparse::StarField::build(
        px_sparse::GridMeta {
            cell: 0.5,
            origin: [-4.0; 3],
            dims: [block, block, block],
        },
        &[],
        &[],
        &[],
    )
    .expect("造空星场")
}

#[cfg(test)]
fn fixture_star_field(
    count: usize,
    cell: f32,
    inner: f32,
    outer: f32,
    brightness: f32,
) -> px_sparse::StarField {
    let mut positions = Vec::with_capacity(count);
    let golden = 2.399_963_2_f32;
    for index in 0..count {
        let z = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        let phi = golden * index as f32;
        let draw = ((index.wrapping_mul(2654435761)) % 1000) as f32 / 1000.0;
        let cube = inner * inner * inner + draw * (outer * outer * outer - inner * inner * inner);
        let radius = cube.max(0.0).cbrt();
        positions.push([
            ring * phi.cos() * radius,
            ring * phi.sin() * radius,
            z * radius,
        ]);
    }
    let values = vec![brightness; count];
    let tints: Vec<[f32; 3]> = (0..count)
        .map(|index| {
            if index % 3 == 0 {
                [0.72, 0.86, 1.0]
            } else {
                [1.0, 1.0, 1.0]
            }
        })
        .collect();
    let block = px_sparse::CHUNK_CELLS;
    let half = outer + 2.0 * cell;
    let dims = (((2.0 * half) / cell).ceil() as u32).div_ceil(block) * block;
    px_sparse::StarField::build(
        px_sparse::GridMeta {
            cell,
            origin: [-(dims as f32) * cell * 0.5; 3],
            dims: [dims, dims, dims],
        },
        &positions,
        &values,
        &tints,
    )
    .expect("造星场")
}

#[cfg(test)]
mod star_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    #[test]
    fn the_gpu_march_matches_the_cpu_with_stars_and_background() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for lane in 0..LANES {
            for (index, value) in data.iter_mut().enumerate() {
                if index % LANES == lane {
                    *value = if lane < 3 { 0.5 } else { 0.8 };
                }
            }
        }
        let volume = VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        };
        let stars = fixture_star_field(1500, 0.25, inner, outer, 8.0);
        let params = px_volume_schema::params::sky::SkyParams {
            face: 8,
            steps: 32,
            jitter: 0.0,
            star_gain: 0.05,
            background: [0.0011, 0.0007, 0.0009],
            ..Default::default()
        };
        let reference = px_volume_alg::raymarch_channel(&volume, Some(&stars), &params, 0);
        let grid = StarGrid::of(&stars);
        let extras = MarchExtras {
            star_grid: Some(&grid),
            star_table: &stars.stars,
            star_meta: StarMeta::sky(&grid, &params),
            background: params.background,
        };
        let gpu_side = px_gpu::require_gpu(march(
            params.face,
            params.steps,
            0,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &extras,
            None,
        ));
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, value) in gpu_side.iter().enumerate() {
            let diff =
                (value - reference.data[index]).abs() / reference.data[index].abs().max(1e-2);
            if diff > worst {
                worst = diff;
                worst_at = index;
            }
        }
        println!(
            "px_volume_gpu_op：星点+底色最大相对偏差 {worst:.6}（第 {worst_at} 个 texel：GPU {} 对 CPU {}）",
            gpu_side[worst_at], reference.data[worst_at]
        );
        assert!(worst < 3e-2, "星点+底色对账最大相对偏差 {worst:.6}");
    }
}

pub fn hue_of(
    keys: &[f32],
    ramp_luma: [f32; 4],
    ramp_hue_table: [[f32; 4]; 4],
) -> Result<Vec<[f32; 3]>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let sky = SkyUniform {
        counts: [0; 4],
        scalars: [0.0; 4],
        background: [0.0; 4],
        tone_in: [0.0; 4],
        tone_out: [0.0; 4],
        ramp_luma,
        ramp_hue: ramp_hue_table,
        limits: [0.0; 4],
    };
    let mut packed: Vec<f32> = Vec::with_capacity(keys.len() * 3);
    for key in keys {
        packed.extend_from_slice(&[*key, 0.0, 0.0]);
    }
    let bytes: Vec<u8> = packed.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "hue_of_keys",
        &[
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&bytes),
            },
        ],
        workgroups(keys.len()),
    )?;
    Ok(out[0]
        .chunks_exact(12)
        .map(|chunk| {
            [
                f32::from_le_bytes(chunk[0..4].try_into().unwrap()),
                f32::from_le_bytes(chunk[4..8].try_into().unwrap()),
                f32::from_le_bytes(chunk[8..12].try_into().unwrap()),
            ]
        })
        .collect())
}

#[cfg(test)]
mod hue_tests {
    use super::*;

    #[test]
    fn the_gpu_hue_ramp_matches_the_cpu() {
        let ramp_luma = px_volume_alg::raymarch::RAMP_LUMA;
        let table = px_volume_alg::raymarch::RAMP_HUE;
        let ramp_hue_table = [
            [table[0][0], table[0][1], table[0][2], 0.0],
            [table[1][0], table[1][1], table[1][2], 0.0],
            [table[2][0], table[2][1], table[2][2], 0.0],
            [table[3][0], table[3][1], table[3][2], 0.0],
        ];
        let mut keys: Vec<f32> = Vec::new();
        let mut key = 1e-4_f32;
        while key < 1.0 {
            keys.push(key);
            key *= 1.07;
        }
        for extra in [0.0_f32, 0.0279, 0.028, 0.034, 0.058, 0.35, 2.0] {
            keys.push(extra);
        }
        let gpu_side = px_gpu::require_gpu(hue_of(&keys, ramp_luma, ramp_hue_table));
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for (index, key) in keys.iter().enumerate() {
            let want = px_volume_alg::ramp_hue_at(*key);
            for channel in 0..3 {
                let diff = (gpu_side[index][channel] - want[channel]).abs();
                if diff > worst {
                    worst = diff;
                    worst_at = index;
                }
            }
        }
        assert!(
            worst < 3e-3,
            "档位色相最大偏差 {worst:.6} 在键 {}（GPU {:?} 对 CPU {:?}）",
            keys[worst_at],
            gpu_side[worst_at],
            px_volume_alg::ramp_hue_at(keys[worst_at])
        );
        println!(
            "px_volume_gpu_op：档位色相最大偏差 {worst:.6}（{} 个键）",
            keys.len()
        );
    }
}

#[allow(clippy::too_many_arguments)]
pub fn sky(
    face: u32,
    steps: u32,
    jitter: f32,
    enter: f32,
    res: u32,
    layers: u32,
    inner: f32,
    outer: f32,
    data: &[f32],
    extras: &MarchExtras<'_>,
    tone: ([f32; 4], [f32; 4], [f32; 2]),
    ramp: ([f32; 4], [[f32; 4]; 4]),
    grade_strength: f32,
    occupancy: Option<&Occupancy>,
) -> Result<Vec<f32>, String> {
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let volume_uniform = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        0_u32.to_le_bytes(),
        inner.to_le_bytes(),
        outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let (occupancy_uniform, occupancy_words) =
        occupancy_bytes(occupancy, res, layers, inner, outer);
    let skip = if occupancy.is_some() { 1 } else { 0 };
    let sky_uniform = SkyUniform {
        counts: [steps, face, 0, skip],
        scalars: [jitter.clamp(0.0, 1.0), 0.0, 0.0, enter],
        background: [
            extras.background[0],
            extras.background[1],
            extras.background[2],
            0.0,
        ],
        tone_in: tone.0,
        tone_out: tone.1,
        ramp_luma: ramp.0,
        ramp_hue: ramp.1,
        limits: [tone.2[0], tone.2[1], grade_strength, 0.0],
    };
    let texels = (face * face * 6) as usize;
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let stars = StarSlots::of(extras);
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "sky_radiance",
        &[
            px_gpu::Slot {
                binding: 0,
                value: Binding::Uniform(&volume_uniform),
            },
            px_gpu::Slot {
                binding: 1,
                value: Binding::Storage(&bytes(data)),
            },
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky_uniform.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&vec![0_u8; texels * 12]),
            },
            px_gpu::Slot {
                binding: 6,
                value: Binding::Storage(&stars.table),
            },
            px_gpu::Slot {
                binding: 10,
                value: Binding::Storage(&stars.index),
            },
            px_gpu::Slot {
                binding: 11,
                value: Binding::Uniform(&stars.meta),
            },
            px_gpu::Slot {
                binding: 12,
                value: Binding::Write(&vec![0_u8; 4]),
            },
            px_gpu::Slot {
                binding: 13,
                value: Binding::Uniform(&occupancy_uniform.to_bytes()),
            },
            px_gpu::Slot {
                binding: 14,
                value: Binding::Storage(&occupancy_words),
            },
        ],
        workgroups(texels),
    )?;
    check_star_overflow(&out[1])?;
    let radiance = out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect::<Vec<f32>>();
    grade_pixels(gpu, &radiance, &sky_uniform)
}

fn grade_pixels(
    gpu: &px_gpu::Gpu,
    radiance: &[f32],
    sky_uniform: &SkyUniform,
) -> Result<Vec<f32>, String> {
    let bytes: Vec<u8> = radiance.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "grade_pixels",
        &[
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky_uniform.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&bytes),
            },
        ],
        workgroups(radiance.len() / 3),
    )?;
    Ok(out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect())
}

pub fn anchors_from_radiance(
    gpu: &px_gpu::Gpu,
    radiance: &[f32],
    sky_uniform: &SkyUniform,
) -> Result<[f32; 4], String> {
    const BIN_COUNT: usize = 512;
    let bytes: Vec<u8> = radiance.iter().flat_map(|v| v.to_le_bytes()).collect();
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "bin_luma",
        &[
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&sky_uniform.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&bytes),
            },
            px_gpu::Slot {
                binding: 7,
                value: Binding::Write(&vec![0_u8; BIN_COUNT * 4]),
            },
        ],
        workgroups(radiance.len() / 3),
    )?;
    let counts: Vec<u32> = out[1]
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    let (log_min, log_max) = (-16.0_f32, 4.0_f32);
    let total: u64 = counts.iter().map(|c| *c as u64).sum();
    if total == 0 {
        return Err("直方图是空的（辐射全零？）".to_string());
    }
    let targets = [0.10_f64, 0.50, 0.85, 0.99];
    let mut anchors = [0.0_f32; 4];
    let mut cumulative = 0_u64;
    let mut next = 0;
    for (bin, count) in counts.iter().enumerate() {
        cumulative += *count as u64;
        while next < 4 && cumulative as f64 >= targets[next] * total as f64 {
            let position = (bin as f32 + 0.5) / BIN_COUNT as f32;
            anchors[next] = 2.0_f32.powf(log_min + position * (log_max - log_min));
            next += 1;
        }
    }
    for value in anchors.iter_mut() {
        if *value <= 0.0 {
            *value = 1e-4;
        }
    }
    for index in 1..4 {
        let floor = anchors[index - 1] * 1.06;
        if anchors[index] < floor {
            anchors[index] = floor;
        }
    }
    Ok(anchors)
}

#[cfg(test)]
mod sky_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    fn f32_from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x3ff) as f32;
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            31 => {
                if mantissa == 0.0 {
                    sign * f32::INFINITY
                } else {
                    f32::NAN
                }
            }
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    fn fixture(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for face in 0..6_u32 {
            for layer in 0..layers {
                let altitude = layer as f32 / (layers - 1).max(1) as f32;
                let _radius = inner + (outer - inner) * altitude;
                for t in 0..res {
                    for s in 0..res {
                        let d = px_volume_schema::direction_of(
                            face,
                            (s as f32 + 0.5) / res as f32,
                            (t as f32 + 0.5) / res as f32,
                        );
                        let wave = (3.0 * d[0]).sin() * (4.0 * d[1]).cos() * (5.0 * d[2]).sin();
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = match lane {
                                0..=2 => (0.45 + 0.3 * wave).max(0.0) * (1.0 + 0.12 * lane as f32),
                                _ => (0.25 + 0.5 * (wave * 0.5 + 0.5)).max(0.0),
                            };
                        }
                    }
                }
            }
        }
        VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        }
    }

    #[test]
    fn the_gpu_sky_matches_the_cpu_end_to_end() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let volume = fixture(res, layers, inner, outer);
        let stars = fixture_star_field(2000, 0.2, inner, outer, 8.0);
        let params = px_volume_schema::params::sky::SkyParams {
            face: 8,
            steps: 32,
            jitter: 0.0,
            star_gain: 0.05,
            background: [0.0011, 0.0007, 0.0009],
            ..Default::default()
        };
        let reference = px_volume_alg::raymarch_sky(&volume, &stars, &params).expect("CPU 出图");
        let table = px_volume_alg::raymarch::RAMP_HUE;
        let ramp_hue_table = [
            [table[0][0], table[0][1], table[0][2], 0.0],
            [table[1][0], table[1][1], table[1][2], 0.0],
            [table[2][0], table[2][1], table[2][2], 0.0],
            [table[3][0], table[3][1], table[3][2], 0.0],
        ];
        let grid = StarGrid::of(&stars);
        let extras = MarchExtras {
            star_grid: Some(&grid),
            star_table: &stars.stars,
            star_meta: StarMeta::sky(&grid, &params),
            background: params.background,
        };
        let gpu_side = px_gpu::require_gpu(sky(
            params.face,
            params.steps,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &extras,
            (
                px_volume_alg::raymarch::TONE_IN,
                px_volume_alg::raymarch::TONE_OUT,
                px_volume_alg::TONE_LIMITS,
            ),
            (px_volume_alg::raymarch::RAMP_LUMA, ramp_hue_table),
            px_volume_alg::GRADE_STRENGTH,
            None,
        ));
        let texels = reference.bytes.len() / 8;
        assert_eq!(gpu_side.len(), texels * 3, "texel 数");
        let mut worst = 0.0_f32;
        let mut worst_at = 0usize;
        for index in 0..texels {
            for channel in 0..3 {
                let at = index * 8 + channel * 2;
                let want = f32_from_half(u16::from_le_bytes([
                    reference.bytes[at],
                    reference.bytes[at + 1],
                ]));
                let got = gpu_side[index * 3 + channel];
                let diff = (got - want).abs() / want.abs().max(1e-2);
                if diff > worst {
                    worst = diff;
                    worst_at = index * 3 + channel;
                }
            }
        }
        assert!(
            worst < 5e-3,
            "整链最大相对偏差 {worst:.6} 在第 {worst_at} 个分量（GPU {} 对 CPU {})",
            gpu_side[worst_at],
            f32_from_half(u16::from_le_bytes([
                reference.bytes[(worst_at / 3) * 8 + (worst_at % 3) * 2],
                reference.bytes[(worst_at / 3) * 8 + (worst_at % 3) * 2 + 1],
            ]))
        );
        println!(
            "px_volume_gpu_op：整链 {} texel 最大相对偏差 {worst:.6}",
            texels
        );
    }

    /// 判据是**参数真的换了采样点**：夹具逐层在两档之间取三角波，格内挪动采样点就会换出不同的积分。
    #[test]
    fn sky_jitter_moves_the_gpu_samples() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for layer in 0..layers {
            let low = layer % 2 == 0;
            for t in 0..res {
                for s in 0..res {
                    for face in 0..6_u32 {
                        let at = flat_index(res, layers, face, layer, t, s, 0);
                        for lane in 0..LANES {
                            data[at + lane] = match lane {
                                0..=2 => {
                                    if low {
                                        0.2 * (s as f32 + 1.0) / res as f32
                                    } else {
                                        1.0 - 0.8 * (s as f32 + 1.0) / res as f32
                                    }
                                }
                                _ => 0.0,
                            };
                        }
                    }
                }
            }
        }
        let volume = VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        };
        let stars = empty_star_field();
        let params = |jitter: f32| px_volume_schema::params::sky::SkyParams {
            face: 8,
            steps: 32,
            jitter,
            star_gain: 0.0,
            ..Default::default()
        };
        let straight = match raymarch_sky(&volume, &stars, &params(0.0)) {
            Ok(texture) => texture,
            Err(message) => panic!("烘焙失败：{message}"),
        };
        let dithered = match raymarch_sky(&volume, &stars, &params(1.0)) {
            Ok(texture) => texture,
            Err(message) => panic!("烘焙失败：{message}"),
        };
        let moved = straight
            .bytes
            .iter()
            .zip(dithered.bytes.iter())
            .filter(|(one, two)| one != two)
            .count();
        assert!(
            moved > 0,
            "`jitter = 1.0` 与 `jitter = 0.0` 烘出了逐字节相同的天空（{} 字节）—— \
             抖动没有走到着色器",
            straight.bytes.len()
        );
        println!(
            "px_volume_gpu_op：抖动改变了 {moved} / {} 个字节",
            straight.bytes.len()
        );
    }
}

pub fn raymarch_sky(
    emission: &px_volume_schema::VolumeData,
    stars: &px_sparse::StarField,
    sky_params: &px_volume_schema::params::sky::SkyParams,
) -> Result<px_volume_schema::TextureData, String> {
    let face = sky_params.face.max(1);
    let steps = sky_params.steps.max(1);
    let occupancy = Occupancy::from_flat(
        emission.res,
        emission.layers,
        emission.lanes(),
        &emission.data,
    );
    if std::env::var("PX_SKIP_REPORT").is_ok() {
        let total = occupancy.sidecar().len();
        let live = occupancy
            .sidecar()
            .iter()
            .filter(|word| **word != 0)
            .count();
        eprintln!(
            "空跳掩码：res {} / layers {} / 每面块 {} / 总块 {total} / 活块 {live}（{:.1}%）\
             ⇒ 空块 {:.1}% 可整段跳过",
            emission.res,
            emission.layers,
            occupancy.blocks_per_face(),
            live as f64 * 100.0 / total as f64,
            (total - live) as f64 * 100.0 / total as f64
        );
    }
    let occupancy = if std::env::var("PX_SKIP_OFF").is_ok() {
        None
    } else {
        Some(occupancy)
    };
    let occupancy = occupancy.as_ref();
    let star_grid = StarGrid::of(stars);
    let extras = MarchExtras {
        star_grid: Some(&star_grid),
        star_table: &stars.stars,
        star_meta: StarMeta::sky(&star_grid, sky_params),
        background: sky_params.background,
    };
    let hue = px_volume_alg::raymarch::RAMP_HUE;
    let ramp_hue_table = [
        [hue[0][0], hue[0][1], hue[0][2], 0.0],
        [hue[1][0], hue[1][1], hue[1][2], 0.0],
        [hue[2][0], hue[2][1], hue[2][2], 0.0],
        [hue[3][0], hue[3][1], hue[3][2], 0.0],
    ];
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU：这一档现在完全跑在 GPU 上".to_string());
    };
    let volume_uniform = [
        emission.res.to_le_bytes(),
        emission.layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        0_u32.to_le_bytes(),
        emission.inner.to_le_bytes(),
        emission.outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let (occupancy_uniform, occupancy_words) = occupancy_bytes(
        occupancy,
        emission.res,
        emission.layers,
        emission.inner,
        emission.outer,
    );
    let mut uniform = SkyUniform {
        counts: [steps, face, 0, if occupancy.is_some() { 1 } else { 0 }],
        scalars: [sky_params.jitter.clamp(0.0, 1.0), 0.0, 0.0, emission.inner],
        background: [
            extras.background[0],
            extras.background[1],
            extras.background[2],
            0.0,
        ],
        tone_in: [0.0; 4],
        tone_out: px_volume_alg::raymarch::TONE_OUT,
        ramp_luma: px_volume_alg::raymarch::RAMP_LUMA,
        ramp_hue: ramp_hue_table,
        limits: [
            px_volume_alg::TONE_LIMITS[0],
            px_volume_alg::TONE_LIMITS[1],
            px_volume_alg::GRADE_STRENGTH,
            0.0,
        ],
    };
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let texels = (face * face * 6) as usize;
    let star_slots = StarSlots::of(&extras);

    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "sky_radiance",
        &[
            px_gpu::Slot {
                binding: 0,
                value: Binding::Uniform(&volume_uniform),
            },
            px_gpu::Slot {
                binding: 1,
                value: Binding::Storage(&bytes(&emission.data)),
            },
            px_gpu::Slot {
                binding: 4,
                value: Binding::Uniform(&uniform.to_bytes()),
            },
            px_gpu::Slot {
                binding: 5,
                value: Binding::Write(&vec![0_u8; texels * 12]),
            },
            px_gpu::Slot {
                binding: 6,
                value: Binding::Storage(&star_slots.table),
            },
            px_gpu::Slot {
                binding: 10,
                value: Binding::Storage(&star_slots.index),
            },
            px_gpu::Slot {
                binding: 11,
                value: Binding::Uniform(&star_slots.meta),
            },
            px_gpu::Slot {
                binding: 12,
                value: Binding::Write(&vec![0_u8; 4]),
            },
            px_gpu::Slot {
                binding: 13,
                value: Binding::Uniform(&occupancy_uniform.to_bytes()),
            },
            px_gpu::Slot {
                binding: 14,
                value: Binding::Storage(&occupancy_words),
            },
        ],
        workgroups(texels),
    )?;
    check_star_overflow(&out[1])?;
    let radiance: Vec<f32> = out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();

    uniform.tone_in = anchors_from_radiance(gpu, &radiance, &uniform)?;

    let graded = grade_pixels(gpu, &radiance, &uniform)?;
    if graded.len() != texels * 3 {
        return Err(format!(
            "GPU 出图长度不对：{}（应为 {}）",
            graded.len(),
            texels * 3
        ));
    }
    let mut packed = Vec::with_capacity(texels * 8);
    for index in 0..texels {
        for channel in 0..3 {
            packed.extend_from_slice(
                &px_volume_alg::half::half_from_f32(graded[index * 3 + channel]).to_le_bytes(),
            );
        }
        packed.extend_from_slice(&px_volume_alg::half::half_from_f32(1.0).to_le_bytes());
    }
    Ok(px_volume_schema::TextureData::new(
        face,
        face,
        6,
        1,
        px_volume_schema::TextureFormat::Rgba16Float,
        packed,
    ))
}

#[cfg(test)]
mod size_tests {
    use super::*;

    #[test]
    fn a_real_size_volume_goes_through() {
        let (res, layers, face) = (128_u32, 128_u32, 1024_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let data = vec![0.5_f32; (6 * layers * res * res * LANES as u32) as usize];
        let result = march(
            face,
            4,
            0,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &data,
            &MarchExtras::default(),
            None,
        );
        match result {
            Ok(values) => {
                assert_eq!(values.len(), (face * face * 6) as usize, "texel 数");
                println!("px_volume_gpu_op：真实尺寸（302 MB 体积 + face 1024）派发通过");
            }
            Err(message) => panic!("真实尺寸派发失败：{message}"),
        }
    }
}

#[cfg(test)]
mod seam_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    fn f32_from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x3ff) as f32;
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            31 => {
                if mantissa == 0.0 {
                    sign * f32::INFINITY
                } else {
                    f32::NAN
                }
            }
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    #[test]
    fn the_baked_sky_is_continuous_across_face_edges() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index.wrapping_mul(2654435761)) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        };
        let stars = empty_star_field();
        let params = px_volume_schema::params::sky::SkyParams {
            face: 32,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            ..Default::default()
        };
        let texture = match raymarch_sky(&volume, &stars, &params) {
            Ok(texture) => texture,
            Err(message) => panic!("烘焙失败：{message}"),
        };
        let face = params.face;
        let lum = |x: u32, y: u32| -> f32 {
            let at = ((y * face + x) * 8) as usize;
            f32_from_half(u16::from_le_bytes([
                texture.bytes[at],
                texture.bytes[at + 1],
            ]))
        };
        let direction = |face_index: u32, x: u32, y: u32| -> [f32; 3] {
            px_volume_schema::direction_of(
                face_index,
                (x as f32 + 0.5) / face as f32,
                (y as f32 + 0.5) / face as f32,
            )
        };
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for _face_index in 0..6_u32 {
            for y in 0..face {
                for x in 0..face - 1 {
                    interior += (lum(x + 1, y) - lum(x, y)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        let mut edge = 0.0_f32;
        let mut edge_count = 0.0_f32;
        let mut worst = 0.0_f32;
        for face_index in 0..6_u32 {
            for y in 0..face {
                let here = direction(face_index, 0, y);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            let angle = 1.0 - dot;
                            if angle < best {
                                best = angle;
                                best_value = lum(ox, oy);
                            }
                        }
                    }
                }
                let diff = (lum(0, y) - best_value).abs();
                edge += diff;
                edge_count += 1.0;
                worst = worst.max(diff);
            }
        }
        let edge = edge / edge_count;
        let ratio = edge / interior.max(1e-6);
        let mut per_face = [0.0_f32; 6];
        let mut per_band = [0.0_f32; 4];
        let mut per_band_count = [0.0_f32; 4];
        for face_index in 0..6_u32 {
            for y in 0..face {
                let here = direction(face_index, 0, y);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                                best_value = lum(ox, oy);
                            }
                        }
                    }
                }
                let diff = (lum(0, y) - best_value).abs();
                per_face[face_index as usize] += diff;
                let band = ((y * 4) / face).min(3) as usize;
                per_band[band] += diff;
                per_band_count[band] += 1.0;
            }
        }
        let mut edge_angle = 0.0_f32;
        let mut edge_angle_count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                            }
                        }
                    }
                }
                edge_angle += best;
                edge_angle_count += 1.0;
            }
        }
        let a0 = direction(0, 0, 5);
        let a1 = direction(0, 1, 5);
        let interior_angle = 1.0 - (a0[0] * a1[0] + a0[1] * a1[1] + a0[2] * a1[2]);
        println!(
            "角度距离：跨棱最近格 {:.6} / 面内相邻格 {:.6} = {:.2}x",
            edge_angle / edge_angle_count,
            interior_angle,
            (edge_angle / edge_angle_count) / interior_angle.max(1e-9)
        );
        println!(
            "逐面棱差：{:?}",
            per_face.map(|v| (v / face as f32 * 1000.0).round() / 1000.0)
        );
        println!(
            "棱上分段（0=一端 3=另一端）：{:?}",
            std::array::from_fn::<f32, 4, _>(|i| per_band[i] / per_band_count[i].max(1.0) * 1000.0)
                .map(|v| (v).round() / 1000.0)
        );
        println!(
            "面内基准 {interior:.5}（乘 1000 后 {:.3}）",
            interior * 1000.0
        );
        assert!(
            ratio < 2.0,
            "面棱上的差 {edge:.5} 是面内基准 {interior:.5} 的 {ratio:.2} 倍（最大 {worst:.5}）\
             —— 棱两侧对不上，缝在**烘焙侧**（采样/布局/面序）"
        );
        println!(
            "px_volume_gpu_op：面棱差 {edge:.5} / 面内 {interior:.5} = {ratio:.2}x（最大 {worst:.5}）"
        );
    }
}

#[cfg(test)]
mod sampler_seam_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    #[test]
    fn the_sampler_is_continuous_across_a_face_edge() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index * 2654435761usize) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        };
        let mut worst = (0.0_f32, 0.0_f32, 0.0_f32);
        for step_index in 0..16 {
            let radius = inner + (outer - inner) * (step_index as f32 + 0.5) / 16.0;
            let mut edge = 0.0_f32;
            let mut inside = 0.0_f32;
            let mut count = 0.0_f32;
            for face in 0..6_u32 {
                for t_index in 1..res - 1 {
                    let t = (t_index as f32 + 0.5) / res as f32;
                    let at = |s: f32| -> f32 {
                        let d = px_volume_schema::direction_of(face, s, t);
                        let point = [d[0] * radius, d[1] * radius, d[2] * radius];
                        px_volume_alg::sample_volume(&volume, point, 0)
                    };
                    let step = 0.5 / res as f32;
                    edge += (at(step) - at(-step)).abs();
                    inside += (at(1.0 + step) - at(step)).abs();
                    count += 1.0;
                }
            }
            let edge = edge / count;
            let inside = inside / count;
            let ratio = edge / inside.max(1e-6);
            println!(
                "  半径 {radius:.3}（高度 {:.3}）：跨棱 {edge:.5} / 面内 {inside:.5} = {ratio:.2}x",
                (radius - inner) / (outer - inner)
            );
            if ratio > worst.2 {
                worst = (radius, edge, ratio);
            }
        }
        println!("采样器：最差在半径 {:.3} —— {:.2}x", worst.0, worst.2);
        assert!(
            worst.2 < 2.0,
            "半径 {:.3} 上跨棱的采样差是面内的 {:.2} 倍 —— 采样语义在该处不连续",
            worst.0,
            worst.2
        );
    }
}

#[cfg(test)]
mod march_seam_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    #[test]
    fn the_march_is_continuous_across_a_face_edge() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index * 2654435761usize) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        };
        let face = 32_u32;
        let params = px_volume_schema::params::sky::SkyParams {
            face,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            ..Default::default()
        };
        let field = px_volume_alg::raymarch_channel(&volume, None, &params, 0);
        let lum = |x: u32, y: u32| -> f32 { field.at(x, y) };
        let direction = |face_index: u32, x: u32, y: u32| -> [f32; 3] {
            px_volume_schema::direction_of(
                face_index,
                (x as f32 + 0.5) / face as f32,
                (y as f32 + 0.5) / face as f32,
            )
        };
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for _face_index in 0..6_u32 {
            for row in 0..face {
                for x in 0..face - 1 {
                    interior += (lum(x + 1, row) - lum(x, row)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        let mut edge = 0.0_f32;
        let mut count = 0.0_f32;
        let mut worst = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                                best_value = lum(ox, other * face + oy);
                            }
                        }
                    }
                }
                let diff = (lum(0, face_index * face + row) - best_value).abs();
                edge += diff;
                count += 1.0;
                worst = worst.max(diff);
            }
        }
        let edge = edge / count;
        println!(
            "步进：棱上差 {edge:.5} / 面内 {interior:.5} = {:.2}x（最大 {worst:.5}）",
            edge / interior.max(1e-6)
        );
        assert!(
            edge / interior.max(1e-6) < 2.0,
            "步进侧在棱上不连续：{:.2} 倍",
            edge / interior.max(1e-6)
        );
    }
}

#[cfg(test)]
mod identity_grade_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    fn f32_from_half(bits: u16) -> f32 {
        let sign = if bits & 0x8000 != 0 { -1.0_f32 } else { 1.0 };
        let exponent = ((bits >> 10) & 0x1f) as i32;
        let mantissa = (bits & 0x3ff) as f32;
        match exponent {
            0 => sign * mantissa * 2.0_f32.powi(-24),
            31 => {
                if mantissa == 0.0 {
                    sign * f32::INFINITY
                } else {
                    f32::NAN
                }
            }
            _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
        }
    }

    #[test]
    fn the_radiance_is_continuous_under_an_identity_grade() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index * 2654435761usize) % 1000) as f32 / 1000.0;
        }
        let volume = VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        };
        let face = 32_u32;
        let params = px_volume_schema::params::sky::SkyParams {
            face,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            ..Default::default()
        };
        let identity = [0.01_f32, 0.1, 1.0, 10.0];
        let texture = sky(
            face,
            params.steps,
            0.0,
            inner,
            res,
            layers,
            inner,
            outer,
            &volume.data,
            &MarchExtras {
                star_grid: None,
                star_table: &[],
                star_meta: StarMeta::default(),
                background: [0.0; 3],
            },
            (identity, identity, [1.0e9, 1.0e9]),
            (
                px_volume_alg::raymarch::RAMP_LUMA,
                [[1.0, 1.0, 1.0, 0.0]; 4],
            ),
            0.0,
            None,
        )
        .expect("恒等分级下出图");
        let lum = |x: u32, y: u32| -> f32 { texture[((y * face + x) * 3) as usize] };
        let direction = |face_index: u32, x: u32, y: u32| -> [f32; 3] {
            px_volume_schema::direction_of(
                face_index,
                (x as f32 + 0.5) / face as f32,
                (y as f32 + 0.5) / face as f32,
            )
        };
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for _face_index in 0..6_u32 {
            for row in 0..face {
                for x in 0..face - 1 {
                    interior += (lum(x + 1, row) - lum(x, row)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        let mut edge = 0.0_f32;
        let mut count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..face {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..face {
                        for ox in 0..face {
                            let there = direction(other, ox, oy);
                            let dot = here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                                best_value = lum(ox, other * face + oy);
                            }
                        }
                    }
                }
                edge += (lum(0, face_index * face + row) - best_value).abs();
                count += 1.0;
            }
        }
        let edge = edge / count;
        println!(
            "恒等分级：棱上差 {edge:.5} / 面内 {interior:.5} = {:.2}x",
            edge / interior.max(1e-6)
        );
    }
}

#[cfg(test)]
mod chain_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    const RES: u32 = 8;
    const LAYERS: u32 = 4;
    const INNER: f32 = 1.0;
    const OUTER: f32 = 2.0;
    const FACE: u32 = 32;

    fn fixture() -> VolumeData {
        let mut data = vec![0.0_f32; (6 * LAYERS * RES * RES * LANES as u32) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = 0.2 + 0.6 * ((index % 97) as f32 / 97.0);
        }
        VolumeData {
            lanes: 1,
            res: RES,
            layers: LAYERS,
            inner: INNER,
            outer: OUTER,
            data,
        }
    }

    fn direction(face_index: u32, x: u32, y: u32) -> [f32; 3] {
        px_volume_schema::direction_of(
            face_index,
            (x as f32 + 0.5) / FACE as f32,
            (y as f32 + 0.5) / FACE as f32,
        )
    }

    fn ratio_of(sample: &dyn Fn(u32, u32, u32) -> f32) -> f32 {
        let mut interior = 0.0_f32;
        let mut interior_count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..FACE {
                for x in 0..FACE - 1 {
                    interior += (sample(face_index, x + 1, row) - sample(face_index, x, row)).abs();
                    interior_count += 1.0;
                }
            }
        }
        let interior = interior / interior_count;
        let mut edge = 0.0_f32;
        let mut count = 0.0_f32;
        for face_index in 0..6_u32 {
            for row in 0..FACE {
                let here = direction(face_index, 0, row);
                let mut best = f32::MAX;
                let mut best_value = 0.0_f32;
                for other in 0..6_u32 {
                    if other == face_index {
                        continue;
                    }
                    for oy in 0..FACE {
                        for ox in 0..FACE {
                            let there = direction(other, ox, oy);
                            let dot = here[0] * there[0] + here[1] * there[1] + here[2] * there[2];
                            if 1.0 - dot < best {
                                best = 1.0 - dot;
                                best_value = sample(other, ox, oy);
                            }
                        }
                    }
                }
                edge += (sample(face_index, 0, row) - best_value).abs();
                count += 1.0;
            }
        }
        (edge / count) / interior.max(1e-6)
    }

    #[test]
    fn the_seam_appears_at_one_specific_stage() {
        let volume = fixture();
        let params = px_volume_schema::params::sky::SkyParams {
            face: FACE,
            steps: 16,
            jitter: 0.0,
            star_gain: 0.0,
            ..Default::default()
        };
        let radius = (INNER * OUTER).sqrt();
        let sampler = ratio_of(&|face_index, x, y| {
            let d = direction(face_index, x, y);
            px_volume_alg::sample_volume(&volume, [d[0] * radius, d[1] * radius, d[2] * radius], 0)
        });
        let field = px_volume_alg::raymarch_channel(&volume, None, &params, 0);
        let march = ratio_of(&|face_index, x, y| field.at(x, face_index * FACE + y));
        let identity = [0.01_f32, 0.1, 1.0, 10.0];
        let radiance = sky(
            FACE,
            params.steps,
            0.0,
            INNER,
            RES,
            LAYERS,
            INNER,
            OUTER,
            &volume.data,
            &MarchExtras {
                star_grid: None,
                star_table: &[],
                star_meta: StarMeta::default(),
                background: [0.0; 3],
            },
            (identity, identity, [1.0e9, 1.0e9]),
            (
                px_volume_alg::raymarch::RAMP_LUMA,
                [[1.0, 1.0, 1.0, 0.0]; 4],
            ),
            0.0,
            None,
        )
        .expect("GPU 辐射");
        let gpu_radiance = ratio_of(&|face_index, x, y| {
            radiance[((face_index * FACE + y) * FACE + x) as usize * 3]
        });
        let stars = empty_star_field();
        let texture = raymarch_sky(&volume, &stars, &params).expect("整链");
        let full = ratio_of(&|face_index, x, y| {
            let at = (((face_index * FACE + y) * FACE + x) * 8) as usize;
            let sign = if texture.bytes[at + 1] & 0x80 != 0 {
                -1.0_f32
            } else {
                1.0
            };
            let bits = u16::from_le_bytes([texture.bytes[at], texture.bytes[at + 1]]);
            let exponent = ((bits >> 10) & 0x1f) as i32;
            let mantissa = (bits & 0x3ff) as f32;
            match exponent {
                0 => sign * mantissa * 2.0_f32.powi(-24),
                _ => sign * (1.0 + mantissa / 1024.0) * 2.0_f32.powi(exponent - 15),
            }
        });
        println!("同一夹具上的二分链（跨棱/面内）：");
        println!("  1 采样器        {sampler:.2}x");
        println!("  2 步进 CPU      {march:.2}x");
        println!("  3 GPU 辐射      {gpu_radiance:.2}x");
        println!("  4 整链（分级）  {full:.2}x");
        assert!(sampler < 2.0, "采样器这一步就 {sampler:.2}x");
    }
}

#[allow(clippy::too_many_arguments)]
pub fn bake_emission(
    density: &px_volume_schema::VolumeData,
    stars: &px_sparse::StarField,
    params: &px_volume_schema::params::emission::EmissionParams,
) -> Result<px_volume_schema::VolumeData, String> {
    if params.starlight_gain > 0.0 && params.starlight_max > STAR_KEEP_MAX {
        return Err(format!(
            "`starlight_max` 是 {}，超过 GPU 这一侧的定长表上限 {STAR_KEEP_MAX}",
            params.starlight_max
        ));
    }
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let (res, layers) = (density.res, density.layers);
    let samples = (6 * layers * res * res) as usize;
    let wanted = if density.data.len() == samples {
        let mut widened = vec![0.0_f32; samples * LANES];
        for (index, value) in density.data.iter().enumerate() {
            widened[index * LANES] = *value;
        }
        widened
    } else {
        density.data.clone()
    };
    let volume_uniform = [
        res.to_le_bytes(),
        layers.to_le_bytes(),
        (LANES as u32).to_le_bytes(),
        0_u32.to_le_bytes(),
        density.inner.to_le_bytes(),
        density.outer.to_le_bytes(),
        0.0_f32.to_le_bytes(),
        0.0_f32.to_le_bytes(),
    ]
    .concat();
    let uniform = [
        params.light_radius,
        params.shadow_gain,
        params.emission_power,
        params.emission_gain,
        params.light[0],
        params.light[1],
        params.light[2],
        0.0,
        params.glow_gain,
        params.glow_power,
        params.glow_threshold,
        0.0,
        params.glow_tint[0],
        params.glow_tint[1],
        params.glow_tint[2],
        0.0,
        params.scatter_tint[0],
        params.scatter_tint[1],
        params.scatter_tint[2],
        0.0,
        params.extinction[0],
        params.extinction[1],
        params.extinction[2],
        params.extinction_power,
        params.dust_bias,
        params.dust_threshold,
        0.0,
        0.0,
    ]
    .iter()
    .flat_map(|value| value.to_le_bytes())
    .chain(
        [res, layers, params.shadow_steps, 0]
            .iter()
            .flat_map(|value| value.to_le_bytes()),
    )
    .collect::<Vec<u8>>();
    let voxels = (6 * layers * res * res) as usize;
    let bytes =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
    let star_grid = StarGrid::of(stars);
    let star_meta = StarMeta::emission(&star_grid, params);
    let star_slots = StarSlots::of(&MarchExtras {
        star_grid: Some(&star_grid),
        star_table: &stars.stars,
        star_meta,
        background: [0.0; 3],
    });
    let out = px_gpu::dispatch_slots(
        gpu,
        SAMPLER_WGSL,
        "bake_emission",
        &[
            px_gpu::Slot {
                binding: 0,
                value: Binding::Uniform(&volume_uniform),
            },
            px_gpu::Slot {
                binding: 1,
                value: Binding::Storage(&bytes(&wanted)),
            },
            px_gpu::Slot {
                binding: 6,
                value: Binding::Storage(&star_slots.table),
            },
            px_gpu::Slot {
                binding: 8,
                value: Binding::Uniform(&uniform),
            },
            px_gpu::Slot {
                binding: 9,
                value: Binding::Write(&vec![0_u8; voxels * 6 * 4]),
            },
            px_gpu::Slot {
                binding: 10,
                value: Binding::Storage(&star_slots.index),
            },
            px_gpu::Slot {
                binding: 11,
                value: Binding::Uniform(&star_slots.meta),
            },
        ],
        workgroups(voxels),
    )?;
    let data: Vec<f32> = out[0]
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect();
    Ok(px_volume_schema::VolumeData {
        lanes: 6,
        res,
        layers,
        inner: density.inner,
        outer: density.outer,
        data,
    })
}

#[cfg(test)]
mod emission_tests {
    use super::*;
    use px_volume_schema::VolumeData;

    #[test]
    fn the_gpu_emission_matches_the_cpu() {
        let (res, layers) = (8_u32, 4_u32);
        let (inner, outer) = (1.0_f32, 2.0_f32);
        let mut data = vec![0.0_f32; (6 * layers * res * res) as usize];
        for (index, value) in data.iter_mut().enumerate() {
            *value = ((index.wrapping_mul(2654435761)) % 1000) as f32 / 1000.0;
        }
        let density = VolumeData {
            lanes: 1,
            res,
            layers,
            inner,
            outer,
            data,
        };
        let stars = fixture_star_field(20_000, 0.1, inner, outer, 2.0);
        for keep in [32_u32, 2, 0] {
            let params = px_volume_schema::params::emission::EmissionParams {
                light: [0.4, 0.7, -0.3],
                light_radius: 0.2,
                shadow_steps: 8,
                shadow_gain: 1.6,
                starlight_gain: 1.4,
                starlight_soft: 0.05,
                starlight_radius: 0.2,
                starlight_steps: 4,
                starlight_max: keep,
                glow_tint: [0.85, 0.30, 0.55],
                scatter_tint: [1.0, 0.6, 0.25],
                ..Default::default()
            };
            let reference = px_volume_alg::bake_emission(&density, &stars, &params);
            let gpu_side = match bake_emission(&density, &stars, &params) {
                Ok(volume) => volume,
                Err(message) => panic!("GPU 发射烘焙失败：{message}"),
            };
            assert_eq!(gpu_side.data.len(), reference.data.len(), "体素数");
            let mut worst = 0.0_f32;
            let mut worst_at = 0usize;
            for (index, value) in gpu_side.data.iter().enumerate() {
                let diff =
                    (value - reference.data[index]).abs() / reference.data[index].abs().max(1e-2);
                if diff > worst {
                    worst = diff;
                    worst_at = index;
                }
            }
            assert!(
                worst < 5e-4,
                "发射烘焙（keep {keep}）最大相对偏差 {worst:.6} 在第 {worst_at} 个分量（GPU {} 对 CPU {}）",
                gpu_side.data[worst_at],
                reference.data[worst_at]
            );
            println!(
                "px_volume_gpu_op：发射烘焙（keep {keep}）最大相对偏差 {worst:.6}（{} 个体素 x 6）",
                gpu_side.data.len() / 6
            );
        }
    }
}
