//! **世界坐标里的星场**：撒点（按体积均匀）+ 交给**统一稀疏格**索引。
//!
//! ⚠⚠ 这一档存在的理由（用户 2026-09-25 的口径）：**星是点，不是贴图上的像素**。
//!   旧那一档（`field.stars` 出 `CubeMap` 场）把星"画"进一张方向网格里，于是三件事
//!   一起发生，而它们与星本身毫无关系：
//!
//!   1. **一个面上纹素角差 3 倍**（面积差 5 倍）⇒ 世界空间里一个**圆**被存成**椭圆**
//!      （实测：面心 σ 径向/切向 = 1.02、面角 1.57），而"星核占几个纹素"随位置变；
//!   2. 星的**位置**由"方向 × 格数取整"得到 —— 立方格在 xyz 里均匀 ≠ 球面上每球面度均匀
//!      （实测面心 11.2e3 星/球面度、面角 15.9e3，1.43x，闭式就是 `|dₓ|+|d_y|+|d_z|`）；
//!   3. 星图与天空面必须**分辨率配对**：实测面 1024 的天空点采面 4096 的星图，
//!      亮面积从面心到面角涨 1.58x（小足迹的星被 1/16 抽样漏掉）。
//!
//!   ⇒ 星只在**世界坐标里撒点**，索引交给 `px_sparse`（R3 稀疏格，球查询各向同性）。
//!   这一档于是只剩"位置、亮度、色"三件事 —— 纯几何，没有分辨率、没有投影、没有接缝。
//!
//! ⚠ 星**按体积均匀**（不是按球面均匀）：它们混在气里 ⇒ 密度是体积密度。
//!   径向按 `r³` 均匀取样（`r = (inner³ + u·(outer³ − inner³))^(1/3)`），
//!   方向按 `z` 与 `φ` 均匀 —— 这一条是闭式的，没有近似、也没有要调的偏置。

use px_sparse::grid::{CHUNK_CELLS, GridMeta};
use px_sparse::{Star, StarField};

use crate::raymarch::star_falloff;
use px_volume_schema::params::stars::StarsParams;

/// 一颗星的确定性随机源：**纯函数**（同一个种子 + 同一个序号永远同一颗星）。
///
/// ⚠ 它必须与"重烘两次逐字节相同"这条地基一致（缓存键 = 内容）⇒ 不许有时间、不许有
///   随机数发生器状态。这一份是 `lattice3` 的同族（同样的三轮 xor-shift-multiply）。
fn hash(seed: u32, index: u32) -> u32 {
    let mut h = index.wrapping_mul(0x9e37_79b9) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    h
}

