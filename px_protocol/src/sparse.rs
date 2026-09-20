//! **稀疏体网格**：多级占用 + 紧凑二进制载荷。
//!
//! ## 为什么要有它
//!
//! 密集表示是 `res² × layers × 6` 个 `f32`。实测（`c17d5ac`）：`--shape 128` 时
//! 单张场 6.29M 行、CAS 里一个产物写 **3.2 GB**、缓存涨到 **29.1 GB**（机器 15.6 GB）
//! ⇒ 换页、烘不完。而局域对比（8×8 std）与参考图差 **3~7 倍**，四组参数对照都推不动
//! ⇒ **瓶颈是网格分辨率**。星云是一层**壳**（占用约 15~25%）⇒ 稀疏能换 2~3 倍线性分辨率。
//!
//! ## 布局：两级占用
//!
//! ```text
//! 面（6）→ 粗块 8×8×8 → 细块 2×2×2 → 格
//! ```
//!
//! * **粗块层**：每面一张位集（`coarse_per_face` 位），位 = `coarse_index(cs, cl, ct)`；
//!   另有一张 `block_order` 把"粗块序号 → 存活序号"直接查出来（`u32::MAX` = 不存在）。
//! * **细块层**：每个存活粗块固定 `FINE_PER_AXIS³`（= 64）个槽位。
//!   一项 = `(值起点 << 16) | 占用掩码`，`u32::MAX` = 这一槽空着。
//! * **值**：按槽位序紧凑排的 `u16`。
//!
//! ## ⚠ 为什么细块层要"固定槽位"而不是"紧凑列表"
//!
//! 第一版用"紧凑细块表 + 两张起点表"，读者要**反推**"第几个存活的细块"。
//! 那种反推的序一旦与写入差一格，值就整片错位 —— 而**"存活格总数"仍然完全正确**
//! （数据没丢，只是位置错），于是判据会显示"哪一层不对"却指不出是哪一处。
//! 固定槽位把"哪一格在哪"变成**直接查**，读写共用同一个 [`fine_block_index`]，
//! 于是"序不一致"这个错误类别**从设计上消失**（代价是每块 64 项的槽位表，
//! 相对省下来的那份密数组可以忽略）。
//!
//! ## ⚠ 编码必须**唯一**（本仓的地基是「键 = 完整产物字节」）
//!
//! 同一份内容若有两种合法编码，缓存就**永不命中**，而症状只是"每次都重烘"
//! （不报错、不崩溃）。所以占用有唯一判据：
//!
//! * **一个细块存在 ⇔ 它里面至少有一格非零**；
//! * **一个粗块存在 ⇔ 它里面至少有一个细块**。
//!
//! ⇒ **没有"显式的 0"**。这不是实现细节，是格式的一部分。
//!
//! ## 数值：`u16` + 每份体积一对 `[lo, hi]`
//!
//! ⚠ 用**线性量化**而不是 `f16`：星云的值基本在 `0..1`，`f16` 的指数位全浪费在量级上、
//! 且**在 0 附近最不准**（而壳里绝大多数格恰恰在 0 附近）。线性 `u16` 在 `[lo, hi]` 上
//! 是均匀精度（1/65535），且比 `f32` 省一半。
//! ⚠ `[lo, hi]` 只由**非零格**的极值给：把 0 算进去会让区间虚胖、有效分辨率白掉一半。

use crate::art::{CUBE_FACES, VolumeData};

/// 粗块边长（格）。
pub const COARSE: u32 = 8;
/// 细块边长（格）。
pub const FINE: u32 = 2;
/// 每个粗块的细块槽位数。
pub const FINE_PER_AXIS: u32 = COARSE / FINE;

/// 每面粗块在各轴上的个数（向上取整）。
fn coarse_axis(res: u32, layers: u32) -> [u32; 3] {
    [
        res.div_ceil(COARSE),
        layers.div_ceil(COARSE),
        res.div_ceil(COARSE),
    ]
}

