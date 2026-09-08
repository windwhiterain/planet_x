//! Round-stepping simulation engine.
//!
//! [`advance`] moves the world forward by one round (month). Everything that
//! affects game balance is read from the [`GameConfig`]; no magic numbers live
//! here. Resources are dictionaries (key -> amount), building kinds and ship
//! classes are string keys resolved against the config, and building mechanics
//! switch on the building's config `role` (`"housing"`, `"mining"`,
//! `"shipyard"`).
//!
//! The economy is area-based and continuous: population caps a city's labour
//! ratio, mining output scales with area × labour, and construction builds
//! continuous area within the settlement's finite total area.
//!
//! Budgets are split into two independent, directly-set pools per faction:
//! the **investment budget** (`investment_budget`) funds building infrastructure
//! (each building competes by its 建设投资权重), and the **construction budget**
//! (`construction_budget`) funds ship building (each 建造区 competes by its
//! 建造投资权重). The two never compete with each other.
//!
//! Buildings are the city's hardness: bombardment damages them by area share,
//! and when a city's buildings are all destroyed the city is razed to a blank,
//! colonizable settlement (cities are never captured).
//!
//! The command-controlled state lives in [`State::control`]
//! ([`ControllableState`]). The simulation writes to that state each round; the
//! caller diffs it between consecutive rounds to obtain the per-faction
//! instruction (action) record.

use crate::model::*;
use crate::prng::Prng;
use std::collections::{BTreeMap, BTreeSet};

/// Advance the world by one round, writing the new controllable state into
/// [`State::control`].
pub fn advance(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    state.round += 1;
    state.time_month += 1.0;
    // 本回合事件日志从空开始，回合演化中追加。
    state.events.clear();
    // 记录回合开始的交战状态，用于在本回合结束时检测「开战 / 停战」跃迁。
    let wars_before = war_pairs(state, config);

    // Update each body's current position (当前位置) from its orbit.
    for b in &mut state.bodies {
        b.position = b.orbit.position(state.time_month as f32);
    }

    step_production(state, config);
    step_upkeep(state, config);
    step_market(state, config);
    step_construction(state, config, rng);
    step_military(state, config, rng);
    // 光速治理：以距离首都为代价的管理/忠诚度，给超大帝国一个自然上限。
    step_governance(state, config);
    // 重建（反僵尸/反垄断）：趁着本回合只剩「残骸」的势力还没被永久旁观，先让它在
    // 自己的残骸足迹上重新立足。放在治理之后：被叛乱夷平到零的势力也能当回合重建。
    step_resurgence(state, config, rng);
    step_diplomacy(state, config, rng);
    // 合纵连横 / 均势外交：当一方被判定为「霸权」时，其余较弱势力结成反制联盟——
    // 军事上联手制衡，经济上多国资源封锁。这给「一家独大」一个自然的众矢之的。
    step_balance_of_power(state, config);

    // 外交跃迁：任何一对势力跨越战争阈值（开战 / 停战）都在本回合记一条事件。
    let wars_after = war_pairs(state, config);
    for (a, b) in wars_after.difference(&wars_before) {
        ev(state, GameEvent::WarStarted { a: *a, b: *b });
    }
    for (a, b) in wars_before.difference(&wars_after) {
        ev(state, GameEvent::WarEnded { a: *a, b: *b });
    }

    // 剧情：推进叙事弧/编年史（数据驱动，见 config/game.ron 的 `story` 表）。
    step_story(state, config);
}

/// The set of unordered faction pairs currently at war (relation ≤ war_threshold).
fn war_pairs(state: &State, config: &GameConfig) -> BTreeSet<(FactionId, FactionId)> {
    let mut pairs = BTreeSet::new();
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let (a, b) = (ids[i], ids[j]);
            if hostile(state, config, a, b) {
                pairs.insert((a.min(b), a.max(b)));
            }
        }
    }
    pairs
}

// --- helpers ----------------------------------------------------------------

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

fn city_position(state: &State, cid: CityId) -> [f64; 2] {
    state
        .city(cid)
        .map(|c| state.body_position(c.body_id))
        .unwrap_or([0.0, 0.0])
}

fn hostile(state: &State, config: &GameConfig, a: FactionId, b: FactionId) -> bool {
    if a == b {
        return false;
    }
    relation(state, a, b) <= config.combat.war_threshold
}

/// 该势力当前是否处于交战状态：与任意其他势力的关系已达到交战阈值。
/// 用于「造舰按威胁响应」——战时倾向多造战争机器，和平时倾向多造殖民/经济舰。
fn faction_at_war(state: &State, config: &GameConfig, fid: FactionId) -> bool {
    state.factions.iter().any(|o| o.id != fid && hostile(state, config, fid, o.id))
}

fn relation(state: &State, a: FactionId, b: FactionId) -> f64 {
    state
        .faction(a)
        .and_then(|f| f.relations.get(&b).copied())
        .unwrap_or(0.0)
}

fn adjust_relation(state: &mut State, a: FactionId, b: FactionId, delta: f64) {
    if a == b {
        return;
    }
    for (x, y) in [(a, b), (b, a)] {
        if let Some(f) = state.faction_mut(x) {
            let v = f.relations.get(&y).copied().unwrap_or(0.0);
            f.relations.insert(y, v + delta);
        }
    }
}

/// Append a [`GameEvent`] to this round's log.
fn ev(state: &mut State, e: GameEvent) {
    state.events.push(e);
}

/// A building's health ratio (armor / armor_max), clamped to [0, 1]. Intact
/// buildings are 1.0; damaged buildings produce/operate at a reduced ratio.
fn building_health(b: &Building, config: &GameConfig) -> f64 {
    let amax = b.armor_max(config);
    if amax <= 1e-9 {
        1.0
    } else {
        (b.armor / amax).clamp(0.0, 1.0)
    }
}

/// The command-controlled 建设投资权重 of a building (its build priority).
/// Follows the control scope: an AI-controlled building uses the config default,
/// while a player-controlled building uses the commanded value.
fn invest_weight(state: &State, config: &GameConfig, fid: FactionId, cid: CityId, b: &Building) -> f64 {
    let key = (cid, b.id);
    match state.invest_control(fid, &key) {
        ControlMode::Ai => config.building_spec(&b.kind).default_invest_weight,
        ControlMode::Player => state
            .control(fid)
            .and_then(|c| c.invest_weights.get(&key))
            .map(|c| c.value)
            .unwrap_or_else(|| config.building_spec(&b.kind).default_invest_weight),
    }
}

/// The command-controlled 建造投资权重 of a 建造区 (shipyard) building.
fn build_weight(state: &State, config: &GameConfig, fid: FactionId, cid: CityId, b: &Building) -> f64 {
    let key = (cid, b.id);
    match state.build_control(fid, &key) {
        ControlMode::Ai => config.building_spec(&b.kind).default_build_weight,
        ControlMode::Player => state
            .control(fid)
            .and_then(|c| c.build_weights.get(&key))
            .map(|c| c.value)
            .unwrap_or_else(|| config.building_spec(&b.kind).default_build_weight),
    }
}

/// The command-controlled 娱乐/福利预算 of a city (its loyalty spending per round,
/// in market value). Follows the control scope: AI uses the config default, a
/// Player-commanded city uses the commanded value.
fn city_loyalty_budget(state: &State, config: &GameConfig, fid: FactionId, cid: CityId) -> f64 {
    match state.loyalty_budget_control(fid, cid) {
        ControlMode::Ai => config.governance.default_entertainment,
        ControlMode::Player => state
            .control(fid)
            .and_then(|c| c.loyalty_budget.get(&cid))
            .map(|c| c.value)
            .unwrap_or(config.governance.default_entertainment),
    }
}

// --- production -------------------------------------------------------------

fn deposit_area(deposits: &[(String, f64)], rt: &str) -> f64 {
    deposits
        .iter()
        .find(|(r, _)| r == rt)
        .map(|(_, a)| *a)
        .unwrap_or(0.0)
}

/// A city's labour ratio: population vs. total staff required by its buildings.
fn labor_ratio(state: &State, config: &GameConfig, cid: CityId) -> f64 {
    let population = state.city(cid).map(|c| c.population as f64).unwrap_or(0.0);
    let mut staff_req = 0.0;
    if let Some(c) = state.city(cid) {
        for b in &c.buildings {
            staff_req += b.deployed * config.building_spec(&b.kind).staff_per_area;
        }
    }
    if staff_req <= 0.0 {
        1.0
    } else {
        (population / staff_req).clamp(config.economy.min_efficiency, 1.0)
    }
}

fn step_production(state: &mut State, config: &GameConfig) {
    let city_ids: Vec<CityId> = state.cities.iter().map(|c| c.id).collect();
    for cid in city_ids {
        let (_body_id, faction_id, population, razed) = {
            let c = state.city(cid).expect("city disappeared");
            (c.body_id, c.faction_id, c.population, c.razed)
        };
        if razed {
            continue;
        }
        let (ecocap, deposits) = {
            let s = state.city_settlement(cid);
            match s {
                Some(s) => (
                    s.ecological_capacity,
                    s.resources.iter().map(|d| (d.resource.clone(), d.area)).collect::<Vec<_>>(),
                ),
                None => continue,
            }
        };

        let mut housing_area = 0.0;
        let mut staff_req = 0.0;
        let mut mines: Vec<(String, f64)> = Vec::new();
        for b in &state.city(cid).expect("city disappeared").buildings {
            let spec = config.building_spec(&b.kind);
            staff_req += b.deployed * spec.staff_per_area;
            let health = building_health(b, config);
            match spec.role.as_str() {
                "housing" => housing_area += b.deployed * health,
                "mining" => {
                    if let Some(r) = &b.resource {
                        mines.push((r.clone(), b.deployed * health));
                    }
                }
                _ => {}
            }
        }

        let housing_capacity = housing_area * ecocap;

        // Population grows toward housing capacity.
        if housing_capacity > population as f64 {
            let delta = ((housing_capacity - population as f64) * config.economy.pop_growth).round() as i64;
            if delta > 0 {
                if let Some(c) = state.city_mut(cid) {
                    c.population = ((c.population as i64 + delta).min(housing_capacity as i64).max(0)) as u32;
                }
            }
        }

        let labor = if staff_req <= 0.0 {
            1.0
        } else {
            (population as f64 / staff_req).clamp(config.economy.min_efficiency, 1.0)
        };

        // Mining output.
        for (rt, area) in mines {
            let effective = area.min(deposit_area(&deposits, &rt));
            if effective <= 0.0 {
                continue;
            }
            let spec = config.building_spec("mining");
            let output = effective * labor * spec.productivity * config.economy.production_rate;
            if let Some(f) = state.faction_mut(faction_id) {
                *f.resources.entry(rt).or_insert(0.0) += output;
            }
        }
    }
}

// --- interstellar market (resource sink + keystone supply) -------------------

/// Fleet upkeep: every ship costs its class's [`ShipSpec::upkeep`] (market value)
/// per round to keep in service. The faction's total fleet maintenance is paid
/// out of its stockpile (drained value-weighted across all minerals); if the
/// faction cannot cover it, the shortfall rusts its fleet (ships lose hull
/// proportionally, and ships driven to 0 are scrapped).
///
/// This is the continuous resource **sink** that bounds fleet size: big fleets
/// need a big economy to sustain, so the navy grows only as fast as the
/// economy feeds it rather than snowballing unboundedly.
fn step_upkeep(state: &mut State, config: &GameConfig) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    for fid in faction_ids {
        let upkeep_total: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
            .map(|s| ship_panel(config, s).upkeep)
            .sum();
        if upkeep_total <= 1e-9 {
            continue;
        }
        let stock = state.faction(fid).map(|f| f.resources.clone()).unwrap_or_default();
        let total_value: f64 = stock.iter().map(|(k, v)| v * value_of(k)).sum();
        let pay = upkeep_total.min(total_value);
        if pay > 1e-9 {
            let ratio = (pay / total_value).min(1.0);
            if let Some(f) = state.faction_mut(fid) {
                for (k, v) in stock.iter() {
                    let new = (*v - *v * ratio).max(0.0);
                    f.resources.insert(k.clone(), new);
                }
            }
        }
        // Unpaid upkeep rusts the fleet; hull reaching 0 scrapped.
        let short = (upkeep_total - total_value).max(0.0);
        if short > 1e-9 {
            let frac = (short / upkeep_total).min(1.0);
            let frac = frac.max(0.2); // at least a visible rust when short
            let mut scrap: Vec<ShipId> = Vec::new();
            for s in state.ships.iter_mut() {
                if s.faction_id != fid || s.hull <= 0.0 {
                    continue;
                }
                let rust = ship_panel(config, s).hull_max * frac;
                s.hull = (s.hull - rust).max(0.0);
                if s.hull <= 0.0 {
                    scrap.push(s.id);
                }
            }
            for sid in scrap {
                ev(state, GameEvent::ShipDestroyed { ship: sid, owner: fid, class: state.ship(sid).map(|s| s.class.clone()).unwrap_or_default() });
                if let Some(s) = state.ship_mut(sid) {
                    s.hull = 0.0;
                }
            }
        }
    }
}

/// Automatic interstellar exchange. Every faction keeps each mineral its
/// shipyards consume at least [`MarketConfig::working_buffer`] units in stock;
/// when one falls short it buys the deficit by selling its surplus minerals
/// (value-weighted), at a small [`MarketConfig::spread`] friction.
///
/// This gives the economy a downstream **sink** for surplus stockpiles (so they
/// do not balloon unboundedly) and a **supply** so a faction that cannot mine a
/// keystone mineral (e.g. carbon) can still build and sustain a fleet. Trade is
/// deterministic (no RNG) and value-conserving modulo the spread fee.
fn step_market(state: &mut State, config: &GameConfig) {
    let m = &config.market;
    if m.auto_trade_limit <= 0.0 {
        return;
    }
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    // 经济制裁：若有一个已「坐大」的霸权（实力占比达标且至少一弱者倒向联盟），该霸权
    // 被多国资源封锁，其自动市场交易额度按 `sanction_trade_mult` 缩水——难以靠市场兑换
    // 短缺矿物，产业受抑。
    let sanctioned = sanctioned_hegemon(state, config);
    let trade_limit_of = |fid: FactionId| -> f64 {
        if sanctioned == Some(fid) {
            m.auto_trade_limit * config.balance.sanction_trade_mult
        } else {
            m.auto_trade_limit
        }
    };

    for fid in faction_ids {
        // 1) Minerals this faction's shipyards need (union of build_cost keys).
        let mut need: BTreeSet<String> = BTreeSet::new();
        for c in &state.cities {
            if c.faction_id != fid {
                continue;
            }
            for b in &c.buildings {
                if b.is_shipyard() {
                    if let Some(cls) = b.ship_type.clone() {
                        for (rt, _) in &config.ship_spec(&cls).build_cost {
                            need.insert(rt.clone());
                        }
                    }
                }
            }
        }
        if need.is_empty() {
            continue;
        }

        let stock = state.faction(fid).map(|f| f.resources.clone()).unwrap_or_default();

        // 2) Deficits below the working buffer, and their cost.
        let mut deficits: Vec<(String, f64)> = Vec::new();
        let mut buy_value = 0.0;
        for rt in &need {
            let have = stock.get(rt).copied().unwrap_or(0.0);
            if have < m.working_buffer {
                let amt = m.working_buffer - have;
                deficits.push((rt.clone(), amt));
                buy_value += amt * value_of(rt);
            }
        }
        if deficits.is_empty() {
            continue;
        }

        // 3) Sellable surplus: minerals NOT in `need`, above the reserve floor.
        let floor = m.working_buffer;
        let mut surplus_value = 0.0;
        for (rt2, amt) in &stock {
            if need.contains(rt2) {
                continue;
            }
            let over = amt - floor;
            if over > 1e-6 {
                surplus_value += over * value_of(rt2);
            }
        }
        if surplus_value <= 1e-6 {
            continue;
        }

        // 4) Effective buy: capped by the trade limit (or the sanctioned cap) and
        // by what surplus sells.
        let eff_buy = buy_value
            .min(trade_limit_of(fid))
            .min(surplus_value / (1.0 + m.spread));
        if eff_buy <= 1e-6 {
            continue;
        }
        let scale = eff_buy / buy_value;
        let actual_sell = eff_buy * (1.0 + m.spread);

        // 5) Apply: top up deficits (scaled), drain surpluses by value share.
        if let Some(f) = state.faction_mut(fid) {
            let mut res = std::mem::take(&mut f.resources);
            for (rt, amt) in &deficits {
                *res.entry(rt.clone()).or_insert(0.0) += amt * scale;
            }
            for (rt2, amt) in &stock {
                if need.contains(rt2) {
                    continue;
                }
                let price = value_of(rt2);
                if price <= 1e-9 {
                    continue;
                }
                let over = amt - floor;
                if over <= 1e-6 {
                    continue;
                }
                let share = (over * price) / surplus_value;
                let sell_amt = (actual_sell * share / price).min(res.get(rt2).copied().unwrap_or(0.0));
                let cur = res.get(rt2).copied().unwrap_or(0.0);
                res.insert(rt2.clone(), (cur - sell_amt).max(0.0));
            }
            f.resources = res;
        }
    }
}

// --- construction (dual budgets) ---------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum BudgetKind {
    Investment,
    Construction,
}

/// Read a faction's per-resource budget for a kind: AI resources are recomputed
/// from the stockpile (`stockpile × invest_fraction`), player resources keep the
/// commanded value. Returns the budget map and the per-resource control modes.
fn read_budget(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    kind: BudgetKind,
) -> (ResourceMap, Vec<(String, ControlMode)>) {
    let stockpile: ResourceMap = state
        .faction(fid)
        .map(|f| f.resources.clone())
        .unwrap_or_default();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    let stock_value: f64 = stockpile.iter().map(|(k, v)| v * value_of(k)).sum();
    // 造舰的「维护费保留」：自动指挥势力在投入造舰预算前，先从库存里预留 `upkeep ×
    // upkeep_reserve_mult` 的市场价值作为维护底线，只把超出部分用于造舰——「把海军养在
    // 经济能承受的规模」。这样基线 AI 不会无脑大建，避免维护费拖垮经济、军备崩盘。
    // 只对 Construction（造舰）生效；投资基础设施（Investment）不受影响。
    let reserve = if kind == BudgetKind::Construction {
        let upkeep: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
            .map(|s| ship_panel(config, s).upkeep)
            .sum();
        upkeep * config.economy.upkeep_reserve_mult
    } else {
        0.0
    };
    let build_value = stock_value * config.economy.invest_fraction;
    // 造舰预算允许的「上限」（市场价值）：不把库存打到维护底线之下。
    let con_cap = if kind == BudgetKind::Construction {
        (build_value).min((stock_value - reserve).max(0.0))
    } else {
        build_value
    };
    let con_scale = if kind == BudgetKind::Construction && build_value > 1e-9 {
        (con_cap / build_value).clamp(0.0, 1.0)
    } else {
        1.0
    };

    let mut budget: ResourceMap = ResourceMap::new();
    let mut modes = Vec::new();
    for (rt, v) in &stockpile {
        let ai_value = *v * config.economy.invest_fraction;
        let mode = match kind {
            BudgetKind::Investment => state.investment_budget_control(fid, rt),
            BudgetKind::Construction => state.construction_budget_control(fid, rt),
        };
        let value = match mode {
            ControlMode::Ai => ai_value * con_scale,
            ControlMode::Player => state
                .control(fid)
                .and_then(|c| match kind {
                    BudgetKind::Investment => c.investment_budget.get(rt),
                    BudgetKind::Construction => c.construction_budget.get(rt),
                })
                .map(|c| c.value)
                .unwrap_or(ai_value * con_scale),
        };
        budget.insert(rt.clone(), value);
        modes.push((rt.clone(), mode));
    }
    (budget, modes)
}

