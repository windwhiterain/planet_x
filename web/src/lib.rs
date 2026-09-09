//! `planet_x_web` — the WebUI server for Planet X, decoupled from the engine.
//!
//! This crate is the player-facing interactive front end. It owns one
//! authoritative in-memory [`planet_x::model::State`] together with the
//! deterministic RNG, and exposes a hotseat-style JSON API over the engine
//! (`planet_x` crate) as a library:
//!
//! * `GET  /api/meta`      resource/building/structure/ship metadata.
//! * `GET  /api/state`     the current world (bodies, cities, factions, ships,
//!                         per-faction controllable state, control scope).
//! * `POST /api/advance`   run `n` rounds, return the new world.
//! * `POST /api/command`   write a faction's controllable state + the scope.
//! * `POST /api/new`       rebuild the world from a seed.
//!
//! Static files (the frontend) are served from `web/static/`. Everything that
//! is *not* HTTP — the control-diff domain (`apply_patch` / `control_surface` /
//! `control_schema_value` / patch & view types) — lives in the engine's
//! `planet_x::control` module so the engine crate stays free of any web stack.

use axum::extract::State as AxState;
use axum::routing::{get, post};
use axum::{Json, Router};
use planet_x::config::parse_seed;
use planet_x::control::{
    apply_diff, control_view, scope_view, CommandReq, FactionControlView, ScopeView,
};
use planet_x::model::*;
use planet_x::prng::Prng;
use planet_x::sim;
use planet_x::world;
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
///
/// The static directory is `PLANET_X_WEB_STATIC` if set, else `<crate>/static`.
pub fn router(shared: Shared) -> Router {
    let static_dir = std::env::var("PLANET_X_WEB_STATIC")
        .unwrap_or_else(|_| format!("{}/static", env!("CARGO_MANIFEST_DIR")));
    Router::new()
        .route("/api/meta", get(get_meta))
        .route("/api/state", get(get_state))
        .route("/api/advance", post(advance))
        .route("/api/command", post(command))
        .route("/api/new", post(new_game))
        .with_state(shared)
        .fallback_service(ServeDir::new(static_dir))
}
