use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{BuildingId, GameConfig, ResourceMap};

/// A single continuous-area building allocation on a city. Not an atom:
/// `area` is the planned extent and `deployed` is how much is actually built.
///
/// `kind` is a config key (open-ended): `"residential"` (居住区),
/// `"mining"` (开采区) or `"construction"` (建造区). The district's
/// characteristic attributes live here next to the kind:
///   * a mining building (`kind == "mining"`) carries the mined resource key
///     in `resource`;
///   * a shipyard building (`kind == "construction"`) carries the ship class it
///     produces in `ship_type` (a command-controlled attribute);
///   * every building carries a `structure` ("concrete" | "steel"), the
///     building's own attribute controlling its hardness/armor and cost.
///
/// Buildings are identified by a stable [`BuildingId`] so a city can hold
/// several shipyards (one per `ship_type`). The command-controlled investment
/// weights live in [`ControllableState`] (keyed by [`InvestKey`]/[`BuildKey`]).
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct Building {
    #[serde(rename = "建筑编号")]
    pub id: BuildingId,
    #[serde(rename = "类型")]
    pub kind: String,
    /// Mined resource key, for a mining (开采区) building.
    #[serde(rename = "开采资源")]
    pub resource: Option<String>,
    /// Ship class produced, for a shipyard (建造区) building.
    #[serde(rename = "建造舰级")]
    pub ship_type: Option<String>,
    /// 本建造区的**设计图**（舰船出厂规格，在所属势力的
    /// [`ControllableState::blueprints`](crate::model::ControllableState::blueprints) 里按名字查）。
    ///
    /// * `None` = 没有图 ⇒ 走 `ship_type` + [`crate::autocontrol::choose_loadout`]
    ///   （**与设计图落地之前逐字节一致**）；
    /// * `Some(名)` 且图存在 ⇒ 下水时把图印成舰（口径 A：图的 `class` 必须 == `ship_type`）；
    /// * `Some(名)` 但图**不存在**（悬空指针：玩家改名了/删了）⇒ **这个建造区停产**
    ///   （进度不再增加）+ `--apply` 提到它时报 `no_such_blueprint` + 读面**原样输出**这个指针
    ///   （用户裁决 Q10(a)：静默回落到生成器 = 「失败看起来像成功」）。
    #[serde(default)]
    #[serde(rename = "设计图")]
    pub blueprint: Option<crate::model::BlueprintId>,
    /// Building's own structure attribute: "concrete" 混凝土 | "steel" 钢结构.
    #[serde(rename = "结构")]
    pub structure: String,
    #[serde(rename = "面积")]
    pub area: f64,
    #[serde(rename = "已建成面积")]
    pub deployed: f64,
    /// 当前硬度（护甲）。上限趋近 `已展开面积 × armor_per_area`。
    #[serde(rename = "护甲")]
    pub armor: f64,
}

impl Building {
    pub fn under_construction(&self) -> bool {
        self.deployed < self.area - 1e-9
    }

    /// Is this building a shipyard (建造区)?
    pub fn is_shipyard(&self) -> bool {
        self.kind == "construction"
    }

    /// The maximum hardness for the currently deployed area, using the config's
    /// per-structure armor-per-area table.
    pub fn armor_max(&self, config: &GameConfig) -> f64 {
        self.deployed * config.structure_spec(&self.structure).armor_per_area
    }
}
/// Building statistics for a building kind. Loaded from `config/game.ron`.
///
/// `role` identifies the mechanical behaviour in the simulation and is one of
/// `"housing"` (provides population capacity), `"mining"` (extracts the
/// building's resource) or `"shipyard"` (builds ships).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BuildingSpec {
    pub label: String,
    pub role: String,
    /// 建设速度 (base area this kind can be built per round).
    pub construction_speed: f64,
    /// 建设各类资源 (resources consumed per unit area).
    pub build_cost: ResourceMap,
    /// 员工需求 (population required per unit area, 人口/面积).
    pub staff_per_area: f64,
    /// 幸福度/生产效率修正 (productivity multiplier).
    pub productivity: f64,
    /// Default 建设投资权重 for buildings of this kind.
    pub default_invest_weight: f64,
    /// Default 建造投资权重 for shipyard (建造区) buildings of this kind.
    pub default_build_weight: f64,
}
