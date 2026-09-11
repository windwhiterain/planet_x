//! **国内市场**：把「势力级资源预算 + 叶子权重」变成
//! 「国库投放资源 + 城市货币预算 + 城市按 recipe 竞买」。
//!
//! 设计见 `.agents/notes/domestic-market.md`。第一版默认关闭
//! （`config.domestic_market.enabled = false`），开启后 `step_construction`
//! 会把开发/建造预算交给这里，得到每城预算再交给 [`build_city`]。
//!
//! 数学上它是中央 allocator 的分布式版本：
//!
//! ```text
//! max Σ_i w_i log x_i  s.t. Σ_i a_i x_i ≤ b
//! ⇔ x_i = w_i/(p·a_i),  Σ_i a_i x_i = b
//! ```
//!
//! 市场版：国库投放 `g`，给城市拨钱 `m_i`，城市按当前价格 `p` 出需求
//! `d_i = a_i · min(u_i, m_i/(p·a_i))`，价格按超额需求逐回合更新。
//! 这里只分配**预算提货权**，不造货；真正的本地库存仍由 `site_affordable`
//! / `stock_at` 守门。

use super::*;
use std::collections::{BTreeMap, BTreeSet};

/// 一个势力、一个类别（开发/建造）的市场计划结果：每城拿到的预算额度。
#[derive(Clone, Debug, Default)]
pub struct DomesticPlan {
    /// 开发预算：城市 → 资源额度。
    pub development: BTreeMap<CityId, ResourceMap>,
    /// 建造预算：城市 → 资源额度。
    pub construction: BTreeMap<CityId, ResourceMap>,
    /// 本回合开发价格（观测/测试用）。
    pub development_prices: ResourceMap,
    /// 本回合建造价格（观测/测试用）。
    pub construction_prices: ResourceMap,
}

