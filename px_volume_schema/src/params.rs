//! 体积烘培的参数：烘哪个场、网格多密、壳的半径、阈值与归一化。
//! （算子 id 与接口形状住在同目录的 `ops.rs` 里 —— 一处定义。）
//!
//! ⚠ 「这些参数怎么变成一份参照场」不在这里 —— 那是**参照实现**的事，住在
//! `px_verify::proxy`（算子与判据仪器共用同一条映射，谁也不许自己再抄一份）。

use serde::{Deserialize, Serialize};

/// 烘进体积网格的**标量场**。
///
/// `Coarse` 是 `shape_of(cover, altitude, 1.0)`（噪声取上界），它在结构上包住真场
/// （`billows ≤ 1`），代价是落点离真表面很远 —— 步进起点因此离命中 40+ 步。
/// `Final` 就是 shader `cloud_field` 返回的那个量，落点贴着真表面。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    #[default]
    Coarse,
    Final,
}

#[derive(Debug, Clone, Serialize, Deserialize, px_derive::PxParams)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// 烘哪个场（见 `FieldKind`）。默认 `coarse` ⇒ 老配方逐位不变。
    pub field: FieldKind,
    /// 每个面 s/t 两个轴的采样点数（`res × res` 条射线）。
    pub res: u32,
    /// 径向层数。
    pub layers: u32,
    /// 壳的内外半径（世界点 = 方向 × 半径）。
    pub inner: f32,
    pub outer: f32,
    /// 硬表面的阈值 τ（就是 shader 的 `surface_level`）。
    pub tau: f32,
    /// 归一化用的梯度上界 `L`：存进体积的是 `(场 - τ) / L`。
    /// 自适应八叉树那类提取器的剪枝测试是 `|f| < size·√3`，只在 `|∇f| ≤ 1` 时不漏，
    /// 所以这个数只许大不许小（大了只费时间，小了网格出洞）。
    /// 现在的提取器是稠密 MC（插值对尺度不变）⇒ `L` 不改变 mesh，只影响存的量级。
    pub scale: f32,
    /// 保守化的邻域半径，单位是**格**（0 = 不保守，只做对照）。
    ///
    /// 点采样出来的场会漏掉节点之间的尖峰：覆盖度的细节波长只有约 2.7 格，65² 的网格
    /// 上峰值能被低估一半（实测漏掉 2/212 条方向上的整块云）。取邻域最大值 = 把覆盖度换成
    /// 它自己的膨胀 ⇒ 等值面只往外走，代价是最多外扩一格 —— 方向是安全的那一边。
    /// ⚠ 缺几何是硬失败（fragment 来自 mesh，没 mesh 就没 fragment，往回步进救不回来）。
    pub reach: u32,
    /// 云的那一档形状参数：必须与场景里 `clouds` 那一档的数一致。
    pub coverage: f32,
    pub base: f32,
    pub top: f32,
    pub taper: f32,
    pub coverage_gain: f32,
    pub erode: f32,
    pub orientation: [f32; 4],
}

