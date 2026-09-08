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
///
/// Query mode (`--query` / `q`) additionally attaches the full story chronicle
/// as a top-level `story` field, so an agent can read the narrative arc with
/// `.story` (or `.story[] | select(...)`). The per-round `render_state` stays
/// lean and only carries the chapter's own `events`.
pub fn state_value(state: &State, config: &GameConfig) -> serde_json::Value {
    let doc = AgentState::from_state(state, config);
    let mut v = serde_json::to_value(doc).expect("agent state is serializable");
    if let serde_json::Value::Object(ref mut m) = v {
        m.insert("story".to_string(), story_value(state));
    }
    v
}

/// The story chronicle (`State::chronicle`) as a JSON array, for `story` /
/// `.story` queries. This is the full, growing narrative arc of the run.
pub fn story_value(state: &State) -> serde_json::Value {
    serde_json::to_value(&state.chronicle).expect("chronicle is serializable")
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
/// * `ships`      ship *class* → full spec (hull, hull_regen, attack, speed,
///                range, build points/cost). An agent needs these to decide
///                what to build.
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
                    "default_build_weight": r2(b.default_build_weight),
                }),
            )
        })
        .collect();
    let structures: BTreeMap<String, serde_json::Value> = config
        .structures
        .iter()
        .map(|(k, s)| {
            (
                k.clone(),
                json!({
                    "name": s.name,
                    "armor_per_area": r2(s.armor_per_area),
                    "cost_mult": r2(s.cost_mult),
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
                    "hull_regen": r2(s.hull_regen),
                    "attack": r2(s.attack),
                    "speed": r2(s.speed),
                    "attack_range": r2(s.attack_range),
                    "build_points": r2(s.build_points),
                    "build_cost": to_cost(&s.build_cost),
                    "upkeep": r2(s.upkeep),
                }),
            )
        })
        .collect();
    json!({
        "resources": resources,
        "structures": structures,
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
            "armor_regen": r2(config.combat.armor_regen),
            "colony_footprint": r2(config.combat.colony_footprint),
        },
        "diplomacy": {
            "attack_delta": r2(config.diplomacy.attack_delta),
            "capture_delta": r2(config.diplomacy.capture_delta),
            "drift_rate": r2(config.diplomacy.drift_rate),
            "war_fatigue": r2(config.diplomacy.war_fatigue),
            "ceasefire_relation": r2(config.diplomacy.ceasefire_relation),
            "affinity_floor": r2(config.diplomacy.affinity_floor),
            "affinity_span": r2(config.diplomacy.affinity_span),
            "noise": r2(config.diplomacy.noise),
            "hostility_floor": r2(config.diplomacy.hostility_floor),
            "friendship_ceiling": r2(config.diplomacy.friendship_ceiling),
        },
        "market": {
            "auto_trade_limit": r2(config.market.auto_trade_limit),
            "working_buffer": r2(config.market.working_buffer),
            "spread": r2(config.market.spread),
            "resource_value": config
                .resources
                .iter()
                .map(|(k, r)| (k.clone(), r2(r.value)))
                .collect::<BTreeMap<_, _>>(),
        },
        "story": config
            .story
            .iter()
            .map(|s| {
                let trigger = match &s.trigger {
                    StoryTrigger::RoundAt { round } => json!({"kind": "round_at", "round": round}),
                    StoryTrigger::FirstWar => json!({"kind": "first_war"}),
                    StoryTrigger::FirstRaze => json!({"kind": "first_raze"}),
                    StoryTrigger::FirstColony => json!({"kind": "first_colony"}),
                    StoryTrigger::WarBetween { a, b } => json!({"kind": "war_between", "a": a, "b": b}),
                    StoryTrigger::FactionAtWar { faction } => json!({"kind": "faction_at_war", "faction": faction}),
                    StoryTrigger::RelationBelow { a, b, value } => {
                        json!({"kind": "relation_below", "a": a, "b": b, "value": r2(*value)})
                    }
                };
                json!({"id": s.id, "title": s.title, "trigger": trigger})
            })
            .collect::<Vec<_>>(),
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
    /// 本回合事件（谁开火/被毁/城被夷平/殖民/陈旧指令降级），damage 已四舍五入。
    events: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct AgentFaction {
    id: FactionId,
    name: String,
    /// 意识形态位置 (-1..1；越正越「西方/国际」，越负越「东方/教派」)。
    alignment: f64,
    /// 好战度 (0..1)：越高越会加速与异己阵营敌对化。
    aggression: f64,
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
    /// 定居点列表（定居点 ↔ 城市 1:1；地球 5 个，其余天体各 1 个）。
    settlements: Vec<AgentSettlement>,
}

/// A body's orbit, summarised to the numbers that matter for planning travel.
#[derive(Serialize)]
struct AgentOrbit {
    perihelion: f64,
    aphelion: f64,
    /// Orbit period in months.
    period: f64,
}

/// One 定居点 (settlement site) on a body: how much can be built, how fast, and
/// what resources are available to mine here (资源矿藏 bounds the mining area).
#[derive(Serialize)]
struct AgentSettlement {
    name: String,
    /// 总面积 (total buildable area of this site).
    total_area: f64,
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
    /// Index of the 定居点 this city occupies within its body's `settlements`.
    settlement_index: usize,
    /// 定居点名 (settlement ↔ city 1:1).
    settlement_name: String,
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
    Dock { body: BodyId },
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
                alignment: r2(f.alignment),
                aggression: r2(f.aggression),
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
                settlements: b
                    .settlements
                    .iter()
                    .map(|s| AgentSettlement {
                        name: s.name.clone(),
                        total_area: r2(s.total_area),
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
                    })
                    .collect(),
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
                settlement_index: c.settlement,
                settlement_name: state
                    .city_settlement(c.id)
                    .map(|s| s.name.clone())
                    .unwrap_or_default(),
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
                    Some(ShipBehavior::Dock { body }) => AgentOrder::Dock { body },
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
            events: state.events.iter().map(game_event_value).collect(),
        }
    }
}