/// 一个面的粗块总数。
fn coarse_per_face(res: u32, layers: u32) -> u32 {
    let axis = coarse_axis(res, layers);
    axis[0] * axis[1] * axis[2]
}

/// 粗块在面内的线性下标（`s` 最快、然后径向、最后 `t`）。
fn coarse_index(res: u32, layers: u32, cs: u32, cl: u32, ct: u32) -> u32 {
    let axis = coarse_axis(res, layers);
    (ct * axis[1] + cl) * axis[0] + cs
}

/// 细块在粗块内的槽位号（`径向` 最慢、`s` 最快）。
///
/// ⚠ **读写必须共用这一个函数**。第一版两处各写一遍，而调用点照变量名顺手传参，
/// 把 `t` 与径向调换了 ⇒ 细块下标偏了好几格（实测：层 0 读出层 2 的值）。
/// 凡是有两个同类型参数的函数，调用点都该按**名字**对齐，不该靠位置。
#[inline]
fn fine_block_index(fsb: u32, ftb: u32, flb: u32) -> u32 {
    (flb * FINE_PER_AXIS + ftb) * FINE_PER_AXIS + fsb
}

/// 细块内某一格的位（`u16` 的低 8 位里的第几位）。
///
/// ⚠ 位序与 [`fine_block_index`] **同源**（`fl` 最慢、`fs` 最快）：
///   细块内坐标各自落在 `0..FINE` ⇒ 系数是 `FINE` 的幂，与块多大无关。
#[inline]
fn fine_bit(fs: u32, ft: u32, fl: u32) -> u32 {
    (fl * FINE + ft) * FINE + fs
}

/// 一个粗块内部某一层的占用掩码：`u64` 的一个字（位 = `ft * COARSE + fs`）。
#[inline]
fn coarse_word(ft: u32, fs: u32) -> u64 {
    1_u64 << (ft * COARSE + fs)
}

#[inline]
fn bit_get(words: &[u64], index: u32) -> bool {
    match words.get((index / 64) as usize) {
        Some(word) => word & (1_u64 << (index % 64)) != 0,
        None => false,
    }
}

#[inline]
fn bit_set(words: &mut [u64], index: u32) {
    if let Some(word) = words.get_mut((index / 64) as usize) {
        *word |= 1_u64 << (index % 64);
    }
}

/// **稀疏体网格**：与 [`VolumeData`] 同一张网格、同一套坐标，只换了存储。
#[derive(Debug, Clone, PartialEq)]
pub struct SparseVolume {
    pub res: u32,
    pub layers: u32,
    pub inner: f32,
    pub outer: f32,
    /// 每面的粗块占用位集（`coarse_per_face × 6` 位）。
    ///
    /// ⚠ 位宽**不能假定是 64**：`res = 64, layers = 64` 时每面就有 512 个粗块。
    ///   第一版写成 `[u64; 6]`，于是 `1 << 64` 直接 panic。
    face_blocks: Vec<u64>,
    /// 粗块序号 → 存活序号（`u32::MAX` = 不存在）。
    block_order: Vec<u32>,
    /// 细块槽位表：每个存活粗块固定 [`FINE_PER_AXIS`]³ 项，`(值起点 << 16) | 掩码`。
    slots: Vec<u32>,
    /// 每个存活粗块的槽位起点（末尾一个哨兵）。
    slots_base: Vec<u32>,
    /// 量化后的值（按槽位序紧凑排）。
    values: Vec<u16>,
    /// 量化区间（只由**非零格**的极值给；全空时两者都是 0）。
    lo: f32,
    hi: f32,
}

