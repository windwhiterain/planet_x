//! HTTP server for the WebUI: a small JSON API over one in-memory [`State`].
//!
//! The server owns the authoritative world state and the deterministic RNG and
//! exposes a hotseat-style interface:
//!
//! * `GET  /api/meta`      resource/building/ship metadata for the frontend.
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
    pub buildings: BTreeMap<String, BuildingMeta>,
    pub ships: BTreeMap<String, ShipSpec>,
}

#[derive(Serialize, Clone)]
pub struct BuildingMeta {
    pub label: String,
    pub role: String,
    pub default_invest_weight: f64,
}

/// One faction as the frontend needs it. `resources`/`relations` are keyed by
/// strings/ids on the wire (JSON objects), not by the internal map key types.
#[derive(Serialize, Clone)]
pub struct FactionView {
    pub id: FactionId,
    pub name: String,
    pub color: String,
    pub resources: Vec<(String, f64)>,
    pub relations: Vec<(FactionId, f64)>,
    pub budget: Vec<(String, f64)>,
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
    pub kind: String,
    pub resource: Option<String>,
    pub value: f64,
    pub mode: Option<ControlMode>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FactionControlView {
    pub faction_id: FactionId,
    pub ship_orders: Vec<ShipOrderEntry>,
    pub budget: Vec<BudgetEntry>,
    pub invest_weights: Vec<InvestWeightEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ScopeView {
    pub global: Option<ControlMode>,
    pub factions: Vec<(FactionId, Option<ControlMode>)>,
    pub bodies: Vec<(BodyId, Option<ControlMode>)>,
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
}

#[derive(Deserialize)]
pub struct AdvanceReq {
    #[serde(default)]
    pub n: u32,
}

// --- presence-aware control patches (the "diff" the agent writes) ----------
//
// The read/template side uses FactionControlView (full leaves with a concrete
// mode). The *apply* side is a structural multi-level patch: you may target a
// whole faction, a category (ships / budget / invest), or an individual leaf,
// and within a leaf you may set only the value, only the mode, or both.
//
// Presence is tracked so that an *absent* field is left unchanged instead of
// being reset:
//   * `behavior` / `value` absent  -> keep the leaf's current value.
//   * `mode` absent                -> keep the leaf's current mode.
//   * `mode: null`                 -> set mode to inherit (None).
//   * `mode: "Ai"|"Player"`        -> set mode explicitly.
// A leaf that is entirely absent from the diff is untouched (recursion stops
// at the finest granularity present). This is what lets the agent edit just the
// one ship it wants to move.

#[derive(Deserialize, Default)]
pub struct ShipOrderPatch {
    pub ship: ShipId,
    #[serde(default)]
    pub behavior: Option<ShipBehavior>,
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

#[derive(Deserialize, Default)]
pub struct BudgetPatch {
    pub resource: String,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

#[derive(Deserialize, Default)]
pub struct InvestWeightPatch {
    pub city: CityId,
    pub kind: String,
    pub resource: Option<String>,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<Option<ControlMode>>,
}

#[derive(Deserialize, Default)]
pub struct FactionControlPatch {
    pub faction_id: FactionId,
    #[serde(default)]
    pub ship_orders: Vec<ShipOrderPatch>,
    #[serde(default)]
    pub budget: Vec<BudgetPatch>,
    #[serde(default)]
    pub invest_weights: Vec<InvestWeightPatch>,
}

#[derive(Deserialize)]
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

fn faction_view(f: &Faction) -> FactionView {
    FactionView {
        id: f.id,
        name: f.name.clone(),
        color: f.color.clone(),
        resources: f.resources.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        relations: f.relations.iter().map(|(k, v)| (*k, *v)).collect(),
        budget: Vec::new(),
    }
}

fn control_view(fid: FactionId, c: &ControllableState) -> FactionControlView {
    let ship_orders = c
        .ship_orders
        .iter()
        .map(|(sid, ctrl)| ShipOrderEntry { ship: *sid, behavior: ctrl.value, mode: ctrl.mode })
        .collect();
    let budget = c
        .budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry { resource: rt.clone(), value: ctrl.value, mode: ctrl.mode })
        .collect();
    let invest_weights = c
        .invest_weights
        .iter()
        .map(|((cid, kind, res), ctrl)| InvestWeightEntry {
            city: *cid,
            kind: kind.clone(),
            resource: res.clone(),
            value: ctrl.value,
            mode: ctrl.mode,
        })
        .collect();
    FactionControlView { faction_id: fid, ship_orders, budget, invest_weights }
}

fn scope_view(s: &ControlScope) -> ScopeView {
    ScopeView {
        global: s.global,
        factions: s.factions.iter().map(|(k, v)| (*k, *v)).collect(),
        bodies: s.bodies.iter().map(|(k, v)| (*k, *v)).collect(),
        cities: s.cities.iter().map(|(k, v)| (*k, *v)).collect(),
    }
}

pub fn state_view(world: &GameWorld) -> StateView {
    let s = &world.state;
    let mut factions: Vec<FactionView> = s.factions.iter().map(faction_view).collect();
    // Attach the (effective) per-round budget to each faction view for display.
    for f in factions.iter_mut() {
        if let Some(c) = s.control.get(&f.id) {
            f.budget = c
                .budget
                .iter()
                .map(|(rt, ctrl)| (rt.clone(), ctrl.value))
                .collect();
        }
    }
    let control = s
        .control
        .iter()
        .map(|(fid, c)| control_view(*fid, c))
        .collect();
    StateView {
        round: s.round,
        time_month: s.time_month,
        bodies: s.bodies.clone(),
        cities: s.cities.clone(),
        factions,
        ships: s.ships.clone(),
        control,
        scope: scope_view(&s.scope),
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
        ship_orders: v
            .ship_orders
            .into_iter()
            .map(|o| ShipOrderEntry { ship: o.ship, behavior: round_behavior(o.behavior), mode: o.mode })
            .collect(),
        budget: v
            .budget
            .into_iter()
            .map(|b| BudgetEntry { resource: b.resource, value: r2(b.value), mode: b.mode })
            .collect(),
        invest_weights: v
            .invest_weights
            .into_iter()
            .map(|i| InvestWeightEntry { city: i.city, kind: i.kind, resource: i.resource, value: r2(i.value), mode: i.mode })
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
        .map(|(fid, c)| round_view(control_view(*fid, c)))
        .collect();
    let surface = ControlSurface { control, scope: scope_view(&state.scope) };
    serde_json::to_value(surface).expect("control surface is serializable")
}

/// Apply a presence-aware control patch (a structural multi-level diff) to
/// `state`. Only the factions and leaves present in `req` are modified; for a
/// leaf that is present, an omitted `value`/`behavior` keeps the current value
/// and an omitted `mode` keeps the current mode. `scope` is overlaid when given.
pub fn apply_diff(state: &mut State, req: &CommandReq) {
    for fac in &req.control {
        let c = state.control.entry(fac.faction_id).or_default();
        for sp in &fac.ship_orders {
            let ctrl = c.ship_orders.entry(sp.ship).or_insert_with(|| Control {
                value: sp.behavior.unwrap_or(ShipBehavior::Idle),
                mode: sp.mode.flatten(),
            });
            if let Some(v) = sp.behavior {
                ctrl.value = v;
            }
            if let Some(m) = sp.mode {
                ctrl.mode = m;
            }
        }
        for bp in &fac.budget {
            let ctrl = c.budget.entry(bp.resource.clone()).or_insert_with(|| Control {
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
            let key = (ip.city, ip.kind.clone(), ip.resource.clone());
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
    }
    if let Some(sv) = &req.scope {
        state.scope.overlay(&scope_from_view(sv));
    }
}

/// Parse a control diff file (JSON) and apply it to `state` as a structural
/// multi-level patch. Accepts the same shape as `POST /api/command`
/// (`{control:[...],scope:{...}}`).
pub fn apply_patch(state: &mut State, value: &serde_json::Value) -> Result<(), String> {
    let req: CommandReq =
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid control diff: {e}"))?;
    apply_diff(state, &req);
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
                },
            )
        })
        .collect();
    Json(MetaView {
        resources: world.config.resources.clone(),
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
    let mut world = shared.lock().unwrap();
    // Structural multi-level patch (a true diff): only the factions/leaves
    // present in the payload are touched; unlisted ones are left unchanged.
    // The frontend sends the full surface so this is equivalent to a full
    // replace for it, while a partial "command" payload is now safely applied
    // instead of silently dropping the unlisted leaves.
    apply_diff(&mut world.state, &req);
    Json(state_view(&world))
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
