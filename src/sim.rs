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
use std::collections::BTreeMap;

/// Advance the world by one round, writing the new controllable state into
/// [`State::control`].
pub fn advance(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    state.round += 1;
    state.time_month += 1.0;
    // 本回合事件日志从空开始，回合演化中追加。
    state.events.clear();

    // Update each body's current position (当前位置) from its orbit.
    for b in &mut state.bodies {
        b.position = b.orbit.position(state.time_month as f32);
    }

    step_production(state, config);
    step_construction(state, config, rng);
    step_military(state, config, rng);
    step_diplomacy(state, config);
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
        let (body_id, faction_id, population, razed) = {
            let c = state.city(cid).expect("city disappeared");
            (c.body_id, c.faction_id, c.population, c.razed)
        };
        if razed {
            continue;
        }
        let (ecocap, deposits) = {
            let s = state.body(body_id).and_then(|b| b.settlement.as_ref());
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

// --- construction (dual budgets) ---------------------------------------------

#[derive(Clone, Copy)]
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
    let mut budget: ResourceMap = ResourceMap::new();
    let mut modes = Vec::new();
    for (rt, v) in &stockpile {
        let ai_value = *v * config.economy.invest_fraction;
        let mode = match kind {
            BudgetKind::Investment => state.investment_budget_control(fid, rt),
            BudgetKind::Construction => state.construction_budget_control(fid, rt),
        };
        let value = match mode {
            ControlMode::Ai => ai_value,
            ControlMode::Player => state
                .control(fid)
                .and_then(|c| match kind {
                    BudgetKind::Investment => c.investment_budget.get(rt),
                    BudgetKind::Construction => c.construction_budget.get(rt),
                })
                .map(|c| c.value)
                .unwrap_or(ai_value),
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
                    slot.insert(rt.clone(), Control::ai(*value));
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

fn can_pay(state: &State, fid: FactionId, cost: &ResourceMap) -> bool {
    cost.iter().all(|(rt, c)| {
        state.faction(fid).map_or(0.0, |f| f.resources.get(rt).copied().unwrap_or(0.0)) >= *c
    })
}

/// Pick a ship class a faction can currently afford (for new colonies / new
/// shipyards with no class yet).
fn choose_next_class(state: &State, fid: FactionId, config: &GameConfig, rng: &mut Prng) -> String {
    let affordable: Vec<String> = config
        .ships
        .keys()
        .filter(|c| can_pay(state, fid, &config.ship_spec(c).build_cost))
        .cloned()
        .collect();
    if affordable.is_empty() {
        "corvette".to_string()
    } else {
        rng.pick(&affordable).clone()
    }
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
    let Some(body_id) = state.city(cid).map(|c| c.body_id) else { return };
    if state.city(cid).map(|c| c.razed).unwrap_or(true) {
        return;
    }
    let (ecocap, total_area, speed_mod, res_mod, deposits) = {
        let s = state.body(body_id).and_then(|b| b.settlement.as_ref());
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
        while to_write_progress.get(cls).copied().unwrap_or(0.0) >= bp - 1e-9 {
            state.ships.push(Ship {
                id: *next_ship_id,
                name: format!("{}-{}", spec.label, fid),
                class: cls.clone(),
                faction_id: fid,
                position: [body_pos[0] + 0.05, body_pos[1] + 0.05],
                hull: spec.hull,
            });
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

    for ship_id in order {
        let Some(ship) = state.ship(ship_id) else { continue };
        if ship.hull <= 0.0 {
            continue;
        }
        let owner = ship.faction_id;
        let class = ship.class.clone();
        let pos = ship.position;
        let range = config.ship_spec(&class).attack_range;

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
                    ShipBehavior::TargetShip { ship, .. } => format!("target ship {ship} gone"),
                    ShipBehavior::TargetSettlement { city, .. } => format!("target city {city} razed or not hostile"),
                    ShipBehavior::Colonize { body } => format!("body {body} has no settlement"),
                    _ => "invalid".to_string(),
                };
                if let Some(c) = state.control_mut(owner) {
                    c.ship_orders.insert(ship_id, Control::player(ShipBehavior::Idle));
                }
                ev(state, GameEvent::StaleOrder { ship: ship_id, reason });
                behavior = ShipBehavior::Idle;
            }
            match behavior {
                ShipBehavior::Idle => continue,
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
                        if let Some(t) = state.ship(ship) {
                            if t.hull > 0.0 && dist(pos, t.position) <= range {
                                fire(state, config, ship_id, ship);
                                continue;
                            }
                        }
                    }
                    move_toward(state, config, ship_id, &class, behavior_dest(state, behavior));
                    if attack {
                        if let Some(t) = state.ship(ship) {
                            if t.hull > 0.0 {
                                let np = state.ship(ship_id).map(|s| s.position).unwrap_or(pos);
                                if dist(np, t.position) <= range {
                                    fire(state, config, ship_id, ship);
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
        if let Some(target) = nearest_enemy_ship(state, config, owner, pos, range) {
            if let Some(c) = state.control_mut(owner) {
                c.ship_orders.insert(ship_id, Control::ai(ShipBehavior::TargetShip { ship: target, attack: true }));
            }
            fire(state, config, ship_id, target);
            continue;
        }

        let Some(behavior) = resolve_target(state, config, ship_id, owner, pos, rng) else {
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
            if let Some(target) = nearest_enemy_ship(state, config, owner, np, range) {
                if let Some(c) = state.control_mut(owner) {
                    c.ship_orders.insert(ship_id, Control::ai(ShipBehavior::TargetShip { ship: target, attack: true }));
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

    // Drop destroyed ships and prune their behaviors from the controllable state.
    state.ships.retain(|s| s.hull > 0.0);
    let alive: std::collections::BTreeSet<ShipId> = state.ships.iter().map(|s| s.id).collect();
    for c in state.control.values_mut() {
        c.ship_orders.retain(|sid, _| alive.contains(sid));
    }
}

/// Move a ship one round's step toward `dest`, capped by its class speed.
fn move_toward(state: &mut State, config: &GameConfig, ship_id: ShipId, class: &str, dest: [f64; 2]) {
    let Some(pos) = state.ship(ship_id).map(|s| s.position) else { return };
    let distance = dist(pos, dest);
    if distance <= 1e-9 {
        return;
    }
    let speed = config.ship_spec(class).speed;
    let step = if distance <= config.combat.arrival_eps { 0.0 } else { speed.min(distance) };
    if step > 0.0 {
        let nx = (dest[0] - pos[0]) / distance;
        let ny = (dest[1] - pos[1]) / distance;
        if let Some(s) = state.ship_mut(ship_id) {
            s.position = [s.position[0] + nx * step, s.position[1] + ny * step];
        }
    }
}

fn nearest_enemy_ship(state: &State, config: &GameConfig, owner: FactionId, pos: [f64; 2], range: f64) -> Option<ShipId> {
    let mut best: Option<(f64, ShipId)> = None;
    for s in &state.ships {
        if s.hull > 0.0 && hostile(state, config, owner, s.faction_id) {
            let d = dist(pos, s.position);
            if d <= range && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, s.id));
            }
        }
    }
    best.map(|(_, id)| id)
}

fn fire(state: &mut State, config: &GameConfig, attacker_id: ShipId, target_id: ShipId) {
    let (dmg, afac) = {
        let a = state.ship(attacker_id).expect("attacker gone");
        (config.ship_spec(&a.class).attack, a.faction_id)
    };
    let (tfac, hull) = {
        let t = state.ship(target_id).expect("target gone");
        (t.faction_id, t.hull)
    };
    let new_hull = hull - dmg;
    let destroyed = new_hull <= 0.0;
    if let Some(t) = state.ship_mut(target_id) {
        t.hull = if destroyed { 0.0 } else { new_hull };
    }
    ev(state, GameEvent::Attack { attacker: attacker_id, target: target_id, damage: dmg });
    if destroyed {
        ev(state, GameEvent::ShipDestroyed { ship: target_id, owner: tfac, class: state.ship(target_id).map(|s| s.class.clone()).unwrap_or_default() });
    }
    adjust_relation(state, afac, tfac, config.diplomacy.attack_delta);
}

fn behavior_is_valid(state: &State, config: &GameConfig, behavior: ShipBehavior, owner: FactionId) -> bool {
    match behavior {
        ShipBehavior::Move { .. } | ShipBehavior::Idle => true,
        ShipBehavior::Colonize { body } => state.body(body).map(|b| b.settlement.is_some()).unwrap_or(false),
        ShipBehavior::TargetShip { ship, .. } => state
            .ship(ship)
            .map(|s| s.hull > 0.0 && hostile(state, config, owner, s.faction_id))
            .unwrap_or(false),
        ShipBehavior::TargetSettlement { city, .. } => state
            .city(city)
            .map(|c| !c.razed && hostile(state, config, owner, c.faction_id))
            .unwrap_or(false),
    }
}

fn resolve_target(state: &mut State, config: &GameConfig, ship_id: ShipId, owner: FactionId, pos: [f64; 2], rng: &mut Prng) -> Option<ShipBehavior> {
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
    let picked = pick_target(state, config, owner, pos, rng);
    let behavior = picked.unwrap_or(ShipBehavior::Idle);
    if let Some(c) = state.control_mut(owner) {
        c.ship_orders.insert(ship_id, Control::ai(behavior));
    }
    picked
}

fn pick_target(state: &State, config: &GameConfig, owner: FactionId, pos: [f64; 2], rng: &mut Prng) -> Option<ShipBehavior> {
    let mut best: Option<(f64, ShipBehavior)> = None;
    let mut consider = |d: f64, b: ShipBehavior, best: &mut Option<(f64, ShipBehavior)>| {
        let replace = match *best {
            None => true,
            Some((bd, _)) => {
                if d < bd - 1e-9 {
                    true
                } else if (d - bd).abs() <= 1e-9 {
                    rng.range(2) == 0
                } else {
                    false
                }
            }
        };
        if replace {
            *best = Some((d, b));
        }
    };

    // Combat first: prefer the nearest hostile ship / city.
    for s in &state.ships {
        if s.hull > 0.0 && hostile(state, config, owner, s.faction_id) {
            consider(dist(pos, s.position), ShipBehavior::TargetShip { ship: s.id, attack: true }, &mut best);
        }
    }
    for c in &state.cities {
        if !c.razed && hostile(state, config, owner, c.faction_id) {
            let p = city_position(state, c.id);
            consider(dist(pos, p), ShipBehavior::TargetSettlement { city: c.id, bombard: true }, &mut best);
        }
    }
    // If there is nothing to fight, re-colonize a nearby razed (blank) settlement.
    if best.is_none() {
        for c in &state.cities {
            if c.razed {
                let p = city_position(state, c.id);
                consider(dist(pos, p), ShipBehavior::Colonize { body: c.body_id }, &mut best);
            }
        }
    }
    best.map(|(_, b)| b)
}

/// Bombard a city: damage is spread across its buildings by area share. When all
/// buildings are destroyed the city is razed to a blank (colonizable) settlement
/// — it is never captured.
fn bombard_city(state: &mut State, config: &GameConfig, ship_id: ShipId, cid: CityId) {
    let (dmg, attacker) = {
        let s = state.ship(ship_id).expect("ship gone");
        (config.ship_spec(&s.class).attack, s.faction_id)
    };
    let old_owner = {
        let c = state.city(cid).expect("city gone");
        c.faction_id
    };
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

/// Colonize a settlement. If the body hosts a razed city, re-seed it (re-colonize);
/// otherwise found a new city if the settlement has room. The colony ship is
/// spent (order reset to idle).
fn colonize(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    ship_id: ShipId,
    body: BodyId,
    next_building_id: &mut BuildingId,
    next_city_id: &mut CityId,
) {
    let (faction, body_name) = {
        let s = state.ship(ship_id).expect("ship gone");
        (s.faction_id, state.body(body).map(|b| b.name.clone()).unwrap_or_else(|| format!("#{body}")))
    };
    let settlement = state.body(body).and_then(|b| b.settlement.as_ref());
    let Some(settlement) = settlement else {
        if let Some(c) = state.control_mut(faction) {
            c.ship_orders.insert(ship_id, Control::ai(ShipBehavior::Idle));
        }
        return;
    };

    let seeded_ship_class = choose_next_class(state, faction, config, rng);
    let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;

    // Re-colonize an existing razed city on this body.
    let razed_cid = state.cities.iter().find(|c| c.body_id == body && c.razed).map(|c| c.id);
    if let Some(cid) = razed_cid {
        let buildings = seed_colony_buildings(settlement, pop, &seeded_ship_class, config, next_building_id);
        if let Some(c) = state.city_mut(cid) {
            c.razed = false;
            c.faction_id = faction;
            c.population = pop;
            c.buildings = buildings;
            c.ship_progress.clear();
            c.ship_progress.insert(seeded_ship_class.clone(), 0.0);
        }
        ev(state, GameEvent::ColonyFounded { city: cid, owner: faction, body, seeded_ship_class: seeded_ship_class.clone() });
    } else {
        // Find a new city if the settlement has room.
        let city_count = state.cities.iter().filter(|c| c.body_id == body).count();
        if (city_count as f64 + 1.0) * pop as f64 <= settlement.total_area * config.economy.housing_buffer {
            let buildings = seed_colony_buildings(settlement, pop, &seeded_ship_class, config, next_building_id);
            let cid = *next_city_id;
            let city = City {
                id: cid,
                name: format!("{}-殖民地", body_name),
                body_id: body,
                faction_id: faction,
                population: pop,
                buildings,
                ship_progress: {
                    let mut m = BTreeMap::new();
                    m.insert(seeded_ship_class.clone(), 0.0);
                    m
                },
                razed: false,
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
        }
    }

    if let Some(c) = state.control_mut(faction) {
        c.ship_orders.insert(ship_id, Control::ai(ShipBehavior::Idle));
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
        ShipBehavior::Colonize { body } => state.body_position(body),
        ShipBehavior::Idle => [0.0, 0.0],
    }
}

// --- diplomacy --------------------------------------------------------------

fn step_diplomacy(state: &mut State, config: &GameConfig) {
    let relax = config.diplomacy.relax_rate;
    let war_abs = config.combat.war_threshold.abs();
    for f in &mut state.factions {
        for rel in f.relations.values_mut() {
            if rel.abs() < war_abs {
                if *rel > 0.0 {
                    *rel = (*rel - relax).max(0.0);
                } else if *rel < 0.0 {
                    *rel = (*rel + relax).min(0.0);
                }
            }
        }
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

        // China (3) corvette id=0 is Player-ordered to approach US (1) corvette id=2.
        let diff = serde_json::json!({
            "control": [{
                "faction_id": 3,
                "ship_orders": [{"ship": 0, "behavior": {"TargetShip": {"ship": 2, "attack": false}}, "mode": "Player"}]
            }]
        });
        crate::web::apply_patch(&mut state, &config, &diff).expect("apply order");

        // Simulate the target being destroyed before the round advances.
        if let Some(t) = state.ship_mut(2) {
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
}
