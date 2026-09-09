//! HTTP server for the WebUI: a small JSON API over one in-memory [`State`].
//!
//! The server owns the authoritative world state and the deterministic RNG and
//! exposes a hotseat-style interface:
//!
//! * `GET  /api/meta`      resource/building/structure/ship metadata.
//! * `GET  /api/state`     the current world (bodies, cities, factions, ships,
//!                         per-faction controllable state, control scope).
//! * `POST /api/advance`   run `n` rounds, return the new world.
//! * `POST /api/command`   write a faction's controllable state + the scope.
//! * `POST /api/new`       rebuild the world from a seed.
//!
//! Static files (the frontend) are served from `static/`.

use crate::config::parse_seed;
use crate::model::*;
use crate::prng::Prng;
use crate::sim;
use crate::world;
use axum::extract::State as AxState;
use axum::routing::{get, post};
use axum::{Json, Router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tower_http::services::ServeDir;

/// The in-memory world owned by the server.
pub struct GameWorld {
    pub state: State,
    pub config: GameConfig,
    pub rng: Prng,
}

pub type Shared = Arc<Mutex<GameWorld>>;

// --- wire types -------------------------------------------------------------

#[derive(Serialize)]
pub struct MetaView {
    pub resources: BTreeMap<String, ResourceDef>,
    pub structures: BTreeMap<String, StructureSpec>,
    pub buildings: BTreeMap<String, BuildingMeta>,
    pub ships: BTreeMap<String, ShipSpec>,
}

#[derive(Serialize, Clone)]
pub struct BuildingMeta {
    pub label: String,
    pub role: String,
    pub default_invest_weight: f64,
    pub default_build_weight: f64,
}

/// One faction as the frontend needs it.
#[derive(Serialize, Clone)]
pub struct FactionView {
    pub id: FactionId,
    pub name: String,
    pub color: String,
    pub resources: Vec<(String, f64)>,
    pub relations: Vec<(FactionId, f64)>,
    pub investment_budget: Vec<(String, f64)>,
    pub construction_budget: Vec<(String, f64)>,
    /// 有效首都天体（迁都唯一事实来源 [`State::capital_body`] 解析）。
    pub capital_body: BodyId,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ShipOrderEntry {
    pub ship: ShipId,
    pub behavior: ShipBehavior,
    pub mode: Option<ControlMode>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BudgetEntry {
    pub resource: String,
    pub value: f64,
    pub mode: Option<ControlMode>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InvestWeightEntry {
    pub city: CityId,
    pub building: BuildingId,
    pub kind: String,
    pub resource: Option<String>,
    pub ship_type: Option<String>,
    pub structure: String,
    pub value: f64,
    pub mode: Option<ControlMode>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BuildWeightEntry {
    pub city: CityId,
    pub building: BuildingId,
    pub ship_type: Option<String>,
    pub value: f64,
    pub mode: Option<ControlMode>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LoyaltyBudgetEntry {
    pub city: CityId,
    pub value: f64,
    pub mode: Option<ControlMode>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FactionControlView {
    pub faction_id: FactionId,
    pub capital: Option<Control<BodyId>>,
    pub ship_orders: Vec<ShipOrderEntry>,
    pub investment_budget: Vec<BudgetEntry>,
    pub construction_budget: Vec<BudgetEntry>,
    pub invest_weights: Vec<InvestWeightEntry>,
    pub build_weights: Vec<BuildWeightEntry>,
    pub loyalty_budget: Vec<LoyaltyBudgetEntry>,
}

#[derive(Serialize, Deserialize, Clone, JsonSchema)]
pub struct ScopeView {
    #[serde(default)]
    pub global: Option<ControlMode>,
    #[serde(default)]
    pub factions: Vec<(FactionId, Option<ControlMode>)>,
    #[serde(default)]
    pub bodies: Vec<(BodyId, Option<ControlMode>)>,
    #[serde(default)]
    pub cities: Vec<(CityId, Option<ControlMode>)>,
}

/// The editable control surface, exactly what the frontend edits and posts back
/// to `/api/command`. Emitted by the agent CLI `control` command as the
/// "template" the agent edits, and accepted by `apply` / `--apply` as a diff.
#[derive(Serialize, Clone)]
pub struct ControlSurface {
    pub control: Vec<FactionControlView>,
    pub scope: ScopeView,
}

#[derive(Serialize, Clone)]
pub struct StateView {
    pub round: u32,
    pub time_month: f64,
    pub bodies: Vec<Body>,
    pub cities: Vec<City>,
    pub factions: Vec<FactionView>,
    pub ships: Vec<Ship>,
    pub control: Vec<FactionControlView>,
    pub scope: ScopeView,
    /// 本回合事件流水（who attacked / ships lost / razed cities / wars / story beats …）。
    #[serde(default)]
    pub events: Vec<GameEvent>,
    /// 剧情编年史：整段已展开的叙事弧。
    #[serde(default)]
    pub chronicle: Vec<ChronicleEntry>,
}

#[derive(Deserialize)]
pub struct AdvanceReq {
    #[serde(default)]
    pub n: u32,
}

// --- presence-aware control patches (the "diff" the agent writes) ----------

/// 一艘舰的指令补丁：`behavior` 用它替换该舰行为；`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipOrderPatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    /// 新行为（Idle/Move/TargetShip/TargetSettlement/Dock/Colonize）。缺省 = 保留现值。
    #[serde(default)]
    pub behavior: Option<ShipBehavior>,
    /// 由谁决定：Ai（系统）/Player（玩家）/ 显式 None（继承上层）。缺省 = 保留现值。
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

/// 资源预算补丁（投资/建造共用）：`value` 替换预算额，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct BudgetPatch {
    /// 资源 raw-key（见 --meta 的 resources：raw-key→中文名）。
    pub resource: String,
    /// 新的预算额（资源投放量）。缺省 = 保留现值。
    #[serde(default)]
    pub value: Option<f64>,
    /// 由谁决定：Ai（系统）/Player（玩家）/ 显式 None（继承上层）。缺省 = 保留现值。
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

/// 某城某「建设投资权重」补丁：`value` 替换权重，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct InvestWeightPatch {
    pub city: CityId,
    pub building: BuildingId,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

/// 某城某建造区「建造投资权重」补丁：`value` 替换权重，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct BuildWeightPatch {
    pub city: CityId,
    pub building: BuildingId,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

/// 某城「娱乐/福利预算」补丁：`value` 替换预算额，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct LoyaltyBudgetPatch {
    pub city: CityId,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

/// A structural building patch: add a new building, remove an existing one, or
/// change an existing building's attributes (structure / ship_type / kind).
#[derive(Deserialize, Default, JsonSchema)]
pub struct BuildingPatch {
    /// Which city to add to / remove from.
    #[serde(default)]
    pub city: Option<CityId>,
    /// Some(id) = target an existing building; None = add a new one.
    #[serde(default)]
    pub building: Option<BuildingId>,
    /// For a new building: kind key (residential | mining | construction).
    #[serde(default)]
    pub kind: Option<String>,
    /// For a new (mining) building: the mined resource key.
    #[serde(default)]
    pub resource: Option<String>,
    /// For a new (建造区) building: the ship class it produces.
    #[serde(default)]
    pub ship_type: Option<String>,
    /// For a new or modified building: structure key (concrete | steel).
    #[serde(default)]
    pub structure: Option<String>,
    /// For a new building: planned area.
    #[serde(default)]
    pub area: Option<f64>,
    /// Remove the referenced building.
    #[serde(default)]
    pub remove: bool,
}

/// 迁都（首都天体）补丁：`value` 指定新的首都天体（BodyId = 天体唯一名）；
/// `mode` 指定由谁决定（Ai/Player/显式 None）。缺省 `value` 保留现值、缺省 `mode`
/// 保留现模式。迁都的唯一事实来源是 [`ControllableState::capital`]（无 shadow 双状态）。
#[derive(Deserialize, Default, JsonSchema)]
pub struct CapitalPatch {
    /// 新的首都天体（天体唯一名）。缺省 = 保留现值。
    #[serde(default)]
    pub value: Option<BodyId>,
    /// 由谁决定：Ai（系统周期性迁移）/Player（玩家，系统不改写，除非首都亡城强迁）/
    /// 显式 None（沿作用域链上溯）。缺省 = 保留现模式。
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

/// 单个势力的可控状态补丁（`--apply` / `POST /api/command` 的 `control[]` 元素）。
/// 只改动**出现在这里**的叶片；缺省的 `Vec` 字段/`Option` 叶子一律保持不变。
#[derive(Deserialize, Default, JsonSchema)]
pub struct FactionControlPatch {
    pub faction_id: FactionId,
    /// 迁都（首都天体）补丁。
    #[serde(default)]
    pub capital: Option<CapitalPatch>,
    /// 本势力各舰的指令补丁。
    #[serde(default)]
    pub ship_orders: Vec<ShipOrderPatch>,
    /// 投资预算补丁（建设）。
    #[serde(default)]
    pub investment_budget: Vec<BudgetPatch>,
    /// 建造预算补丁（造舰）。
    #[serde(default)]
    pub construction_budget: Vec<BudgetPatch>,
    /// 建设投资权重补丁。
    #[serde(default)]
    pub invest_weights: Vec<InvestWeightPatch>,
    /// 建造投资权重补丁。
    #[serde(default)]
    pub build_weights: Vec<BuildWeightPatch>,
    /// 娱乐/福利预算补丁。
    #[serde(default)]
    pub loyalty_budget: Vec<LoyaltyBudgetPatch>,
    /// 结构性建筑补丁（新增/删除/改属性）。
    #[serde(default)]
    pub buildings: Vec<BuildingPatch>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CommandReq {
    /// Factions' controllable-state patches. Only the factions/leaves that are
    /// present are touched; everything else is left as-is.
    #[serde(default)]
    pub control: Vec<FactionControlPatch>,
    /// Optional scope (AI/玩家 boundary tree) overlay.
    #[serde(default)]
    pub scope: Option<ScopeView>,
}

#[derive(Deserialize)]
pub struct NewReq {
    #[serde(default = "default_random_seed")]
    pub seed: String,
}

fn default_random_seed() -> String {
    "random".to_string()
}

// --- conversions ------------------------------------------------------------

fn faction_view(state: &State, f: &Faction) -> FactionView {
    FactionView {
        // Faction identity is its unique name; `id` carries that name now.
        id: f.name.clone(),
        name: f.name.clone(),
        color: f.color.clone(),
        resources: f.resources.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        relations: f.relations.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        investment_budget: Vec::new(),
        construction_budget: Vec::new(),
        capital_body: state.capital_body(&f.name),
    }
}

fn control_view(state: &State, fid: FactionId, c: &ControllableState) -> FactionControlView {
    let ship_orders = c
        .ship_orders
        .iter()
        .map(|(sid, ctrl)| ShipOrderEntry { ship: sid.clone(), behavior: ctrl.value.clone(), mode: ctrl.mode })
        .collect();
    let investment_budget = c
        .investment_budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry { resource: rt.clone(), value: ctrl.value, mode: ctrl.mode })
        .collect();
    let construction_budget = c
        .construction_budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry { resource: rt.clone(), value: ctrl.value, mode: ctrl.mode })
        .collect();
    let invest_weights = c
        .invest_weights
        .iter()
        .map(|((cid, bid), ctrl)| {
            let b = state.city(cid).and_then(|cty| cty.buildings.iter().find(|b| b.id == *bid));
            InvestWeightEntry {
                city: cid.clone(),
                building: *bid,
                kind: b.map(|x| x.kind.clone()).unwrap_or_default(),
                resource: b.and_then(|x| x.resource.clone()),
                ship_type: b.and_then(|x| x.ship_type.clone()),
                structure: b.map(|x| x.structure.clone()).unwrap_or_default(),
                value: ctrl.value,
                mode: ctrl.mode,
            }
        })
        .collect();
    let build_weights = c
        .build_weights
        .iter()
        .map(|((cid, bid), ctrl)| {
            let b = state.city(cid).and_then(|cty| cty.buildings.iter().find(|b| b.id == *bid));
            BuildWeightEntry {
                city: cid.clone(),
                building: *bid,
                ship_type: b.and_then(|x| x.ship_type.clone()),
                value: ctrl.value,
                mode: ctrl.mode,
            }
        })
        .collect();
    let loyalty_budget = c
        .loyalty_budget
        .iter()
        .map(|(cid, ctrl)| LoyaltyBudgetEntry { city: cid.clone(), value: ctrl.value, mode: ctrl.mode })
        .collect();
    FactionControlView {
        faction_id: fid,
        capital: c.capital.clone(),
        ship_orders,
        investment_budget,
        construction_budget,
        invest_weights,
        build_weights,
        loyalty_budget,
    }
}

fn scope_view(s: &ControlScope) -> ScopeView {
    ScopeView {
        global: s.global,
        factions: s.factions.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        bodies: s.bodies.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        cities: s.cities.iter().map(|(k, v)| (k.clone(), *v)).collect(),
    }
}

pub fn state_view(world: &GameWorld) -> StateView {
    let s = &world.state;
    let mut factions: Vec<FactionView> = s.factions.iter().map(|f| faction_view(s, f)).collect();
    // Attach the (effective) budgets to each faction view for display.
    for f in factions.iter_mut() {
        if let Some(c) = s.control.get(&f.id) {
            f.investment_budget = c.investment_budget.iter().map(|(rt, ctrl)| (rt.clone(), ctrl.value)).collect();
            f.construction_budget = c.construction_budget.iter().map(|(rt, ctrl)| (rt.clone(), ctrl.value)).collect();
        }
    }
    let control = s.control.iter().map(|(fid, c)| control_view(s, fid.clone(), c)).collect();
    StateView {
        round: s.round,
        time_month: s.time_month,
        bodies: s.bodies.clone(),
        cities: s.cities.clone(),
        factions,
        ships: s.ships.clone(),
        control,
        scope: scope_view(&s.scope),
        events: s.events.clone(),
        chronicle: s.chronicle.clone(),
    }
}

fn scope_from_view(v: &ScopeView) -> ControlScope {
    ControlScope {
        global: v.global,
        factions: v.factions.iter().cloned().collect(),
        bodies: v.bodies.iter().cloned().collect(),
        cities: v.cities.iter().cloned().collect(),
    }
}

/// Round to 2 decimals (token-noise reduction, matching the agent output).
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round_behavior(b: ShipBehavior) -> ShipBehavior {
    match b {
        ShipBehavior::Move { position } => ShipBehavior::Move {
            position: [r2(position[0]), r2(position[1])],
        },
        other => other,
    }
}

/// Round every numeric field of a control view so the agent template has no
/// float noise. Only used by the agent `control` command; the web `state_view`
/// keeps raw values.
fn round_view(v: FactionControlView) -> FactionControlView {
    FactionControlView {
        faction_id: v.faction_id,
        capital: v.capital,
        ship_orders: v
            .ship_orders
            .into_iter()
            .map(|o| ShipOrderEntry { ship: o.ship, behavior: round_behavior(o.behavior), mode: o.mode })
            .collect(),
        investment_budget: v
            .investment_budget
            .into_iter()
            .map(|b| BudgetEntry { resource: b.resource, value: r2(b.value), mode: b.mode })
            .collect(),
        construction_budget: v
            .construction_budget
            .into_iter()
            .map(|b| BudgetEntry { resource: b.resource, value: r2(b.value), mode: b.mode })
            .collect(),
        invest_weights: v
            .invest_weights
            .into_iter()
            .map(|i| InvestWeightEntry { value: r2(i.value), ..i })
            .collect(),
        build_weights: v
            .build_weights
            .into_iter()
            .map(|i| BuildWeightEntry { value: r2(i.value), ..i })
            .collect(),
        loyalty_budget: v
            .loyalty_budget
            .into_iter()
            .map(|l| LoyaltyBudgetEntry { value: r2(l.value), ..l })
            .collect(),
    }
}

/// Render the current editable control surface (control + scope) as JSON —
/// the template an agent edits and posts back as a diff. Values are rounded to
/// 2 decimals so the template is clean for an LLM.
pub fn control_surface(state: &State) -> serde_json::Value {
    let control = state
        .control
        .iter()
        .map(|(fid, c)| round_view(control_view(state, fid.clone(), c)))
        .collect();
    let surface = ControlSurface { control, scope: scope_view(&state.scope) };
    serde_json::to_value(surface).expect("control surface is serializable")
}

/// The machine-readable JSON Schema for the **control/`--apply` diff**
/// ([`CommandReq`]). Handed to the agent so it can write a steering diff without
/// memorising the contract. Auto-derived from the same structs the diff is
/// deserialised into, so it can never drift from `apply_patch`'s shape.
pub fn control_schema_value() -> serde_json::Value {
    let schema = schemars::schema_for!(CommandReq);
    serde_json::to_value(schema).expect("control schema is serializable")
}

/// Apply a presence-aware control patch (a structural multi-level diff) to
/// `state`. Only the factions and leaves present in `req` are modified; for a
/// leaf that is present, an omitted `value`/`behavior` keeps the current value
/// and an omitted `mode` keeps the current mode. `scope` is overlaid when given.
pub fn apply_diff(state: &mut State, config: &GameConfig, req: &CommandReq) {
    for fac in &req.control {
        let c = state.control.entry(fac.faction_id.clone()).or_default();
        for sp in &fac.ship_orders {
            // Resolve the ship by its **name** (the unique key): an order only
            // applies to a ship that exists and that this faction actually owns,
            // so ordering another faction's ship (or a vanished one) is a no-op.
            let Some(ship_name) = state
                .ships
                .iter()
                .find(|s| s.name == sp.ship && s.faction_id == fac.faction_id)
                .map(|s| s.name.clone())
            else {
                continue;
            };
            let ctrl = c.ship_orders.entry(ship_name).or_insert_with(|| Control {
                value: sp.behavior.clone().unwrap_or(ShipBehavior::Idle),
                mode: sp.mode.flatten(),
            });
            if let Some(v) = &sp.behavior {
                ctrl.value = v.clone();
            }
            if let Some(m) = sp.mode {
                ctrl.mode = m;
            }
        }
        for bp in &fac.investment_budget {
            let ctrl = c.investment_budget.entry(bp.resource.clone()).or_insert_with(|| Control {
                value: bp.value.unwrap_or(0.0),
                mode: bp.mode.flatten(),
            });
            if let Some(v) = bp.value {
                ctrl.value = v;
            }
            if let Some(m) = bp.mode {
                ctrl.mode = m;
            }
        }
        for bp in &fac.construction_budget {
            let ctrl = c.construction_budget.entry(bp.resource.clone()).or_insert_with(|| Control {
                value: bp.value.unwrap_or(0.0),
                mode: bp.mode.flatten(),
            });
            if let Some(v) = bp.value {
                ctrl.value = v;
            }
            if let Some(m) = bp.mode {
                ctrl.mode = m;
            }
        }
        for ip in &fac.invest_weights {
            let key = (ip.city.clone(), ip.building);
            let ctrl = c.invest_weights.entry(key).or_insert_with(|| Control {
                value: ip.value.unwrap_or(0.0),
                mode: ip.mode.flatten(),
            });
            if let Some(v) = ip.value {
                ctrl.value = v;
            }
            if let Some(m) = ip.mode {
                ctrl.mode = m;
            }
        }
        for bp in &fac.build_weights {
            let key = (bp.city.clone(), bp.building);
            let ctrl = c.build_weights.entry(key).or_insert_with(|| Control {
                value: bp.value.unwrap_or(0.0),
                mode: bp.mode.flatten(),
            });
            if let Some(v) = bp.value {
                ctrl.value = v;
            }
            if let Some(m) = bp.mode {
                ctrl.mode = m;
            }
        }
        for lp in &fac.loyalty_budget {
            let ctrl = c.loyalty_budget.entry(lp.city.clone()).or_insert_with(|| Control {
                value: lp.value.unwrap_or(0.0),
                mode: lp.mode.flatten(),
            });
            if let Some(v) = lp.value {
                ctrl.value = v;
            }
            if let Some(m) = lp.mode {
                ctrl.mode = m;
            }
        }
        for bpatch in &fac.buildings {
            apply_building_patch(state, config, fac.faction_id.clone(), bpatch);
        }
    }
    // 迁都：写「有效首都」的唯一事实来源（`ControllableState::capital`）。独立的循环
    // 以拿到干净的借用：先做只读（当前首都、天体存在性），再在独立作用域里写控制。
    // 只允许迁到真实存在的天体；若指向的天体上并无本势力活城，则由 sim 的亡城强迁
    // 规则兜底，避免把首都钉在虚天体上。`mode` 沿作用域链与其它叶子一致。
    for fac in &req.control {
        if let Some(cap) = &fac.capital {
            let cur = state.capital_body(&fac.faction_id);
            let new_value = cap.value.as_ref().filter(|v| state.body(v).is_some()).cloned();
            {
                let ctrl = state.control.entry(fac.faction_id.clone()).or_default();
                let ctrl = ctrl.capital.get_or_insert_with(|| Control::inherit(cur));
                if let Some(v) = new_value {
                    ctrl.value = v;
                }
                if let Some(m) = cap.mode {
                    ctrl.mode = m;
                }
            }
        }
    }
    if let Some(sv) = &req.scope {
        state.scope.overlay(&scope_from_view(sv));
    }
}

/// Apply a single structural building patch: add / remove / modify a building.
fn apply_building_patch(state: &mut State, config: &GameConfig, fid: FactionId, patch: &BuildingPatch) {
    let Some(cid) = patch.city.clone() else { return };

    if patch.building.is_none() {
        // Add a new building.
        let kind = patch.kind.clone().unwrap_or_else(|| "residential".to_string());
        let Some(spec) = config.buildings.get(&kind) else { return };
        // City identity = its unique name; the new building only lands in a city
        // owned by this faction.
        if state.city(&cid).map(|c| c.faction_id.as_str()) != Some(fid.as_str()) {
            return;
        }
        let structure = patch.structure.clone().unwrap_or_else(|| "concrete".to_string());
        if !config.structures.contains_key(&structure) {
            return;
        }
        let area = patch.area.unwrap_or(4.0).max(0.0);
        let id = state
            .cities
            .iter()
            .flat_map(|c| c.buildings.iter().map(|b| b.id))
            .max()
            .map_or(0, |m| m + 1);
        let resource = if kind == "mining" { patch.resource.clone() } else { None };
        let ship_type = if kind == "construction" { patch.ship_type.clone().or_else(|| Some("corvette".to_string())) } else { None };
        let b = Building {
            id,
            kind: kind.clone(),
            resource,
            ship_type,
            structure: structure.clone(),
            area,
            deployed: 0.0,
            armor: 0.0,
        };
        if let Some(city) = state.city_mut(&cid) {
            city.buildings.push(b);
        }
        let ctrl = state.control.entry(fid.clone()).or_default();
        ctrl.invest_weights.insert((cid.clone(), id), Control::player(spec.default_invest_weight));
        if kind == "construction" {
            ctrl.build_weights.insert((cid.clone(), id), Control::player(spec.default_build_weight));
        }
        return;
    }

    let bid = patch.building.unwrap_or(u32::MAX);
    if patch.remove {
        if let Some(city) = state.city_mut(&cid) {
            city.buildings.retain(|b| b.id != bid);
        }
        if let Some(c) = state.control_mut(fid.clone()) {
            c.invest_weights.remove(&(cid.clone(), bid));
            c.build_weights.remove(&(cid.clone(), bid));
        }
        return;
    }

    // Modify an existing building's attributes (e.g. structure / ship_type).
    if let Some(city) = state.city_mut(&cid) {
        if let Some(b) = city.buildings.iter_mut().find(|b| b.id == bid) {
            if let Some(s) = &patch.structure {
                if config.structures.contains_key(s) {
                    b.structure = s.clone();
                }
            }
            if let Some(s) = &patch.ship_type {
                if b.is_shipyard() {
                    b.ship_type = Some(s.clone());
                }
            }
            if let Some(k) = &patch.kind {
                if config.buildings.contains_key(k) {
                    b.kind = k.clone();
                }
            }
            if let Some(r) = &patch.resource {
                if b.kind == "mining" {
                    b.resource = Some(r.clone());
                }
            }
            if let Some(a) = patch.area {
                b.area = a.max(0.0);
            }
        }
    }
}

/// Normalize a single ship `behavior` value so `apply` accepts BOTH shapes:
///   * the default serde enum form (`{"TargetShip":{"ship":"华盛顿","attack":true}}`,
///     `"Idle"`) — what the `control` template emits and what `.ron` uses; and
///   * the tagged agent-state form (`{"type":"target_ship","ship":"华盛顿","attack":true}`,
///     `{"type":"idle"}`) — exactly what an agent sees in a ship's `order`.
/// The latter is rewritten into the former so the rest of the pipeline stays
/// unchanged. Unknown tags are left as-is (they'll fail downstream cleanly).
fn normalize_behavior(v: &mut serde_json::Value) {
    let Some(ty) = v.get("type").and_then(|t| t.as_str()).map(str::to_string) else {
        return; // already the default form (object or "Idle")
    };
    let obj = v.as_object().expect("behavior with type is an object");
    let mut inner = serde_json::Map::new();
    for (k, val) in obj {
        if k != "type" {
            inner.insert(k.clone(), val.clone());
        }
    }
    let variant = match ty.as_str() {
        "idle" => {
            *v = serde_json::Value::String("Idle".to_string());
            return;
        }
        "move" => "Move",
        "target_ship" => "TargetShip",
        "target_settlement" => "TargetSettlement",
        "dock" => "Dock",
        "colonize" => "Colonize",
        _ => return,
    };
    let mut m = serde_json::Map::new();
    m.insert(variant.to_string(), serde_json::Value::Object(inner));
    *v = serde_json::Value::Object(m);
}

/// Walk a control diff and normalize every `ship_orders[].behavior` (see
/// [`normalize_behavior`]). Only the apply-side JSON path; the state's `order`
/// view is untouched.
fn normalize_control_diffs(value: &mut serde_json::Value) {
    let Some(control) = value.get_mut("control").and_then(|c| c.as_array_mut()) else { return };
    for fac in control.iter_mut() {
        let Some(orders) = fac.get_mut("ship_orders").and_then(|o| o.as_array_mut()) else { continue };
        for order in orders.iter_mut() {
            if let Some(behavior) = order.get_mut("behavior") {
                normalize_behavior(behavior);
            }
        }
    }
}

/// Parse a control diff file (JSON) and apply it to `state` as a structural
/// multi-level patch. Accepts the same shape as `POST /api/command`
/// (`{control:[...],scope:{...}}`). Ship behaviors may be written in either the
/// default enum form or the tagged agent-state form (see [`normalize_behavior`]).
pub fn apply_patch(state: &mut State, config: &GameConfig, value: &serde_json::Value) -> Result<(), String> {
    let mut v = value.clone();
    normalize_control_diffs(&mut v);
    let req: CommandReq =
        serde_json::from_value(v).map_err(|e| format!("invalid control diff: {e}"))?;
    apply_diff(state, config, &req);
    Ok(())
}

// --- handlers ---------------------------------------------------------------

async fn get_meta(AxState(shared): AxState<Shared>) -> Json<MetaView> {
    let world = shared.lock().unwrap();
    let buildings = world
        .config
        .buildings
        .iter()
        .map(|(k, spec)| {
            (
                k.clone(),
                BuildingMeta {
                    label: spec.label.clone(),
                    role: spec.role.clone(),
                    default_invest_weight: spec.default_invest_weight,
                    default_build_weight: spec.default_build_weight,
                },
            )
        })
        .collect();
    Json(MetaView {
        resources: world.config.resources.clone(),
        structures: world.config.structures.clone(),
        buildings,
        ships: world.config.ships.clone(),
    })
}

async fn get_state(AxState(shared): AxState<Shared>) -> Json<StateView> {
    let world = shared.lock().unwrap();
    Json(state_view(&world))
}

async fn advance(AxState(shared): AxState<Shared>, Json(req): Json<AdvanceReq>) -> Json<StateView> {
    let mut guard = shared.lock().unwrap();
    let world = &mut *guard;
    for _ in 0..req.n {
        sim::advance(&mut world.state, &world.config, &mut world.rng);
    }
    Json(state_view(world))
}

async fn command(AxState(shared): AxState<Shared>, Json(req): Json<CommandReq>) -> Json<StateView> {
    let mut guard = shared.lock().unwrap();
    let world = &mut *guard;
    apply_diff(&mut world.state, &world.config, &req);
    Json(state_view(world))
}

async fn new_game(AxState(shared): AxState<Shared>, Json(req): Json<NewReq>) -> Json<StateView> {
    let mut world = shared.lock().unwrap();
    let seed = parse_seed(&req.seed);
    world.state = world::default_state(&world.config, seed);
    world.rng = Prng::new(seed);
    Json(state_view(&world))
}

/// Build the axum router serving the JSON API and the static frontend.
pub fn router(shared: Shared) -> Router {
    Router::new()
        .route("/api/meta", get(get_meta))
        .route("/api/state", get(get_state))
        .route("/api/advance", post(advance))
        .route("/api/command", post(command))
        .route("/api/new", post(new_game))
        .with_state(shared)
        .fallback_service(ServeDir::new("static"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ControlMode, ShipBehavior};

    /// The tagged agent-state `order` form must be accepted and rewritten into
    /// the default enum form that the rest of the pipeline expects.
    #[test]
    fn normalize_behavior_accepts_tagged_form() {
        let cases = [
            (serde_json::json!({"type":"idle"}), serde_json::json!("Idle")),
            (
                serde_json::json!({"type":"target_ship","ship":"华盛顿","attack":true}),
                serde_json::json!({"TargetShip":{"ship":"华盛顿","attack":true}}),
            ),
            (
                serde_json::json!({"type":"target_settlement","city":"长三角","bombard":true}),
                serde_json::json!({"TargetSettlement":{"city":"长三角","bombard":true}}),
            ),
            (
                serde_json::json!({"type":"move","position":[-0.5,0.3]}),
                serde_json::json!({"Move":{"position":[-0.5,0.3]}}),
            ),
            (
                serde_json::json!({"type":"colonize","body":"地球"}),
                serde_json::json!({"Colonize":{"body":"地球"}}),
            ),
        ];
        for (tagged, expected) in cases {
            let mut v = tagged.clone();
            normalize_behavior(&mut v);
            assert_eq!(v, expected, "tagged input {tagged:?} must normalize to {expected:?}");
        }
    }

    /// The default form must pass through unchanged.
    #[test]
    fn normalize_behavior_keeps_default_form() {
        let mut v = serde_json::json!({"TargetShip":{"ship":"华盛顿","attack":true}});
        normalize_behavior(&mut v);
        assert_eq!(v, serde_json::json!({"TargetShip":{"ship":"华盛顿","attack":true}}));
    }

    /// Applying a tagged-form diff to a real world must produce the same
    /// controllable behavior as the equivalent default-form diff.
    #[test]
    fn apply_patch_accepts_tagged_ship_order() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        // 长城 = 中国 (faction 3) 的起始护卫舰；华盛顿 = 美国的一艘舰。舰名即唯一 key。
        let tagged = serde_json::json!({
            "control": [{
                "faction_id": "中国",
                "ship_orders": [{"ship": "长城", "behavior": {"type": "target_ship", "ship": "华盛顿", "attack": true}, "mode": "Player"}]
            }]
        });
        apply_patch(&mut state, &config, &tagged).expect("tagged diff applies");
        let b = state.ship_behavior("长城".to_string()).expect("长城 has an order");
        assert_eq!(b, ShipBehavior::TargetShip { ship: "华盛顿".to_string(), attack: true });
    }

    /// Setting a faction's scope to Player must actually take over its leaves
    /// (which are inert `inherit` now), even though the AI has "written" values.
    #[test]
    fn scope_player_takes_over_independent_faction() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        // Default scope (all None) → everything resolves to Ai.
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Ai, "default scope is Ai");

        // Take over faction 中国 via a scope-only diff.
        let scope_diff = serde_json::json!({"scope": {"factions": [["中国", "Player"]]}});
        apply_patch(&mut state, &config, &scope_diff).expect("scope diff applies");
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Player, "ship of a Player faction is player-owned");
        assert_eq!(state.investment_budget_control("中国".to_string(), "铁"), ControlMode::Player, "budget leaf follows scope");
        assert_eq!(state.construction_budget_control("中国".to_string(), "铁"), ControlMode::Player);
        // Other factions are untouched (still Ai): 华盛顿 is a US ship (美国).
        assert_eq!(state.ship_control("华盛顿".to_string()), ControlMode::Ai, "untouched faction stays Ai");

        // An explicit leaf mode still overrides scope in the opposite direction:
        // force 长城 back to Ai inside a Player faction.
        let leaf_diff = serde_json::json!({
            "control": [{"faction_id": "中国", "ship_orders": [{"ship": "长城", "mode": "Ai"}]}]
        });
        apply_patch(&mut state, &config, &leaf_diff).expect("leaf diff applies");
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Ai, "explicit Ai leaf beats Player scope");
    }

    /// 娱乐/福利预算：一座城的忠诚度投入是一个可控叶子。按 Player 覆盖后，治理模型
    /// 会读取它；省略 value 时保留当前值、省略 mode 时保留当前模式。
    #[test]
    fn apply_loyalty_budget_patch() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "loyalty_budget": [{"city": "长三角", "value": 40.0, "mode": "Player"}]}]
        });
        apply_patch(&mut state, &config, &diff).expect("loyalty budget diff applies");
        assert_eq!(state.loyalty_budget_control("中国".to_string(), "长三角".to_string()), ControlMode::Player);
        let v = state
            .control("中国".to_string())
            .and_then(|c| c.loyalty_budget.get("长三角"))
            .map(|c| c.value)
            .unwrap_or(f64::NAN);
        assert!((v - 40.0).abs() < 1e-7, "loyalty budget value should be 40.0, got {v}");
    }
}
