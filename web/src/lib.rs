//! `planet_x_web` — the WebUI server for Planet X, decoupled from the engine.
//!
//! This crate is the player-facing interactive front end. It owns one
//! authoritative in-memory [`planet_x::model::State`] together with the
//! deterministic RNG, and exposes a hotseat-style JSON API over the engine
//! (`planet_x` crate) as a library:
//!
//! * `GET  /api/state`     the current world (bodies, cities, factions, ships,
//!                         per-faction controllable state, control scope) **plus**
//!                         the generic read-only info tree ([`InfoRoot`]).
//! * `POST /api/advance`   run `n` rounds, return the new world.
//! * `POST /api/command`   write a faction's controllable state + the scope.
//! * `POST /api/new`       rebuild the world from a seed.
//! * `GET  /api/ping`      server identity: pid / port / binary + build age / open pages.
//! * `POST /api/tab`       register one open page (tab).
//! * `POST /api/bye`       release one page; the **last** one leaving retires the server.
//!
//! 后三条是**生命周期**面（见 [`WebCtx`]）：服务不该比「看它的人」活得久。它没有
//! 空闲计时器，只有两个「触发即退」的条件——启动它的进程退出（[`owner`]）、或者最后
//! 一个页面关掉。端口仍由 `main` 自动扫，多 worktree 并存互不干扰。
//!
//! Static files (the frontend) are served from `web/static/`. Everything that
//! is *not* HTTP — the control-diff domain (`apply_patch` / `control_surface` /
//! `control_schema_value` / patch & view types) — lives in the engine's
//! `planet_x::control` module so the engine crate stays free of any web stack.

use axum::extract::Extension as AxExtension;
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
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;

pub mod owner;

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

// --- 服务生命周期：身份 / 页面登记 / 退出闸门 --------------------------------

/// 一台 WebUI 服务的身份证：谁在跑（`pid`）、在哪（`port`）、跑的是哪个构建（`exe` +
/// 构建了多久）、几个页面在看。
///
/// 值得有：端口是**自动扫**的（`3000` 被占就 `3001`…），所以「浏览器里这个到底是哪个
/// 进程、哪个构建」从前只能靠猜。`exe_age_secs` 一眼看出「页面里是十分钟前编译的旧
/// 二进制」——旧实例锁住 `target\debug\planet_x_web.exe` 时正是这个症状。
#[derive(Serialize, Clone)]
pub struct Identity {
    pub app: &'static str,
    pub pid: u32,
    pub port: u16,
    pub uptime_secs: u64,
    /// 当前进程的可执行文件（谁在跑）。
    pub exe: String,
    /// 那个文件被写下的时刻距今多久（`None` = 取不到）。**这是判断「跑的是不是刚编的
    /// 那个」的依据**，比看目录时间戳靠谱。
    pub exe_age_secs: Option<u64>,
    /// 看护目标（`PLANET_X_WEB_OWNER_PID`）：它一退出，本服务就自退。`None` = 没人看护。
    pub owner_pid: Option<u32>,
    /// 当前登记在看这个服务的页面数。
    pub tabs: usize,
}

/// 退出闸门：任何一条「该退了」的理由都往这里投一次，**只有第一次算数**。
///
/// 用 `oneshot` 是因为它同时干两件事：给 `axum::serve` 一个 graceful-shutdown 信号，
/// 并把「为什么退」这段人话带到最后那行日志里——排查「服务怎么自己没了」时，
/// 「谁让它退的」比「它退了」重要得多。
#[derive(Clone)]
pub struct ExitGate {
    tx: Arc<Mutex<Option<oneshot::Sender<String>>>>,
}

impl ExitGate {
    fn channel() -> (ExitGate, oneshot::Receiver<String>) {
        let (tx, rx) = oneshot::channel();
        (ExitGate { tx: Arc::new(Mutex::new(Some(tx))) }, rx)
    }

    /// 请求退出；返回这次调用是否是**触发者**（重复请求返回 `false`，不再改理由）。
    pub fn request(&self, why: impl Into<String>) -> bool {
        let why = why.into();
        let taken = self.tx.lock().unwrap().take();
        match taken {
            Some(tx) => tx.send(why).is_ok(),
            None => false,
        }
    }
}