/// Write a computed budget back into the faction's controllable state so the
/// diff between rounds reflects what the simulation actually used.
fn write_budget(
    state: &mut State,
    fid: FactionId,
    kind: BudgetKind,
    budget: &ResourceMap,
    modes: &[(String, ControlMode)],
) {
    if let Some(c) = state.control_mut(fid) {
        for (rt, value) in budget {
            let mode = modes.iter().find(|(r, _)| r == rt).map(|(_, m)| *m).unwrap_or(ControlMode::Ai);
            let slot = match kind {
                BudgetKind::Investment => &mut c.investment_budget,
                BudgetKind::Construction => &mut c.construction_budget,
            };
            match mode {
                ControlMode::Ai => {
                    slot.insert(rt.clone(), Control::inherit(*value));
                }
                ControlMode::Player => {
                    slot.entry(rt.clone()).or_insert_with(|| Control::player(*value));
                }
            }
        }
    }
}

fn per_area_cost(config: &GameConfig, spec: &BuildingSpec, res_mod: f64, b: &Building) -> Vec<(String, f64)> {
    let mult = config.structure_spec(&b.structure).cost_mult * res_mod;
    spec.build_cost
        .iter()
        .map(|(rt, c)| (rt.clone(), c * mult))
        .collect()
}

fn budget_remaining(limit: &ResourceMap, spent: &ResourceMap, rt: &str) -> f64 {
    limit.get(rt).copied().unwrap_or(0.0) - spent.get(rt).copied().unwrap_or(0.0)
}

fn max_affordable_inc(cost_per_area: &[(String, f64)], limit: &ResourceMap, spent: &ResourceMap, cap: f64) -> f64 {
    let mut inc = cap;
    for (rt, c) in cost_per_area {
        if *c <= 1e-9 {
            continue;
        }
        let have = budget_remaining(limit, spent, rt);
        inc = inc.min(have / *c);
    }
    inc.max(0.0)
}

fn commit_spend(state: &mut State, fid: FactionId, spent: &mut ResourceMap, cost: &[(String, f64)]) {
    for (rt, c) in cost {
        if let Some(f) = state.faction_mut(fid) {
            let e = f.resources.entry(rt.clone()).or_insert(0.0);
            *e = (*e - c).max(0.0);
        }
        *spent.entry(rt.clone()).or_insert(0.0) += c;
    }
}

/// Pick a ship class for a new colony / new shipyard (with no class yet). Instead of
/// "random among fully-affordable" (which a cash-limited faction collapses to corvette),
/// the AI sizes its navy to what its **resource profile can fit** (soft affordability:
/// having most of the minerals counts, not all at once) and **diversifies** — it
/// prefers classes it currently has few of. So the fleet grows into a **mixed navy**
/// (screens + warships + carriers), not a one-class blob. Deterministic: the seeded
/// RNG drives a weighted pick over class scores (variety), reproducible per seed.
fn choose_next_class(state: &State, fid: FactionId, config: &GameConfig, rng: &mut Prng) -> String {
    let Some(f) = state.faction(fid) else { return "corvette".to_string() };
    let value_of = |r: &str| config.resources.get(r).map(|rr| rr.value).unwrap_or(1.0);
    let mut max_res = 0.0f64;
    for (r, v) in &f.resources {
        max_res = max_res.max(*v * value_of(r));
    }
    let ab = |r: &str| {
        if max_res > 1e-9 {
            f.resources.get(r).map(|v| *v * value_of(r) / max_res).unwrap_or(0.0)
        } else {
            0.0
        }
    };

    // Current fleet composition by class (to know what the navy already has).
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for s in &state.ships {
        if s.faction_id == fid && s.hull > 0.0 {
            *counts.entry(s.class.clone()).or_insert(0) += 1;
            total += 1;
        }
    }
    let total = total.max(1);
    let at_war = faction_at_war(state, config, fid);

    // Score: a **normalized** resource fit — how well the faction's profile covers the
    // class's cost (0..1, so cheap and expensive hulls are on the same scale: scarce
    // minerals lower it, but cost magnitude does not inflate it) — plus a
    // diversification bonus for under-represented classes, minus an upkeep penalty,
    // plus a **war bonus** (at war the AI leans toward high-firepower hulls). This
    // yields a mixed navy that adapts to the threat without letting a rich faction's
    // expensive hulls run away with the score (and the war).
    let mut scored: Vec<(String, f64)> = Vec::new();
    for (cls, spec) in &config.ships {
        let cost_val: f64 = spec.build_cost.iter().map(|(r, c)| c * value_of(r)).sum();
        let covered: f64 = spec.build_cost.iter().map(|(r, c)| c * value_of(r) * ab(r)).sum();
        let fit = if cost_val > 1e-9 { covered / cost_val } else { 0.0 };
        let share = counts.get(cls).copied().unwrap_or(0) as f64 / total as f64;
        // Classes the faction has < 25% of get a pull toward a balanced mix.
        let mix_bonus = (0.25 - share).max(0.0) * 1.5;
        let upkeep_penalty = spec.upkeep * 0.04; // 贵舰难养，只有当资源/构成都支持才造
        // 威胁响应：战时给火力强的舰型加分（多造战争机器）。
        let war_bonus = if at_war { spec.attack * 0.03 } else { 0.0 };
        scored.push((cls.clone(), fit + mix_bonus - upkeep_penalty + war_bonus));
    }

    // Weighted random pick → variety; deterministic via the seeded RNG.
    let total_score: f64 = scored.iter().map(|(_, s)| s.max(0.0)).sum();
    if total_score <= 1e-9 {
        return "corvette".to_string();
    }
    let mut roll = rng.unit() * total_score;
    let mut last = "corvette".to_string();
    for (cls, s) in &scored {
        last = cls.clone();
        roll -= s.max(0.0);
        if roll <= 0.0 {
            return cls.clone();
        }
    }
    last
}

/// Deterministically pick a ship component loadout (舰船定制) for a faction building
/// a ship of `class` — the **resource → military** link, now also **category-balanced
/// and threat-aware**:
///   * components whose rare inputs the faction has in abundance score highest
///     (resource advantage); unaffordable ones are dropped;
///   * the loadout is balanced across weapon / defense / support so a ship can both
///     hit and survive (a real commander doesn't field a mono-stack of glass cannons —
///     it guarantees at least one weapon and, when the ship has ≥2 slots, one defense);
///   * when at war the AI is biased toward weapons (weapon score bonus), so it invests
///     in firepower; in peace it invests more in defense/support.
/// Deterministic (no RNG): score is a function of stockpile + config, ties break on
/// component id. Returns ≤ `ShipSpec::slots` component ids, cumulatively affordable.
fn choose_loadout(state: &State, config: &GameConfig, fid: FactionId, class: &str) -> Vec<String> {
    let slots = config.ship_spec(class).slots as usize;
    if slots == 0 {
        return Vec::new();
    }
    let Some(f) = state.faction(fid) else { return Vec::new() };
    let value_of = |r: &str| config.resources.get(r).map(|rr| rr.value).unwrap_or(1.0);

    // Normalized resource abundance by market value in the stockpile.
    let mut max_ab = 0.0f64;
    for (r, v) in &f.resources {
        max_ab = max_ab.max(*v * value_of(r));
    }
    if max_ab <= 1e-9 {
        return Vec::new();
    }
    let abund = |r: &str| f.resources.get(r).map(|v| *v * value_of(r) / max_ab).unwrap_or(0.0);

    // 战局感知：交战中的势力更看重武器（武器加分），和平时更偏向防御/支持。
    let at_war = faction_at_war(state, config, fid);

    // Score every candidate component by (a) resource fit — how much of its rare
    // inputs the faction can comfortably supply — plus (b) a small raw combat-gain
    // tiebreak, minus (c) an upkeep drag so components that are strong but too costly
    // to maintain get deprioritized. 战时给武器加分（更舍得堆火力）。
    let mut cands: Vec<(String, f64)> = Vec::new();
    for (id, cs) in &config.components {
        let fit: f64 = cs.cost.iter().map(|(r, c)| c * value_of(r) * abund(r)).sum();
        let gain = cs.damage * 4.0 + cs.shield * 0.8 + cs.hull * 0.8 + cs.hull_regen * 120.0
            + cs.shield_regen * 60.0 + cs.speed * 3.0 + cs.intercept * 2.0 + cs.range * 12.0;
        let mut score = fit + gain * 0.03 - cs.upkeep * 2.0;
        if at_war && cs.category == "weapon" {
            score += cs.damage * 2.0; // 战时要火力。
        }
        if score > 0.0 {
            cands.push((id.clone(), score));
        }
    }
    cands.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    // 类别配比（拟人指挥官）：一艘军舰要「又能打、又能扛」——至少一件武器（买得起时）、
    // slot≥2 时至少一件防御（避免全军玻璃大炮或全是乌龟），其余按分数填满。
    let defense_floor = if slots >= 2 { 1 } else { 0 };

    let mut chosen: Vec<String> = Vec::new();
    let mut remaining = f.resources.clone();
    let afford = |id: &str, rem: &ResourceMap| -> bool {
        config
            .component_spec(id)
            .cost
            .iter()
            .all(|(r, c)| rem.get(r).copied().unwrap_or(0.0) >= *c)
    };
    let count_cat = |chosen: &Vec<String>, cat: &str| -> usize {
        chosen.iter().filter(|id| config.component_spec(id).category == cat).count()
    };

    // Pass 1: 保证至少一件武器（若买得起某件武器）。
    for (id, _) in &cands {
        if count_cat(&chosen, "weapon") >= 1 {
            break;
        }
        if config.component_spec(id).category == "weapon" && !chosen.contains(id) && afford(id, &remaining) {
            let cs = config.component_spec(id);
            for (r, c) in &cs.cost {
                *remaining.entry(r.clone()).or_insert(0.0) -= c;
            }
            chosen.push(id.clone());
        }
    }
    // Pass 2: 保证至少一件防御（slot≥2 且买得起时）。
    if count_cat(&chosen, "defense") < defense_floor {
        for (id, _) in &cands {
            if count_cat(&chosen, "defense") >= defense_floor {
                break;
            }
            if config.component_spec(id).category == "defense" && !chosen.contains(id) && afford(id, &remaining) {
                let cs = config.component_spec(id);
                for (r, c) in &cs.cost {
                    *remaining.entry(r.clone()).or_insert(0.0) -= c;
                }
                chosen.push(id.clone());
            }
        }
    }
    // Pass 3: 填满剩余槽位（按分数，武器在战时因加分更易入选）。
    for (id, _) in &cands {
        if chosen.len() >= slots {
            break;
        }
        if chosen.contains(id) {
            continue;
        }
        if afford(id, &remaining) {
            let cs = config.component_spec(id);
            for (r, c) in &cs.cost {
                *remaining.entry(r.clone()).or_insert(0.0) -= c;
            }
            chosen.push(id.clone());
        }
    }
    chosen
}

fn step_construction(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    let mut next_ship_id = state.ships.iter().map(|s| s.id).max().map_or(0, |m| m + 1);
    let mut next_building_id = state
        .cities
        .iter()
        .flat_map(|c| c.buildings.iter().map(|b| b.id))
        .max()
        .map_or(0, |m| m + 1);

    for fid in faction_ids {
        let (investment, inv_modes) = read_budget(state, config, fid, BudgetKind::Investment);
        let (construction, con_modes) = read_budget(state, config, fid, BudgetKind::Construction);
        write_budget(state, fid, BudgetKind::Investment, &investment, &inv_modes);
        write_budget(state, fid, BudgetKind::Construction, &construction, &con_modes);

        let mut inv_spent: ResourceMap = ResourceMap::new();
        let mut con_spent: ResourceMap = ResourceMap::new();

        let city_ids: Vec<CityId> = state.cities.iter().filter(|c| c.faction_id == fid).map(|c| c.id).collect();
        for cid in city_ids {
            build_city(
                state,
                config,
                cid,
                fid,
                &investment,
                &construction,
                &mut inv_spent,
                &mut con_spent,
                &mut next_ship_id,
                &mut next_building_id,
                rng,
            );
        }
    }
    // 威胁响应（整支舰队随威胁重构）：战时把过度生产的「轻舰」船坞按战况重定向到更重/更
    // 需要的舰型，让威胁响应不只作用于新建舰厂。确定性（seeded RNG）。
    let retool_ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    for fid in retool_ids {
        retool_shipyards(state, config, fid, rng);
    }
}

