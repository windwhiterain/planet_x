//! **国内市场的逐城预算账本**（P0-2）：市场给每座城发的是逐城提货权，
//! `build_city` 的预算门也必须逐城看自己的 spent，不能被全势力总 spent 先到先得吞掉。

use super::*;

/// 两座条件相同的城市、同一份 recipe 需求；第一座花完自己的额度后，
/// 第二座仍必须能按自己的额度开工，且 `flow.spend` 继续记全势力实际总花费。
#[test]
fn domestic_market_keeps_a_budget_ledger_per_city() {
    let (mut config, mut state) = fresh_world(42);
    config.domestic_market.enabled = true;

    let fid = "中国".to_string();
    let cities = ["长三角", "珠三角"];
    // 只留这两座中国城市，排除其它城市把预算分配/总账搅浑。
    state
        .cities
        .retain(|c| c.faction_id != fid || cities.contains(&c.name.as_str()));

    // 把两座城都改造成「同样一座在建居住区」：同样人口、同样定居点修正、
    // 同一份 `residential` recipe ⇒ 纯预算账本成为唯一变量。
    for cid in cities {
        if let Some(c) = state.city_mut(cid) {
            c.population = 1000;
            c.ship_progress.clear();
            c.buildings.truncate(1);
            let b = &mut c.buildings[0];
            b.kind = "residential".to_string();
            b.resource = None;
            b.ship_type = None;
            b.blueprint = None;
            b.structure = "concrete".to_string();
            b.area = 10.0;
            b.deployed = 0.0;
            b.armor = 0.0;
        }
    }
    // 两座城的定居点参数也拉平（否则市场额度可能因速度/资源修正不同而错开）。
    let settlement_names: Vec<String> = cities
        .iter()
        .filter_map(|cid| state.city(cid).map(|c| c.settlement.clone()))
        .collect();
    for body in state.bodies.iter_mut() {
        for settlement in body.settlements.iter_mut() {
            if settlement_names.contains(&settlement.name) {
                settlement.total_area = 100.0;
                settlement.ecological_capacity = 10.0;
                settlement.construction_speed_mod = 1.0;
                settlement.construction_resource_mod = 1.0;
            }
        }
    }

    // 清空所有库存，只给中国足够大的同一种库存基数；预算按库存比例生成。
    for f in state.factions.iter_mut() {
        f.resources.clear();
    }
    state.depots.clear();
    if let Some(f) = state.faction_mut(&fid) {
        for rt in config.resources.keys() {
            f.resources.insert(rt.clone(), 10_000.0);
        }
    }

    let before: Vec<f64> = cities
        .iter()
        .map(|cid| state.city(cid).unwrap().buildings[0].deployed)
        .collect();
    let mut rng = Prng::new(7);
    let mut flow = RoundSink::default();
    step_construction(&mut state, &config, &mut rng, &mut flow);
    let after: Vec<f64> = cities
        .iter()
        .map(|cid| state.city(cid).unwrap().buildings[0].deployed)
        .collect();

    assert!(
        after[0] > before[0] + 1e-6,
        "第一座城该按自己的额度开工（{:.3} → {:.3}）",
        before[0],
        after[0]
    );
    assert!(
        after[1] > before[1] + 1e-6,
        "第一座城花完后，第二座城仍能按自己的额度开工（{:.3} → {:.3}）——\
         这正是全势力 spent 被两城共用时会失效的 P0-2",
        before[1],
        after[1]
    );
    let spend = flow
        .spend
        .get(&fid)
        .expect("step_construction 必须写全势力实际花费");
    assert!(
        spend.investment.values().any(|v| *v > 1e-9),
        "这一局真的发生过投资消费——守卫不能空转"
    );
}

/// **配置级 `iterations = 0`**：从 `GameConfig` 读入 0 时，`plan_faction` 仍出需求/配给，
/// 但不改 `DomesticMarketSide.price`（0 = 只投放、不更新价格）。
#[test]
fn zero_iterations_config_does_not_touch_domestic_prices() {
    let (mut config, mut state) = fresh_world(42);
    config.domestic_market.enabled = true;
    config.domestic_market.iterations = 0;

    let fid = "中国".to_string();
    let cid = "长三角".to_string();
    if let Some(c) = state.city_mut(&cid) {
        c.buildings.truncate(1);
        if let Some(b) = c.buildings.first_mut() {
            b.kind = "residential".to_string();
            b.resource = None;
            b.ship_type = None;
            b.blueprint = None;
            b.structure = "concrete".to_string();
            b.area = b.area.max(1.0);
            b.deployed = 0.0;
            b.armor = 0.0;
        }
    }

    let mut b_dev = ResourceMap::new();
    let mut b_con = ResourceMap::new();
    for rt in config.resources.keys() {
        b_dev.insert(rt.clone(), 100.0);
        b_con.insert(rt.clone(), 100.0);
    }
    let plan = plan_faction(&mut state, &config, &fid, &b_dev, &b_con);
    let side = &state.market.domestic.get(&fid).expect("国内市场已写回").development;
    for (rt, spec) in &config.resources {
        let p = side.price.get(rt).copied().unwrap_or(f64::NAN);
        assert!(
            (p - spec.value).abs() < 1e-9,
            "iterations=0 不应改开发价格：{rt} 基价 {} vs 实际 {p}",
            spec.value
        );
    }
    assert!(
        plan.development
            .get(&cid)
            .map(|m| !m.is_empty())
            .unwrap_or(false)
            || !plan.development_prices.is_empty(),
        "iterations=0 仍应用初始价完成需求/配给——守卫不能空转"
    );
}
