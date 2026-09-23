//! **统一稀疏 R3 格**（索引那一半）：世界坐标里的物质都存这里。
//!
//! ⚠⚠ 这一档存在的理由（用户 2026-09-25 拍的口径）：
//!   *"一切物质都应该在世界坐标生成，球体坐标只应当用于储存/采样"* +
//!   *"把这个作为统一的稀疏结构，星云也用这个"*。
//!
//!   旧那套把**方向**当存储轴（立方图 / 立方球体网格）：一个面上纹素角差 3 倍
//!   ⇒ 世界空间里一个圆被存成椭圆、星图与天空面必须配对分辨率、六面接缝要单独补
//!   —— 那些全是"用方向网格存点/存体"带来的。R3 格里没有面、没有极、没有斜度。
//!
//! ⚠ 三级，**固定分叉、固定深度、全扁平数组**（GPU 优先：没有指针、没有变长下探）：
//!
//! ```text
//! chunk（块，每轴 CHUNK 个 brick）  稠密 CSR：chunk_start[块] → brick_slot 的区间
//!   └ brick（每轴 BRICK 个细格）    块内**稠密**表：每个未空块 CHUNK_BRICKS 项，EMPTY = 整块空
//!       └ 细格（cell）              BRICK³ 位掩码（u32 × MASK_WORDS）+ 占用细格的子 CSR
//! ```
//!
//! **"空则子全空"**是这一档的核心规则：`chunk_start[c] == chunk_start[c + 1]`（空块）
//! 或 `brick_slot[...] == EMPTY`（空 brick）⇒ 下面所有细格都不必看。查询于是是
//! 一条**定长**的下行：块 → brick → 掩码位 → 子 CSR 区间。
//!
//! ⚠ 常数取 2 的幂（`BRICK = 8`、`CHUNK = 4`）⇒ 细格 → (块, brick, 局部) 的分解是
//!   移位与掩码，不是除法；掩码正好 `BRICK³ / 32 = 16` 个 u32（WGSL 没有 64 位整数）。
//!
//! ⚠ **索引不带载荷**：`Grid` 只说"哪个细格里有东西、有几项、从第几项起"，
//!   载荷（星表 / 体素 brick 样本）由调用方按 [`Buckets::order`] 自己排。
//!   于是"点"与"体素"共用同一份索引与同一套遍历语义（GPU 侧那份 WGSL 也只写一遍）。

use std::ops::Range;

/// 每个 brick 每轴的细格数（掩码 = `BRICK³ = 512` 位 = `MASK_WORDS` 个 u32）。
pub const BRICK: u32 = 8;
/// 每个 chunk 每轴的 brick 数（块内 brick 表 = `CHUNK_BRICKS` 项）。
pub const CHUNK: u32 = 4;
/// 一个 chunk 每轴的细格数（`BRICK × CHUNK`）。
pub const CHUNK_CELLS: u32 = BRICK * CHUNK;
/// 掩码用几个 `u32`（`BRICK³ / 32`）。
///
/// ⚠ **用 `u32` 而不是 `u64`**：GPU 那一侧（WGSL）没有 64 位整数 ⇒ 掩码若按 u64 编进
///   blob，还得再拆一次两半（"拆得对不对"是一条没人看得见的隐患）。这里直接按 32 位词存，
///   两侧读法逐字相同。
pub const MASK_WORDS: usize = (BRICK * BRICK * BRICK / 32) as usize;
/// 块内 brick 表的项数。
pub const CHUNK_BRICKS: usize = (CHUNK * CHUNK * CHUNK) as usize;
/// "这一格/这一块是空的"。
pub const EMPTY: u32 = u32::MAX;

/// 格的空间参数（**进清单参数**，不进 blob —— 与 `VolumeData` 的 `inner/outer` 同一个口径）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridMeta {
    /// 细格边长（世界单位）。
    pub cell: f32,
    /// 格 `(0,0,0)` 的**近角**（世界坐标）。
    pub origin: [f32; 3],
    /// 每轴的细格数（**是 `CHUNK_CELLS` 的整数倍**：块对齐是位移分解的前提）。
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

    /// 世界点 → 细格坐标（**不判界**；越界由调用方筛）。
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

    /// 细格坐标 → 线性键（`x` 最快）。
    pub fn key_of(&self, cell: [i64; 3]) -> u32 {
        ((cell[2] as u32 * self.dims[1] + cell[1] as u32) * self.dims[0]) + cell[0] as u32
    }

    /// 线性键 → 细格坐标。
    pub fn cell_of_key(&self, key: u32) -> [i64; 3] {
        let x = key % self.dims[0];
        let rest = key / self.dims[0];
        let y = rest % self.dims[1];
        let z = rest / self.dims[1];
        [x as i64, y as i64, z as i64]
    }

    /// 细格坐标 → `(块键, brick 的块内局部键, 细格在 brick 内的局部键)`。
    ///
    /// ⚠ 全是移位与掩码（常数是 2 的幂）：GPU 那一侧逐格调用，除法在这里是浪费。
    ///   三个轴的权重都按 `x` 最快拼（`dims` 三轴可以不等）。
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

    /// [`Self::decompose`] 的逆（遍历与判据用）。
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

