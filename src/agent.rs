//! Machine-readable ("zero-noise") rendering for an LLM agent player.
//!
//! The CLI's default human output (ASCII star map, comfy-table with Unicode
//! box-drawing characters, colored legend, Chinese prose) is optimised for a
//! terminal and is hostile to a machine. This module is the exact opposite:
//! it renders a single [`State`] as one compact, stable JSON object with no
//! colour, no tables, no decoration, and values rounded to kill token noise.
//!
//! Emit one object per line (JSON Lines): `round 0` first, then one per round
//! as the simulation advances. Field order is fixed and stable; floats are
//! rounded to 2 decimals; empty resource buckets and sub-threshold values are
//! dropped so the document stays small.

use crate::model::*;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;

/// Round a float to 2 decimals (token-noise reduction).
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The agent state as a `serde_json::Value`, for in-process querying.
pub fn state_value(state: &State, config: &GameConfig) -> serde_json::Value {
    let doc = AgentState::from_state(state, config);
    serde_json::to_value(doc).expect("agent state is serializable")
}

/// Zero-noise rendering of one state as a single-line JSON object.
pub fn render_state(state: &State, config: &GameConfig) -> String {
    let doc = AgentState::from_state(state, config);
    serde_json::to_string(&doc).expect("agent state is serializable")
}

/// The game's full tunable configuration, rendered as one JSON object for the
/// `meta` command / `--meta` flag. This is the agent's "rules dictionary":
///
/// * `resources`  resource *key* → display name. This is the authoritative
///                table an agent uses to translate the display-name keys it sees
///                in [`state_value`] into the raw `control` keys it must write
///                (e.g. `"水冰"` → `"water_ice"`).
/// * `buildings`  building *kind* → full spec (role, construction speed/cost,
///                staffing, productivity, default invest weight).
/// * `ships`      ship *class* → full spec (hull, attack, speed, range, build
///                points/cost). An agent needs these to decide what to build.
/// * `economy` / `combat` / `diplomacy`  the numeric tuning constants
///                (`invest_fraction`, `war_threshold`, `production_rate`, …).
///
/// Every key here is a raw config key (not a display name), so it lines up
/// directly with the `control` template and the machine keys; the `resources`
/// table is the single place that maps raw key ↔ 中文名.
pub fn meta_value(config: &GameConfig) -> serde_json::Value {
    let to_cost = |m: &BTreeMap<String, f64>| m.iter().map(|(k, v)| (k.clone(), r2(*v))).collect::<BTreeMap<_, _>>();

    let resources: BTreeMap<String, String> =
        config.resources.iter().map(|(k, d)| (k.clone(), d.name.clone())).collect();
    let buildings: BTreeMap<String, serde_json::Value> = config
        .buildings
        .iter()
        .map(|(k, b)| {
            (
                k.clone(),
                json!({
                    "label": b.label,
                    "role": b.role,
                    "construction_speed": r2(b.construction_speed),
                    "build_cost": to_cost(&b.build_cost),
                    "staff_per_area": r2(b.staff_per_area),
                    "productivity": r2(b.productivity),
                    "default_invest_weight": r2(b.default_invest_weight),
                }),
            )
        })
        .collect();
    let ships: BTreeMap<String, serde_json::Value> = config
        .ships
        .iter()
        .map(|(k, s)| {
            (
                k.clone(),
                json!({
                    "label": s.label,
                    "hull": r2(s.hull),
                    "attack": r2(s.attack),
                    "speed": r2(s.speed),
                    "attack_range": r2(s.attack_range),
                    "build_points": r2(s.build_points),
                    "build_cost": to_cost(&s.build_cost),
                }),
            )
        })
        .collect();
    json!({
        "resources": resources,
        "buildings": buildings,
        "ships": ships,
        "economy": {
            "production_rate": r2(config.economy.production_rate),
            "pop_growth": r2(config.economy.pop_growth),
            "min_efficiency": r2(config.economy.min_efficiency),
            "invest_fraction": r2(config.economy.invest_fraction),
            "housing_buffer": r2(config.economy.housing_buffer),
        },
        "combat": {
            "war_threshold": r2(config.combat.war_threshold),
            "siege_range": r2(config.combat.siege_range),
            "arrival_eps": r2(config.combat.arrival_eps),
            "defense_initial": r2(config.combat.defense_initial),
            "defense_reset": r2(config.combat.defense_reset),
        },
        "diplomacy": {
            "attack_delta": r2(config.diplomacy.attack_delta),
            "capture_delta": r2(config.diplomacy.capture_delta),
            "relax_rate": r2(config.diplomacy.relax_rate),
        },
    })
}

