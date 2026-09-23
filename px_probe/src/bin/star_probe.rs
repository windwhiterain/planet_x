//! **星场的单点探针**：星撒得均不均、稀疏格花多少内存、一次查询要翻多少东西、
//! 以及**直射项在立方图上的密度还是不是随位置变**。
//!
//! 用法：
//!
//! ```text
//! cargo run --release -p px_probe --bin star_probe -- [face] [SKY.pxart]
//! ```
//!
//! ⚠ 给了 `SKY.pxart` 就**量那份真产物**（而不是现算一份）：这是唯一能同时验到
//!   "GPU 那一侧的直射项扫的星与 CPU 语义一致"的一条 —— 少扫了星，亮面积就会掉下去。
//!   产物是 `rgba16f` 的立方贴图（`px_volume_alg::half` 解回 f32）。
//!
//! ⚠ 它量的是 2026-09-25 那次改存储模型**之前**的三个病（都用同一套口径量出来，
//!   好与旧版对照）：
//!
//! | 病 | 旧版实测 | 现在应有的值 |
//! |---|---|---|
//! | 位置密度按方向偏（立方格投到球面） | 面心 11.2e3 / 面角 15.9e3 星/球面度 = **1.43x** | 1.0x |
//! | 成品天空的亮面积比（星图 4096 点采到面 1024） | 面心 2.75% → 面角 4.33% = **1.58x** | 1.0x |
//! | 星的横向足迹（纹素角跨面差 3 倍） | σ 径向/切向 1.02 → **1.57** | 1.0 |
//!
//! ⚠ 星场参数**从 `art/nebulasky/stars.toml` 读**（不在探针里抄一份）：抄一份的下场是
//!   "探针量的是一份与烘图不同的星场"，而那种偏差看起来只是"数字不好看"。

use std::collections::BTreeMap;

use px_sparse::StarField;
use px_volume_alg::stars::bake_stars;
use px_volume_schema::VolumeData;
use px_volume_schema::params::sky::SkyParams;
use px_volume_schema::params::stars::StarsParams;

/// 读 `art/nebulasky/stars.toml`（探针只读，不改）。
fn star_params() -> Result<StarsParams, String> {
    let path = px_graph::workspace_root()
        .join("art")
        .join("nebulasky")
        .join("stars.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))
}

/// 索引与载荷各占多少字节（清单里那几段 u32/f32）。
fn memory(field: &StarField) -> BTreeMap<&'static str, usize> {
    let grid = &field.grid;
    BTreeMap::from([
        ("块 CSR", grid.chunk_start.len() * 4),
        ("brick 表", grid.brick_slot.len() * 4),
        ("掩码", grid.brick_mask.len() * 4),
        ("brick 子起点", grid.brick_sub.len() * 4),
        ("子 CSR", grid.sub_start.len() * 4),
        ("星表", field.stars.len() * 4),
    ])
}

/// **位置密度**：每个箱里的星数 ÷ 那个箱的体积（按体积均匀 ⇒ 所有箱一样）。
///
/// ⚠ 分箱必须**按体积等分**，不是按半径等分：半径等分的箱体积不同，量到的差是几何给的，
///   与撒点无关（那是"量错了工具"）。
fn density_by_band(field: &StarField, params: &StarsParams, bands: usize) -> Vec<(f64, f64)> {
    let (inner, outer) = (params.inner as f64, params.outer as f64);
    let cube = |r: f64| r * r * r;
    let mut counts = vec![0_usize; bands];
    for index in 0..field.count() {
        let p = field.star(index).position;
        let r = ((p[0] as f64).powi(2) + (p[1] as f64).powi(2) + (p[2] as f64).powi(2)).sqrt();
        let t = ((cube(r) - cube(inner)) / (cube(outer) - cube(inner))).clamp(0.0, 0.999_999);
        counts[(t * bands as f64) as usize] += 1;
    }
    let shell = 4.0 / 3.0 * std::f64::consts::PI * (cube(outer) - cube(inner));
    (0..bands)
        .map(|band| {
            let volume = shell / bands as f64;
            (counts[band] as f64 / volume, counts[band] as f64)
        })
        .collect()
}

