//! See docs/nurbs.md

use px_gpu::{Binding, connect, dispatch};
use px_nurbs_schema::curve::Curve;
use px_nurbs_schema::mesh::{grid_mesh, polyline};
use px_nurbs_schema::surface::Surface;
use px_nurbs_schema::{MeshData, PolylineData};

pub const SURFACE_WGSL: &str = include_str!("surface.wgsl");

pub const CURVE_WGSL: &str = include_str!("curve.wgsl");

pub const MAX_DEGREE: usize = 8;

pub const MAX_POINTS: u64 = 4_000_000;

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    if values.is_empty() {
        return vec![0_u8; 4];
    }
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub fn workgroups(threads: usize) -> (u32, u32, u32) {
    const WG_X: usize = 65535;
    let blocks = threads.div_ceil(64).max(1);
    let x = blocks.min(WG_X);
    let y = blocks.div_ceil(WG_X);
    (x as u32, y as u32, 1)
}

fn decode_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn homogeneous_surface(surface: &Surface) -> Vec<f32> {
    let count = surface.nu * surface.nv;
    let mut out = Vec::with_capacity(count * 4);
    for index in 0..count {
        let weight = if surface.weights.is_empty() {
            1.0
        } else {
            surface.weights[index]
        };
        for lane in 0..3 {
            out.push((surface.control[index * 3 + lane] * weight) as f32);
        }
        out.push(weight as f32);
    }
    out
}

fn homogeneous_curve(curve: &Curve) -> Vec<f32> {
    let count = curve.count();
    let mut out = Vec::with_capacity(count * 4);
    for index in 0..count {
        let weight = if curve.weights.is_empty() {
            1.0
        } else {
            curve.weights[index]
        };
        for lane in 0..3 {
            out.push((curve.control[index * 3 + lane] * weight) as f32);
        }
        out.push(weight as f32);
    }
    out
}

fn check_degree(degree: usize) -> Result<(), String> {
    if degree > MAX_DEGREE {
        return Err(format!(
            "次数 {degree} 超过 GPU 那一侧的上限 {MAX_DEGREE}（WGSL 的基函数表是定长的）"
        ));
    }
    Ok(())
}

fn surface_shape(surface: &Surface, out_u: u32, out_v: u32, offset: f32) -> Vec<u8> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    let along = |count: u32| (count.max(2) - 1) as f32;
    [
        (surface.nu as u32).to_le_bytes(),
        (surface.nv as u32).to_le_bytes(),
        (surface.degree.0 as u32).to_le_bytes(),
        (surface.degree.1 as u32).to_le_bytes(),
        out_u.to_le_bytes(),
        out_v.to_le_bytes(),
        (u0 as f32).to_le_bytes(),
        (v0 as f32).to_le_bytes(),
        ((u1 - u0) as f32 / along(out_u)).to_le_bytes(),
        ((v1 - v0) as f32 / along(out_v)).to_le_bytes(),
        offset.to_le_bytes(),
        offset.to_le_bytes(),
    ]
    .concat()
}

pub fn surface_grid(
    surface: &Surface,
    out_u: u32,
    out_v: u32,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    surface_grid_at(surface, out_u, out_v, 0.0)
}

pub fn surface_grid_at(
    surface: &Surface,
    out_u: u32,
    out_v: u32,
    offset: f32,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    check_degree(surface.degree.0)?;
    check_degree(surface.degree.1)?;
    if out_u < 2 || out_v < 2 {
        return Err(format!("格点至少 2×2，给的是 {out_u}×{out_v}"));
    }
    let points = u64::from(out_u) * u64::from(out_v);
    if points > MAX_POINTS {
        return Err(format!(
            "一次要 {points} 个格点（上限 {MAX_POINTS}）：把容差放宽或者把深度上限降下来"
        ));
    }
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let shape = surface_shape(surface, out_u, out_v, offset);
    let hom = f32_bytes(&homogeneous_surface(surface));
    let knots_u = f32_bytes(
        &surface
            .knots_u
            .iter()
            .map(|value| *value as f32)
            .collect::<Vec<_>>(),
    );
    let knots_v = f32_bytes(
        &surface
            .knots_v
            .iter()
            .map(|value| *value as f32)
            .collect::<Vec<_>>(),
    );
    let bytes = points as usize * 3 * 4;
    let out = dispatch(
        gpu,
        SURFACE_WGSL,
        "surface_grid",
        &[
            Binding::Uniform(&shape),
            Binding::Storage(&hom),
            Binding::Storage(&knots_u),
            Binding::Storage(&knots_v),
            Binding::Write(&vec![0_u8; bytes]),
            Binding::Write(&vec![0_u8; bytes]),
        ],
        workgroups(points as usize),
    )?;
    Ok((decode_f32(&out[0]), decode_f32(&out[1])))
}

pub fn curve_points(curve: &Curve, out_n: u32) -> Result<Vec<f32>, String> {
    curve_points_at(curve, out_n, 0.0)
}

pub fn curve_points_at(curve: &Curve, out_n: u32, offset: f32) -> Result<Vec<f32>, String> {
    check_degree(curve.degree)?;
    if out_n < 2 {
        return Err(format!("格点至少 2 个，给的是 {out_n}"));
    }
    if u64::from(out_n) > MAX_POINTS {
        return Err(format!("一次要 {out_n} 个点（上限 {MAX_POINTS}）"));
    }
    let Some(gpu) = connect() else {
        return Err("没有可用 GPU".to_string());
    };
    let (low, high) = curve.domain();
    let shape = [
        (curve.count() as u32).to_le_bytes(),
        (curve.degree as u32).to_le_bytes(),
        out_n.to_le_bytes(),
        (low as f32).to_le_bytes(),
        ((high - low) as f32 / (out_n.max(2) - 1) as f32).to_le_bytes(),
        offset.to_le_bytes(),
    ]
    .concat();
    let hom = f32_bytes(&homogeneous_curve(curve));
    let knots = f32_bytes(
        &curve
            .knots
            .iter()
            .map(|value| *value as f32)
            .collect::<Vec<_>>(),
    );
    let out = dispatch(
        gpu,
        CURVE_WGSL,
        "curve_points",
        &[
            Binding::Uniform(&shape),
            Binding::Storage(&hom),
            Binding::Storage(&knots),
            Binding::Write(&vec![0_u8; out_n as usize * 3 * 4]),
        ],
        workgroups(out_n as usize),
    )?;
    Ok(decode_f32(&out[0]))
}