/// The compact, decision-relevant view of a round.
#[derive(Serialize)]
struct AgentState {
    round: u32,
    time_month: f64,
    factions: Vec<AgentFaction>,
    bodies: Vec<AgentBody>,
    cities: Vec<AgentCity>,
    ships: Vec<AgentShip>,
}

#[derive(Serialize)]
struct AgentFaction {
    id: FactionId,
    name: String,
    /// Nonzero resource stockpile (resource display name -> amount).
    resources: BTreeMap<String, f64>,
    /// Relations toward other factions (name -> relation, nonzero only).
    relations: BTreeMap<String, f64>,
    /// Names of factions at or below the war threshold.
    wars: Vec<String>,
}

#[derive(Serialize)]
struct AgentBody {
    id: BodyId,
    name: String,
    position: [f64; 2],
    /// Keplerian orbit, so the agent can reason about travel time.
    orbit: AgentOrbit,
    /// Total buildable area, when this body has a settlement.
    settlement_area: Option<f64>,
    /// Full settlement detail (capacity, construction modifiers, deposits).
    settlement: Option<AgentSettlement>,
}

/// A body's orbit, summarised to the numbers that matter for planning travel.
#[derive(Serialize)]
struct AgentOrbit {
    perihelion: f64,
    aphelion: f64,
    /// Orbit period in months.
    period: f64,
}

/// Settlement detail: how much can be built, how fast, and what resources are
/// available to mine here (资源矿藏 bounds the mining area).
#[derive(Serialize)]
struct AgentSettlement {
    ecological_capacity: f64,
    construction_speed_mod: f64,
    construction_resource_mod: f64,
    /// Mineable deposits: (display name, area). `area` caps the mining area.
    deposits: Vec<AgentDeposit>,
}

#[derive(Serialize)]
struct AgentDeposit {
    resource: String,
    area: f64,
}

#[derive(Serialize)]
struct AgentCity {
    id: CityId,
    name: String,
    body: String,
    owner: FactionId,
    owner_name: String,
    population: u32,
    defense: f64,
    /// Ship class currently being built (建造点 queue).
    building: String,
    /// Progress toward finishing the current `building` (in build points).
    ship_build_progress: f64,
    buildings: Vec<AgentBuilding>,
}

#[derive(Serialize)]
struct AgentBuilding {
    kind: String,
    /// Mined resource display name, when this building mines one.
    resource: Option<String>,
    area: f64,
    deployed: f64,
}

#[derive(Serialize)]
struct AgentShip {
    id: ShipId,
    name: String,
    class: String,
    owner: FactionId,
    owner_name: String,
    position: [f64; 2],
    hull: f64,
    hull_max: f64,
    order: AgentOrder,
}

/// A ship's current command, as a tagged union for clean parsing.
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AgentOrder {
    Idle,
    Move { position: [f64; 2] },
    TargetShip { ship: ShipId, attack: bool },
    TargetSettlement { city: CityId, bombard: bool },
}

