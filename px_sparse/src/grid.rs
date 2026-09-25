use std::ops::Range;

pub const BRICK: u32 = 8;
pub const CHUNK: u32 = 4;
pub const CHUNK_CELLS: u32 = BRICK * CHUNK;
pub const MASK_WORDS: usize = (BRICK * BRICK * BRICK / 32) as usize;
pub const CHUNK_BRICKS: usize = (CHUNK * CHUNK * CHUNK) as usize;
pub const EMPTY: u32 = u32::MAX;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridMeta {
    pub cell: f32,
    pub origin: [f32; 3],
    pub dims: [u32; 3],
}

impl GridMeta {
    pub fn chunk_dims(&self) -> [u32; 3] {
        [
            self.dims[0] / CHUNK_CELLS,
            self.dims[1] / CHUNK_CELLS,
            self.dims[2] / CHUNK_CELLS,
        ]
    }

    pub fn chunks(&self) -> usize {
        let dims = self.chunk_dims();
        dims[0] as usize * dims[1] as usize * dims[2] as usize
    }

    pub fn cells(&self) -> usize {
        self.dims[0] as usize * self.dims[1] as usize * self.dims[2] as usize
    }

    pub fn cell_of(&self, point: [f32; 3]) -> [i64; 3] {
        let mut out = [0_i64; 3];
        for axis in 0..3 {
            let local = (point[axis] - self.origin[axis]) / self.cell;
            out[axis] = local.floor() as i64;
        }
        out
    }

    pub fn inside(&self, cell: [i64; 3]) -> bool {
        (0..3).all(|axis| cell[axis] >= 0 && cell[axis] < self.dims[axis] as i64)
    }

    pub fn key_of(&self, cell: [i64; 3]) -> u32 {
        ((cell[2] as u32 * self.dims[1] + cell[1] as u32) * self.dims[0]) + cell[0] as u32
    }

    pub fn cell_of_key(&self, key: u32) -> [i64; 3] {
        let x = key % self.dims[0];
        let rest = key / self.dims[0];
        let y = rest % self.dims[1];
        let z = rest / self.dims[1];
        [x as i64, y as i64, z as i64]
    }

    pub fn decompose(&self, cell: [i64; 3]) -> (u32, u32, u32) {
        let dims = self.chunk_dims();
        let geometry = |axis: usize| -> (u32, u32, u32) {
            let value = cell[axis] as u32;
            (value / CHUNK_CELLS, (value / BRICK) % CHUNK, value % BRICK)
        };
        let (cx, bx, lx) = geometry(0);
        let (cy, by, ly) = geometry(1);
        let (cz, bz, lz) = geometry(2);
        let chunk = (cz * dims[1] + cy) * dims[0] + cx;
        let brick = (bz * CHUNK + by) * CHUNK + bx;
        let local = (lz * BRICK + ly) * BRICK + lx;
        (chunk, brick, local)
    }

