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
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;

/// Round a float to 2 decimals (token-noise reduction).
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The agent state as a `serde_json::Value`, with the full story chronicle attached
/// as a top-level `story` field, so an agent can read the narrative arc alongside
/// the current snapshot. The per-round [`render_state`] / [`state_json`] stay lean
/// and only carry the chapter's own `events` — use this when you want the whole
/// story in one document.
pub fn state_value(state: &State, config: &GameConfig) -> serde_json::Value {
    let mut v = state_json(state, config);
    if let serde_json::Value::Object(ref mut m) = v {
        m.insert("story".to_string(), story_value(state));
    }
    v
}

/// The agent state view as a `serde_json::Value`, without the story chronicle.
///
/// This is the atomic per-round snapshot used to build a **trajectory**: a linear
/// timeline of agent views, one per round, that the agent can index and query over
/// time. Kept separate from [`state_value`] so a trajectory of thousands of rounds
/// doesn't drag along a copy of the whole `story` chronicle in every snapshot.
pub fn state_json(state: &State, config: &GameConfig) -> serde_json::Value {
    serde_json::to_value(AgentState::from_state(state, config)).expect("agent state is serializable")
}

/// The story chronicle (`State::chronicle`) as a JSON array, for `story` /
/// `.story` queries. This is the full, growing narrative arc of the run.
pub fn story_value(state: &State) -> serde_json::Value {
    serde_json::to_value(&state.chronicle).expect("chronicle is serializable")
}

/// A JSON Schema for the agent's per-round state view (`AgentState`), so an agent
/// can introspect the exact field names / types it may query — instead of
/// memorising the schema. It stays in sync because it is *derived* from the same
/// `#[derive(Serialize, JsonSchema)]` structs the state is rendered from. Expose
/// via the `schema` REPL command (optionally piped through a jq filter).
pub fn schema_value() -> serde_json::Value {
    let schema = schemars::schema_for!(AgentState);
    serde_json::to_value(schema).expect("schema is serializable")
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
///
/// Everything below except `resources`/`story` is rendered by [`config_json`],
/// i.e. derived straight from the config structs — so the rules dictionary can
/// never drift from `config/game.ron`. (The old hand-written field lists already
/// omitted `combat.component_spill`, `component_repair`, `escort_range`,
/// `pursuit_range` and `pd_radius`.) `resources` is the intentional raw-key →
/// 中文名 translation table; `story` maps trigger/effects into a flat readable
/// shape.
///
/// Round every float in a JSON tree to 2 decimals (agent token-noise reduction).
/// Integer numbers (ids, `slots`, `min_members`, …) are left untouched so they
/// don't render as `3.0`.
fn round_value(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Number(n) if n.is_f64() => {
            if let Some(f) = n.as_f64() {
                *v = serde_json::Value::from((f * 100.0).round() / 100.0);
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(round_value),
        serde_json::Value::Object(m) => m.values_mut().for_each(round_value),
        _ => {}
    }
}

/// Serialize any serializable config value to JSON with floats already rounded
/// to 2 decimals. This is the single-source renderer for `meta_value`'s config
/// sections: derive from the struct, never transcribe a field list.
fn config_json<T: serde::Serialize>(value: &T) -> serde_json::Value {
    let mut v = serde_json::to_value(value).expect("config value is serializable");
    round_value(&mut v);
    v
}

pub fn meta_value(config: &GameConfig) -> serde_json::Value {
    let resources: BTreeMap<String, String> =
        config.resources.iter().map(|(k, d)| (k.clone(), d.name.clone())).collect();

    // `market` = 当前配置结构 + 由资源定义派生的每资源价值表。
    let mut market = config_json(&config.market);
    if let serde_json::Value::Object(m) = &mut market {
        m.insert(
            "resource_value".to_string(),
            config_json(
                &config
                    .resources
                    .iter()
                    .map(|(k, r)| (k.clone(), r.value))
                    .collect::<BTreeMap<_, _>>(),
            ),
        );
    }

    json!({
        "resources": resources,
        "structures": config_json(&config.structures),
        "buildings": config_json(&config.buildings),
        "ships": config_json(&config.ships),
        "components": config_json(&config.components),
        "economy": config_json(&config.economy),
        "combat": config_json(&config.combat),
        "diplomacy": config_json(&config.diplomacy),
        "market": market,
        "governance": config_json(&config.governance),
        "mond": config_json(&config.mond),
        "balance": config_json(&config.balance),
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
                let effects = s
                    .effects
                    .iter()
                    .map(|e| match e {
                        StoryEffect::Relations { a, b, delta } => {
                            json!({"kind": "relations", "a": a, "b": b, "delta": r2(*delta)})
                        }
                        StoryEffect::GrantResources { faction, resource, amount } => {
                            json!({"kind": "grant_resources", "faction": faction, "resource": resource, "amount": r2(*amount)})
                        }
                        StoryEffect::GrantShip { faction, class, body } => {
                            json!({"kind": "grant_ship", "faction": faction, "class": class, "body": body})
                        }
                    })
                    .collect::<Vec<_>>();
                json!({"id": s.id, "title": s.title, "trigger": trigger, "effects": effects})
            })
            .collect::<Vec<_>>(),
    })
}