/// 为一个势力跑开发/建造两个国内市场，返回每城预算。
///
/// 这是纯计算 + 写回 `State::market.domestic` 的价格状态；不碰库存、不扣钱。
pub fn plan_faction(
    state: &mut State,
    config: &GameConfig,
    fid: &FactionId,
    b_dev: &ResourceMap,
    b_con: &ResourceMap,
) -> DomesticPlan {
    let mut dev_weights: BTreeMap<CityId, f64> = BTreeMap::new();
    let mut dev_demands: BTreeMap<CityId, ResourceMap> = BTreeMap::new();
    let mut con_weights: BTreeMap<CityId, f64> = BTreeMap::new();
    let mut con_demands: BTreeMap<CityId, ResourceMap> = BTreeMap::new();

    let city_ids: Vec<CityId> = state
        .cities
        .iter()
        .filter(|c| c.faction_id == *fid && !c.razed)
        .map(|c| c.name.clone())
        .collect();

    for cid in city_ids {
        let labor = labor_ratio(state, config, &cid);
        let (speed_mod, res_mod) = state
            .city_settlement(&cid)
            .map(|s| (s.construction_speed_mod, s.construction_resource_mod))
            .unwrap_or((1.0, 1.0));
        let mut dev_d: ResourceMap = ResourceMap::new();
        let mut dev_w = 0.0;
        let mut con_d: ResourceMap = ResourceMap::new();
        let mut con_w = 0.0;

        if let Some(city) = state.city(&cid) {
            for b in &city.buildings {
                // --- 开发：每个在造建筑按 per-area recipe 出需求 ---
                if b.under_construction() {
                    let spec = config.building_spec(&b.kind);
                    let per_area = per_area_cost(config, spec, res_mod, b);
                    let desired = (b.area - b.deployed)
                        .min(spec.construction_speed * speed_mod * spec.productivity * labor)
                        .max(0.0);
                    if desired > 0.0 {
                        dev_w += invest_weight(state, config, fid, &cid, b);
                        add_scaled(&mut dev_d, per_area.iter().map(|(k, v)| (k, v)), desired);
                    }
                }
                // --- 建造：每个建造区按 per-progress recipe 出需求 ---
                if b.is_shipyard() {
                    if let Some(cls) = &b.ship_type {
                        let spec = config.ship_spec(cls);
                        if spec.build_points > 1e-9 && b.deployed > 0.0 {
                            let rate = b.deployed
                                * config.building_spec("construction").productivity
                                * labor;
                            if rate > 0.0 {
                                con_w += build_weight(state, config, fid, &cid, b);
                                add_scaled(
                                    &mut con_d,
                                    spec.build_cost.iter(),
                                    rate / spec.build_points,
                                );
                            }
                        }
                    }
                }
            }
        }

        if !dev_d.is_empty() {
            dev_weights.insert(cid.clone(), dev_w);
            dev_demands.insert(cid.clone(), dev_d);
        }
        if !con_d.is_empty() {
            con_weights.insert(cid.clone(), con_w);
            con_demands.insert(cid.clone(), con_d);
        }
    }

    let old_dev_price = state
        .market
        .domestic
        .get(fid)
        .map(|m| m.development.price.clone())
        .unwrap_or_default();
    let old_con_price = state
        .market
        .domestic
        .get(fid)
        .map(|m| m.construction.price.clone())
        .unwrap_or_default();

    let dev_price0 = initial_price(config, &old_dev_price);
    let con_price0 = initial_price(config, &old_con_price);

    let money_mult = config.domestic_market.money_multiplier.max(0.0);
    let dev_base_money = base_value(config, b_dev) * money_mult;
    let con_base_money = base_value(config, b_con) * money_mult;

    let dev_money = city_money_map(
        state,
        fid,
        MoneyKind::Development,
        &dev_demands,
        &dev_weights,
        dev_base_money,
    );
    let con_money = city_money_map(
        state,
        fid,
        MoneyKind::Construction,
        &con_demands,
        &con_weights,
        con_base_money,
    );

    let (dev_alloc, dev_price, dev_unspent, dev_actual_demand) =
        clear_market(config, b_dev, &dev_money, &dev_demands, dev_price0);
    let (con_alloc, con_price, con_unspent, con_actual_demand) =
        clear_market(config, b_con, &con_money, &con_demands, con_price0);

    let entry = state.market.domestic.entry(fid.clone()).or_default();
    entry.development.price = dev_price.clone();
    entry.development.unspent = dev_unspent;
    // P2-2：记的是**按最终价、经货币约束后、配给前**的实际需求，不是原始 recipe 需求。
    entry.development.last_demand = dev_actual_demand;
    entry.construction.price = con_price.clone();
    entry.construction.unspent = con_unspent;
    entry.construction.last_demand = con_actual_demand;

    DomesticPlan {
        development: dev_alloc,
        construction: con_alloc,
        development_prices: dev_price,
        construction_prices: con_price,
    }
}

fn add_scaled<'a, I>(dst: &mut ResourceMap, src: I, scale: f64)
where
    I: IntoIterator<Item = (&'a String, &'a f64)>,
{
    if scale <= 0.0 {
        return;
    }
    for (rt, c) in src {
        if *c > 0.0 {
            *dst.entry(rt.clone()).or_insert(0.0) += *c * scale;
        }
    }
}

fn base_price(config: &GameConfig, rt: &str) -> f64 {
    config
        .resources
        .get(rt)
        .map(|r| r.value)
        .unwrap_or(1.0)
        .max(1e-9)
}

fn initial_price(config: &GameConfig, existing: &ResourceMap) -> ResourceMap {
    let mut p = ResourceMap::new();
    for rt in config.resources.keys() {
        let v = existing
            .get(rt)
            .copied()
            .filter(|x| *x > 0.0)
            .unwrap_or_else(|| base_price(config, rt));
        p.insert(rt.clone(), v);
    }
    p
}

fn clamp_price(config: &GameConfig, rt: &str, p: f64) -> f64 {
    let base = base_price(config, rt);
    p.clamp(
        base * config.domestic_market.price_floor.max(1e-6),
        base * config.domestic_market.price_ceiling.max(1e-6),
    )
}

