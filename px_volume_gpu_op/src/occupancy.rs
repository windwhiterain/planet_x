//! **占用索引**（层次化空跳的形状）：从发射体积派生的一份**只读**结构，与 `data` 并列上传。
//!
//! ## 它是什么，不是什么
//!
//! * 它**不是**存储格式：体积仍然是密集的六通道 `data`，写盘、缓存键、载荷一律没动。
//!   这一份只是步进核的**加速结构**（像一张索引），丢了可以随时重建。
//! * 它是 `px_protocol::sparse::SparseVolume` 那套**两级占用的索引那一半**：
//!   面 → 粗块 `8³` → 细块 `2³` → 格。位序与那边的 [`fine_bit`] 同源，将来接稀疏值存储时
//!   不用再改一遍。
//! * 派生它**不是**"跑 CPU 实现"：它读的是已经烘好的产物（`cloud.emission` 的六通道体积），
//!   做的是打包上传，不产出任何辐射亮度。渲染的每一步算术仍然只在 WGSL 里。
//!
//! ## 空的判据（本轮口径：**块内精确 0** —— 用户 2026-09-25 明确选它）
//!
//! 一个格算空 ⇔ 它的发射（三通道）与消光（三通道）**六个值全恰好是 0.0**；
//! 一个块算空 ⇔ 它里面一个非空格都没有。这一条是**恒等**的：三个通道精确 0 时三线性
//! 在那一段的贡献也精确 0，跳过它不改变算术结果。
//!
//! ⚠ 保守化的退路是"膨胀一格"（把"支撑域会伸进来的邻块"也算活）。这一轮**没有**用它：
//!   实测的 Δ（见下）落在 quadrature 那一侧，不是"边界上漏了气"。
//!
//! ## 空跳的实测代价（2026-09-25，本轮的数）
//!
//! `the_skip_matches_the_dense_march_on_a_blocky_volume` 把**空跳开/关**在同一份 WGSL、
//! 同一个 `steps` 下逐 texel 比（384 条，夹具是"只有粗块 `(cl = 1, cs/ct = 0...1)` 有值、
//! 其余精确 0"）：最大 Δ = **0.0142**（相对 3.9%），均值 0.0909。
//!
//! ⚠ 这 3.9% **不是**"空块里漏了东西"，而是两档的 quadrature 不同：空跳档在有内容的块里
//!   **逐层中点**取样（`samples = 块内层数`、`step = 该层的径向跨度`），密集档取
//!   `shell_radius((i + 0.5)·du)`。两者是同一族尺子、但不是同一个点集。
//!   真要压到 1e-3，得让空跳档的样本点逐一对齐密集档 —— 见 notes。
//!
//! ## 真实星云体积上的收益（2026-09-25 实测）
//!
//! `px_graphs nebula --face 64`（`art/nebulasky/sky.toml` 的 `steps = 96`，release）：
//!
//! * 掩码形状：`res 64 / layers 64 / 每面 512 块 / 总块 3072 / **活块 1020（33.2%）**`
//!   ⇒ **66.8% 的粗块是精确空**，可以整段跳过。这与 `cloud.density` 那一侧量到的
//!   "硬门之后 57.2% 的格恰好为 0" 对得上（块级空比格级空更严，因为块内只要有**一个**
//!   非空格就不空）。
//! * 时间：**冷启一次 13.0 s**（含密度/发射/星场的烘焙，每面 1024²、steps 96）。
//!   `PX_SKIP_OFF=1` 那一档实测 4.4 s 对开着的 3.8 s（steps 98/97，热缓存），
//!   ⇒ 天穹那一段 **约快 14%**。⚠ 这个差带着噪声（各一次），不是严格的基准。
//!
//! ⚠ 两个只影响**测量**的开关（默认关，不进产物）：
//!   `PX_SKIP_REPORT=1` 打一行掩码形状；`PX_SKIP_OFF=1` 关掉空跳走密集档。
//!   ⚠ CAS 的键**不含**环境变量 ⇒ 想量两档必须让 `steps` 变一下（或清缓存），
//!     否则第二次是直接命中缓存。
//!
//! ## 布局（与 `sampler.wgsl` 的 `occupancy_class` 逐字段对齐）
//!
//! ⚠⚠ 内部存储与**上传布局是两套**，`upload_words` 负责换算；改动一处必须同时改
//!   `at_cell`（按内部布局）与 WGSL 的 `occupancy_class`（按上传布局）。
//!
//! ```text
//! 内部：sidecar[面 × blocks_per_face + block]                    每块一格 u32（值 0/1）
//!       words  [面 × blocks_per_face × WORDS_PER_BLOCK + block × WORDS_PER_BLOCK + sub/32]
//! 上传：words  [L1 位（每块 1 位）][L2 掩码（每块 WORDS_PER_BLOCK 字）]
//! ```
//!
//! * L1 位号 = [`block_index`]（面内 `(ct · blocks_l + cl) · blocks_s + cs`），
//!   **上传时按位号取整字**（`words[块号 / 32]` 的第 `块号 % 32` 位）；
//! * L2 位 = [`sub_index`]（`(sub_l · 4 + sub_t) · 4 + sub_s`），`1` = 这个 `2³`
//!   子块里有非零格。**L1 = 0 的块一个字节都不写**（值恒 0）。
//! * ⚠ `WORDS_PER_BLOCK = 4³/32 = 2`：L2 正好把两个 `u32` 用满 ⇒ L1 **必须独占一段**，
//!   不能塞进"每块的第 0 个字"。

