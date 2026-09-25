pub const PAGE_SIZE: u32 = 128;

pub const PAGES_PER_ROW: u32 = 512;
pub const WORDS_PER_ROW: u32 = PAGES_PER_ROW / 32;
pub const ROWS_PER_FACE: u32 = 512;
pub const MAX_PAGES_PER_FACE: u32 = ROWS_PER_FACE * PAGES_PER_ROW;
pub const MAX_PAGES_PER_SIDE: u32 = 512;

pub const CUBE_FACES: u32 = 6;

pub const MAX_ATLAS_PAGES_PER_SIDE: u32 = 32;

pub const SHADOW_ATLAS_RESOURCES: [&str; MAX_LEVELS as usize] = [
    "point_shadow_atlas",
    "point_shadow_atlas_l1",
    "point_shadow_atlas_l2",
    "point_shadow_atlas_l3",
];

pub fn shadow_atlas_resource(level: u32) -> String {
    SHADOW_ATLAS_RESOURCES[level as usize].to_string()
}

pub const TABLE_HEAD_WORDS: u32 = 2;

pub const PREFIX_WORDS: u32 = MAX_LEVELS;

pub const MAX_LEVELS: u32 = 4;

pub fn table_words_per_light(pages_per_side: u32, levels: u32) -> u32 {
    let mut words = TABLE_HEAD_WORDS + PREFIX_WORDS;
    for level in 0..levels {
        words += CUBE_FACES * pps_rows_words(pages_per_side, level);
    }
    words
}

