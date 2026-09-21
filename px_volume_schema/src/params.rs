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

/// TOML 原文 → **类型化**的参数（图脚本拿它去跑判据仪器时用这条；`cook` 走的是
/// `O::Params` 那一份，两者同一条 `Default` 口径）。
pub fn parse(toml_text: Option<&str>) -> Result<Params, String> {
    match toml_text {
        Some(text) => toml::from_str(text).map_err(|err| err.to_string()),
        None => Ok(Params::default()),
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
                shadow_steps: 24,
                shadow_gain: 1.6,
                emission_power: 2.2,
                emission_gain: 1.0,
                glow_gain: 0.0,
                glow_power: 1.0,
                glow_tint: [1.0, 1.0, 1.0],
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
        /// 背景天空的底色（通常是近黑）。
        pub background: [f32; 3],
        /// 星图的哪一档算数（星图里亮纹素的比例很小，门限决定"有几颗星"）。
        pub star_floor: f32,
    }

    impl Default for SkyParams {
        fn default() -> Self {
            Self {
                face: 256,
                steps: 96,
                jitter: 1.0,
                star_gain: 1.0,
                background: [0.0, 0.0, 0.0],
                star_floor: 0.4,
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
