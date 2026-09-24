//! **NURBS 求值的 GPU 那一侧**：曲面格点（位置 + 法线）与曲线格点（位置）。
//!
//! 形状与体积那一档（`px_volume_gpu_op`）**同一个模子**：
//!
//! 1. **热点在 WGSL 里**（逐格求值：基函数 + 齐次混合 + 商法则），
//!    **编排在 Rust 里**（细分到哪一级、参数怎么分、装配成网格）；
//! 2. **WGSL 源随 crate 一起装运**（`include_str!`），并且带一份
//!    **移植规格**（`surface.wgsl` / `curve.wgsl` 的头注释：CPU 那一侧是真源）；
//! 3. **判据是 GPU ↔ CPU 对账**（同一套参数、同一条曲线/曲面，逐点比），
//!    外加解析靶子（球面半径、圆上折线）与"同一次派发跑两遍逐位相同"；
//! 4. **没有可用设备时判据跳过**（不是失败）：判据测的是"两侧算的是不是同一个东西"，
//!    不是"这台机器必须有卡"。生产那一侧是**硬失败**（`Err`），不回退 CPU ——
//!    与用户定的口径一致：选了 GPU 这一路就等于声明有卡。
//!
//! ⚠⚠ **缓存键不受这里的影响**（GPU 的浮点不确定性不破坏缓存语义）：节点键 = 算子身份
//! （含实现库源码指纹）+ 参数 + 画布 + **输入键**，**不看产物内容**；产物字节只在落 CAS
//! 时算 blake3 当文件名 ⇒ 同机同输入命中不重算，换机器/驱动时内容不同而**键相同**，
//! 各自在本地 CAS 里重算自己那一份，不会混。唯一失去的是"跨机器 `.pxart` 逐字节相同"。
//! （这一条与 `px_gpu` 的文件头同一口径。）

use px_gpu::{Binding, connect, dispatch};
use px_nurbs_schema::curve::Curve;
use px_nurbs_schema::mesh::{grid_mesh, polyline};
use px_nurbs_schema::surface::Surface;
// ⚠ 两个产物类型从**声明那一层**出去（`px_nurbs_schema` re-export 了它们）：
//   少一条直接依赖，也就少一处"哪天线格式换了这里忘了跟"。
use px_nurbs_schema::{MeshData, PolylineData};

/// 曲面求值的 WGSL（位置 + 法线）。
pub const SURFACE_WGSL: &str = include_str!("surface.wgsl");

/// 曲线求值的 WGSL（位置）。
pub const CURVE_WGSL: &str = include_str!("curve.wgsl");

/// WGSL 里那张基函数表的宽度（`MAXD + 2`）—— 与 `surface.wgsl` 的 `MAXD` 一起改。
pub const MAX_DEGREE: usize = 8;

/// 一次派发最多求值多少个格点（`(unit+1)²` 的上界）。
///
/// ⚠ 没有它的话"容差极小 + 深度上限很大"会去申请一张几十 GB 的缓冲区，
///   而报错会落在驱动里、指不到这里。
pub const MAX_POINTS: u64 = 4_000_000;

/// `f32` 一串 → 小端字节（空的给 4 个字节：storage buffer 的绑定不能是 0 字节）。
fn f32_bytes(values: &[f32]) -> Vec<u8> {
    if values.is_empty() {
        return vec![0_u8; 4];
    }
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// 64 个线程一组；`x` 上限 65535（超过就换成 `y` 那一维）。
pub fn workgroups(threads: usize) -> (u32, u32, u32) {
    const WG_X: usize = 65535;
    let blocks = threads.div_ceil(64).max(1);
    let x = blocks.min(WG_X);
    let y = blocks.div_ceil(WG_X);
    (x as u32, y as u32, 1)
}

/// 解析回读的 `f32` 数组。
fn decode_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

/// 齐次控制点（`(w·x, w·y, w·z, w)` 四格一个，行主序）。
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

/// 次数必须在 WGSL 那张表的宽度之内 —— 超了当场拒（不是静默算错）。
fn check_degree(degree: usize) -> Result<(), String> {
    if degree > MAX_DEGREE {
        return Err(format!(
            "次数 {degree} 超过 GPU 那一侧的上限 {MAX_DEGREE}（WGSL 的基函数表是定长的）"
        ));
    }
    Ok(())
}

/// 参数网格：`out_u × out_v` 个点，`u = u0 + u_step·(i + offset)`。
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

/// **曲面格点求值**：返回 `(位置, 法线)`，各 `out_u · out_v` 个（行主序 `i·out_v + j`）。
///
/// `offset = 0` 是格点本身、`0.5` 是每格中心（细分那一侧用它估弦误差）。
pub fn surface_grid(
    surface: &Surface,
    out_u: u32,
    out_v: u32,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    surface_grid_at(surface, out_u, out_v, 0.0)
}

/// [`surface_grid`] 的带偏移版本（`offset` 见上）。
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

/// **曲线格点求值**：返回 `out_n` 个 `[x, y, z]`（摊平）。
///
/// ⚠ 没有可用设备时**当场 Err**（生产那一侧是硬失败）；判据里那一层再把 `Err`
///   解释成"跳过"（见文件头第 4 条）。
pub fn curve_points(curve: &Curve, out_n: u32) -> Result<Vec<f32>, String> {
    curve_points_at(curve, out_n, 0.0)
}

/// [`curve_points`] 的带偏移版本。
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

/// **曲面细分**：统一细分到弦误差达标，装配走**与 CPU 同一份**
/// `px_nurbs_schema::mesh::grid_mesh`。
///
/// ⚠ 每一级要两次派发（格点 + 格心）才估得出弦误差 —— 这是"判据是弦误差"的代价，
///   而求值本身在 GPU 上，比 CPU 那一侧便宜得多。
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

/// 每一格的格心偏离四角平均的最大距离（弦误差的估计）。
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

/// **曲线细分**：统一对分到弦误差达标，装配走 `px_nurbs_schema::mesh::polyline`。
///
/// ⚠ 与 CPU 那一侧（逐段自适应对分）**拓扑可能不同**（这里每一级整体对分一次），
///   但判据是同一条：**折线到曲线的弦误差 ≤ `tolerance`**。
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
            // 偶数下标就是那一套格点（奇数下标是对分出来的中点）。
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

/// 每个中点离它两侧格点弦的最大距离。
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

#[cfg(test)]
mod tests;