/// 服务的运行时上下文：身份 + 打开的页面 + 退出闸门。
///
/// 生命周期规则（两把，都「触发即退」，**没有空闲计时器**）：
///
/// 1. **启动者租约**（[`owner`]）：`PLANET_X_WEB_OWNER_PID` 指的进程一结束就退。
///    这条治的是「会话/job 没了，exe 变孤儿继续监听」——Windows 不会替你收孙进程。
/// 2. **最后一个页面关掉**：前端在 `pagehide` 时注销自己，登记数掉到 0 就退。
///    这条治的是「人早走了，服务还在」。`PLANET_X_WEB_CLOSE_EXIT=0` 可以关掉它。
#[derive(Clone)]
pub struct WebCtx {
    inner: Arc<WebInner>,
}

struct WebInner {
    started: Instant,
    port: u16,
    owner_pid: Option<u32>,
    exe: PathBuf,
    exe_mtime: Option<SystemTime>,
    /// 登记在看这个服务的页面（tab id）。空 = 没人看。
    tabs: Mutex<HashSet<String>>,
    /// 关掉最后一个页面是否自退。
    close_exit: bool,
    /// 「最后一个页面关了」之后等多久再真退：**只为吸收刷新**（旧页面 `pagehide` 的
    /// 注销会先到，新页面的登记晚几十毫秒）。这不是空闲超时——没人操作不会退。
    close_grace: Duration,
    exit: ExitGate,
}

impl WebCtx {
    /// 按环境变量建上下文，并把退出闸门的接收端交给 `axum::serve` 的优雅停机。
    ///
    /// * `PLANET_X_WEB_OWNER_PID`     看护目标（见 [`owner`]）；没设 = 不设租约。
    /// * `PLANET_X_WEB_CLOSE_EXIT`    `0`/`false`/`off` = 关掉「最后一个页面关了自退」。
    /// * `PLANET_X_WEB_CLOSE_GRACE_MS` 刷新窗口，默认 `500`（`0` = 关页面立刻退，
    ///   代价是**刷新会把服务带走**）。
    pub fn new(port: u16, owner_pid: Option<u32>) -> (Self, oneshot::Receiver<String>) {
        let close_exit = parse_flag(std::env::var("PLANET_X_WEB_CLOSE_EXIT").ok().as_deref(), true);
        let close_grace =
            Duration::from_millis(parse_u64(std::env::var("PLANET_X_WEB_CLOSE_GRACE_MS").ok().as_deref(), 500));
        WebCtx::with_close_policy(port, owner_pid, close_exit, close_grace)
    }

    /// 显式给策略的构造器（测试用；生产走 [`WebCtx::new`] 读环境变量）。
    pub fn with_close_policy(
        port: u16,
        owner_pid: Option<u32>,
        close_exit: bool,
        close_grace: Duration,
    ) -> (Self, oneshot::Receiver<String>) {
        let (exit, rx) = ExitGate::channel();
        let exe = std::env::current_exe().unwrap_or_default();
        let exe_mtime = std::fs::metadata(&exe).and_then(|m| m.modified()).ok();
        let ctx = WebCtx {
            inner: Arc::new(WebInner {
                started: Instant::now(),
                port,
                owner_pid,
                exe,
                exe_mtime,
                tabs: Mutex::new(HashSet::new()),
                close_exit,
                close_grace,
                exit,
            }),
        };
        (ctx, rx)
    }

    /// 退出闸门（给 main 里的看护线程用：主人一没就往这里投）。
    pub fn exit_gate(&self) -> ExitGate {
        self.inner.exit.clone()
    }

    pub fn owner_pid(&self) -> Option<u32> {
        self.inner.owner_pid
    }

    pub fn identity(&self) -> Identity {
        Identity {
            app: "planet_x_web",
            pid: std::process::id(),
            port: self.inner.port,
            uptime_secs: self.inner.started.elapsed().as_secs(),
            exe: self.inner.exe.display().to_string(),
            exe_age_secs: self.inner.exe_mtime.and_then(|t| {
                SystemTime::now().duration_since(t).ok().map(|d| d.as_secs())
            }),
            owner_pid: self.inner.owner_pid,
            tabs: self.tab_count(),
        }
    }

    pub fn tab_count(&self) -> usize {
        self.inner.tabs.lock().unwrap().len()
    }

    /// 登记一个页面；返回当前页面数。
    pub fn open_tab(&self, tab: &str) -> usize {
        let mut tabs = self.inner.tabs.lock().unwrap();
        tabs.insert(tab.to_string());
        tabs.len()
    }