impl AgentState {
    fn from_state(state: &State, config: &GameConfig) -> Self {
        let factions = state
            .factions
            .iter()
            .map(|f| AgentFaction {
                id: f.id,
                name: f.name.clone(),
                resources: f
                    .resources
                    .iter()
                    .filter(|(_, v)| **v >= 0.05)
                    .map(|(k, v)| (config.resource_name(k), r2(*v)))
                    .collect(),
                relations: f
                    .relations
                    .iter()
                    .filter(|(_, v)| v.abs() >= 0.5)
                    .map(|(id, v)| {
                        (
                            state.faction(*id).map(|x| x.name.clone()).unwrap_or_else(|| format!("#{id}")),
                            r2(*v),
                        )
                    })
                    .collect(),
                wars: f
                    .relations
                    .iter()
                    .filter(|(_, v)| **v <= config.combat.war_threshold)
                    .map(|(id, _)| state.faction(*id).map(|x| x.name.clone()).unwrap_or_else(|| format!("#{id}")))
                    .collect(),
            })
            .collect();

        let bodies = state
            .bodies
            .iter()
            .map(|b| AgentBody {
                id: b.id,
                name: b.name.clone(),
                position: [r2(b.position[0]), r2(b.position[1])],
                orbit: AgentOrbit {
                    perihelion: r2(b.orbit.perihelion_distance as f64),
                    aphelion: r2(b.orbit.aphelion_distance as f64),
                    period: r2(b.orbit.period as f64),
                },
                settlement_area: b.settlement.as_ref().map(|s| r2(s.total_area)),
                settlement: b.settlement.as_ref().map(|s| AgentSettlement {
                    ecological_capacity: r2(s.ecological_capacity),
                    construction_speed_mod: r2(s.construction_speed_mod),
                    construction_resource_mod: r2(s.construction_resource_mod),
                    deposits: s
                        .resources
                        .iter()
                        .map(|d| AgentDeposit {
                            resource: config.resource_name(&d.resource),
                            area: r2(d.area),
                        })
                        .collect(),
                }),
            })
            .collect();

        let cities = state
            .cities
            .iter()
            .map(|c| AgentCity {
                id: c.id,
                name: c.name.clone(),
                body: state
                    .body(c.body_id)
                    .map(|b| b.name.clone())
                    .unwrap_or_else(|| format!("#{}", c.body_id)),
                owner: c.faction_id,
                owner_name: faction_name(state, c.faction_id),
                population: c.population,
                defense: r2(c.defense),
                building: c.ship_build.target_class.clone(),
                ship_build_progress: r2(c.ship_build.progress),
                buildings: c
                    .buildings
                    .iter()
                    .map(|b| AgentBuilding {
                        kind: b.kind.clone(),
                        resource: b.resource.as_ref().map(|r| config.resource_name(r)),
                        area: r2(b.area),
                        deployed: r2(b.deployed),
                    })
                    .collect(),
            })
            .collect();

        let ships = state
            .ships
            .iter()
            .map(|s| AgentShip {
                id: s.id,
                name: s.name.clone(),
                class: s.class.clone(),
                owner: s.faction_id,
                owner_name: faction_name(state, s.faction_id),
                position: [r2(s.position[0]), r2(s.position[1])],
                hull: r2(s.hull),
                hull_max: r2(config.ship_spec(&s.class).hull),
                order: match state.ship_behavior(s.id) {
                    Some(ShipBehavior::Idle) | None => AgentOrder::Idle,
                    Some(ShipBehavior::Move { position }) => AgentOrder::Move {
                        position: [r2(position[0]), r2(position[1])],
                    },
                    Some(ShipBehavior::TargetShip { ship, attack }) => {
                        AgentOrder::TargetShip { ship, attack }
                    }
                    Some(ShipBehavior::TargetSettlement { city, bombard }) => {
                        AgentOrder::TargetSettlement { city, bombard }
                    }
                },
            })
            .collect();

        AgentState {
            round: state.round,
            time_month: r2(state.time_month),
            factions,
            bodies,
            cities,
            ships,
        }
    }
}

fn faction_name(state: &State, id: FactionId) -> String {
    state
        .faction(id)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| format!("#{id}"))
}