/// 32 位哈希 → `[0, 1)` 的 **24 位**小数（f32 尾数的全部精度）。
///
/// ⚠⚠ **为什么不是噪声那一档的 8 位**（`px_field_alg::noise::unit`，那是本仓库的惯例）：
///   8 位只有 256 档，而 `direction_of` 是 `z = 2u − 1` —— `u` 取到 1 时 `z` 正好是 1、
///   `ring = √(1 − z²) = 0` ⇒ **这一档的星全落在同一个方向上**（实测 703 颗挤在 `(0,0,1)`）。
///   一条近轴视线的"单层候选"因此从基线 1 冲到 **40**（整条视线 701 颗，而均匀场该是 40），
///   把 GPU 那一侧"按层收、按半径消费"的定长队列撑爆（实测溢出 24 万次）。
///   8 位还会让同一颗位置被重复用到（实测 18 万颗里有 1339 颗位置逐位重合）。
///
/// ⚠ 这类错**不会**被"按体积分层""按卦限"那种判据抓到（它们量的是体积与八分球，都对得上），
///   只会以"某一层突然装下几十颗"的尾巴露出来 —— 判据是 `slab_candidate_counts`。
fn unit24(hash: u32) -> f32 {
    (hash >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// 亮度：**暗的多、亮的少**。
///
/// ⚠ 取 `(1 / draw)^power`（不是 `draw^power`）：`draw ≤ 1` 时前者把**中位数压得很低、
///   尾巴拉得很长**，后者会把典型的星推向饱和 —— 那正好是幂律的反面
///   （与旧 `field.stars` 同一条理由，这一条没变）。
fn brightness(hash: u32, params: &StarsParams) -> f32 {
    let draw = unit24(hash).max(1e-3);
    (1.0 / draw)
        .powf(params.brightness_power)
        .min(params.max_brightness.max(1.0))
}

/// 单位方向：**按立体角均匀**（`z` 均匀、`φ` 均匀 ⇒ 球面上每球面度一样密）。
///
/// ⚠ 这一条与"位置按体积均匀"是两件事，但**都必须按世界坐标的度量来**
///   （这正是旧版栽的地方：格点在 xyz 里均匀，投到球面上就不均匀了）。
fn direction_of(u: f32, v: f32) -> [f32; 3] {
    let z = 2.0 * u - 1.0;
    let phi = std::f32::consts::TAU * v;
    let ring = (1.0 - z * z).max(0.0).sqrt();
    [ring * phi.cos(), ring * phi.sin(), z]
}

/// 一颗星的半径：**按体积均匀**（`r³` 线性）。
fn radius_of(u: f32, params: &StarsParams) -> f32 {
    let inner = params.inner.max(0.0);
    let outer = params.outer.max(inner);
    let cube = inner * inner * inner + u * (outer * outer * outer - inner * inner * inner);
    cube.max(0.0).cbrt()
}

/// **大尺度：星跟着气那一族的噪声走**（用户 2026-09-25："让星星和星云在大尺度上分布近似"）。
///
/// ⚠⚠ 算式与 `px_field_op::noise::fbm_3` **逐行相同**（同一条 `faded_gradient_noise_3`
///   累加、同一个 `seed ^ octave`），只是搬在星场这一侧 —— 这样 `stars.toml` 里写
///   上 `art/nebula/envelope.toml` 的频率/种子/octaves，星与气的**大尺度图案就是同一个**。
///   ⚠ 抄的是**数值**（频率、种子），不是把那条 remap 链也搬过来：那串阈值/mix 是气自己的
///   造型，星只要"跟它像" —— 用户的原话是"近似"。
///
/// ⚠ 为什么不用 `px_field_op` 那一份：那是 **op 层**（dylib），算法层不该反向依赖它。
///   噪声的**原件**在 `px_field_alg::noise`（本函数用的就是它）。
fn sky_density(direction: [f32; 3], params: &StarsParams) -> f32 {
    let settings = px_field_schema::noise::FbmSettings {
        frequency: params.sky_frequency,
        octaves: params.sky_octaves,
        lacunarity: params.sky_lacunarity,
        gain: params.sky_gain,
        seed: params.sky_seed,
    };
    // ⚠ `zonal` 只拉纬度分量（与 `fbm` 算子那一档同一条）：`1.0` = 各向同性。
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
    // 梯度噪声在 `[-1, 1]`（夹一下再映射到 `[0, 1]`）。
    value.clamp(-1.0, 1.0) * 0.5 + 0.5
}

/// 格的空间参数：**盒子要罩住壳，而且每边留两格**。
///
/// ⚠ 留余量不是保险而是**必需**：查询点（体素/视线采样点）在壳内，而它要看的星可能在
///   相邻的格里 —— 盒子正好卡在壳外壁上会让"外壁附近的星查不到"，症状是"壳边上星少一圈"。
///
/// ⚠ `dims` 每轴对齐到 `CHUNK_CELLS` 的整数倍：细格 → (块, brick, 局部) 的分解是
///   位移与掩码（`px_sparse::grid` 那条口径），不是除法。
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

/// 撒一批星 → **R3 星场**（索引 + 点表）。
///
/// ⚠ 细格边长（`cell`）与光照半径（`light_radius`）是**两个旋钮**（用户 2026-09-25 定的）：
///   前者是**存储分辨率**（一个格装几颗星），后者是**物理查询半径**（星光能照多远）。
///   把它们绑死会在"想细一点"和"想照远一点"之间二选一。
pub fn bake_stars(
    params: &StarsParams,
    density: Option<&px_volume_schema::VolumeData>,
) -> Result<StarField, String> {
    let seed = params.seed;
    // ⚠⚠ **精确相关**用的参照密度（拒绝采样只能变稀 ⇒ 拿平均密度当 1.0 的基准，
    //   气的峰上保持原样、别处按幂律稀下去）。一遍累加，量级是几十毫秒。
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

    // ⚠ **簇心**先撒好（体积均匀撒在壳里）：成团那一部分星以它们为心落在 `clump_radius` 内。
    //   簇心用**自己的种子**（`0x1b87_3f21`）⇒ 与星的哈希流无关，改簇数不会挪动别的星。
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
        // ⚠ 三个通道各取**一颗独立的哈希**（24 位小数）：位置与亮度共用一个种子的相邻字节
        //   会让"亮的星总长在格的同一角"，而半径与方向的低位共用会让两个通道相关。
        let h0 = hash(seed, index as u32);
        let h1 = hash(seed ^ 0x9e37_79b9, index as u32);
        let h2 = hash(seed ^ 0x85eb_ca6b, index as u32);
        let value = brightness(hash(seed ^ 0x51ed_270b, index as u32), params);
        // 成团还是均匀：**同一颗星自己决定**（与位置/亮度的哈希独立）⇒ 换个比例只挪走
        // 该挪的那些星，其余星位逐位不变。
        let clumped = clump_count > 0
            && params.clump_share > 0.0
            && unit24(hash(seed ^ 0x6d2b_79f5, index as u32)) < params.clump_share;
        let position = if clumped {
            let pick = unit24(hash(seed ^ 0x4a1c_9e37, index as u32));
            let centre = clumps[((pick * clump_count as f32) as usize).min(clump_count - 1)];
            // 簇内**按体积均匀**（球里均匀，不是壳上均匀）⇒ `u^(1/3)`，方向按球面度均匀。
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
        // ⚠⚠ **精确相关**（用户 2026-09-25 定的一档）：按**真实密度场**拒绝采样 ⇒
        //   星云在哪、星就在哪（大尺度上同分布）。实测只抄 `envelope` 那一层噪声时
        //   相关只有 +0.022 —— 气的分布是整条链的阈值/mix 定的，光对上低频层对不出来。
        //   ⚠ 抽签的哈希与位置/亮度/其他偏置的哈希各自独立（改对比度不会挪动位置）。
        if params.gas_biased {
            if let (Some(volume), Some(mean)) = (density, gas_mean) {
                let here = crate::density::sample_world(volume, position).max(0.0);
                // ⚠ 地板以下**一颗都不留**（空洞必须真的空 —— 用户 2026-09-25）。
                let ratio = here / mean;
                let floor = params.gas_floor.clamp(0.0, 0.999);
                let over = ((ratio - floor) / (1.0 - floor)).clamp(0.0, 1.0);
                // ⚠ 再按**半径**压向外侧：越靠外，相机与星之间的气柱越长 ⇒ 越容易被遮住 ✓
                //   （相机在壳内 ⇒ 近侧的星前面没有气，消光遮不住它们 ✗ —— 用户 2026-09-25）
                let span = (params.outer - params.inner).max(1e-4);
                let outward = ((r - params.inner) / span).clamp(0.0, 1.0);
                let chance = over.powf(params.gas_contrast) * outward.powf(params.gas_depth);
                if unit24(hash(seed ^ 0x77c1_5a3d, index as u32)) >= chance {
                    continue;
                }
            }
        }
        // ⚠⚠ **大尺度偏置**（用户 2026-09-25："让星星和星云在大尺度上分布近似"）：
        //   按"气那一族噪声"做一次**拒绝采样**（抽签也用哈希 ⇒ 确定性）⇒
        //   留下来的星在大尺度上跟气同分布。
        //   ⚠ 抽签的哈希与位置/亮度的哈希**各自独立** ⇒ 改锐度/频率不会挪动"哪些位置
        //     被抽到"，只会改"哪些被留下"。
        if params.sky_biased && params.sky_contrast > 0.0 {
            let direction = [position[0] / r, position[1] / r, position[2] / r];
            let chance = sky_density(direction, params).powf(params.sky_contrast);
            if unit24(hash(seed ^ 0x2b9f_41c7, index as u32)) >= chance {
                continue;
            }
        }
        // ⚠⚠ **按表观亮度剔除**（用户 2026-09-25）：直接看见那一档像素值 ∝ `B/r²`
        //   （辐照律，见 `raymarch::star_falloff`）⇒ 远处的暗星根本读不出来。
        //   剔掉它们等于**星等截断**（真实星表就是这么干的），而它的副产品正是
        //   "近密远疏"：`1/r²` 让远处的暗星先掉出去（`r = 3` 处阈值收到 1/9）。
        //   ⚠ 阈值只看**几何 + 亮度**，不看气（用户："先不管介质"）⇒ 星场不必吃密度。
        if value * star_falloff(r, params.inner) < params.min_apparent {
            continue;
        }
        positions.push(position);
        values.push(value);
        tints.push(params.star_tint);
    }

    // 星簇：参考图里那几颗嵌在气里的亮星。它们与"场的星"**是同一种东西**（同一张表、
    // 同一个格）⇒ 既直射进画面、也照亮周围的气；旧 `cloud.emission::cluster_*` 由此退役。
    if cluster_count > 0 {
        let n = cluster_count as f32;
        let golden = 2.399_963_2_f32;
        for index in 0..cluster_count {
            let h = hash(seed ^ 0x2545_f491, index as u32);
            let z = 1.0 - 2.0 * (index as f32 + 0.5) / n;
            let ring = (1.0 - z * z).max(0.0).sqrt();
            let phi = golden * index as f32;
            // 簇内半径：**按体积均匀**（球里均匀，不是壳上均匀）⇒ `u^(1/3)`。
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

    // 盒子按**真实星位**定（星簇可以由参数摆到壳外，那时壳的半径不是上界）。
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

/// **判据用**：一个世界点的邻域里有哪些星（球查询的公开口，与 GPU 那份同一套语义）。
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

/// 邻域里**最亮的 `keep` 颗**（次序：亮度降序；并列保持载荷次序）。
///
/// ⚠ 这条**次序规则必须两侧一致**：CPU 与 GPU 各挑各的"前几名"时，只要并列的处理不同，
///   同一个体素两边的光照就会差一点点 —— 而那是逐位判据下最容易看不出来的分叉。
///   `keep = 0` 表示不封顶（全都留下）。
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
        // ⚠ `sort_by` 是**稳定**排序 ⇒ 亮度并列时保持载荷次序（两侧一致的那条规则）。
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

    /// **确定性**：同参数两遍逐位相同（缓存键靠它）。
    #[test]
    fn the_same_parameters_give_the_same_stars() {
        let one = bake_stars(&params(), None).expect("烘星");
        let two = bake_stars(&params(), None).expect("烘星");
        assert_eq!(one.stars, two.stars);
        assert_eq!(one.grid, two.grid);
    }

    /// **每颗星都落在它自己那一格里**：逐格回查，登记它的那一格必须就是它所在的格。
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

    /// **一颗远处的星能被查到**（最朴素的一条：格、掩码、子 CSR 三段都对上）。
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

    /// **邻域查询与逐颗暴力一致**：`stars_near` 是 CPU/GPU 共用的语义口。
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

    /// **星撒在壳里**（不是撒在一个球面上、也不是撒到盒子角上）：半径分布必须夹在
    /// `[inner, outer]` 里，而且**中位半径落在体积中位**（`((inner³+outer³)/2)^(1/3)`）。
    ///
    /// ⚠ 后半句才是"按体积均匀"的判据：按**半径**均匀的话中位会落在 `(inner+outer)/2`。
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

    /// **球查询的各向同性**：同一个半径的球在任何位置都该有"同样多"的星。
    ///
    /// ⚠ 这一条就是"旧那套为什么不行"的判据：那里星的位置由**方向**格点决定，
    ///   每球面度的密度按 `|dₓ|+|d_y|+|d_z|` 偏（实测面心/面角 1.43x）。
    ///   现在位置在世界坐标里按体积均匀 ⇒ 采样一批球心，星数的相对散布必须小。
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
        // 球心撒在一张壳上（与星的半径分布无关），方向按 Fibonacci 球均匀。
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
        // 泊松散布的相对起伏约 `1/√mean`；给三倍余量。
        let expected = 3.0 / mean.sqrt();
        assert!(
            spread / mean < expected,
            "每球的星数相对起伏 {:.3} 超过泊松上界 {expected:.3} —— 位置不均匀",
            spread / mean
        );
    }
}