impl Default for Params {
    fn default() -> Self {
        Self {
            field: FieldKind::Coarse,
            res: 65,
            layers: 65,
            inner: 1.01,
            outer: 1.06,
            tau: 0.20,
            scale: 240.0,
            reach: 1,
            coverage: 0.35,
            base: 0.06,
            top: 0.62,
            taper: 0.45,
            coverage_gain: 2.6,
            erode: 0.0,
            orientation: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl Params {
    pub fn span(&self) -> f32 {
        self.outer - self.inner
    }
}

/// `FieldKind` 是**纯局部开关**（决定粗场还是真场），但产物确实不同 ⇒ 进键。
impl px_graph_schema::HashField for FieldKind {
    fn hash_field(&self, hasher: &mut blake3::Hasher) {
        hasher.update(&[*self as u8]);
    }
}

/// **`cloud.emission` 的参数**：把密度体积变成"逐体素的发射与消光"。
///
/// ⚠ 为什么它要**单独一档**（而不是让步进每一步自己算光照）：光照里的阴影步进是
///   **逐体素**的量（"从这一点到光源之间有多少气"），与视线无关 ⇒ 算一遍就够。
///   塞进步进就等于**每条射线每一步都重算一遍**，代价是"射线数 × 步数"倍。
///
/// ⚠ 这也是"烘图时光线步进"能成立的全部理由：递归光照在烘图时按体素摊掉，渲染期不用再算。
pub mod emission {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct EmissionParams {
        /// **星光**在气体上的增益：逐体素把"附近星点的辐照"折进发射（`0` = 关）。
        ///
        /// ⚠⚠ 这一档是用户 2026-09-25 定的口径：**星不是贴在天穹上的像素，是 R3 里的点光源，
        ///   它照亮周围的气**。光晕（参考图里那圈粉晕）因此**长在气上** ——
        ///   星在浓气里晕小而亮、在空处几乎没有晕，而且天然是世界坐标、天然抗锯齿。
        ///
        /// ⚠ 与旧 `cluster_*`（4 颗程序化的、只知道方向的假光源）的关系是**替换**：
        ///   星簇现在就是星表里真实的一组星（`params::stars::StarsParams::cluster_*`），
        ///   既直射进画面、也照亮气体 ⇒ 两套光照不会打架。
        pub starlight_gain: f32,
        /// 星光辐照的**软化半径**（世界单位）：辐照取 `亮度 / (d² + soft²)`。
        ///
        /// ⚠ 它是"这一颗星的光在气里铺多开"，与 `starlight_radius`（**查哪几颗**）是两件事：
        ///   前者是物理衰减的形状，后者是搜索的截断。给 `0` = 只吃 `1/d²`（近处会炸）。
        ///
        /// ⚠⚠ 分母里那个 `soft²` 是**有量纲的常数** ⇒ `starlight_gain` 的量级被它决定
        ///   （`soft = 0.05` 时是 `1e-3` 那一档，不是 1 那一档）。给增益之前先算一遍：
        ///   典型星亮度 ~10、距离 ~0.1 ⇒ 单颗 ~10/(0.01+0.0025) = 800，
        ///   八颗之和量级 `1e3` ⇒ 增益取 `1e-3` 上下才与自发光（`d^power × 0.85`）同量级。
        pub starlight_soft: f32,
        /// **查多远**：逐体素只在以自己为心、半径 `starlight_radius` 的球里找星（世界单位）。
        ///
        /// ⚠ 它是**物理**旋钮（星光能照到多远的气），与 `stars.cell`（**存储**分辨率：
        ///   一个细格装几颗星）分开 —— 用户 2026-09-25 定的两个旋钮，别绑死。
        pub starlight_radius: f32,
        /// 逐星的阴影步数（与 `shadow_steps` 同量纲）：`N 颗候选 ⇒ 代价 ≈ N × 这个`。
        pub starlight_steps: u32,
        /// 每个体素最多吃几颗星（按"亮度 / 距离²"取前几名）。
        ///
        /// ⚠ 它是**截断**而不是物理：亮度是幂律的（少数亮星 + 大量暗星），
        ///   而辐照按 `1/d²` 掉 ⇒ 前几名之外的和本来就可以忽略。
        pub starlight_max: u32,
        /// **光源方向**（世界空间，单位向量）：星云内部那颗电离源的方位。
        ///
        /// ⚠ 参考图里"亮脊 + 暗柱"的来源就是它：朝着光源的那一侧被照亮，背光那一侧与
        ///   "被前面的气挡住"的地方是暗的。没有这一栏，雾就是均匀自发光的（第一版那样）。
        pub light: [f32; 3],
        /// 光源在壳内的**半径比例**（`0` = 球心、`1` = 外壁）。
        pub light_radius: f32,
        /// 朝光源步进多少步算遮挡。⚠ 步数越多边缘越锐（代价线性）。
        pub shadow_steps: u32,
        /// 遮挡的浓度：`exp(-τ × 这个)`。`0` = 完全不投影（处处同亮）。
        pub shadow_gain: f32,
        /// 发射 = `density^power × gain`（`power > 1` ⇒ 只有浓的地方才亮）。
        pub emission_power: f32,
        pub emission_gain: f32,
        /// 消光 = `density^power × gain`，**逐通道**（尘埃的偏红就是这么来的）。
        ///
        /// ⚠ 三个数**不是**同一个数乘系数：真实的尘埃消光随波长走（蓝光被吃得更多），
        ///   于是"被尘埃压过的区域偏红"。给成一样的话尘埃只会把东西变暗，不会变色。
        pub extinction: [f32; 3],
        pub extinction_power: f32,
        /// 尘埃的**额外**权重（叠在消光上）：参考图里那些黑柱比周围的雾浓得多。
        pub dust_bias: f32,
        pub dust_threshold: f32,
        /// **中性底光**（不吃消光的那个白蓝核）。
        ///
        /// ⚠⚠ 这一栏是为了解决一个**结构性**的问题，不是调色：
        ///   发射是**单标量**，颜色**只**来自逐通道消光 ⇒
        ///   核与边缘的**色相必然相同**（只是亮度不同），
        ///   而参考图的核是**白蓝**、边缘是**玫红** —— 单标量发射**给不出**这个。
        ///   实测参考图的亮度分档（线性均值）：
        ///
        ///   | | B/R | G/R |
        ///   |---|---|---|
        ///   | 最亮 15% | 1.10 | **0.81**（核：绿不低 ⇒ 中性偏蓝） |
        ///   | 最暗 50% | 0.61 | **0.24**（边缘：绿被压死 ⇒ 玫红） |
        ///
        ///   把 `σ_G` 调大能让边缘变玫红，但核也跟着变紫 —— 这是前几轮"要么全灰、
        ///   要么全紫"的根源。
        ///
        /// ⚠ 做法：叠一份 `density^glow_power × glow_gain`。
        ///   它的 `power` 取得**低**（≈1）⇒ 它在**浓处**相对更强
        ///   （浓处 `d^1` 比 `d^5` 大得多），正好是"核里那层白蓝"。
        ///   它照样走消光（所以薄处的白光会被染红），只是它在薄处本来就很弱。
        pub glow_gain: f32,
        pub glow_power: f32,
        /// **底光的色相**（逐通道权重，`[R, G, B]`）。
        ///
        /// ⚠⚠ 这一栏是"**核白蓝、边玫红**"的关键，而它必须存在的原因是**结构性**的：
        ///   发射若是**单标量**，则 `每通道辐射 = 同一个 emit × 各通道的透过率` ——
        ///   而透过率只随**总消光**走 ⇒ **浓处与薄处的色相必然相同**（只是亮度不同）。
        ///   把 `σ_G` 调大能让薄处玫红，**浓处也跟着变紫** ⇒ 要么全灰、要么全紫。
        ///
        ///   参考图（线性均值，按亮度分档）：
        ///   | | B/R | G/R |
        ///   |---|---|---|
        ///   | 最亮 15% | 1.10 | 0.81 | ⇒ 核是**白蓝**（绿不低、蓝略高） |
        ///   | 最暗 50% | 0.61 | 0.24 | ⇒ 边缘是**玫红**（绿被压死） |
        ///
        ///   ⇒ 底光要带**自己的颜色**：给成偏蓝青（`[0.6, 1.0, 1.3]` 这种），
        ///   而它在浓处才显著（`glow_power` 低）⇒ 核里那层蓝白就出来了。
        ///   主发射仍然是中性标量，它的颜色由消光决定（薄处自然变玫红）。
        pub glow_tint: [f32; 3],
        /// **散射的通道配比**（诊断用）：把"星云散射出来的光"与"星自己的光"分开看。
        ///
        /// ⚠ 用户 2026-09-25："为了区分星星自己的光和星云散射的光，星光用蓝色、散射用红色。"
        ///   做法是**分通道**：这里的三个数乘在**发射**通道上（消光通道不动 ——
        ///   尘埃染色是物理，不该跟着诊断走），而星自己那一档的配比在
        ///   `SkyParams::star_tint`。两边取 `[1,0,0]` / `[0,0,1]` 就得到一张
        ///   **红=散射、蓝=星光** 的分色图（每通道本来就各积一遍 ⇒ 零额外成本）。
        ///   `[1,1,1]` = 不染色（正式出图那一档）。
        pub scatter_tint: [f32; 3],
        /// **底光的密度门限**：只有密度超过它的地方才吃底光。
        ///
        /// ⚠⚠ 这一栏是"**门控**"而不是"曲线"，而两者的区别是这一轮的教训：
        ///   上一轮想靠 `glow_power` 一个幂把底光"只留在浓处"，
        ///   实测**做不到** —— 幂低时薄处的 `d^1` 也不小（暗部 G/R 1.58，参考 0.24），
        ///   幂高时核的绿又不够（0.66，参考 0.81）。一个旋钮管不了两头。
        ///
        ///   ⇒ 门控是**按位置挑**（密度不到就没有），曲线是**按数值压**（处处都有、只是大小不同）。
        ///   星云的暗部与核是**两种地方**，所以要用门控。
        ///   （同一条教训在上一轮的高频细丝上也出现过：按位置挑比按数值压有效。）
        ///
        /// 取法：`max(0, d - 门限)^glow_power`。门限以下**正好是 0**，
        /// 于是暗部完全交回"主发射 + 逐通道消光"（玫红）。
        /// ⚠ 默认 `0.0` ⇒ 与"没有门限"逐字相同（老行为不变）。
        pub glow_threshold: f32,
    }

    impl Default for EmissionParams {
        fn default() -> Self {
            Self {
                light: [0.3, 0.5, 0.8],
                light_radius: 0.25,
                starlight_gain: 0.0,
                starlight_soft: 0.05,
                starlight_radius: 0.20,
                starlight_steps: 12,
                starlight_max: 8,
                shadow_steps: 24,
                shadow_gain: 1.6,
                emission_power: 2.2,
                emission_gain: 1.0,
                glow_gain: 0.0,
                glow_power: 1.0,
                glow_tint: [1.0, 1.0, 1.0],
                scatter_tint: [1.0, 1.0, 1.0],
                glow_threshold: 0.0,
                // 蓝吃得比红多 ⇒ 透过尘埃的光偏红（星云照片里那条"红化"）。
                extinction: [1.6, 2.4, 3.4],
                extinction_power: 1.0,
                dust_bias: 2.0,
                dust_threshold: 0.35,
            }
        }
    }
}

/// **`sky.nebula` 的参数**：沿视线积分，出一张立方贴图。
///
/// ⚠ 这一档**只积分**（不造形状、不投影）：它的输入是 [`super::emission`] 出来的
///   "逐体素发射 + 消光"，输出是一张立方贴图 —— 于是它可以直接当天空盒用。
pub mod sky {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct SkyParams {
        /// 立方贴图每个面的分辨率（输出是 `face × face × 6`）。
        pub face: u32,
        /// 每条射线的步数（**含**抖动）。⚠ 给少了会出层状条纹。
        pub steps: u32,
        /// 抖动：按格给采样点加一个只与格子有关的伪随机偏移，把层状条纹打散成噪声。
        ///
        /// ⚠ 它是**确定性**的（同一个格子永远同一个偏移）⇒ 重烘逐字节相同，缓存键不受影响。
        ///   用时间/随机数做抖动会让同一份参数烘出两张不同的图。
        pub jitter: f32,
        /// 星点的亮度倍率（星点**乘**透射率 ⇒ 被前面的气遮住、被尘埃染红）。
        pub star_gain: f32,
        /// **星光自己的通道配比**（诊断用，与 `EmissionParams::scatter_tint` 成对）。
        ///
        /// ⚠ 用户 2026-09-25："星光用蓝色、散射用红色。" 两边一起取
        ///   `star_tint = [0,0,1]` + `scatter_tint = [1,0,0]` ⇒ 一张
        ///   **红=星云散射、蓝=星光本身** 的分色图（每通道各积一遍 ⇒ 零额外成本）。
        ///   `[1,1,1]` = 不染色。
        pub star_tint: [f32; 3],
        /// 星点的**核半径**（弧度，世界空间的角半径）—— 就是"这颗星多大"。
        ///
        /// ⚠⚠ 它是**弧度**而不是"几个纹素"（2026-09-25 改）：按纹素给等于让**存储网格的
        ///   斜度**决定世界空间里一个点的大小 —— 而立方图的纹素角跨一个面差 3 倍
        ///   （实测：面心的星核 σ≈1.0 纹素、面角 1.6 纹素，即"圆被存成椭圆"）。
        ///   给弧度之后，横向形状与位置无关；采样要多细是**输出面**自己的事。
        pub star_core: f32, // 世界长度（不是弧度：角尺寸 = 它/距离 ⇒ 近大远小）
        /// 星点的**外晕半径**（弧度）与它的权重（参考图里那圈粉晕）。
        ///
        /// ⚠ 这一档只是"星自己的晕"；**被星照亮的气**那份晕住在 `cloud.emission`
        ///   （`starlight_*`），两者是两件事：前者是点光源本身的光学弥散，
        ///   后者是气被照亮 —— 后者天然有形状（气浓处亮），前者是各向同性的。
        pub star_halo: f32, // 世界长度
        pub star_halo_gain: f32,
        /// 背景天空的底色（通常是近黑）。
        pub background: [f32; 3],
    }

    impl Default for SkyParams {
        fn default() -> Self {
            Self {
                face: 256,
                steps: 96,
                jitter: 1.0,
                star_gain: 1.0,
                star_tint: [1.0, 1.0, 1.0],
                // 面 1024 的面心纹素角是 2/1024 ⇒ 核给 1.5 纹素 ≈ 2.9e-3 rad：
                // 面角处纹素角只小 3 倍 ⇒ 到处都是"至少一个纹素宽"，不会漏采样。
                star_core: 0.0029,
                star_halo: 0.010,
                star_halo_gain: 0.035,
                background: [0.0, 0.0, 0.0],
            }
        }
    }
}

/// **`sky.stars` 的参数**：世界坐标里的一批点光源（R3 星场）。
///
/// ⚠⚠ 它**不是一张场**（输出是 `StarField` 载荷）：星是**点**，既没有"近精远粗"，
///   也没有"方向参数化"这回事 ⇒ 世界坐标里按体积撒点 + 一个均匀三维格当搜索结构。
///   它出的东西有两个消费者，各取一半：
///     * `cloud.emission`：拿"哪几颗星在附近 + 多亮"去**照亮气体**（在散射那一条）；
///     * `sky.nebula`：沿视线查格，把星自己按**角半径**（弧度）画成一个点。
pub mod stars {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct StarsParams {
        /// 星数（撒在壳内的**体积**里）。
        pub count: u32,
        pub seed: u32,
        /// 星所在的那一层壳（世界单位）。
        ///
        /// ⚠ 与 `cloud.density` 的 `inner` / `outer` **刻意一致**：星不是贴在天穹上的像素，
        ///   是**混在气里的点光源** —— 近处的星不被整层气遮住、远处的被前面的气吃掉，
        ///   这就是"星嵌在星云里"的那件事。两处不一致的症状只是"星看起来飘在雾外面"。
        pub inner: f32,
        pub outer: f32,
        /// **存储分辨率**：统一稀疏格的细格边长（世界单位）。
        ///
        /// ⚠⚠ 它与 `light_radius`（物理查询半径）是**两个旋钮**（用户 2026-09-25 定的）：
        ///   这个决定"一个细格装几颗星"（越小越省逐格的下探、块也越多），
        ///   那个决定"星光能照多远"。绑死会在"想细一点"和"想照远一点"之间二选一。
        pub cell: f32,
        /// 亮度幂律：`(1 / draw)^power`（暗的多、亮的少）。
        pub brightness_power: f32,
        /// 亮度的上限（幂律的尾巴很长，钳住免得一颗星打爆整幅图）。
        pub max_brightness: f32,
        /// **表观亮度**的下限：`B · (inner/r)²` 低于它的星**在生成时就剔掉**。
        ///
        /// ⚠⚠ 直接看见那一档的像素值 ∝ `B/r²`（辐照律，见 `raymarch::star_falloff`）
        ///   ⇒ 远处的暗星根本读不出来。剔除等价于**星等截断**（真实星表就是这么干的），
        ///   副产品正是"近密远疏"：`r = outer` 处阈值被 `1/r²` 收到 `inner²/outer²`。
        ///
        /// ⚠ 它只看**几何 + 亮度**、不看气（用户 2026-09-25："先不管介质"）
        ///   ⇒ 星场不必吃密度，"剔哪些"完全由星自己决定（可复现、与邻档无关）。
        ///   `0` = 不剔（与从前逐字相同的那一档）。
        pub min_apparent: f32,
        /// **星簇**：嵌在气里的那几颗亮星（参考图里明显的那一簇）。
        ///
        /// ⚠ 它们与"场的星"是同一种东西（同一张表、同一个格）⇒ 既直射、也照亮气体。
        ///   旧的 `cloud.emission::cluster_*`（4 颗只知道方向的程序化假光源）由此**退役**。
        pub cluster_count: u32,
        /// 簇心（**世界坐标的点**，不是方向）。
        pub cluster: [f32; 3],
        /// 星簇的散布半径（世界单位）。
        pub cluster_radius: f32,
        /// 星簇的亮度倍率。
        pub cluster_gain: f32,
        /// 星簇的色温色调（线性 RGB）。
        pub cluster_tint: [f32; 3],
        /// **成团**：把一部分场星按簇撒（而不是全体体积均匀）。
        ///
        /// ⚠⚠ 用户 2026-09-25 的口径是"星分布要不均匀、气要均匀" —— 变化由**星**给，
        ///   背景由**气**给。体积均匀的星场在画面上的症状正是"一层均匀的环境光"
        ///   （18 万颗暗星从所有方向把气照亮 ⇒ 没有受光面也没有背面 ⇒ 平）。
        ///   成团之后：簇内密、簇间空 ⇒ 气被**少数几处**点亮 ⇒ 才有立体感。
        ///
        /// ⚠ `0` = 全是体积均匀（与从前逐字相同）；`cluster_*`（上面那组）是**特意留的
        ///   亮星簇**，与这里的统计成团是两件事，别混。
        pub clump_count: u32,
        /// 每个簇的散布半径（世界单位，簇内按体积均匀：`u^(1/3)`）。
        pub clump_radius: f32,
        /// 归到簇里的比例（`0` = 全均匀、`1` = 全部成团）。
        pub clump_share: f32,
        /// **场星自己的颜色**（写进星表，逐星一份）。
        ///
        /// ⚠ 用户 2026-09-25："星光用蓝色、散射用红色。" 星表里的颜色同时喂**两处**：
        ///   * 直射那一档（`raymarch` 里 `power = 亮度 × tint`）⇒ 画面上的星走这个色；
        ///   * 照亮气体那一档（`bake_emission` 的 `star_lit`）⇒ 气被染成这个色，
        ///     而散射整体还要乘 `EmissionParams::glow_tint`（红）⇒ 蓝 × 红 ≈ 只剩一成。
        ///   ⚠ 想让"星光照亮的气"完整保留，就得给直射那一档单开一个通道配比
        ///     （一份新 uniform）—— 现在这条是零管道成本的版本。
        pub star_tint: [f32; 3],
        /// **大尺度**：星按这一族 fbm 的密度偏置（与气用同一个噪声 ⇒ 星云在哪、星就在哪）。
        ///
        /// ⚠ 把这几个数写成 `art/nebula/envelope.toml`（或 `blobs.toml`）那一份，
        ///   星与气的**大尺度图案就是同一个** —— 用户要的"近似"就是这么来的。
        ///   ⚠ `sky_biased = false` 时整档不生效（与从前逐字相同的那一档）。
        pub sky_biased: bool,
        pub sky_frequency: f32,
        pub sky_octaves: u32,
        pub sky_lacunarity: f32,
        pub sky_gain: f32,
        pub sky_seed: u32,
        pub sky_zonal: f32,
        /// 偏置的**锐度**：接受概率 `= 噪声^这个`（越大 ⇒ 星越挤在气的峰上）。
        pub sky_contrast: f32,
        /// **精确相关**：星按**真实的密度场**拒绝采样（体积图里 `stars` 吃 `density` 节点）。
        ///
        /// ⚠⚠ 用户 2026-09-25 定的："让星星和星云在大尺度上分布近似" —— 实测只抄
        ///   `envelope` 那一层噪声时相关只有 **+0.022**（气的分布是整条链的阈值/mix 定的），
        ///   所以必须吃**密度场本身**。
        /// ⚠ 接受概率 `= (密度 / 平均密度)^gas_contrast`，**上限 1**（拒绝采样只能变稀，
        ///   不能把密集处再加浓）⇒ 气的峰上保持原样、别处按幂律稀下去。
        ///   `gas_biased = false` 时整档不生效（与从前逐字相同）。
        pub gas_biased: bool,
        pub gas_contrast: f32,
    }

    impl Default for StarsParams {
        fn default() -> Self {
            Self {
                count: 180_000,
                seed: 60613,
                inner: 1.0,
                outer: 3.0,
                cell: 0.05,
                brightness_power: 1.1,
                max_brightness: 64.0,
                min_apparent: 0.0,
                clump_count: 0,
                clump_radius: 0.25,
                clump_share: 0.0,
                star_tint: [1.0, 1.0, 1.0],
                sky_biased: false,
                sky_frequency: 0.35,
                sky_octaves: 3,
                sky_lacunarity: 2.0,
                sky_gain: 0.5,
                sky_seed: 9173,
                sky_zonal: 0.9,
                sky_contrast: 2.0,
                gas_biased: false,
                gas_contrast: 1.5,
                cluster_count: 4,
                // 与旧版 `emission.toml` 的 `cluster = [0.25, 0.35, 0.90]`（方向）
                // 同一个方位，落在半径 2.0 处 ⇒ 世界点 ≈ 方向 × 2.0。
                cluster: [0.50, 0.70, 1.80],
                cluster_radius: 0.35,
                cluster_gain: 1.2,
                cluster_tint: [0.72, 0.86, 1.0],
            }
        }
    }
}

///
/// ⚠ 它与顶上的 [`Params`]（`cloud.coarse`）**不是同一件事**，所以各留一份：
///   `cloud.coarse` 吃"覆盖度场 + 云的形状参数"，烘的是**等值面提取**要的那张场
///   （存 `(场 - τ) / L`，服务于网格）；这一档吃**一张三维密度场**，烘的是**体渲染**
///   要的密度（存密度本身，服务于沿视线的积分）。两者的"值"含义不同 ⇒ 参数表也不同
///   （这一档没有 `tau` / `scale` 那些给等值面用的栏，多了半径与径向的保守化）。
pub mod density {
    use serde::{Deserialize, Serialize};

    /// 一张三维场 → 一份可步进的体网格（立方球参数空间）。
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, px_derive::PxParams)]
    #[serde(default, deny_unknown_fields)]
    pub struct DensityParams {
        /// **面内分辨率占画布宽度的比例**（`1.0` = 与画布同细）。
        ///
        /// ⚠ 这里**故意不给绝对 `res`**：上游那张三维场是照画布造的，两边差一个绝对数就
        ///   永远对不上。给比例则可以"画布细、体积粗"（体积按体素坐标三线性读那张场）。
        pub res_ratio: f32,
        /// **径向层数**（**独立**于面内分辨率）。
        ///
        /// ⚠⚠ 它与 `res_ratio` 分开是**必须**的，不是留白：格数是 `res × res × layers` 级
        ///   ⇒ 层数一旦跟着面内分辨率走就是 `res³`。实测把两者绑死（`layers = res/2`）时
        ///   `--face 128` **烘不完**（10 分钟超时）、`--face 256` **分配 50 GB 失败**。
        ///   星云要的是"角向细节 + 适中的径向分层"：丝与星点都在角向上，径向给 64~96 层
        ///   已经够（径向的细结构另有 `reach` 保守化兜底）。
        pub layers: u32,
        /// 壳的内外半径（世界点 = 方向 × 半径）。
        ///
        /// ⚠ 它**只在这一档有定义**：上游那张三维场活在体素坐标 `(s, t, altitude)` 里、
        ///   不知道世界尺度；"这张网格摆在世界的哪"是**这一档**的事。步进要的是世界里
        ///   的一段区间，所以半径必须在这里钉下来。
        pub inner: f32,
        pub outer: f32,
        /// **径向的保守化**（单位：格，`0` = 不保守）。
        ///
        /// ⚠ 为什么径向要有它、而面内不要：步进是**沿视线**走的，而视线在参数空间里主要
        ///   沿径向推进 ⇒ 面内那一维被三线性插值平滑掉了，径向的细结构才会**整根跨过**。
        ///   保守化 = 取"这一格与它里外各一层"的最大值 ⇒ 密度只增不减（视觉上边界往外长，
        ///   方向是安全的那一边）。给 1 就够；给 0 是"我要看原样"那一档。
        pub reach: u32,
    }

    impl Default for DensityParams {
        fn default() -> Self {
            Self {
                res_ratio: 1.0,
                layers: 64,
                inner: 1.0,
                outer: 1.6,
                reach: 1,
            }
        }
    }

    impl DensityParams {
        /// 壳的厚度。⚠ 步进要按它算步长，所以两处必须是同一个数。
        pub fn span(&self) -> f32 {
            self.outer - self.inner
        }

        /// 这份参数 + 画布宽度 ⇒ 体网格的形状 `(res, layers)`。
        pub fn shape_of(&self, canvas_width: u32) -> (u32, u32) {
            let res = ((canvas_width as f32 * self.res_ratio).round() as u32).max(2);
            let layers = self.layers.max(2);
            (res, layers)
        }
    }
}
