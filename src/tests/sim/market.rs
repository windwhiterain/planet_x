//! **国际市场口径**：价格发现的世界库存必须把产地货栈算进去（P0-3），
//! 否则等船的产出会被误判成消费，系统性抬高价格。

use super::*;

/// 非首都产出持续进 `depots`、首都池不动时，`avg_demand` / `price` 不应把它当消费。
///
/// 第一回合先建立 `last_stock`；第二回合只往货栈加产出、池子一格不动。
/// 若市场只数首都池，`produced` 会被整段误判成 `consumed`；按世界总库存口径应为 0。
#[test]
fn depot_production_is_not_counted_as_market_consumption() {
    let (config, mut state) = fresh_world(42);
    let fid = state
        .factions
        .first()
        .map(|f| f.name.clone())
        .expect("世界至少一个势力");

    // 隔离：只有一个势力有库存，且清空所有货栈；市场从第一回合重新起算。
    for f in state.factions.iter_mut() {
        f.resources.clear();
    }
    state.depots.clear();
    state.market = MarketState::default();
    if let Some(f) = state.faction_mut(&fid) {
        f.resources.insert("碳".to_string(), 100.0);
    }

    let mut flow0 = RoundSink::default();
    step_market(&mut state, &config, &mut flow0);
    let base_price = state
        .market
        .price
        .get("碳")
        .copied()
        .expect("市场必须给配置资源定价");
    let last = state
        .market
        .last_stock
        .get("碳")
        .copied()
        .unwrap_or(0.0);
    assert!(
        (last - 100.0).abs() < 1e-9,
        "用例前提：第一回合世界库存 = 首都池 100（实为 {last}）"
    );

    // 非首都产出进 50 件碳货栈，首都池仍只有 100；`flow` 如实记开采量。
    state.depot_add(&fid, "月球", "碳", 50.0);
    let mut flow = RoundSink::default();
    *flow
        .faction_production
        .entry(fid.clone())
        .or_default()
        .entry("碳".to_string())
        .or_insert(0.0) += 50.0;
    step_market(&mut state, &config, &mut flow);

    let demand = state
        .market
        .avg_demand
        .get("碳")
        .copied()
        .unwrap_or(0.0);
    assert!(
        demand.abs() < 1e-9,
        "货栈积压的 50 件不能被算成消费（avg_demand = {demand}）"
    );
    let price = state.market.price.get("碳").copied().unwrap_or(0.0);
    assert!(
        (price - base_price).abs() < 1e-9,
        "没有真实消费时价格应保持基价：{base_price} → {price}"
    );

    // 防空转：货栈真的增长了，这个回合也真的报了产出、且资源在市场价表里。
    assert!(
        state
            .depot(&fid, "月球")
            .and_then(|m| m.get("碳"))
            .copied()
            .unwrap_or(0.0)
            >= 50.0,
        "用例前提：这一局的 `depots` 真的增长了"
    );
    assert!(!flow.faction_production.is_empty(), "这一局真的记了开采量");
    assert!(
        config.resources.contains_key("碳") && state.market.price.contains_key("碳"),
        "用例前提：碳真的在市场价表里"
    );
}

/// **锁在货栈里的总库存不该把市场价格压到地板**（P1-4）：可售挂单为零时要走
/// `cover_floor`/高价，而不是拿世界总库存给买方画一张“货很多”的假图。
#[test]
fn reserved_stock_does_not_floor_the_price() {
    let (config, mut state) = fresh_world(42);
    let fid = state
        .factions
        .first()
        .map(|f| f.name.clone())
        .expect("世界至少一个势力");

    // 所有势力池都清空；1000 件铁只在**产地货栈**里，市场可见挂单为 0。
    for f in state.factions.iter_mut() {
        f.resources.clear();
    }
    state.depots.clear();
    state.depot_add(&fid, "月球", "铁", 1000.0);
    state.market = MarketState::default();
    // 让需求滑窗与上一回合世界库存都非零：这一回合没有真实消费，
    // 但需求不为零（否则价格本来就会回基价，测不出 P1-4）。
    state.market.last_stock.insert("铁".to_string(), 1000.0);
    state.market.avg_demand.insert("铁".to_string(), 10.0);

    let mut flow = RoundSink::default();
    step_market(&mut state, &config, &mut flow);

    let base = config
        .resources
        .get("铁")
        .map(|r| r.value)
        .expect("配置里有铁");
    let price = state
        .market
        .price
        .get("铁")
        .copied()
        .expect("市场会为配置资源定价");
    assert!(
        !state.market.offers.iter().any(|o| o.resource == "铁"),
        "用例前提：铁全部锁在储备/货栈里，可售挂单应为空"
    );
    assert!(
        price > base * 2.0,
        "可售供给为 0、需求为正时应走高稀缺价（基价 {base}，实际 {price}），\
         而不是拿 1000 件不可售库存把价格压到地板"
    );

    // 防空转：不可售库存真的存在，且它确实没有进入可售挂单。
    assert!(
        state
            .depot(&fid, "月球")
            .and_then(|m| m.get("铁"))
            .copied()
            .unwrap_or(0.0)
            >= 1000.0,
        "用例前提：货栈里真的压着 1000 件铁"
    );
}
