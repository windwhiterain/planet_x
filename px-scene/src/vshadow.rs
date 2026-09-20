//! **虚拟影图**（virtual shadow map）的分配器：从"每个投影物体要多少 texel/世界单位"
//! 算出一张**稀疏**的页表 + 物理 atlas 的布局。
//!
//! ## 它替掉了什么
//!
//! 从前是**每盏投影灯一张固定 1024² 的 cube**：一个 texel 的世界尺寸是 `2·d / 1024`
//! （`d` = 到灯的距离）。于是"太阳拉远"直接变成"影变糊" —— 太阳从 4.92 拉到 98 时，
//! 一个 texel 从 0.0096 世界单位涨到 0.19；而且那个按 `distance_to_light` 缩放的
//! `normal_offset` 会把整颗行星压暗一档（实测：平均通道差 4.43、65% 的像素差 >8）。
//!
//! ## 量纲：两个量分开算
//!
//! - **一个物体要多少 texel 才看得清**：`2·R·ρ`（直径 × 密度）。**只由物体定**。
//! - **那些 texel 摊在多大的页块上**：`ceil(2Rρ / PAGE_SIZE)` 页见方。
//!
//! 于是 `N_virt` 与**灯多远无关** —— 灯拉远不改变影的清晰度，也不改变分配
//! （旧版把 `d` 算进了虚拟面，于是"太阳拉远"会把虚拟面涨到几十万 texel，
//! 与"省显存"背道而驰）。
//!
//! ## 稀疏与页表
//!
//! `pages_per_side² × 6` 是**虚拟**页数，只有"真有投影物体落在上面"的那些页才分配物理
//! 槽位。采样侧要回答"虚拟页 → 物理槽位"，这一版把它放进一张 storage 缓冲：
//!
//! - **占用位掩码**：每面每行 `words_per_row` 个字，第 `px` 位 = 那个虚拟页占没占；
//! - **基址**：每面每行一个 u32 —— 那一行里**第一个**被占用的页的物理槽位。
//!
//! 着色器那一步于是是"弹出这一行里位于我前面（含我）的位数 - 1 + 基址"，一个 **rank**。
//! 位掩码把表压到"每页一个 u32"的 1/32，而 rank 只是一条 `countOneBits()` 的和。
//!
//! ## 一页一条 draw（而**不是**一条 pass）
//!
//! 执行器没有 indirect、也没有 per-draw 的 viewport ⇒ "按页栅格化"只能落在"每页一笔
//! draw、每笔一块 viewport"上。而**页数每帧都可能变**（灯动、物体动），所以：
//!
//! ⚠ **每帧重建的是资源与 draw 列表，绝不许因此重建管线。** 管线的缓存键只含
//! "顶点/片元 WGSL 文本 + 两个入口名 + 顶点布局 + 颜色格式 + 状态 + 每一组的布局身份"，
//! **不含页数、不含 viewport、不含绑定组对象** —— 所以每帧生成 N 笔 draw 不会让
//! `create_render_pipeline` 多跑一次。这正是"每帧重建"与"管线不许重建"能同时成立的原因。
//!
//! ## 为什么分配器住在这里而不是宿主里
//!
//! 它是**数据**：只读物体、灯、密度，产出布局与页表，一个 wgpu 类型都不碰。
//! 而"这一帧的影图怎么分配"是文档/执行器这一层的事（`ResourceSpec` 的尺寸规则也住这里）
//! ⇒ 纯函数住这一层，宿主只负责把结果变成 GPU 资源。

/// 一个页的边长（texel）。
pub const PAGE_SIZE: u32 = 128;