use px_protocol::art::CUBE_FACES;
use px_volume_schema::VolumeData;

/// 粗块边长（格）：与 `px_protocol::sparse::COARSE` 同一个数。
pub const COARSE: u32 = 8;
/// 细块边长（格）：与 `px_protocol::sparse::FINE` 同一个数。
pub const FINE: u32 = 2;
/// 每个粗块的细块槽位数（每轴）。
pub const FINE_PER_AXIS: u32 = COARSE / FINE;
/// 每个粗块在 L2 里占多少个 `u32` 字（`FINE_PER_AXIS³ / 32`）。
pub const WORDS_PER_BLOCK: u32 = FINE_PER_AXIS * FINE_PER_AXIS * FINE_PER_AXIS / 32;

/// 子块在粗块内的槽位号（`径向` 最慢、`s` 最快）—— 与 `px_protocol::sparse::fine_block_index` 同源。
#[inline]
pub fn sub_index(sub_s: u32, sub_t: u32, sub_l: u32) -> u32 {
    (sub_l * FINE_PER_AXIS + sub_t) * FINE_PER_AXIS + sub_s
}

/// 粗块在面内的线性下标（`s` 最快、然后径向、最后 `t`）—— 与 `px_protocol::sparse::coarse_index` 同源。
#[inline]
pub fn block_index(blocks_s: u32, blocks_l: u32, cs: u32, cl: u32, ct: u32) -> u32 {
    (ct * blocks_l + cl) * blocks_s + cs
}

/// 每面的粗块数在三个轴上的个数（各自向上取整）。
#[inline]
pub fn block_axes(res: u32, layers: u32) -> [u32; 3] {
    [
        res.div_ceil(COARSE),
        layers.div_ceil(COARSE),
        res.div_ceil(COARSE),
    ]
}

/// 每面的粗块总数。
#[inline]
pub fn blocks_per_face(res: u32, layers: u32) -> u32 {
    let axis = block_axes(res, layers);
    axis[0] * axis[1] * axis[2]
}

/// **占用索引**：与体积同一张网格、同一套坐标，只记"哪里有内容"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupancy {
    pub res: u32,
    pub layers: u32,
    /// 面内粗块数（s / 径向 / t 三个轴）。
    pub blocks: [u32; 3],
    /// 每面有多少个粗块（`blocks` 三者之积）。
    pub blocks_per_face: u32,
    /// 存活位（L1），长度 = `面 × blocks_per_face`，即 `sidecar`。
    pub sidecar: Vec<u32>,
    /// 子块掩码（L2），长度 = `面 × blocks_per_face × WORDS_PER_BLOCK`。
    pub words: Vec<u32>,
}