/// **方向各向同性**：八个卦限里的星数（卦限体积相同 ⇒ 数直接可比）。
fn density_by_octant(field: &StarField) -> Vec<usize> {
    let mut counts = vec![0_usize; 8];
    for index in 0..field.count() {
        let p = field.star(index).position;
        let octant = (p[0] >= 0.0) as usize
            | (((p[1] >= 0.0) as usize) << 1)
            | (((p[2] >= 0.0) as usize) << 2);
        counts[octant] += 1;
    }
    counts
}

/// 一份**真空**发射体积（只剩星与背景）：直射项才能单独量。
fn vacuum(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
    VolumeData {
        res,
        layers,
        inner,
        outer,
        data: vec![0.0; (6 * layers * res * res * 6) as usize],
    }
}

/// **直射项在立方图上的密度**：按"离面心的距离"分三个环带，数亮面积占该带的比例。
///
/// ⚠ 这正是用户报的那个症状的量法（旧版 1.58x）。现在星是世界坐标里的点、轮廓是
///   弧度的角函数、每条视线按**方向**解析求值 ⇒ 三个环带必须一样。
fn lit_by_ring(field: &StarField, params: &SkyParams, face: u32) -> Vec<(f64, f64, f64)> {
    let scene = vacuum(8, 4, 1.0, 2.0);
    let stars = SkyParams {
        face,
        steps: 4,
        jitter: 0.0,
        star_halo_gain: 0.0,
        ..params.clone()
    };
    let plane = px_volume_alg::raymarch_channel(&scene, Some(field), &stars, 0);
    let mut lit = [0.0_f64; 3];
    let mut area = [0.0_f64; 3];
    for face_index in 0..6 {
        for y in 0..face {
            for x in 0..face {
                let a = (x as f32 + 0.5) / face as f32 * 2.0 - 1.0;
                let b = (y as f32 + 0.5) / face as f32 * 2.0 - 1.0;
                let r = (a * a + b * b).sqrt();
                let band = if r < 0.5 {
                    0
                } else if r < 0.9 {
                    1
                } else {
                    2
                };
                let value = plane.at(x, face_index * face + y);
                // ⚠⚠ 按**球面度**加权，不是数 texel：立方图一个 texel 的立体角是
                //   `4/(1+a²+b²)^{3/2}/face²` —— 面角比面心小 **5.2 倍**（a=b=1 时
                //   `3^{3/2} = 5.196`）⇒ 数 texel 会把面角的亮面积凭空抬高（同一个角半径的
                //   星在面角盖住 5 倍多的 texel）。旧版那个 1.58x 量的是**球面度**，
                //   所以这里也必须按球面度，否则两件事不可比。
                let weight =
                    1.0 / ((1.0 + (a * a + b * b) as f64) * (1.0 + (a * a + b * b) as f64).sqrt());
                area[band] += weight;
                if value > 0.05 {
                    lit[band] += weight;
                }
            }
        }
    }
    (0..3)
        .map(|band| (lit[band] / area[band], lit[band], area[band]))
        .collect()
}

