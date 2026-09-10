//! 回合派生汇总：`balance_picture` 与 `observe`（单一权威观测）。

use super::*;

/// 合纵连横格局快照：返回 (当前霸权(若有), 针对它的反制联盟成员, 各势力综合实力占比)。
/// 供 agent 层读取政治格局（霸权是谁、谁在联合制衡、谁是当前最强）。
pub fn balance_picture(
    state: &State,
    config: &GameConfig,
) -> (Option<FactionId>, Vec<FactionId>, BTreeMap<FactionId, f64>) {
    let powers = faction_power_share(state, config);
    let hegemon = active_coalition_hegemon(state, config);
    let members = match &hegemon {
        Some(h) => {
            let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
            let members: Vec<FactionId> = ids.into_iter().filter(|x| *x != *h).collect();
            coalition_of(state, config, h, &members)
        }
        None => Vec::new(),
    };
    (hegemon, members, powers)
}

/// **观测一次**：把「此刻的世界（`state`）+ 本回合的过程量（`sink`）」折成一份 [`RoundView`]。
///
/// 这是全部派生数据的**唯一出口**——`pre`（`sink` 为空）与 `post`（`sink` 装着本回合过程量）
/// 走的都是它，只是喂进去的 `sink` 不同，所以两个槽**永远同形**。
///
/// 这些数字**就是步进函数本身用的中间计算量**：它复用一次 `balance_picture`
/// （内部是 `faction_power_share` + `coalition_of`）、一次 `sanctioned_hegemon`
/// 与 `war_pairs`，再补上世界/各势力的城市/舰/兵力/人口/库存价值聚合。因此直接状态
/// （`State` 的实体字段）与此视图**严格同源、永不漂移**——不会像其它地方独立重算的
/// 汇总那样与模拟脱节。
///
/// `sink` 携带本回合的**过程量**（产出/维护/治理/贸易/判定，见 [`RoundSink`]）；`state` 提供
/// 观测快照。纯函数、无 RNG，同一种子完全复现；O(势力 + 舰 + 城) 一次遍历，足够在每回合
/// 轻量调用。
pub fn observe(state: &State, config: &GameConfig, sink: &RoundSink) -> RoundView {
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    let (hegemon, members, powers) = balance_picture(state, config);
    let sanctioned = sanctioned_hegemon(state, config);
    let wars: Vec<(FactionId, FactionId)> = war_pairs(state, config).into_iter().collect();

    let mut factions = BTreeMap::new();
    let mut total_population = 0u64;
    let mut world_cities = 0;
    let mut world_fleet = 0.0;
    for f in &state.factions {
        let fid = f.name.clone();
        let living: Vec<&City> = state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid && !c.razed)
            .collect();
        let city_count = living.len();
        let population: u64 = living.iter().map(|c| c.population as u64).sum();
        total_population += population;
        world_cities += city_count;
        let ships = state.ships.iter().filter(|s| s.faction_id == fid);
        let ship_count = ships.clone().count();
        let fleet_value: f64 = ships.clone().map(|s| s.hull).sum();
        world_fleet += fleet_value;
        let market_value: f64 = f.resources.iter().map(|(k, v)| v * value_of(k)).sum();
        let at_war = state
            .factions
            .iter()
            .any(|o| o.name != fid && hostile(state, config, &fid, &o.name));
        let production: ResourceMap = sink
            .faction_production
            .get(&fid)
            .cloned()
            .unwrap_or_default();
        let production_value: f64 = production.iter().map(|(k, v)| v * value_of(k)).sum();
        // 治理流：总开销 / 覆盖率 / 行政娱乐拆分 / 人口超载倍率 / 思潮惩罚。**缺省值不是这里
        // 定的**——它们来自 `model::neutral`（读面中性值的唯一声明处），这里只是把声明取来用；
        // 那两个 1.0（覆盖率、超载倍率）必须走常量，因为「1.0 = 没有账」与「0 = 能力归零」是
        // 两件事，写字面量迟早会有人改成 0。
        let gov = sink.governance.get(&fid);
        let governance_cost = gov.map(|g| g.total).unwrap_or(0.0);
        let governance_coverage = gov
            .map(|g| g.coverage)
            .unwrap_or(crate::model::neutral::value::GOVERNANCE_COVERAGE);
        let governance_admin = gov.map(|g| g.admin).unwrap_or(0.0);
        let governance_entertainment = gov.map(|g| g.entertainment).unwrap_or(0.0);
        let governance_scale = gov
            .map(|g| g.scale)
            .unwrap_or(crate::model::neutral::value::GOVERNANCE_SCALE);
        let ideology_loyalty_penalty = gov.map(|g| g.ideology_penalty).unwrap_or(0.0);
        // 首都向心项：**按势力算一次**，城行不重复它（见 `LoyaltyTarget` 的文档）。
        let capital_loyalty_bonus = gov.map(|g| g.capital_bonus).unwrap_or(0.0);
        // 「谁不卖给你、为什么」：对本势力**全面禁运**的那些势力各是什么原因（B3 把它从
        // 「计数」升级成「名单 + 三档原因」——战争/冷关系/联盟封锁的对策完全不同）。
        // 判据就是市场结算用的那一个函数（`trade_block_cause`），不在读面另编一套。
        let trade_blocked_by: BTreeMap<FactionId, String> = state
            .factions
            .iter()
            .filter(|o| o.name != fid)
            .filter_map(|o| {
                trade_block_cause(state, config, &o.name, &fid).map(|c| (o.name.clone(), c.to_string()))
            })
            .collect();
        // 钱去哪了（B2）：维护欠费与生锈，以及**实际花掉**的投资/建造预算。限额是控制面的
        // 持久叶（`control` 的 investment_budget/construction_budget），这里不重复它。
        let up = sink.upkeep.get(&fid);
        let spend = sink.spend.get(&fid);
        factions.insert(
            fid.clone(),
            FactionRow {
                // —— 观测 ——
                city_count,
                ship_count,
                fleet_value,
                population,
                market_value,
                at_war,
                trade_blocked_by,
                // —— 本回合过程（`pre` 里是 0 / 空）——
                production,
                production_value,
                upkeep: up.map(|u| u.total).unwrap_or(0.0),
                governance_cost,
                governance_coverage,
                governance_admin,
                governance_entertainment,
                governance_scale,
                ideology_loyalty_penalty,
                capital_loyalty_bonus,
                freight_paid: sink.market_freight.get(&fid).copied().unwrap_or(0.0),
                carrier_income: sink.market_carrier_income.get(&fid).copied().unwrap_or(0.0),
                net_import: sink.market_net.get(&fid).copied().unwrap_or(0.0),
                // B2：钱去哪了。0 = 付清/没锈（`pre` 里也是这两个 0——那一步还没跑）。
                upkeep_unpaid: up.map(|u| u.unpaid).unwrap_or(0.0),
                fleet_rust: up.map(|u| u.rust).unwrap_or(0.0),
                // 只列真花过的资源（稀疏 map）；限额在 `control` 的预算叶上，相减 = 没花掉的。
                investment_spent: spend.map(|s| s.investment.clone()).unwrap_or_default(),
                construction_spent: spend.map(|s| s.construction.clone()).unwrap_or_default(),
                // B3：市场里的位置（结算那一刻的购买力与名次；`pre` 里是 0 / null = 还没排队）
                // 与每一处货栈的运力账（挂单那一步算的那本）。
                purchasing_power: sink.market_power.get(&fid).copied().unwrap_or(0.0),
                market_rank: sink.market_rank.get(&fid).copied(),
                freight_gap: sink.freight_gap.get(&fid).cloned().unwrap_or_default(),
            },
        );
    }

    // 每座活城一行（观测：人口/忠诚；过程：本回合开采产出——step_production 的中间量）。
    let mut cities = BTreeMap::new();
    for c in &state.cities {
        if c.razed {
            continue;
        }
        let production = sink
            .city_production
            .get(&c.name)
            .cloned()
            .unwrap_or_default();
        let production_value = production.iter().map(|(k, v)| v * value_of(k)).sum::<f64>();
        // 产出与建造的中间量（B2）：用工系数 / 住房容量 / 是否集散地 / 每舰级造舰进度。
        // ⚠ 用工系数缺省走**具名常量 1.0**（不缺人手），不是 0——写 0 会被读成「全城没人上工」。
        let cf = sink.city_flow.get(&c.name);
        cities.insert(
            c.name.clone(),
            CityRow {
                population: c.population,
                loyalty: c.loyalty,
                production_value,
                production,
                // 本回合的忠诚目标值分项；`pre` 里是全 0 的 `LoyaltyTarget::default()`。
                loyalty_target: sink.city_loyalty.get(&c.name).cloned().unwrap_or_default(),
                labor: cf
                    .map(|f| f.labor)
                    .unwrap_or(crate::model::neutral::value::CITY_LABOR),
                housing_capacity: cf.map(|f| f.housing_capacity).unwrap_or(0.0),
                is_hub: cf.map(|f| f.is_hub).unwrap_or(false),
                build: cf.map(|f| f.build.clone()).unwrap_or_default(),
            },
        );
    }

    RoundView {
        city_count: world_cities,
        ship_count: state.ships.len(),
        fleet_value: world_fleet,
        population: total_population,
        power_share: powers,
        faction_power: faction_power(state, config),
        hegemon,
        coalition_members: members,
        sanctioned,
        wars,
        // 市场观察面：价、成交、挂单（由 step_market 当回合写入 state.market）。
        market_price: state.market.price.clone(),
        market_settled: state.market.settled.clone(),
        market_offered: offered_by_resource(&state.market),
        factions,
        cities,
        // B3：本回合的结算事实——真的成交的每一笔贸易（价格分解 + 丢货）与每艘在跑运输的舰
        // 走了哪一步。`pre` 里两者都为空（这一回合还没结算/还没跑）。
        market_trades: sink.market_trades.clone(),
        haul_steps: sink.haul_steps.clone(),
        // 本回合 AI 的判定流水（`pre` 里为空 = 这一回合还没掷）。
        decisions: sink.decisions.clone(),
    }
}