/// 世界方向 → 体积网格的 `(面, u, v)`：`px_protocol::art::cube_direction` 的**逆**。
///
/// ⚠⚠ 与 WGSL 的 `grid_coords_of` **逐字对齐**。**不要**换成 `px_protocol::art::cube_face_of`：
///   那是另一套面序/轴向约定（同一个方向给出另一个面号），拿它查按网格面序建的掩码
///   会读到别的面 ⇒ 真空判定整片落空（实测踩过：skip 侧全黑，而掩码本身完全正确）。
pub fn grid_coords_of(direction: [f32; 3]) -> (u32, f32, f32) {
    let (x, y, z) = (direction[0], direction[1], direction[2]);
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, major, a, b) = if ax >= ay && ax >= az {
        if x > 0.0 {
            (0, ax, -z, -y)
        } else {
            (1, ax, z, -y)
        }
    } else if ay >= az {
        if y > 0.0 {
            (2, ay, x, z)
        } else {
            (3, ay, x, -z)
        }
    } else if z > 0.0 {
        (4, az, x, -y)
    } else {
        (5, az, -x, -y)
    };
    let major = major.max(f32::MIN_POSITIVE);
    (
        face,
        ((a / major) * 0.5 + 0.5).clamp(0.0, 1.0),
        ((b / major) * 0.5 + 0.5).clamp(0.0, 1.0),
    )
}

/// 一个格的密集下标（与 `VolumeData::data` 的体素序**逐字相同**，不含通道步长）。
///
/// ⚠⚠ 角向用**格心**口径（`s·res`：`0..res-1` 个格心均匀铺在 `[0,1]` 上），径向用
///   `altitude·(layers-1)`。掩码的建与查必须同一套取整；这一份与 WGSL 的
///   `occupancy_class` **逐字对齐**，改一处必须改另一处。
///
/// ⚠ `s`/`t` 必须是**体积网格自己的面内参数**（`px_protocol::art::cube_direction` 那一套，
///   也就是 `px_volume_schema::direction_of`）。用 `cube_face_of` 反查会给另一套面序
///   ⇒ 读到别的面 ⇒ 真空判定整片落空（实测踩过，症状是 skip 侧全黑而掩码本身是对的）。
#[inline]
#[allow(clippy::too_many_arguments)]
fn voxel_index(
    res: u32,
    layers: u32,
    s: f32,
    t: f32,
    radius: f32,
    inner: f32,
    outer: f32,
) -> [u32; 3] {
    let cs = ((s * res as f32) as u32).min(res - 1);
    let ct = ((t * res as f32) as u32).min(res - 1);
    let altitude = if inner > 0.0 {
        let ratio = (outer / inner).ln();
        if ratio.abs() <= 1e-30 {
            0.0
        } else {
            (radius.max(1e-30).ln() - inner.ln()) / ratio
        }
    } else {
        let span = outer - inner;
        if span.abs() <= 1e-30 {
            0.0
        } else {
            (radius - inner) / span
        }
    };
    let cl = (altitude.clamp(0.0, 1.0) * (layers - 1) as f32) as u32;
    [cs, cl.min(layers - 1), ct]
}

/// 一个格的六个通道是否**全恰好为 0**（`data` 是交错的六通道）。
#[inline]
fn voxel_is_empty(data: &[f32], voxel: usize, lanes: usize) -> bool {
    let base = voxel * lanes;
    for lane in 0..6.min(lanes) {
        if data[base + lane] != 0.0 {
            return false;
        }
    }
    // lanes < 6 时（理论上不会发生）按"只看读得到的那些通道"算 —— 不静默当成空。
    true
}

