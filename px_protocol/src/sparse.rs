use crate::art::{CUBE_FACES, VolumeData};

pub const COARSE: u32 = 8;
pub const FINE: u32 = 2;
pub const FINE_PER_AXIS: u32 = COARSE / FINE;

fn coarse_axis(res: u32, layers: u32) -> [u32; 3] {
    [
        res.div_ceil(COARSE),
        layers.div_ceil(COARSE),
        res.div_ceil(COARSE),
    ]
}

fn coarse_per_face(res: u32, layers: u32) -> u32 {
    let axis = coarse_axis(res, layers);
    axis[0] * axis[1] * axis[2]
}

fn coarse_index(res: u32, layers: u32, cs: u32, cl: u32, ct: u32) -> u32 {
    let axis = coarse_axis(res, layers);
    (ct * axis[1] + cl) * axis[0] + cs
}

#[inline]
fn fine_block_index(fsb: u32, ftb: u32, flb: u32) -> u32 {
    (flb * FINE_PER_AXIS + ftb) * FINE_PER_AXIS + fsb
}

#[inline]
fn fine_bit(fs: u32, ft: u32, fl: u32) -> u32 {
    (fl * FINE + ft) * FINE + fs
}

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

#[derive(Debug, Clone, PartialEq)]
pub struct SparseVolume {
    pub res: u32,
    pub layers: u32,
    pub inner: f32,
    pub outer: f32,
    face_blocks: Vec<u64>,
    block_order: Vec<u32>,
    slots: Vec<u32>,
    slots_base: Vec<u32>,
    values: Vec<u16>,
    lo: f32,
    hi: f32,
}

impl SparseVolume {
    pub fn from_dense(dense: &VolumeData) -> Self {
        let res = dense.res.max(1);
        let layers = dense.layers.max(1);

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
                            continue;
                        }

                        let global = face as usize * per_face
                            + coarse_index(res, layers, cs, cl, ct) as usize;
                        bit_set(&mut face_blocks, global as u32);
                        block_order[global] = slots_base.len() as u32;
                        slots_base.push(slots.len() as u32);

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
                                        slots.push(u32::MAX);
                                        continue;
                                    }
                                    let start = values.len() as u32;
                                    values.extend_from_slice(&cell_values);
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

    pub fn live_blocks(&self) -> usize {
        self.slots_base.len().saturating_sub(1)
    }

    pub fn live_cells(&self) -> usize {
        self.values.len()
    }

    pub fn cells(&self) -> usize {
        self.res as usize * self.layers as usize * self.res as usize * CUBE_FACES as usize
    }

    pub fn mask_words(&self) -> &[u64] {
        &self.face_blocks
    }

    pub fn range(&self) -> (f32, f32) {
        (self.lo, self.hi)
    }

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
            lanes: 1,
            data,
        }
    }
}

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
            lanes: 1,
            data,
        }
    }

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
