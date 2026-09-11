use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, Building, FactionId};

/// A city occupying one 定居点 (settlement) on a body, controlled by a faction.
/// 定居点 ↔ 城市一一对应: `settlement` is the index (into `Body::settlements`)
/// of the site this city sits on, and a settlement hosts at most one city — a
/// razed city stays on its site as a blank, re-colonizable footprint until it
/// is re-seeded. The city's area is split among a set of continuous-area
/// [`Building`]s, bounded by its settlement's `total_area`.
///
/// Ship production (`ship_progress`) is **per city**, keyed by the ship class
/// (舰型). Each 建造区 (shipyard building) contributes to its class's rate; the
/// rates of every shipyard in the city keep contributing into that city pool.
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct City {
    pub name: String,
    pub body_id: BodyId,
    /// 定居点名：本城占据的定居点（定居点 ↔ 城市 1:1，按**名字**引用）。
    pub settlement: String,
    pub faction_id: FactionId,
    /// 人口, limits production efficiency.
    pub population: u32,
    /// 忠诚度 (0..1)：城市对其统治势力的向心力。治理到位（能付清光速管理费）时
    /// 向「距离目标」恢复；欠费时下降；低于叛变阈值则爆发「离心叛乱」，城市被
    /// 夷平为空白。这让超大帝国难以维持遥远殖民地——光速治理延迟的体现。
    #[serde(default = "default_loyalty")]
    pub loyalty: f64,
    /// 被夷平为空白 (razed): no buildings / population; colonizable again.
    pub razed: bool,
    /// 城市形态：`true` = 轨道空间站（建在气态/冰巨行星的轨道上，如木星/土星/天王星/
    /// 海王星的「XX轨道空间站」「XX轨道站」）；`false` = 地面城市（建在固体天体表面）。
    /// 供前端按行星相对坐标放置模型（地面城市贴星体表面、空间站悬在更高轨道）。
    #[serde(default)]
    pub space_station: bool,
    pub buildings: Vec<Building>,
    /// 建造进度 (以城市为单位): ship class -> progress.
    pub ship_progress: BTreeMap<String, f64>,
}

fn default_loyalty() -> f64 {
    1.0
}
