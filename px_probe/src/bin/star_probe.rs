use std::collections::BTreeMap;

use px_sparse::StarField;
use px_volume_alg::stars::bake_stars;
use px_volume_schema::VolumeData;
use px_volume_schema::params::sky::SkyParams;
use px_volume_schema::params::stars::StarsParams;

fn star_params() -> Result<StarsParams, String> {
    let path = px_graph::workspace_root()
        .join("art")
        .join("nebulasky")
        .join("stars.toml");
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("读不到 {}：{err}", path.display()))?;
    toml::from_str(&text).map_err(|err| format!("{} 解不开：{err}", path.display()))
}

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

fn vacuum(res: u32, layers: u32, inner: f32, outer: f32) -> VolumeData {
    VolumeData {
        lanes: 1,
        res,
        layers,
        inner,
        outer,
        data: vec![0.0; (6 * layers * res * res * 6) as usize],
    }
}

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

fn analytic_core_area(field: &StarField, params: &SkyParams, threshold: f64) -> Vec<(f64, f64)> {
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

fn lit_rings_of_artifact(
    path: &str,
    threshold: f32,
) -> Result<(Vec<(f64, f64, f64)>, u32), String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let bundle = px_protocol::payload::PayloadBundle::from_bytes(&bytes)
        .map_err(|err| format!("{path} 不是一份产物：{err}"))?;
    let texture =
        <px_protocol::art::TextureData as px_graph_schema::Build>::decode(&bundle, "sky")?;
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
                let texel = ((face_index * face + y) * face + x) as usize;
                let value = half(texel * 4);
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
    let density = args.next();

    let params = star_params()?;
    println!(
        "星场参数：{} 颗｜壳 {:.2}..{:.2}｜细格 {:.3}",
        params.count, params.inner, params.outer, params.cell
    );

    let started = std::time::Instant::now();
    let field = bake_stars(&params, None).map_err(|err| err.to_string())?;
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

    for radius in [0.1_f32, 0.2, 0.4] {
        let (bricks, cells, stars) = query_cost(&field, radius, 512);
        println!(
            "球查询 r = {radius:.2}：平均翻 {bricks:.1} 个 brick、{cells:.1} 个细格、{stars:.1} 颗星（逐体素那一档）"
        );
    }

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
        let Ok(tuned_field) = px_volume_alg::bake_stars(&tuned, None) else {
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

    if let Some(path) = density {
        let gas = column_density(&path, 32)?;
        let mut stars_dir = vec![0.0_f64; gas.len()];
        for index in 0..field.count() {
            let p = field.star(index).position;
            let r = ((p[0] as f64).powi(2) + (p[1] as f64).powi(2) + (p[2] as f64).powi(2)).sqrt();
            if r <= 0.0 {
                continue;
            }
            let direction = [p[0] as f64 / r, p[1] as f64 / r, p[2] as f64 / r];
            let bin = coarse_bin(direction, 8);
            stars_dir[bin] += 1.0;
        }
        let solid = coarse_solid_angles(8);
        let a: Vec<f64> = (0..gas.len()).map(|i| gas[i] / solid[i]).collect();
        let b: Vec<f64> = (0..gas.len()).map(|i| stars_dir[i] / solid[i]).collect();
        let correlation = pearson(&a, &b);
        println!(
            "星 ↔ 气的大尺度相关（{} 个方向格，各自按球面度归一）：**{correlation:+.3}**",
            gas.len()
        );
    }

    if let Some(path) = artifact {
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

fn coarse_bin(direction: [f64; 3], bins: usize) -> usize {
    let theta = direction[2].clamp(-1.0, 1.0).acos() / std::f64::consts::PI;
    let phi = direction[1].atan2(direction[0]) / std::f64::consts::TAU + 0.5;
    let row = ((theta * bins as f64) as usize).min(bins - 1);
    let column = ((phi * (2 * bins) as f64) as usize).min(2 * bins - 1);
    row * 2 * bins + column
}

fn coarse_solid_angles(bins: usize) -> Vec<f64> {
    let dphi = std::f64::consts::TAU / (2 * bins) as f64;
    let mut out = Vec::with_capacity(bins * 2 * bins);
    for row in 0..bins {
        let theta0 = std::f64::consts::PI * row as f64 / bins as f64;
        let theta1 = std::f64::consts::PI * (row + 1) as f64 / bins as f64;
        let band = (theta0.cos() - theta1.cos()).abs() * dphi;
        for _ in 0..2 * bins {
            out.push(band.max(1e-9));
        }
    }
    out
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let scale = n as f64;
    let mean_a = a.iter().sum::<f64>() / scale;
    let mean_b = b.iter().sum::<f64>() / scale;
    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;
    for index in 0..n {
        let da = a[index] - mean_a;
        let db = b[index] - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }
    if var_a <= 0.0 || var_b <= 0.0 {
        return 0.0;
    }
    cov / (var_a.sqrt() * var_b.sqrt())
}

fn column_density(path: &str, samples: usize) -> Result<Vec<f64>, String> {
    let bytes = std::fs::read(path).map_err(|err| format!("读不到 {path}：{err}"))?;
    let bundle = px_protocol::payload::PayloadBundle::from_bytes(&bytes)
        .map_err(|err| format!("{path} 不是一份产物：{err}"))?;
    let volume =
        <px_volume_schema::VolumeData as px_graph_schema::Build>::decode(&bundle, "density")?;
    let bins = 8_usize;
    {
        let count = volume.data.len() as f64;
        let mean = volume.data.iter().map(|v| *v as f64).sum::<f64>() / count.max(1.0);
        let mut max = 0.0_f64;
        for v in &volume.data {
            max = max.max(*v as f64);
        }
        println!(
            "  密度产物自检：{} 面 × {}² × {} 层｜均值 {:.4}（该与烘图日志那一行一致）、最大 {:.4}",
            px_volume_schema::volume::PATCHES,
            volume.res,
            volume.layers,
            mean,
            max
        );
        let total_cells = volume.data.len();
        let zero = volume.data.iter().filter(|v| **v == 0.0).count();
        let tiny = volume
            .data
            .iter()
            .filter(|v| **v > 0.0 && **v <= 0.01)
            .count();
        let mut live: Vec<f32> = volume.data.iter().copied().filter(|v| *v > 0.0).collect();
        live.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pick = |q: f64| -> f32 {
            if live.is_empty() {
                0.0
            } else {
                live[((live.len() - 1) as f64 * q) as usize]
            }
        };
        println!(
            "  稀疏度：恰好 0 的体素 **{:.1}%**｜(0, 0.01] 的 {:.1}%｜非零共 {} 个 ⇒ 非零分位 p10 {:.4} / p50 {:.4} / p90 {:.4}",
            100.0 * zero as f64 / total_cells as f64,
            100.0 * tiny as f64 / total_cells as f64,
            live.len(),
            pick(0.10),
            pick(0.50),
            pick(0.90)
        );
    }
    let mut out = vec![0.0_f64; bins * 2 * bins];
    let golden = 2.399_963_2_f64;
    let total = bins * 2 * bins * 16;
    let shell = px_volume_schema::volume::Shell::new(volume.inner, volume.outer);
    for index in 0..total {
        let z = 1.0 - 2.0 * (index as f64 + 0.5) / total as f64;
        let ring = (1.0 - z * z).max(0.0).sqrt();
        let phi = golden * index as f64;
        let direction = [ring * phi.cos(), ring * phi.sin(), z];
        let bin = coarse_bin(direction, bins);
        let mut column = 0.0_f64;
        for step in 0..samples {
            let u = (step as f64 + 0.5) / samples as f64;
            let radius = shell.radius_of(u as f32);
            let point = [
                direction[0] as f32 * radius,
                direction[1] as f32 * radius,
                direction[2] as f32 * radius,
            ];
            let step_world = shell.stretch_of(u as f32) as f64 / samples as f64;
            column += px_volume_alg::density::sample_world(&volume, point) as f64 * step_world;
        }
        out[bin] = out[bin].max(column);
    }
    Ok(out)
}