/// **索引**：三级稀疏结构，不带载荷。
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    pub meta: GridMeta,
    /// 块的 CSR（长度 = 块数 + 1）：空块两端相等 ⇒ 整块一次判掉。
    pub chunk_start: Vec<u32>,
    /// 每个**未空块**一块的稠密 brick 表（`CHUNK_BRICKS` 项）：局部 brick 键 → brick 下标。
    pub brick_slot: Vec<u32>,
    /// 每个 brick 的细格占用掩码（`MASK_WORDS` 个 u32）。
    pub brick_mask: Vec<u32>,
    /// 每个 brick 在 `sub_start` 里的起点。
    pub brick_sub: Vec<u32>,
    /// 占用细格的子 CSR（全局拼接）：每个 brick 占"占用数 + 1"项。
    pub sub_start: Vec<u32>,
    /// 载荷项数（= `sub_start` 的末项 = 每个细格的项数之和）。
    pub items: u32,
}

/// 造格的中间物：索引 + **载荷该按什么次序排**。
///
/// ⚠ 索引不认识载荷 ⇒ 造格只交出"第几项该放到第几位"，载荷由调用方自己搬。
///   于是"点"与"体素"共用这一份（体素那一档的"项"就是砖块样本）。
#[derive(Debug, Clone)]
pub struct Buckets {
    pub grid: Grid,
    /// 载荷的新次序：`order[新] = 旧`。
    pub order: Vec<u32>,
    /// 每一项落在哪个 brick（按**新**次序；判据与探针用）。
    pub brick_of: Vec<u32>,
}

/// 一个 brick 在造格过程中的累积（只在这一档内部用）。
struct BrickAcc {
    chunk: u32,
    local_brick: u32,
    /// `(细格局部键, 项数)`，**按局部键升序**。
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
    /// **从一批世界位置造格**（载荷项数 = `positions.len()`，每项一个位置）。
    ///
    /// ⚠ 排序是**确定性**的（细格键升序、同格内保持输入次序）⇒ 同一批位置永远同一个产物
    ///   （"键 = 内容"那条地基）。
    pub fn build(meta: GridMeta, positions: &[[f32; 3]]) -> Result<Buckets, String> {
        meta.validate()?;
        // ⚠⚠ 排序键必须是 **(块, brick, 细格)** 这个三元组，**不是**细格的线性键：
        //   细格键按 (z, y, x) 扫，一行 x 扫过去要跨过一排 brick，换行之后 brick 键
        //   又跳回去 ⇒ 同一个 brick 的项**不连续**（实测 224 个 brick 被切成 2387 段，
        //   症状是"绝大多数字段只有一颗星、查询大批落空"）。
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

        // 块层：每个未空块一块稠密 brick 表（"空则子全空"靠块区间长度 0 一次判掉）。
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

        // 掩码 + 子 CSR。
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

    /// 每个轴上的细格数（方便调用方）。
    pub fn dims(&self) -> [u32; 3] {
        self.meta.dims
    }

    /// brick 数。
    pub fn bricks(&self) -> usize {
        self.brick_sub.len()
    }

    /// 占用细格数（有载荷的细格；一个细格可以有多个项）。
    pub fn occupied_cells(&self) -> usize {
        self.sub_start.len() - self.bricks()
    }

    /// 一个 brick 的掩码那一段。
    fn mask_words(&self, brick: u32) -> &[u32] {
        let at = brick as usize * MASK_WORDS;
        &self.brick_mask[at..at + MASK_WORDS]
    }

    /// 细格坐标 → brick 下标（空 / 越界回 `None`）。
    ///
    /// ⚠ 这一条就是"空则子全空"：块区间长度 0 ⇒ 整块跳过；局部表项 `EMPTY` ⇒ 整块跳过。
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

    /// 一个 brick 里某个细格的载荷区间（该格为空回 `None`）。
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

    /// 世界点所在细格的载荷区间（空 / 越界回 `None`）。
    pub fn range_at(&self, point: [f32; 3]) -> Option<Range<usize>> {
        let cell = self.meta.cell_of(point);
        let (_, _, local) = self.meta.decompose(cell);
        let brick = self.brick_at(cell)?;
        self.cell_range(brick, local)
    }

    /// 一个 brick 里**所有非空细格**（回调：细格局部键 + 载荷区间）。
    ///
    /// ⚠ 走的是掩码里**置位的那些位**（不是 512 个格子全扫）：这是掩码在这一档里最值钱的
    ///   用法 —— 空细格一次都不进循环。
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

    /// 一个 AABB 里**所有非空细格**（回调：细格坐标 + 载荷区间）。
    ///
    /// ⚠ 逐**细格**走（每格 `brick_at` 是定长下行、一次掩码位测试），不是逐 brick
    ///   扫块内那张 64 项的表 —— 后者在"细格比块小得多"时白扫（锥形查询一帧要几万次）。
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

    /// 判据用：遍历所有非空细格（回调：细格坐标 + 载荷区间）。
    pub fn for_each_cell(&self, f: impl FnMut([i64; 3], Range<usize>)) {
        let low = self.meta.origin;
        let high = [
            self.meta.origin[0] + self.meta.dims[0] as f32 * self.meta.cell,
            self.meta.origin[1] + self.meta.dims[1] as f32 * self.meta.cell,
            self.meta.origin[2] + self.meta.dims[2] as f32 * self.meta.cell,
        ];
        self.for_each_cell_in(low, high, f);
    }

    /// 往返自检（构造口的自检，与 `TextureData::new` / `VolumeData::samples` 同一个口径）。
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
        // 子 CSR 的项数由**掩码**算出来（不是由它自己）：
        // 每个 brick 占 `占用细格数 + 1` 项 ⇒ 期望项数 = Σ(置位数) + brick 数。
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
