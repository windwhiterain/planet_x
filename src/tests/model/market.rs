//! 市场模型的单元测试。

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
            Offer {
                seller: "欧盟".into(),
                resource: "铀".into(),
                amount: 2.0,
                ask: 6.0,
            },
            Offer {
                seller: "欧盟".into(),
                resource: "铂".into(),
                amount: 1.0,
                ask: 6.0,
            },
            Offer {
                seller: "俄罗斯".into(),
                resource: "铀".into(),
                amount: 3.0,
                ask: 6.0,
            },
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
        offers: vec![Offer {
            seller: "美国".into(),
            resource: "铁".into(),
            amount: 4.0,
            ask: 1.4,
        }],
        price: [("铁".to_string(), 1.4)].into_iter().collect(),
        settled: [("铁".to_string(), 4.0)].into_iter().collect(),
        avg_demand: [("铁".to_string(), 3.0)].into_iter().collect(),
        last_stock: [("铁".to_string(), 12.0)].into_iter().collect(),
        domestic: Default::default(),
    };
    let text = ron::ser::to_string(&m).expect("serializable");
    let back: MarketState = ron::from_str(&text).expect("deserializable");
    assert_eq!(back.offers, m.offers);
    assert_eq!(back.price_of("铁", 1.0), 1.4);
    assert_eq!(back.settled.get("铁"), Some(&4.0));
    assert_eq!(back.avg_demand.get("铁"), Some(&3.0));
    assert_eq!(back.last_stock.get("铁"), Some(&12.0));
}