    pub fn compose(&self, chunk: [u32; 3], brick: [u32; 3], local: [u32; 3]) -> [i64; 3] {
        let mut out = [0_i64; 3];
        for axis in 0..3 {
            out[axis] = (chunk[axis] * CHUNK_CELLS + brick[axis] * BRICK + local[axis]) as i64;
        }
        out
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(self.cell > 0.0) || !self.cell.is_finite() {
            return Err(format!("细格边长是 {}（必须是正的有限数）", self.cell));
        }
        for axis in 0..3 {
            if self.dims[axis] == 0 || self.dims[axis] % CHUNK_CELLS != 0 {
                return Err(format!(
                    "第 {axis} 轴的细格数 {} 必须是 {} 的整数倍（块对齐是位移分解的前提）",
                    self.dims[axis], CHUNK_CELLS
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub meta: GridMeta,
    pub chunk_start: Vec<u32>,
    pub brick_slot: Vec<u32>,
    pub brick_mask: Vec<u32>,
    pub brick_sub: Vec<u32>,
    pub sub_start: Vec<u32>,
    pub items: u32,
}

#[derive(Debug, Clone)]
pub struct Buckets {
    pub grid: Grid,
    pub order: Vec<u32>,
    pub brick_of: Vec<u32>,
}

struct BrickAcc {
    chunk: u32,
    local_brick: u32,
    cells: Vec<(u32, u32)>,
}

impl BrickAcc {
    fn mask(&self) -> [u32; MASK_WORDS] {
        let mut words = [0_u32; MASK_WORDS];
        for (local, _) in &self.cells {
            words[*local as usize / 32] |= 1_u32 << (*local % 32);
        }
        words
    }
}

impl Grid {
    pub fn build(meta: GridMeta, positions: &[[f32; 3]]) -> Result<Buckets, String> {
        meta.validate()?;
        let mut keyed: Vec<((u32, u32, u32), u32)> = Vec::with_capacity(positions.len());
        for (index, position) in positions.iter().enumerate() {
            let cell = meta.cell_of(*position);
            if !meta.inside(cell) {
                return Err(format!(
                    "第 {index} 项的位置 {position:?} 落在格外面（格从 {:?} 起、每轴 {} 格、边长 {}）",
                    meta.origin, meta.dims[0], meta.cell
                ));
            }
            keyed.push((meta.decompose(cell), index as u32));
        }
        keyed.sort_by_key(|(cell, index)| (*cell, *index));

        let mut bricks: Vec<BrickAcc> = Vec::new();
        let mut order: Vec<u32> = Vec::with_capacity(keyed.len());
        let mut brick_of: Vec<u32> = Vec::with_capacity(keyed.len());
        for ((chunk, local_brick, local_cell), original) in &keyed {
            let (chunk, local_brick, local_cell) = (*chunk, *local_brick, *local_cell);
            let index = match bricks.last() {
                Some(acc) if acc.chunk == chunk && acc.local_brick == local_brick => {
                    bricks.len() - 1
                }
                _ => {
                    bricks.push(BrickAcc {
                        chunk,
                        local_brick,
                        cells: Vec::new(),
                    });
                    bricks.len() - 1
                }
            };
            let acc = &mut bricks[index];
            match acc.cells.last_mut() {
                Some((local, count)) if *local == local_cell => *count += 1,
                _ => acc.cells.push((local_cell, 1)),
            }
            order.push(*original);
            brick_of.push(index as u32);
        }

        let dims = meta.chunk_dims();
        let chunks = meta.chunks();
        let mut chunk_start = vec![0_u32; chunks + 1];
        let mut brick_slot: Vec<u32> = Vec::new();
        let mut per_chunk: Vec<Vec<u32>> = vec![Vec::new(); chunks];
        for (index, acc) in bricks.iter().enumerate() {
            per_chunk[acc.chunk as usize].push(index as u32);
        }
        for (chunk, list) in per_chunk.iter().enumerate() {
            chunk_start[chunk] = brick_slot.len() as u32;
            if list.is_empty() {
                continue;
            }
            let base = brick_slot.len();
            brick_slot.resize(base + CHUNK_BRICKS, EMPTY);
            for brick_index in list {
                let local = bricks[*brick_index as usize].local_brick;
                brick_slot[base + local as usize] = *brick_index;
            }
        }
        chunk_start[chunks] = brick_slot.len() as u32;
        let _ = dims;

        let mut brick_mask: Vec<u32> = Vec::with_capacity(bricks.len() * MASK_WORDS);
        let mut brick_sub: Vec<u32> = Vec::with_capacity(bricks.len());
        let mut sub_start: Vec<u32> = Vec::new();
        let mut running = 0_u32;
        for acc in &bricks {
            brick_sub.push(sub_start.len() as u32);
            brick_mask.extend_from_slice(&acc.mask());
            sub_start.push(running);
            for (_, count) in &acc.cells {
                running += count;
                sub_start.push(running);
            }
        }
        let grid = Grid {
            meta,
            chunk_start,
            brick_slot,
            brick_mask,
            brick_sub,
            sub_start,
            items: running,
        };
        grid.validate()?;
        Ok(Buckets {
            grid,
            order,
            brick_of,
        })
    }

    pub fn dims(&self) -> [u32; 3] {
        self.meta.dims
    }

    pub fn bricks(&self) -> usize {
        self.brick_sub.len()
    }

    pub fn occupied_cells(&self) -> usize {
        self.sub_start.len() - self.bricks()
    }

    fn mask_words(&self, brick: u32) -> &[u32] {
        let at = brick as usize * MASK_WORDS;
        &self.brick_mask[at..at + MASK_WORDS]
    }

    pub fn brick_at(&self, cell: [i64; 3]) -> Option<u32> {
        if !self.meta.inside(cell) {
            return None;
        }
        let (chunk, local_brick, _) = self.meta.decompose(cell);
        let start = self.chunk_start[chunk as usize] as usize;
        let end = self.chunk_start[chunk as usize + 1] as usize;
        if start == end {
            return None;
        }
        let brick = self.brick_slot[start + local_brick as usize];
        (brick != EMPTY).then_some(brick)
    }

    pub fn cell_range(&self, brick: u32, local: u32) -> Option<Range<usize>> {
        let words = self.mask_words(brick);
        let word = local as usize / 32;
        let bit = local as usize % 32;
        if words[word] & (1_u32 << bit) == 0 {
            return None;
        }
        let rank: u32 = words[..word].iter().map(|w| w.count_ones()).sum::<u32>()
            + (words[word] & ((1_u32 << bit) - 1)).count_ones();
        let at = self.brick_sub[brick as usize] as usize + rank as usize;
        Some(self.sub_start[at] as usize..self.sub_start[at + 1] as usize)
    }

    pub fn range_at(&self, point: [f32; 3]) -> Option<Range<usize>> {
        let cell = self.meta.cell_of(point);
        let (_, _, local) = self.meta.decompose(cell);
        let brick = self.brick_at(cell)?;
        self.cell_range(brick, local)
    }

    pub fn for_each_occupied(&self, brick: u32, mut f: impl FnMut(u32, Range<usize>)) {
        for (word_index, word) in self.mask_words(brick).iter().enumerate() {
            let mut bits = *word;
            while bits != 0 {
                let bit = bits.trailing_zeros();
                bits &= bits - 1;
                let local = (word_index * 32) as u32 + bit;
                let range = self
                    .cell_range(brick, local)
                    .expect("掩码置位的细格必然有区间");
                f(local, range);
            }
        }
    }

    pub fn for_each_cell_in(
        &self,
        low: [f32; 3],
        high: [f32; 3],
        mut f: impl FnMut([i64; 3], Range<usize>),
    ) {
        let lo = self.meta.cell_of(low);
        let hi = self.meta.cell_of(high);
        let clamp =
            |value: i64, axis: usize| -> i64 { value.clamp(0, self.meta.dims[axis] as i64 - 1) };
        for z in clamp(lo[2], 2)..=clamp(hi[2], 2) {
            for y in clamp(lo[1], 1)..=clamp(hi[1], 1) {
                for x in clamp(lo[0], 0)..=clamp(hi[0], 0) {
                    let cell = [x, y, z];
                    let (_, _, local) = self.meta.decompose(cell);
                    let Some(brick) = self.brick_at(cell) else {
                        continue;
                    };
                    if let Some(range) = self.cell_range(brick, local) {
                        f(cell, range);
                    }
                }
            }
        }
    }

    pub fn for_each_cell(&self, f: impl FnMut([i64; 3], Range<usize>)) {
        let low = self.meta.origin;
        let high = [
            self.meta.origin[0] + self.meta.dims[0] as f32 * self.meta.cell,
            self.meta.origin[1] + self.meta.dims[1] as f32 * self.meta.cell,
            self.meta.origin[2] + self.meta.dims[2] as f32 * self.meta.cell,
        ];
        self.for_each_cell_in(low, high, f);
    }

    pub fn validate(&self) -> Result<(), String> {
        self.meta.validate()?;
        let chunks = self.meta.chunks();
        if self.chunk_start.len() != chunks + 1 {
            return Err(format!(
                "块 CSR 长度是 {}，应当是块数 {} + 1",
                self.chunk_start.len(),
                chunks
            ));
        }
        if self.chunk_start.first().copied() != Some(0) {
            return Err("块 CSR 的第一个前缀必须是 0".to_string());
        }
        for pair in self.chunk_start.windows(2) {
            let span = pair[1] as i64 - pair[0] as i64;
            if span < 0 || (span != 0 && span != CHUNK_BRICKS as i64) {
                return Err(format!(
                    "块区间只能是 0（空块）或 {CHUNK_BRICKS}（未空块），实际 {}",
                    pair[1] as i64 - pair[0] as i64
                ));
            }
        }
        if self.chunk_start.last().map(|last| *last as usize) != Some(self.brick_slot.len()) {
            return Err("块 CSR 的末尾与 brick 表的长度对不上".to_string());
        }
        let bricks = self.brick_sub.len();
        if self.brick_mask.len() != bricks * MASK_WORDS {
            return Err(format!(
                "掩码长度 {} 与 brick 数 {bricks} × {MASK_WORDS} 对不上",
                self.brick_mask.len()
            ));
        }
        let occupied: usize = self
            .brick_mask
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum();
        if self.sub_start.len() != occupied + bricks {
            return Err(format!(
                "子 CSR 有 {} 项，掩码说有 {} 个占用细格 + {bricks} 个 brick",
                self.sub_start.len(),
                occupied
            ));
        }
        if !self.sub_start.is_empty() && self.sub_start[0] != 0 {
            return Err("子 CSR 的第一个前缀必须是 0".to_string());
        }
        for pair in self.sub_start.windows(2) {
            if pair[1] < pair[0] {
                return Err(format!("子 CSR 前缀回退了：{} → {}", pair[0], pair[1]));
            }
        }
        if self.sub_start.last().copied().unwrap_or(0) != self.items {
            return Err(format!(
                "子 CSR 末尾是 {:?}，载荷项数却是 {}",
                self.sub_start.last(),
                self.items
            ));
        }
        for slot in &self.brick_slot {
            if *slot != EMPTY && *slot as usize >= bricks {
                return Err(format!(
                    "brick 表里有一个越界的下标 {slot}（brick 数 {bricks}）"
                ));
            }
        }
        for brick in 0..bricks {
            let words = self.mask_words(brick as u32);
            if words.iter().all(|word| *word == 0) {
                return Err(format!("第 {brick} 个 brick 的掩码是空的（不该被登记）"));
            }
        }
        Ok(())
    }
}