impl Occupancy {
    /// **从发射体积建**（六通道交错；`lanes` 不是 6 时按读得到的那几条通道判）。
    ///
    /// ⚠ 这是唯一的构造口：占用判据只在这里实现一次（与 `SparseVolume::from_dense` 同一个口径）。
    pub fn from_emission(volume: &VolumeData) -> Self {
        let res = volume.res.max(1);
        let layers = volume.layers.max(1);
        let lanes = volume.lanes();
        let blocks = block_axes(res, layers);
        let per_face = blocks_per_face(res, layers);
        let mut sidecar = vec![0_u32; CUBE_FACES as usize * per_face as usize];
        let mut words =
            vec![0_u32; CUBE_FACES as usize * per_face as usize * WORDS_PER_BLOCK as usize];

        for face in 0..CUBE_FACES {
            for cl in 0..blocks[1] {
                for cs in 0..blocks[0] {
                    for ct in 0..blocks[2] {
                        let packed = face as usize * per_face as usize
                            + block_index(blocks[0], blocks[1], cs, cl, ct) as usize;
                        // 先扫一遍这一块：哪些 2³ 子块里有非零格。
                        let mut masks = [0_u32; WORDS_PER_BLOCK as usize];
                        let l_hi = (cl * COARSE + COARSE).min(layers);
                        let s_hi = (cs * COARSE + COARSE).min(res);
                        let t_hi = (ct * COARSE + COARSE).min(res);
                        for l in cl * COARSE..l_hi {
                            for t in ct * COARSE..t_hi {
                                for s in cs * COARSE..s_hi {
                                    let voxel =
                                        (((face * layers + l) * res + t) * res + s) as usize;
                                    if voxel_is_empty(&volume.data, voxel, lanes) {
                                        continue;
                                    }
                                    let sub = sub_index(
                                        (s % COARSE) / FINE,
                                        (t % COARSE) / FINE,
                                        (l % COARSE) / FINE,
                                    );
                                    masks[(sub / 32) as usize] |= 1_u32 << (sub % 32);
                                }
                            }
                        }
                        if masks.iter().all(|word| *word == 0) {
                            continue; // 空块：L1 与 L2 都一个字节不写（规范）。
                        }
                        sidecar[packed] = 1;
                        let base = packed * WORDS_PER_BLOCK as usize;
                        words[base..base + WORDS_PER_BLOCK as usize].copy_from_slice(&masks);
                    }
                }
            }
        }

        Self {
            res,
            layers,
            blocks,
            blocks_per_face: per_face,
            sidecar,
            words,
        }
    }

    /// 每面的粗块数（与 [`Self::blocks_per_face`] 同一个数，判据用）。
    pub fn blocks_per_face(&self) -> u32 {
        self.blocks_per_face
    }

    /// **从上传用的扁平缓冲建**（`data[((face·layers + layer)·res + t)·res + s)·lanes + lane]`）。
    ///
    /// ⚠ 它与 [`Self::from_emission`] 的差别**只有承载类型**：同一个判据、同一套下标。
    ///   两条路各写一遍占用判据就会漂开，所以这里只做一次转换、不重写扫描。
    pub fn from_flat(res: u32, layers: u32, lanes: usize, data: &[f32]) -> Self {
        let res = res.max(1);
        let lanes = lanes.max(1);
        let layers = layers.max(1);
        let samples = res as usize * layers as usize * res as usize * CUBE_FACES as usize;
        let usable = data.len() / lanes;
        let filled = (samples.min(usable)) * lanes;
        let mut padded = data[..filled].to_vec();
        padded.resize(samples * lanes, 0.0);
        Self::from_emission(&VolumeData {
            res,
            layers,
            // 半径在这里**无关**：占用只看格值与网格形状（差值是 0，不参与判空）。
            inner: 0.0,
            outer: 0.0,
            lanes: lanes as u32,
            data: padded,
        })
    }

    /// L1 侧车（判据仪器用）。
    pub fn sidecar(&self) -> &[u32] {
        &self.sidecar
    }

    /// L2 掩码（判据仪器用）。
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    /// **按世界点查占用**：`0` = 空块（跳过）、`1` = 粗块活了但子块空、`2` = 有内容的细块。
    ///
    /// ⚠ 世界方向 → 面内参数走的是[体积网格那一套](grid_coords_of)（`cube_direction` 的逆），
    ///   **不是** `cube_face_of`（另一套面序，见 [`Self::at_cell`]）。
    pub fn at(&self, point: [f32; 3], inner: f32, outer: f32) -> u32 {
        let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
        let (face, s, t) = grid_coords_of([
            point[0] / radius.max(1e-30),
            point[1] / radius.max(1e-30),
            point[2] / radius.max(1e-30),
        ]);
        self.at_cell(face, s, t, radius, inner, outer)
    }