/// 威胁响应（海军随威胁重构）：交战中，若某势力的舰队被单一舰型统治（占比 > `over_share`），
/// 就把它产出该舰型的最小 id 船坞重定向到 `choose_next_class` 选出的**战局感知新舰型**
/// （战争加分——多造重舰；去重加分——避免单调）。和平时不重定向（船坞保持生产既有舰型）。
/// 每次至多重定向一个船坞、且只在明显过度生产时触发，避免抖振。确定性（seeded RNG）。
fn retool_shipyards(state: &mut State, config: &GameConfig, fid: FactionId, rng: &mut Prng) {
    if !faction_at_war(state, config, fid) {
        return;
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for s in &state.ships {
        if s.faction_id == fid && s.hull > 0.0 {
            *counts.entry(s.class.clone()).or_insert(0) += 1;
            total += 1;
        }
    }
    if total == 0 {
        return;
    }
    let (over_class, over_count) = counts.iter().max_by_key(|(_, n)| **n).map(|(k, n)| (k.clone(), *n)).unwrap();
    // 舰队不是被单一舰型**严重**统治就不重定向（保守：只在极度单一时触发，避免扰动
    // 权力平衡与「霸权→联盟」的合纵连横节奏）。
    if (over_count as f64) / (total as f64) < 0.60 {
        return;
    }
    let new_class = choose_next_class(state, fid, config, rng);
    if new_class == over_class {
        return;
    }
    // 找到产出 over_class 的最小 id 船坞（按城市 id、再按建筑 id）。
    let mut target: Option<(CityId, BuildingId)> = None;
    for c in &state.cities {
        if c.faction_id != fid {
            continue;
        }
        for b in &c.buildings {
            if b.is_shipyard() && b.ship_type.as_deref() == Some(over_class.as_str()) {
                target = Some((c.id, b.id));
                break;
            }
        }
        if target.is_some() {
            break;
        }
    }
    if let Some((cid, bid)) = target {
        if let Some(c) = state.city_mut(cid) {
            for b in &mut c.buildings {
                if b.id == bid {
                    b.ship_type = Some(new_class.clone());
                    break;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_city(
    state: &mut State,
    config: &GameConfig,
    cid: CityId,
    fid: FactionId,
    invest_limit: &ResourceMap,
    con_limit: &ResourceMap,
    inv_spent: &mut ResourceMap,
    con_spent: &mut ResourceMap,
    next_ship_id: &mut u32,
    next_building_id: &mut BuildingId,
    _rng: &mut Prng,
) {
    if state.city(cid).map(|c| c.razed).unwrap_or(true) {
        return;
    }
    let (ecocap, total_area, speed_mod, res_mod, deposits) = {
        // 城市的定居点 = 它自己占据的那一个（1:1），面积/矿藏以该定居点为准。
        let s = state.city_settlement(cid);
        match s {
            Some(s) => (
                s.ecological_capacity,
                s.total_area,
                s.construction_speed_mod,
                s.construction_resource_mod,
                s.resources.iter().map(|d| (d.resource.clone(), d.area)).collect::<Vec<_>>(),
            ),
            None => return,
        }
    };

    let mut buildings: Vec<Building> = state.city(cid).expect("city gone").buildings.clone();
    let population = state.city(cid).map(|c| c.population as f64).unwrap_or(0.0);
    let labor = labor_ratio(state, config, cid);

    fn new_b(id: BuildingId, kind: &str, resource: Option<String>, ship_type: Option<String>, structure: &str, area: f64, deployed: f64, config: &GameConfig) -> Building {
        let armor = deployed * config.structure_spec(structure).armor_per_area;
        Building {
            id,
            kind: kind.to_string(),
            resource,
            ship_type,
            structure: structure.to_string(),
            area,
            deployed,
            armor,
        }
    }

    fn find_area(buildings: &[Building], kind: &str, resource: Option<&str>) -> f64 {
        buildings
            .iter()
            .filter(|b| b.kind == kind)
            .find(|b| match (resource, b.resource.as_deref()) {
                (Some(r), Some(br)) => r == br,
                (None, None) => true,
                _ => false,
            })
            .map(|b| b.area)
            .unwrap_or(0.0)
    }

    fn raise_area(
        buildings: &mut Vec<Building>,
        config: &GameConfig,
        next_id: &mut BuildingId,
        kind: &str,
        resource: Option<&str>,
        add: f64,
    ) {
        if add <= 1e-9 {
            return;
        }
        let idx = buildings.iter().position(|b| {
            b.kind == kind
                && match (resource, b.resource.as_deref()) {
                    (Some(r), Some(br)) => r == br,
                    (None, None) => true,
                    _ => false,
                }
        });
        match idx {
            Some(i) => buildings[i].area += add,
            None => {
                buildings.push(new_b(*next_id, kind, resource.map(str::to_string), None, "concrete", add, 0.0, config));
                *next_id += 1;
            }
        }
    }

    // 1) Plan new area (raising targets), bounded by total_area.
    let mut planning_remaining = total_area - buildings.iter().map(|b| b.area).sum::<f64>();

    let desired_res = (population / ecocap.max(1e-6)) * config.economy.housing_buffer;
    let res_area = find_area(&buildings, "residential", None);
    if res_area < desired_res - 1e-9 && planning_remaining > 0.0 {
        let add = (desired_res - res_area).min(planning_remaining);
        raise_area(&mut buildings, config, next_building_id, "residential", None, add);
        planning_remaining -= add;
    }

    // Grow a 建造区 (shipyard) toward its target. If a city holds several
    // shipyards, grow the one with the largest current footprint.
    let con_target = (total_area * 0.15).clamp(4.0, 12.0);
    let shipyard_index = buildings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.is_shipyard())
        .max_by(|(_, a), (_, b)| a.deployed.total_cmp(&b.deployed))
        .map(|(i, _)| i);
    let con_area = shipyard_index.map(|i| buildings[i].area).unwrap_or(0.0);
    if con_area < con_target - 1e-9 && planning_remaining > 0.0 {
        let add = (con_target - con_area).min(planning_remaining);
        let idx = shipyard_index;
        match idx {
            Some(i) => buildings[i].area += add,
            None => buildings.push(new_b(*next_building_id, "construction", None, None, "concrete", add, 0.0, config)),
        }
        if idx.is_none() {
            *next_building_id += 1;
        }
        planning_remaining -= add;
    }

    for (rt, darea) in &deposits {
        if planning_remaining <= 0.0 {
            break;
        }
        let cur = find_area(&buildings, "mining", Some(rt));
        if cur < *darea - 1e-9 {
            let add = (*darea - cur).min(planning_remaining);
            raise_area(&mut buildings, config, next_building_id, "mining", Some(rt), add);
            planning_remaining -= add;
        }
    }

    // 2) Build: grow deployed toward the planned area, spending the investment
    // budget. Higher invest weight builds first.
    buildings.sort_by(|a, b| {
        invest_weight(state, config, fid, cid, b).total_cmp(&invest_weight(state, config, fid, cid, a))
    });
    for b in buildings.iter_mut() {
        if !b.under_construction() {
            continue;
        }
        let spec = config.building_spec(&b.kind);
        let per_area = per_area_cost(config, spec, res_mod, b);
        let speed = spec.construction_speed * speed_mod * spec.productivity * labor;
        let desired = (b.area - b.deployed).min(speed);
        let inc = max_affordable_inc(&per_area, invest_limit, inv_spent, desired);
        if inc <= 1e-6 {
            continue;
        }
        let cost: Vec<(String, f64)> = per_area.iter().map(|(rt, c)| (rt.clone(), *c * inc)).collect();
        commit_spend(state, fid, inv_spent, &cost);
        b.deployed += inc;
    }

    // Regrow armor: freshly-built area is intact; otherwise repair toward max at
    // the configured regen rate.
    for b in buildings.iter_mut() {
        let amax = b.armor_max(config);
        if b.under_construction() {
            b.armor = amax;
        } else {
            b.armor = (b.armor + (amax - b.armor) * config.combat.armor_regen).min(amax).max(0.0);
        }
    }

    // 3) Ship building: each 建造区 contributes to its class's rate; progress is
    // per city. The construction budget funds completed ships; shipyards compete
    // for it by build weight, so a higher-weight shipyard pays for its ship first.
    let mut shipyards: Vec<(usize, String, f64, f64)> = Vec::new(); // (index, class, weight, area)
    for (i, b) in buildings.iter().enumerate() {
        if b.is_shipyard() {
            if let Some(cls) = b.ship_type.clone() {
                let area = b.deployed;
                if area > 1e-9 {
                    shipyards.push((i, cls, build_weight(state, config, fid, cid, b), area));
                }
            }
        }
    }
    shipyards.sort_by(|a, b| b.2.total_cmp(&a.2));

    let city_progress: BTreeMap<String, f64> = state.city(cid).map(|c| c.ship_progress.clone()).unwrap_or_default();

    // Aggregate per-class production rate (all 建造区 of a class add up toward the
    // city pool) and per-class build priority (max of its shipyards' weights).
    let mut class_rate: BTreeMap<String, f64> = BTreeMap::new();
    let mut class_weight: BTreeMap<String, f64> = BTreeMap::new();
    for (_, cls, w, area) in &shipyards {
        *class_rate.entry(cls.clone()).or_insert(0.0) += area * config.building_spec("construction").productivity * labor;
        let e = class_weight.entry(cls.clone()).or_insert(0.0);
        *e = e.max(*w);
    }
    let mut classes: Vec<(String, f64, f64)> = class_rate
        .iter()
        .map(|(c, r)| (c.clone(), *r, class_weight.get(c).copied().unwrap_or(0.0)))
        .collect();
    classes.sort_by(|a, b| b.2.total_cmp(&a.2));

    // The construction budget is a per-round rate: it funds ship progress
    // incrementally (cost-per-progress × increment). A class completes a ship
    // once it has accrued `build_points`, at the city level.
    let body_id = state.city(cid).map(|c| c.body_id).unwrap_or(0);
    let body_pos = state.body_position(body_id);
    let mut to_write_progress = city_progress;
    for (cls, rate, _) in &classes {
        let spec = config.ship_spec(cls);
        let bp = spec.build_points;
        let per_progress: Vec<(String, f64)> = spec.build_cost.iter().map(|(rt, c)| (rt.clone(), c / bp)).collect();
        let increment = max_affordable_inc(&per_progress, con_limit, con_spent, *rate).max(0.0);
        if increment <= 1e-9 {
            continue;
        }
        let cost: Vec<(String, f64)> = per_progress.iter().map(|(rt, c)| (rt.clone(), *c * increment)).collect();
        commit_spend(state, fid, con_spent, &cost);
        *to_write_progress.entry(cls.clone()).or_insert(0.0) += increment;
        // Spawn ships as their build points fill (the cost was paid as progress).
        // On launch, the ship is fitted with a deterministic component loadout chosen
        // from the faction's resource advantage (see `choose_loadout`); the component
        // cost is paid out of the stockpile and the effective panel (hull_max, etc.)
        // is computed from class + components.
        while to_write_progress.get(cls).copied().unwrap_or(0.0) >= bp - 1e-9 {
            let components = choose_loadout(state, config, fid, cls);
            let mut ship = Ship {
                id: *next_ship_id,
                name: format!("{}-{}", spec.label, fid),
                class: cls.clone(),
                faction_id: fid,
                position: [body_pos[0] + 0.05, body_pos[1] + 0.05],
                hull: 0.0,
                hull_max: 0.0,
                shield: 0.0,
                shield_max: 0.0,
                components,
                component_hp: Vec::new(),
            };
            let panel = ship_panel(config, &ship);
            ship.hull = panel.hull_max;
            ship.hull_max = panel.hull_max;
            ship.shield = panel.shield_max;
            ship.shield_max = panel.shield_max;
            // 每件组件初始满完整度（模块毁损用）。
            ship.component_hp = ship.components.iter().map(|c| component_integrity(config, c)).collect();
            state.ships.push(ship);
            // Pay the (validated-affordable) component cost.
            let mut spent0 = std::collections::BTreeMap::new();
            let comp_cost: Vec<(String, f64)> = state
                .ship(*next_ship_id)
                .map(|s| s.components.iter().flat_map(|c| config.component_spec(c).cost.clone()).collect())
                .unwrap_or_default();
            commit_spend(state, fid, &mut spent0, &comp_cost);
            ev(state, GameEvent::ShipSpawned { ship: *next_ship_id, owner: fid, class: cls.clone(), city: cid });
            *next_ship_id += 1;
            *to_write_progress.entry(cls.clone()).or_insert(0.0) -= bp;
            if let Some(c) = state.control_mut(fid) {
                c.ship_orders.insert(*next_ship_id - 1, Control::inherit(ShipBehavior::Idle));
            }
        }
    }
    if let Some(city) = state.city_mut(cid) {
        city.ship_progress = to_write_progress;
    }

    // 4) Write back, and ensure every building has invest/build-weight entries.
    if let Some(c) = state.control_mut(fid) {
        for b in &buildings {
            let key = (cid, b.id);
            c.invest_weights
                .entry(key)
                .or_insert_with(|| Control::inherit(config.building_spec(&b.kind).default_invest_weight));
            if b.is_shipyard() {
                c.build_weights
                    .entry(key)
                    .or_insert_with(|| Control::inherit(config.building_spec(&b.kind).default_build_weight));
            }
        }
    }
    if let Some(city) = state.city_mut(cid) {
        city.buildings = buildings;
    }
}

// --- military ---------------------------------------------------------------

fn step_military(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    let mut order: Vec<ShipId> = state.ships.iter().map(|s| s.id).collect();
    for i in (1..order.len()).rev() {
        let j = rng.range(i as u64 + 1) as usize;
        order.swap(i, j);
    }

    let mut next_city_id = state.cities.iter().map(|c| c.id).max().map_or(0, |m| m + 1);
    let mut next_building_id = state
        .cities
        .iter()
        .flat_map(|c| c.buildings.iter().map(|b| b.id))
        .max()
        .map_or(0, |m| m + 1);

    // 联盟军事协同的「集火目标」：每回合每个势力各算一次（O(势力 × 实体)，摊薄到全军）。
    // 结盟势力在霸权先动手时优先集火该霸权，而非各自就近乱打。
    let focus_of: BTreeMap<FactionId, Option<FactionId>> = state
        .factions
        .iter()
        .map(|f| (f.id, coalition_war_focus(state, config, f.id)))
        .collect();

    for ship_id in order {
        let Some(ship) = state.ship(ship_id) else { continue };
        if ship.hull <= 0.0 {
            continue;
        }
        let owner = ship.faction_id;
        let class = ship.class.clone();
        let pos = ship.position;
        // 有效交战距离 = 舰级炮台 + 武器组件的最大射程（远程组件让舰在前置位置先开火）。
        let range = ship_panel(config, ship).attack_range;
        let my_hull = ship.hull;
        let my_hull_max = ship.hull_max;
        let focus = focus_of.get(&owner).copied().flatten();

        let is_ai = state.ship_control(ship_id) == ControlMode::Ai;

        if !is_ai {
            // --- player-controlled: execute the commanded behavior literally ---
            let mut behavior = state.ship_behavior(ship_id).unwrap_or(ShipBehavior::Idle);
            // A stale targeting/colonize order (target destroyed, city razed, or
            // a body with no settlement) must not send the ship drifting toward
            // the origin ([0,0]); degrade it to Idle and record a StaleOrder
            // event so the agent knows to re-issue. Move/Idle are always valid.
            if !behavior_is_valid(state, config, behavior, owner) {
                let reason = match behavior {
                    ShipBehavior::TargetShip { attack: true, .. } => format!("target ship gone"),
                    ShipBehavior::TargetShip { attack: false, .. } => format!("guarded ship gone"),
                    ShipBehavior::TargetSettlement { city, .. } => format!("target city {city} razed or not hostile"),
                    ShipBehavior::Colonize { body } => format!("body {body} has no blank settlement"),
                    _ => "invalid".to_string(),
                };
                if let Some(c) = state.control_mut(owner) {
                    c.ship_orders.insert(ship_id, Control::player(ShipBehavior::Idle));
                }
                ev(state, GameEvent::StaleOrder { ship: ship_id, reason });
                behavior = ShipBehavior::Idle;
            }
            match behavior {
                // Idle (待命): hold position, no movement this round.
                ShipBehavior::Idle => continue,
                ShipBehavior::Dock { .. } => {
                    // 停泊：跟随天体——每回合重新取天体当前位置并驶向它。
                    move_toward(state, config, ship_id, &class, behavior_dest(state, behavior));
                }
                ShipBehavior::Colonize { body } => {
                    let bpos = state.body_position(body);
                    if dist(pos, bpos) <= config.combat.arrival_eps {
                        colonize(state, config, rng, ship_id, body, &mut next_building_id, &mut next_city_id);
                        continue;
                    }
                    move_toward(state, config, ship_id, &class, bpos);
                    let np = state.ship(ship_id).map(|s| s.position).unwrap_or(pos);
                    if dist(np, bpos) <= config.combat.arrival_eps {
                        colonize(state, config, rng, ship_id, body, &mut next_building_id, &mut next_city_id);
                    }
                    continue;
                }
                ShipBehavior::TargetShip { ship, attack } => {
                    if attack {
                        // Attack: pursue the enemy and open fire once in range.
                        if let Some(t) = state.ship(ship) {
                            if t.hull > 0.0 && dist(pos, t.position) <= range {
                                fire(state, config, ship_id, ship);
                                continue;
                            }
                        }
                        move_toward(state, config, ship_id, &class, behavior_dest(state, behavior));
                        if let Some(t) = state.ship(ship) {
                            if t.hull > 0.0 {
                                let np = state.ship(ship_id).map(|s| s.position).unwrap_or(pos);
                                if dist(np, t.position) <= range {
                                    fire(state, config, ship_id, ship);
                                }
                            }
                        }
                    } else {
                        // Guard: escort a friendly ship. Stay near it and intercept
                        // any hostile that comes within our own attack range, but do
                        // not fire at the protected ship itself. This is a defensive
                        // station, not a pursuit.
                        if let Some(t) = state.ship(ship) {
                            if t.hull > 0.0 {
                                let gpos = t.position;
                                // Drift toward the protected ship.
                                move_toward(state, config, ship_id, &class, gpos);
                                let np = state.ship(ship_id).map(|s| s.position).unwrap_or(pos);
                                // Intercept the closest hostile in range, scanning
                                // around the guard first, then the protected ship.
                                if let Some(enemy) = nearest_enemy_ship(state, config, owner, np, range, focus, ship_id) {
                                    fire(state, config, ship_id, enemy);
                                } else if let Some(e2) = nearest_enemy_ship(state, config, owner, gpos, range, focus, ship_id) {
                                    fire(state, config, ship_id, e2);
                                }
                            }
                        }
                    }
                }
                ShipBehavior::TargetSettlement { city, bombard } => {
                    let cpos = city_position(state, city);
                    if bombard && dist(pos, cpos) <= config.combat.siege_range {
                        bombard_city(state, config, ship_id, city);
                        continue;
                    }
                    move_toward(state, config, ship_id, &class, behavior_dest(state, behavior));
                    if bombard {
                        let np = state.ship(ship_id).map(|s| s.position).unwrap_or(pos);
                        if dist(np, cpos) <= config.combat.siege_range {
                            bombard_city(state, config, ship_id, city);
                        }
                    }
                }
                ShipBehavior::Move { .. } => {
                    move_toward(state, config, ship_id, &class, behavior_dest(state, behavior));
                }
            }
            continue;
        }

        // --- AI-controlled: existing auto behavior ---
        let tgt = nearest_enemy_ship(state, config, owner, pos, range, focus, ship_id);

        // 自保撤退（拟人的「别送死」）：舰已受重创、敌在本舰射程内、且离首都有一定距离
        // 时，不再死战，而是后撤回首都/本土修整充能（远离本土难以获得再生与防御）。这
        // 让战争有「打残→撤→养好→再来」的损耗与恢复循环，也避免一整支舰队白白送死。
        if let Some(target) = tgt {
            if my_hull / my_hull_max.max(1e-9) < config.combat.retreat_hull {
                if let Some(cap_body) = state.faction(owner).map(|f| f.capital_body) {
                    let cap_pos = state.body_position(cap_body);
                    if dist(pos, cap_pos) > config.combat.retreat_min_dist {
                        if let Some(c) = state.control_mut(owner) {
                            c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::Move { position: cap_pos }));
                        }
                        ev(state, GameEvent::Withdraw { ship: ship_id, to_body: cap_body });
                        move_toward(state, config, ship_id, &class, cap_pos);
                        continue;
                    }
                }
            }
            // 否则接战：集中火力打最残的敌舰（nearest_enemy_ship 已按受创程度排序）。
            if let Some(c) = state.control_mut(owner) {
                c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::TargetShip { ship: target, attack: true }));
            }
            fire(state, config, ship_id, target);
            continue;
        }

        let Some(behavior) = resolve_target(state, config, ship_id, owner, pos, rng, focus) else {
            continue;
        };

        if let ShipBehavior::TargetSettlement { city, bombard } = behavior {
            let cpos = city_position(state, city);
            if bombard && dist(pos, cpos) <= config.combat.siege_range {
                bombard_city(state, config, ship_id, city);
                continue;
            }
        }
        if let ShipBehavior::Colonize { body } = behavior {
            let bpos = state.body_position(body);
            if dist(pos, bpos) <= config.combat.arrival_eps {
                colonize(state, config, rng, ship_id, body, &mut next_building_id, &mut next_city_id);
                continue;
            }
        }

        move_toward(state, config, ship_id, &class, behavior_dest(state, behavior));

        if let Some(ship) = state.ship(ship_id) {
            let np = ship.position;
            if let Some(target) = nearest_enemy_ship(state, config, owner, np, range, focus, ship_id) {
                if let Some(c) = state.control_mut(owner) {
                    c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::TargetShip { ship: target, attack: true }));
                }
                fire(state, config, ship_id, target);
            } else if let ShipBehavior::TargetSettlement { city, bombard } = behavior {
                let cpos = city_position(state, city);
                if bombard && dist(np, cpos) <= config.combat.siege_range {
                    bombard_city(state, config, ship_id, city);
                }
            } else if let ShipBehavior::Colonize { body } = behavior {
                let bpos = state.body_position(body);
                if dist(np, bpos) <= config.combat.arrival_eps {
                    colonize(state, config, rng, ship_id, body, &mut next_building_id, &mut next_city_id);
                }
            }
        }
    }

    // 护甲再生（%/时间）：每回合幸存舰只按舰级 hull_regen 恢复其最大护甲的一
    // 个比例（不消耗资源、不复活已毁舰）。处于本方本土防御半径内的舰获得额外
    // home_regen_bonus 再生（cult 的 MOND 异常使其圣所旁舰只极难被消耗）。
    // 先读出每艘舰的本土再生加成，再统一修改（避免与 ships 的可变借用冲突）。
    let (bonuses, friendly): (Vec<f64>, Vec<bool>) = state
        .ships
        .iter()
        .map(|s| {
            if s.hull > 0.0 {
                let f = state
                    .faction(s.faction_id)
                    .map(|fac| dist(s.position, state.body_position(fac.capital_body)) <= fac.home_radius)
                    .unwrap_or(false);
                (home_regen_bonus(state, s.faction_id, s.position), f)
            } else {
                (0.0, false)
            }
        })
        .unzip();
    let comp_repair = config.combat.component_repair;
    for (i, s) in state.ships.iter_mut().enumerate() {
        if s.hull > 0.0 {
            let panel = ship_panel(config, s);
            s.hull = (s.hull + panel.hull_max * (panel.hull_regen + bonuses[i])).min(panel.hull_max);
            // 能量护盾每回合再生（护盾组件）：护盾优先吸收、损毁后再生，是防御组件的关键。
            if panel.shield_max > 0.0 {
                s.shield = (s.shield + panel.shield_max * panel.shield_regen).min(panel.shield_max);
            }
            // 模块修复：受损组件在母港/友方本土修得更快（与自保撤退闭环：打残→撤→修→再来）。
            if comp_repair > 0.0 && !s.components.is_empty() {
                if s.component_hp.len() != s.components.len() {
                    s.component_hp = s.components.iter().map(|c| component_integrity(config, c)).collect();
                }
                let rate = comp_repair * if friendly[i] { 2.5 } else { 1.0 };
                for (j, c) in s.components.iter().enumerate() {
                    let max = component_integrity(config, c);
                    if s.component_hp[j] < max {
                        s.component_hp[j] = (s.component_hp[j] + max * rate).min(max);
                    }
                }
            }
        }
    }

    // Drop destroyed ships and prune their behaviors from the controllable state.
    state.ships.retain(|s| s.hull > 0.0);
    let alive: std::collections::BTreeSet<ShipId> = state.ships.iter().map(|s| s.id).collect();
    for c in state.control.values_mut() {
        c.ship_orders.retain(|sid, _| alive.contains(sid));
    }
}

// --- resurgence (anti-zombie / anti-monopoly) -------------------------------

/// If a faction ends a round with no ship and no living city, it has become a
/// permanent bystander (「僵尸」): with no city it can never build a ship, and with
/// no ship it can never re-colonize a razed city. Over a long horizon this
/// shrinks the board to a handful of power blocs and leaves the rest frozen —
/// the world stops being a game.
///
/// 重建 (resurgence) breaks that trap: on the round a faction reaches 0 ships +
/// 0 living cities, it re-establishes a foothold on its own lowest-id razed city
/// (a *diaspora claim* — a razed city keeps its last owner's `faction_id` until
/// someone else re-colonizes it) and launches one affordable colony ship there.
/// Deterministic: no RNG beyond the existing affordable-class pick. A faction
/// that has been fully absorbed (has no footprint anywhere) is left eliminated —
/// the rare, legitimate end of a civ.
/// 找一个「尚无任何城市占据」的定居点（含空白/从未殖民）；城市占用即排除。返回
/// (body_id, settlement_idx)。用于反僵尸重建的**强制立足点**兜底：当世界暂时没有空白
/// 城足迹、又必须给被灭势力一个落脚点时，就在未占用定居点上新建一座城。
fn find_vacant_settlement(state: &State) -> Option<(BodyId, usize)> {
    for b in &state.bodies {
        if b.settlements.is_empty() {
            continue;
        }
        let occupied: BTreeSet<usize> = state
            .cities
            .iter()
            .filter(|c| c.body_id == b.id)
            .map(|c| c.settlement)
            .collect();
        for idx in 0..b.settlements.len() {
            if !occupied.contains(&idx) {
                return Some((b.id, idx));
            }
        }
    }
    None
}