/// **解析版的位置均匀性**：把每颗星的"亮于阈值的那块角面积"按它所在的环带累加，
/// 再除以**带内的球面度**。
///
/// ⚠⚠ 为什么要有这一条（而不是只看烘出来的贴图）：产物那一侧混着**气**（亮的带本来就多
///   过线的）、**分级曲线**（非线性）与**立方图 texel 的立体角差 5.2 倍**（面角一个 texel
///   的立体角只有面心的 1/5.2 ⇒ 同一个角半径的星在面角盖住 5 倍多的 texel）。
///   这一条把星这一层**单独**拿出来量：真空（透过率 1）、不给分级、按球面度加权。
///
/// 面积取高斯的解析式 `π σ² ln(P / 阈值)`（`P = 亮度 × 增益`；`P ≤ 阈值` 时为 0）——
/// ⚠ 它是**每颗星各自**的，与采样分辨率无关。
fn analytic_core_area(field: &StarField, params: &SkyParams, threshold: f64) -> Vec<(f64, f64)> {
    // 三个带的球面度（把立方图表面按 `1/(1+a²+b²)^{3/2}` 积一遍）。
    let grid = 2048_usize;
    let mut solid = [0.0_f64; 3];
    for face in 0..6 {
        for y in 0..grid {
            for x in 0..grid {
                let a = (x as f64 + 0.5) / grid as f64 * 2.0 - 1.0;
                let b = (y as f64 + 0.5) / grid as f64 * 2.0 - 1.0;
                let _ = face;
                let r = (a * a + b * b).sqrt();
                let band = if r < 0.5 {
                    0
                } else if r < 0.9 {
                    1
                } else {
                    2
                };
                let weight = 1.0 / ((1.0 + a * a + b * b) * (1.0 + a * a + b * b).sqrt());
                solid[band] += weight;
            }
        }
    }
    let mut lit = [0.0_f64; 3];
    let sigma = params.star_core.max(1e-6) as f64;
    for index in 0..field.count() {
        let star = field.star(index);
        let p = star.position;
        let length = ((p[0] as f64).powi(2) + (p[1] as f64).powi(2) + (p[2] as f64).powi(2)).sqrt();
        if length <= 0.0 {
            continue;
        }
        // 方向 → 立方图的面 + (a, b)（取主轴那一面，与 `cube_direction` 同一套参数）。
        let d = [
            p[0] as f64 / length,
            p[1] as f64 / length,
            p[2] as f64 / length,
        ];
        let axis = (0..3)
            .max_by(|x, y| d[*x].abs().partial_cmp(&d[*y].abs()).expect("没有 NaN"))
            .unwrap_or(0);
        let major = d[axis].abs().max(1e-9);
        let (u, v) = match axis {
            0 => (d[1], d[2]),
            1 => (d[0], d[2]),
            _ => (d[0], d[1]),
        };
        let a = u / major;
        let b = v / major;
        let r = (a * a + b * b).sqrt();
        let band = if r < 0.5 {
            0
        } else if r < 0.9 {
            1
        } else {
            2
        };
        let peak = star.brightness as f64 * params.star_gain as f64;
        if peak <= threshold {
            continue;
        }
        lit[band] += std::f64::consts::PI * sigma * sigma * (peak / threshold).ln();
    }
    (0..3)
        .map(|band| (lit[band] / solid[band], solid[band]))
        .collect()
}

/// **一次球查询要翻多少东西**（`cloud.emission` 的逐体素光照那一档的形状）。
fn query_cost(field: &StarField, radius: f32, probes: usize) -> (f64, f64, f64) {
    let mut bricks = 0.0_f64;
    let mut cells = 0.0_f64;
    let mut stars = 0.0_f64;
    let golden = 2.399_963_2_f32;
    for index in 0..probes {
        let z = 1.0 - 2.0 * (index as f32 + 0.5) / probes as f32;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        let phi = golden * index as f32;
        let point = [ring * phi.cos() * 2.0, ring * phi.sin() * 2.0, z * 2.0];
        let low = [point[0] - radius, point[1] - radius, point[2] - radius];
        let high = [point[0] + radius, point[1] + radius, point[2] + radius];
        let mut seen_bricks: Vec<u32> = Vec::new();
        field.for_each_cell_in(low, high, |cell, range| {
            cells += 1.0;
            stars += range.len() as f64;
            if let Some(brick) = field.grid.brick_at(cell) {
                seen_bricks.push(brick);
            }
        });
        seen_bricks.sort_unstable();
        seen_bricks.dedup();
        bricks += seen_bricks.len() as f64;
    }
    let scale = probes as f64;
    (bricks / scale, cells / scale, stars / scale)
}

