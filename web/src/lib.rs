//! `planet_x_web` — the WebUI server for Planet X, decoupled from the engine.
//!
//! This crate is the player-facing interactive front end. It owns one
//! authoritative in-memory [`planet_x::model::State`] together with the
//! deterministic RNG, and exposes a hotseat-style JSON API over the engine
//! (`planet_x` crate) as a library:
//!
//! * `GET  /api/meta`      resource/building/structure/ship metadata.
//! * `GET  /api/state`     the current world (bodies, cities, factions, ships,
//!                         per-faction controllable state, control scope) **plus**
//!                         the generic read-only info tree ([`InfoRoot`]).
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
    /// 上一回合开头（还没掷随机）的派生观测 `pre`。
    pub pre: Derived,
    /// 上一回合结束的派生态 `post`：`flow` 是步进函数**实际用过**的流量
    /// （每城/每势力产出、舰队维护费、治理成本/覆盖率），只有 [`sim::advance`] 的
    /// 返回值才带得到；`metrics` 是该回合结束时的存量/政治总结。语义与 CLI `--save`
    /// 的 [`RoundState`] 完全一致（round 0 / 新局时两者都是 [`sim::derived_from_state`]
    /// 的「无流量」观测）。
    pub post: Derived,
}

impl GameWorld {
    /// 建一个世界：派生快照初始化为「只看当前 state、尚无流量」的观测。
    pub fn new(state: State, config: GameConfig, rng: Prng) -> Self {
        let d = sim::derived_from_state(&state, &config);
        GameWorld { state, config, rng, pre: d.clone(), post: d }
    }
}

pub type Shared = Arc<Mutex<GameWorld>>;

// --- wire types -------------------------------------------------------------

#[derive(Serialize)]
pub struct MetaView {
    pub resources: BTreeMap<String, ResourceDef>,
    pub structures: BTreeMap<String, StructureSpec>,
    pub buildings: BTreeMap<String, BuildingMeta>,
    pub ships: BTreeMap<String, ShipSpec>,
    /// 天体/行星**类型**表（`BodyKindSpec`）：`state` 天体只带 `kind` key，前端据本表
    /// 解析出颜色/尺寸/类别/星环/着色器分支等视觉属性。
    pub body_kinds: BTreeMap<String, BodyKindSpec>,
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

/// 通用只读信息树的**一个根**：一个名字 + 一份任意 JSON。
///
/// 前端用**同一个** schema-agnostic widget 渲染任意根——widget 不认识 `State` /
/// `Derived` / `GameConfig` 的任何字段名，只认 JSON 的形状（对象/数组/标量）。
/// 因此模型加字段/改结构，前端**一个字符都不用改**：树自动变。
///
/// 每个根的值都是 `serde_json::to_value` 出来的**模型本体**，没有任何手工投影
/// （手工投影才会漂移）——`StateView` 里那些「拍平给地图用」的字段是另一回事，
/// 这里不做那件事。
#[derive(Serialize, Clone)]
pub struct InfoRoot {
    /// 根名（前端据此显示 tab）。
    pub name: String,
    /// 该根的整份 JSON。
    pub value: serde_json::Value,
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
    /// 全量只读信息树：每个根都是模型的**整份** JSON dump（见 [`InfoRoot`]）。
    /// 前端的「状态」面板用通用 widget 渲染它，因此普通 state（天体/城/势力/舰/
    /// 事件/编年史…）以及派生量与全部配置表都在这里，且**加字段即自动出现**。
    #[serde(default)]
    pub info: Vec<InfoRoot>,
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

/// 通用信息树的根：**整份模型**的 JSON dump，没有任何手工挑字段。
///
/// * `state`   —— 规范世界（普通 state：天体/定居点/城/建筑/势力/舰/事件/编年史…，
///   以及可控 state `control` 与作用域 `scope`——它是 `State` 的一部分，自然也在）。
/// * `pre` / `post` —— 上一回合的派生态（`RoundState` 的一对）：`post` 带**真实的
///   本回合流量**（每城/每势力产出、舰队维护费、治理成本/覆盖率）与回合末的存量/
///   政治总结（实力占比/霸权/联盟/制裁/战争…）；`pre` 是回合开头（随机未落地）
///   的同构观测。由 [`sim::advance`] / [`sim::derived_from_state`] 产出，与 CLI
///   `--save` 落盘的 checkpoint 同源、逐回合一致。
/// * `config`  —— `config/game.ron` 的全部调参表（经济/战斗/外交/市场/治理/MOND/
///   均势/思潮 + 资源/结构/天体类型/舰/组件/建筑/剧情/名字库）。
/// * `session` —— 会话层信息（当前 RNG 位置），不属于世界状态但值得一看。
///
/// 加一个新根 = 在这里加一行；前端会自动多出一个 tab。dump 走引擎的
/// [`planet_x::json::to_value`]（而非 `serde_json::to_value`）：它把 RON 里允许的
/// 非字符串 map key（如 `(城市, 建筑id)` 元组键）转成字符串，所以**整份模型**都能
/// 落到 JSON，而不是只有被手工挑过、恰好 JSON-able 的那部分。
fn info_roots(world: &GameWorld) -> Vec<InfoRoot> {
    let root = |name: &str, value: serde_json::Value| InfoRoot { name: name.to_string(), value };
    vec![
        root("state", planet_x::json::to_value(&world.state).expect("state is dumpable")),
        root("pre", planet_x::json::to_value(&world.pre).expect("pre is dumpable")),
        root("post", planet_x::json::to_value(&world.post).expect("post is dumpable")),
        root("config", planet_x::json::to_value(&world.config).expect("config is dumpable")),
        root(
            "session",
            serde_json::json!({
                "rng_state": world.rng.state(),
                "schema_version": world.state.schema_version,
            }),
        ),
    ]
}

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
        info: info_roots(world),
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
        body_kinds: world.config.body_kinds.clone(),
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
        // 与 CLI 同语义：`pre` = 回合开头（随机还没落地）的观测，`post` = advance 的返回
        // （带本回合真实流量）。两者都随最后一次推进更新，成为「当前回合记录」。
        world.pre = sim::derived_from_state(&world.state, &world.config);
        world.post = sim::advance(&mut world.state, &world.config, &mut world.rng);
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
    let d = sim::derived_from_state(&world.state, &world.config);
    world.pre = d.clone();
    world.post = d;
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

#[cfg(test)]
mod tests {
    use super::*;
    use planet_x::config::load_config_from;