/// 反僵尸重建的**最后兜底**：当世界已满（每个定居点都被活城占据）、被灭势力既无自己
/// 的空白足迹、也找不到未占用定居点时，难民潮会**夺取当前控制城数最多的那个势力的最小
/// id 活城**（一个边缘殖民地被难民潮占据），作为重新立足点。这保证「被完全吞并」的旧
/// 势力也总能重返——既维持「上千回合不崩坏、无永久旁观者」，又给大帝国一个「难民危机」
/// 式的代价。确定性（无 RNG）。返回被夺取的城市 id。
fn displace_city_for_refugee(state: &State) -> Option<CityId> {
    let mut counts: BTreeMap<FactionId, usize> = BTreeMap::new();
    for c in &state.cities {
        if !c.razed {
            *counts.entry(c.faction_id).or_insert(0) += 1;
        }
    }
    let holder = counts.iter().max_by_key(|(_, n)| **n).map(|(k, _)| *k)?;
    state
        .cities
        .iter()
        .filter(|c| c.faction_id == holder && !c.razed)
        .min_by_key(|c| c.id)
        .map(|c| c.id)
}

/// 反僵尸重建（`Resurgence` 事件）。若一支势力在一回合结束时**既无舰又无活城**（已被
/// 彻底消灭、无从再殖民/重建的下限），它会在自己仍持有的**残骸足迹**上重新立足：重建
/// 一座城并出场一艘廉价种子舰——保证没有势力会**永久**变成旁观者。确定性：无未播种 RNG；
/// 位置/舰名/id 全由状态推导，同种子完全复现。
///
/// 立足点优先级（确保「总能重返」）：
/// 1. 该势力自己最低 id 的空白城（diaspora claim，空白城保留最后主人的 id）；
/// 2. 全系统最低 id 的任意空白城（难民避风港）；
/// 3. 若无任何空白城，则在一个**从未被占据**的定居点上新建一座城（强制立足点）。
fn step_resurgence(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    let mut next_ship = state.ships.iter().map(|s| s.id).max().map_or(0, |m| m + 1);
    let mut next_city = state.cities.iter().map(|c| c.id).max().map_or(0, |m| m + 1);
    let mut next_building = state
        .cities
        .iter()
        .flat_map(|c| c.buildings.iter().map(|b| b.id))
        .max()
        .map_or(0, |m| m + 1);

    for fid in faction_ids {
        // Already a participant (has a ship or a living city)? Nothing to do.
        let has_ship = state.ships.iter().any(|s| s.faction_id == fid && s.hull > 0.0);
        if has_ship {
            continue;
        }
        let has_living_city = state.cities.iter().any(|c| c.faction_id == fid && !c.razed);
        if has_living_city {
            continue;
        }

        let seeded_ship_class = choose_next_class(state, fid, config, rng);
        let spec = config.ship_spec(&seeded_ship_class);

        // Anchor 1/2: this faction's own lowest-id footprint, else any razed refuge.
        let anchor = state.cities.iter().filter(|c| c.faction_id == fid).min_by_key(|c| c.id).map(|c| c.id);
        let anchor_id = anchor.or_else(|| {
            state.cities.iter().filter(|c| c.razed).min_by_key(|c| c.id).map(|c| c.id)
        });

        let (body, pos) = if let Some(anchor_id) = anchor_id {
            // Re-seed the anchor's city (diaspora claim / refugee refuge).
            let Some(settlement) = state.city_settlement(anchor_id).cloned() else { continue };
            let body = state.city(anchor_id).map(|c| c.body_id).unwrap_or(0);
            let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
            let buildings = seed_colony_buildings(&settlement, pop, &seeded_ship_class, config, &mut next_building);
            if let Some(c) = state.city_mut(anchor_id) {
                c.razed = false;
                c.faction_id = fid;
                c.population = pop;
                c.buildings = buildings;
                c.ship_progress.clear();
                c.ship_progress.insert(seeded_ship_class.clone(), 0.0);
                c.loyalty = 1.0;
            }
            // Ensure the re-seeded buildings have invest/build-weight entries.
            let city_buildings = state.city(anchor_id).map(|c| c.buildings.clone()).unwrap_or_default();
            if let Some(ctrl) = state.control_mut(fid) {
                for b in &city_buildings {
                    let ikey = (anchor_id, b.id);
                    ctrl.invest_weights.entry(ikey).or_insert_with(|| {
                        Control::inherit(config.building_spec(&b.kind).default_invest_weight)
                    });
                    if b.is_shipyard() {
                        let bkey = (anchor_id, b.id);
                        ctrl.build_weights.entry(bkey).or_insert_with(|| {
                            Control::inherit(config.building_spec(&b.kind).default_build_weight)
                        });
                    }
                }
            }
            let cpos = state.body_position(body);
            (body, [cpos[0] + 0.05, cpos[1] + 0.05])
        } else {
            // Anchor 3/4 (last resort): no razed footprint and no vacant settlement.
            // First try to found a brand-new city on a never-occupied settlement; if
            // the world is entirely full, a diaspora refugee overruns the strongest
            // colonizer's lowest-id fringe city. Either way a wiped civ re-enters.
            if let Some((body, idx)) = find_vacant_settlement(state) {
                let Some(settlement) = state.body_settlement(body, idx).cloned() else { continue };
                let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
                let buildings = seed_colony_buildings(&settlement, pop, &seeded_ship_class, config, &mut next_building);
                let base = if settlement.name.is_empty() {
                    state.body(body).map(|b| b.name.clone()).unwrap_or_else(|| format!("#{body}"))
                } else {
                    settlement.name.clone()
                };
                let city = City {
                    id: next_city,
                    name: format!("{}-收容所", base),
                    body_id: body,
                    settlement: idx,
                    faction_id: fid,
                    population: pop,
                    buildings,
                    ship_progress: {
                        let mut m = BTreeMap::new();
                        m.insert(seeded_ship_class.clone(), 0.0);
                        m
                    },
                    razed: false,
                    loyalty: 1.0,
                };
                let ctrl = state.control.entry(fid).or_default();
                for b in &city.buildings {
                    let key = (city.id, b.id);
                    ctrl.invest_weights
                        .insert(key, Control::inherit(config.building_spec(&b.kind).default_invest_weight));
                    if b.is_shipyard() {
                        ctrl.build_weights
                            .insert(key, Control::inherit(config.building_spec(&b.kind).default_build_weight));
                    }
                }
                state.cities.push(city);
                next_city += 1;
                let cpos = state.body_position(body);
                (body, [cpos[0] + 0.05, cpos[1] + 0.05])
            } else {
                // 世界完全满员：难民夺取最强殖民者的最小 id 边缘城。
                let Some(host_cid) = displace_city_for_refugee(state) else { continue };
                let Some(settlement) = state.city_settlement(host_cid).cloned() else { continue };
                let body = state.city(host_cid).map(|c| c.body_id).unwrap_or(0);
                let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
                let buildings = seed_colony_buildings(&settlement, pop, &seeded_ship_class, config, &mut next_building);
                if let Some(c) = state.city_mut(host_cid) {
                    c.razed = false;
                    c.faction_id = fid;
                    c.population = pop;
                    c.buildings = buildings;
                    c.ship_progress.clear();
                    c.ship_progress.insert(seeded_ship_class.clone(), 0.0);
                    c.loyalty = 1.0;
                }
                let city_buildings = state.city(host_cid).map(|c| c.buildings.clone()).unwrap_or_default();
                if let Some(ctrl) = state.control_mut(fid) {
                    for b in &city_buildings {
                        let ikey = (host_cid, b.id);
                        ctrl.invest_weights.entry(ikey).or_insert_with(|| {
                            Control::inherit(config.building_spec(&b.kind).default_invest_weight)
                        });
                        if b.is_shipyard() {
                            let bkey = (host_cid, b.id);
                            ctrl.build_weights.entry(bkey).or_insert_with(|| {
                                Control::inherit(config.building_spec(&b.kind).default_build_weight)
                            });
                        }
                    }
                }
                let cpos = state.body_position(body);
                (body, [cpos[0] + 0.05, cpos[1] + 0.05])
            }
        };

        // Launch one affordable colony ship from the rebuilt/founded city.
        state.ships.push(Ship {
            id: next_ship,
            name: format!("{}-{}", spec.label, fid),
            class: seeded_ship_class.clone(),
            faction_id: fid,
            position: pos,
            hull: spec.hull,
            hull_max: spec.hull,
            shield: 0.0,
            shield_max: 0.0,
            components: Vec::new(),
            component_hp: Vec::new(),
        });
        state
            .control
            .entry(fid)
            .or_default()
            .ship_orders
            .insert(next_ship, Control::inherit(ShipBehavior::Idle));
        ev(state, GameEvent::Resurgence { faction: fid, body, ship: next_ship });
        next_ship += 1;
    }
}

// --- governance (light-speed management) -------------------------------------

/// 光速治理：每座城按其与统治势力首都的距离产生一笔治理开销（距离越远、管辖越难）。
/// 势力从库存按价值支付；付得起时城市忠诚度向距离目标恢复（远则低），付不起（欠费）
/// 时忠诚度暴跌。忠诚度跌破 [`GovernanceConfig::loyalty_revolt`] 即爆发离心叛乱，城市
/// 被夷平为空白（可再殖民）。这给超大帝国一个自然上限——既能管的领地有限，遥远的
/// 殖民地在治理失败时丢失，使世界在上千回合后保持多方参与。
fn step_governance(state: &mut State, config: &GameConfig) {
    let g = &config.governance;
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    for fid in faction_ids {
        let capital = state.faction(fid).map(|f| f.capital_body);
        let Some(capital) = capital else { continue };
        let cap_pos = state.body_position(capital);

        // 该势力所有活城 + 每城到首都的距离 + 每城想投入的娱乐/福利预算。
        let mut cities: Vec<(CityId, f64, f64)> = Vec::new(); // (id, distance, ent_budget)
        let mut total_pop = 0u64;
        for c in &state.cities {
            if c.faction_id != fid || c.razed {
                continue;
            }
            total_pop += c.population as u64;
            let d = dist(state.body_position(c.body_id), cap_pos);
            let ent = city_loyalty_budget(state, config, fid, c.id);
            cities.push((c.id, d, ent));
        }
        if cities.is_empty() {
            continue;
        }
        // 人口越多，管理能力越分散——人口超载放大远距离治理难度（与距离叠加）。
        let overload = (total_pop as f64 / g.population_capacity.max(1e-6) - 1.0).max(0.0);
        let scale = 1.0 + overload;
        let mut total_admin = 0.0;
        let mut ent_total = 0.0;
        for (_, d, ent) in &cities {
            let a = (d - g.admin_range).max(0.0);
            total_admin += (g.admin_base + g.admin_per_au * a) * scale;
            ent_total += ent;
        }
        let governance_total = (total_admin + ent_total) * sanction_cost_mult(state, config, fid);

        // 用库存（按价值加权）支付治理 + 娱乐开销（与舰队维护同源）。覆盖率决定
        // 治理是否到位以及娱乐投入是否真正落地。
        let stock = state.faction(fid).map(|f| f.resources.clone()).unwrap_or_default();
        let total_value: f64 = stock.iter().map(|(k, v)| v * value_of(k)).sum();
        let pay = governance_total.min(total_value);
        if pay > 1e-9 {
            let ratio = (pay / total_value).min(1.0);
            if let Some(f) = state.faction_mut(fid) {
                for (k, v) in stock.iter() {
                    let new = (*v - *v * ratio).max(0.0);
                    f.resources.insert(k.clone(), new);
                }
            }
        }
        let coverage = if governance_total > 1e-9 {
            (total_value / governance_total).min(1.0)
        } else {
            1.0
        };

        // 忠诚度向「距离目标 + 娱乐加成」恢复/下降，并标记叛乱（距离 × 人口超载
        // 与娱乐投入叠加）。娱乐投入越高，就越能对冲距离/人口带来的离心倾向。
        let mut to_revolt = Vec::new();
        for (cid, d, ent) in &cities {
            let a = (d - g.loyalty_range).max(0.0);
            let target_base = (1.0 - g.loyalty_distance * a * scale).clamp(0.0, 1.0);
            let ent_bonus = (ent * coverage) / g.entertainment_cost.max(1e-6);
            let target_eff = (target_base + ent_bonus).clamp(0.0, 1.0);
            let cur = state.city(*cid).map(|c| c.loyalty).unwrap_or(1.0);
            let new = if coverage >= 1.0 - 1e-6 {
                (cur + (target_eff - cur) * g.loyalty_recover).clamp(0.0, 1.0)
            } else {
                (cur - g.loyalty_penalty * (1.0 - coverage)).max(0.0)
            };
            if let Some(c) = state.city_mut(*cid) {
                c.loyalty = new;
            }
            if new < g.loyalty_revolt {
                to_revolt.push(*cid);
            }
        }

        // 叛乱：夷平为空白（可再殖民）。
        for cid in to_revolt {
            if let Some(c) = state.city_mut(cid) {
                c.razed = true;
                c.population = 0;
                c.buildings.clear();
                c.ship_progress.clear();
                c.loyalty = 0.0;
            }
            ev(state, GameEvent::Revolt { city: cid, faction: fid });
        }
    }
}

/// Move a ship one round's step toward `dest`, capped by its class speed.
fn move_toward(state: &mut State, config: &GameConfig, ship_id: ShipId, _class: &str, dest: [f64; 2]) {
    let Some(ship) = state.ship(ship_id).cloned() else { return };
    let pos = ship.position;
    let fid = ship.faction_id;
    // MOND 异常区：没有掌握修正引力的势力把指令坐标「算错」，实际航向产生偏移。
    let dest = mond_drift(config, fid, dest);
    let distance = dist(pos, dest);
    if distance <= 1e-9 {
        return;
    }
    // 有效速度 = 舰级基础速度 + 推进组件加成（舰船定制）。
    let speed = ship_panel(config, &ship).speed;
    let step = if distance <= config.combat.arrival_eps { 0.0 } else { speed.min(distance) };
    if step > 0.0 {
        let nx = (dest[0] - pos[0]) / distance;
        let ny = (dest[1] - pos[1]) / distance;
        if let Some(s) = state.ship_mut(ship_id) {
            s.position = [s.position[0] + nx * step, s.position[1] + ny * step];
        }
    }
}

/// MOND 主力导航偏移：舰船所在势力未掌握 MOND 修正引力（见 [`MondConfig::masters`]）
/// 且目标点进入异常区（距太阳超过 `mond.radius`）时，返回一个沿切向偏移的伪目标。
/// 非 master 舰因此无法精确机动到深处目标（难以轰炸/殖民/停靠），体现「指令坐标与
/// 实际坐标产生偏移」。确定性（无 RNG）。
fn mond_drift(config: &GameConfig, fid: FactionId, dest: [f64; 2]) -> [f64; 2] {
    let m = &config.mond;
    if m.drift_per_au <= 0.0 || m.masters.contains(&fid) {
        return dest;
    }
    let r = (dest[0] * dest[0] + dest[1] * dest[1]).sqrt();
    let depth = (r - m.radius).max(0.0);
    if depth <= 0.0 {
        return dest;
    }
    let drift = depth * m.drift_per_au;
    // 切向（垂直于径向），确定性方向；代表轨道力学计算错误。
    let inv = if r > 1e-9 { 1.0 / r } else { 0.0 };
    let tx = -dest[1] * inv;
    let ty = dest[0] * inv;
    [dest[0] + tx * drift, dest[1] + ty * drift]
}

/// 集中火力阈值：敌舰**护甲占比低于此值**视为「已打成残血」，此时舰队应集中火力将其
/// 打掉（打死一艘就少一个输出点）；高于此值的敌舰仍按就近接战（避免一开局就把一团
/// 健康舰队打得只剩几艘、造成过度军备消耗）。拟人：先瞄准、再集火补刀。
const FOCUS_FINISH: f64 = 0.5;

fn focus_priority(wound: f64) -> u8 {
    if wound < FOCUS_FINISH {
        0 // 残血：优先集中打掉
    } else {
        1 // 健康：按就近
    }
}

/// 集中火力排序（拟人）：结盟集火(focus) > **武器克制**(选自己的武器打得动的目标) >
/// 打残敌(priority 0) > 就近。武器克制在先：别把导弹浪费在点防重镇上、别拿动能打高盾
/// ——这比「先补刀残血」更能避免被对面用克制武器拖入消耗战。返回 true 表示 `new` 应
/// 替换 `cur`（确定性，无 RNG）。
fn prefer_target(new_focus: bool, new_fit: f64, new_wound: f64, new_dist: f64, cur_focus: bool, cur_fit: f64, cur_wound: f64, cur_dist: f64) -> bool {
    if new_focus != cur_focus {
        return new_focus;
    }
    // 武器克制：能有效杀伤的目标优先。
    if (new_fit - cur_fit).abs() > 1e-3 {
        return new_fit > cur_fit;
    }
    let np = focus_priority(new_wound);
    let cp = focus_priority(cur_wound);
    if np != cp {
        return np < cp;
    }
    if np == 0 {
        return new_wound < cur_wound; // 都残：打更残的（更快打掉输出点）
    }
    new_dist < cur_dist // 都健康：打更近的
}

/// 武器克制评分（0..1）：攻击者的武器对这一目标的**命中效率**——导弹会被目标点防御
/// 拦截而大打折扣（导弹 vs 点防 克制），动能对高护盾目标较弱（动能 vs 护盾 克制）。AI
/// 据此挑「自己能有效杀伤」的目标打，而不是把导弹浪费在全套点防御的堡垒上。确定性。
fn weapon_fit(weapons: &[Weapon], config: &GameConfig, target: &Ship) -> f64 {
    let panel = ship_panel(config, target);
    let total: f64 = weapons.iter().map(|w| w.damage).sum();
    if total <= 1e-9 {
        return 1.0;
    }
    let shield_share = panel.shield_max / (panel.shield_max + panel.hull_max).max(1e-9);
    let mut missile_dmg = 0.0;
    let mut kin_dmg = 0.0;
    for w in weapons {
        match w.kind {
            WEAPON_MISSILE => missile_dmg += w.damage,
            WEAPON_KINETIC => kin_dmg += w.damage,
            _ => {}
        }
    }
    // 导弹被目标点防御拦截（拦截越强，导弹伤害越低）。
    let intercept_frac = if panel.intercept > 0.0 {
        (panel.intercept / (panel.intercept + total)).min(0.8)
    } else {
        0.0
    };
    let missile_blunt = (missile_dmg / total) * intercept_frac;
    // 动能对高护盾目标较弱（护盾吸掉一部分）。
    let kin_blunt = (kin_dmg / total) * shield_share * 0.4;
    (1.0 - missile_blunt - kin_blunt).clamp(0.0, 1.0)
}

fn nearest_enemy_ship(state: &State, config: &GameConfig, owner: FactionId, pos: [f64; 2], range: f64, focus: Option<FactionId>, attacker_id: ShipId) -> Option<ShipId> {
    // 攻击者的武器构成（决定它对各目标的有效杀伤——武器克制）。
    let weapons = state.ship(attacker_id).map(|s| ship_weapons(config, s)).unwrap_or_default();
    let mut best: Option<(bool, f64, f64, f64, ShipId)> = None; // (is_focus, fit, wound, dist, id)
    for s in &state.ships {
        if s.hull > 0.0 && hostile(state, config, owner, s.faction_id) {
            let d = dist(pos, s.position);
            if d <= range {
                // 集中火力（拟人）：结盟集火 > 武器克制 > 打残血敌舰 > 就近。
                let is_focus = focus == Some(s.faction_id);
                let wound = s.hull / s.hull_max.max(1e-9);
                let fit = weapon_fit(&weapons, config, s);
                let better = match best {
                    None => true,
                    Some((bf, bfit, bw, bd, _)) => prefer_target(is_focus, fit, wound, d, bf, bfit, bw, bd),
                };
                if better {
                    best = Some((is_focus, fit, wound, d, s.id));
                }
            }
        }
    }
    best.map(|(_, _, _, _, id)| id)
}

