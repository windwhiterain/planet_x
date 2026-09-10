//! 星际市场（真实交换所）的**世界状态**。
//!
//! 旧的「市场」不是市场：它是一台以常数价无限供货的自动贩卖机（见
//! `.agents/notes/trade-and-sanctions.md` 的实测基线）。这里换成真的：
//!
//! * **有卖家**——挂单记名（[`Offer::seller`]），供给来自**别人真实的产出**，不是凭空生成。
//! * **有价格**——[`MarketState::price`] 由「仓/需求」稀缺度逐回合算出（稀缺 → 高价）。
//! * **能封锁**——挂单按卖家记账，买方只能买**看得见的**挂单；禁运因此是真的
//!   （若把货先汇总进公共仓，被禁运的货会经市场转手，封锁形同虚设）。
//! * **有配给**——仓里有多少卖多少，买不到就是买不到。
//!
//! 价格与成交量是**世界状态**（随回合演化、随存档持久化），不是派生观测，所以住在
//! [`State`] 里而不是 `Derived` 里；回合的派生观测（本回合成交明细）由 `round_metrics`
//! 汇总给 agent。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{FactionId, ResourceMap};

/// 一条挂单：谁卖、卖什么、卖多少、每单位要价。
///
/// `ask` 在挂单时就定死（基价 × 挂单时的稀缺系数），所以同一回合里买方看到的是
/// **确定的价格表**，不会因为结算顺序而变——determinism 优先于撮合精度。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Offer {
    /// 卖家（势力名）。买方只能买「对自己可见」的卖家的挂单（禁运的判据在此）。
    pub seller: FactionId,
    /// 资源 key（可读中文名，见 `wysiwyg-resource-keys.md`）。
    pub resource: String,
    /// 可售数量。
    pub amount: f64,
    /// 每单位要价（市场价值/信用点）。
    pub ask: f64,
}

/// 星际市场的持久状态。
///
/// `#[serde(default)]`（在 [`crate::model::State`] 上）保证旧存档/旧 `.ron` 缺这一节时
/// 退化成「空市场」而不是加载失败。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MarketState {
    /// 本回合的挂单（每回合重建：先清空，再由各势力的当期富余挂出）。
    #[serde(default)]
    pub offers: Vec<Offer>,
    /// 本回合每种资源的**市场价**（基价 × 稀缺系数，见 `MarketConfig`）。
    /// 价格是持久状态：它逐回合向均衡演化，而不是每回合从零算。
    #[serde(default)]
    pub price: ResourceMap,
    /// 本回合每种资源的**成交量**（用于观测与价格发现的需求侧统计）。
    #[serde(default)]
    pub settled: ResourceMap,
    /// 滑窗平均需求（每回合全球消费，指数滑窗）。价格发现的 `target` 基准：
    /// 「仓里够全球用几个回合」比「绝对数量」更能表达稀缺。
    #[serde(default)]
    pub avg_demand: ResourceMap,
    /// **上一回合市场时刻**的世界总库存（按资源）。与「本回合市场时刻的库存 + 本回合产出」
    /// 相减即得**实测消费率**——价格发现不猜需求，而是量出来的。
    #[serde(default)]
    pub last_stock: ResourceMap,
}

impl MarketState {
    /// 某种资源本回合每单位的市场价（未定价时退回 `fallback`——通常是配置基价）。
    pub fn price_of(&self, resource: &str, fallback: f64) -> f64 {
        self.price.get(resource).copied().unwrap_or(fallback)
    }

    /// 本回合可见挂单里、某种资源对 `buyer` 可见的总量（禁运过滤后）。
    /// 只做统计用；真正的结算在 `sim::step_market`（那里才有 config 的禁运判据）。
    pub fn offered_amount(&self, resource: &str) -> f64 {
        self.offers
            .iter()
            .filter(|o| o.resource == resource)
            .map(|o| o.amount)
            .sum()
    }

    /// 按卖家汇总的挂单价值（观测用：谁在卖、卖了多少）。
    pub fn offered_value_by_seller(&self) -> BTreeMap<FactionId, f64> {
        let mut out: BTreeMap<FactionId, f64> = BTreeMap::new();
        for o in &self.offers {
            *out.entry(o.seller.clone()).or_insert(0.0) += o.amount * o.ask;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 空市场是惰性的：没有任何挂单、没有价格、成交量 0——`price_of` 退回基价。
    /// 这保证「世界生成不预置市场」不会让任何读者读到 NaN/0 价。
    #[test]
    fn empty_market_falls_back_to_base_price() {
        let m = MarketState::default();
        assert!(m.offers.is_empty());
        assert_eq!(m.offered_amount("氦-3"), 0.0);
        assert_eq!(m.price_of("氦-3", 4.0), 4.0);
        assert!(m.settled.is_empty());
    }

    /// 挂单记名：按卖家汇总价值，且同一资源的多条挂单会被加起来。
    #[test]
    fn offers_are_attributed_to_their_seller() {
        let m = MarketState {
            offers: vec![
                Offer { seller: "欧盟".into(), resource: "铀".into(), amount: 2.0, ask: 6.0 },
                Offer { seller: "欧盟".into(), resource: "铂".into(), amount: 1.0, ask: 6.0 },
                Offer { seller: "俄罗斯".into(), resource: "铀".into(), amount: 3.0, ask: 6.0 },
            ],
            ..Default::default()
        };
        assert_eq!(m.offered_amount("铀"), 5.0);
        let by_seller = m.offered_value_by_seller();
        assert_eq!(by_seller.get("欧盟"), Some(&18.0));
        assert_eq!(by_seller.get("俄罗斯"), Some(&18.0));
    }

    /// 市场状态随存档往返——它是世界状态，不是派生观测。
    #[test]
    fn market_state_survives_a_round_trip() {
        let m = MarketState {
            offers: vec![Offer { seller: "美国".into(), resource: "铁".into(), amount: 4.0, ask: 1.4 }],
            price: [("铁".to_string(), 1.4)].into_iter().collect(),
            settled: [("铁".to_string(), 4.0)].into_iter().collect(),
            avg_demand: [("铁".to_string(), 3.0)].into_iter().collect(),
            last_stock: [("铁".to_string(), 12.0)].into_iter().collect(),
        };
        let text = ron::ser::to_string(&m).expect("serializable");
        let back: MarketState = ron::from_str(&text).expect("deserializable");
        assert_eq!(back.offers, m.offers);
        assert_eq!(back.price_of("铁", 1.0), 1.4);
        assert_eq!(back.settled.get("铁"), Some(&4.0));
        assert_eq!(back.avg_demand.get("铁"), Some(&3.0));
        assert_eq!(back.last_stock.get("铁"), Some(&12.0));
    }
}
