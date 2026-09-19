//! 体积烘培的参数：烘哪个场、网格多密、壳的半径、阈值与归一化。
//!
//! ⚠ 「这些参数怎么变成一份参照场」不在这里 —— 那是**参照实现**的事，住在
//! `px_verify::proxy`（算子与判据仪器共用同一条映射，谁也不许自己再抄一份）。

use serde::{Deserialize, Serialize};

pub const CLOUD_COARSE: &str = "cloud.coarse";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// TOML 原文 → 键用的规范 JSON（`None` = 文件不存在 ⇒ 默认值）。
pub fn canonical(op_id: &str, toml_text: Option<&str>) -> Result<String, String> {
    match op_id {
        CLOUD_COARSE => Ok(px_graph_schema::canonical_params(&parse(toml_text)?)),
        other => Err(format!("px_volume_schema 不认识算子 {other}")),
    }
}

pub fn parse(toml_text: Option<&str>) -> Result<Params, String> {
    match toml_text {
        Some(text) => toml::from_str(text).map_err(|err| err.to_string()),
        None => Ok(Params::default()),
    }
}