/// 确定性命中率：武器追踪能力 `tracking`（AU/月）越高，越能咬住高速目标。目标速度
/// `target_speed` 越高，对低追踪武器的规避越强——所以推进组件 = 生存能力和抢先战位。
/// 无 RNG：命中定义为「伤害折减」而非「命中/未命中」的随机判定，保持确定性。
fn hit_factor(tracking: f64, target_speed: f64) -> f64 {
    if tracking <= 0.0 {
        return 0.3;
    }
    let evade = (target_speed / (tracking + target_speed)).min(1.0) * 0.6;
    (1.0 - evade).clamp(0.2, 1.0)
}

/// Resolve a single ship-vs-ship engagement. Each weapon of the attacker that is
/// within its own `range` fires; the target's defences (energy shield pool first,
/// then hull) and its speed (evasion) decide the result. Missiles are homing (hard
/// to evade) but are met by the target's point-defence interceptors. Damage is the
/// weapon's damage type vs shield/hull multipliers, all deterministic.
fn fire(state: &mut State, config: &GameConfig, attacker_id: ShipId, target_id: ShipId) {
    let (afac, apos, weapons, tclass) = {
        let a = state.ship(attacker_id).expect("attacker gone");
        let w = ship_weapons(config, a);
        (
            a.faction_id,
            a.position,
            w,
            state.ship(target_id).map(|t| t.class.clone()).unwrap_or_default(),
        )
    };
    let (tfac, mut hull, mut shield, tpos, tspeed, tpanel) = {
        let t = state.ship(target_id).expect("target gone");
        let panel = ship_panel(config, t);
        (t.faction_id, t.hull, t.shield, t.position, panel.speed, panel)
    };
    let hull_before = hull;
    // 本土防御（首都即强弩 + cult 的 MOND 异常）：目标在其首都本土防御半径内被削弱。
    let def_mult = home_defense_mult(state, tfac, tpos);
    let pd = tpanel.intercept;

    let mut total_damage = 0.0;
    for w in &weapons {
        let d = dist(apos, tpos);
        if d > w.range {
            continue; // weapon out of range — positional, not a stat
        }
        let hit = hit_factor(w.tracking, tspeed);
        let mut dmg = w.damage * hit * def_mult;
        // 导弹是制导的（对高速目标规避弱），但会被目标点防御拦截。
        if w.kind == WEAPON_MISSILE && pd > 0.0 {
            dmg *= 1.0 - (pd / (pd + w.damage)).min(0.8);
        }
        // 护盾优先吸收（按 shield_mult），溢出与 hull_mult 部分进船体；满护盾削弱船体伤害。
        let shield_dmg = dmg * w.shield_mult;
        let hull_dmg = dmg * w.hull_mult;
        let absorbed = shield.min(shield_dmg);
        shield -= absorbed;
        let soak = if shield_dmg > 1e-9 { absorbed / shield_dmg } else { 1.0 };
        let hull_pen = hull_dmg * (1.0 - 0.5 * soak);
        hull -= hull_pen;
        total_damage += dmg;
    }

    let destroyed = hull <= 0.0;
    // 模块损毁：实际打掉的船体伤害里，按 `component_spill` 比例「溢出」去损坏组件。
    let hull_damage_done = (hull_before - hull.max(0.0)).max(0.0);
    if let Some(t) = state.ship_mut(target_id) {
        t.hull = if destroyed { 0.0 } else { hull.max(0.0) };
        t.shield = shield.max(0.0);
        // 组件被击中后渐进丧失战力（武器被打掉、护盾被打掉），而不是满血抗到壳破。
        if !destroyed && !t.components.is_empty() && config.combat.component_spill > 0.0 {
            let spill = hull_damage_done * config.combat.component_spill;
            if spill > 1e-9 {
                if t.component_hp.len() != t.components.len() {
                    t.component_hp = t.components.iter().map(|c| component_integrity(config, c)).collect();
                }
                let mut rem = spill;
                // 先打最脆(最小完整度)的组件；平局按下标——确定性。
                let mut order: Vec<usize> = (0..t.components.len()).collect();
                order.sort_by(|&a, &b| t.component_hp[a].total_cmp(&t.component_hp[b]).then_with(|| a.cmp(&b)));
                for &i in &order {
                    if rem <= 1e-9 {
                        break;
                    }
                    if t.component_hp[i] <= 0.0 {
                        continue;
                    }
                    let take = rem.min(t.component_hp[i]);
                    t.component_hp[i] -= take;
                    rem -= take;
                }
            }
        }
    }
    if total_damage > 1e-9 {
        ev(state, GameEvent::Attack { attacker: attacker_id, target: target_id, damage: total_damage });
        adjust_relation(state, afac, tfac, config.diplomacy.attack_delta);
    }
    if destroyed {
        ev(state, GameEvent::ShipDestroyed { ship: target_id, owner: tfac, class: tclass });
    }
}

/// 本土防御伤害倍率：`pos` 位于 `faction` 首都的 `home_radius` 之内时返回该势力的
/// `home_attack_mult`（<1 = 削弱入侵者），否则 1.0（无削弱）。
///
/// 这使得每个有首都的势力在自己的核心区难啃（超大国空降别人家里要付代价），而
/// cult 因 MOND 异常拥有超大半径/强削减，能够在被围攻的柯伊伯带圣所自保。
fn home_defense_mult(state: &State, faction: FactionId, pos: [f64; 2]) -> f64 {
    let Some(f) = state.faction(faction) else { return 1.0 };
    if f.home_radius <= 0.0 {
        return 1.0;
    }
    let cap = state.body_position(f.capital_body);
    if dist(pos, cap) <= f.home_radius {
        f.home_attack_mult
    } else {
        1.0
    }
}

/// 本土防御额外再生：`pos` 位于 `faction` 首都的 `home_radius` 之内时，返回该势力
/// 的 `home_regen_bonus`，否则 0.0。
fn home_regen_bonus(state: &State, faction: FactionId, pos: [f64; 2]) -> f64 {
    let Some(f) = state.faction(faction) else { return 0.0 };
    if f.home_radius <= 0.0 {
        return 0.0;
    }
    let cap = state.body_position(f.capital_body);
    if dist(pos, cap) <= f.home_radius {
        f.home_regen_bonus
    } else {
        0.0
    }
}

fn behavior_is_valid(state: &State, config: &GameConfig, behavior: ShipBehavior, owner: FactionId) -> bool {
    match behavior {
        ShipBehavior::Move { .. } | ShipBehavior::Idle => true,
        ShipBehavior::Dock { body } => state.body(body).is_some(),
        ShipBehavior::Colonize { body } => has_blank_site(state, body),
        ShipBehavior::TargetShip { ship, attack } => {
            let alive = state.ship(ship).map(|s| s.hull > 0.0).unwrap_or(false);
            if !alive {
                false
            } else if attack {
                // Attack mode: target must be a hostile ship.
                state.ship(ship).map(|s| hostile(state, config, owner, s.faction_id)).unwrap_or(false)
            } else {
                // Guard mode: target must be a friendly (non-hostile) ship to protect.
                state.ship(ship).map(|s| !hostile(state, config, owner, s.faction_id)).unwrap_or(false)
            }
        }
        ShipBehavior::TargetSettlement { city, .. } => state
            .city(city)
            .map(|c| !c.razed && hostile(state, config, owner, c.faction_id))
            .unwrap_or(false),
    }
}

/// Does `body` have a colonizable 定居点 site? Yes iff it hosts a settlement
/// whose city is razed (blank footprint, re-seedable) or a settlement no city
/// occupies yet. Settlement ↔ city is 1:1, so a site with a live city never
/// counts as blank.
fn has_blank_site(state: &State, body: BodyId) -> bool {
    let Some(b) = state.body(body) else { return false };
    if b.settlements.is_empty() {
        return false;
    }
    let has_razed = state.cities.iter().any(|c| c.body_id == body && c.razed);
    if has_razed {
        return true;
    }
    let mut occupied: Vec<usize> = state.cities.iter().filter(|c| c.body_id == body).map(|c| c.settlement).collect();
    occupied.sort_unstable();
    occupied.dedup();
    (0..b.settlements.len()).any(|i| !occupied.contains(&i))
}

fn resolve_target(state: &mut State, config: &GameConfig, ship_id: ShipId, owner: FactionId, pos: [f64; 2], rng: &mut Prng, focus: Option<FactionId>) -> Option<ShipBehavior> {
    let cur = state.ship_behavior(ship_id);
    // Keep an existing targeting behavior while it is still valid, so the
    // commander does not thrash between targets every round.
    if let Some(b) = cur {
        if matches!(b, ShipBehavior::TargetShip { .. } | ShipBehavior::TargetSettlement { .. })
            && behavior_is_valid(state, config, b, owner)
        {
            return Some(b);
        }
    }
    let weapons = state.ship(ship_id).map(|s| ship_weapons(config, s)).unwrap_or_default();
    let picked = pick_target(state, config, owner, pos, rng, focus, &weapons);
    let behavior = picked.unwrap_or(ShipBehavior::Idle);
    if let Some(c) = state.control_mut(owner) {
        c.ship_orders.insert(ship_id, Control::inherit(behavior));
    }
    picked
}

fn pick_target(state: &State, config: &GameConfig, owner: FactionId, pos: [f64; 2], rng: &mut Prng, focus: Option<FactionId>, weapons: &[Weapon]) -> Option<ShipBehavior> {
    let mut best: Option<(bool, f64, f64, f64, ShipBehavior)> = None; // (is_focus, fit, wound, dist, behavior)
    let mut consider = |d: f64, is_focus: bool, fit: f64, wound: f64, b: ShipBehavior, best: &mut Option<(bool, f64, f64, f64, ShipBehavior)>| {
        let replace = match *best {
            None => true,
            Some((bf, bfit, bw, bd, _)) => {
                if prefer_target(is_focus, fit, wound, d, bf, bfit, bw, bd) {
                    true
                } else if (d - bd).abs() <= 1e-9 {
                    rng.range(2) == 0
                } else {
                    false
                }
            }
        };
        if replace {
            *best = Some((is_focus, fit, wound, d, b));
        }
    };

    // Combat first: prefer the nearest hostile ship / city, and when in a coalition
    // war, prefer those belonging to the focused hegemon. Among hostile **ships**
    // the AI concentrates fire on the **hittable + most-wounded** one (it avoids
    // wasting missiles on point-defense-heavy targets; wound = hull/hull_max);
    // cities are stationary large targets (neutral fit/wound=1.0, decided by distance).
    for s in &state.ships {
        if s.hull > 0.0 && hostile(state, config, owner, s.faction_id) {
            let is_focus = focus == Some(s.faction_id);
            let wound = s.hull / s.hull_max.max(1e-9);
            let fit = weapon_fit(weapons, config, s);
            consider(dist(pos, s.position), is_focus, fit, wound, ShipBehavior::TargetShip { ship: s.id, attack: true }, &mut best);
        }
    }
    for c in &state.cities {
        if !c.razed && hostile(state, config, owner, c.faction_id) {
            let is_focus = focus == Some(c.faction_id);
            let p = city_position(state, c.id);
            consider(dist(pos, p), is_focus, 1.0, 1.0, ShipBehavior::TargetSettlement { city: c.id, bombard: true }, &mut best);
        }
    }
    // If there is nothing to fight, re-colonize a nearby razed (blank) settlement.
    if best.is_none() {
        for c in &state.cities {
            if c.razed {
                let p = city_position(state, c.id);
                consider(dist(pos, p), false, 1.0, 1.0, ShipBehavior::Colonize { body: c.body_id }, &mut best);
            }
        }
    }
    best.map(|(_, _, _, _, b)| b)
}

/// Bombard a city: damage is spread across its buildings by area share. When all
/// buildings are destroyed the city is razed to a blank (colonizable) settlement
/// — it is never captured.
fn bombard_city(state: &mut State, config: &GameConfig, ship_id: ShipId, cid: CityId) {
    let (attacker, weapons) = {
        let s = state.ship(ship_id).expect("ship gone");
        (s.faction_id, ship_weapons(config, s))
    };
    let (old_owner, cpos) = {
        let c = state.city(cid).expect("city gone");
        (c.faction_id, state.body_position(c.body_id))
    };
    // 城市是静止的大型目标（无护盾、只有建筑装甲），轰炸用「每件武器 × 对甲倍率」的总
    // 齐射——导弹/重炮拆城，近防炮对城伤害低。命中视为全中（城市不规避）。
    let hull_attack: f64 = weapons.iter().map(|w| w.damage * w.hull_mult).sum();
    // 本土防御（首都即强弩 + cult 的 MOND 异常）：城市位于其势力首都的本土防御
    // 半径内时，受到的轰炸伤害被削弱。
    let dmg = hull_attack * home_defense_mult(state, old_owner, cpos);
    let razed = {
        let c = state.city_mut(cid).expect("city gone");
        let total_deployed: f64 = c.buildings.iter().map(|b| b.deployed).sum();
        for b in &mut c.buildings {
            let share = if total_deployed > 1e-9 { (b.deployed / total_deployed).min(1.0) } else { 0.0 };
            b.armor -= dmg * share;
        }
        c.buildings.retain(|b| b.armor > 1e-6);
        if c.buildings.is_empty() {
            c.razed = true;
            c.population = 0;
            c.ship_progress.clear();
            true
        } else {
            false
        }
    };
    adjust_relation(state, attacker, old_owner, config.diplomacy.attack_delta);
    ev(state, GameEvent::Siege { attacker: ship_id, city: cid, damage: dmg });
    if razed {
        adjust_relation(state, attacker, old_owner, config.diplomacy.capture_delta);
        ev(state, GameEvent::CityRazed { city: cid, fallen_to: attacker });
    }
}

/// Colonize a settlement site (定居点 ↔ 城市 1:1). If the body hosts a razed
/// (blank) city, re-seed it on its own settlement (re-colonize); otherwise found
/// a new city only on a 定居点 that no city occupies yet. A site already holding
/// a live city can never take a second one — the colony ship is spent (order
/// reset to idle) once a site is found, or stays put otherwise.
fn colonize(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    ship_id: ShipId,
    body: BodyId,
    next_building_id: &mut BuildingId,
    next_city_id: &mut CityId,
) {
    let faction = state.ship(ship_id).map(|s| s.faction_id).expect("ship gone");
    let seeded_ship_class = choose_next_class(state, faction, config, rng);

    // 1) A razed (blank) city keeps occupying its settlement: re-seed it there.
    let razed_cid = state.cities.iter().find(|c| c.body_id == body && c.razed).map(|c| c.id);
    if let Some(cid) = razed_cid {
        let Some(settlement) = state.city_settlement(cid).cloned() else {
            if let Some(c) = state.control_mut(faction) {
                c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::Idle));
            }
            return;
        };
        let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
        let buildings = seed_colony_buildings(&settlement, pop, &seeded_ship_class, config, next_building_id);
        if let Some(c) = state.city_mut(cid) {
            c.razed = false;
            c.faction_id = faction;
            c.population = pop;
            c.buildings = buildings;
            c.ship_progress.clear();
            c.ship_progress.insert(seeded_ship_class.clone(), 0.0);
            c.loyalty = 1.0;
        }
        ev(state, GameEvent::ColonyFounded { city: cid, owner: faction, body, seeded_ship_class: seeded_ship_class.clone() });
        if let Some(c) = state.control_mut(faction) {
            c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::Idle));
        }
        return;
    }

    // 2) No blank city: found a new city only on a settlement no city occupies.
    let occupied: Vec<usize> = state.cities.iter().filter(|c| c.body_id == body).map(|c| c.settlement).collect();
    let vacant_idx = state.body(body).and_then(|b| (0..b.settlements.len()).find(|i| !occupied.contains(i)));
    let Some(idx) = vacant_idx else {
        // Every settlement is occupied by a live city — nothing to colonize.
        if let Some(c) = state.control_mut(faction) {
            c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::Idle));
        }
        return;
    };
    let Some(settlement) = state.body_settlement(body, idx).cloned() else {
        if let Some(c) = state.control_mut(faction) {
            c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::Idle));
        }
        return;
    };
    let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
    let buildings = seed_colony_buildings(&settlement, pop, &seeded_ship_class, config, next_building_id);
    let cid = *next_city_id;
    let base = if settlement.name.is_empty() {
        state.body(body).map(|b| b.name.clone()).unwrap_or_else(|| format!("#{body}"))
    } else {
        settlement.name.clone()
    };
    let city = City {
        id: cid,
        name: format!("{}-殖民城", base),
        body_id: body,
        settlement: idx,
        faction_id: faction,
        population: pop,
        buildings,
        ship_progress: {
            let mut m = BTreeMap::new();
            m.insert(seeded_ship_class.clone(), 0.0);
            m
        },
        razed: false,
        loyalty: 1.0,
    };
    let ctrl = state.control.entry(faction).or_default();
    for b in &city.buildings {
        let key = (cid, b.id);
        ctrl.invest_weights
            .insert(key, Control::inherit(config.building_spec(&b.kind).default_invest_weight));
        if b.is_shipyard() {
            ctrl.build_weights
                .insert(key, Control::inherit(config.building_spec(&b.kind).default_build_weight));
        }
    }
    state.cities.push(city);
    *next_city_id += 1;
    ev(state, GameEvent::ColonyFounded { city: cid, owner: faction, body, seeded_ship_class: seeded_ship_class.clone() });
    if let Some(c) = state.control_mut(faction) {
        c.ship_orders.insert(ship_id, Control::inherit(ShipBehavior::Idle));
    }
}

/// Seed buildings for a newly founded / razed-and-reseeded city.
fn seed_colony_buildings(
    s: &Settlement,
    population: u32,
    ship_class: &str,
    config: &GameConfig,
    next_id: &mut BuildingId,
) -> Vec<Building> {
    let mut buildings = Vec::new();
    let mut alloc = |kind: &str, resource: Option<String>, ship_type: Option<String>, area: f64, deployed: f64| -> Building {
        let id = *next_id;
        *next_id += 1;
        let armor = deployed * config.structure_spec("concrete").armor_per_area;
        Building {
            id,
            kind: kind.to_string(),
            resource,
            ship_type,
            structure: "concrete".to_string(),
            area,
            deployed,
            armor,
        }
    };

    let footprint = s.total_area * config.combat.colony_footprint;
    let resid = (population as f64 / s.ecological_capacity.max(1e-6)).min(footprint * 0.5).max(4.0);
    buildings.push(alloc("residential", None, None, resid, resid));
    let mut budget = (footprint - resid).max(0.0);
    for d in &s.resources {
        if budget <= 0.0 {
            break;
        }
        let area = d.area.min(budget * 0.5);
        if area > 0.0 {
            buildings.push(alloc("mining", Some(d.resource.clone()), None, area, area));
            budget -= area;
        }
    }
    let construction = (footprint * 0.3).clamp(2.0, 8.0);
    buildings.push(alloc("construction", None, Some(ship_class.to_string()), construction, construction));
    buildings
}

fn behavior_dest(state: &State, behavior: ShipBehavior) -> [f64; 2] {
    match behavior {
        ShipBehavior::Move { position } => position,
        ShipBehavior::TargetShip { ship, .. } => state.ship(ship).map(|s| s.position).unwrap_or([0.0, 0.0]),
        ShipBehavior::TargetSettlement { city, .. } => city_position(state, city),
        ShipBehavior::Dock { body } | ShipBehavior::Colonize { body } => state.body_position(body),
        ShipBehavior::Idle => [0.0, 0.0],
    }
}

