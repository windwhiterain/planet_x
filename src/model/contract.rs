//! **承包市场**：托运方挂单、承运方接单（用户裁决 Q2）的持久状态。
//!
//! 设计见 `.agents/notes/freight-collection.md` 的 §4。裁决摘要：
//!
//! * **Q1(b)**——承运人**不赔货值**，砸单只掉信誉（用户：「赔钱太麻烦了，就用扣信誉来
//!   抵扣」）。连带后果：**信誉是唯一的抵押品**，所以托运方的信誉门槛是**主闸**（M4b）。
//! * **Q3**——信誉是**势力级**的（[`Faction::reputation`](crate::model::Faction::reputation)），
//!   没有舰船级/名船长信用：担责的是势力，派哪条船是它的内部事务。
//! * **Q10**——报酬是**抽成制**：[`Contract::share`]，承运人交付时从货里自留，
//!   余数进托运方首都池。游戏没有货币，抽成不需要新造一种；且报酬自动随货值缩放、
//!   **零递归**（自留的那份本来就在它手上）、**无瞬移**（不像「首都池付实物」那样让货跨星际闪现）。
//! * **Q11**——超期**只扣一次信誉，合同不作废、货不消失**：承运人照旧送到、照旧拿抽成。
//!
//! 本模块只管**数据**。挂单簿上只留「还在等人接」与「已有人接、正在履行」的单子；
//! **完成的单子不留在状态里**——挂单簿无限增长没有读者，而「这一单成交了」由当回合的
//! 事件（`contract_*`）与流水账记录。

use serde::{Deserialize, Serialize};

use crate::model::{BodyId, FactionId};

/// 一条**运输承包挂单**：`shipper` 要人把 `amount` 单位的 `resource` 从 `from` 运到 `to`，
/// 承运人凭**抽成** [`Contract::share`] 取酬（交付时从货里自留）。
///
/// 难度、门槛、撮合（M4b）**不在这里**：那些是**算出来**的（航程/货值/截止期），
/// 存进状态只会多一份会与实际漂移的副本。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Contract {
    /// 单调递增的挂单号（由 [`ContractState::next_id`] 分配）——事件与投影靠它引用这一单。
    pub id: u64,
    /// 托运方（挂单的人）。
    pub shipper: FactionId,
    /// 承运方。`None` = 还在挂单簿上等人接。
    #[serde(default)]
    pub carrier: Option<FactionId>,
    /// 货物资源 key（可读中文名，见 `wysiwyg-resource-keys.md`）。
    pub resource: String,
    /// 挂单总量（单位）。
    pub amount: f64,
    /// **已交付给托运方**的量（**不含**承运人自留的抽成，见 [`Contract::carrier_cut`]）。
    #[serde(default)]
    pub delivered: f64,
    /// 起运天体（托运方的**产地货栈**——公理「首都即集散地」下，集货腿的另一端）。
    pub from: BodyId,
    /// 目的天体（照公理 = **托运方首都**）。
    pub to: BodyId,
    /// 承运人抽成比例（挂单时定死；`0.15` = 承运人自留 15%）。定死是为了**确定性**：
    /// 同一回合里所有承运人看到的是同一张价目表，不会因结算顺序而变。
    pub share: f64,
    /// 挂单回合。
    pub posted_round: u32,
    /// 截止回合（`state.round > deadline` = 超期）。超期**只扣一次信誉**，合同继续有效（Q11）。
    pub deadline: u32,
}

impl Contract {
    /// 还差多少没送到（单位，不含抽成）。
    pub fn outstanding(&self) -> f64 {
        (self.amount - self.delivered).max(0.0)
    }

    /// 承运人交付 `units` 单位时**自留**的抽成；其余 `units - carrier_cut(units)` 进托运方首都池。
    ///
    /// 这是 Q10 的机制落点：**没有货币转移**，承运人的报酬就是它没交出去的那一部分货。
    pub fn carrier_cut(&self, units: f64) -> f64 {
        units * self.share.clamp(0.0, 1.0)
    }

    /// 这一单是否还在挂单簿上等人接。
    pub fn is_open(&self) -> bool {
        self.carrier.is_none()
    }

    /// 这一单是否已经履行完毕（量已交足）。
    pub fn is_fulfilled(&self) -> bool {
        self.outstanding() <= 1e-9
    }
}

/// 承包市场的持久状态（进 [`State`](crate::model::State)，随存档一起持久化）。
///
/// `#[serde(default)]`（在 `State` 上）保证 v9 及更早的存档/`.ron` 缺这一节时退化成
/// 「空挂单簿」而不是加载失败——那时的世界本来就没有承包这件事，**零信息损失**。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ContractState {
    /// 挂单簿：**只留未完成的单子**（等人接的 + 正在履行的）。完成即移出。
    #[serde(default)]
    pub contracts: Vec<Contract>,
    /// 挂单号分配器（单调递增、永不复用；不复用是为了让历史事件里的单号永远指得准）。
    #[serde(default)]
    pub next_id: u64,
}