impl SparseVolume {
    /// **从密集的一份建**（唯一的构造口：占用判据只在这里实现一次）。
    pub fn from_dense(dense: &VolumeData) -> Self {
        let res = dense.res.max(1);
        let layers = dense.layers.max(1);

        // 量化区间只算命中的格。
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for value in &dense.data {
            if *value != 0.0 && value.is_finite() {
                lo = lo.min(*value);
                hi = hi.max(*value);
            }
        }
        if !lo.is_finite() || !hi.is_finite() {
            lo = 0.0;
            hi = 0.0;
        }

        let caxis = coarse_axis(res, layers);
        let per_face = coarse_per_face(res, layers) as usize;
        let mut face_blocks = vec![0_u64; (per_face * CUBE_FACES as usize).div_ceil(64)];
        let mut block_order = vec![u32::MAX; per_face * CUBE_FACES as usize];
        let mut slots_base = Vec::new();
        let mut slots: Vec<u32> = Vec::new();
        let mut values: Vec<u16> = Vec::new();

        for face in 0..CUBE_FACES {
            for ct in 0..caxis[2] {
                for cl in 0..caxis[1] {
                    for cs in 0..caxis[0] {
                        // 这个粗块里有哪些格非零？（每层一个字）
                        let mut words = [0_u64; COARSE as usize];
                        for fl in 0..COARSE {
                            let l = cl * COARSE + fl;
                            if l >= layers {
                                break;
                            }
                            for ft in 0..COARSE {
                                let t = ct * COARSE + ft;
                                if t >= res {
                                    break;
                                }
                                for fs in 0..COARSE {
                                    let s = cs * COARSE + fs;
                                    if s >= res {
                                        break;
                                    }
                                    let value = dense.at(face, l, t, s);
                                    if value != 0.0 && value.is_finite() {
                                        words[fl as usize] |= coarse_word(ft, fs);
                                    }
                                }
                            }
                        }
                        if words.iter().all(|w| *w == 0) {
                            continue; // ⚠ 空的粗块：一个字节都不写（规范）。
                        }

                        let global = face as usize * per_face
                            + coarse_index(res, layers, cs, cl, ct) as usize;
                        bit_set(&mut face_blocks, global as u32);
                        block_order[global] = slots_base.len() as u32;
                        slots_base.push(slots.len() as u32);

                        // 逐槽扫：槽里的 8 格由 `words` 直接查（`fl` 最慢、`fs` 最快）。
                        for flb in 0..FINE_PER_AXIS {
                            for ftb in 0..FINE_PER_AXIS {
                                for fsb in 0..FINE_PER_AXIS {
                                    let mut mask = 0_u16;
                                    let mut cell_values = Vec::new();
                                    for fl in 0..FINE {
                                        for ft in 0..FINE {
                                            for fs in 0..FINE {
                                                let fsx = fsb * FINE + fs;
                                                let ftx = ftb * FINE + ft;
                                                let flx = flb * FINE + fl;
                                                if words[flx as usize] & coarse_word(ftx, fsx) == 0
                                                {
                                                    continue;
                                                }
                                                let l = cl * COARSE + flx;
                                                let t = ct * COARSE + ftx;
                                                let s = cs * COARSE + fsx;
                                                mask |= 1_u16 << fine_bit(fs, ft, fl);
                                                cell_values.push(quantize(
                                                    dense.at(face, l, t, s),
                                                    lo,
                                                    hi,
                                                ));
                                            }
                                        }
                                    }
                                    if mask == 0 {
                                        // ⚠ 空的细块：槽位写哨兵（规范：不存在的细块只有一种写法）。
                                        slots.push(u32::MAX);
                                        continue;
                                    }
                                    let start = values.len() as u32;
                                    values.extend_from_slice(&cell_values);
                                    // 值起点与掩码打进一项（高位起点、低 16 位掩码）。
                                    slots.push((start << 16) | mask as u32);
                                }
                            }
                        }
                    }
                }
            }
        }
        slots_base.push(slots.len() as u32);

        Self {
            res,
            layers,
            inner: dense.inner,
            outer: dense.outer,
            face_blocks,
            block_order,
            slots,
            slots_base,
            values,
            lo,
            hi,
        }
    }