pub fn tessellate_surface(
    surface: &Surface,
    tolerance: f64,
    depth: u32,
    segments: u32,
) -> Result<MeshData, String> {
    let ((u0, u1), (v0, v1)) = surface.domain();
    if !(u1 > u0) || !(v1 > v0) {
        return Err("细分一份参数域为空的曲面".to_string());
    }
    let steps = u64::from(segments.clamp(1, 64));
    let depth = depth.min(16);
    let mut level = 0_u32;
    loop {
        let unit = steps << level;
        if (unit + 1) * (unit + 1) > MAX_POINTS {
            return Err(format!(
                "细分到第 {level} 级要 {} 个格点（上限 {MAX_POINTS}）：把容差放宽一点",
                (unit + 1) * (unit + 1)
            ));
        }
        let along = (unit + 1) as u32;
        let (corners, normals) = surface_grid(surface, along, along)?;
        let (centers, _) = surface_grid_at(surface, unit as u32, unit as u32, 0.5)?;
        let worst = worst_error(&corners, &centers, along as usize, unit as usize);
        if worst <= tolerance || level >= depth {
            let mut uvs = Vec::with_capacity(corners.len() / 3 * 2);
            for i in 0..along {
                for j in 0..along {
                    let u = u0 + (u1 - u0) * i as f64 / unit as f64;
                    let v = v0 + (v1 - v0) * j as f64 / unit as f64;
                    uvs.push(u as f32);
                    uvs.push(v as f32);
                }
            }
            return Ok(grid_mesh(
                &corners,
                &normals,
                &uvs,
                along as usize,
                along as usize,
            ));
        }
        level += 1;
    }
}

fn worst_error(corners: &[f32], centers: &[f32], along: usize, unit: usize) -> f64 {
    let at = |values: &[f32], i: usize, j: usize, stride: usize| -> [f64; 3] {
        let index = (i * stride + j) * 3;
        [
            f64::from(values[index]),
            f64::from(values[index + 1]),
            f64::from(values[index + 2]),
        ]
    };
    let mut worst = 0.0_f64;
    for i in 0..unit {
        for j in 0..unit {
            let four = [
                at(corners, i, j, along),
                at(corners, i + 1, j, along),
                at(corners, i, j + 1, along),
                at(corners, i + 1, j + 1, along),
            ];
            let average = [
                (four[0][0] + four[1][0] + four[2][0] + four[3][0]) * 0.25,
                (four[0][1] + four[1][1] + four[2][1] + four[3][1]) * 0.25,
                (four[0][2] + four[1][2] + four[2][2] + four[3][2]) * 0.25,
            ];
            let middle = at(centers, i, j, unit);
            let delta = [
                middle[0] - average[0],
                middle[1] - average[1],
                middle[2] - average[2],
            ];
            let distance = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
            if distance > worst {
                worst = distance;
            }
        }
    }
    worst
}

pub fn tessellate_curve(
    curve: &Curve,
    tolerance: f64,
    depth: u32,
    segments: u32,
) -> Result<PolylineData, String> {
    let (low, high) = curve.domain();
    let steps = segments.clamp(1, 64);
    let depth = depth.min(16);
    let mut count = steps + 1;
    let mut level = 0_u32;
    loop {
        let dense = curve_points(curve, 2 * count - 1)?;
        let worst = worst_midpoint(&dense);
        if worst <= tolerance || level >= depth {
            let points: Vec<[f64; 3]> = (0..count as usize)
                .map(|index| {
                    let at = index * 2 * 3;
                    [
                        f64::from(dense[at]),
                        f64::from(dense[at + 1]),
                        f64::from(dense[at + 2]),
                    ]
                })
                .collect();
            let closed = distance(points[0], points[points.len() - 1]) <= 1e-12;
            return Ok(polyline(&points, closed));
        }
        let _ = (low, high);
        count = 2 * count - 1;
        level += 1;
    }
}

fn worst_midpoint(dense: &[f32]) -> f64 {
    let count = dense.len() / 3;
    let mut worst = 0.0_f64;
    for index in (1..count - 1).step_by(2) {
        let at = |i: usize| -> [f64; 3] {
            [
                f64::from(dense[i * 3]),
                f64::from(dense[i * 3 + 1]),
                f64::from(dense[i * 3 + 2]),
            ]
        };
        let middle = at(index);
        let before = at(index - 1);
        let after = at(index + 1);
        let delta = [
            middle[0] - (before[0] + after[0]) * 0.5,
            middle[1] - (before[1] + after[1]) * 0.5,
            middle[2] - (before[2] + after[2]) * 0.5,
        ];
        let distance = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if distance > worst {
            worst = distance;
        }
    }
    worst
}

fn distance(one: [f64; 3], two: [f64; 3]) -> f64 {
    let delta = [one[0] - two[0], one[1] - two[1], one[2] - two[2]];
    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt()
}

pub mod ops;

px_graph_schema::px_impl_lib!();

#[cfg(test)]
mod tests;
