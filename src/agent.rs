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
                    "slots": s.slots,
                    // 舰级 = 平台修正器：这些系数缩放模块输出（护甲/护盾/护盾再生/速度/加速度/
                    // 伤害/攻击距离）。速度/加速度/攻击距离**没有舰级基础值**——它们完全来自
                    // 推进/武器模块，所以「推进」和「武器」一样是必须的。
                    "armor_mult": r2(s.armor_mult),
                    "shield_mult": r2(s.shield_mult),
                    "shield_regen_mult": r2(s.shield_regen_mult),
                    "speed_mult": r2(s.speed_mult),
                    "accel_mult": r2(s.accel_mult),
                    "attack_mult": r2(s.attack_mult),
                    "range_mult": r2(s.range_mult),
                    "pd_mult": r2(s.pd_mult),
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
        // 舰船定制组件：每件武器的伤害类型/射程/追踪/盾甲倍率，防御的护盾池/装甲/点防，
        // 推进的速度。AI 可据此理解「每支舰队怎么打」以及哪家势力（资源优势）能造什么。
        "components": config
            .components
            .iter()
            .map(|(k, c)| {
                (
                    k.clone(),
                    json!({
                        "label": c.label,
                        "category": c.category,
                        "damage": r2(c.damage),
                        "damage_type": c.damage_type,
                        "range": r2(c.range),
                        "tracking": r2(c.tracking),
                        "shield_mult": r2(c.shield_mult),
                        "hull_mult": r2(c.hull_mult),
                        "shield": r2(c.shield),
                        "shield_regen": r2(c.shield_regen),
                        // 护甲 = 让船体变硬（减伤系数），不是加血。
                        "hardness": r2(c.hardness),
                        "intercept": r2(c.intercept),
                        "speed": r2(c.speed),
                        "hull_regen": r2(c.hull_regen),
                        "upkeep": r2(c.upkeep),
                        "cost": to_cost(&c.cost),
                    }),
                )
            })
            .collect::<BTreeMap<_, _>>(),
        "economy": {
            "production_rate": r2(config.economy.production_rate),
            "pop_growth": r2(config.economy.pop_growth),
            "min_efficiency": r2(config.economy.min_efficiency),
            "invest_fraction": r2(config.economy.invest_fraction),
            "housing_buffer": r2(config.economy.housing_buffer),
            "upkeep_reserve_mult": r2(config.economy.upkeep_reserve_mult),
        },
        "combat": {
            "war_threshold": r2(config.combat.war_threshold),
            "siege_range": r2(config.combat.siege_range),
            "arrival_eps": r2(config.combat.arrival_eps),
            "armor_regen": r2(config.combat.armor_regen),
            "colony_footprint": r2(config.combat.colony_footprint),
            "retreat_hull": r2(config.combat.retreat_hull),
            "retreat_min_dist": r2(config.combat.retreat_min_dist),
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
        // 光速治理：以「距离首都 × 人口超载」为代价的管理/忠诚度。城市远离首都、
        // 或势力人口过多，管理越难；治理不到位（欠费）时忠诚度暴跌，跌破
        // loyalty_revolt 即爆发离心叛乱（城市夷平为空白）。投入娱乐预算
        // (`loyalty_budget`) 可提升忠诚度、对冲距离/人口。
        "governance": {
            "admin_base": r2(config.governance.admin_base),
            "admin_per_au": r2(config.governance.admin_per_au),
            "admin_range": r2(config.governance.admin_range),
            "loyalty_recover": r2(config.governance.loyalty_recover),
            "loyalty_penalty": r2(config.governance.loyalty_penalty),
            "loyalty_distance": r2(config.governance.loyalty_distance),
            "loyalty_range": r2(config.governance.loyalty_range),
            "loyalty_revolt": r2(config.governance.loyalty_revolt),
            "population_capacity": r2(config.governance.population_capacity),
            "default_entertainment": r2(config.governance.default_entertainment),
            "entertainment_cost": r2(config.governance.entertainment_cost),
        },
        // MOND / 柯伊伯引力异常：距太阳超过 radius 的深空进入异常区；除 masters
        // 之外的势力在异常区内轨道计算错误，指令坐标与实际坐标偏移——无法精确轰炸/
        // 殖民/停靠深处目标。掌握了 MOND 的势力（cult）在异常区内指哪打哪。
        "mond": {
            "radius": r2(config.mond.radius),
            "drift_per_au": r2(config.mond.drift_per_au),
            "masters": config.mond.masters.clone(),
        },
        // 合纵连横 / 均势外交：当一方综合实力占比≥hegemon_power 时被定为「霸权」，
        // 其余较弱势力结成反制联盟——弱者相互亲近(向 coalition_affinity 靠拢)，弱者对
        // 霸权疏远/敌意(向 hegemon_affinity 靠拢)；霸权对任一弱者开战触发集体安全
        // (其余弱者对霸权关系骤降)；被封锁的霸权经济制裁(自动市场交易额度缩水到
        // sanction_trade_mult)，造成资源封锁与失衡。min_members 为联盟成立的最小成员数。
        "balance": {
            "hegemon_power": r2(config.balance.hegemon_power),
            "power_city_weight": r2(config.balance.power_city_weight),
            "power_fleet_weight": r2(config.balance.power_fleet_weight),
            "coalition_affinity": r2(config.balance.coalition_affinity),
            "coalition_rate": r2(config.balance.coalition_rate),
            "hegemon_affinity": r2(config.balance.hegemon_affinity),
            "hegemon_rate": r2(config.balance.hegemon_rate),
            "collective_defense_delta": r2(config.balance.collective_defense_delta),
            "min_members": config.balance.min_members,
            "coalition_estrange": r2(config.balance.coalition_estrange),
            "sanction_trade_mult": r2(config.balance.sanction_trade_mult),
            "sanction_cost_mult": r2(config.balance.sanction_cost_mult),
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
    /// 合纵连横格局：当前霸权（若有）、各势力综合实力占比、以及针对霸权的反制联盟。
    coalition: serde_json::Value,
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
    /// 首都天体（治理/忠诚度锚点）。
    capital_body: BodyId,
    /// 本土防御半径（AU）：本方城市/舰在此半径内获得本土防御。
    home_radius: f64,
    /// 在本方本土区域内，敌方造成的伤害倍率（<1 = 削弱入侵者）。
    home_attack_mult: f64,
    /// 在本方本土区域内，本方舰只的额外护甲再生（占最大护甲/回合）。
    home_regen_bonus: f64,
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
    /// 忠诚度 (0..1)：城市对其统治势力的向心力。治理不到位/太远/人口过多时下降，
    /// 跌破叛变阈值即离心叛乱（夷平为空白）。投入娱乐预算可提升。
    loyalty: f64,
    /// 该城到其统治势力首都天体的距离（AU，治理距离）。可读的「治理压力」信号——
    /// AI 一眼看出哪些城在失稳边缘，好据此投娱乐预算 / 决定是否放弃远端殖民地。
    gov_distance: f64,
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