    /// The workspace config, anchored on this crate's manifest dir so the tests do
    /// not depend on the process cwd (cargo runs them with cwd = `web/`).
    fn world() -> GameWorld {
        let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../config/game.ron"));
        let config = load_config_from(path).expect("workspace config/game.ron loads");
        let state = world::default_state(&config, 42);
        GameWorld::new(state, config, Prng::new(42))
    }

    /// 信息树必须是**整份模型**的 dump——没有任何手工挑字段。手工挑字段正是会漂移
    /// 的东西：`StateView` 手工列举的字段之外（`ship_name_seq` / `schema_version` /
    /// 各实体的全部字段）也必须都在树里。这条测试就是「模型加字段，前端自动看到」
    /// 这个不变量的守门人。
    #[test]
    fn info_roots_are_whole_model_dumps() {
        let w = world();
        let roots = info_roots(&w);
        let names: Vec<&str> = roots.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["state", "pre", "post", "config", "session"]);

        let state = &roots[0].value;
        assert_eq!(
            state,
            &planet_x::json::to_value(&w.state).unwrap(),
            "the `state` root must be the raw State, not a projection"
        );
        for k in ["schema_version", "ship_name_seq", "control", "scope", "events", "chronicle"] {
            assert!(state.get(k).is_some(), "State field `{k}` missing from the info tree");
        }
        // 实体也要带**全部**字段（不是给地图用的那套拍平视图）。
        let ship = &state["ships"][0];
        for k in ["name", "faction_id", "doctrine", "kiting"] {
            assert!(ship.get(k).is_some(), "Ship field `{k}` missing from the info tree");
        }

        // 可控 state 也整份在树里（含 RON 特有的元组键 → "城市|建筑id" 字符串键）。
        let ctrl = &state["control"];
        assert!(ctrl.is_object(), "control must be dumped as a map of faction -> ControllableState");

        for i in [1, 2] {
            assert!(roots[i].value.get("flow").is_some(), "derived root must carry flow");
            assert!(roots[i].value["metrics"].get("faction_power").is_some());
        }

        let config = &roots[3].value;
        for k in ["economy", "combat", "diplomacy", "market", "governance", "mond", "balance", "ideology", "resources", "structures", "body_kinds", "ships", "components", "buildings", "story", "name_pool"] {
            assert!(config.get(k).is_some(), "GameConfig section `{k}` missing from the info tree");
        }

        assert!(roots[4].value.get("rng_state").is_some());
    }

    /// The info tree must survive a round of simulation (events/chronicle filled
    /// in) and stay serializable end-to-end — that is what `/api/state` returns.
    /// It must also carry the **real** per-round flow, which only `sim::advance`
    /// produces (production / upkeep / governance captured while stepping).
    #[test]
    fn info_tree_carries_real_round_flow_after_advance() {
        let mut w = world();
        assert!(w.post.flow.faction_production.is_empty(), "round 0 has no flow yet");
        for _ in 0..5 {
            w.pre = sim::derived_from_state(&w.state, &w.config);
            w.post = sim::advance(&mut w.state, &w.config, &mut w.rng);
        }
        let view = state_view(&w);
        let json = serde_json::to_value(&view).expect("StateView serializes");
        assert_eq!(json["info"][0]["value"]["round"].as_u64().unwrap(), 5);
        assert!(json["info"][0]["value"]["events"].is_array());
        let post = &json["info"][2]["value"];
        assert!(!post["flow"]["faction_production"].as_object().unwrap().is_empty(), "the round flow must be real, not empty");
        assert!(!post["flow"]["upkeep"].as_object().unwrap().is_empty());
        assert!(!post["flow"]["governance"].as_object().unwrap().is_empty());
        assert!(!post["metrics"]["power_share"].as_object().unwrap().is_empty());
    }
}