/// 每面每一行**最多**几个页（掩码宽度 = `PAGES_PER_ROW / 32` 个字）。
///
/// ⚠ 它是上限不是实况：实况是这一盏灯的 `pages_per_side`，表里每一行只存
/// `ceil(pages_per_side / 32)` 个字。
pub const PAGES_PER_ROW: u32 = 256;
/// 每行掩码的字数上限。
pub const WORDS_PER_ROW: u32 = PAGES_PER_ROW / 32;
/// 每面最多几行页（= 每面最多几页）。
pub const ROWS_PER_FACE: u32 = 256;
/// 每面最多分配多少页。
pub const MAX_PAGES_PER_FACE: u32 = ROWS_PER_FACE * PAGES_PER_ROW;
/// 每面页格的边长上限。
pub const MAX_PAGES_PER_SIDE: u32 = 256;

/// 一个 cube 的面数（次序照 `px_render::camera::CUBE_MAP_FACES`：`+X −X +Y −Y +Z −Z`）。
pub const CUBE_FACES: u32 = 6;

/// 一盏灯那一页表段的**头**字数。
///
/// ```text
///   [0] virtual_size（低 16 位）| pages_per_side（高 16 位）
///   [1] words_per_row
///   [2] 每一面那一段的字数（= rows × (1 + words_per_row)）
///   [3..] 面 0 的行段（每行：基址 1 字 + 掩码 words_per_row 字），接着面 1 ……
/// ```
pub const TABLE_HEAD_WORDS: u32 = 3;

/// 一盏灯的页表段字数：头 + 每面 `rows × (1 + words_per_row)`。
pub fn table_words_per_light(pages_per_side: u32) -> u32 {
    let words = pages_per_side.div_ceil(32);
    let rows = pages_per_side;
    TABLE_HEAD_WORDS + CUBE_FACES * rows * (1 + words)
}

/// 一个投影物体：灯要把它画进影图的那一份。
#[derive(Debug, Clone, PartialEq)]
pub struct Caster {
    pub id: String,
    /// 世界系的包围球半径（含变换的缩放）。
    pub radius: f32,
    /// 物体中心相对**灯**的位置。
    pub position: [f32; 3],
    /// texel / 世界单位。
    pub density: f32,
}

/// 一盏灯的虚拟影图布局。
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualShadowMap {
    /// 虚拟面边长（texel），是 `PAGE_SIZE` 的整数倍。
    pub virtual_size: u32,
    /// 每面的页数边长（`virtual_size / PAGE_SIZE`）。
    pub pages_per_side: u32,
    /// atlas 每层的页格边长（2 的幂，≥ `pages_per_side`）。
    pub atlas_pages_per_side: u32,
    /// 这一盏灯在页表缓冲里的**字偏移**。
    pub table_offset: u32,
}

/// 影子栅格化的一页：**画到哪**。
///
/// ⚠ 这是**每帧重算**的：它只依赖这一帧的物体 / 灯 / 密度。
#[derive(Debug, Clone, PartialEq)]
pub struct PagePatch {
    pub light: u32,
    pub face: u32,
    /// 虚拟页格坐标。
    pub page_x: u32,
    pub page_y: u32,
    /// 面内的物理页槽位（连续，从 0 开始）。
    pub slot: u32,
    /// 这一页在 atlas 那一层里的左上角（texel）。
    pub atlas_x: u32,
    pub atlas_y: u32,
    /// 这一页的虚拟 texel 窗口 `[x0, y0, x1, y1]`（右开、下开）。
    pub window: [u32; 4],
    /// 落进这一页的投影物体（`Caster::id`），按 `id` 排序去重 —— **每页只画这些**。
    pub casters: Vec<String>,
}

/// 分配结果。
#[derive(Debug, Clone, PartialEq)]
pub struct Allocation {
    pub lights: Vec<VirtualShadowMap>,
    pub patches: Vec<PagePatch>,
    /// atlas 的尺寸：`(宽, 高, 层数)`。层 = `灯数 × 6`（层号 = 灯 × 6 + 面）。
    pub atlas: (u32, u32, u32),
    /// 页表缓冲的 `u32` 字数。
    pub table_words: u32,
    /// 页表本体（所有灯连在一起）。
    pub table: Vec<u32>,
}

