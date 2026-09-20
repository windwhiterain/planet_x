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
pub const PAGES_PER_ROW: u32 = 512;
/// 每行掩码的字数上限。
pub const WORDS_PER_ROW: u32 = PAGES_PER_ROW / 32;
/// 每面最多几行页（= 每面最多几页）。
pub const ROWS_PER_FACE: u32 = 512;
/// 每面最多分配多少页。
pub const MAX_PAGES_PER_FACE: u32 = ROWS_PER_FACE * PAGES_PER_ROW;
/// 每面页格的边长上限。
pub const MAX_PAGES_PER_SIDE: u32 = 512;

/// 一个 cube 的面数（次序照 `px_render::camera::CUBE_MAP_FACES`：`+X −X +Y −Y +Z −Z`）。
pub const CUBE_FACES: u32 = 6;

/// 一盏灯那一页表段的**头**字数（不含前缀表那一段）。
///
/// ```text
///   [0] pages_per_side（低 16 位）| **物理 atlas 的页格边长**（高 16 位）
///   [1] levels（低 16 位）| 0
///   [2 .. 2+levels]  level_offset[级] —— 那一级的六个面从第几个字开始（前缀和）
///   [2+levels ..]   级 0 的六个面，然后级 1 的六个面 ……
///                   每一面：`pps_级` 行 ×（基址 1 字 + 掩码 `ceil(pps_级/32)` 字）
/// ```
///
/// ⚠ 页表**从这一轮起是"级优先"的两维表**（用户裁决的 (ii)）：`pps_级 = pages_per_side >> 级`。
///    级 0 最细（`pages_per_side = dρ_max/64`，精度要求定的），级 k 的一页盖住级 0 的
///    `2^k × 2^k` 格 ⇒ 一个 texel 的世界尺寸 ×`2^k`。**"细格子重新聚合成大格子"就是这一维。**
///
/// ⚠ `level_offset` 落一张前缀表而不是让采样侧自己求和：着色器每次采样都要算段首，
///    让它跑一遍 Σ 就是在最热的那条路径上加一个循环。
pub const TABLE_HEAD_WORDS: u32 = 2;

/// 头里那张前缀表的字数上限（= `MAX_LEVELS`）。
pub const PREFIX_WORDS: u32 = MAX_LEVELS;

/// 级数上限。**硬编码的定长**：WGSL 不允许动态下标一组纹理，将来"每级一张 atlas"
/// （目标 ③）要靠一条定长链选纹理，所以级数必须是个常数。
pub const MAX_LEVELS: u32 = 4;