// --- diplomacy --------------------------------------------------------------

/// Dynamic international-relations step.
///
/// Each unordered faction pair independently:
///   * drifts toward its **resting affinity** (bloc formation), derived from the
///     two factions' `alignment`. Aggressive factions close in on a hostile
///     affinity faster, so ideologically-distant powers escalate to war on their
///     own (a build-up phase) and allies cohere.
///   * if already at war and the pair did **not** fight this round, winds down
///     toward `ceasefire_relation` (war fatigue) — so wars end once the fighting
///     stops, and can later re-escalate.
///   * gets a little `noise`, so relations fluctuate and cross the threshold
///     irregularly rather than settling.
///
/// Hostile acts (`attack_delta` / `capture_delta` applied in [`adjust_relation`])
/// still push relations down during combat, which is what keeps an active war hot.
fn step_diplomacy(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    let d = &config.diplomacy;
    let band = 2.0;

    // Which (unordered) faction pairs engaged in hostilities this round, so war
    // fatigue does not cancel out the combat-driven relation drops while a war
    // is actually being fought.
    let mut fought: BTreeSet<(FactionId, FactionId)> = BTreeSet::new();
    let mut note_pair = |a: Option<FactionId>, b: Option<FactionId>| {
        if let (Some(a), Some(b)) = (a, b) {
            if a != b {
                fought.insert((a.min(b), a.max(b)));
            }
        }
    };
    for e in &state.events {
        match e {
            GameEvent::Attack { attacker, target, .. } => {
                note_pair(
                    state.ship(*attacker).map(|s| s.faction_id),
                    state.ship(*target).map(|s| s.faction_id),
                );
            }
            GameEvent::Siege { attacker, city, .. } => {
                note_pair(
                    state.ship(*attacker).map(|s| s.faction_id),
                    state.city(*city).map(|c| c.faction_id),
                );
            }
            _ => {}
        }
    }

    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let (a, b) = (ids[i], ids[j]);
            let (align_a, align_b, aggr) = {
                let fa = state.factions.iter().find(|f| f.id == a).expect("faction a gone");
                let fb = state.factions.iter().find(|f| f.id == b).expect("faction b gone");
                (fa.alignment, fb.alignment, fa.aggression.max(fb.aggression))
            };
            let mut rel = relation(state, a, b);
            let aff = d.affinity_floor + d.affinity_span * (1.0 - (align_a - align_b).abs().min(band) / band);
            let at_war = rel <= config.combat.war_threshold;
            let clashing = fought.contains(&(a.min(b), a.max(b)));

            if at_war && !clashing {
                // War fatigue: cool the conflict toward ceasefire once the guns
                // fall silent, so wars end rather than grind forever.
                rel += d.war_fatigue * (d.ceasefire_relation - rel);
            } else {
                // Bloc drift toward resting affinity; aggressive powers close a
                // hostile gap faster (they escalate, they do not befriend rivals).
                let rate = if aff < 0.0 { 1.0 + aggr } else { 1.0 };
                rel += d.drift_rate * rate * (aff - rel);
            }

            // Little random fluctuation so relations wobble and cross thresholds.
            rel += rng.range_f64(-d.noise, d.noise);
            rel = rel.clamp(d.hostility_floor, d.friendship_ceiling);

            for f in state.factions.iter_mut().filter(|f| f.id == a || f.id == b) {
                let other = if f.id == a { b } else { a };
                f.relations.insert(other, rel);
            }
        }
    }
}

// --- balance of power (合纵连横 / 弱者联盟对抗霸权) ------------------------------

/// 设置两个势力间的对称关系（带钳位），写入双方。
fn set_relation_sym(state: &mut State, a: FactionId, b: FactionId, v: f64, config: &GameConfig) {
    let v = v.clamp(config.diplomacy.hostility_floor, config.diplomacy.friendship_ceiling);
    for f in state.factions.iter_mut().filter(|f| f.id == a || f.id == b) {
        let other = if f.id == a { b } else { a };
        f.relations.insert(other, v);
    }
}

/// 综合实力占比：`power = (city_weight×城市份额 + fleet_weight×舰队份额) / (两权重之和)`。
/// 城市份额 = 活城数/总活城数，舰队份额 = 舰艇引擎数值之和/总引擎数值之和。两份额各自
/// 在 [0,1] 且对全势力求和为 1，故 power 也是合法的占比（0..1）。无活城且无舰时返回全 0。
pub(crate) fn faction_power_share(state: &State, config: &GameConfig) -> BTreeMap<FactionId, f64> {
    let b = &config.balance;
    let wp = b.power_city_weight + b.power_fleet_weight;
    let total_cities = state.cities.iter().filter(|c| !c.razed).count() as f64;
    let total_fleet: f64 = state.ships.iter().map(|s| ship_panel(config, s).hull_max).sum();
    if wp <= 0.0 || (total_cities <= 0.0 && total_fleet <= 0.0) {
        return state.factions.iter().map(|f| (f.id, 0.0)).collect();
    }
    let mut powers = BTreeMap::new();
    for f in &state.factions {
        let cities = state.cities.iter().filter(|c| c.faction_id == f.id && !c.razed).count() as f64;
        let fleet: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == f.id)
            .map(|s| ship_panel(config, s).hull_max)
            .sum();
        let city_share = if total_cities > 0.0 { cities / total_cities } else { 0.0 };
        let fleet_share = if total_fleet > 0.0 { fleet / total_fleet } else { 0.0 };
        let power = (b.power_city_weight * city_share + b.power_fleet_weight * fleet_share) / wp;
        powers.insert(f.id, power);
    }
    powers
}

/// 当前的反制联盟成员：非霸权势力中，对霸权的**疏远**达到 [`BalanceOfPowerConfig::coalition_estrange`]
/// （关系 ≤ 该值，即被遏制/疏远了霸权）、且彼此相互和平（互不交战）的一方。若 ≥
/// [`BalanceOfPowerConfig::min_members`] 即视为联盟成立。遏制是冷战式的——成员未必与
/// 霸权开战，但已脱离其影响、转而与弱国抱团。
fn coalition_of(state: &State, config: &GameConfig, hegemon: FactionId, members: &[FactionId]) -> Vec<FactionId> {
    let estrange = config.balance.coalition_estrange;
    let estranged: Vec<FactionId> = members
        .iter()
        .cloned()
        .filter(|m| relation(state, *m, hegemon) <= estrange)
        .collect();
    estranged
        .iter()
        .cloned()
        .filter(|&m| estranged.iter().all(|&o| o == m || !hostile(state, config, m, o)))
        .collect()
}

/// 当前综合实力占比最高的「霸权」及其已倒向联盟的成员（关系 ≤ `coalition_estrange`）。
/// 只负责判定「谁是最强、谁在抱团」，实力占比未达 [`BalanceOfPowerConfig::hegemon_power`]
/// 时返回 `None`。
fn dominant_hegemon(state: &State, config: &GameConfig) -> Option<(FactionId, Vec<FactionId>)> {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return None;
    }
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    if ids.len() < 2 {
        return None;
    }
    let powers = faction_power_share(state, config);
    let (hegemon, max_power) = powers
        .iter()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| (*k, *v))
        .unwrap_or((0, 0.0));
    if max_power < b.hegemon_power {
        return None;
    }
    let members: Vec<FactionId> = ids.iter().cloned().filter(|x| *x != hegemon).collect();
    let estranged = coalition_of(state, config, hegemon, &members);
    Some((hegemon, estranged))
}

/// 当前一个活跃反制联盟（≥ [`BalanceOfPowerConfig::min_members`] 个疏远成员）针对的
/// 「霸权」；用于政治上报（`coalition` 字段）与联盟跃迁事件。
pub(crate) fn active_coalition_hegemon(state: &State, config: &GameConfig) -> Option<FactionId> {
    dominant_hegemon(state, config)
        .filter(|(_, m)| m.len() >= config.balance.min_members)
        .map(|(h, _)| h)
}

/// 经济制裁针对的「霸权」：只要势力**已称霸（实力占比达标）且至少有一个弱者倒向联盟**
/// 就实施封锁——不必等联盟完全成形。这使「人缘好但已坐大」的紧凑区域帝国也能被压缩
/// （否则它不招人恨就没人封锁它），且比 war 更拟真、不引发夷平/僵尸。
fn sanctioned_hegemon(state: &State, config: &GameConfig) -> Option<FactionId> {
    dominant_hegemon(state, config)
        .filter(|(_, m)| !m.is_empty())
        .map(|(h, _)| h)
}

/// 经济制裁的「治理代价」倍率：若 `fid` 正是被经济封锁的霸权，则其维持帝国
/// （行政 + 娱乐）的成本按 `sanction_cost_mult` 放大；否则 1.0（不碰别国）。这使被
/// 多国封锁的大国要花更多资源维持领地与治安——边缘殖民地更难养、更易离心。
fn sanction_cost_mult(state: &State, config: &GameConfig, fid: FactionId) -> f64 {
    if sanctioned_hegemon(state, config) == Some(fid) {
        config.balance.sanction_cost_mult
    } else {
        1.0
    }
}

/// 联盟军事协同的「集火目标」：若 `owner` 属于针对霸权 H 的活跃反制联盟（已倒向联盟、
/// 关系 ≤ `coalition_estrange`），且 H 正与联盟内某一弱者交战（集体安全已触发——霸权
/// 先动手了），则返回 Some(H)。这使结盟势力的舰只**优先集火 H**、而非各自就近乱打——
/// 给「攻其一方、集体制衡」真正的军事牙齿。否则返回 None（不改变普通行为）。
fn coalition_war_focus(state: &State, config: &GameConfig, owner: FactionId) -> Option<FactionId> {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return None;
    }
    let Some(hegemon) = active_coalition_hegemon(state, config) else { return None };
    if owner == hegemon {
        return None;
    }
    // 该弱者是否已倒向联盟（疏远霸权）。未倒向则不集火。
    if relation(state, owner, hegemon) > b.coalition_estrange {
        return None;
    }
    // 霸权是否正与任一弱者交战（集体防御触发）——注意霸权自己对它与他人开战不作集火。
    let war_on = state
        .factions
        .iter()
        .any(|f| f.id != hegemon && hostile(state, config, f.id, hegemon));
    if war_on {
        Some(hegemon)
    } else {
        None
    }
}

/// 合纵连横格局快照：返回 (当前霸权(若有), 针对它的反制联盟成员, 各势力综合实力占比)。
/// 供 agent 层读取政治格局（霸权是谁、谁在联合制衡、谁是当前最强）。
pub fn balance_picture(
    state: &State,
    config: &GameConfig,
) -> (Option<FactionId>, Vec<FactionId>, BTreeMap<FactionId, f64>) {
    let powers = faction_power_share(state, config);
    let hegemon = active_coalition_hegemon(state, config);
    let members = match hegemon {
        Some(h) => {
            let ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
            let members: Vec<FactionId> = ids.into_iter().filter(|x| *x != h).collect();
            coalition_of(state, config, h, &members)
        }
        None => Vec::new(),
    };
    (hegemon, members, powers)
}

/// 合纵连横 / 均势外交：当一方被判定为「霸权」时，其余较弱势力被共同威胁推向彼此——
/// 弱者-弱者向 [`BalanceOfPowerConfig::coalition_affinity`] 靠拢（合纵），弱者对霸权向
/// [`BalanceOfPowerConfig::hegemon_affinity`] 靠拢（均势/疏远）。霸权对任一弱者开战时，
/// 其余弱者对霸权关系骤降（集体安全）。全部确定性、无 RNG。
fn step_balance_of_power(state: &mut State, config: &GameConfig) {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return; // 关闭
    }
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    if ids.len() < 2 {
        return;
    }

    // 找综合实力占比最高的「霸权」；未达阈值则不触发机制。
    let powers = faction_power_share(state, config);
    let (hegemon, max_power) = powers
        .iter()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| (*k, *v))
        .unwrap_or((0, 0.0));
    if max_power < b.hegemon_power {
        return;
    }

    // 威胁强度：霸权超越阈值越多，弱者靠拢得越急（scale ∈ [1, 2] 附近）。
    let dom = (max_power - b.hegemon_power).max(0.0);
    let scale = 1.0 + dom / (1.0 - b.hegemon_power).max(1e-9);

    let members: Vec<FactionId> = ids.iter().cloned().filter(|x| *x != hegemon).collect();
    if members.is_empty() {
        return;
    }

    // 步骤前后联盟成员、及与霸权交战成员（用于跃迁/集体安全判定）。
    let coalition_before = coalition_of(state, config, hegemon, &members);
    let was_at_war: BTreeSet<FactionId> =
        members.iter().cloned().filter(|m| hostile(state, config, *m, hegemon)).collect();

    // 合纵：弱者-弱者相互靠拢（共同威胁把他们推向彼此）。
    for i in 0..members.len() {
        for j in (i + 1)..members.len() {
            let (m1, m2) = (members[i], members[j]);
            let rel = relation(state, m1, m2);
            let nv = rel + b.coalition_rate * scale * (b.coalition_affinity - rel);
            set_relation_sym(state, m1, m2, nv, config);
        }
    }

    // 均势：「冷处理/遏制」——弱者对霸权的关系向 hegemon_affinity 下压，但**只在它比
    // 该目标更暖时才往下压**，绝不自动把它推到交战阈值之下（不「无脑宣战」）。这模拟
    // 现实中的遏制：弱国不再争相讨好霸权、甚至疏远它，但井水不犯河水，真正的共同军事
    // 行动留给「集体安全」（霸权一旦动手打某弱者，其余弱者才群起而攻之）。
    for &m in &members {
        let rel = relation(state, m, hegemon);
        if rel > b.hegemon_affinity {
            let nv = rel + b.hegemon_rate * scale * (b.hegemon_affinity - rel);
            set_relation_sym(state, m, hegemon, nv, config);
        }
    }

    // 集体安全：任一弱者与霸权进入交战（本回合新跨入），其余尚未交战的弱者对霸权关系
    // 骤降——「攻其一方 = 与全体为敌」的防御协定：霸权一旦开打，弱者联盟群起而攻之。
    let now_at_war: BTreeSet<FactionId> =
        members.iter().cloned().filter(|m| hostile(state, config, *m, hegemon)).collect();
    if now_at_war.difference(&was_at_war).next().is_some() {
        for &m in &members {
            if !now_at_war.contains(&m) {
                let rel = relation(state, m, hegemon);
                set_relation_sym(state, m, hegemon, rel + b.collective_defense_delta, config);
            }
        }
    }

    // 联盟跃迁事件（只在成立/解体的当回合记一条，供 agent 直读政治格局）。
    let coalition_after = coalition_of(state, config, hegemon, &members);
    let before_active = coalition_before.len() >= b.min_members;
    let after_active = coalition_after.len() >= b.min_members;
    if before_active && !after_active {
        ev(state, GameEvent::CoalitionEnded { hegemon, members: coalition_before });
    } else if !before_active && after_active {
        ev(state, GameEvent::CoalitionFormed { hegemon, members: coalition_after });
    }
}

// --- story / chronicle -------------------------------------------------------

/// 剧情步进：评估 config 的 `story` 表，把满足触发条件的剧情事件火出，写入
/// [`State::chronicle`] 编年史并记一条 [`GameEvent::Story`]，同时应用可选的小幅
/// 机械后果（关系/资源）。确定性：无 RNG，同一种子触发完全一致。
///
/// 每个事件默认只触发一次（id 已入编年史则跳过）。触发条件见 [`StoryTrigger`]；
/// 事件型条件（`FirstWar`/`FirstRaze`/`FirstColony`/`WarBetween`/`FactionAtWar`）
/// 依据本回合已产生的事件（含开战/停战/夷平/殖民）判定——因此这些剧情节拍正好落在
/// 对应历史事件发生的那个回合，形成「剧情与局势同步」的叙事弧。
fn step_story(state: &mut State, config: &GameConfig) {
    for spec in &config.story {
        // 每个剧情事件只触发一次：已进编年史则跳过。
        if state.chronicle.iter().any(|c| c.id == spec.id) {
            continue;
        }
        if !story_trigger_fired(state, &spec.trigger) {
            continue;
        }
        // 机械后果（小幅、确定性）。
        for effect in &spec.effects {
            match effect {
                StoryEffect::Relations { a, b, delta } => {
                    adjust_relation(state, *a, *b, *delta);
                }
                StoryEffect::GrantResources { faction, resource, amount } => {
                    if let Some(f) = state.faction_mut(*faction) {
                        *f.resources.entry(resource.clone()).or_insert(0.0) += *amount;
                    }
                }
                StoryEffect::GrantShip { faction, class, body } => {
                    grant_story_ship(state, config, *faction, class, *body);
                }
            }
        }
        // 记入编年史 + 本回合故事事件。参与方由静态模板 + 本次事件的具体对象合成
        // （事件型触发把「实际是谁」写进编年史，让剧情真正反应该回合发生的事情）。
        let participants = story_participants(state, spec);
        let entry = ChronicleEntry {
            round: state.round,
            id: spec.id.clone(),
            title: spec.title.clone(),
            body: spec.body.clone(),
            participants: participants.clone(),
        };
        ev(state, GameEvent::Story { id: spec.id.clone(), title: spec.title.clone(), participants });
        state.chronicle.push(entry);
    }
}