    /// **按面内坐标查占用**（掩码的真源就是这套坐标：面、`s`、`t`、层）。
    ///
    /// ⚠ 这是判据该用的口子：方向 → 面内坐标是**体积网格自己的** `direction_of`
    ///   （`px_volume_schema::direction_of`），不是 `cube_face_of`（两套约定不同，见 [`Self::at`]）。
    pub fn at_cell(&self, face: u32, s: f32, t: f32, radius: f32, inner: f32, outer: f32) -> u32 {
        let [cs, cl, ct] = voxel_index(self.res, self.layers, s, t, radius, inner, outer);
        let (bs, bl, bt) = (cs / COARSE, cl / COARSE, ct / COARSE);
        let packed = face as usize * self.blocks_per_face as usize
            + block_index(self.blocks[0], self.blocks[1], bs, bl, bt) as usize;
        if self.sidecar.get(packed).copied().unwrap_or(0) == 0 {
            return 0;
        }
        let sub = sub_index(
            (cs % COARSE) / FINE,
            (ct % COARSE) / FINE,
            (cl % COARSE) / FINE,
        );
        // ⚠ 每块的 L2 掩码从 `packed · WORDS_PER_BLOCK` 起（`WORDS_PER_BLOCK` = 64 位 / 32 = 2）。
        let word = self.words[packed * WORDS_PER_BLOCK as usize + (sub / 32) as usize];
        if word & (1_u32 << (sub % 32)) == 0 {
            return 1;
        }
        2
    }

    /// **L1 区的字数**：`ceil(块数 / 32)` **再向上取整到 4 的倍数**。
    ///
    /// ⚠⚠ 必须凑到 4 的倍数：`u32` 数组的下标单位是 4 字节，而"第 `i` 个字里的 32 个块"
    ///   这套位号要求每个字都落在 4 字节边界上。只按 `ceil(块数/32)` 取（72 块 ⇒ 3 个字）
    ///   会让第 3 个字被折进第 2 个字（72 = 2·32 + 8）⇒ **块号 64 往后的 L1 位读到 L2 区**
    ///   （实测：块 2 的位读到 `words[2]`，那里是别的块的掩码 ⇒ 恒判空、skip 侧全 0）。
    pub fn l1_words(&self) -> usize {
        self.sidecar.len().div_ceil(32).div_ceil(4) * 4
    }