impl ContractState {
    /// 按单号取单。
    pub fn get(&self, id: u64) -> Option<&Contract> {
        self.contracts.iter().find(|c| c.id == id)
    }

    /// 挂出一张新单，返回分配到的单号（`next_id` 从 0 起，先取号再自增）。
    #[allow(clippy::too_many_arguments)]
    pub fn post(
        &mut self,
        shipper: FactionId,
        resource: String,
        amount: f64,
        from: BodyId,
        to: BodyId,
        share: f64,
        posted_round: u32,
        deadline: u32,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.contracts.push(Contract {
            id,
            shipper,
            carrier: None,
            resource,
            amount,
            delivered: 0.0,
            from,
            to,
            share,
            posted_round,
            deadline,
        });
        id
    }

    /// 某人是否**已经**为「这处货栈的这种货」挂过一张还没完成的单。
    pub fn has_unfinished(&self, shipper: &str, from: &str, resource: &str) -> bool {
        self.contracts
            .iter()
            .any(|c| c.shipper == shipper && c.from == from && c.resource == resource)
    }

    /// 找一张「某人挂出、**还没人接**」的单——用于把它的数量**改成此刻的实况**。
    ///
    /// 只有在 `carrier` 为空时才改得动：一旦有人接了，数量就是**承诺**，不能再动
    /// （Q11：超期也不作废，见 [`Contract::carrier_cut`] 与模块文档）。
    pub fn open_mut(&mut self, shipper: &str, from: &str, resource: &str) -> Option<&mut Contract> {
        self.contracts
            .iter_mut()
            .find(|c| c.shipper == shipper && c.from == from && c.resource == resource && c.is_open())
    }

    /// 某人**挂出且还开着**的单子（等人接的）。
    pub fn open_by(&self, shipper: &str) -> Vec<&Contract> {
        self.contracts
            .iter()
            .filter(|c| c.shipper == shipper && c.is_open())
            .collect()
    }

    /// 某人**正在履行**的单子（已经有人接了）。
    pub fn carrying_by(&self, carrier: &str) -> Vec<&Contract> {
        self.contracts
            .iter()
            .filter(|c| c.carrier.as_deref() == Some(carrier))
            .collect()
    }

    /// 移出已完成的单子（挂单簿只留未完成的）。返回移出的张数。
    pub fn retire_fulfilled(&mut self) -> usize {
        let before = self.contracts.len();
        self.contracts.retain(|c| !c.is_fulfilled());
        before - self.contracts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Contract {
        let mut cs = ContractState::default();
        cs.post(
            "中国".to_string(),
            "碳".to_string(),
            10.0,
            "金星".to_string(),
            "地球".to_string(),
            0.15,
            3,
            9,
        );
        cs.contracts[0].clone()
    }

    /// 单号单调递增且不复用（历史事件里的单号永远指得准）。
    #[test]
    fn contract_ids_are_allocated_monotonically() {
        let mut cs = ContractState::default();
        let a = cs.post("中国".into(), "碳".into(), 1.0, "金星".into(), "地球".into(), 0.1, 0, 5);
        cs.contracts.clear(); // 即使单子被移出，号也不回头
        let b = cs.post("美国".into(), "铁".into(), 1.0, "水星".into(), "火星".into(), 0.1, 1, 6);
        assert_eq!((a, b), (0, 1), "号必须单调递增、不复用");
    }

    /// **抽成 = 承运人的报酬**（Q10）：它交出 `units` 里的 `1-share`，自留 `share`。
    #[test]
    fn the_carrier_keeps_its_share_of_each_delivery() {
        let c = sample();
        assert!((c.carrier_cut(10.0) - 1.5).abs() < 1e-9, "15% 抽成 ⇒ 10 件里自留 1.5");
        assert!((10.0 - c.carrier_cut(10.0) - 8.5).abs() < 1e-9, "余数 8.5 进托运方首都池");
        // 抽成比例越界也要被夹住（配置写错不该让货凭空翻倍）。
        let mut wild = c.clone();
        wild.share = 3.0;
        assert!((wild.carrier_cut(10.0) - 10.0).abs() < 1e-9, "比例被夹到 1.0");
    }

    /// **一张一张来**：同一处货栈的同一种货，未完成期间不再挂第二张（否则挂单簿会被淹掉）。
    #[test]
    fn posting_is_one_contract_per_depot_and_resource_at_a_time() {
        let mut cs = ContractState::default();
        cs.post("中国".into(), "碳".into(), 5.0, "金星".into(), "地球".into(), 0.1, 0, 3);
        assert!(cs.has_unfinished("中国", "金星", "碳"));
        assert!(!cs.has_unfinished("中国", "金星", "铁"), "别的资源各自成单");
        assert!(!cs.has_unfinished("美国", "金星", "碳"), "别人挂的单不算我的");
        // 完成 ⇒ 移出 ⇒ 下一回合可以再挂一张。
        cs.contracts[0].delivered = 5.0;
        assert_eq!(cs.retire_fulfilled(), 1);
        assert!(!cs.has_unfinished("中国", "金星", "碳"));
    }
}
