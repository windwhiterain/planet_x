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
        let governance_cost = sink.governance.get(&fid).map(|g| g.total).unwrap_or(0.0);
        let governance_coverage = sink.governance.get(&fid).map(|g| g.coverage).unwrap_or(1.0);
        // 「谁不卖给你」：有多少势力对本势力**全面禁运**（本回合市场结算的实际判据）。
        let trade_blocked_by = state
            .factions
            .iter()
            .filter(|o| trade_blocked(state, config, &o.name, &fid))
            .count();
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
                upkeep: sink.upkeep.get(&fid).copied().unwrap_or(0.0),
                governance_cost,
                governance_coverage,
                freight_paid: sink.market_freight.get(&fid).copied().unwrap_or(0.0),
                carrier_income: sink.market_carrier_income.get(&fid).copied().unwrap_or(0.0),
                net_import: sink.market_net.get(&fid).copied().unwrap_or(0.0),
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
        cities.insert(
            c.name.clone(),
            CityRow {
                population: c.population,
                loyalty: c.loyalty,
                production_value,
                production,
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
        // 本回合 AI 的判定流水（`pre` 里为空 = 这一回合还没掷）。
        decisions: sink.decisions.clone(),
    }
}