/// 一盏灯的页表段字数：头 + 前缀表 + 每一级每一面 `pps_级 × (1 + 掩码字)`。
///
/// ⚠ 等比级数是 `Σ 1/4^级 ≈ 4/3` ⇒ **把金字塔建满只比只建级 0 贵 1/3**。
pub fn table_words_per_light(pages_per_side: u32, levels: u32) -> u32 {
    let mut words = TABLE_HEAD_WORDS + PREFIX_WORDS;
    for level in 0..levels {
        words += CUBE_FACES * pps_rows_words(pages_per_side, level);
    }
    words
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
    /// 每面的页数边长（`virtual_size / PAGE_SIZE`）—— **级 0（最细那一级）**的。
    pub pages_per_side: u32,
    /// 这一盏灯有几级（`1..=MAX_LEVELS`）。级 k 的页数边长 = `pages_per_side >> k`。
    pub levels: u32,
    /// atlas 每层的页格边长（2 的幂，≥ 那一层里最多的页数）。
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
    /// 这一页在哪一级。`0` 最细；级 k 的一页盖住级 0 的 `2^k × 2^k` 格。
    pub level: u32,
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

/// 装得下 `pages` 页的方形网格边长（向上取整），**不**取 2 的幂。
fn ceil_sqrt(pages: u32) -> u32 {
    let mut side = 1_u32;
    while side.saturating_mul(side) < pages {
        side += 1;
    }
    side
}

/// 一个**级**的页数边长：`pages_per_side >> level`。
///
/// level 0 是**最细**的那张格子（精度要求定的，见 `allocate` 那段推导）；level k 的
/// 每一页仍然 128 texel，但它盖住 level 0 的 `2^k × 2^k` 格 ⇒ **一页顶 4^k 页**，
/// 一个 texel 的世界尺寸是最细那级的 `2^k` 倍。这就是"细格子重新聚合成大格子"。
pub fn pages_at_level(pages_per_side: u32, level: u32) -> u32 {
    (pages_per_side >> level).max(1)
}

/// `pages_per_side` 这一档最多能有几级：`pps >> (levels-1) >= 1`。
pub fn levels_for(pages_per_side: u32, max_levels: u32) -> u32 {
    let mut levels = 1_u32;
    while levels < max_levels && (pages_per_side >> levels) >= 1 {
        levels += 1;
    }
    levels
}

/// 表里 `(级, 面)` 那一段的起点（相对这一盏灯的段首）。
///
/// ⚠ 布局是**级优先**：`level 0` 的六个面，然后 `level 1` 的六个面……
///    头里因此存一张 `level_offset[级]` 的前缀表（`allocate` 里填），采样侧一次载入
///    就能算出段首 —— 不在着色器里跑一遍前缀和。
fn stage_section(level: u32, face: u32, pages_per_side: u32) -> u32 {
    let mut at = TABLE_HEAD_WORDS + PREFIX_WORDS;
    for j in 0..level {
        at += CUBE_FACES * pps_rows_words(pages_per_side, j);
    }
    at + face * pps_rows_words(pages_per_side, level)
}

/// 一个级里**一面**的表字数（`pps × (1 + 掩码字)`）。
pub fn pps_rows_words(pages_per_side: u32, level: u32) -> u32 {
    let pps = pages_at_level(pages_per_side, level);
    pps * (1 + pps.div_ceil(32))
}

/// 表里**某一级某一面某一行**的基址字（相对这一盏灯的段首）。
pub fn base_word(level: u32, face: u32, row: u32, pages_per_side: u32) -> u32 {
    let words = pages_at_level(pages_per_side, level).div_ceil(32);
    stage_section(level, face, pages_per_side) + row * (1 + words)
}

/// 表里**某一级某一面某一行**的掩码首字（相对这一盏灯的段首）。
pub fn mask_word(level: u32, face: u32, row: u32, pages_per_side: u32) -> u32 {
    base_word(level, face, row, pages_per_side) + 1
}

/// 一个物体该落到哪一级：取**最粗**的那一级，使它的 texel 世界尺寸仍 ≤ `1/ρ`。
///
/// 最细那级（level 0）是按全场**最细**的 ρ 定的（`pages_per_side = dρ_max/64`），
/// 所以 level k 的 texel 世界尺寸 = `2^k / ρ_max`。要它 ≤ `1/ρ`：
///
/// ```text
///   2^k / ρ_max ≤ 1/ρ   ⇒   2^k ≤ ρ_max/ρ   ⇒   k = floor(log2(ρ_max/ρ))
/// ```
///
/// ⚠ **这就是"按精度配置稀疏分配"那条口径**：精度要求低的物体自己落到粗级，
///    页数按 `4^-k` 掉，不必跟最细的那个物体一样占细页。
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

/// 把世界方向投到 cube 的某一面上：`(u, v)` 是**面内归一化坐标**（`∈ [−1, 1]`，
/// `v` **向上**为正），`u` 向右为正。
///
/// ⚠ 朝向**不在这里定义**：它读 [`px_protocol::scene::SHADOW_FACE_BASIS`] ——
/// 那是这条契约的唯一一处（用户裁决）。从前这里是六条手写的 `match`，而它与渲染器、
/// 与着色器**三份都不同**：4/5 面的朝向与渲染器反、v 轴在六个面上还不自洽
/// （2/3 面一个符号、0/1/4/5 面另一个符号）。症状是页分到了错的面/错的半面。
///
/// ⚠ `v` 这里**不翻**：翻的那一次只在"换成 atlas 像素"那一步（见 [`face_texel`]），
/// 一处翻转好过两处相互抵消。
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

/// 这一面**在 atlas 里的边长**（texel）。页格与 texel 都按它折算。
pub fn face_side(pages_per_side: u32) -> f32 {
    (pages_per_side * PAGE_SIZE) as f32
}

/// 面内归一化坐标 → **atlas 面内的 texel**（`x` 向右、`y` **向下**）。
///
/// ⚠ **全流程唯一的 y 翻转就在这一行**：`face_uv` 的 `v` 与 NDC y 同向（向上为正），
/// 而 atlas 的行号向下增。翻两次就等于没翻，而"没翻"的症状是影子上下镜像 ——
/// 在球面上看着像"影子有点歪"，不像镜像。
pub fn face_texel(uv: (f32, f32), face_side: f32) -> (f32, f32) {
    let (u, v) = uv;
    ((u * 0.5 + 0.5) * face_side, (0.5 - v * 0.5) * face_side)
}

/// 一个物体在**某一面**上的页块左上角（页格坐标）。
///
/// 物体中心投到那一面 ⇒ `[0,1]` 的归一化位置 ⇒ 折算成页格；然后让 `span × span` 的块
/// 以它为中心、并**钳进** `[0, pages_per_side)`（中心贴边时块整体移进来，**不截断** ——
/// 截断就是"这个物体在那一面的影缺一块"，而那在画面上看不出来是分配错了）。
fn page_block_origin(position: [f32; 3], face: u32, span: u32, pages_per_side: u32) -> (u32, u32) {
    let (u, v) = face_uv(position, face);
    let side = face_side(pages_per_side);
    // ⚠ 走**同一套**换算（`face_uv` → `face_texel`）：页的格子位置与着色器查页时
    //    算出来的 texel 必须落在同一格里，否则影子会整体偏移若干页 —— 而那就成了
    //    "影糊了"或"影缺一块"，从画面上看不出是分配错了。
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

        // ---- 共享的虚拟格子：**由精度要求决定，所以它随灯距变**（§本轮的用户口径）----
        //
        // ⚠⚠ 这一栏从前是"页数边长取最大的那个 span、向上取到 2 的幂"——那是个**错的模型**，
        //    而它错的方式正好等于用户报的那个症状。推导：
        //
        //      一面是 90° 视锥 ⇒ 在距灯 `d` 处，这一面横跨 **2d** 个世界单位。
        //      这一面有 `pages_per_side × 128` 个 texel。
        //      ⇒ 一个 texel 的世界尺寸 = 2d / (pages_per_side × 128)。
        //
        //    要 1 个 texel = 1/ρ 个世界单位（ρ = texel/世界单位，用户给的精度要求），
        //    就必须 `pages_per_side = 2dρ/128 = dρ/64` —— **与 d 成正比**。
        //    从前它是"某个 caster 的 span + 2 取 2 的幂"，与 d **无关** ⇒
        //    实际的 texel 世界尺寸 = 2d/虚拟边长 ∝ d ⇒ **太阳拉远，精度就按比例变差**。
        //
        //    而"页稀疏"这件事一分都没少：网格变细，但**只有 caster 盖到的那几页**
        //    被分配（下面的 `wanted`），atlas 的尺寸只跟分出去的页数有关。
        //    ⇒ 变细的是**格子**，不是 atlas。
        //
        // ⚠ 取整到 2 的幂：槽位的行内解码用 `slot % pages_per_side`，而 atlas 每层的页格
        //    边长是 2 的幂（`atlas_side_for`）—— 两者不相等时"页格 → 物理槽位"会错位。
        //    向上取到 2 的幂只会让精度**比要求的更细**（128/110 ≈ 1.16 倍），是安全的一侧。
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
        // `pages_per_side = dρ/64`，向上取整，再向上取到 2 的幂。
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

        // ---- 落页 ----
        //
        // ⚠ 这一版**不按面挑可见性**：一个包围球在六面上占同样大的格子数（球是各向同性
        //    的），所以"它落在哪几面"不影响格数，只影响哪些页存在。保守地六面都建是
        //    **确定的**；而"只建朝向它的那几面"要靠一个半空间判据，那个判据错了的症状是
        //    "某一面缺一块影"（难查），收益却只是页数减半 —— 页数本来就只有几十。
        //
        // ⚠ 块的大小**按这一格真的有多少世界单位**算，不照 `pages_for` 那个按 ρ 估的数：
        //    一页 = `2d/pages_per_side` 个世界单位，而 `pages_per_side` 是取过 2 的幂的，
        //    所以按 ρ 估会**偏小**（偏小 = 这个物体的影缺一块）。
        //
        // ⚠⚠ **每个 caster 按自己的 ρ 选级**（§本轮的目标 ②）：精度要求低的物体落到粗级，
        //    它占的页数按 `4^-级` 掉 —— 这才是"按精度配置稀疏分配"。粗级的一页盖住的
        //    世界范围是 `2^级` 倍，所以同一个物体在粗级上占的**页数边长**几乎不变
        //    （世界尺寸没变、一页更大 ⇒ 页数更少）。
        let levels = levels_for(pages_per_side, MAX_LEVELS);
        let mut wanted: Vec<(u32, u32, u32, u32, String)> = Vec::new();
        for caster in &live {
            let level = caster_level(caster.density, density as f32, levels);
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
                                wanted.push((level, face, py, px, caster.id.clone()));
                            }
                        }
                    }
                }
            }
        }
        wanted.sort_by_key(|(level, face, y, x, _)| (*level, *face, *y, *x));

        // ---- 物理 atlas 的页格边长：**按分出去的页数**算，与虚拟格子脱钩 ----
        //
        // ⚠⚠ 这一条是"格子随灯距变细"落地时**必须**一起改的那一半（§本轮）。
        //    从前 `atlas_pages = atlas_side_for(pages_per_side)` —— atlas 的页格边长
        //    直接抄虚拟格子。而虚拟格子现在随 `d × ρ` 变细（那是精度要求的实现），
        //    于是 atlas 跟着炸：5× 那一档要 16384² × 6 层 × 4B = **6.4 GB**。
        //
        //    两者本来是**两种网格**，只是从前同值：
        //      · 虚拟格子（`pages_per_side`）：精度要求定的，细；
        //      · 物理 atlas（`atlas_pages`）：**分出去多少页**定的，稀疏那一半。
        //    槽位解码（`slot % atlas_pages`）用后者，页索引用前者。
        //    两者混用 ⇒ "格子一变细 atlas 就爆"。
        //
        // ⚠ 槽位**在一面里跨级连续**（一层的 atlas 是那个面独占的）：级 0 的页先排，
        //    接着级 1 的页…… ⇒ 每面一个计数器，`atlas_pages` 按**面**上最多的那个数算。
        let mut per_face = [0_u32; CUBE_FACES as usize];
        for (_, face, _, _, _) in &wanted {
            per_face[*face as usize] += 1;
        }
        let max_per_face = per_face.iter().copied().max().unwrap_or(1).max(1);
        // ⚠ 要的是**边长**不是页数：一层里塞 `max_per_face` 页，边长至少 `ceil(√页数)`。
        //    直接拿页数当边长会浪费一个平方（81 页 ⇒ 边长 128 ⇒ 16384² 的 atlas，
        //    而 9 页边长就够）。
        let atlas_pages = atlas_side_for(ceil_sqrt(max_per_face));
        atlas_pages_side = atlas_pages_side.max(atlas_pages);

        // ---- 装箱：级优先 → 面 → 行（每面一条扫描线，槽位在面内跨级连续）----
        let table_offset = out.table_words;
        let mut table = vec![0_u32; table_words_per_light(pages_per_side, levels) as usize];
        // ⚠ `virtual_size = pages_per_side × PAGE_SIZE` **不落盘**：`pages_per_side`
        //    到 512 时它是 65536，塞不进 16 位。采样侧由 `pages_per_side` 现算 ——
        //    反正两者差一个常数因子。
        table[0] = pages_per_side | (atlas_pages << 16);
        table[1] = levels;
        // 前缀表：级 `l` 的六个面从第几个字开始。
        {
            let mut at = TABLE_HEAD_WORDS + PREFIX_WORDS;
            for level in 0..levels {
                table[(TABLE_HEAD_WORDS + level) as usize] = at;
                at += CUBE_FACES * pps_rows_words(pages_per_side, level);
            }
        }

        let mut patches: Vec<PagePatch> = Vec::with_capacity(wanted.len());
        // 槽位**每面一个计数器**（跨级连续）—— 因为 atlas 的一层是那个面独占的。
        let mut slots = [0_u32; CUBE_FACES as usize];
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
                row_first_slot = slots[*face as usize];
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
                row_first_slot = slots[*face as usize];
                words = vec![0_u32; words_per_row as usize];
            }
            words[(page_x / 32) as usize] |= 1_u32 << (page_x % 32);
            let mut casters_here: Vec<String> = ids.split('|').map(str::to_string).collect();
            casters_here.sort();
            casters_here.dedup();
            let slot = slots[*face as usize];
            patches.push(PagePatch {
                light,
                level: *level,
                face: *face,
                page_x: *page_x,
                page_y: *page_y,
                slot,
                atlas_x: (slot % atlas_pages) * PAGE_SIZE,
                atlas_y: (slot / atlas_pages) * PAGE_SIZE,
                window: [
                    page_x * PAGE_SIZE,
                    page_y * PAGE_SIZE,
                    (page_x + 1) * PAGE_SIZE,
                    (page_y + 1) * PAGE_SIZE,
                ],
                casters: casters_here,
            });
            slots[*face as usize] += 1;
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

    /// **这一轮的核心判据**：物体处"一个 texel 有多少世界"与灯距**无关**。
    ///
    /// 推导（`allocate` 里那段注释的定点对照）：一面 90° ⇒ 距灯 `d` 处这一面横跨 `2d`
    /// 个世界单位，而它有 `pages_per_side × 128` 个 texel ⇒ 一个 texel 的世界尺寸
    /// = `2d / (pages_per_side × 128)`。要它 = `1/ρ` 就必须 `pages_per_side = dρ/64`
    /// ⇒ **格子随 d 变细**。
    ///
    /// ⚠ 这条判据**从前写反了**：旧版断的是"灯拉远**不改变分配**"（把整个
    /// `LightLayout` 逐字段相等当成功），而那条恰好**就是**用户报的那个病 ——
    /// 格子不随距离变 ⇒ texel 世界尺寸 ∝ d ⇒ 太阳拉远精度按比例变差。
    /// 判据把一个 bug 当成不变量钉住，比没有判据更糟：它会挡住修复。
    ///
    /// ⚠ 取整到 2 的幂 ⇒ 实际比要求**更细**（最多细一倍），永远不会更粗。
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

    /// 精度要求越高、灯越远，**格子越细**（这是上一条判据的另一面）：
    /// `pages_per_side` 随 `d × ρ` 涨，而**分出去的页数不跟着涨**（稀疏那一半没丢）。
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
        // 物体没变大 ⇒ 它占的**页数**该差不多（成比例地涨一点点，因为块按世界尺寸算）。
        let ratio = far.patches.len() as f64 / near.patches.len() as f64;
        assert!(
            ratio < 4.0,
            "灯远十倍而分出去的页数涨了 {ratio} 倍 —— 稀疏那一半丢了"
        );
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
        // ⚠ 这里从前断的是"页数 ≤ 6×9×9"（一个**绝对**上界）—— 那个数绑在
        //    "格子不随灯距变"的旧模型上，格子一变细就假红。稀疏这件事的正确说法是
        //    **比例**：分出去的页要远小于格子总数。
        assert!(
            allocation.patches.len() * 4 <= grid,
            "页数 {} 该远小于格子数 {grid}（至少稀 4 倍）",
            allocation.patches.len()
        );
    }

    /// **这一轮的核心判据**：精度要求低的物体自己落到**粗级**，页数按 `4^-级` 掉 ——
    /// 而粗级的"基址 + 掩码"仍然说得回它自己的物理槽位（表是两维的，不是只有级 0）。
    ///
    /// ⚠ 上一条 `the_row_mask_and_base_reproduce_every_slot` 用的是**单个**密度 256 的
    ///    物体 ⇒ `caster_level(256, 256, _) = 0` ⇒ 它**只**走过级 0。两级混在一起时
    ///    行段偏移、掩码字数、页数边长全都跟着级走，那条判据一个字都没覆盖。
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
        // `floor(log2(256/64)) = 2` ⇒ 一页的世界尺寸 ×4 ⇒ **不含余量**的页数边长 1/4、
        // 页数 1/16。实测 486 → 96（**5.06 倍**），比 16 少一截的原因值得写下来：
        // 边长里那个固定的 `+2` 余量在小物体上占了大头 ——
        //   级 0：`ceil(2.0/0.309) + 2 = 9` 页边长
        //   级 2：`ceil(2.0/1.2375) + 2 = 4` 页边长
        // 真正"装得下物体"的是 7 与 2（那才是 12 倍），余量把经济性冲淡了。
        // ⚠ 留这个 `+2` 是有意的：它防的是"块切掉物体的一角"（画面上看不出来是分配错了）。
        assert!(
            coarse.len() * 4 < fine.len(),
            "粗级的页数 {} 该显著少于细级 {}",
            coarse.len(),
            fine.len()
        );
        // 粗级的页格坐标必须在粗级的格子里（`pps >> 2` 以内）。
        let coarse_pps = pages_at_level(light.pages_per_side, 2);
        for patch in &coarse {
            assert!(patch.page_x < coarse_pps && patch.page_y < coarse_pps);
        }

        // 两级混着，rank 仍然说得回每一页的槽位。
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

    /// 表里的"基址 + 掩码"说得出一页的物理槽位 —— 采样侧 rank 的定点对照。
    #[test]
    fn the_row_mask_and_base_reproduce_every_slot() {
        let allocation = one(at(4.95, 1.75, 256.0));
        let light = &allocation.lights[0];
        let base = light.table_offset;
        let pps = light.pages_per_side;
        for patch in &allocation.patches {
            // ⚠ 现在每页自己带**级**：行段、掩码字数、页数边长都跟着这一页的级走。
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

    /// 面映射的六条：**朝向照 `CUBE_MAP_FACES`**（`0:+X 1:−X 2:+Y 3:−Y 4:−Z 5:+Z`），
    /// 而 `(u, v)` 是**面内归一化坐标**（`v` 向上为正，与 NDC y 同向）。
    ///
    /// ⚠ 这六条**不再手写**：它们是从 `px_protocol::scene::SHADOW_FACE_BASIS` 算出来的
    /// （用户裁决：这条契约只留一处定义）。所以这条判据钉的是"那份表确实按
    /// Bevy 的 `looking_at` 展开"—— 数值在这里现算，不抄结论。
    ///
    /// ⚠ 第 4/5 面**从前是反的**（那时这里写着 `+Z` 在 4 号），而渲染器在 4 号画的是 −Z
    /// ⇒ 页分到了"不是渲染器画的那一面"上。
    #[test]
    fn the_face_map_matches_the_cube_order() {
        let dir = [3.0, 0.4, 0.2];
        // 轴的次序：`+X −X +Y −Y −Z +Z`。
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
            // `(u, v)` = 两条面内基向量点乘方向，再除以轴点乘 —— 与着色器同一条算式。
            let dot = |v: [f32; 3]| v[0] * dir[0] + v[1] * dir[1] + v[2] * dir[2];
            let denom = dot(basis[2]);
            assert_eq!(
                face_uv(dir, face as u32),
                (dot(basis[0]) / denom, dot(basis[1]) / denom),
                "第 {face} 面的 `(u, v)`"
            );
        }
        // 逐条对一遍**实数值**（`v` 向上为正），防"算式对了但表填错"：
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
        // 第 4 面（−Z）：right = +X、up2 = +Y ⇒ 世界 +x 给 u=+1、世界 +y 给 v=+1。
        assert_eq!(face_uv([0.2, 0.4, -3.0], 4), (0.2 / 3.0, 0.4 / 3.0));
        // 第 5 面（+Z）：right = −X、up2 = +Y ⇒ 世界 +x 给 u=−1。
        assert_eq!(face_uv([0.2, 0.4, 3.0], 5), (-0.2 / 3.0, 0.4 / 3.0));
    }

    /// **y 的翻转只发生一次**：`face_uv` 的 `v` 向上为正，而 atlas 行号向下增。
    #[test]
    fn the_atlas_row_grows_downwards() {
        let side = 128.0;
        // 面正中央 → texel 正中央。
        assert_eq!(face_texel((0.0, 0.0), side), (64.0, 64.0));
        // 往上（`v = +1`）⇒ 行号**小**（atlas 顶行）；往右 ⇒ 列号大。
        assert_eq!(face_texel((1.0, 1.0), side), (128.0, 0.0));
        assert_eq!(face_texel((-1.0, -1.0), side), (0.0, 128.0));
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