    /// 逐格取值（越界 / 不存在 ⇒ `0.0`）。
    ///
    /// ⚠ 每次都要定位两级块（**慢的那条路**）。热路径该用块级采样；
    ///   今天先用它把**正确性**钉住 —— 错了会立刻在某两格上露出来。
    pub fn at(&self, face: u32, layer: u32, t: u32, s: u32) -> f32 {
        if face >= CUBE_FACES || s >= self.res || t >= self.res || layer >= self.layers {
            return 0.0;
        }
        let (cs, cl, ct) = (s / COARSE, layer / COARSE, t / COARSE);
        let Some(order) = self.coarse_order(face, cs, cl, ct) else {
            return 0.0;
        };
        let fsb = (s % COARSE) / FINE;
        let ftb = (t % COARSE) / FINE;
        let flb = (layer % COARSE) / FINE;
        let slot = fine_block_index(fsb, ftb, flb) as usize;
        let bit = fine_bit(s % FINE, t % FINE, layer % FINE);

        let base = self.slots_base[order as usize] as usize;
        let packed = match self.slots.get(base + slot) {
            Some(packed) => *packed,
            None => return 0.0,
        };
        if packed == u32::MAX {
            return 0.0;
        }
        let mask = (packed & 0xffff) as u16;
        if mask & (1_u16 << bit) == 0 {
            return 0.0;
        }
        let value_index =
            (packed >> 16) as usize + (mask & ((1_u16 << bit) - 1)).count_ones() as usize;
        dequantize(self.values[value_index], self.lo, self.hi)
    }

    /// 粗块 → 存活序号（不存在 ⇒ `None`）。
    #[inline]
    fn coarse_order(&self, face: u32, cs: u32, cl: u32, ct: u32) -> Option<u32> {
        let per_face = coarse_per_face(self.res, self.layers);
        let global = face as usize * per_face as usize
            + coarse_index(self.res, self.layers, cs, cl, ct) as usize;
        match self.block_order.get(global) {
            Some(order) if *order != u32::MAX => Some(*order),
            _ => None,
        }
    }

    /// 存活粗块数。
    pub fn live_blocks(&self) -> usize {
        self.slots_base.len().saturating_sub(1)
    }

    /// 存活格数（= 非零格的个数）。
    pub fn live_cells(&self) -> usize {
        self.values.len()
    }

    /// 密集表示下会占多少格（算占用率用）。
    pub fn cells(&self) -> usize {
        self.res as usize * self.layers as usize * self.res as usize * CUBE_FACES as usize
    }

    /// 粗块占用位集（判据仪器用）。
    pub fn mask_words(&self) -> &[u64] {
        &self.face_blocks
    }

    /// 量化区间。
    pub fn range(&self) -> (f32, f32) {
        (self.lo, self.hi)
    }

    /// 回到密集表示（判据与互操作用）。
    pub fn to_dense(&self) -> VolumeData {
        let mut data = vec![0.0_f32; self.cells()];
        for face in 0..CUBE_FACES {
            for layer in 0..self.layers {
                for t in 0..self.res {
                    for s in 0..self.res {
                        let value = self.at(face, layer, t, s);
                        if value != 0.0 {
                            let slot = (((face * self.layers + layer) * self.res + t) * self.res
                                + s) as usize;
                            data[slot] = value;
                        }
                    }
                }
            }
        }
        VolumeData {
            res: self.res,
            layers: self.layers,
            inner: self.inner,
            outer: self.outer,
            data,
        }
    }
}

/// `[lo, hi]` 上的线性量化（`lo == hi` ⇒ 一律 0，解码回 `lo`）。
#[inline]
fn quantize(value: f32, lo: f32, hi: f32) -> u16 {
    let span = hi - lo;
    if !(span > 0.0) {
        return 0;
    }
    let t = ((value - lo) / span).clamp(0.0, 1.0);
    (t * 65535.0 + 0.5) as u16
}

