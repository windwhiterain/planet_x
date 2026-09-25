use px_protocol::art::CUBE_FACES;
use px_volume_schema::VolumeData;

pub const COARSE: u32 = 8;
pub const FINE: u32 = 2;
pub const FINE_PER_AXIS: u32 = COARSE / FINE;
pub const WORDS_PER_BLOCK: u32 = FINE_PER_AXIS * FINE_PER_AXIS * FINE_PER_AXIS / 32;

#[inline]
pub fn sub_index(sub_s: u32, sub_t: u32, sub_l: u32) -> u32 {
    (sub_l * FINE_PER_AXIS + sub_t) * FINE_PER_AXIS + sub_s
}

#[inline]
pub fn block_index(blocks_s: u32, blocks_l: u32, cs: u32, cl: u32, ct: u32) -> u32 {
    (ct * blocks_l + cl) * blocks_s + cs
}

#[inline]
pub fn block_axes(res: u32, layers: u32) -> [u32; 3] {
    [
        res.div_ceil(COARSE),
        layers.div_ceil(COARSE),
        res.div_ceil(COARSE),
    ]
}

#[inline]
pub fn blocks_per_face(res: u32, layers: u32) -> u32 {
    let axis = block_axes(res, layers);
    axis[0] * axis[1] * axis[2]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupancy {
    pub res: u32,
    pub layers: u32,
    pub blocks: [u32; 3],
    pub blocks_per_face: u32,
    pub sidecar: Vec<u32>,
    pub words: Vec<u32>,
}

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

#[inline]
fn voxel_is_empty(data: &[f32], voxel: usize, lanes: usize) -> bool {
    let base = voxel * lanes;
    for lane in 0..6.min(lanes) {
        if data[base + lane] != 0.0 {
            return false;
        }
    }
    true
}

impl Occupancy {
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
                            continue;
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

    pub fn blocks_per_face(&self) -> u32 {
        self.blocks_per_face
    }

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
            inner: 0.0,
            outer: 0.0,
            lanes: lanes as u32,
            data: padded,
        })
    }

    pub fn sidecar(&self) -> &[u32] {
        &self.sidecar
    }

    pub fn words(&self) -> &[u32] {
        &self.words
    }

    pub fn at(&self, point: [f32; 3], inner: f32, outer: f32) -> u32 {
        let radius = (point[0] * point[0] + point[1] * point[1] + point[2] * point[2]).sqrt();
        let (face, s, t) = grid_coords_of([
            point[0] / radius.max(1e-30),
            point[1] / radius.max(1e-30),
            point[2] / radius.max(1e-30),
        ]);
        self.at_cell(face, s, t, radius, inner, outer)
    }

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
        let word = self.words[packed * WORDS_PER_BLOCK as usize + (sub / 32) as usize];
        if word & (1_u32 << (sub % 32)) == 0 {
            return 1;
        }
        2
    }

    pub fn l1_words(&self) -> usize {
        self.sidecar.len().div_ceil(32).div_ceil(4) * 4
    }

    pub fn upload_words(&self) -> Vec<u32> {
        let l1_words = self.l1_words();
        let mut packed_words = vec![0_u32; l1_words + self.words.len()];
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

    #[test]
    fn a_marked_cell_is_never_all_zero() {
        let (res, layers) = (16_u32, 24_u32);
        let volume = blocked(res, layers);
        let occupancy = Occupancy::from_emission(&volume);
        let mut marked = 0_usize;
        for index in 0..4096_usize {
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
