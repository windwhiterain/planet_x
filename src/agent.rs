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
    /// Total buildable area, when this body has a settlement.
    settlement_area: Option<f64>,
}

#[derive(Serialize)]
struct AgentCity {
    id: CityId,
    name: String,
    body: String,
    owner: FactionId,
    owner_name: String,
    population: u32,
    /// 被夷平为空白 (razed) — a city with no buildings, colonizable again.
    razed: bool,
    /// 城市总硬度 = Σ building armor (no separate city defense).
    armor: f64,
    /// Per-class ship production progress (建造区各舰型进度累加).
    ship_progress: BTreeMap<String, f64>,
    buildings: Vec<AgentBuilding>,
}

#[derive(Serialize)]
struct AgentBuilding {
    id: BuildingId,
    kind: String,
    /// 建筑自身属性: concrete 混凝土 | steel 钢结构.
    structure: String,
    /// Mined resource display name, when this building mines one.
    resource: Option<String>,
    /// Ship class produced, when this building is a 建造区.
    ship_type: Option<String>,
    area: f64,
    deployed: f64,
    armor: f64,
    armor_max: f64,
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
    Colonize { body: BodyId },
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
                settlement_area: b.settlement.as_ref().map(|s| r2(s.total_area)),
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
                razed: c.razed,
                armor: r2(c.buildings.iter().map(|b| b.armor).sum::<f64>()),
                ship_progress: c.ship_progress.iter().map(|(k, v)| (k.clone(), r2(*v))).collect(),
                buildings: c
                    .buildings
                    .iter()
                    .map(|b| AgentBuilding {
                        id: b.id,
                        kind: b.kind.clone(),
                        structure: b.structure.clone(),
                        resource: b.resource.as_ref().map(|r| config.resource_name(r)),
                        ship_type: b.ship_type.clone(),
                        area: r2(b.area),
                        deployed: r2(b.deployed),
                        armor: r2(b.armor),
                        armor_max: r2(b.armor_max(config)),
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
                    Some(ShipBehavior::Colonize { body }) => AgentOrder::Colonize { body },
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