    /// 注销一个页面。
    ///
    /// 只有**确实登记过**的 id 才算数（陌生 id 什么都不动、返回 `None`）——否则一个
    /// 迟到的、来自上一个服务的注销就能把当前服务带走。`Some(0)` = 最后一个页面走了。
    pub fn close_tab(&self, tab: &str) -> Option<usize> {
        let mut tabs = self.inner.tabs.lock().unwrap();
        if !tabs.remove(tab) {
            return None;
        }
        Some(tabs.len())
    }

    /// 刚有一个页面注销：若它是最后一个，等一个刷新窗口，期间没人回来就退。
    pub fn tab_closed(&self) {
        if !self.inner.close_exit {
            return;
        }
        let me = self.clone();
        tokio::spawn(async move {
            if !me.inner.close_grace.is_zero() {
                tokio::time::sleep(me.inner.close_grace).await;
            }
            if me.tab_count() == 0 {
                me.inner.exit.request("最后一个页面已关闭");
            }
        });
    }
}

/// 开关型环境变量：`0` / `false` / `no` / `off`（大小写无关、忽略空白）= 关，别的 = 开，
/// 没设 / 空 = `default`。
fn parse_flag(raw: Option<&str>, default: bool) -> bool {
    let Some(s) = raw else { return default };
    let s = s.trim().to_ascii_lowercase();
    match s.as_str() {
        "" => default,
        "0" | "false" | "no" | "off" => false,
        _ => true,
    }
}

