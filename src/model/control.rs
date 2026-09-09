use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, BuildingId, CityId, FactionId, ShipBehavior, ShipId};


/// 沿作用域链（从具体到宽泛）取第一个显式设置的 `ControlMode`，全 `None` 则
/// 默认 [`ControlMode::Ai`]。
pub(crate) fn resolve_chain(chain: &[Option<ControlMode>]) -> ControlMode {
    chain.iter().find_map(|m| *m).unwrap_or(ControlMode::Ai)
}

impl ControlScope {
    /// 把 `other` 按节点叠加到 `self`：仅覆盖 `other` 中显式给定的节点；`global`
    /// 只有在 `other.global` 为 `Some` 时才被改写。节点值 `None` 表示「继承/清除
    /// 该层的显式指定」。
    pub fn overlay(&mut self, other: &ControlScope) {
        if other.global.is_some() {
            self.global = other.global;
        }
        self.factions.extend(other.factions.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.bodies.extend(other.bodies.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.cities.extend(other.cities.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
}
// --- 可控状态 (controllable / command-controlled state) --------------------

/// 建筑「建设投资权重」定位键：(城市, 建筑)。同一座城的每栋建筑一个值。
pub type InvestKey = (CityId, BuildingId);
/// 建造区「建造投资权重」定位键：(城市, 建造区建筑)。每座城的每个建造区一个值。
pub type BuildKey = (CityId, BuildingId);

/// 一个可控字段由「谁决定」：AI（系统每回合自动决策/改写）还是
/// Player（玩家指令，系统只读不改写）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, JsonSchema)]
pub enum ControlMode {
    /// 系统自动决策（现有行为）。
    #[default]
    Ai,
    /// 玩家下达指令，系统只用该值。
    Player,
}

/// 一个可控叶子：值 + 由谁决定。
///
/// `mode = None` 表示未在此层显式指定，沿作用域链上溯继承
/// （舰 → 资源 → 城市 → 天体 → 势力 → 全局），全链 `None` 时默认 [`ControlMode::Ai`]。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Control<T> {
    pub value: T,
    pub mode: Option<ControlMode>,
}

impl<T> Control<T> {
    /// 一个由系统自动决策的可控值。
    pub fn ai(value: T) -> Self {
        Self { value, mode: Some(ControlMode::Ai) }
    }

    /// 一个由玩家指令决定的可控值。
    pub fn player(value: T) -> Self {
        Self { value, mode: Some(ControlMode::Player) }
    }

    /// 一个继承上层作用域的可控值。
    pub fn inherit(value: T) -> Self {
        Self { value, mode: None }
    }
}

/// 城市/天体/势力/全局 的控制作用域。这些不是 [`ControllableState`] 的字段，
/// 单独建一棵作用域树；判定可控叶子的 AI/玩家边界时沿链上溯。
///
/// `None` 表示该作用域没有显式指定，继承下一层（更宽泛）作用域。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ControlScope {
    pub global: Option<ControlMode>,
    pub factions: BTreeMap<FactionId, Option<ControlMode>>,
    pub bodies: BTreeMap<BodyId, Option<ControlMode>>,
    pub cities: BTreeMap<CityId, Option<ControlMode>>,
}

/// 单个势力的可控状态：所有「指令控制」的量的集合。
///
/// 实体演化结果（资源量、耐久、人口、防御…）不在其中；本结构体是被指令
/// 直接改写的状态。指令 = 对它的修改（diff）。
///
/// 每个叶子用 [`Control`] 包裹：值 + 谁决定它。舰/资源/建筑的粒度在各自的
/// `mode`；城市/天体/势力/全局这些更粗的作用域在 [`State::scope`]。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ControllableState {
    /// 本方各飞船的当前指令（每艘舰一个 Control）。
    pub ship_orders: BTreeMap<ShipId, Control<ShipBehavior>>,
    /// 投资预算（资源/时间）：决定拿出多少资源用于「建设（建筑）」，按各建筑
    /// 建设投资权重竞争（每资源一个 Control）。
    pub investment_budget: BTreeMap<String, Control<f64>>,
    /// 建造预算（资源/时间）：决定拿出多少资源用于「造舰」，按各建造区建造
    /// 投资权重竞争（每资源一个 Control）。
    pub construction_budget: BTreeMap<String, Control<f64>>,
    /// 本方各建筑的「建设投资权重」（每建筑一个 Control）。
    pub invest_weights: BTreeMap<InvestKey, Control<f64>>,
    /// 本方各建造区的「建造投资权重」（每建造区一个 Control）。
    pub build_weights: BTreeMap<BuildKey, Control<f64>>,
    /// 本方各城的「娱乐/福利预算」（每城一个 Control，市场价值/回合）：把资源投入
    /// 城市娱乐以提升忠诚度。这是「枪支与黄油」的现实权衡——花钱安抚居民，就少了
    /// 建设与造舰的预算；玩家/agent 用它来稳固对大/远城市的统治（见治理模型）。
    #[serde(default)]
    pub loyalty_budget: BTreeMap<CityId, Control<f64>>,
    /// 迁都（首都被命控制）：本势力当前希望的首都天体。`mode=Player` 时玩家说了算、
    /// 系统不改写（除非首都亡城——硬规则先于一切）；`mode=Ai`/`None` 时由 sim 的周期
    /// 迁都步骤决定。`None` 表示未命令控制（回落到 [`default_capital_body`] 兜底）。
    /// 「有效首都」的唯一事实来源就是这里，用 [`State::capital_body`] 解析——无 shadow
    /// 双状态。
    #[serde(default)]
    pub capital: Option<Control<BodyId>>,
}
