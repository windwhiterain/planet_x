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

/// **`cloud.density` 的参数**：体网格多粗、壳摆在哪、要不要保守化。
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
        /// 每个面 `s` / `t` 两个轴的采样点数（`res × res` 个格）。
        ///
        /// ⚠ 它就是上游体网格场的 `width`：两边必须一致（上游场是照这张画布造的）。
        pub res: u32,
        /// 径向层数。⚠ 同样要与上游场一致（行数 = `res² × layers × 6`）。
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
                res: 64,
                layers: 32,
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
    }
}