impl Allocation {
    /// 一盏投影灯都没有 ⇒ 一份空分配（宿主据此走兜底 atlas）。
    pub fn empty() -> Self {
        Self {
            lights: Vec::new(),
            patches: Vec::new(),
            atlas: (PAGE_SIZE, PAGE_SIZE, 0),
            table_words: 0,
            table: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.patches.is_empty()
    }
}

/// **分配失败**。这一版**没有**"悄悄降精度"这一档。
///
/// ⚠ 为什么不做降精度：降了之后画面**照样出得来**，只是影比要求糊，而"谁的精度被牺牲了"
/// 在画面上看不出来。这个工程对"能跑但悄悄画错"的容忍度是零。
#[derive(Debug, Clone, PartialEq)]
pub enum Overflow {
    /// 每面的页数超了上限。
    PerFace {
        light: u32,
        face: u32,
        wanted: u32,
        limit: u32,
    },
    /// 一页都没分出去。
    NoCasters { light: u32 },
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

/// 一个物体要多少 texel 才看得清：`2·R·ρ`。**只由物体定**（与灯多远无关）。
pub fn texels_for(radius: f32, density: f32) -> u32 {
    if density <= 0.0 || radius <= 0.0 {
        return 0;
    }
    let needed = 2.0 * f64::from(radius) * f64::from(density);
    needed.ceil().max(1.0) as u32
}

/// 一个物体在每一面上占的**页数边长**。
pub fn pages_for(radius: f32, density: f32) -> u32 {
    texels_for(radius, density).div_ceil(PAGE_SIZE).max(1)
}

/// `atlas` 每层的页格边长：2 的幂，≥ `pages_per_side`。
fn atlas_side_for(pages_per_side: u32) -> u32 {
    let mut side = 1_u32;
    while side < pages_per_side {
        side *= 2;
    }
    side
}

/// 表里某一面那一段的起点（相对这一盏灯的段首）。
fn face_section(face: u32, pages_per_side: u32) -> u32 {
    let words = pages_per_side.div_ceil(32);
    TABLE_HEAD_WORDS + face * pages_per_side * (1 + words)
}

/// 表里**某一面某一行**的基址字（相对这一盏灯的段首）。
pub fn base_word(face: u32, row: u32, pages_per_side: u32) -> u32 {
    let words = pages_per_side.div_ceil(32);
    face_section(face, pages_per_side) + row * (1 + words)
}

/// 表里**某一面某一行**的掩码首字（相对这一盏灯的段首）。
pub fn mask_word(face: u32, row: u32, pages_per_side: u32) -> u32 {
    base_word(face, row, pages_per_side) + 1
}

/// 把世界方向投到 cube 的某一面上：`(u, v)` 是**分量**，还没除主轴。
///
/// ⚠ 这两条映射**必须**与 `px_render::camera::CUBE_MAP_FACES` 的六条朝向、以及着色器侧
/// 的同名函数逐字对齐。三处各写一遍而写得不同，症状是"影贴到别的面上"或"影歪一点" ——
/// 两者都像内容的问题。
pub fn face_uv(direction: [f32; 3], face: u32) -> (f32, f32) {
    let [x, y, z] = direction;
    match face {
        0 => (-z, -y), // +X
        1 => (z, -y),  // −X
        2 => (x, z),   // +Y
        3 => (x, -z),  // −Y
        4 => (x, -y),  // +Z
        _ => (-x, -y), // −Z
    }
}

/// 一个物体在**某一面**上的页块左上角（页格坐标）。
///
/// 物体中心投到那一面 ⇒ `[0,1]` 的归一化位置 ⇒ 折算成页格；然后让 `span × span` 的块
/// 以它为中心、并**钳进** `[0, pages_per_side)`（中心贴边时块整体移进来，**不截断** ——
/// 截断就是"这个物体在那一面的影缺一块"，而那在画面上看不出来是分配错了）。
fn page_block_origin(position: [f32; 3], face: u32, span: u32, pages_per_side: u32) -> (u32, u32) {
    let (u, v) = face_uv(position, face);
    let major = position[0]
        .abs()
        .max(position[1].abs())
        .max(position[2].abs());
    // 投影到面上之后的正切 = 面内分量 / 主轴分量；`tan(45°) = 1` 就是半个面。
    let tan_u = if major > 0.0 {
        f64::from(u / major)
    } else {
        0.0
    };
    let tan_v = if major > 0.0 {
        f64::from(v / major)
    } else {
        0.0
    };
    let centre_u = (tan_u * 0.5 + 0.5).clamp(0.0, 1.0);
    let centre_v = (tan_v * 0.5 + 0.5).clamp(0.0, 1.0);
    let last = i64::from(pages_per_side) - i64::from(span);
    let x = (centre_u * f64::from(pages_per_side)) as i64 - i64::from(span / 2);
    let y = (centre_v * f64::from(pages_per_side)) as i64 - i64::from(span / 2);
    (
        x.clamp(0, last.max(0)) as u32,
        y.clamp(0, last.max(0)) as u32,
    )
}

/// 分配这一帧的全部灯。`lights[i]` 是第 `i` 盏投影灯照到的那些投影物体。
///
/// ⚠ **每帧都调**（灯与物体可能动）。它只读输入、只产出数据 —— 不碰 wgpu，
/// 所以"每帧重建"在这里的代价是几次浮点与一次排序。
pub fn allocate(lights: &[Vec<Caster>]) -> Result<Allocation, Overflow> {
    let mut out = Allocation::empty();
    let mut atlas_pages_side = 1_u32;

    for (index, casters) in lights.iter().enumerate() {
        let light = index as u32;
        let live: Vec<&Caster> = casters
            .iter()
            .filter(|caster| caster.density > 0.0 && caster.radius > 0.0)
            .collect();
        if live.is_empty() {
            return Err(Overflow::NoCasters { light });
        }

        // ---- 共享的虚拟格子：页数边长取最大的那个 span，向上取到 2 的幂 ----
        //
        // ⚠ **与灯的距离无关**（见模块头）：所以太阳拉远不会让分配变大。
        let mut pages_per_side = 4_u32;
        for caster in &live {
            pages_per_side = pages_per_side.max(pages_for(caster.radius, caster.density) + 2);
        }
        pages_per_side = pages_per_side.next_power_of_two();
        if pages_per_side > MAX_PAGES_PER_SIDE {
            return Err(Overflow::PerFace {
                light,
                face: 0,
                wanted: pages_per_side,
                limit: MAX_PAGES_PER_SIDE,
            });
        }

        // ---- 落页 ----
        //
        // ⚠ 这一版**不按面挑可见性**：一个包围球在六面上占同样大的格子数（球是各向同性
        //    的），所以"它落在哪几面"不影响格数，只影响哪些页存在。保守地六面都建是
        //    **确定的**；而"只建朝向它的那几面"要靠一个半空间判据，那个判据错了的症状是
        //    "某一面缺一块影"（难查），收益却只是页数减半 —— 页数本来就只有几十。
        let mut wanted: Vec<(u32, u32, u32, String)> = Vec::new();
        for caster in &live {
            let span = pages_for(caster.radius, caster.density) + 2;
            for face in 0..CUBE_FACES {
                let (x0, y0) = page_block_origin(caster.position, face, span, pages_per_side);
                for dy in 0..span {
                    for dx in 0..span {
                        let (px, py) = (x0 + dx, y0 + dy);
                        match wanted
                            .iter_mut()
                            .find(|(f, y, x, _)| *f == face && *y == py && *x == px)
                        {
                            Some((_, _, _, id)) => {
                                if !id.split('|').any(|seen| seen == caster.id) {
                                    id.push('|');
                                    id.push_str(&caster.id);
                                }
                            }
                            None => wanted.push((face, py, px, caster.id.clone())),
                        }
                    }
                }
            }
        }
        wanted.sort_by_key(|(face, y, x, _)| (*face, *y, *x));
        let atlas_pages = atlas_side_for(pages_per_side);
        atlas_pages_side = atlas_pages_side.max(atlas_pages);

        // ---- 装箱：每面一条扫描线，槽位在面内连续 ----
        let table_offset = out.table_words;
        let mut table = vec![0_u32; table_words_per_light(pages_per_side) as usize];
        let virtual_size = pages_per_side * PAGE_SIZE;
        table[0] = virtual_size | (pages_per_side << 16);
        table[1] = pages_per_side.div_ceil(32);
        table[2] = pages_per_side * (1 + table[1]);

        let mut patches: Vec<PagePatch> = Vec::with_capacity(wanted.len());
        let mut slot_in_face = 0_u32;
        let mut current_face = u32::MAX;
        let mut row = u32::MAX;
        let mut row_first_slot = 0_u32;
        let mut words = vec![0_u32; table[1] as usize];

        for (face, page_y, page_x, ids) in &wanted {
            if *face != current_face {
                if current_face != u32::MAX {
                    flush_row(
                        &mut table,
                        current_face,
                        row,
                        row_first_slot,
                        &words,
                        pages_per_side,
                    );
                }
                current_face = *face;
                slot_in_face = 0;
                row = *page_y;
                row_first_slot = 0;
                words = vec![0_u32; table[1] as usize];
            } else if *page_y != row {
                flush_row(
                    &mut table,
                    current_face,
                    row,
                    row_first_slot,
                    &words,
                    pages_per_side,
                );
                row = *page_y;
                row_first_slot = slot_in_face;
                words = vec![0_u32; table[1] as usize];
            }
            words[(page_x / 32) as usize] |= 1_u32 << (page_x % 32);
            let mut casters_here: Vec<String> = ids.split('|').map(str::to_string).collect();
            casters_here.sort();
            casters_here.dedup();
            patches.push(PagePatch {
                light,
                face: *face,
                page_x: *page_x,
                page_y: *page_y,
                slot: slot_in_face,
                atlas_x: (slot_in_face % atlas_pages) * PAGE_SIZE,
                atlas_y: (slot_in_face / atlas_pages) * PAGE_SIZE,
                window: [
                    page_x * PAGE_SIZE,
                    page_y * PAGE_SIZE,
                    (page_x + 1) * PAGE_SIZE,
                    (page_y + 1) * PAGE_SIZE,
                ],
                casters: casters_here,
            });
            slot_in_face += 1;
        }
        if current_face != u32::MAX {
            flush_row(
                &mut table,
                current_face,
                row,
                row_first_slot,
                &words,
                pages_per_side,
            );
        }

        out.table_words += table.len() as u32;
        out.table.extend_from_slice(&table);
        out.lights.push(VirtualShadowMap {
            virtual_size,
            pages_per_side,
            atlas_pages_per_side: atlas_pages,
            table_offset,
        });
        out.patches.extend(patches);
    }

    let side = atlas_pages_side * PAGE_SIZE;
    let layers = (out.lights.len() as u32) * CUBE_FACES;
    out.atlas = (side, side, layers.max(1));
    Ok(out)
}

/// 写一行的基址与掩码。
fn flush_row(
    table: &mut [u32],
    face: u32,
    row: u32,
    first_slot: u32,
    words: &[u32],
    pages_per_side: u32,
) {
    table[base_word(face, row, pages_per_side) as usize] = first_slot;
    let at = mask_word(face, row, pages_per_side) as usize;
    table[at..at + words.len()].copy_from_slice(words);
}

/// 从**物理槽位**反推 atlas 里的 texel 坐标 —— 与着色器里那两步逐字对应。
pub fn atlas_texel(slot: u32, atlas_pages_per_side: u32) -> (u32, u32) {
    (
        (slot % atlas_pages_per_side) * PAGE_SIZE,
        (slot / atlas_pages_per_side) * PAGE_SIZE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 灯在原点、物体在 `(distance, 0, 0)`。
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

    /// **这一轮的核心判据**：灯拉远不改变"物体处一个 texel 有多少世界大"，
    /// 也**不改变分配**。
    ///
    /// 旧的 1024² cube 在这一点上是红的（texel 世界尺寸 = `2d/1024`，灯远 20 倍就粗 20 倍）。
    #[test]
    fn density_is_independent_of_how_far_the_light_is() {
        let (radius, density) = (1.07_f32, 256.0_f32);
        let mut layouts = Vec::new();
        for factor in [1.0_f32, 5.0, 20.0, 100.0] {
            let allocation = one(at(4.92 * factor, radius, density));
            // 物体占的 texel 数 = `2Rρ` ⇒ 一个 texel 的世界尺寸 = `1/ρ`。
            let texel_world = 1.0 / density;
            let got = 1.0 / texel_world;
            assert!(
                got >= density - 1e-3,
                "灯拉远 {factor} 倍后密度掉到 {got}，要求 ≥ {density}"
            );
            layouts.push(allocation.lights[0].clone());
        }
        for other in &layouts[1..] {
            assert_eq!(
                other, &layouts[0],
                "灯拉远不该改变分配：旧版把虚拟面绑在灯的距离上"
            );
        }
    }

    /// 密度越高、物体越大，页块越大；而**页数远小于格子数**（稀疏）。
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
        // 半径 1.75、密度 256 ⇒ `ceil(2·1.75·256/128) = 7` 页边长，加余量 2 ⇒ 9×9。
        assert!(allocation.patches.len() <= 6 * 9 * 9);
    }

    /// 表里的"基址 + 掩码"说得出一页的物理槽位 —— 采样侧 rank 的定点对照。
    #[test]
    fn the_row_mask_and_base_reproduce_every_slot() {
        let allocation = one(at(4.95, 1.75, 256.0));
        let light = &allocation.lights[0];
        let base = light.table_offset;
        let pps = light.pages_per_side;
        for patch in &allocation.patches {
            let row_base =
                allocation.table[(base + base_word(patch.face, patch.page_y, pps)) as usize];
            let words = pps.div_ceil(32);
            let mut rank = 0_u32;
            for word in 0..words {
                let bits = allocation.table
                    [(base + mask_word(patch.face, patch.page_y, pps) + word) as usize];
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
                atlas_texel(patch.slot, light.atlas_pages_per_side),
                (patch.atlas_x, patch.atlas_y)
            );
            assert!(patch.atlas_x + PAGE_SIZE <= allocation.atlas.0);
            assert!(patch.atlas_y + PAGE_SIZE <= allocation.atlas.1);
        }
    }

    /// 一盏投影灯一个正密度物体都没有 ⇒ 拒（而不是立一盏空转的灯）。
    #[test]
    fn a_light_with_no_casters_is_refused() {
        let err = allocate(&[vec![at(4.0, 1.0, 0.0)]]).expect_err("没有投影物体 ⇒ 拒");
        assert!(matches!(err, Overflow::NoCasters { light: 0 }), "{err:?}");
    }

    /// 分配是**确定的**：同样的输入两次给出逐字相同的表与页。
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

    /// 面映射的六条：`+Z` 面看到 `(x, −y)`、`−Z` 面看到 `(−x, −y)`
    /// （cube 是左手 y-up，而世界是右手 —— 这两条是翻过的）。
    #[test]
    fn the_face_map_matches_the_cube_order() {
        let dir = [3.0, 0.4, 0.2];
        assert_eq!(face_uv(dir, 0), (-0.2, -0.4)); // +X
        assert_eq!(face_uv(dir, 1), (0.2, -0.4)); // −X
        assert_eq!(face_uv(dir, 2), (3.0, 0.2)); // +Y
        assert_eq!(face_uv(dir, 3), (3.0, -0.2)); // −Y
        assert_eq!(face_uv(dir, 4), (3.0, -0.4)); // +Z
        assert_eq!(face_uv(dir, 5), (-3.0, -0.4)); // −Z
    }

    /// 页块**不许越出格子**：物体中心贴边时块整体移进来（不是截断）。
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
