//! 运输动作（[`HaulStep`]）：一条运输路线的**这一步做了什么**。
//!
//! 它由 `sim::haul_step` 产生（引擎内部用它做守卫与观察），同时**也是读面的一部分**
//! （`view.haul_steps`，一轮一行/舰）——所以它住在这里（`model`）而不是 `sim`：
//! `model` 不许依赖 `sim`，而读面类型必须能被 `RoundView` 引用。定义**只有这一份**，
//! 引擎与读面共用同一组变体名（`#[serde(tag = "step")]` 的判别式 = [`HaulStep::step`]）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{BodyId, ShipId};

/// 一条运输路线（[`crate::model::ShipBehavior::Haul`]）本回合**做了什么**。
///
/// 为什么它必须被捕获：`Waiting` 与 `EnRoute` **既不落 `State` 也不发事件**——「我派它去拉货，
/// 它为什么一件没运回来」在 B3 之前没有任何读法（货舱是不是空的只说明「装没装上」，不说明
/// 它是停在空货栈干等、还是在路上）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum HaulStep {
    /// 在 `body` 装上了 `units` 件货（Q5 A：有多少装多少，绝不空舱等待）。
    Loaded {
        /// 装货的天体（起运的产地货栈）。
        body: BodyId,
        /// 这一次装走的件数。
        units: f64,
    },
    /// 在 `body` 卸下 `units` 件货；`into_pool = true` 表示卸进了**货主首都池**（集货完成）。
    Delivered {
        /// 卸货的天体。
        body: BodyId,
        /// 这一次卸下的件数。
        units: f64,
        /// 卸的是不是**货主**的首都池（承包时货主是托运方，不是船东）：true = 这趟集货算完成。
        into_pool: bool,
    },
    /// 停在 `body` 但**货栈是空的**：原地等——「有货就走」的另一半正是「没货就不走」
    /// （空载跑一趟是白烧时间，而货栈随时会因产出再涨）。
    Waiting {
        /// 干等的地点（起运端）。
        body: BodyId,
    },
    /// 这一回合只是**在路上**，正驶向 `body`（装/卸都还没发生）。
    EnRoute {
        /// 正驶向的那一端（舱里有货 ⇒ 目的地；空舱 ⇒ 起运地）。
        body: BodyId,
    },
}

impl HaulStep {
    /// 这一步发生在哪个天体（`EnRoute` = 正驶向的那一端）——判定表与探针读它。
    pub fn body(&self) -> &str {
        match self {
            HaulStep::Loaded { body, .. }
            | HaulStep::Delivered { body, .. }
            | HaulStep::Waiting { body }
            | HaulStep::EnRoute { body } => body,
        }
    }

    /// 变体名，与 serde 的 `step` 判别式（`rename_all = "snake_case"`）**逐字一致**
    /// ——投影把它摊成一列，别在表里另写一套名字（同 `GameEvent::kind`）。
    pub fn step(&self) -> &'static str {
        match self {
            HaulStep::Loaded { .. } => "loaded",
            HaulStep::Delivered { .. } => "delivered",
            HaulStep::Waiting { .. } => "waiting",
            HaulStep::EnRoute { .. } => "en_route",
        }
    }

    /// 这一步**搬动了多少件**（`waiting` / `en_route` = 0：没搬）。
    pub fn units(&self) -> f64 {
        match self {
            HaulStep::Loaded { units, .. } | HaulStep::Delivered { units, .. } => *units,
            HaulStep::Waiting { .. } | HaulStep::EnRoute { .. } => 0.0,
        }
    }

    /// 卸货**是不是卸进了货主的首都池**（只对 `delivered` 有意义；其余变体为 `false`）。
    pub fn into_pool(&self) -> bool {
        match self {
            HaulStep::Delivered { into_pool, .. } => *into_pool,
            _ => false,
        }
    }
}

/// 读面里 `haul_steps` 的键是舰名（[`ShipId`]）——这里只是把这个形状写下来，
/// 免得读者去猜「一艘舰一回合会不会有多行」（不会：一条路线一回合只走一步）。
pub type HaulSteps = std::collections::BTreeMap<ShipId, HaulStep>;
