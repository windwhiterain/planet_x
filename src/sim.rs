//! Round-stepping simulation engine.
//!
//! [`advance`] moves the world forward by one round (month). Everything that
//! affects game balance (production rate, build costs, combat ranges, diplomacy
//! deltas) is read from the [`GameConfig`]; no magic numbers live here.

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

fn efficiency(capacity: f64, population: u32, config: &GameConfig) -> f64 {
    if capacity <= 0.0 {
        return 0.0;
    }
    (population as f64 / capacity).clamp(config.economy.min_efficiency, 1.0)
}

fn can_afford(state: &State, faction_id: FactionId, spec: &ShipSpec) -> bool {
    spec.build_cost
        .iter()
        .all(|(rt, cost)| state.faction(faction_id).map_or(0.0, |f| f.resources.get(rt).copied().unwrap_or(0.0)) >= *cost)
}

fn pay_cost(state: &mut State, faction_id: FactionId, spec: &ShipSpec) {
    for (rt, cost) in &spec.build_cost {
        if let Some(f) = state.faction_mut(faction_id) {
            let e = f.resources.entry(*rt).or_insert(0.0);
            *e = (*e - cost).max(0.0);
        }
    }
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

fn step_production(state: &mut State, config: &GameConfig) {
    let city_ids: Vec<CityId> = state.cities.iter().map(|c| c.id).collect();
    for cid in city_ids {
        let (body_id, faction_id, population) = {
            let c = state.city(cid).expect("city disappeared");
            (c.body_id, c.faction_id, c.population)
        };
        let capacity = state
            .body(body_id)
            .and_then(|b| b.settlement.as_ref())
            .map(|s| s.population_capacity as f64)
            .unwrap_or(0.0);
        if capacity <= 0.0 {
            continue;
        }
        let eff = efficiency(capacity, population, config);

        // Mining: each mining point yields resources to the controlling faction.
        let points: Vec<(ResourceType, f64)> = state
            .city(cid)
            .expect("city disappeared")
            .mining_points
            .iter()
            .map(|m| (m.resource_type, m.area as f64))
            .collect();
        for (rt, area) in points {
            let output = area * eff * config.economy.production_rate;
            if let Some(f) = state.faction_mut(faction_id) {
                *f.resources.entry(rt).or_insert(0.0) += output;
            }
        }

        // Population grows toward capacity.
        let delta = ((capacity - population as f64) * config.economy.pop_growth).round() as i64;
        if delta > 0 {
            if let Some(c) = state.city_mut(cid) {
                c.population = (c.population as i64 + delta).min(capacity as i64) as u32;
            }
        }
    }
}

// --- construction -----------------------------------------------------------

fn step_construction(state: &mut State, config: &GameConfig, rng: &mut Prng, events: &mut Vec<String>) {
    let city_ids: Vec<CityId> = state.cities.iter().map(|c| c.id).collect();
    let mut next_ship_id = state.ships.iter().map(|s| s.id).max().map_or(0, |m| m + 1);

    for cid in city_ids {
        let Some(city) = state.city(cid) else { continue };
        let Some(cp) = city.construction.as_ref() else { continue };
        let faction_id = city.faction_id;
        let body_id = city.body_id;
        let population = city.population;
        let target_class = cp.target;
        let current_progress = cp.progress;

        let spec = config.ship_spec(target_class);
        let construction_speed = state
            .body(body_id)
            .and_then(|b| b.settlement.as_ref())
            .map(|s| s.construction_speed)
            .unwrap_or(1.0);
        let capacity = state
            .body(body_id)
            .and_then(|b| b.settlement.as_ref())
            .map(|s| s.population_capacity as f64)
            .unwrap_or(1.0);
        let eff = efficiency(capacity, population, config);
        let progress = current_progress + construction_speed * eff;
        let done = progress >= spec.build_points;
        let affordable = can_afford(state, faction_id, spec);

        if done && affordable {
            pay_cost(state, faction_id, spec);
            let body_pos = state.body_position(body_id);
            let new_id = next_ship_id;
            next_ship_id += 1;
            let name = format!("{}-{}", spec.label, faction_id);
            let jitter = 0.06;
            state.ships.push(Ship {
                id: new_id,
                name,
                class: target_class,
                faction_id,
                position: [
                    body_pos[0] + rng.range_f64(-jitter, jitter),
                    body_pos[1] + rng.range_f64(-jitter, jitter),
                ],
                hull: spec.hull,
                target: None,
            });
            // Reset the yard and pick the next class to build.
            let next_class = choose_next_class(state, faction_id, config, rng);
            if let Some(c) = state.city_mut(cid) {
                if let Some(cp) = c.construction.as_mut() {
                    cp.progress = 0.0;
                    cp.target = next_class;
                }
            }
            events.push(format!(
                "{} 建成 {} \"{}\"",
                faction_name(state, faction_id),
                spec.label,
                state.ship(new_id).map(|s| s.name.as_str()).unwrap_or("")
            ));
        } else {
            // Hold progress at the required build points (waiting on resources).
            let held = progress.min(spec.build_points);
            if let Some(c) = state.city_mut(cid) {
                if let Some(cp) = c.construction.as_mut() {
                    cp.progress = held;
                }
            }
        }
    }
}

fn choose_next_class(state: &State, faction_id: FactionId, config: &GameConfig, rng: &mut Prng) -> ShipClass {
    let affordable: Vec<ShipClass> = ShipClass::ALL
        .into_iter()
        .filter(|c| can_afford(state, faction_id, config.ship_spec(*c)))
        .collect();
    if affordable.is_empty() {
        ShipClass::Corvette
    } else {
        *rng.pick(&affordable)
    }
}

// --- military ---------------------------------------------------------------

fn step_military(state: &mut State, config: &GameConfig, rng: &mut Prng, events: &mut Vec<String>) {
    // Resolve ship actions in a seed-shuffled order so outcomes vary by seed.
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
        let class = ship.class;
        let pos = ship.position;
        let range = config.ship_spec(class).attack_range;

        // 1) Fire at the nearest enemy ship already in range.
        if let Some(target) = nearest_enemy_ship(state, config, owner, pos, range) {
            fire(state, config, ship_id, target, events);
            continue;
        }

        // 2) Otherwise figure out a target and move/engage.
        let Some(target) = resolve_target(state, config, ship_id, owner, pos, rng) else {
            continue;
        };

        let dest = target_position(state, target);
        let distance = dist(pos, dest);

        // 3) Siege an enemy city we are already standing in front of.
        if let ShipTarget::City(cid) = target {
            let cpos = city_position(state, cid);
            if dist(pos, cpos) <= config.combat.siege_range {
                siege_city(state, config, ship_id, cid, events);
                continue;
            }
        }

        // 4) Move toward the target.
        let speed = config.ship_spec(class).speed;
        let step = if distance <= config.combat.arrival_eps {
            0.0
        } else {
            speed.min(distance)
        };
        if distance > 1e-9 {
            let nx = (dest[0] - pos[0]) / distance;
            let ny = (dest[1] - pos[1]) / distance;
            if let Some(s) = state.ship_mut(ship_id) {
                s.position = [s.position[0] + nx * step, s.position[1] + ny * step];
            }
        }

        // 5) Fire (or begin a siege) once more if the move brought us into range.
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

    // Drop destroyed ships.
    state.ships.retain(|s| s.hull > 0.0);
}

fn nearest_enemy_ship(
    state: &State,
    config: &GameConfig,
    owner: FactionId,
    pos: [f64; 2],
    range: f64,
) -> Option<ShipId> {
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
        (config.ship_spec(a.class).attack, a.name.clone(), a.faction_id)
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

fn resolve_target(
    state: &mut State,
    config: &GameConfig,
    ship_id: ShipId,
    owner: FactionId,
    pos: [f64; 2],
    rng: &mut Prng,
) -> Option<ShipTarget> {
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

fn siege_city(
    state: &mut State,
    config: &GameConfig,
    ship_id: ShipId,
    cid: CityId,
    events: &mut Vec<String>,
) {
    let (dmg, attacker, aname) = {
        let s = state.ship(ship_id).expect("ship gone");
        (config.ship_spec(s.class).attack, s.faction_id, s.name.clone())
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