/// The compact, decision-relevant view of a round.
#[derive(Serialize, JsonSchema)]
struct AgentState {
    round: u32,
    time_month: f64,
    factions: Vec<AgentFaction>,
    bodies: Vec<AgentBody>,
    cities: Vec<AgentCity>,
    ships: Vec<AgentShip>,
    /// 本回合事件（谁开火/被毁/城被夷平/殖民/陈旧指令降级），damage 已四舍五入。
    events: Vec<serde_json::Value>,
    /// 合纵连横格局：当前霸权（若有）、各势力综合实力占比、以及针对霸权的反制联盟。
    coalition: serde_json::Value,
}

#[derive(Serialize, JsonSchema)]
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
    /// 首都天体（治理/忠诚度锚点）。
    capital_body: BodyId,
    /// 本土防御半径（AU）：本方城市/舰在此半径内获得本土防御。
    home_radius: f64,
    /// 在本方本土区域内，敌方造成的伤害倍率（<1 = 削弱入侵者）。
    home_attack_mult: f64,
    /// 在本方本土区域内，本方舰只的额外护甲再生（占最大护甲/回合）。
    home_regen_bonus: f64,
}

#[derive(Serialize, JsonSchema)]
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
#[derive(Serialize, JsonSchema)]
struct AgentOrbit {
    perihelion: f64,
    aphelion: f64,
    /// Orbit period in months.
    period: f64,
}

/// One 定居点 (settlement site) on a body: how much can be built, how fast, and
/// what resources are available to mine here (资源矿藏 bounds the mining area).
#[derive(Serialize, JsonSchema)]
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

#[derive(Serialize, JsonSchema)]
struct AgentDeposit {
    resource: String,
    area: f64,
}

#[derive(Serialize, JsonSchema)]
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
    /// 忠诚度 (0..1)：城市对其统治势力的向心力。治理不到位/太远/人口过多时下降，
    /// 跌破叛变阈值即离心叛乱（夷平为空白）。投入娱乐预算可提升。
    loyalty: f64,
    /// 该城到其统治势力首都天体的距离（AU，治理距离）。可读的「治理压力」信号——
    /// AI 一眼看出哪些城在失稳边缘，好据此投娱乐预算 / 决定是否放弃远端殖民地。
    gov_distance: f64,
}

#[derive(Serialize, JsonSchema)]
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

#[derive(Serialize, JsonSchema)]
struct AgentShip {
    id: ShipId,
    name: String,
    class: String,
    owner: FactionId,
    owner_name: String,
    position: [f64; 2],
    hull: f64,
    hull_max: f64,
    /// 当前能量护盾值。
    shield: f64,
    /// 最大能量护盾（无护盾组件为 0）。
    shield_max: f64,
    /// 有效作战面板（class + 组件）：攻击、射程、速度、护甲再生、维护费。
    attack: f64,
    range: f64,
    speed: f64,
    hull_regen: f64,
    upkeep: f64,
    /// 本舰装配的定制组件 id（舰船定制）；空 = 裸舰。
    components: Vec<String>,
    /// 每件组件的完整度（与 `components` 同下标；<1 说明被击中受损，0 = 被击毁不再贡献
    /// 面板/武器）。空 = 裸舰/旧数据（视为全部完好）。
    component_hp: Vec<f64>,
    order: AgentOrder,
}