fn dot(p: &ResourceMap, d: &ResourceMap) -> f64 {
    d.iter()
        .map(|(rt, q)| p.get(rt).copied().unwrap_or(0.0) * q)
        .sum()
}

fn base_value(config: &GameConfig, m: &ResourceMap) -> f64 {
    m.iter().map(|(rt, q)| base_price(config, rt) * q).sum()
}

/// 逐城货币预算的来源：开发 / 建造。
#[derive(Clone, Copy)]
enum MoneyKind {
    Development,
    Construction,
}

/// 给每个有需求的城市算一笔货币预算：
/// * 该城有 `Player` 归属的货币叶 ⇒ 用叶里的值；
/// * 否则用 `base_money × 该城权重 / 总权重`（没有权重时均分）。
fn city_money_map(
    state: &State,
    fid: &FactionId,
    kind: MoneyKind,
    demands: &BTreeMap<CityId, ResourceMap>,
    weights: &BTreeMap<CityId, f64>,
    base_money: f64,
) -> BTreeMap<CityId, f64> {
    let control = state.control(fid.clone());
    let total_w: f64 = weights.values().filter(|w| **w > 0.0).sum();
    let n = demands.len().max(1) as f64;
    demands
        .keys()
        .map(|cid| {
            let mode = match kind {
                MoneyKind::Development => {
                    state.development_money_control(fid.clone(), cid.clone())
                }
                MoneyKind::Construction => {
                    state.construction_money_control(fid.clone(), cid.clone())
                }
            };
            let w = weights.get(cid).copied().unwrap_or(0.0).max(0.0);
            let fallback = if total_w > 1e-9 {
                base_money * w / total_w
            } else {
                base_money / n
            };
            let value = if mode.is_player() {
                control
                    .and_then(|c| match kind {
                        MoneyKind::Development => c.development_money.get(cid),
                        MoneyKind::Construction => c.construction_money.get(cid),
                    })
                    .map(|leaf| leaf.value.max(0.0))
                    .unwrap_or(fallback)
            } else {
                fallback
            };
            (cid.clone(), value)
        })
        .collect()
}