/// Render one [`GameEvent`] as a clean, rounded tagged-union JSON value for the
/// agent (damage rounded to 2 decimals, matching the rest of the agent output).
fn game_event_value(e: &GameEvent) -> serde_json::Value {
    use GameEvent::*;
    match e {
        Attack { attacker, target, damage } => {
            json!({"type":"attack", "attacker": attacker, "target": target, "damage": r2(*damage)})
        }
        ShipDestroyed { ship, owner, class } => {
            json!({"type":"ship_destroyed", "ship": ship, "owner": owner, "class": class})
        }
        Siege { attacker, city, damage } => {
            json!({"type":"siege", "attacker": attacker, "city": city, "damage": r2(*damage)})
        }
        CityRazed { city, fallen_to } => {
            json!({"type":"city_razed", "city": city, "fallen_to": fallen_to})
        }
        ShipSpawned { ship, owner, class, city } => {
            json!({"type":"ship_spawned", "ship": ship, "owner": owner, "class": class, "city": city})
        }
        ColonyFounded { city, owner, body, seeded_ship_class } => {
            json!({"type":"colony_founded", "city": city, "owner": owner, "body": body, "seeded_ship_class": seeded_ship_class})
        }
        StaleOrder { ship, reason } => {
            json!({"type":"stale_order", "ship": ship, "reason": reason})
        }
        WarStarted { a, b } => {
            json!({"type":"war_started", "a": a, "b": b})
        }
        WarEnded { a, b } => {
            json!({"type":"war_ended", "a": a, "b": b})
        }
        Story { id, title } => {
            json!({"type":"story", "id": id, "title": title})
        }
    }
}

fn faction_name(state: &State, id: FactionId) -> String {
    state
        .faction(id)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| format!("#{id}"))
}