/// **量一份烘好的天空产物**：逐 texel 取 R（星点是白的 ⇒ 任一通道都能代表它），
/// 按"离面心的距离"分三个环带数亮面积。
///
/// ⚠ 用真产物量（不是现算）有一个别处替代不了的用处：**GPU 那一侧少扫了星，这里立刻掉**。
fn lit_rings_of_artifact(
    path: &str,
    threshold: f32,
) -> Result<(Vec<(f64, f64, f64)>, u32), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let bundle = px_protocol::payload::PayloadBundle::from_bytes(&bytes)
        .map_err(|err| format!("{path} 不是一份产物：{err}"))?;
    let texture = <px_protocol::art::TextureData as px_protocol::payload::Build>::decode(
        &bundle,
        px_protocol::art::Domain::CubeMap,
        "sky",
    )?;
    let face = texture.width;
    if texture.format != px_protocol::art::TextureFormat::Rgba16Float {
        return Err(format!(
            "天空产物的格式是 {:?}（这份探针只认 rgba16f）",
            texture.format
        ));
    }
    let half = |at: usize| -> f32 {
        let bits = u16::from_le_bytes([texture.bytes[at * 2], texture.bytes[at * 2 + 1]]);
        px_volume_alg::half::f32_from_half(bits)
    };
    let mut lit = [0.0_f64; 3];
    let mut area = [0.0_f64; 3];
    for face_index in 0..6u32 {
        for y in 0..face {
            for x in 0..face {
                let a = (x as f32 + 0.5) / face as f32 * 2.0 - 1.0;
                let b = (y as f32 + 0.5) / face as f32 * 2.0 - 1.0;
                let r = (a * a + b * b).sqrt();
                let band = if r < 0.5 {
                    0
                } else if r < 0.9 {
                    1
                } else {
                    2
                };
                // 布局与 `VolumeData` 同一个口径：行 = 面 × 面 + y，每行 `面 × 4` 个通道。
                let texel = ((face_index * face + y) * face + x) as usize;
                let value = half(texel * 4);
                // ⚠ 与 `lit_by_ring` 同一个口径：**按球面度加权**（面角的 texel 小 5.2 倍）。
                let radius2 = (a * a + b * b) as f64;
                let weight = 1.0 / ((1.0 + radius2) * (1.0 + radius2).sqrt());
                area[band] += weight;
                if value > threshold {
                    lit[band] += weight;
                }
            }
        }
    }
    Ok((
        (0..3)
            .map(|band| (lit[band] / area[band], lit[band], area[band]))
            .collect(),
        face,
    ))
}