/// 非负整数型环境变量：没设 / 空 / 不是数字 = `default`。
fn parse_u64(raw: Option<&str>, default: u64) -> u64 {
    let Some(s) = raw else { return default };
    let s = s.trim();
    if s.is_empty() {
        return default;
    }
    s.parse().unwrap_or(default)
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

// --- 生命周期 handlers ------------------------------------------------------

/// 一个页面的登记体（`/api/tab` 与 `/api/bye` 同形）。
#[derive(Deserialize)]
pub struct TabReq {
    #[serde(default)]
    pub tab: String,
}

/// 页面数回执。
#[derive(Serialize)]
pub struct TabCount {
    pub tabs: usize,
}

/// `GET /api/ping` —— 我是谁、在哪、跑的是哪个构建、几个页面在看。
///
/// 轻量（不碰世界、不加锁），所以前端可以随手打，agent 也可以用它确认「3001 上那个
/// 到底是不是我刚起的」。
async fn ping(AxExtension(web): AxExtension<WebCtx>) -> Json<Identity> {
    Json(web.identity())
}

/// `POST /api/tab` —— 一个页面报到了（载入时、以及从 bfcache 回来时）。
async fn tab_open(AxExtension(web): AxExtension<WebCtx>, Json(req): Json<TabReq>) -> Json<TabCount> {
    Json(TabCount { tabs: web.open_tab(&req.tab) })
}

/// `POST /api/bye` —— 一个页面走了；**最后一个**走的会触发服务自退（留一个刷新窗口）。
async fn tab_bye(AxExtension(web): AxExtension<WebCtx>, Json(req): Json<TabReq>) -> Json<TabCount> {
    match web.close_tab(&req.tab) {
        Some(0) => {
            web.tab_closed();
            Json(TabCount { tabs: 0 })
        }
        Some(left) => Json(TabCount { tabs: left }),
        // 陌生 id：什么都不动（迟到/伪造的注销不该带走当前服务）。
        None => Json(TabCount { tabs: web.tab_count() }),
    }
}

// --- 监听端口 ---------------------------------------------------------------

/// 自动挑一个空闲端口：从 `base` 起向上扫 `attempts` 个端口，绑上**第一个**空闲的。
///
/// 全都占着就退到 OS 临时端口（`bind :0`，内核给哪个算哪个）——于是「自动模式」
/// **不会**因为端口被占而启动失败。想钉死某个端口就别走这条路：直接绑那个端口，
/// 占用即报错（见 `main`）——显式点名的端口被默默换掉，比启动失败更难查。
///
/// 为什么是「向上扫」而不是「直接 `:0`」：本机常同时开着好几个 WebUI（玩家一个、
/// agent 再开一个做实机验证），URL 钉在 `3000` / `3001` / `3002` 附近比一个随机高位
/// 端口好记、好写进脚本，而且 `3000` 空着时行为与从前**逐字一致**。
///
/// `base = 0` 时第一次尝试就是「让内核挑」，即 OS 临时端口。
pub async fn bind_auto(host: &str, base: u16, attempts: u16) -> std::io::Result<TcpListener> {
    let mut busy: Option<std::io::Error> = None;
    for offset in 0..attempts {
        let Some(port) = base.checked_add(offset) else { break };
        match TcpListener::bind((host, port)).await {
            Ok(listener) => return Ok(listener),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => busy = Some(e),
            // 别的错误（端口越权、地址不合法…）不是「这个端口被占」——换端口也没用，
            // 直接上报，别把它藏成「扫了一百个都不行」。
            Err(e) => return Err(e),
        }
    }
    match TcpListener::bind((host, 0)).await {
        Ok(listener) => Ok(listener),
        // 连临时端口都拿不到：报「端口被占」这个更可能的原因（`busy`）。
        Err(e) => Err(busy.unwrap_or(e)),
    }
}

/// Build the axum router serving the JSON API and the static frontend.
///
/// The static directory is `PLANET_X_WEB_STATIC` if set, else `<crate>/static`.
///
/// `web` 带上生命周期面（身份 / 页面登记 / 退出闸门）——路由本身不决定什么时候退，
/// 它只把「谁在看」记下来、把「我是谁」答出去；决定权在 [`WebCtx`] 与 [`owner`]。
///
/// 每个响应都带 `Cache-Control: no-cache`。这不是「不许缓存」，而是「用之前先问一句」：
/// `ServeDir` 只发 `Last-Modified`，浏览器于是按**启发式**缓存（`10% × (Date − Last-Modified)`）
/// 把改过的 `map3d.js`/`app.js` 缓存住——**改了前端、刷新却看不到旧代码**，排查时极费时间
/// （本轮就吃了一次：以为改动没生效，其实是浏览器喂了旧脚本）。no-cache 仍带 `Last-Modified`，
/// 命中就是 304，代价可忽略；前端改动从此「刷新即生效」。
pub fn router(shared: Shared, web: WebCtx) -> Router {
    let static_dir = std::env::var("PLANET_X_WEB_STATIC")
        .unwrap_or_else(|_| format!("{}/static", env!("CARGO_MANIFEST_DIR")));
    Router::new()
        .route("/api/state", get(get_state))
        .route("/api/advance", post(advance))
        .route("/api/command", post(command))
        .route("/api/new", post(new_game))
        .route("/api/ping", get(ping))
        .route("/api/tab", post(tab_open))
        .route("/api/bye", post(tab_bye))
        .with_state(shared)
        .layer(AxExtension(web))
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

    /// 起始端口空着时，自动模式**必须**原样用它——「`3000` 空着就和从前一样」是这条
    /// 特性的全部兼容性承诺，破了它等于偷偷改掉所有人的 URL。
    #[tokio::test]
    async fn bind_auto_keeps_the_base_port_when_it_is_free() {
        // 先问内核要一个肯定空闲的端口，再放手把它交给 `bind_auto`。
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let listener = bind_auto("127.0.0.1", port, 16).await.expect("一个刚放手的端口能重绑");
        assert_eq!(listener.local_addr().unwrap().port(), port);
    }

    /// 占用时**让位**：向上扫到下一个空闲端口，而不是把「地址已占用」原样抛出去——
    /// 这正是「本机已经有 `3000` 在跑」时想要的。也顺带钉住「绝不返回已占端口」。
    #[tokio::test]
    async fn bind_auto_skips_a_busy_port() {
        let held = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let busy = held.local_addr().unwrap().port();

        let listener = bind_auto("127.0.0.1", busy, 16).await.expect("邻居端口总有空的");
        let picked = listener.local_addr().unwrap().port();
        assert_ne!(picked, busy, "自动模式不能把已经被占的端口当成自己的");
        assert!(picked >= busy, "自动模式只向上扫：{picked} < {busy}");
    }

    /// 环境变量的读法：`0`/`false`/`no`/`off` 是关，别的（含乱写）是开，没设/空回落默认。
    #[test]
    fn env_flag_and_u64_parsing() {
        assert!(parse_flag(None, true), "没设 = 默认");
        assert!(!parse_flag(None, false));
        assert!(parse_flag(Some(""), true), "空 = 默认");
        assert!(!parse_flag(Some(" 0 "), true));
        assert!(!parse_flag(Some("FALSE"), true));
        assert!(!parse_flag(Some("no"), true));
        assert!(!parse_flag(Some("off"), true));
        assert!(parse_flag(Some("1"), false));
        assert!(parse_flag(Some("yes"), false));

        assert_eq!(parse_u64(None, 500), 500);
        assert_eq!(parse_u64(Some(""), 500), 500);
        assert_eq!(parse_u64(Some(" 0 "), 500), 0, "0 = 关页面立刻退，是合法值");
        assert_eq!(parse_u64(Some("1200"), 500), 1200);
        assert_eq!(parse_u64(Some("banana"), 500), 500);
    }

    /// 页面登记：陌生 id 不许动任何东西。这是「迟到的注销把当前服务带走」的守门人——
    /// 一个来自**上一个**服务的 `bye` 不该让新服务退出。
    #[tokio::test]
    async fn unknown_tab_id_changes_nothing() {
        let (web, _rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
        assert_eq!(web.close_tab("never-seen"), None);
        assert_eq!(web.tab_count(), 0);
        web.open_tab("a");
        assert_eq!(web.close_tab("bogus"), None);
        assert_eq!(web.tab_count(), 1, "陌生 id 不该让别人的登记消失");
    }

    /// 最后一个页面关掉才退；还留着一个页面时不许退。
    #[tokio::test]
    async fn last_tab_leaving_retires_the_server() {
        let (web, mut rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
        web.open_tab("a");
        web.open_tab("b");

        // a 走了，b 还在 → 不许退。
        assert_eq!(web.close_tab("a"), Some(1));
        web.tab_closed();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err(), "还有一个页面在看，不该退");

        // b 也走了 → 退，并且带上人话理由。
        assert_eq!(web.close_tab("b"), Some(0));
        web.tab_closed();
        let why = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("最后一个页面关掉必须触发退出")
            .expect("理由要送到");
        assert!(why.contains("页面"), "退出理由要说清是谁让退的：{why}");
    }

    /// `PLANET_X_WEB_CLOSE_EXIT=0` 时，关页面**不**退（只留启动者租约这条命）。
    #[tokio::test]
    async fn close_exit_can_be_disabled() {
        let (web, mut rx) = WebCtx::with_close_policy(0, None, false, Duration::ZERO);
        web.open_tab("only");
        assert_eq!(web.close_tab("only"), Some(0));
        web.tab_closed();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(rx.try_recv().is_err(), "关掉 close-exit 后关页面不该退");
    }

    /// 退出闸门只认第一次：后来的理由不许覆盖第一个（也不许再触发一次）。
    #[tokio::test]
    async fn exit_gate_fires_once() {
        let (web, rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
        let gate = web.exit_gate();
        assert!(gate.request("第一个理由"));
        assert!(!gate.request("第二个理由"), "第二次不许再触发");
        assert_eq!(rx.await.unwrap(), "第一个理由");
    }

    /// `/api/ping` 必须如实报出身份：pid / 端口 / 二进制路径 / 页面数。agent 靠它区分
    /// 「3001 上那个是不是我刚起的」。
    #[tokio::test]
    async fn ping_reports_identity() {
        let (web, _rx) = WebCtx::with_close_policy(3013, Some(4242), true, Duration::ZERO);
        web.open_tab("t");
        let Json(id) = ping(AxExtension(web.clone())).await;
        assert_eq!(id.app, "planet_x_web");
        assert_eq!(id.pid, std::process::id());
        assert_eq!(id.port, 3013);
        assert_eq!(id.owner_pid, Some(4242));
        assert_eq!(id.tabs, 1);
        assert!(id.exe.ends_with(".exe") || !id.exe.is_empty(), "要报出在跑哪个文件");
    }

    /// 3 条生命周期路由都要注册上（`/api/ping` GET，`/api/tab`、`/api/bye` POST）——
    /// 前端 `pagehide` 的注销打不中就等于没有自退。
    #[test]
    fn lifecycle_routes_are_mounted() {
        let (web, _rx) = WebCtx::with_close_policy(0, None, true, Duration::ZERO);
        let shared: Shared = Arc::new(Mutex::new(world()));
        let app = router(shared, web);
        // `Router` 没有公开的路由表读取接口；用 `Debug` 打印确认这三条真的挂上了。
        let dumped = format!("{app:?}");
        for path in ["/api/ping", "/api/tab", "/api/bye"] {
            assert!(dumped.contains(path), "{path} 没挂上：{dumped}");
        }
    }
}