/// A ship's current command, as a tagged union for clean parsing.
#[derive(Serialize, JsonSchema)]
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
                capital_body: f.capital_body,
                home_radius: r2(f.home_radius),
                home_attack_mult: r2(f.home_attack_mult),
                home_regen_bonus: r2(f.home_regen_bonus),
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
                loyalty: r2(c.loyalty),
                gov_distance: r2(governance_distance(state, c.faction_id, c.body_id)),
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
            .map(|s| {
                let panel = crate::model::ship_panel(config, s);
                AgentShip {
                    id: s.id,
                    name: s.name.clone(),
                    class: s.class.clone(),
                    owner: s.faction_id,
                    owner_name: faction_name(state, s.faction_id),
                    position: [r2(s.position[0]), r2(s.position[1])],
                    hull: r2(s.hull),
                    hull_max: r2(s.hull_max),
                    shield: r2(s.shield),
                    shield_max: r2(s.shield_max),
                    attack: r2(panel.attack),
                    range: r2(panel.attack_range),
                    speed: r2(panel.speed),
                    hull_regen: r2(panel.hull_regen),
                    upkeep: r2(panel.upkeep),
                    components: s.components.clone(),
                    component_hp: s.component_hp.clone(),
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
                }
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
            coalition: {
                let (hegemon, members, powers) = crate::sim::balance_picture(state, config);
                json!({
                    "hegemon": hegemon,
                    "members": members,
                    "power_share": powers.iter().map(|(k, v)| (k.clone(), r2(*v))).collect::<BTreeMap<_, _>>(),
                })
            },
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
        Withdraw { ship, to_body } => {
            json!({"type":"withdraw", "ship": ship, "to_body": to_body})
        }
        WarStarted { a, b } => {
            json!({"type":"war_started", "a": a, "b": b})
        }
        WarEnded { a, b } => {
            json!({"type":"war_ended", "a": a, "b": b})
        }
        Story { id, title, participants } => {
            json!({"type":"story", "id": id, "title": title, "participants": participants})
        }
        Resurgence { faction, body, ship } => {
            json!({"type":"resurgence", "faction": faction, "body": body, "ship": ship})
        }
        Revolt { city, faction } => {
            json!({"type":"revolt", "city": city, "faction": faction})
        }
        CoalitionFormed { hegemon, members } => {
            json!({"type":"coalition_formed", "hegemon": hegemon, "members": members})
        }
        CoalitionEnded { hegemon, members } => {
            json!({"type":"coalition_ended", "hegemon": hegemon, "members": members})
        }
    }
}

fn faction_name(state: &State, id: FactionId) -> String {
    state
        .faction(id)
        .map(|f| f.name.clone())
        .unwrap_or_else(|| format!("#{id}"))
}

/// 一座城（其宿主天体 `body_id`）到其统治势力首都天体的距离（AU）——可读的治理压力
/// 信号：越远，管理越难、忠诚越易跌破叛变阈值。无主/首都缺失时返回 0。
pub fn governance_distance(state: &State, owner: FactionId, body_id: BodyId) -> f64 {
    let Some(capital) = state.faction(owner).map(|f| f.capital_body) else { return 0.0 };
    let bpos = state.body_position(body_id);
    let cpos = state.body_position(capital);
    ((bpos[0] - cpos[0]).powi(2) + (bpos[1] - cpos[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    /// `meta_value` 是让 agent 读的「规则字典」。它必须从 config 结构体**派生**，
    /// 而不是手写字段清单——此守卫确保新增的 config 字段（尤其是 `combat` 那些）
    /// 一定出现在 meta 里，防止再次像 `component_spill` 那样「配置有、meta 无」。
    #[test]
    fn meta_derives_all_combat_fields_and_keeps_ints() {
        let cfg = config::load_config();
        let m = meta_value(&cfg);
        let combat = m.get("combat").and_then(|v| v.as_object()).expect("combat section");
        for f in [
            "component_spill",
            "component_repair",
            "escort_range",
            "pursuit_range",
            "pd_radius",
        ] {
            assert!(combat.contains_key(f), "meta.combat 缺少 {f}（曾被手写清单漏掉）");
        }
        // 整数型配置字段必须保持整数，不能因圆整变成 `2.0`。
        let slots = &m["ships"]["corvette"]["slots"];
        assert!(slots.is_i64() || slots.is_u64(), "slots 应为整数，实为 {slots}");
    }
}
