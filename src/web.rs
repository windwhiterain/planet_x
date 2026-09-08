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

// Faction display colors, delivered as CSS hex strings so the frontend never has
// to map ids itself. Keyed by FactionId (modulo the palette length).
const FACTION_COLORS: &[&str] = &[
    "#3b82f6", "#06b6d4", "#8b5cf6", "#ef4444", "#ec4899",
    "#eab308", "#22c55e", "#f8fafc", "#d946ef", "#fb923c",
];
fn faction_color(fid: FactionId) -> String {
    FACTION_COLORS[(fid as usize) % FACTION_COLORS.len()].to_string()
}

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

#[derive(Deserialize)]
pub struct CommandReq {
    /// All factions' controllable state at once (hotseat). Each entry carries
    /// its own `faction_id`.
    #[serde(default)]
    pub control: Vec<FactionControlView>,
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
        color: faction_color(f.id),
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

fn control_from_view(v: &FactionControlView) -> ControllableState {
    let mut c = ControllableState::default();
    for e in &v.ship_orders {
        c.ship_orders.insert(e.ship, Control { value: e.behavior, mode: e.mode });
    }
    for e in &v.budget {
        c.budget.insert(e.resource.clone(), Control { value: e.value, mode: e.mode });
    }
    for e in &v.invest_weights {
        let key = (e.city, e.kind.clone(), e.resource.clone());
        c.invest_weights.insert(key, Control { value: e.value, mode: e.mode });
    }
    c
}

fn scope_from_view(v: &ScopeView) -> ControlScope {
    ControlScope {
        global: v.global,
        factions: v.factions.iter().cloned().collect(),
        bodies: v.bodies.iter().cloned().collect(),
        cities: v.cities.iter().cloned().collect(),
    }
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
    for cv in &req.control {
        world.state.control.insert(cv.faction_id, control_from_view(cv));
    }
    if let Some(sv) = &req.scope {
        world.state.scope = scope_from_view(sv);
    }
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