#[derive(Debug, Clone, PartialEq)]
pub struct Caster {
    pub id: String,
    pub radius: f32,
    pub position: [f32; 3],
    pub density: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VirtualShadowMap {
    pub virtual_size: u32,
    pub pages_per_side: u32,
    pub levels: u32,
    pub atlas_pages: [u32; MAX_LEVELS as usize],
    pub table_offset: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PagePatch {
    pub light: u32,
    pub level: u32,
    pub face: u32,
    pub page_x: u32,
    pub page_y: u32,
    pub slot: u32,
    pub atlas_x: u32,
    pub atlas_y: u32,
    pub window: [u32; 4],
    pub casters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Allocation {
    pub lights: Vec<VirtualShadowMap>,
    pub patches: Vec<PagePatch>,
    pub atlas: (u32, u32, u32),
    pub atlas_sides: Vec<u32>,
    pub table_words: u32,
    pub table: Vec<u32>,
}

impl Allocation {
    pub fn empty() -> Self {
        Self {
            lights: Vec::new(),
            patches: Vec::new(),
            atlas: (PAGE_SIZE, PAGE_SIZE, 0),
            atlas_sides: Vec::new(),
            table_words: 0,
            table: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Overflow {
    PerFace {
        light: u32,
        face: u32,
        wanted: u32,
        limit: u32,
    },
    NoCasters {
        light: u32,
    },
}

impl std::fmt::Display for Overflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Overflow::PerFace {
                light,
                face,
                wanted,
                limit,
            } => write!(
                f,
                "第 {light} 盏灯的 cube 第 {face} 面要 {wanted} 页（每页 {PAGE_SIZE}²），\
                 而上限是 {limit}：把这一帧的 shadow_density 调小。\
                 这一版**不降精度** —— 降了之后画面照样出得来，只是影比要求糊"
            ),
            Overflow::NoCasters { light } => write!(
                f,
                "第 {light} 盏投影灯一个密度为正的物体都没有：没有任何页要分配。\
                 一个投影物体都没有时该把 `shadows` 关掉，而不是立一盏空转的灯"
            ),
        }
    }
}

pub fn texels_for(radius: f32, density: f32) -> u32 {
    if density <= 0.0 || radius <= 0.0 {
        return 0;
    }
    let needed = 2.0 * f64::from(radius) * f64::from(density);
    needed.ceil().max(1.0) as u32
}

pub fn pages_for(radius: f32, density: f32) -> u32 {
    texels_for(radius, density).div_ceil(PAGE_SIZE).max(1)
}

fn ceil_sqrt(pages: u32) -> u32 {
    let mut side = 1_u32;
    while side.saturating_mul(side) < pages {
        side += 1;
    }
    side
}

pub fn pages_at_level(pages_per_side: u32, level: u32) -> u32 {
    (pages_per_side >> level).max(1)
}

pub fn levels_for(pages_per_side: u32, max_levels: u32) -> u32 {
    let mut levels = 1_u32;
    while levels < max_levels && (pages_per_side >> levels) >= 1 {
        levels += 1;
    }
    levels
}

fn stage_section(level: u32, face: u32, pages_per_side: u32) -> u32 {
    let mut at = TABLE_HEAD_WORDS + PREFIX_WORDS;
    for j in 0..level {
        at += CUBE_FACES * pps_rows_words(pages_per_side, j);
    }
    at + face * pps_rows_words(pages_per_side, level)
}

pub fn pps_rows_words(pages_per_side: u32, level: u32) -> u32 {
    let pps = pages_at_level(pages_per_side, level);
    pps * (1 + pps.div_ceil(32))
}

pub fn base_word(level: u32, face: u32, row: u32, pages_per_side: u32) -> u32 {
    let words = pages_at_level(pages_per_side, level).div_ceil(32);
    stage_section(level, face, pages_per_side) + row * (1 + words)
}

pub fn mask_word(level: u32, face: u32, row: u32, pages_per_side: u32) -> u32 {
    base_word(level, face, row, pages_per_side) + 1
}

pub fn caster_level(density: f32, max_density: f32, levels: u32) -> u32 {
    if density <= 0.0 || max_density <= density {
        return 0;
    }
    let ratio = f64::from(max_density) / f64::from(density);
    let mut level = 0_u32;
    while level + 1 < levels && (1_u64 << (level + 1)) <= ratio as u64 {
        level += 1;
    }
    level
}

pub fn face_uv(direction: [f32; 3], face: u32) -> (f32, f32) {
    let basis = px_protocol::scene::SHADOW_FACE_BASIS[(face as usize).min(5)];
    let dot = |vector: [f32; 3]| {
        vector[0] * direction[0] + vector[1] * direction[1] + vector[2] * direction[2]
    };
    let denom = dot(basis[2]);
    if denom == 0.0 {
        return (0.0, 0.0);
    }
    (dot(basis[0]) / denom, dot(basis[1]) / denom)
}

pub fn face_side(pages_per_side: u32) -> f32 {
    (pages_per_side * PAGE_SIZE) as f32
}

pub fn face_texel(uv: (f32, f32), face_side: f32) -> (f32, f32) {
    let (u, v) = uv;
    ((u * 0.5 + 0.5) * face_side, (0.5 - v * 0.5) * face_side)
}

fn page_block_origin(position: [f32; 3], face: u32, span: u32, pages_per_side: u32) -> (u32, u32) {
    let (u, v) = face_uv(position, face);
    let side = face_side(pages_per_side);
    let (x_texel, y_texel) = face_texel((u, v), side);
    let centre_u = f64::from((x_texel / side).clamp(0.0, 1.0));
    let centre_v = f64::from((y_texel / side).clamp(0.0, 1.0));
    let last = i64::from(pages_per_side) - i64::from(span);
    let x = (centre_u * f64::from(pages_per_side)) as i64 - i64::from(span / 2);
    let y = (centre_v * f64::from(pages_per_side)) as i64 - i64::from(span / 2);
    (
        x.clamp(0, last.max(0)) as u32,
        y.clamp(0, last.max(0)) as u32,
    )
}

pub fn allocate(lights: &[Vec<Caster>]) -> Result<Allocation, Overflow> {
    let mut out = Allocation::empty();
    let mut atlas_pages_side = [1_u32; MAX_LEVELS as usize];

    for (index, casters) in lights.iter().enumerate() {
        let light = index as u32;
        let live: Vec<&Caster> = casters
            .iter()
            .filter(|caster| caster.density > 0.0 && caster.radius > 0.0)
            .collect();
        if live.is_empty() {
            return Err(Overflow::NoCasters { light });
        }

        let light_reach = live
            .iter()
            .map(|caster| {
                let [x, y, z] = caster.position;
                f64::from(x) * f64::from(x)
                    + f64::from(y) * f64::from(y)
                    + f64::from(z) * f64::from(z)
            })
            .fold(0.0_f64, f64::max)
            .sqrt();
        let density = live
            .iter()
            .map(|caster| f64::from(caster.density))
            .fold(0.0_f64, f64::max);
        let wanted_pages = (light_reach * density / (PAGE_SIZE as f64 / 2.0))
            .ceil()
            .max(4.0);
        let mut pages_per_side = 4_u32;
        while f64::from(pages_per_side) < wanted_pages {
            pages_per_side *= 2;
            if pages_per_side > MAX_PAGES_PER_SIDE {
                return Err(Overflow::PerFace {
                    light,
                    face: 0,
                    wanted: pages_per_side,
                    limit: MAX_PAGES_PER_SIDE,
                });
            }
        }

        let levels = levels_for(pages_per_side, MAX_LEVELS);
        let mut wanted: Vec<(u32, u32, u32, u32, String)> = Vec::new();
        for caster in &live {
            let own = caster_level(caster.density, density as f32, levels);
            for level in own..levels {
                let pps = pages_at_level(pages_per_side, level);
                let page_world_level = 2.0 * light_reach / f64::from(pps);
                let span = if page_world_level > 0.0 {
                    ((2.0 * f64::from(caster.radius) / page_world_level).ceil() as u32).max(1) + 2
                } else {
                    pages_for(caster.radius, caster.density) + 2
                };
                let span = span.min(pps);
                for face in 0..CUBE_FACES {
                    let (x0, y0) = page_block_origin(caster.position, face, span, pps);
                    for dy in 0..span {
                        for dx in 0..span {
                            let (px, py) = (x0 + dx, y0 + dy);
                            match wanted.iter_mut().find(|(l, f, y, x, _)| {
                                *l == level && *f == face && *y == py && *x == px
                            }) {
                                Some((_, _, _, _, id)) => {
                                    if !id.split('|').any(|seen| seen == caster.id) {
                                        id.push('|');
                                        id.push_str(&caster.id);
                                    }
                                }
                                None => {
                                    let id = if level == own {
                                        caster.id.clone()
                                    } else {
                                        String::new()
                                    };
                                    wanted.push((level, face, py, px, id));
                                }
                            }
                        }
                    }
                }
            }
        }
        wanted.sort_by_key(|(level, face, y, x, _)| (*level, *face, *y, *x));

        let mut per_level_face = [[0_u32; CUBE_FACES as usize]; MAX_LEVELS as usize];
        for (level, face, _, _, _) in &wanted {
            per_level_face[*level as usize][*face as usize] += 1;
        }
        let mut atlas_pages_by_level = [0_u32; MAX_LEVELS as usize];
        for level in 0..levels {
            let max_per_face = per_level_face[level as usize]
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .max(1);
            let grid = ceil_sqrt(max_per_face);
            let cap = MAX_ATLAS_PAGES_PER_SIDE;
            if grid > cap {
                return Err(Overflow::PerFace {
                    light,
                    face: 0,
                    wanted: max_per_face,
                    limit: cap * cap,
                });
            }
            atlas_pages_by_level[level as usize] = grid;
        }
        let atlas_pages = atlas_pages_by_level[0];
        for level in 0..levels {
            atlas_pages_side[level as usize] =
                atlas_pages_side[level as usize].max(atlas_pages_by_level[level as usize]);
        }

        let table_offset = out.table_words;
        let mut table = vec![0_u32; table_words_per_light(pages_per_side, levels) as usize];
        table[0] = pages_per_side | (atlas_pages << 16);
        table[1] = levels;
        {
            let mut at = TABLE_HEAD_WORDS + PREFIX_WORDS;
            for level in 0..levels {
                table[(TABLE_HEAD_WORDS + level) as usize] = at;
                at += CUBE_FACES * pps_rows_words(pages_per_side, level);
            }
        }

        let mut patches: Vec<PagePatch> = Vec::with_capacity(wanted.len());
        let mut slots = [[0_u32; CUBE_FACES as usize]; MAX_LEVELS as usize];
        let mut current: Option<(u32, u32)> = None;
        let mut row = u32::MAX;
        let mut row_first_slot = 0_u32;
        let mut words = vec![0_u32; pages_per_side.div_ceil(32) as usize];

        for (level, face, page_y, page_x, ids) in &wanted {
            let pps = pages_at_level(pages_per_side, *level);
            let words_per_row = pps.div_ceil(32);
            if current != Some((*level, *face)) {
                if let Some((l, f)) = current {
                    flush_row(
                        &mut table,
                        l,
                        f,
                        row,
                        row_first_slot,
                        &words,
                        pages_per_side,
                    );
                }
                current = Some((*level, *face));
                row = *page_y;
                row_first_slot = slots[*level as usize][*face as usize];
                words = vec![0_u32; words_per_row as usize];
            } else if *page_y != row {
                flush_row(
                    &mut table,
                    *level,
                    *face,
                    row,
                    row_first_slot,
                    &words,
                    pages_per_side,
                );
                row = *page_y;
                row_first_slot = slots[*level as usize][*face as usize];
                words = vec![0_u32; words_per_row as usize];
            }
            words[(page_x / 32) as usize] |= 1_u32 << (page_x % 32);
            let mut casters_here: Vec<String> = ids
                .split('|')
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect();
            casters_here.sort();
            casters_here.dedup();
            let slot = slots[*level as usize][*face as usize];
            patches.push(PagePatch {
                light,
                level: *level,
                face: *face,
                page_x: *page_x,
                page_y: *page_y,
                slot,
                atlas_x: (slot % atlas_pages_by_level[*level as usize]) * PAGE_SIZE,
                atlas_y: (slot / atlas_pages_by_level[*level as usize]) * PAGE_SIZE,
                window: [
                    page_x * PAGE_SIZE,
                    page_y * PAGE_SIZE,
                    (page_x + 1) * PAGE_SIZE,
                    (page_y + 1) * PAGE_SIZE,
                ],
                casters: casters_here,
            });
            slots[*level as usize][*face as usize] += 1;
        }
        if let Some((l, f)) = current {
            flush_row(
                &mut table,
                l,
                f,
                row,
                row_first_slot,
                &words,
                pages_per_side,
            );
        }

        out.table_words += table.len() as u32;
        out.table.extend_from_slice(&table);
        out.lights.push(VirtualShadowMap {
            virtual_size: pages_per_side * PAGE_SIZE,
            pages_per_side,
            levels,
            atlas_pages: atlas_pages_by_level,
            table_offset,
        });
        out.patches.extend(patches);
    }

    let layers = (out.lights.len() as u32) * CUBE_FACES;
    out.atlas_sides = atlas_pages_side
        .iter()
        .map(|grid| grid * PAGE_SIZE)
        .collect();
    out.atlas = (out.atlas_sides[0], out.atlas_sides[0], layers.max(1));
    Ok(out)
}

fn flush_row(
    table: &mut [u32],
    level: u32,
    face: u32,
    row: u32,
    first_slot: u32,
    words: &[u32],
    pages_per_side: u32,
) {
    table[base_word(level, face, row, pages_per_side) as usize] = first_slot;
    let at = mask_word(level, face, row, pages_per_side) as usize;
    table[at..at + words.len()].copy_from_slice(words);
}

pub fn atlas_texel(slot: u32, atlas_pages_per_side: u32) -> (u32, u32) {
    (
        (slot % atlas_pages_per_side) * PAGE_SIZE,
        (slot / atlas_pages_per_side) * PAGE_SIZE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(distance: f32, radius: f32, density: f32) -> Caster {
        Caster {
            id: "c".to_string(),
            radius,
            position: [distance, 0.0, 0.0],
            density,
        }
    }

    fn one(caster: Caster) -> Allocation {
        allocate(&[vec![caster]]).expect("分得出来")
    }

    #[test]
    fn the_texel_world_size_is_independent_of_how_far_the_light_is() {
        let (radius, density) = (1.07_f32, 256.0_f32);
        let mut sizes = Vec::new();
        for factor in [1.0_f32, 2.0, 5.0, 10.0] {
            let distance = 4.92 * factor;
            let allocation = one(at(distance, radius, density));
            let light = &allocation.lights[0];
            let texel_world = 2.0 * distance / (light.pages_per_side * PAGE_SIZE) as f32;
            assert!(
                texel_world <= 1.0 / density * 1.001,
                "灯在 {distance} 处一个 texel 有 {texel_world} 个世界单位，要求 ≤ {}（1/ρ）—— \
                 比要求粗就是「太阳拉远精度变差」那一条",
                1.0 / density
            );
            sizes.push(texel_world);
        }
        let min = sizes.iter().cloned().fold(f32::MAX, f32::min);
        let max = sizes.iter().cloned().fold(0.0, f32::max);
        assert!(
            max / min <= 2.001,
            "四档的 texel 世界尺寸 {sizes:?} 差得超过一倍（2 的幂取整的余量）"
        );
    }

    #[test]
    fn the_grid_grows_with_distance_while_the_pages_stay_sparse() {
        let near = one(at(4.92, 1.07, 256.0));
        let far = one(at(49.2, 1.07, 256.0));
        assert!(
            far.lights[0].pages_per_side > near.lights[0].pages_per_side,
            "灯远十倍，格子该更细：近 {} vs 远 {}",
            near.lights[0].pages_per_side,
            far.lights[0].pages_per_side
        );
        let ratio = far.patches.len() as f64 / near.patches.len() as f64;
        assert!(
            ratio < 4.0,
            "灯远十倍而分出去的页数涨了 {ratio} 倍（近 {} 页 / 远 {} 页，近侧 levels={} pps={} own 该是 {}）—— 稀疏那一半丢了",
            near.patches.len(),
            far.patches.len(),
            near.lights[0].levels,
            near.lights[0].pages_per_side,
            0
        );
    }

    #[test]
    fn pages_are_sparse_in_the_virtual_grid() {
        let allocation = allocate(&[vec![
            at(4.95, 1.01, 256.0),
            at(4.95, 1.75, 256.0),
            at(3.82, 0.093, 2048.0),
        ]])
        .expect("分得出来");
        let light = &allocation.lights[0];
        let grid = (light.pages_per_side * light.pages_per_side * CUBE_FACES) as usize;
        assert!(
            allocation.patches.len() < grid,
            "页数 {} 该小于格子数 {grid}",
            allocation.patches.len()
        );
        assert!(
            allocation.patches.len() * 4 <= grid,
            "页数 {} 该远小于格子数 {grid}（至少稀 4 倍）",
            allocation.patches.len()
        );
    }

    #[test]
    fn a_low_density_caster_lands_on_a_coarser_level() {
        let allocation =
            allocate(&[vec![at(4.95, 1.0, 256.0), at(4.95, 1.0, 64.0)]]).expect("分得出来");
        let light = &allocation.lights[0];
        assert_eq!(light.levels, 4, "pages_per_side 这一档该有 4 级");

        let fine: Vec<&PagePatch> = allocation
            .patches
            .iter()
            .filter(|patch| patch.level == 0)
            .collect();
        let coarse: Vec<&PagePatch> = allocation
            .patches
            .iter()
            .filter(|patch| patch.level == 2)
            .collect();
        assert!(!fine.is_empty() && !coarse.is_empty(), "两级都该有页");

        let coarse_boxes: Vec<(u32, u32, u32, u32)> = coarse
            .iter()
            .map(|patch| (patch.page_x, patch.page_y, patch.page_x, patch.page_y))
            .collect();
        assert!(
            !coarse_boxes.is_empty(),
            "级 2 上该有页（否则接收者要粗级时只能退到细级）"
        );
        let fine_count = fine.len();
        let coarse_count = coarse.len();
        assert!(
            fine_count > 0 && coarse_count > 0,
            "细级 {fine_count} 页、粗级 {coarse_count} 页"
        );
        assert!(
            coarse_count < fine_count,
            "粗级的页数 {coarse_count} 该少于细级 {fine_count}"
        );
        let coarse_pps = pages_at_level(light.pages_per_side, 2);
        for patch in &coarse {
            assert!(patch.page_x < coarse_pps && patch.page_y < coarse_pps);
        }

        let base = light.table_offset;
        let pps = light.pages_per_side;
        for patch in &allocation.patches {
            let patch_pps = pages_at_level(pps, patch.level);
            let row_base = allocation.table
                [(base + base_word(patch.level, patch.face, patch.page_y, pps)) as usize];
            let words = patch_pps.div_ceil(32);
            let mut rank = 0_u32;
            for word in 0..words {
                let bits = allocation.table[(base
                    + mask_word(patch.level, patch.face, patch.page_y, pps)
                    + word) as usize];
                if word < patch.page_x / 32 {
                    rank += bits.count_ones();
                } else {
                    let upto = patch.page_x % 32;
                    let mask = if upto == 31 {
                        u32::MAX
                    } else {
                        (1_u32 << (upto + 1)) - 1
                    };
                    rank += (bits & mask).count_ones();
                    break;
                }
            }
            assert_eq!(
                row_base + rank - 1,
                patch.slot,
                "级 {} 面 {} 行 {} 列 {} 的槽位对不上",
                patch.level,
                patch.face,
                patch.page_y,
                patch.page_x
            );
        }
    }

    #[test]
    fn the_row_mask_and_base_reproduce_every_slot() {
        let allocation = one(at(4.95, 1.75, 256.0));
        let light = &allocation.lights[0];
        let base = light.table_offset;
        let pps = light.pages_per_side;
        for patch in &allocation.patches {
            let patch_pps = pages_at_level(pps, patch.level);
            let row_base = allocation.table
                [(base + base_word(patch.level, patch.face, patch.page_y, pps)) as usize];
            let words = patch_pps.div_ceil(32);
            let mut rank = 0_u32;
            for word in 0..words {
                let bits = allocation.table[(base
                    + mask_word(patch.level, patch.face, patch.page_y, pps)
                    + word) as usize];
                if word < patch.page_x / 32 {
                    rank += bits.count_ones();
                } else {
                    let upto = patch.page_x % 32;
                    let mask = if upto == 31 {
                        u32::MAX
                    } else {
                        (1_u32 << (upto + 1)) - 1
                    };
                    rank += (bits & mask).count_ones();
                    break;
                }
            }
            assert_eq!(
                row_base + rank - 1,
                patch.slot,
                "面 {} 页 ({}, {}) 的表读数与槽位对不上",
                patch.face,
                patch.page_x,
                patch.page_y
            );
            assert_eq!(
                atlas_texel(patch.slot, light.atlas_pages[patch.level as usize]),
                (patch.atlas_x, patch.atlas_y)
            );
            assert!(patch.atlas_x + PAGE_SIZE <= allocation.atlas.0);
            assert!(patch.atlas_y + PAGE_SIZE <= allocation.atlas.1);
        }
    }

    #[test]
    fn a_light_with_no_casters_is_refused() {
        let err = allocate(&[vec![at(4.0, 1.0, 0.0)]]).expect_err("没有投影物体 ⇒ 拒");
        assert!(matches!(err, Overflow::NoCasters { light: 0 }), "{err:?}");
    }

    #[test]
    fn allocation_is_deterministic() {
        let lights = vec![
            vec![at(4.95, 1.01, 256.0), at(3.82, 0.093, 2048.0)],
            vec![at(2.0, 0.5, 512.0)],
        ];
        let one = allocate(&lights).expect("分得出来");
        let two = allocate(&lights).expect("分得出来");
        assert_eq!(one, two);
        assert_eq!(one.atlas.2, 12, "层号 = 灯 × 6 + 面");
    }

    #[test]
    fn the_face_map_matches_the_cube_order() {
        let dir = [3.0, 0.4, 0.2];
        let axes = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
        ];
        for (face, axis) in axes.iter().enumerate() {
            let basis = px_protocol::scene::SHADOW_FACE_BASIS[face];
            assert_eq!(
                basis[2], *axis,
                "第 {face} 面的轴：`CUBE_MAP_FACES` 说 {axis:?}，协议那张表说 {:?}",
                basis[2]
            );
            let dot = |v: [f32; 3]| v[0] * dir[0] + v[1] * dir[1] + v[2] * dir[2];
            let denom = dot(basis[2]);
            assert_eq!(
                face_uv(dir, face as u32),
                (dot(basis[0]) / denom, dot(basis[1]) / denom),
                "第 {face} 面的 `(u, v)`"
            );
        }
        assert_eq!(face_uv([1.0, 0.0, 0.0], 0), (0.0, 0.0), "+X 面正中央");
        assert_eq!(face_uv([-1.0, 0.0, 0.0], 1), (0.0, 0.0), "−X 面正中央");
        assert_eq!(face_uv([0.0, 1.0, 0.0], 2), (0.0, 0.0), "+Y 面正中央");
        assert_eq!(face_uv([0.0, -1.0, 0.0], 3), (0.0, 0.0), "−Y 面正中央");
        assert_eq!(
            face_uv([0.0, 0.0, -1.0], 4),
            (0.0, 0.0),
            "−Z 面正中央（第 4 面）"
        );
        assert_eq!(
            face_uv([0.0, 0.0, 1.0], 5),
            (0.0, 0.0),
            "+Z 面正中央（第 5 面）"
        );
        assert_eq!(face_uv([0.2, 0.4, -3.0], 4), (0.2 / 3.0, 0.4 / 3.0));
        assert_eq!(face_uv([0.2, 0.4, 3.0], 5), (-0.2 / 3.0, 0.4 / 3.0));
    }

    #[test]
    fn the_atlas_row_grows_downwards() {
        let side = 128.0;
        assert_eq!(face_texel((0.0, 0.0), side), (64.0, 64.0));
        assert_eq!(face_texel((1.0, 1.0), side), (128.0, 0.0));
        assert_eq!(face_texel((-1.0, -1.0), side), (0.0, 128.0));
    }

    #[test]
    fn a_block_stays_inside_the_grid() {
        let allocation = one(at(4.95, 1.75, 256.0));
        let light = &allocation.lights[0];
        for patch in &allocation.patches {
            assert!(patch.page_x < light.pages_per_side);
            assert!(patch.page_y < light.pages_per_side);
        }
    }
}