    /// **上传给 GPU 的一份连续缓冲**：`[L1 位（每块 1 位）][L2 掩码（每块 `WORDS_PER_BLOCK` 字）]`。
    ///
    /// ⚠⚠ **不要**直接把 [`Self::words`] 传上去：那是纯 L2 段，而着色器要按"每块一个字"先查 L1
    ///   ⇒ 两段的基准对不上。这一份与 WGSL 的 `occupancy_class` 是**一对**：L1 用位号 `packed`
    ///   取，L2 用 `l1_words + packed·WORDS_PER_BLOCK + sub/32` 取。
    ///   ⚠ `WORDS_PER_BLOCK` = `4³/32` = 2 时，L2 段正好把两个字节用满 ⇒ L1 必须独占一段。
    pub fn upload_words(&self) -> Vec<u32> {
        let l1_words = self.l1_words();
        let mut packed_words = vec![0_u32; l1_words + self.words.len()];
        // ⚠⚠ `sidecar` 是**一格一字**（值是 0/1），`upload` 要的是**一格一位**：
        //   第 `i` 块的位 = `words[i / 32]` 的第 `i % 32` 位。
        //   写成 `index * 32 + bit` 会整体乘 32 ⇒ 只剩最后 1/32 能落进 L1 区（实测）。
        for (index, word) in self.sidecar.iter().enumerate() {
            if *word == 0 {
                continue;
            }
            packed_words[index / 32] |= 1 << (index % 32);
        }
        packed_words[l1_words..].copy_from_slice(&self.words);
        packed_words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份**成块**的假体积：粗块 `(cs = 0...1, cl = 1...2, ct = 0...1)` 里有值，其余精确 0。
    ///
    /// ⚠ 夹具要按**块**给（不是按"某个高度带"）：`8³` 的块与任意高度带都只部分相交，
    ///   于是每一个块都会被标成活 —— 那样"占用明显少于一半"就永远不成立，
    ///   判据会变成一条假红（第一版正是这么写的）。空块必须**整块**空。
    fn blocked(res: u32, layers: u32) -> VolumeData {
        let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res * 6) as usize];
        let axis = block_axes(res, layers);
        for face in 0..CUBE_FACES {
            for cl in 1..axis[1].saturating_sub(1).max(1) {
                for t in 0..res {
                    for s in 0..res {
                        if s / COARSE != 0 || t / COARSE != 0 {
                            continue;
                        }
                        let voxel = (((face * layers + cl * COARSE) * res + t) * res + s) as usize;
                        for lane in 0..6 {
                            data[voxel * 6 + lane] = 0.5;
                        }
                    }
                }
            }
        }
        VolumeData {
            res,
            layers,
            inner: 1.0,
            outer: 3.0,
            lanes: 6,
            data,
        }
    }

    /// **空块一个都不标**：存活粗块数正好等于"里面有非零格的块数"。
    ///
    /// ⚠ 这条钉的是"索引不漏也不多"：多标一格不会出错（只是白跑），少标一格就是
    ///   画面上一截凭空没有的气 —— 两边的症状完全不同，所以要分开量。
    #[test]
    fn only_blocks_with_content_are_marked() {
        let (res, layers) = (16_u32, 24_u32);
        let volume = blocked(res, layers);
        let occupancy = Occupancy::from_emission(&volume);
        let axis = block_axes(res, layers);
        let mut expected = 0_usize;
        for face in 0..CUBE_FACES {
            for ct in 0..axis[2] {
                for cl in 0..axis[1] {
                    for cs in 0..axis[0] {
                        let mut live = false;
                        for l in cl * COARSE..(cl * COARSE + COARSE).min(layers) {
                            for t in ct * COARSE..(ct * COARSE + COARSE).min(res) {
                                for s in cs * COARSE..(cs * COARSE + COARSE).min(res) {
                                    let voxel =
                                        (((face * layers + l) * res + t) * res + s) as usize;
                                    if !voxel_is_empty(&volume.data, voxel, 6) {
                                        live = true;
                                    }
                                }
                            }
                        }
                        if live {
                            expected += 1;
                        }
                    }
                }
            }
        }
        let marked = occupancy.sidecar.iter().filter(|bit| **bit != 0).count();
        assert_eq!(marked, expected, "存活粗块数应当正好等于有内容的块数");
        assert!(
            expected < occupancy.sidecar.len() / 2,
            "壳的占用应当明显少于一半（实际 {expected} / {}）",
            occupancy.sidecar.len()
        );
    }

    /// **谓词与格值一致**：随机撒点，`at` 说"有内容"的点，六通道不能全零。
    ///
    /// ⚠ 这是"掩码与采样核同源"的判据：取整规则一旦与 WGSL 差一格，空段边界就会
    ///   落在错误的格上 —— 而症状只是"气多/少了一小截"，靠看图归因不到取整。
    #[test]
    fn a_marked_cell_is_never_all_zero() {
        let (res, layers) = (16_u32, 24_u32);
        let volume = blocked(res, layers);
        let occupancy = Occupancy::from_emission(&volume);
        let mut marked = 0_usize;
        for index in 0..4096_usize {
            // 一份确定性的伪随机方向 + 半径（不引 rand：判据要逐次可复现）。
            let hash = (index as u32).wrapping_mul(0x9e37_79b9);
            let u = (hash & 0xffff) as f32 / 65535.0;
            let v = ((hash >> 16) & 0xffff) as f32 / 65535.0;
            let s = u;
            let t = v;
            let radius = 1.0 + 2.0 * (((hash >> 8) & 0xff) as f32 / 255.0);
            let face = hash % CUBE_FACES;
            let direction = px_protocol::art::cube_direction(face, s, t);
            let point = [
                direction[0] * radius,
                direction[1] * radius,
                direction[2] * radius,
            ];
            if occupancy.at(point, volume.inner, volume.outer) == 2 {
                marked += 1;
                assert!(
                    radius >= volume.inner && radius <= volume.outer,
                    "标了内容的点却在壳外：{radius}"
                );
            }
        }
        assert!(marked > 0, "一个点都没标上内容 —— 判据没有在测东西");
    }
}
