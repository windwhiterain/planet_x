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
//! continuous area within the settlement's finite total area, paying resources
//! from a command-controlled per-round budget.

use crate::model::*;
use crate::prng::Prng;

/// Advance the world by one round and return a human-readable list of events.
pub fn advance(state: &mut State, config: &GameConfig, rng: &mut Prng) -> Vec<String> {
    state.round += 1;
    state.time_month += 1.0;

    let mut events = Vec::new();
    step_production(state, config);
    step_construction(state, config, rng, &mut events);
    step_military(state, config, rng, &mut events);
    step_diplomacy(state, config);
    events
}

// --- helpers ----------------------------------------------------------------

fn faction_name(state: &State, id: FactionId) -> String {
    state
        .faction(id)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| format!("#{}", id))
}

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
        let (body_id, faction_id, population) = {
            let c = state.city(cid).expect("city disappeared");
            (c.body_id, c.faction_id, c.population)
        };
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
            match spec.role.as_str() {
                "housing" => housing_area += b.deployed,
                "mining" => {
                    if let Some(r) = &b.resource {
                        mines.push((r.clone(), b.deployed));
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

// --- construction -----------------------------------------------------------

fn per_area_cost(spec: &BuildingSpec, res_mod: f64) -> Vec<(String, f64)> {
    spec.build_cost
        .iter()
        .map(|(rt, c)| (rt.clone(), c * res_mod))
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

fn pay(state: &mut State, fid: FactionId, cost: &ResourceMap) {
    for (rt, c) in cost {
        if let Some(f) = state.faction_mut(fid) {
            let e = f.resources.entry(rt.clone()).or_insert(0.0);
            *e = (*e - c).max(0.0);
        }
    }
}

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

fn step_construction(state: &mut State, config: &GameConfig, rng: &mut Prng, events: &mut Vec<String>) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.id).collect();
    let mut next_ship_id = state.ships.iter().map(|s| s.id).max().map_or(0, |m| m + 1);

    for fid in faction_ids {
        // Command-controlled per-round construction budget.
        let limit: ResourceMap = state
            .faction(fid)
            .unwrap()
            .resources
            .iter()
            .map(|(rt, v)| (rt.clone(), *v * config.economy.invest_fraction))
            .collect();
        let mut spent: ResourceMap = ResourceMap::new();

        let city_ids: Vec<CityId> = state.cities.iter().filter(|c| c.faction_id == fid).map(|c| c.id).collect();
        for cid in city_ids {
            next_ship_id = build_city(state, config, cid, fid, &limit, &mut spent, &mut next_ship_id, rng, events);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_city(
    state: &mut State,
    config: &GameConfig,
    cid: CityId,
    fid: FactionId,
    limit: &ResourceMap,
    spent: &mut ResourceMap,
    next_ship_id: &mut u32,
    rng: &mut Prng,
    events: &mut Vec<String>,
) -> u32 {
    let Some(body_id) = state.city(cid).map(|c| c.body_id) else { return *next_ship_id };
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
            None => return *next_ship_id,
        }
    };

    let mut buildings: Vec<Building> = state.city(cid).expect("city gone").buildings.clone();
    let population = state.city(cid).map(|c| c.population as f64).unwrap_or(0.0);
    let labor = labor_ratio(state, config, cid);

    // Helper: find the building target area for a kind (+ optional resource).
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

    fn raise_area(buildings: &mut Vec<Building>, kind: &str, resource: Option<&str>, add: f64) {
        if add <= 1e-9 {
            return;
        }
        let found = buildings
            .iter_mut()
            .filter(|b| b.kind == kind)
            .find(|b| match (resource, b.resource.as_deref()) {
                (Some(r), Some(br)) => r == br,
                (None, None) => true,
                _ => false,
            });
        match found {
            Some(b) => b.area += add,
            None => buildings.push(Building {
                kind: kind.to_string(),
                resource: resource.map(str::to_string),
                area: add,
                deployed: 0.0,
            }),
        }
    }

    // 1) Decide new area plans (raising targets), bounded by total_area.
    let mut planning_remaining = total_area - buildings.iter().map(|b| b.area).sum::<f64>();

    let desired_res = (population / ecocap.max(1e-6)) * config.economy.housing_buffer;
    let res_area = find_area(&buildings, "residential", None);
    if res_area < desired_res - 1e-9 && planning_remaining > 0.0 {
        let add = (desired_res - res_area).min(planning_remaining);
        raise_area(&mut buildings, "residential", None, add);
        planning_remaining -= add;
    }

    let con_target = (total_area * 0.15).clamp(4.0, 12.0);
    let con_area = find_area(&buildings, "construction", None);
    if con_area < con_target - 1e-9 && planning_remaining > 0.0 {
        let add = (con_target - con_area).min(planning_remaining);
        raise_area(&mut buildings, "construction", None, add);
        planning_remaining -= add;
    }

    for (rt, darea) in &deposits {
        if planning_remaining <= 0.0 {
            break;
        }
        let cur = find_area(&buildings, "mining", Some(rt));
        if cur < *darea - 1e-9 {
            let add = (*darea - cur).min(planning_remaining);
            raise_area(&mut buildings, "mining", Some(rt), add);
            planning_remaining -= add;
        }
    }

    // 2) Build: grow deployed toward the planned area, spending resources.
    for b in buildings.iter_mut() {
        if !b.under_construction() {
            continue;
        }
        let spec = config.building_spec(&b.kind);
        let per_area = per_area_cost(spec, res_mod);
        let speed = spec.construction_speed * speed_mod * spec.productivity * labor;
        let desired = (b.area - b.deployed).min(speed);
        let inc = max_affordable_inc(&per_area, limit, spent, desired);
        if inc <= 1e-6 {
            continue;
        }
        let cost: Vec<(String, f64)> = per_area.iter().map(|(rt, c)| (rt.clone(), *c * inc)).collect();
        commit_spend(state, fid, spent, &cost);
        b.deployed += inc;
    }

    // 3) Ship building (shipyard area drives the queue).
    let construction_area: f64 = buildings
        .iter()
        .filter(|b| config.building_spec(&b.kind).role == "shipyard")
        .map(|b| b.deployed)
        .sum();
    let (mut ship_progress, target_class) = {
        let sb = state.city(cid).expect("city gone").ship_build.clone();
        (sb.progress, sb.target_class)
    };
    ship_progress += construction_area * labor * config.building_spec("construction").productivity;
    let ship_spec = config.ship_spec(&target_class);
    let mut spawned = false;
    if ship_progress >= ship_spec.build_points && can_pay(state, fid, &ship_spec.build_cost) {
        pay(state, fid, &ship_spec.build_cost);
        let body_pos = state.body_position(body_id);
        let new_id = *next_ship_id;
        *next_ship_id += 1;
        let name = format!("{}-{}", ship_spec.label, fid);
        state.ships.push(Ship {
            id: new_id,
            name: name.clone(),
            class: target_class.clone(),
            faction_id: fid,
            position: [body_pos[0] + 0.05, body_pos[1] + 0.05],
            hull: ship_spec.hull,
            target: None,
        });
        ship_progress -= ship_spec.build_points;
        events.push(format!("{} 建成 {} \"{}\"", faction_name(state, fid), ship_spec.label, name));
        spawned = true;
    }
    let next = if spawned { choose_next_class(state, fid, config, rng) } else { target_class.clone() };

    // 4) Write back.
    if let Some(c) = state.city_mut(cid) {
        c.buildings = buildings;
        c.ship_build.progress = ship_progress;
        c.ship_build.target_class = next;
    }
    *next_ship_id
}

// --- military ---------------------------------------------------------------

fn step_military(state: &mut State, config: &GameConfig, rng: &mut Prng, events: &mut Vec<String>) {
    let mut order: Vec<ShipId> = state.ships.iter().map(|s| s.id).collect();
    for i in (1..order.len()).rev() {
        let j = rng.range(i as u64 + 1) as usize;
        order.swap(i, j);
    }

    for ship_id in order {
        let Some(ship) = state.ship(ship_id) else { continue };
        if ship.hull <= 0.0 {
            continue;
        }
        let owner = ship.faction_id;
        let class = ship.class.clone();
        let pos = ship.position;
        let range = config.ship_spec(&class).attack_range;

        if let Some(target) = nearest_enemy_ship(state, config, owner, pos, range) {
            fire(state, config, ship_id, target, events);
            continue;
        }

        let Some(target) = resolve_target(state, config, ship_id, owner, pos, rng) else {
            continue;
        };

        let dest = target_position(state, target);
        let distance = dist(pos, dest);

        if let ShipTarget::City(cid) = target {
            let cpos = city_position(state, cid);
            if dist(pos, cpos) <= config.combat.siege_range {
                siege_city(state, config, ship_id, cid, events);
                continue;
            }
        }

        let speed = config.ship_spec(&class).speed;
        let step = if distance <= config.combat.arrival_eps { 0.0 } else { speed.min(distance) };
        if distance > 1e-9 {
            let nx = (dest[0] - pos[0]) / distance;
            let ny = (dest[1] - pos[1]) / distance;
            if let Some(s) = state.ship_mut(ship_id) {
                s.position = [s.position[0] + nx * step, s.position[1] + ny * step];
            }
        }

        if let Some(ship) = state.ship(ship_id) {
            let np = ship.position;
            if let Some(target) = nearest_enemy_ship(state, config, owner, np, range) {
                fire(state, config, ship_id, target, events);
            } else if let ShipTarget::City(cid) = target {
                let cpos = city_position(state, cid);
                if dist(np, cpos) <= config.combat.siege_range {
                    siege_city(state, config, ship_id, cid, events);
                }
            }
        }
    }

    state.ships.retain(|s| s.hull > 0.0);
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

fn fire(state: &mut State, config: &GameConfig, attacker_id: ShipId, target_id: ShipId, events: &mut Vec<String>) {
    let (dmg, aname, afac) = {
        let a = state.ship(attacker_id).expect("attacker gone");
        (config.ship_spec(&a.class).attack, a.name.clone(), a.faction_id)
    };
    let (tname, tfac, hull) = {
        let t = state.ship(target_id).expect("target gone");
        (t.name.clone(), t.faction_id, t.hull)
    };
    let new_hull = hull - dmg;
    let destroyed = new_hull <= 0.0;
    if let Some(t) = state.ship_mut(target_id) {
        t.hull = if destroyed { 0.0 } else { new_hull };
    }
    adjust_relation(state, afac, tfac, config.diplomacy.attack_delta);

    let an = faction_name(state, afac);
    let tn = faction_name(state, tfac);
    if destroyed {
        events.push(format!("{} 击毁 {} 的 \"{}\"", an, tn, tname));
    } else {
        events.push(format!("{} 的 \"{}\" 炮击 {} 的 \"{}\"", an, aname, tn, tname));
    }
}

fn rescue_target(state: &State, config: &GameConfig, t: ShipTarget, owner: FactionId) -> bool {
    match t {
        ShipTarget::Body(_) | ShipTarget::Position(_) => true,
        ShipTarget::Ship(id) => state
            .ship(id)
            .map(|s| s.hull > 0.0 && hostile(state, config, owner, s.faction_id))
            .unwrap_or(false),
        ShipTarget::City(id) => state
            .city(id)
            .map(|c| hostile(state, config, owner, c.faction_id))
            .unwrap_or(false),
    }
}

fn resolve_target(state: &mut State, config: &GameConfig, ship_id: ShipId, owner: FactionId, pos: [f64; 2], rng: &mut Prng) -> Option<ShipTarget> {
    let cur = state.ship(ship_id).expect("ship gone").target;
    if let Some(t) = cur {
        if rescue_target(state, config, t, owner) {
            return Some(t);
        }
    }
    let picked = pick_target(state, config, owner, pos, rng);
    if let Some(t) = picked {
        if let Some(s) = state.ship_mut(ship_id) {
            s.target = Some(t);
        }
    }
    picked
}

fn pick_target(state: &State, config: &GameConfig, owner: FactionId, pos: [f64; 2], rng: &mut Prng) -> Option<ShipTarget> {
    let mut best: Option<(f64, ShipTarget)> = None;
    let mut consider = |d: f64, t: ShipTarget, best: &mut Option<(f64, ShipTarget)>| {
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
            *best = Some((d, t));
        }
    };

    for s in &state.ships {
        if s.hull > 0.0 && hostile(state, config, owner, s.faction_id) {
            consider(dist(pos, s.position), ShipTarget::Ship(s.id), &mut best);
        }
    }
    for c in &state.cities {
        if hostile(state, config, owner, c.faction_id) {
            let p = city_position(state, c.id);
            consider(dist(pos, p), ShipTarget::City(c.id), &mut best);
        }
    }
    best.map(|(_, t)| t)
}

fn siege_city(state: &mut State, config: &GameConfig, ship_id: ShipId, cid: CityId, events: &mut Vec<String>) {
    let (dmg, attacker, aname) = {
        let s = state.ship(ship_id).expect("ship gone");
        (config.ship_spec(&s.class).attack, s.faction_id, s.name.clone())
    };
    let (cname, old_owner) = {
        let c = state.city(cid).expect("city gone");
        (c.name.clone(), c.faction_id)
    };

    let taken = {
        let c = state.city_mut(cid).expect("city gone");
        c.defense -= dmg;
        if c.defense <= 0.0 {
            c.faction_id = attacker;
            c.defense = config.combat.defense_reset;
            c.population = (c.population as f64 * 0.6) as u32;
            true
        } else {
            false
        }
    };

    if taken {
        events.push(format!(
            "{} 攻占 {} 的城市 \"{}\"",
            faction_name(state, attacker),
            faction_name(state, old_owner),
            cname
        ));
        adjust_relation(state, old_owner, attacker, config.diplomacy.capture_delta);
    } else {
        events.push(format!("{} 的 \"{}\" 围攻 {}", faction_name(state, attacker), aname, cname));
    }
}

fn target_position(state: &State, t: ShipTarget) -> [f64; 2] {
    match t {
        ShipTarget::Body(id) => state.body_position(id),
        ShipTarget::City(id) => city_position(state, id),
        ShipTarget::Ship(id) => state.ship(id).map(|s| s.position).unwrap_or([0.0, 0.0]),
        ShipTarget::Position(p) => p,
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