#[inline]
fn dequantize(code: u16, lo: f32, hi: f32) -> f32 {
    let span = hi - lo;
    if !(span > 0.0) {
        return lo;
    }
    lo + (code as f32 / 65535.0) * span
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份有结构的假体积：只有"半径比例在 `0.3~0.7`"的那一层有值（模拟壳）。
    fn shell(res: u32, layers: u32) -> VolumeData {
        let mut data = vec![0.0_f32; (CUBE_FACES * layers * res * res) as usize];
        for face in 0..CUBE_FACES {
            for layer in 0..layers {
                let altitude = layer as f32 / (layers - 1).max(1) as f32;
                if !(0.3..=0.7).contains(&altitude) {
                    continue;
                }
                for t in 0..res {
                    for s in 0..res {
                        data[(((face * layers + layer) * res + t) * res + s) as usize] = altitude;
                    }
                }
            }
        }
        VolumeData {
            res,
            layers,
            inner: 1.0,
            outer: 2.0,
            data,
        }
    }

    /// **逐格往返**：搬到稀疏、再搬回来，每一格都对得上（量化误差之内）。
    ///
    /// ⚠ 这条是这一档的核心判据。**必须逐格比**，不能只看"总数对不对" ——
    ///   位置错而数量对，是这一档最容易犯也最难归因的一类错。
    #[test]
    fn a_dense_volume_survives_the_sparse_round_trip() {
        for (res, layers) in [(4_u32, 4_u32), (16, 16), (8, 20), (20, 8)] {
            let dense = shell(res, layers);
            let sparse = SparseVolume::from_dense(&dense);
            let (lo, hi) = sparse.range();
            let tolerance = (hi - lo) / 65535.0 * 0.5 + 1e-6;
            let mut bad = Vec::new();
            for face in 0..CUBE_FACES {
                for layer in 0..layers {
                    for t in 0..res {
                        for s in 0..res {
                            let (want, got) =
                                (dense.at(face, layer, t, s), sparse.at(face, layer, t, s));
                            if (got - want).abs() > tolerance && bad.len() < 8 {
                                bad.push(format!(
                                    "{res}×{layers} 面{face} 层{layer} t{t} s{s}: 密集 {want} → 稀疏 {got}"
                                ));
                            }
                        }
                    }
                }
            }
            assert!(bad.is_empty(), "逐格对不上：\n  {}", bad.join("\n  "));
        }
    }

    /// **零格一个都不存**，而且**同一个值只会出现一次**（占用是规范的）。
    #[test]
    fn empty_cells_are_not_stored() {
        let dense = shell(16, 16);
        let sparse = SparseVolume::from_dense(&dense);
        let non_zero = dense.data.iter().filter(|v| **v != 0.0).count();
        assert_eq!(sparse.live_cells(), non_zero, "存活格数应当正好是非零格数");
        assert!(
            sparse.live_cells() < dense.data.len() / 2,
            "壳的占用应当明显少于一半（实际 {} / {}）",
            sparse.live_cells(),
            dense.data.len()
        );
    }

    /// **同一份内容只有一种编码**（建两次、以及来回一趟之后再建，结构必须一致）。
    ///
    /// ⚠ 这条钉的是本仓最要紧那条口径 ——「键 = 完整产物字节」。
    #[test]
    fn the_encoding_is_canonical() {
        let dense = shell(16, 16);
        let once = SparseVolume::from_dense(&dense);
        let twice = SparseVolume::from_dense(&dense);
        assert_eq!(once, twice, "同一份输入必须得到逐字段相同的结构");
        let again = SparseVolume::from_dense(&once.to_dense());
        assert_eq!(once.live_blocks(), again.live_blocks());
        assert_eq!(once.live_cells(), again.live_cells());
        assert_eq!(once.slots, again.slots, "槽位表也必须一致");
    }

    /// 越界与空块一律回 0。
    #[test]
    fn outside_or_empty_reads_zero() {
        let sparse = SparseVolume::from_dense(&shell(8, 8));
        assert_eq!(sparse.at(0, 0, 0, 0), 0.0, "壳外的层应当是 0");
        assert_eq!(sparse.at(0, 99, 0, 0), 0.0);
        assert_eq!(sparse.at(0, 0, 99, 0), 0.0);
        assert_eq!(sparse.at(0, 0, 0, 99), 0.0);
        assert_eq!(sparse.at(99, 0, 0, 0), 0.0);
    }
}