fn ratio(values: &[f64]) -> f64 {
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    if min <= 0.0 { f64::INFINITY } else { max / min }
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let face: u32 = args
        .next()
        .and_then(|text| text.parse().ok())
        .unwrap_or(128)
        .max(16);
    let artifact = args.next();

    let params = star_params()?;
    println!(
        "星场参数：{} 颗｜壳 {:.2}..{:.2}｜细格 {:.3}",
        params.count, params.inner, params.outer, params.cell
    );

    let started = std::time::Instant::now();
    let field = bake_stars(&params).map_err(|err| err.to_string())?;
    println!(
        "撒点 + 建格：{:.2} 秒｜{} 颗｜{} 个 brick｜{} 个占用细格｜每 brick {:.2} 颗、每细格 {:.2} 颗",
        started.elapsed().as_secs_f64(),
        field.count(),
        field.grid.bricks(),
        field.grid.occupied_cells(),
        field.count() as f64 / field.grid.bricks().max(1) as f64,
        field.count() as f64 / field.grid.occupied_cells().max(1) as f64,
    );

    let memory = memory(&field);
    let total: usize = memory.values().sum();
    let text: Vec<String> = memory
        .iter()
        .map(|(name, bytes)| format!("{name} {:.2} MB", *bytes as f64 / 1048576.0))
        .collect();
    println!(
        "索引 + 载荷：合计 {:.2} MB（{}）",
        total as f64 / 1048576.0,
        text.join("｜")
    );

    // ── 位置密度 ────────────────────────────────────────────────────────────
    let bands = density_by_band(&field, &params, 3);
    let band_density: Vec<f64> = bands.iter().map(|(density, _)| *density).collect();
    println!(
        "按体积等分的三层（内→外）每单位体积星数：{:.1} / {:.1} / {:.1}｜最大最小比 **{:.3}**",
        band_density[0],
        band_density[1],
        band_density[2],
        ratio(&band_density)
    );
    let octants = density_by_octant(&field);
    let octant_counts: Vec<f64> = octants.iter().map(|count| *count as f64).collect();
    println!(
        "八个卦限的星数：{:?}｜最大最小比 **{:.3}**（按球面度均匀 ⇒ 应接近 1）",
        octants,
        ratio(&octant_counts)
    );

    // ── 查询成本 ────────────────────────────────────────────────────────────
    for radius in [0.1_f32, 0.2, 0.4] {
        let (bricks, cells, stars) = query_cost(&field, radius, 512);
        println!(
            "球查询 r = {radius:.2}：平均翻 {bricks:.1} 个 brick、{cells:.1} 个细格、{stars:.1} 颗星（逐体素那一档）"
        );
    }

    // ── 天空直射项在立方图上的密度 ──────────────────────────────────────────
    let params_sky: SkyParams = toml::from_str(
        &std::fs::read_to_string(
            px_graph::workspace_root()
                .join("art")
                .join("nebulasky")
                .join("sky.toml"),
        )
        .map_err(|err| format!("读不到 sky.toml：{err}"))?,
    )
    .map_err(|err| format!("sky.toml 解不开：{err}"))?;
    println!(
        "面 {face} 的直射项（真空 + 只留星核）：核 {:.4} rad ⇒ 面心纹素角 {:.2} 倍",
        params_sky.star_core,
        params_sky.star_core / (2.0 / face as f32)
    );
    let rings = lit_by_ring(&field, &params_sky, face);
    let lit: Vec<f64> = rings.iter().map(|(value, _, _)| *value).collect();
    println!(
        "亮面积占比（面心环 / 中环 / 面角环）：{:.4}% / {:.4}% / {:.4}%｜面角/面心 **{:.3}x**（旧版实测 1.58x）",
        lit[0] * 100.0,
        lit[1] * 100.0,
        lit[2] * 100.0,
        if lit[0] > 0.0 {
            lit[2] / lit[0]
        } else {
            f64::INFINITY
        }
    );

    // ⚠ 这一条才是**星层单独**的判据（真空、不分级、按球面度加权）：上面那条现算与下面
    //   那条产物都被气/分级/texel 面积混着，只有这一条能指认"星自己偏没偏"。
    let analytic = analytic_core_area(&field, &params_sky, 0.05);
    println!(
        "星核亮面积（解析、真空、按球面度）：面心 {:.3e} / 中 {:.3e} / 面角 {:.3e}｜面角/面心 **{:.3}x**",
        analytic[0].0,
        analytic[1].0,
        analytic[2].0,
        if analytic[0].0 > 0.0 {
            analytic[2].0 / analytic[0].0
        } else {
            f64::INFINITY
        }
    );

    // ── 直射项：每条视线的候选星（GPU 那份定长队列该开多大，靠这个数说话）──────
    let support = px_volume_alg::raymarch::star_support(&params_sky);
    let mut per_ray: Vec<usize> = Vec::new();
    let mut per_slab: Vec<usize> = Vec::new();
    let mut probes = 0;
    let mut worst: Vec<usize> = vec![0; 64];
    let golden = 2.399_963_2_f32;
    let mut over32 = 0_usize;
    let mut over16 = 0_usize;
    let mut over8 = 0_usize;
    let mut total_slabs = 0_usize;
    let mut worst_at = (0_usize, 0_usize, 0.0_f32);
    for index in 0..4096 {
        let z = 1.0 - 2.0 * (index as f32 + 0.5) / 4096.0;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        let phi = golden * index as f32;
        let direction = [ring * phi.cos(), ring * phi.sin(), z];
        let counts = px_volume_alg::raymarch::slab_candidate_counts(
            &field,
            direction,
            params.inner,
            params.outer,
            support,
        );
        per_ray.push(counts.iter().sum());
        for (slab, count) in counts.iter().enumerate() {
            if slab < worst.len() {
                worst[slab] = worst[slab].max(*count);
            }
            total_slabs += 1;
            if *count > 32 {
                over32 += 1;
            } else if *count > 16 {
                over16 += 1;
            } else if *count > 8 {
                over8 += 1;
            }
            if *count > worst_at.2 as usize {
                worst_at = (index, slab, *count as f32);
            }
            per_slab.push(*count);
        }
        probes += 1;
    }
    let mean = |values: &[usize]| -> f64 {
        values.iter().map(|value| *value as f64).sum::<f64>() / values.len().max(1) as f64
    };
    let max = |values: &[usize]| -> usize { values.iter().copied().max().unwrap_or(0) };
    println!(
        "锥查询（支持域 {support:.4} rad，{probes} 条视线）：每条视线候选 平均 {:.1}、最多 {}｜单层候选 平均 {:.2}、**最多 {}**",
        mean(&per_ray),
        max(&per_ray),
        mean(&per_slab),
        max(&per_slab),
    );
    // ⚠ 逐条视线的候选数**必须只差泊松**（σ = √均值）：它是"星按球面度均匀 + 视线穿过壳的
    //   路径一样长"的直接推论。这一列也是"某个方向星特别多/特别少"的唯一量法
    //   —— 画面上"某一片星更密"多半只是那里**气更暗**（对比度），不是密度。
    let mut sorted_rays = per_ray.clone();
    sorted_rays.sort_unstable();
    let pick = |q: f64| -> usize { sorted_rays[((sorted_rays.len() - 1) as f64 * q) as usize] };
    println!(
        "  逐条视线候选分位：最小 {}、p10 {}、中位 {}、p90 {}、最大 {}（泊松给的散布约 ±{:.0}）",
        sorted_rays[0],
        pick(0.10),
        pick(0.50),
        pick(0.90),
        sorted_rays[sorted_rays.len() - 1],
        mean(&per_ray).sqrt(),
    );
    println!("  逐层上限（按半径分段，前 12 段）：{:?}", &worst[..12]);
    println!(
        "  单层候选的分布（{} 层）：>8 有 {over8} 层、>16 有 {over16} 层、**>32 有 {over32} 层**｜最坏一条的第 {} 层 = {} 颗",
        total_slabs, worst_at.1, worst_at.2 as usize
    );

    // ⚠ 单层 40 颗是个**离群**（基线约 1）：先确认星场自己有没有"抱团"（同一个位置很多颗、
    //   或者某处密度是别处的几十倍）—— 那会让任何定长队列都不够，而且看不出来。
    let mut worst_cell = 0_usize;
    let mut worst_cell_at = [0.0_f32; 3];
    for index in 0..512 {
        let z = 1.0 - 2.0 * (index as f32 + 0.5) / 512.0;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        let phi = 2.399_963_2_f32 * index as f32;
        let point = [ring * phi.cos() * 2.0, ring * phi.sin() * 2.0, z * 2.0];
        let count = px_volume_alg::stars_near(&field, point, 0.05).len();
        if count > worst_cell {
            worst_cell = count;
            worst_cell_at = point;
        }
    }
    println!(
        "  半径 0.05 的球里最多有 {worst_cell} 颗星（均匀场该是 1~2；抱团会在这里露出来）@ {worst_cell_at:?}"
    );
    // 重合的星：位置逐位相同的对数（哈希相关会让一批星落在同一个点上）。
    let mut positions: Vec<[u32; 3]> = (0..field.count())
        .map(|index| {
            let p = field.star(index).position;
            [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()]
        })
        .collect();
    positions.sort_unstable();
    let before = positions.len();
    positions.dedup();
    println!(
        "  不同的星位 {} / {}（差 {} 个 ⇒ 有重合）",
        positions.len(),
        before,
        before - positions.len()
    );

    // ⚠ **按体积等分**分箱（`r³` 均匀）：按半径等分的话每箱体积 ∝ r²，量到的 9 倍差全是几何给的。
    let bins = 128_usize;
    let mut histogram = vec![0_usize; bins];
    for index in 0..field.count() {
        let p = field.star(index).position;
        let r = ((p[0] as f64).powi(2) + (p[1] as f64).powi(2) + (p[2] as f64).powi(2)).sqrt();
        let cube = |value: f64| value * value * value;
        let t = ((cube(r) - cube(params.inner as f64))
            / (cube(params.outer as f64) - cube(params.inner as f64)))
        .clamp(0.0, 0.999_999);
        histogram[(t * bins as f64) as usize] += 1;
    }
    let expected = field.count() as f64 / bins as f64;
    let worst_bin = histogram.iter().copied().max().unwrap_or(0);
    let thinnest_bin = histogram.iter().copied().min().unwrap_or(0);
    println!(
        "  按体积等分的半径直方图 {bins} 段：每段期望 {expected:.0}、最厚 {worst_bin}、最薄 {thinnest_bin}（{:.3}x）",
        worst_bin as f64 / thinnest_bin.max(1) as f64
    );

    // ⚠ 最坏那条视线的逐层剖面：看它是**一层突然装下几十颗**（并集式的异常）还是均匀铺开。
    let worst_ray = {
        let mut best = (0_usize, 0_usize);
        for index in 0..4096 {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / 4096.0;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = 2.399_963_2_f32 * index as f32;
            let direction = [ring * phi.cos(), ring * phi.sin(), z];
            let counts = px_volume_alg::raymarch::slab_candidate_counts(
                &field,
                direction,
                params.inner,
                params.outer,
                support,
            );
            let total: usize = counts.iter().sum();
            if total > best.0 {
                best = (total, index);
            }
        }
        let z = 1.0 - 2.0 * (best.1 as f32 + 0.5) / 4096.0;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        let phi = 2.399_963_2_f32 * best.1 as f32;
        let direction = [ring * phi.cos(), ring * phi.sin(), z];
        let counts = px_volume_alg::raymarch::slab_candidate_counts(
            &field,
            direction,
            params.inner,
            params.outer,
            support,
        );
        (best.0, direction, counts)
    };
    println!(
        "  最坏那条视线（{worst_ray:?}）—— 方向 {:?}，逐层候选（半径 0.95 起每 0.05 一段）：",
        worst_ray.1
    );
    let nonzero: Vec<(usize, usize)> = worst_ray
        .2
        .iter()
        .enumerate()
        .filter(|(_, count)| **count > 0)
        .map(|(slab, count)| (slab, *count))
        .collect();
    println!("    {nonzero:?}");
    // 那条视线上、最坏那一层的 AABB 里到底有多少颗星（"测过多少"对"收下多少"）。
    let slab = nonzero
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(slab, _)| *slab)
        .unwrap_or(0);
    let t1 = 0.95 + (slab as f32 + 1.0) * 0.05;
    let half = (t1 * support + 0.05).max(0.05);
    let centre = [
        worst_ray.1[0] * t1,
        worst_ray.1[1] * t1,
        worst_ray.1[2] * t1,
    ];
    let inside = px_volume_alg::stars_near(&field, centre, half * 1.732).len();
    println!(
        "    第 {slab} 层（t1 = {t1:.2}、半宽 {half:.3}）：AABB 外接球里有 {inside} 颗星（这是「测过」的上界）"
    );

    // ⚠ **按表观亮度剔除**的账（用户 2026-09-25）：`1/r²` 之后远处的暗星读不出来 ⇒ 剔掉。
    //   三栏一起看才够：①剔多少颗 ②每条视线的候选降多少（省下来的那件事）
    //   ③**照亮气体**那一档的辐照降多少（被剔的星本来就最暗，但剔多了气会跟着暗）。
    //   ⚠ ③ 用未封顶的和（`starlight_max` 截断是 emission 那一档的事）—— 这里要的是相对变化。
    let (light_radius, light_soft) = (0.2_f32, 0.05_f32);
    let lit_at = |field: &StarField| -> f64 {
        let probes = 256;
        let mut total = 0.0_f64;
        for index in 0..probes {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / probes as f32;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = 2.399_963_2_f32 * index as f32;
            let point = [ring * phi.cos() * 2.0, ring * phi.sin() * 2.0, z * 2.0];
            for star in px_volume_alg::stars_near(field, point, light_radius) {
                let delta = [
                    star.position[0] - point[0],
                    star.position[1] - point[1],
                    star.position[2] - point[2],
                ];
                let distance2 = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
                total += (star.brightness / (distance2 + light_soft * light_soft)) as f64;
            }
        }
        total / probes as f64
    };
    let baseline_lit = lit_at(&field);
    println!("按表观亮度剔除（阈值 → 存活 / 每条视线候选 / 气体受照）：");
    for threshold in [0.0_f32, 0.05, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0] {
        let mut tuned = params.clone();
        tuned.min_apparent = threshold;
        let Ok(tuned_field) = px_volume_alg::bake_stars(&tuned) else {
            continue;
        };
        let support = px_volume_alg::raymarch::star_support(&params_sky);
        let mut total = 0_usize;
        let probes = 512;
        for index in 0..probes {
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / probes as f32;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = 2.399_963_2_f32 * index as f32;
            let direction = [ring * phi.cos(), ring * phi.sin(), z];
            total += px_volume_alg::raymarch::slab_candidate_counts(
                &tuned_field,
                direction,
                params.inner,
                params.outer,
                support,
            )
            .iter()
            .sum::<usize>();
        }
        let lit = lit_at(&tuned_field);
        println!(
            "  {threshold:>5.3} → {:>7} 颗（{:>5.1}%）／候选 {:>5.1}／气体受照 {:.1}%（该星核峰值 {:.4}）",
            tuned_field.count(),
            100.0 * tuned_field.count() as f64 / params.count.max(1) as f64,
            total as f64 / probes as f64,
            100.0 * lit / baseline_lit.max(1e-9),
            threshold * params_sky.star_gain,
        );
    }

    if let Some(path) = artifact {
        // ⚠ 两个阈值都要看：**0.05 那一档量的是"整幅亮不亮"**（气也过线 ⇒ 它反映气的分布
        //   与分级曲线），而 **0.5 那一档只有星核过线**（气很少那么亮）⇒ 后者才是
        //   "星在立方图上的位置偏不偏"。旧版那个 1.58x 是量在星这一档上的，两个都印出来，
        //   免得拿混淆的那一档去比。
        for threshold in [0.05_f32, 0.5] {
            let (rings, face) = lit_rings_of_artifact(&path, threshold)?;
            let lit: Vec<f64> = rings.iter().map(|(value, _, _)| *value).collect();
            println!(
                "产物 {path}（面 {face}，阈值 {threshold:.2}）：亮面积占比 {:.4}% / {:.4}% / {:.4}%｜面角/面心 **{:.3}x**",
                lit[0] * 100.0,
                lit[1] * 100.0,
                lit[2] * 100.0,
                if lit[0] > 0.0 {
                    lit[2] / lit[0]
                } else {
                    f64::INFINITY
                }
            );
        }
    }
    Ok(())
}
