//! 回合派生汇总：`balance_picture` 与 `round_metrics`（单一权威观测）。

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

/// 一局世界在某回合结束时的**总结指标**（agent 的「总结」视图，`RoundMetrics`）。
///
/// 这些数字**就是步进函数本身用的中间计算量**：它复用一次 `balance_picture`
/// （内部是 `faction_power_share` + `coalition_of`）、一次 `sanctioned_hegemon`
/// 与 `war_pairs`，再补上世界/各势力的城市/舰/兵力/人口/库存价值聚合。因此直接状态
/// （`State` 的实体字段）与此视图**严格同源、永不漂移**——不会像其它地方独立重算的
/// 汇总那样与模拟脱节。
///
/// `flow` 携带本回合的**流量**中间量（产出/维护/治理，见 [`RoundFlow`]）；`state` 提供
/// 存量/政治快照。纯函数、无 RNG，同一种子完全复现；O(势力 + 舰 + 城) 一次遍历，足够在
/// 每回合轻量调用。
pub fn round_metrics(state: &State, config: &GameConfig, flow: &RoundFlow) -> RoundMetrics {
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
        let living: Vec<&City> = state.cities.iter().filter(|c| c.faction_id == fid && !c.razed).collect();
        let city_count = living.len();
        let population: u64 = living.iter().map(|c| c.population as u64).sum();
        total_population += population;
        world_cities += city_count;
        let ships = state.ships.iter().filter(|s| s.faction_id == fid);
        let ship_count = ships.clone().count();
        let fleet_value: f64 = ships.clone().map(|s| s.hull).sum();
        world_fleet += fleet_value;
        let market_value: f64 = f.resources.iter().map(|(k, v)| v * value_of(k)).sum();
        let at_war = state.factions.iter().any(|o| o.name != fid && hostile(state, config, &fid, &o.name));
        let production: ResourceMap = flow.faction_production.get(&fid).cloned().unwrap_or_default();
        let production_value: f64 = production.iter().map(|(k, v)| v * value_of(k)).sum();
        let governance_cost = flow.governance.get(&fid).map(|g| g.total).unwrap_or(0.0);
        let governance_coverage = flow.governance.get(&fid).map(|g| g.coverage).unwrap_or(1.0);
        // 「谁不卖给你」：有多少势力对本势力**全面禁运**（本回合市场结算的实际判据）。
        let trade_blocked_by = state
            .factions
            .iter()
            .filter(|o| trade_blocked(state, config, &o.name, &fid))
            .count();
        factions.insert(
            fid.clone(),
            FactionMetrics {
                city_count,
                ship_count,
                fleet_value,
                population,
                market_value,
                at_war,
                production_value,
                production,
                upkeep: flow.upkeep.get(&fid).copied().unwrap_or(0.0),
                governance_cost,
                governance_coverage,
                trade_blocked_by,
                freight_paid: flow.market_freight.get(&fid).copied().unwrap_or(0.0),
                carrier_income: flow.market_carrier_income.get(&fid).copied().unwrap_or(0.0),
            },
        );
    }

    // 每座活城的本回合产出（step_production 的「中间量」）。
    let mut city_production = BTreeMap::new();
    for c in &state.cities {
        if c.razed {
            continue;
        }
        let production = flow.city_production.get(&c.name).cloned().unwrap_or_default();
        let production_value = production.iter().map(|(k, v)| v * value_of(k)).sum::<f64>();
        city_production.insert(
            c.name.clone(),
            CityMetrics {
                population: c.population,
                loyalty: c.loyalty,
                production_value,
                production,
            },
        );
    }

    RoundMetrics {
        cities: world_cities,
        ships: state.ships.len(),
        fleet_value: world_fleet,
        population: total_population,
        power_share: powers,
        faction_power: faction_power(state, config),
        hegemon,
        coalition_members: members,
        sanctioned,
        wars,
        factions,
        city_production,
        // 市场观察面：价、成交、挂单、每势力净进口（由 step_market 当回合写入）。
        market_price: state.market.price.clone(),
        market_settled: state.market.settled.clone(),
        market_offered: offered_by_resource(&state.market),
        market_net_import: flow.market_net.clone(),
    }
}