/// 一个类别市场的价格反馈 + 配给。
///
/// * `supply`：国库本回合投放的资源预算（`g`）；
/// * `city_money`：逐城货币预算 `m_i`；
/// * `base_demands`：城市在“钱无限”时的 recipe 需求；
/// * 返回：每城预算额度、最终价格、未用额度。
fn clear_market(
    config: &GameConfig,
    supply: &ResourceMap,
    city_money: &BTreeMap<CityId, f64>,
    base_demands: &BTreeMap<CityId, ResourceMap>,
    mut price: ResourceMap,
) -> (BTreeMap<CityId, ResourceMap>, ResourceMap, ResourceMap, ResourceMap) {
    let mut alloc: BTreeMap<CityId, ResourceMap> = base_demands
        .keys()
        .map(|c| (c.clone(), ResourceMap::new()))
        .collect();
    if base_demands.is_empty() {
        return (alloc, price, supply.clone(), ResourceMap::new());
    }

    // 补全价格表：所有配置资源 + 供给/需求里出现的资源。
    let mut keys: BTreeSet<String> = config.resources.keys().cloned().collect();
    keys.extend(supply.keys().cloned());
    for d in base_demands.values() {
        keys.extend(d.keys().cloned());
    }
    for rt in &keys {
        price
            .entry(rt.clone())
            .or_insert_with(|| base_price(config, rt));
    }

    // `iterations = 0` 是合法配置：**只投放/按初始价配给一次，不更新价格**。
    let iterations = config.domestic_market.iterations;
    let damping = config.domestic_market.damping.clamp(0.0, 1.0);

    // 需求与配给的结算口径：给定价格，按城市货币预算缩放 recipe 需求。
    let demand_at = |price: &ResourceMap| -> BTreeMap<CityId, ResourceMap> {
        base_demands
            .iter()
            .map(|(cid, base)| {
                let money = city_money.get(cid).copied().unwrap_or(0.0).max(0.0);
                let mut d = base.clone();
                let cost = dot(price, &d);
                if cost > money && cost > 1e-9 {
                    let scale = (money / cost).clamp(0.0, 1.0);
                    for v in d.values_mut() {
                        *v *= scale;
                    }
                }
                (cid.clone(), d)
            })
            .collect()
    };

    let mut current = demand_at(&price);
    if iterations > 0 {
        for _ in 0..iterations {
            current = demand_at(&price);
            let mut total_demand: ResourceMap = ResourceMap::new();
            for d in current.values() {
                for (rt, q) in d {
                    if *q > 0.0 {
                        *total_demand.entry(rt.clone()).or_insert(0.0) += *q;
                    }
                }
            }
            for rt in &keys {
                let d = total_demand.get(rt).copied().unwrap_or(0.0);
                let s = supply.get(rt).copied().unwrap_or(0.0);
                let ratio = (d + 1e-9) / (s + 1e-9);
                let old = price
                    .get(rt)
                    .copied()
                    .unwrap_or_else(|| base_price(config, rt));
                let new = old * ratio.powf(damping);
                price.insert(rt.clone(), clamp_price(config, rt, new.max(1e-9)));
            }
        }
        // P2-1：价格更新之后，配给/未用额度必须按**最终价格**再算一次需求，
        // 否则返回的是新价格、配给却是旧价格下的需求。
        current = demand_at(&price);
    }

    let mut total_demand: ResourceMap = ResourceMap::new();
    for d in current.values() {
        for (rt, q) in d {
            if *q > 0.0 {
                *total_demand.entry(rt.clone()).or_insert(0.0) += *q;
            }
        }
    }

    let mut unspent: ResourceMap = ResourceMap::new();
    for (rt, s) in supply {
        let d = total_demand.get(rt).copied().unwrap_or(0.0);
        if d <= 1e-9 {
            unspent.insert(rt.clone(), *s);
            continue;
        }
        if d <= *s {
            for (cid, dem) in &current {
                let q = dem.get(rt).copied().unwrap_or(0.0);
                if q > 0.0 {
                    alloc
                        .get_mut(cid)
                        .expect("city demand key must exist")
                        .insert(rt.clone(), q);
                }
            }
            let left = (*s - d).max(0.0);
            if left > 0.0 {
                unspent.insert(rt.clone(), left);
            }
        } else {
            let scale = *s / d;
            for (cid, dem) in &current {
                let q = dem.get(rt).copied().unwrap_or(0.0) * scale;
                if q > 0.0 {
                    alloc
                        .get_mut(cid)
                        .expect("city demand key must exist")
                        .insert(rt.clone(), q);
                }
            }
        }
    }

    (alloc, price, unspent, total_demand)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GameConfig {
        crate::config::load_config()
    }

    #[test]
    fn single_resource_same_recipe_splits_by_weight() {
        let config = cfg();
        let mut supply = ResourceMap::new();
        supply.insert("铁".to_string(), 100.0);
        let mut money = BTreeMap::new();
        money.insert("A".to_string(), 25.0);
        money.insert("B".to_string(), 75.0);
        let mut demands = BTreeMap::new();
        let mut da = ResourceMap::new();
        da.insert("铁".to_string(), 999.0);
        let mut db = ResourceMap::new();
        db.insert("铁".to_string(), 999.0);
        demands.insert("A".to_string(), da);
        demands.insert("B".to_string(), db);
        let (alloc, _price, _unspent, _demand) = clear_market(
            &config,
            &supply,
            &money,
            &demands,
            initial_price(&config, &ResourceMap::new()),
        );
        let a = alloc["A"]["铁"];
        let b = alloc["B"]["铁"];
        assert!((a - 25.0).abs() < 1e-6, "A got {a}");
        assert!((b - 75.0).abs() < 1e-6, "B got {b}");
    }

    #[test]
    fn price_rises_when_demand_exceeds_supply() {
        let config = cfg();
        let mut supply = ResourceMap::new();
        supply.insert("铁".to_string(), 10.0);
        let mut money = BTreeMap::new();
        money.insert("A".to_string(), 100.0);
        let mut demands = BTreeMap::new();
        let mut d = ResourceMap::new();
        d.insert("铁".to_string(), 100.0);
        demands.insert("A".to_string(), d);
        let p0 = base_price(&config, "铁");
        let (_alloc, price, _unspent, _demand) = clear_market(
            &config,
            &supply,
            &money,
            &demands,
            initial_price(&config, &ResourceMap::new()),
        );
        assert!(price["铁"] > p0, "price should rise: {}", price["铁"]);
    }
    #[test]
    fn zero_iterations_keeps_initial_price_but_still_allocates() {
        let mut config = cfg();
        config.domestic_market.iterations = 0;
        let mut supply = ResourceMap::new();
        supply.insert("铁".to_string(), 10.0);
        let mut money = BTreeMap::new();
        money.insert("A".to_string(), 100.0);
        let mut demands = BTreeMap::new();
        let mut d = ResourceMap::new();
        d.insert("铁".to_string(), 20.0);
        demands.insert("A".to_string(), d);
        let p0 = initial_price(&config, &ResourceMap::new());
        let (alloc, price, unspent, demand) = clear_market(&config, &supply, &money, &demands, p0.clone());
        for (rt, p) in &p0 {
            assert!(
                (price.get(rt).copied().unwrap_or(0.0) - *p).abs() < 1e-12,
                "iterations=0 时价格必须保持初始值：{rt} {p} → {}",
                price[rt]
            );
        }
        assert!(
            (alloc["A"]["铁"] - 10.0).abs() < 1e-9,
            "仍然要按初始价完成一次配给（20 件需求、10 件供给 ⇒ 配 10），实为 {}",
            alloc["A"]["铁"]
        );
        assert!(unspent.get("铁").copied().unwrap_or(0.0) <= 1e-9, "供给已被需求吃满");
        assert!(
            (demand.get("铁").copied().unwrap_or(0.0) - 20.0).abs() < 1e-9,
            "last_demand 口径应是配给前的实际需求 20（不是供给配给后的 10）"
        );
    }

    #[test]
    fn plan_faction_populates_market_and_allocates_for_recipes() {
        let mut config = cfg();
        config.domestic_market.enabled = true;
        let mut state = crate::world::default_state(&config, 42);
        crate::world::pin_roles_to_war(&mut state);
        let fid = state
            .cities
            .iter()
            .find(|c| !c.razed)
            .map(|c| c.faction_id.clone())
            .expect("world has at least one city");
        let cid = state
            .cities
            .iter()
            .find(|c| c.faction_id == fid && !c.razed)
            .map(|c| c.name.clone())
            .unwrap();
        if let Some(city) = state.city_mut(&cid) {
            if let Some(b) = city.buildings.first_mut() {
                b.deployed = 0.0;
                b.area = b.area.max(1.0);
            }
            if let Some(b) = city.buildings.get_mut(1) {
                b.kind = "construction".to_string();
                b.ship_type = Some("corvette".to_string());
                b.deployed = 1.0;
                b.area = 1.0;
            }
        }
        let mut b_dev = ResourceMap::new();
        let mut b_con = ResourceMap::new();
        for rt in config.resources.keys() {
            b_dev.insert(rt.clone(), 10.0);
            b_con.insert(rt.clone(), 10.0);
        }
        let plan = plan_faction(&mut state, &config, &fid, &b_dev, &b_con);
        assert!(state.market.domestic.contains_key(&fid));
        assert!(!plan.development_prices.is_empty());
        assert!(
            plan.development
                .get(&cid)
                .map(|m| !m.is_empty())
                .unwrap_or(false)
                || plan
                    .construction
                    .get(&cid)
                    .map(|m| !m.is_empty())
                    .unwrap_or(false),
            "market should allocate at least one resource to the test city"
        );
    }
}
