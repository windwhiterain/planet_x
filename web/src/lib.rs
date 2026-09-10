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
use axum::http::{header, HeaderValue};
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
use std::sync::{Arc, Mutex};
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

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

/// 通用只读信息树的**一个根**：一个名字 + 一份任意 JSON。
///
/// 前端用**同一个** schema-agnostic widget 渲染任意根——widget 不认识 `State` /
/// `Derived` / `GameConfig` 的任何字段名，只认 JSON 的形状（对象/数组/标量）。
/// 因此模型加字段/改结构，前端**一个字符都不用改**：树自动变。
///
/// 每个根的值都是 `serde_json::to_value` 出来的**模型本体**，没有任何手工投影
/// （手工投影才会漂移）。
#[derive(Serialize, Clone)]
pub struct InfoRoot {
    /// 根名（前端据此显示 tab）。
    pub name: String,
    /// 该根的整份 JSON。
    pub value: serde_json::Value,
}

/// 一次 `/api/state` 的响应：**读面 + 写面**。
///
/// 这里刻意**只有**两样东西，没有第三种：
///
/// * **写面**（[`Self::control`] / [`Self::scope`]）——命令面的读面模板。它必须是
///   **语义化**的（`Control<T>` 展开成 `{value, mode}`、`(城, 建筑)` 元组键展开成
///   `{city, building, kind, …}` 字段），因为 agent/玩家要在这上面写 presence-aware
///   的 diff（见 `planet_x::control`）。这不是「投影」，是**另一种数据结构**，保留。
/// * **读面**（[`Self::info`]）——模型的整份 dump。**没有**任何「给前端的拍平视图」：
///   以前那些 `bodies`/`cities`/`ships`/`FactionView`/`MetaView` 字段全是手工挑选的
///   投影（会漂移、会漏字段、加字段要改两处），现在前端直接从 `state` / `config` 根取
///   ——要什么自己从模型里拿，模型加字段前端自动看到。
#[derive(Serialize, Clone)]
pub struct StateView {
    /// 可控 state（写面的读模板）：各势力的舰指令/预算/权重/迁都。
    pub control: Vec<FactionControlView>,
    /// 控制作用域树（写面的读模板）：谁 AI、谁玩家。
    pub scope: ScopeView,
    /// 全量只读信息树：每个根都是模型的**整份** JSON dump（见 [`InfoRoot`]）。
    /// 读面（地图/状态面板/选中详情）全部从这里取，因此普通 state（天体/定居点/城/
    /// 建筑/势力/舰/事件/编年史…）、派生量与全部配置表都在，且**加字段即自动出现**。
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

/// 组装一次响应：写面（control/scope 的读模板）+ 读面（整份模型 dump）。
///
/// `state_view` 现在**只剩**这两件事——以前那些 `bodies`/`cities`/`ships`/`factions`
/// 拍平字段（以及 `/api/meta` 那份 BuildingMeta 改名表）都是「前端要什么就在后端拼一份」
/// 的手工投影：会漏字段、会与模型漂移、模型加字段要改两处。现在前端要什么就从
/// `info` 的 `state`/`config` 根里取。
pub fn state_view(world: &GameWorld) -> StateView {
    let s = &world.state;
    let control = s.control.iter().map(|(fid, c)| control_view(s, fid.clone(), c)).collect();
    StateView { control, scope: scope_view(&s.scope), info: info_roots(world) }
}

// --- handlers ---------------------------------------------------------------

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
///
/// 每个响应都带 `Cache-Control: no-cache`。这不是「不许缓存」，而是「用之前先问一句」：
/// `ServeDir` 只发 `Last-Modified`，浏览器于是按**启发式**缓存（`10% × (Date − Last-Modified)`）
/// 把改过的 `map3d.js`/`app.js` 缓存住——**改了前端、刷新却看不到旧代码**，排查时极费时间
/// （本轮就吃了一次：以为改动没生效，其实是浏览器喂了旧脚本）。no-cache 仍带 `Last-Modified`，
/// 命中就是 304，代价可忽略；前端改动从此「刷新即生效」。
pub fn router(shared: Shared) -> Router {
    let static_dir = std::env::var("PLANET_X_WEB_STATIC")
        .unwrap_or_else(|_| format!("{}/static", env!("CARGO_MANIFEST_DIR")));
    Router::new()
        .route("/api/state", get(get_state))
        .route("/api/advance", post(advance))
        .route("/api/command", post(command))
        .route("/api/new", post(new_game))
        .with_state(shared)
        .fallback_service(ServeDir::new(static_dir))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-cache"),
        ))
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
        // 读面**不许**再有手工投影字段：响应的顶层只有写面（control/scope）与整份树。
        // 这条测试是「想再塞一个给前端用的拍平字段」时的守门人——要读什么，从树里取。
        let keys: Vec<&str> = json.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, ["control", "info", "scope"], "StateView must stay control+scope+info");
        let post = &json["info"][2]["value"];
        assert!(!post["flow"]["faction_production"].as_object().unwrap().is_empty(), "the round flow must be real, not empty");
        assert!(!post["flow"]["upkeep"].as_object().unwrap().is_empty());
        assert!(!post["flow"]["governance"].as_object().unwrap().is_empty());
        assert!(!post["metrics"]["power_share"].as_object().unwrap().is_empty());
    }
}