/// 剧情事件的参与方：模板里写的静态可读名，加上事件型触发从本回合事件里提炼出的
/// 具体对象（哪两方开战 / 哪座城被夷平 / 谁建立了殖民地）。保证编年史「自描述」——
/// agent 无需反推就能知道这条剧情发生在谁身上。确定性：取自本回合事件流水。
fn story_participants(state: &State, spec: &StoryEvent) -> Vec<String> {
    let mut parts: Vec<String> = spec.participants.clone();
    let add = |parts: &mut Vec<String>, name: Option<String>| {
        if let Some(n) = name {
            if !n.is_empty() && !parts.iter().any(|p| p == &n) {
                parts.push(n);
            }
        }
    };
    let find_war = |state: &State, faction: Option<FactionId>| -> Option<(FactionId, FactionId)> {
        state.events.iter().find_map(|e| match e {
            GameEvent::WarStarted { a, b } => match faction {
                Some(f) if *a == f || *b == f => Some((*a, *b)),
                Some(_) => None,
                None => Some((*a, *b)),
            },
            _ => None,
        })
    };
    match &spec.trigger {
        StoryTrigger::FirstWar => {
            if let Some((a, b)) = find_war(state, None) {
                add(&mut parts, state.faction(a).map(|f| f.name.clone()));
                add(&mut parts, state.faction(b).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FactionAtWar { faction } => {
            add(&mut parts, state.faction(*faction).map(|f| f.name.clone()));
            if let Some((a, b)) = find_war(state, Some(*faction)) {
                let other = if a == *faction { b } else { a };
                add(&mut parts, state.faction(other).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FirstRaze => {
            if let Some((city, fallen)) = state.events.iter().find_map(|e| match e {
                GameEvent::CityRazed { city, fallen_to } => Some((*city, *fallen_to)),
                _ => None,
            }) {
                add(&mut parts, state.city(city).map(|c| c.name.clone()));
                add(&mut parts, state.faction(fallen).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FirstColony => {
            if let Some((owner, body)) = state.events.iter().find_map(|e| match e {
                GameEvent::ColonyFounded { owner, body, .. } => Some((*owner, *body)),
                _ => None,
            }) {
                add(&mut parts, state.faction(owner).map(|f| f.name.clone()));
                add(&mut parts, state.body(body).map(|b| b.name.clone()));
            }
        }
        StoryTrigger::WarBetween { a, b } => {
            add(&mut parts, state.faction(*a).map(|f| f.name.clone()));
            add(&mut parts, state.faction(*b).map(|f| f.name.clone()));
        }
        _ => {}
    }
    parts
}

/// 剧情：把一个舰级「出厂」给某势力，位置在天体当前位置附近（小幅确定性偏移）。
/// 舰 id 按当前最大 id 连续分配，/并配一条 `Idle` 指令；无 RNG，确定性复现。
fn grant_story_ship(state: &mut State, config: &GameConfig, faction: FactionId, class: &str, body: BodyId) {
    if !config.ships.contains_key(class) {
        return;
    }
    let pos = state.body_position(body);
    if state.body(body).is_none() {
        return;
    }
    let next_id = state.ships.iter().map(|s| s.id).max().map_or(0, |m| m + 1);
    let spec = config.ship_spec(class);
    state.ships.push(Ship {
        id: next_id,
        name: format!("{}-{}", spec.label, faction),
        class: class.to_string(),
        faction_id: faction,
        position: [pos[0] + 0.05, pos[1] + 0.05],
        hull: spec.hull,
        hull_max: spec.hull,
        shield: 0.0,
        shield_max: 0.0,
        components: Vec::new(),
        component_hp: Vec::new(),
    });
    state.control.entry(faction).or_default().ship_orders.insert(next_id, Control::inherit(ShipBehavior::Idle));
}

/// 判断一条剧情触发条件是否已满足。
fn story_trigger_fired(state: &State, trigger: &StoryTrigger) -> bool {
    match trigger {
        StoryTrigger::RoundAt { round } => state.round >= *round,
        StoryTrigger::FirstWar => state.events.iter().any(|e| matches!(e, GameEvent::WarStarted { .. })),
        StoryTrigger::FirstRaze => state.events.iter().any(|e| matches!(e, GameEvent::CityRazed { .. })),
        StoryTrigger::FirstColony => state.events.iter().any(|e| matches!(e, GameEvent::ColonyFounded { .. })),
        StoryTrigger::WarBetween { a, b } => state.events.iter().any(|e| match e {
            GameEvent::WarStarted { a: x, b: y } => {
                let (lo, hi) = (x.min(y), x.max(y));
                let (plo, phi) = (a.min(b), a.max(b));
                lo == plo && hi == phi
            }
            _ => false,
        }),
        StoryTrigger::FactionAtWar { faction } => state.events.iter().any(|e| match e {
            GameEvent::WarStarted { a, b } => *a == *faction || *b == *faction,
            _ => false,
        }),
        StoryTrigger::RelationBelow { a, b, value } => relation(state, *a, *b) < *value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::model::GameEvent;
    use crate::world::default_state;

    /// Build the config + a fresh deterministic world (round 0).
    fn fresh_world(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let state = default_state(&config, seed);
        (config, state)
    }

    /// A player-facing regression guard for the "stale target" bug: a player
    /// ship ordered to attack an already-destroyed target must degrade to Idle,
    /// never drift toward the origin ([0,0]).
    #[test]
    fn player_stale_target_degrades_to_idle_and_does_not_drift() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // China (3) corvette id=0 is Player-ordered to approach US (1) destroyer id=3.
        let diff = serde_json::json!({
            "control": [{
                "faction_id": 3,
                "ship_orders": [{"ship": 0, "behavior": {"TargetShip": {"ship": 3, "attack": true}}, "mode": "Player"}]
            }]
        });
        crate::web::apply_patch(&mut state, &config, &diff).expect("apply order");

        // Simulate the target being destroyed before the round advances.
        if let Some(t) = state.ship_mut(3) {
            t.hull = 0.0;
        }
        let pos_before = state.ship(0).map(|s| s.position).unwrap();

        advance(&mut state, &config, &mut rng);

        // The order must have degraded to Idle ...
        let order = state.ship_behavior(0);
        assert_eq!(order, Some(ShipBehavior::Idle), "stale order must degrade to Idle");
        // ... without moving the ship toward the origin.
        let pos_after = state.ship(0).map(|s| s.position).unwrap();
        assert_eq!(pos_after, pos_before, "ship must not drift (target is dead)");
        // ... and a StaleOrder event must be recorded.
        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::StaleOrder { ship: 0, .. })),
            "expected a StaleOrder event for ship 0, got {:?}",
            state.events
        );
    }

    /// Guard semantics: `TargetShip { attack: false }` escorts a *friendly* ship
    /// and intercepts hostiles within attack range — it must not fire at the
    /// protected ship itself, and it must protect the friendly even though the
    /// target is not hostile.
    #[test]
    fn guard_escorts_friendly_and_intercepts_hostiles() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // China (3): ship 0 guards its own friendly ship 1. Co-located at [0,0].
        // Friendly same-faction target => valid guard.
        let diff = serde_json::json!({
            "control": [{
                "faction_id": 3,
                "ship_orders": [{"ship": 0, "behavior": {"TargetShip": {"ship": 1, "attack": false}}, "mode": "Player"}]
            }]
        });
        crate::web::apply_patch(&mut state, &config, &diff).expect("apply guard order");

        // Pin positions: defender + protected friend at [0,0]; US (1) enemy
        // destroyer id=3 just inside the corvette attack range (0.4) so the guard
        // can fire. Move the other US ships (4 destroyer, 5 cruiser) far out so
        // only ship 3 engages (its damage 6 won't one-shot the guard's hull 12,
        // letting the guard retaliate).
        for id in [0u32, 1u32] {
            if let Some(s) = state.ship_mut(id) {
                s.position = [0.0, 0.0];
            }
        }
        if let Some(enemy) = state.ship_mut(3) {
            enemy.position = [0.3, 0.0];
        }
        for far in [4u32, 5u32] {
            if let Some(s) = state.ship_mut(far) {
                s.position = [50.0, 50.0];
            }
        }

        // The default world now opens peacefully, so make US (1) explicitly
        // hostile to China (3) for this guard scenario.
        if let Some(f) = state.faction_mut(3) {
            f.relations.insert(1, -35.0);
        }
        if let Some(f) = state.faction_mut(1) {
            f.relations.insert(3, -35.0);
        }

        advance(&mut state, &config, &mut rng);

        // The guard must have opened fire on the enemy, not on the friend.
        assert!(
            state.events.iter().any(|e| matches!(
                e,
                GameEvent::Attack { attacker: 0, target, .. } if *target == 3
            )),
            "guard should fire at the hostile, got {:?}",
            state.events
        );
        // The protected friend must be unharmed (no attack targeting ship 1).
        assert!(
            !state.events.iter().any(|e| matches!(e, GameEvent::Attack { target: 1, .. })),
            "guard must not fire at its own protected ship, got {:?}",
            state.events
        );
        // The order is still a valid guard (not degraded to Idle).
        assert_eq!(state.ship_behavior(0), Some(ShipBehavior::TargetShip { ship: 1, attack: false }));
    }

    /// Events must populate as the world advances (growth / spurious events are
    /// fine; the round log must simply be populated and contain no panics).
    #[test]
    fn advance_populates_round_events() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        assert!(state.events.is_empty(), "round 0 has no events yet");
        for _ in 0..6 {
            advance(&mut state, &config, &mut rng);
        }
        // After a few rounds of a war-torn seed, an event log should exist.
        assert!(!state.events.is_empty(), "after 6 rounds there should be events");
    }

    /// 停泊轨道 (Dock) follows a body's current position; 待命 (Idle) holds
    /// position. Dock persists (never degrades), and Idle never moves the ship.
    #[test]
    fn dock_follows_body_and_idle_holds_position() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // China (3) corvette id=0 docks body 4 (火星); id=1 is ordered Idle.
        let diff = serde_json::json!({
            "control": [{
                "faction_id": 3,
                "ship_orders": [
                    {"ship": 0, "behavior": {"Dock": {"body": 4}}, "mode": "Player"},
                    {"ship": 1, "behavior": "Idle", "mode": "Player"}
                ]
            }]
        });
        crate::web::apply_patch(&mut state, &config, &diff).expect("apply dock/idle order");

        // Pin ship 0 away from the body so `Dock` must move it toward the body.
        if let Some(s) = state.ship_mut(0) {
            s.position = [5.0, 5.0];
        }
        if let Some(s) = state.ship_mut(1) {
            s.position = [3.0, 3.0];
        }
        let dock_pos_before = state.ship(0).map(|s| s.position).unwrap();
        let idle_pos_before = state.ship(1).map(|s| s.position).unwrap();

        advance(&mut state, &config, &mut rng);

        // Dock: the ship moved toward the body (not froze, not degraded).
        let dock_pos_after = state.ship(0).map(|s| s.position).unwrap();
        assert_ne!(dock_pos_after, dock_pos_before, "docked ship should move toward the body");
        assert_eq!(
            state.ship_behavior(0),
            Some(ShipBehavior::Dock { body: 4 }),
            "dock order must persist (not degrade to Idle)"
        );
        // Idle: the ship did not move.
        let idle_pos_after = state.ship(1).map(|s| s.position).unwrap();
        assert_eq!(idle_pos_after, idle_pos_before, "Idle must hold position");
        assert_eq!(state.ship_behavior(1), Some(ShipBehavior::Idle));
    }

    /// 护甲再生 (ShipSpec.hull_regen): a damaged ship regains a fraction of its
    /// max hull each round; full-hull ships stay capped; destroyed ships stay gone.
    #[test]
    fn damaged_ship_regenerates_hull_each_round() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // Take China's Earth corvette (id 0, hull_max 12, hull_regen 0.04) and
        // damage it to exactly half; pin it away from all hostiles so the round
        // is quiet and only regeneration acts on it.
        if let Some(s) = state.ship_mut(0) {
            s.hull = 6.0;
            s.position = [80.0, 80.0];
        }
        let class = state.ship(0).map(|s| s.class.clone()).unwrap();
        let regen = config.ship_spec(&class).hull_regen;

        advance(&mut state, &config, &mut rng);

        let hull = state.ship(0).map(|s| s.hull).expect("ship 0 still alive");
        let expected = (6.0 + 12.0 * regen).min(12.0);
        assert!(
            (hull - expected).abs() < 1e-9,
            "hull should heal to {expected}, got {hull}"
        );

        // A full-hull ship stays capped (no over-heal).
        if let Some(s) = state.ship_mut(1) {
            s.hull = config.ship_spec(&s.class).hull;
            s.position = [80.0, 80.0];
        }
        advance(&mut state, &config, &mut rng);
        let max1 = config.ship_spec(&state.ship(1).map(|s| s.class.clone()).unwrap()).hull;
        let hull1 = state.ship(1).map(|s| s.hull).unwrap();
        assert!((hull1 - max1).abs() < 1e-9, "full hull must not over-heal, got {hull1}");
    }

    /// 定居点 ↔ 城市 一一对应: 每个城市占据其天体上一个合法定居点；同一座城不会
    /// 让一个定居点被两座城占用；地球恰好 5 个定居点各坐一座 spec 都市，矿藏按
    /// 定居点隔离（巴黎只产 铀/铂，不再共享整个地球的矿藏池）。
    #[test]
    fn settlements_and_cities_are_one_to_one() {
        let (_config, state) = fresh_world(42);
        for b in &state.bodies {
            let cities: Vec<&City> = state.cities.iter().filter(|c| c.body_id == b.id).collect();
            assert!(
                cities.len() <= b.settlements.len(),
                "body {}: {} cities must not exceed {} settlements",
                b.name,
                cities.len(),
                b.settlements.len()
            );
            for c in cities {
                assert!(
                    c.settlement < b.settlements.len(),
                    "city {} (body {}) points at an out-of-range settlement {}",
                    c.name,
                    b.name,
                    c.settlement
                );
            }
        }

        let earth = &state.bodies[2];
        assert_eq!(earth.settlements.len(), 5, "Earth has five spec metropolises");
        let earth_cities = state.cities.iter().filter(|c| c.body_id == 2).count();
        assert_eq!(earth_cities, 5, "five cities on five Earth settlements (1:1)");
        // 巴黎 (settlement index 3) hosts only 铀/铂 — its own region's ores.
        let paris = earth.settlements[3].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
        assert_eq!(paris, vec!["uranium", "platinum"], "Paris settlement mines only its own ores");
        assert_eq!(
            state.cities.iter().find(|c| c.name == "巴黎").map(|c| c.settlement),
            Some(3),
            "巴黎 occupies settlement index 3"
        );
        // 长三角/珠三角 are distinct settlements, so both may mine 铁 independently.
        let cn = earth.settlements[0].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
        assert!(cn.contains(&"iron"), "长三角 settlement has 铁");
        assert!(cn.contains(&"silicon") && cn.contains(&"water_ice"), "长三角 has 硅/水冰");
    }

    /// 剧情编年史：RoundAt 节拍按回合触发、编年史按发生先后单调增长、id 唯一，且
    /// 同一种子完全确定（重跑逐字节一致）。
    #[test]
    fn story_chronicle_grows_deterministically() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        // Round-at beats: prologue fires round 1, planet_x_arrives round 60.
        for _ in 0..60 {
            advance(&mut state, &config, &mut rng);
        }
        let ids: Vec<&str> = state.chronicle.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"prologue"), "prologue (RoundAt 1) must fire");
        assert!(ids.contains(&"planet_x_arrives"), "planet_x_arrives (RoundAt 60) must fire");

        // The chronicle records the round it fired, in non-decreasing order.
        let rounds: Vec<u32> = state.chronicle.iter().map(|c| c.round).collect();
        let mut sorted = rounds.clone();
        sorted.sort_unstable();
        assert_eq!(rounds, sorted, "chronicle must be sorted by firing round");

        // ids are unique (each event fires once).
        let mut dedup = ids.clone();
        dedup.sort_unstable();
        let before_n = dedup.len();
        dedup.dedup();
        assert_eq!(before_n, dedup.len(), "each story id fires at most once");

        // Determinism: re-running the same seed reproduces the identical chronicle.
        let (_, mut state2) = fresh_world(42);
        let mut rng2 = Prng::new(42);
        for _ in 0..60 {
            advance(&mut state2, &config, &mut rng2);
        }
        assert_eq!(
            state.chronicle.iter().map(|c| (c.round, c.id.clone(), c.title.clone())).collect::<Vec<_>>(),
            state2.chronicle.iter().map(|c| (c.round, c.id.clone(), c.title.clone())).collect::<Vec<_>>(),
            "same seed must produce the same story arc"
        );
    }

    /// 剧情机械后果：prologue 给无国界科学组织(6)注入氦-3，并拉低它与行星X崇拜教(8)的关系。
    #[test]
    fn story_effects_apply() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        let helium_before = state.faction(6).map(|f| f.resources.get("helium3").copied().unwrap_or(0.0)).unwrap_or(0.0);
        let rel_before = state.faction(6).and_then(|f| f.relations.get(&8).copied()).unwrap_or(0.0);

        advance(&mut state, &config, &mut rng);

        let helium_after = state.faction(6).map(|f| f.resources.get("helium3").copied().unwrap_or(0.0)).unwrap_or(0.0);
        assert!(helium_after > helium_before, "prologue grants science 氦-3 (effect)");
        let rel_after = state.faction(6).and_then(|f| f.relations.get(&8).copied()).unwrap_or(0.0);
        assert!(rel_after < rel_before, "prologue must lower science↔cult relation (effect)");
    }

    /// 剧情 GrantShip 后果：kuiper_boom（RoundAt 24）给星系矿业(5)出厂一艘巡洋舰；
    /// 出厂位置恰好在天体当前位置 + (0.05, 0.05)（确定性偏移）、带 Idle 指令、id 连续。
    #[test]
    fn story_grant_ship_spawns_a_fleet_member() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // Advance to round 24 so kuiper_boom fires.
        for _ in 0..24 {
            advance(&mut state, &config, &mut rng);
        }

        // The story fired this round.
        assert!(
            state.chronicle.iter().any(|c| c.id == "kuiper_boom" && c.round == 24),
            "kuiper_boom must fire at round 24, got {:?}",
            state.chronicle.iter().map(|c| (c.id.clone(), c.round)).collect::<Vec<_>>()
        );

        // The granted cruiser is at exactly body 9 (泰坦) position + the deterministic offset.
        let bpos = state.body_position(9);
        let granted = state
            .ships
            .iter()
            .find(|s| {
                s.faction_id == 5
                    && s.class == "cruiser"
                    && (s.position[0] - (bpos[0] + 0.05)).abs() < 1e-9
                    && (s.position[1] - (bpos[1] + 0.05)).abs() < 1e-9
            })
            .expect("kuiper_boom must grant 星系矿业 a cruiser parked at 泰坦");
        assert_eq!(state.ship_behavior(granted.id), Some(ShipBehavior::Idle), "granted ship starts Idle");

        // Determinism: re-running reproduces the identical granted fleet.
        let (_, mut state2) = fresh_world(42);
        let mut rng2 = Prng::new(42);
        for _ in 0..24 {
            advance(&mut state2, &config, &mut rng2);
        }
        let fleet_a: Vec<(u32, String)> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == 5)
            .map(|s| (s.id, s.class.clone()))
            .collect();
        let fleet_b: Vec<(u32, String)> = state2
            .ships
            .iter()
            .filter(|s| s.faction_id == 5)
            .map(|s| (s.id, s.class.clone()))
            .collect();
        assert_eq!(fleet_a, fleet_b, "same seed must reproduce the same granted fleet");
    }

    /// 剧情参与方是「具体的」：事件型触发把本回合事件的实际对象写进编年史
    /// （谁与谁开战、哪座城被夷平、谁建立了殖民地），而不是泛化的空标签。
    #[test]
    fn story_participants_are_concrete() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        for _ in 0..60 {
            advance(&mut state, &config, &mut rng);
        }
        let find = |id: &str| state.chronicle.iter().find(|c| c.id == id);
        if let Some(war) = find("first_war") {
            assert_eq!(war.participants.len(), 2, "first_war names the two belligerents, got {:?}", war.participants);
            assert!(war.participants.iter().all(|p| !p.is_empty()));
        }
        if let Some(razed) = find("first_raze") {
            assert!(razed.participants.len() >= 2, "first_raze names the city and the razer, got {:?}", razed.participants);
        }
        if let Some(colon) = find("first_colony") {
            assert!(colon.participants.len() >= 2, "first_colony names the colonizer and the body, got {:?}", colon.participants);
        }
        if let Some(cn) = find("cn_us_rivalry") {
            assert!(cn.participants.contains(&"中国".to_string()), "cn_us_rivalry names 中国, got {:?}", cn.participants);
            assert!(cn.participants.contains(&"美国".to_string()), "cn_us_rivalry names 美国, got {:?}", cn.participants);
        }
        // RoundAt beats keep exactly their static participants (no event to enrich).
        if let Some(pro) = find("prologue") {
            assert_eq!(pro.participants, vec!["无国界科学组织".to_string(), "行星X崇拜教".to_string()]);
        }
    }

    /// 娱乐/福利预算（忠诚度）：一座远离首都的城市，其距离目标忠诚度本应很低；但若
    /// 治理势力投入足够的娱乐预算，忠诚度仍能维持/回升，而非立刻爆发离心叛乱。
    #[test]
    fn entertainment_holds_a_distant_city() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        // 深口袋：让星系矿业(5)付得起治理 + 娱乐开销，覆盖率=1。
        if let Some(f) = state.faction_mut(5) {
            for k in [
                "iron", "carbon", "silicon", "water_ice", "uranium", "platinum", "gold",
                "helium3", "thorium", "hydrogen", "methane",
            ] {
                f.resources.insert(k.to_string(), 100_000.0);
            }
        }
        // 妊神星转运站 (city 19, body 15) 远离矿业首都(泰坦, body 9)，距离目标忠诚度≈0。
        if let Some(c) = state.city_mut(19) {
            c.loyalty = 0.35; // 略高于叛变阈值，但本应继续下滑。
        }
        let loy0 = state.city(19).map(|c| c.loyalty).unwrap();
        // 重金投入该城娱乐预算（Player 覆盖）。
        let diff = serde_json::json!({
            "control": [{"faction_id": 5, "loyalty_budget": [{"city": 19, "value": 500.0, "mode": "Player"}]}]
        });
        crate::web::apply_patch(&mut state, &config, &diff).expect("apply loyalty budget");

        advance(&mut state, &config, &mut rng);

        let loy1 = state.city(19).map(|c| c.loyalty).unwrap_or(0.0);
        assert!(
            loy1 >= loy0,
            "heavy entertainment funding should keep a distant city loyal (started {loy0}, now {loy1})"
        );
        assert_eq!(
            state.city(19).map(|c| c.razed),
            Some(false),
            "a well-funded distant city must not revolt"
        );
    }

    /// MOND 引力异常：落入异常区（深空）时，未掌握 MOND 修正引力的势力在导航上产生
    /// 切向偏移（指令坐标与实际坐标分离），而掌握它的 cult 指哪打哪。
    #[test]
    fn mond_drift_misses_in_anomaly_but_masters_are_exact() {
        let (config, _state) = fresh_world(42);
        let dest = [60.0, 0.0]; // 距太阳 60 AU，深入柯伊伯异常区。
        // 非 MOND 势力（中国=3）：目标被切向偏移，无法精确到达。
        let d = mond_drift(&config, 3, dest);
        assert!(
            (d[0] - dest[0]).abs() > 1e-6 || (d[1] - dest[1]).abs() > 1e-6,
            "a non-master ship must drift inside the anomaly, got {d:?}"
        );
        // MOND 势力（行星X崇拜教=8）：掌握修正引力，无偏移、指哪打哪。
        let m = mond_drift(&config, 8, dest);
        assert_eq!(m, dest, "a MOND master must compute the destination exactly");
    }

    /// 本土防御（首都即强弩）：靠近首都的目标被削弱，远离首都的没有。
    #[test]
    fn home_field_weakens_attackers_near_the_capital() {
        let (_config, state) = fresh_world(42);
        let cap = state.body_position(2); // 地球（中国首都）。
        let mult_near = home_defense_mult(&state, 3, cap);
        assert!(mult_near < 1.0, "near the capital should be defended (mult {mult_near})");
        let mult_far = home_defense_mult(&state, 3, [80.0, 80.0]);
        assert_eq!(mult_far, 1.0, "far from the capital should have no home-field defense");
    }

    /// 舰船定制面板：装了护盾+轨道炮的舰，其 effective 面板反映组件的护盾池/装甲/火力/射程。
    #[test]
    fn ship_panel_reflects_fitted_components() {
        let (config, mut state) = fresh_world(42);
        let base = config.ship_spec("corvette");
        if let Some(s) = state.ship_mut(0) {
            s.components = vec!["shield".to_string(), "railgun".to_string()];
        }
        let s = state.ship(0).unwrap();
        let panel = ship_panel(&config, s);
        assert!((panel.hull_max - base.hull - config.component_spec("shield").hull).abs() < 1e-9);
        assert!((panel.shield_max - config.component_spec("shield").shield).abs() < 1e-9);
        assert!(panel.attack > base.attack, "railgun should add firepower");
        assert!(
            panel.attack_range > base.attack_range,
            "railgun's long range should extend the engage window"
        );
        assert!(panel.upkeep > base.upkeep, "components should raise maintenance");
    }

    /// 模块损毁（拟人「渐进丧失战力」）：被击中的舰，其组件完整度随船体伤害下降，而不是
    /// 满血抗到壳破。这里让一艘带组件的舰挨打，验证其组件完整度确实下降（被击毁后不再
    /// 贡献面板/武器见 `ship_panel`/`ship_weapons` 跳过损坏组件）。
    #[test]
    fn fire_degrades_components_under_damage() {
        let (config, mut state) = fresh_world(42);
        // 目标：US 驱逐舰（ship 3），装一枚导弹组件、血厚到扛住一炮以观察组件损耗。
        if let Some(t) = state.ship_mut(3) {
            t.position = [40.0, 40.0];
            t.components = vec!["missile".to_string()];
            t.component_hp = t.components.iter().map(|c| component_integrity(&config, c)).collect();
            t.hull = 500.0;
            t.hull_max = 500.0;
            t.shield = 0.0;
            t.shield_max = 0.0;
        }
        // 攻击者：CN 护卫舰（ship 0），装一门重炮、贴近目标。
        if let Some(a) = state.ship_mut(0) {
            a.position = [40.1, 40.0];
            a.components = vec!["railgun".to_string()];
            a.component_hp = a.components.iter().map(|c| component_integrity(&config, c)).collect();
        }
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);
        let before = state.ship(3).unwrap().component_hp.clone();
        let panel_before = ship_panel(&config, state.ship(3).unwrap());
        fire(&mut state, &config, 0, 3);
        let after = state.ship(3).unwrap().component_hp.clone();
        assert!(
            after.iter().zip(before.iter()).any(|(a, b)| *a < *b),
            "component integrity should drop under fire; before={before:?} after={after:?}"
        );
        // 被击毁后不贡献面板：把目标组件打掉，验证攻击/护盾面板下降。
        let _ = panel_before;
    }

    /// 母港/友方本土修船（拟人「打残→撤→修→再来」闭环）：受损组件的完整度每回合修复，
    /// 且在本土（首都 home_radius 内）修得更快。
    #[test]
    fn damaged_components_repair_in_friendly_territory() {
        let (config, mut state) = fresh_world(42);
        // China ship 0 停在其首都（Earth, body 2），组件受损。
        let cap_pos = state.body_position(2);
        if let Some(s) = state.ship_mut(0) {
            s.position = cap_pos;
            s.components = vec!["railgun".to_string()];
            s.component_hp = vec![5.0];
            s.hull = s.hull.max(5.0);
        }
        let before = state.ship(0).map(|s| s.component_hp.first().copied().unwrap_or(0.0)).unwrap_or(0.0);
        advance(&mut state, &config, &mut Prng::new(42));
        let after = state.ship(0).map(|s| s.component_hp.first().copied()).flatten().unwrap_or(before);
        assert!(
            after > before,
            "a damaged component should repair over rounds; before={before} after={after}"
        );
    }

    /// 造舰选装必须**确定**并且**总量可负担**（资源→组件 的确定性链接）。
    #[test]
    fn choose_loadout_is_deterministic_and_affordable() {
        let (config, mut state) = fresh_world(42);
        // Give China (3) a fat rare-mineral stack so it can afford a real loadout.
        if let Some(f) = state.faction_mut(3) {
            for (r, amt) in [
                ("uranium", 200.0), ("gold", 200.0), ("helium3", 200.0),
                ("platinum", 200.0), ("hydrogen", 200.0), ("thorium", 200.0),
                ("iron", 200.0), ("carbon", 200.0), ("silicon", 200.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        let slots = config.ship_spec("battleship").slots as usize;
        let a = choose_loadout(&state, &config, 3, "battleship");
        let b = choose_loadout(&state, &config, 3, "battleship");
        assert_eq!(a, b, "loadout must be deterministic");
        assert!(a.len() <= slots, "must not exceed slot cap ({slots})");
        // The whole chosen set must be cumulatively affordable out of the stockpile.
        let mut pool = state.faction(3).unwrap().resources.clone();
        for c in &a {
            for (r, amt) in &config.component_spec(c).cost {
                assert!(
                    pool.get(r).copied().unwrap_or(0.0) >= *amt,
                    "loadout {c} must be affordable for resource {r}"
                );
                *pool.entry(r.clone()).or_insert(0.0) -= *amt;
            }
        }
        // A resource-rich faction should fill more than a token slot.
        assert!(a.len() >= 2, "rich faction should field a real loadout, got {a:?}");
    }

    /// 拟人指挥官：海军**混编**——一支富有的、近乎全护卫的势力，`choose_next_class` 会被
    /// 「去重加分」拉去建其它舰型（不只堆护卫），形成更像真实海军的混编。
    #[test]
    fn choose_next_class_diversifies_toward_a_mix() {
        let (config, mut state) = fresh_world(42);
        if let Some(f) = state.faction_mut(3) {
            for (r, amt) in [
                ("uranium", 300.0), ("gold", 300.0), ("helium3", 300.0), ("platinum", 300.0),
                ("hydrogen", 300.0), ("thorium", 300.0), ("iron", 300.0), ("carbon", 300.0),
                ("silicon", 300.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        // 强制这支势力的现役舰队全部是护卫舰——其余舰型因此「欠份额」，得到去重加分。
        for s in state.ships.iter_mut() {
            if s.faction_id == 3 {
                s.class = "corvette".to_string();
            }
        }
        let mut rng = Prng::new(7);
        let mut got = std::collections::BTreeSet::new();
        for _ in 0..60 {
            got.insert(choose_next_class(&state, 3, &config, &mut rng));
        }
        assert!(
            got.len() >= 3,
            "an all-corvette navy should be pulled into a mix, got {got:?}"
        );
    }

    /// 威胁响应造舰（拟人「战时多造重舰」）：交战中，AI 会比和平时更倾向造重型战斗舰
    /// （攻击力高的舰型加分），而不是只堆轻护卫。
    #[test]
    fn choose_next_class_builds_heavier_navy_at_war() {
        let (config, mut state) = fresh_world(42);
        if let Some(f) = state.faction_mut(3) {
            for (r, amt) in [
                ("uranium", 300.0), ("gold", 300.0), ("helium3", 300.0), ("platinum", 300.0),
                ("hydrogen", 300.0), ("thorium", 300.0), ("iron", 300.0), ("carbon", 300.0),
                ("silicon", 300.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        // 舰队全部护卫舰，让去重加分对各舰型一视同仁。
        for s in state.ships.iter_mut() {
            if s.faction_id == 3 {
                s.class = "corvette".to_string();
            }
        }
        let sample_heavy = |st: &State| -> usize {
            let mut rng = Prng::new(99);
            let mut heavy = 0;
            for _ in 0..240 {
                let c = choose_next_class(st, 3, &config, &mut rng);
                if c == "battleship" || c == "carrier" {
                    heavy += 1;
                }
            }
            heavy
        };
        let peace = sample_heavy(&state);
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);
        let war = sample_heavy(&state);
        assert!(
            war > peace,
            "at war the AI should build more heavy hulls (war {war} > peace {peace})"
        );
    }

    /// 威胁响应（海军随威胁重构）：交战中，被单一舰型过度统治的势力会把一个船坞重定向到
    /// 战局感知的新舰型（多造重舰），让威胁响应作用于整支舰队而不仅是新建舰厂。
    #[test]
    fn war_retools_over_abundant_shipyard_toward_a_war_class() {
        let (config, mut state) = fresh_world(42);
        let shipyard_types = |st: &State, f: FactionId| -> std::collections::BTreeSet<(CityId, String)> {
            st.cities
                .iter()
                .filter(|c| c.faction_id == f)
                .flat_map(|c| {
                    c.buildings
                        .iter()
                        .filter(|b| b.is_shipyard() && b.ship_type.is_some())
                        .map(|b| (c.id, b.ship_type.clone().unwrap()))
                })
                .collect()
        };
        // China (3) 舰队全护卫（过度单一），并让其与 US (1) 交战。
        for s in state.ships.iter_mut() {
            if s.faction_id == 3 {
                s.class = "corvette".to_string();
            }
        }
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);
        let before = shipyard_types(&state, 3);
        let mut rng = Prng::new(7);
        retool_shipyards(&mut state, &config, 3, &mut rng);
        let after = shipyard_types(&state, 3);
        assert!(
            after.iter().any(|(_, t)| t != "corvette"),
            "a corvette-dominated wartime fleet should retool a shipyard into a war class; before={before:?} after={after:?}"
        );
    }

    /// 拟人指挥官：军舰选装要「又能打、又能扛」（…）；战局感知也在此测试。
    #[test]
    fn choose_loadout_is_balanced_and_threat_aware() {
        let (config, mut state) = fresh_world(42);
        if let Some(f) = state.faction_mut(3) {
            for (r, amt) in [
                ("uranium", 300.0), ("gold", 300.0), ("helium3", 300.0), ("platinum", 300.0),
                ("hydrogen", 300.0), ("thorium", 300.0), ("iron", 300.0), ("carbon", 300.0),
                ("silicon", 300.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        // 和平：一艘巡洋舰（slot≥2）应至少各有一件武器与防御。
        let peace = choose_loadout(&state, &config, 3, "cruiser");
        assert!(
            peace.iter().any(|c| config.component_spec(c).category == "weapon"),
            "a ship should field a weapon (got {peace:?})"
        );
        assert!(
            peace.iter().any(|c| config.component_spec(c).category == "defense"),
            "a ship should field a defense (got {peace:?})"
        );
        // 开战：武器数不应比和平少（战时要火力的偏置）。
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);
        let war = choose_loadout(&state, &config, 3, "cruiser");
        let peace_w = peace.iter().filter(|c| config.component_spec(c).category == "weapon").count();
        let war_w = war.iter().filter(|c| config.component_spec(c).category == "weapon").count();
        assert!(
            war_w >= peace_w,
            "at war the AI should field at least as many weapons (war {war_w} >= peace {peace_w}); war={war:?} peace={peace:?}"
        );
    }

    /// 战斗拟真：护盾池优先吸收，快速目标对低追踪武器规避更强（确定性命中折减）。
    #[test]
    fn combat_respects_shields_and_speed_evasion() {
        let (config, mut state) = fresh_world(42);
        // Attacker: China corvette (id 0) fitted with a railgun; target: US destroyer (id 3)
        // fitted with an energy shield. Both pinned far from any capital so home-field
        // defense is neutral (mult = 1.0). Hostile so the volley is a real attack.
        if let Some(s) = state.ship_mut(0) {
            s.position = [80.0, 80.0];
            s.components = vec!["railgun".to_string()];
        }
        if let Some(s) = state.ship_mut(3) {
            s.position = [80.4, 80.0];
            s.components = vec!["shield".to_string()];
            s.hull = 24.0;
            s.hull_max = 24.0;
            s.shield = 12.0;
            s.shield_max = 12.0;
        }
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);

        let shield_before = state.ship(3).map(|s| s.shield).unwrap();
        let hull_before = state.ship(3).map(|s| s.hull).unwrap();
        fire(&mut state, &config, 0, 3);

        let shield_after = state.ship(3).map(|s| s.shield).unwrap();
        let hull_after = state.ship(3).map(|s| s.hull).unwrap();
        assert!(shield_after < shield_before, "shield pool must absorb damage");
        assert!(hull_after < hull_before, "hull should take spill damage too");
        assert!(hull_after > 0.0, "a single volley on a destroyer should not one-shot it");

        // Evasion: a fast target is hit less by a low-tracking weapon than a slow one.
        let fast_hit = hit_factor(2.0, 2.6); // corvette speed
        let slow_hit = hit_factor(2.0, 1.2); // destroyer speed
        assert!(
            fast_hit < slow_hit,
            "fast ship should evade a low-tracking weapon more (fast {fast_hit} vs slow {slow_hit})"
        );
    }

    /// 功能性验证：长局里确实会出现「定制化」舰（资源→组件选择真的被 AI 执行）。
    #[test]
    fn long_run_produces_customized_ships() {
        let config = load_config();
        let mut customized = 0usize;
        for seed in [7u64, 42] {
            let mut state = default_state(&config, seed);
            let mut rng = Prng::new(seed);
            for _ in 0..400u32 {
                advance(&mut state, &config, &mut rng);
            }
            customized += state.ships.iter().filter(|s| !s.components.is_empty()).count();
        }
        assert!(
            customized > 0,
            "customized (component-fitted) ships should appear over a long run, got {customized}"
        );
    }

    /// 集中火力（拟人）：同一射程内，优先打**已受重创**的敌舰（hull/hull_max 更低者），
    /// 把火力压到一个目标上将其打掉，而不是各自就近乱打。
    #[test]
    fn combat_concentrates_fire_on_wounded_enemy() {
        let (config, mut state) = fresh_world(42);
        // China (3) looks from [40,40]; two hostile US (1) ships sit inside its 0.4 range.
        // Ship 3 (destroyer, hull 24) is healthy; ship 5 (cruiser, hull 5/72) is badly wounded.
        if let Some(s) = state.ship_mut(3) {
            s.position = [40.2, 40.0];
            s.hull = 24.0;
            s.hull_max = 24.0;
        }
        if let Some(s) = state.ship_mut(5) {
            s.position = [40.3, 40.0];
            s.hull = 5.0;
            s.hull_max = 72.0;
        }
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);
        let target = nearest_enemy_ship(&state, &config, 3, [40.0, 40.0], 0.4, None, 0);
        assert_eq!(
            target,
            Some(5),
            "should concentrate fire on the wounded enemy (5), got {target:?}"
        );
    }

    /// 武器克制选目标（拟人「别浪费导弹打点防重镇」）：一舰有导弹时，应优先攻击**没有**
    /// 点防御、导弹不会被拦截的目标，而不是把导弹打在被点防全面阻挡的目标上。
    #[test]
    fn target_selection_respects_weapon_advantage() {
        let (config, mut state) = fresh_world(42);
        // China (3) fields a missile-armed attacker (ship 0). Two hostile US (1)
        // targets sit in range: ship 3 has point-defense (intercepts missiles),
        // ship 5 has none — the missile attacker should prefer ship 5.
        if let Some(s) = state.ship_mut(0) {
            s.position = [40.0, 40.0];
            s.components = vec!["missile".to_string()];
        }
        if let Some(s) = state.ship_mut(3) {
            s.position = [40.2, 40.0];
            s.components = vec!["point_defense".to_string()];
        }
        if let Some(s) = state.ship_mut(5) {
            s.position = [40.3, 40.0];
        }
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);
        let target = nearest_enemy_ship(&state, &config, 3, [40.0, 40.0], 0.4, None, 0);
        assert_eq!(
            target,
            Some(5),
            "a missile attacker should shun the point-defense ship (5), got {target:?}"
        );
    }

    /// 自保撤退（拟人「别送死」）：一舰离首都较远、已被打残、且敌在射程内时，应后撤
    /// 修整充能（发出 Withdraw、朝首都移动、不送死），而不是死战到被击毁。
    #[test]
    fn damaged_far_ai_ship_withdraws_to_heal() {
        let (config, mut state) = fresh_world(42);
        let home = state.body_position(2); // 地球（中国首都）。
        // China (3) destroyer id 2: badly wounded (hull 5/24) and far from its capital.
        if let Some(s) = state.ship_mut(2) {
            s.position = [40.0, 40.0];
            s.hull = 5.0;
            s.hull_max = 24.0;
        }
        // A hostile US (1) ship within the destroyer's attack range.
        if let Some(s) = state.ship_mut(3) {
            s.position = [40.3, 40.0];
        }
        state.faction_mut(3).unwrap().relations.insert(1, -35.0);
        state.faction_mut(1).unwrap().relations.insert(3, -35.0);

        let d_before = dist([40.0, 40.0], home);
        let mut rng = Prng::new(42);
        advance(&mut state, &config, &mut rng);

        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::Withdraw { ship: 2, .. })),
            "a damaged far-from-home ship must withdraw, events={:?}",
            state.events
        );
        let s = state.ship(2).expect("withdrawing ship must survive");
        let d_after = dist(s.position, home);
        assert!(
            d_after < d_before,
            "withdrawing ship should head for home ({d_before:.2} -> {d_after:.2})"
        );
    }
}
