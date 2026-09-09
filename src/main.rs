//! planet_x — a sandbox trajectory generator for the Planet X game.
//!
//! The CLI is agent-first: it always emits zero-noise JSON Lines (or one JSON
//! value per REPL command) for an LLM agent player. There is no separate
//! `--agent` flag and no interactive human / trajectory mode — the tool's only
//! mode is this machine-readable one.
//!
//! Usage: `planet_x [--seed <random>] [--start <path.ron>] [--round <n>]`
//! `[--query <jq>] [--apply <file.json>] [--script <file>]`
//!
//! * `--seed`   the deterministic RNG seed, or `random` (the default).
//! * `--start`  load an initial `State` from a RON file; otherwise the default
//!              procedural solar system is generated.
//! * `--round`  run `n` rounds and emit one JSON object per round (round 0
//!              first, then one per round) as JSON Lines.
//! * `--query`  run a jq filter over the agent state and print JSON Lines. With
//!              `--round N` it advances N rounds first, so filters run against
//!              the end-of-sim state (e.g. the surviving marines or captured
//!              cities). Useful for one-shot, composable pipelines.
//! * `--apply`  overlay a control-state diff (JSON, same shape as the web
//!              `POST /api/command`: `{control:[...], scope:{...}}`) onto the
//!              state before anything else runs. A structural multi-level patch:
//!              only the factions/leaves present in the file are touched, and a
//!              leaf's omitted `value`/`behavior` keeps the current value while
//!              an omitted `mode` keeps the current mode.
//! * `--script` read REPL commands from a file, run them non-interactively, and
//!              exit. Deterministic and zero-noise on stdout; ideal for a
//!              scripted agent loop with `--start` + `--apply`.
//!
//! Without `--round` the tool enters the **query REPL**: commands over stdin or
//! a `--script` file (`q <jq>`, `summary`, `advance [n]`, `delta [n]`,
//! `control`, `apply <file>`, `save`/`load <file>`, `cities`, `city <id>`,
//! `ships`, `faction <id>`, `bodies`, `guide`, `quit` — plus aliases `s`/`a`/`q`),
//! each returning a JSON value —
//! hierarchical, on-demand access instead of a per-round full dump. `control`
//! emits the editable control surface, and `apply <file>` overlays a diff onto
//! the state mid-session.

use clap::Parser;
use planet_x::agent;
use planet_x::config::{load_checkpoint, load_config, load_initial, parse_seed, save_checkpoint};
use planet_x::model::{City, GameConfig, Ship, State};
use planet_x::prng::Prng;
use planet_x::{query, sim, web, world};
use serde_json::json;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "planet_x",
    version,
    about = "行星X——太空沙盘轨迹生成器",
    long_about = "《行星X》是一个回合制太阳系沙盘轨迹生成器：每回合 = 1 个月，整个游戏由一个可确定复现的\n\
State 快照推进，全部数值由 config/game.ron 数据驱动、不硬编码。\n\
\n\
【世界与实体】\n\
- 天体 body：绕太阳做 2D 椭圆轨道（给定近日点/远日点距离、远日点方向、公转周期），每回合位置\n\
  按轨道重算。18 个天体（spec 天体表）各有 1~5 个定居点 settlement（地球 5 城各占一个；\n\
  气态巨行星的定居点为轨道空间站）。定居点与城市一一对应：每个定居点至多一座城市，含有限总面积、\n\
  生态容量（人口/面积）、建设速度修正、建设资源修正，以及若干资源矿藏（类型 + 面积，限定采矿上限）。\n\
- 城市 city：建在定居点上、由一个势力控制，内有若干连续面积分配的 建筑，并有人口（限制生产效率）；\n\
- 建筑 building：非原子，是一个连续面积分配（计划 area 与实际 deployed），总和不超定居点总面积。三种角色：\n\
  residential 居住点（提供人口容量）、mining 开采点（采对应矿藏资源）、construction 建造点（船坞，造舰）。\n\
- 势力 faction：拥有城市与飞船，库存各资源，并与其它势力两两外交（关系 relations）。\n\
- 飞船 ship：必属某一势力，从城市出厂，在 2D 平面移动，可按指令开火/围城。舰级参数（护甲 hull、护甲再生\n\
  hull_regen、伤害、速度、攻击距离、点防御修正、建造点/建造成本、维护费 upkeep）由 config 定义，spec 五级舰：\n\
  护卫舰 corvette、驱逐舰 destroyer、巡洋舰 cruiser、航空母舰 carrier、战列舰 battleship；每级把 spec 的「招牌」\n\
  数值拉到极高，使各级各有一席之地：护卫舰=高加速度+高雷达点防的哨戒、驱逐舰=高再生高速的远洋部署、\n\
  巡洋舰=重甲重盾的扛伤害主力、航空母舰=超远程放风筝、战列舰=玻璃大炮（一锤定音）。\n\
\n\
【资源】11 种：水冰 water_ice、氦-3 helium3、铀 uranium、钍 thorium、金 gold、铂 platinum、铁 iron、\n\
氢 hydrogen、甲烷 methane、碳 carbon、硅 silicon。\n\
\n\
【每回合演化（sim::advance）】\n\
1. 天体位置按轨道重算。\n\
2. 经济：开采点按 面积×劳动力×生产率 产出资源；人口向住房容量增长（增长比例受 pop_growth 控制）。\n\
3. 维护 upkeep：每艘幸存舰每回合按舰级 upkeep 从本方资源（按价值加权）扣除维护费；付不起则舰队锈蚀（扣 hull）。\n\
4. 市场：每势力自动以「参考价值」把富余矿物兑换成其所缺的关键矿物（维持 working_buffer 工作库存，收 spread 价差），\n\
   使资源分布不均不再卡死舰队——富余矿物有了下游消耗（资源池），缺 keymineral 也能继续造舰。\n\
5. 建设：建筑按各自投资权重竞争本轮资源预算（默认每资源最多拿出库存的 invest_fraction=0.3 投入建设）；\n\
   连续地把计划面积建成 deployed；船坞按面积×劳动力 累积造船进度，够了就付建造成本造出新舰。\n\
6. 军事：每艘舰按指令 移动/开火/围城。开火削减目标舰 hull；围城削减城市 defense；defense≤0 则城市被\n\
   攻占（守备重置、人口×0.6、改由攻击方控制）。关系 ≤ 战争阈值（war_threshold=-20）即视为敌对（wars）。\n\
7. 外交：开局和平（无战争状态），国际关系动态波动——每对势力按意识形态(alignment)静息亲和漂移（阵营靠拢/异己升温）、\n\
   好战(aggression)加速敌对化；开火/占领会把关系(attack_delta/capture_delta)压到战争阈值(war_threshold=-20)之下进入战争；\n\
   停战后关系经战争疲态(war_fatigue)向停战线(ceasefire_relation)回落，随后可再度升温——战争有始有终，而非永久僵局。\n\
\n\
【控制模型 = 指令】每个势力有一份可控状态 State::control：\n\
- ship_orders：本方各舰的 行为（ShipBehavior）：Idle（待命，原地）、Move{position}（前往某位置）、\n\
  TargetShip{ship,attack}（attack=true 追袭某舰并开火；attack=false 守卫：靠近并保护某友舰，\n\
  对近身敌方舰拦截开火，不攻击被保护舰）、TargetSettlement{city,bombard}（围攻某城）、\n\
  Dock{body}（停泊轨道：持续驶向某天体当前位置、随其巡航）。\n\
- budget：每种资源每回合的投资预算（决定拿出多少资源用于建设）。\n\
- invest_weights：本方各建筑的 建设投资权重（决定建造优先级）。\n\
每个可控叶子带一个 mode：Ai（系统自动决策/改写）| Player（玩家指令，系统只读不改写）| None（继承上层）。\n\
State::scope 是一棵作用域树（全局→势力→天体→城市），决定某叶子由谁控制：沿链上溯、取最具体者，全 None 默认 Ai。\n\
\n\
【agent 怎么玩】stdout 只输出零噪声 JSON Lines（无颜色/星图/表格/散文，浮点四舍五入到 2 位）。\n\
- `control` 命令读出当前可编辑的控制面（control + scope）JSON，即你要改写的模板；\n\
- 只改想动的叶子，把结果作为 diff 用 `--apply <file>` 或 REPL `apply <file>` 结构化、多层级叠加：只触碰\n\
  文件里出现的势力/叶子；叶子内省略 value/behavior 保留当前值、省略 mode 保留当前模式（mode:null=继承）；\n\
  整叶不出现则完全不动。既可只移动一条船，也可整体替换一个势力。\n\
- `advance [n]` 推进 n 回合（默认 1）；`--query <jq>` 或 `q <jq>` 对当前状态执行 jq 过滤；`summary` 打印雷达。\n\
- `meta`（或 `--meta`）输出整份游戏配置 JSON：resources（raw key→中文名）、buildings/ships 全表、\n\
  economy/combat/diplomacy 常量——即 agent 的规则字典，也是状态中文名↔control 原始 key 的映射表；\n\
  可跟 `meta <jq>` 过滤。\n\
- 每一行 JSON 是 `{round,time_month,factions,bodies,cities,ships}`，字段固定、引用一律用整数 id、名称字段便于直读；\n\
  飞船 order 为 tag 联合 `{type:\"idle\"|\"move\"|\"target_ship\"|\"target_settlement\",...}`。",
    after_help = "agent 专用：stdout 只输出零噪声 JSON Lines（或每条 REPL 命令一个 JSON 值），无颜色/星图/表格/散文。--round N 输出 N+1 行 JSON（回合 0 + N 回合）；不带 --round 时进入查询 REPL。--apply 在开始时叠加可控制状态 diff，--script 从文件非交互执行 REPL 命令。"
)]
struct Cli {
    /// 确定性随机种子（数字，或 random / 随机）
    #[arg(long, default_value = "random", value_name = "SEED")]
    seed: String,

    /// 从指定的初始状态 .ron 文件开始
    #[arg(long, value_name = "PATH")]
    start: Option<PathBuf>,

    /// 运行 n 个回合并把每个回合输出为一行 JSON（回合 0 先），而非交互式推进
    #[arg(long = "round", visible_alias = "rounds", value_name = "N")]
    round: Option<u32>,

    /// 对初始(或 --start 加载的)状态执行一个 jq 过滤并输出 JSON Lines 结果。
    /// 例如: `planet_x --query '.cities[] | select(.owner_name=="中国") | {name,population}'`
    #[arg(long, value_name = "JQ")]
    query: Option<String>,

    /// 输出整份游戏配置（resources/buildings/ships/economy/combat/diplomacy）为一行
    /// JSON，然后退出。这是 agent 的规则字典：资源的 raw key→中文名映射，以及建造/舰船
    /// 数值规则。可配合 --query 对 config 做 jq 过滤。
    #[arg(long)]
    meta: bool,

    /// 从文件读取一段 REPL 命令脚本，非交互式执行后退出。stdout 仍只输出
    /// JSON Lines；配合 --start/--apply 可串联成一个回合的 agent 决策循环。
    #[arg(long, visible_alias = "commands", value_name = "FILE")]
    script: Option<PathBuf>,

    /// 把一份可控状态 diff（JSON，形同 web 的 POST /api/command：{control,scope}）
    /// 按键 overlay 到状态上，再继续执行 --round/REPL。用于 agent 下达指令。
    #[arg(long, value_name = "FILE")]
    apply: Option<PathBuf>,

    /// 批量运行（--round/--query）结束后，把当前状态连同 PRNG 位置写入一个
    /// RON checkpoint，以便下次用 --start 确定性续玩（复现后续回合）。
    #[arg(long, value_name = "FILE")]
    save: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();

    // Agent-first output: always zero-noise. Redirected output keeps the single
    // JSON Lines stream clean, so disable colours unconditionally.
    colored::control::set_override(false);

    let config = load_config();
    let seed = parse_seed(&cli.seed);

    // --meta: dump the game config (rules dictionary) and exit. Doesn't need a
    // world. Honors --query <jq> to filter the meta value; otherwise prints it
    // whole as a single JSON line.
    if cli.meta {
        let input = agent::meta_value(&config);
        match &cli.query {
            Some(filter) => match query::apply_lines(&input, filter) {
                Ok(lines) => {
                    if !lines.is_empty() {
                        emit(&lines);
                    }
                }
                Err(e) => {
                    eprintln!("{}", json!({"ok": false, "code": "ERR_QUERY", "message": e.to_string()}));
                    std::process::exit(10);
                }
            },
            None => emit(&input.to_string()),
        }
        return;
    }

    // Initial state: from a checkpoint (state + RNG) or bare .ron state file
    // (fresh RNG), or generated procedurally.
    let (mut state, mut rng) = match &cli.start {
        Some(path) => load_initial(path, seed),
        None => (world::default_state(&config, seed), Prng::new(seed)),
    };

    // Apply a control-state diff (agent instructions) before anything else.
    if let Some(path) = &cli.apply {
        if let Err(e) = apply_diff(&mut state, path) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_APPLY", "message": e}));
            std::process::exit(10);
        }
    }

    // Batch query: run a jq filter over the agent state and print JSON Lines.
    // Honors --round N by advancing that many rounds first, so the filter runs
    // against the end-of-sim state. Takes precedence over interactive/REPL.
    if let Some(filter) = &cli.query {
        let n = cli.round.unwrap_or(0);
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
        }
        if let Some(path) = &cli.save {
            if let Err(e) = save_checkpoint(path, &state, &rng) {
                eprintln!("{}", json!({"ok": false, "code": "ERR_SAVE", "message": e.to_string()}));
                std::process::exit(10);
            }
        }
        let input = agent::state_value(&state, &config);
        match query::apply_lines(&input, filter) {
            Ok(lines) => {
                if !lines.is_empty() {
                    emit(&lines);
                }
            }
            Err(e) => {
                eprintln!("{}", json!({"ok": false, "code": "ERR_QUERY", "message": e.to_string()}));
                std::process::exit(10);
            }
        }
        return;
    }

    match cli.round {
        Some(n) => run_rounds(&mut state, &config, &mut rng, n, cli.save.as_deref()),
        None if cli.script.is_some() => {
            let path = cli.script.as_ref().expect("script is some");
            match File::open(path) {
                Ok(file) => {
                    let mut reader = BufReader::new(file);
                    run_agent_repl(&mut state, &config, &mut rng, &mut reader);
                }
                Err(e) => {
                    eprintln!(
                        "{}",
                        json!({"ok": false, "code": "ERR_SCRIPT", "message": format!("cannot open {}: {e}", path.display())})
                    );
                    std::process::exit(10);
                }
            }
        }
        None => run_agent_repl(&mut state, &config, &mut rng, &mut io::stdin().lock()),
    }
}

/// Read a control-state diff file (JSON, same shape as `POST /api/command`,
/// i.e. `{control:[...], scope:{...}}`) and overlay it onto `state` by key.
fn apply_diff(state: &mut State, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let config = load_config();
    web::apply_patch(state, &config, &value)
}

/// The editable control surface filtered to a single faction. A fraction of the
/// token cost of the whole-surface dump, so an agent can read just the faction
/// it is driving: `control <faction_id>`.
fn control_for_faction(state: &State, fid: u32) -> serde_json::Value {
    let mut v = web::control_surface(state);
    if let Some(control) = v.get_mut("control").and_then(|c| c.as_array_mut()) {
        control.retain(|c| c.get("faction_id").and_then(|x| x.as_u64()) == Some(fid as u64));
    }
    v
}

/// High-level ship command: `order <ship> attack|guard|siege|move|dock|colonize|idle ...`.
/// Builds the same `{control:[{ship_orders:[...]}]}` diff the agent writes by
/// hand, so a human/agent can iterate quickly without writing nested JSON.
fn cmd_order(state: &mut State, config: &GameConfig, rest: &str) -> Result<(), String> {
    let mut t = rest.split_whitespace();
    let ship: u32 = t.next().and_then(|s| s.parse().ok()).ok_or("usage: order <ship> attack|guard|siege|move|dock|colonize|idle ...")?;
    let verb = t.next().ok_or("missing verb")?.to_string();
    let owner = state.ship(ship).map(|s| s.faction_id).ok_or_else(|| format!("no ship {ship}"))?;
    let behavior = match verb.as_str() {
        "attack" => {
            let target: u32 = t.next().and_then(|s| s.parse().ok()).ok_or("attack needs a target ship id")?;
            json!({"TargetShip": {"ship": target, "attack": true}})
        }
        "guard" => {
            let target: u32 = t.next().and_then(|s| s.parse().ok()).ok_or("guard needs a friendly ship id")?;
            json!({"TargetShip": {"ship": target, "attack": false}})
        }
        "siege" => {
            let city: u32 = t.next().and_then(|s| s.parse().ok()).ok_or("siege needs a city id")?;
            json!({"TargetSettlement": {"city": city, "bombard": true}})
        }
        "move" => {
            let x: f64 = t.next().and_then(|s| s.parse().ok()).ok_or("move needs x y")?;
            let y: f64 = t.next().and_then(|s| s.parse().ok()).ok_or("move needs x y")?;
            json!({"Move": {"position": [x, y]}})
        }
        "dock" => {
            let body: u32 = t.next().and_then(|s| s.parse().ok()).ok_or("dock needs a body id")?;
            json!({"Dock": {"body": body}})
        }
        "colonize" => {
            let body: u32 = t.next().and_then(|s| s.parse().ok()).ok_or("colonize needs a body id")?;
            json!({"Colonize": {"body": body}})
        }
        "none" => return Err("unknown verb 'none' (原地不动请用 idle 待命)".to_string()),
        "idle" => json!("Idle"),
        other => return Err(format!("unknown verb '{other}'")),
    };
    if t.next().is_some() {
        return Err("too many arguments".to_string());
    }
    let diff = json!({"control": [{"faction_id": owner, "ship_orders": [{"ship": ship, "behavior": behavior, "mode": "Player"}]}]});
    web::apply_patch(state, config, &diff).map_err(|e| e.to_string())?;
    emit(&control_for_faction(state, owner).to_string());
    Ok(())
}

/// Set a per-faction per-resource budget to a Player value.
/// `budget <faction> <resource> <value>`  -> investment_budget (建设建筑)
/// `build  <faction> <resource> <value>`  -> construction_budget (造舰)
fn cmd_budget(state: &mut State, config: &GameConfig, field: &str, rest: &str) -> Result<(), String> {
    let mut t = rest.split_whitespace();
    let fid: u32 = t.next().and_then(|s| s.parse().ok()).ok_or("usage: budget|build <faction> <resource> <value>")?;
    let resource = t.next().ok_or("missing resource key")?;
    let value: f64 = t.next().and_then(|s| s.parse().ok()).ok_or("missing numeric value")?;
    if t.next().is_some() {
        return Err("too many arguments".to_string());
    }
    let mut entry = serde_json::Map::new();
    entry.insert("resource".to_string(), json!(resource));
    entry.insert("value".to_string(), json!(value));
    entry.insert("mode".to_string(), json!("Player"));
    let mut faction = serde_json::Map::new();
    faction.insert("faction_id".to_string(), json!(fid));
    faction.insert(field.to_string(), json!([serde_json::Value::Object(entry)]));
    let diff = serde_json::Value::Object({
        let mut m = serde_json::Map::new();
        m.insert("control".to_string(), json!([serde_json::Value::Object(faction)]));
        m
    });
    web::apply_patch(state, config, &diff).map_err(|e| e.to_string())?;
    emit(&control_for_faction(state, fid).to_string());
    Ok(())
}

/// The built-in `summary` radar: a compact per-round overview.
const SUMMARY_JQ: &str = "{round, time_month, counts:{factions:(.factions|length), cities:(.cities|length), ships:(.ships|length)}, factions:[.factions[]|{id,name,wars}]}";

/// Round a float to 2 decimals for the machine-readable surface.
fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Build the `delta` value: a compact semantic diff of the world between two
/// states (`before` -> `after`). The agent's `events` log records *what
/// happened* in a round; `delta` records *what the state changed to* — the
/// structurally interesting deltas an agent would otherwise have to eyeball
/// across two full snapshots. Emitted as one JSON value (round + diffs).
///
/// Sections (only populated when non-empty):
///   * `new_ships` / `dead_ships`  — ships present in `after` but not `before`
///     (and vice versa), by id + class + owner + hull.
///   * `cities_changed`            — owner / razed / population changes.
///   * `resource_deltas`           — per-faction per-resource stockpile change,
///                                    with raw-key and 中文名 labels.
///   * `wars_changed`              — relations that crossed the war threshold.
fn delta_state(before: &State, after: &State, config: &GameConfig) -> serde_json::Value {
    let fname = |id: u32| after.faction(id).map(|f| f.name.clone()).unwrap_or_else(|| format!("#{id}"));

    // --- ships ----------------------------------------------------------------
    fn ship_map<'a>(s: &'a State) -> std::collections::HashMap<u32, &'a Ship> {
        s.ships.iter().map(|sh| (sh.id, sh)).collect()
    }
    let bmap = ship_map(before);
    let amap = ship_map(after);
    let mut new_ships = Vec::new();
    let mut dead_ships = Vec::new();
    for (id, ship) in &amap {
        if !bmap.contains_key(id) {
            new_ships.push(json!({
                "id": ship.id, "name": ship.name.clone(), "class": ship.class.clone(),
                "owner": fname(ship.faction_id), "hull": r2(ship.hull),
            }));
        }
    }
    for (id, ship) in &bmap {
        if !amap.contains_key(id) {
            dead_ships.push(json!({
                "id": ship.id, "name": ship.name.clone(), "class": ship.class.clone(),
                "owner": fname(ship.faction_id),
            }));
        }
    }

    // --- cities ---------------------------------------------------------------
    fn city_map<'a>(s: &'a State) -> std::collections::HashMap<u32, &'a City> {
        s.cities.iter().map(|c| (c.id, c)).collect()
    }
    let bcity = city_map(before);
    let acity = city_map(after);
    let mut cities_changed = Vec::new();
    for (id, city) in &acity {
        if let Some(prev) = bcity.get(id) {
            let mut change = serde_json::Map::new();
            let mut changed = false;
            change.insert("id".into(), json!(id));
            change.insert("name".into(), json!(city.name.clone()));
            if prev.faction_id != city.faction_id {
                change.insert("owner".into(), json!({
                    "from": fname(prev.faction_id), "to": fname(city.faction_id),
                }));
                changed = true;
            }
            if prev.razed != city.razed {
                change.insert("razed".into(), json!({"from": prev.razed, "to": city.razed}));
                changed = true;
            }
            if prev.population != city.population {
                change.insert("population".into(), json!({
                    "from": prev.population, "to": city.population,
                }));
                changed = true;
            }
            if changed {
                cities_changed.push(serde_json::Value::Object(change));
            }
        } else {
            // Did not exist before: new colony this round (events would also say so).
            cities_changed.push(json!({
                "id": id, "name": city.name.clone(), "owner": fname(city.faction_id), "new_colony": true,
            }));
        }
    }

    // --- resource stockpile deltas (per faction, per resource) ----------------
    let mut resource_deltas = Vec::new();
    for fac in &after.factions {
        let prev = before.faction(fac.id);
        let mut per = Vec::new();
        for (k, v) in &fac.resources {
            let beforev = prev.map(|p| {
                let mut found = 0.0;
                for (rk, rv) in &p.resources {
                    if rk.as_str() == k.as_str() {
                        found = *rv;
                        break;
                    }
                }
                found
            }).unwrap_or(0.0);
            let dv = *v - beforev;
            if dv.abs() >= 0.05 {
                per.push(json!({
                    "resource": k, "label": config.resource_name(k), "delta": r2(dv),
                }));
            }
        }
        if !per.is_empty() {
            resource_deltas.push(json!({"faction": fac.id, "name": fac.name, "deltas": per}));
        }
    }

    // --- wars crossed ---------------------------------------------------------
    let mut wars_changed = Vec::new();
    let hostile = |s: &State, a: u32, b: u32| -> bool {
        s.faction(a)
            .and_then(|f| f.relations.get(&b))
            .map(|rel| *rel <= config.combat.war_threshold)
            .unwrap_or(false)
    };
    for fac in &after.factions {
        for (other, rel) in &fac.relations {
            if *other <= fac.id {
                continue; // report each pair once
            }
            let was = hostile(before, fac.id, *other);
            let now = hostile(after, fac.id, *other);
            if was != now {
                wars_changed.push(json!({
                    "a": fac.name.clone(), "b": fname(*other),
                    "state": if now { "at_war" } else { "peace" },
                    "relation": r2(*rel),
                }));
            }
        }
    }

    json!({
        "round": after.round,
        "rounds_advanced": after.round.saturating_sub(before.round),
        "new_ships": new_ships,
        "dead_ships": dead_ships,
        "cities_changed": cities_changed,
        "resource_deltas": resource_deltas,
        "wars_changed": wars_changed,
    })
}

/// Write a line to stdout, ignoring broken-pipe errors so piping into `head`
/// (or an agent that closes the pipe early) exits quietly instead of panicking.
fn emit(s: &str) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{s}");
    let _ = out.flush();
}

/// Run a jq filter against the current agent state, printing each output on its
/// own line (JSON Lines). Errors go to stderr as structured JSON.
fn print_query(state: &State, config: &GameConfig, filter: &str) {
    match query::apply_lines(&agent::state_value(state, config), filter) {
        Ok(lines) => {
            if !lines.is_empty() {
                emit(&lines);
            }
        }
        Err(e) => eprintln!(
            "{}",
            json!({"ok": false, "code": "ERR_QUERY", "message": e.to_string()})
        ),
    }
}

/// Dump the game config (rules dictionary). With `filter` (Some), run a jq
/// filter over the meta value; otherwise emit the whole document as one line.
fn print_meta(config: &GameConfig, filter: Option<&str>) {
    let input = agent::meta_value(config);
    match filter {
        Some(f) => match query::apply_lines(&input, f) {
            Ok(lines) => {
                if !lines.is_empty() {
                    emit(&lines);
                }
            }
            Err(e) => eprintln!(
                "{}",
                json!({"ok": false, "code": "ERR_QUERY", "message": e.to_string()})
            ),
        },
        None => emit(&input.to_string()),
    }
}

/// Structured command catalog returned by `guide`/`help` (progressive discovery).
fn guide_json() -> String {
    json!({
        "schema_version": "1.2",
        "commands": {
            "q":  {"usage": "q <jq>",        "desc": "run a jq filter over the current state; JSON Lines out (alias query)"},
            "summary": {"usage": "summary",   "desc": "compact radar of the current round (alias s)"},
            "advance": {"usage": "advance [n]", "desc": "advance n rounds (default 1), then print summary (alias a)"},
            "control": {"usage": "control [<faction_id>|<jq>]", "desc": "dump the editable control surface. Bare → whole surface; control <id> → one faction (cheaper); control <jq> → filter the surface. The template you edit into a diff."},
            "meta": {"usage": "meta [<jq>]",  "desc": "dump the game config (resources raw-key→中文名, structures/buildings/ships specs, economy/combat/diplomacy tuning) — the rules dictionary. With a jq filter, filters the meta value."},
            "apply": {"usage": "apply <file.json>", "desc": "overlay a control diff file onto the state, then print the updated control surface. A ship behavior may be written in the default enum form ({\"TargetShip\":{...}}, \"Idle\") or the tagged state-view form ({\"type\":\"target_ship\",...}, {\"type\":\"idle\"}) — both are accepted."},
            "order": {"usage": "order <ship> attack|guard|siege|move|dock|colonize|idle ...", "desc": "one-shot ship command (Player mode). attack <enemy> chases+fires; guard <friend> escorts; dock <body> parks in orbit of a body (follows it); idle holds position. e.g. order 0 attack 3 | order 3 guard 1 | order 8 dock 9 | order 6 idle"},
            "budget": {"usage": "budget <faction> <resource> <value>", "desc": "set a faction's investment budget leaf (建设建筑) to a Player value"},
            "build":  {"usage": "build <faction> <resource> <value>",  "desc": "set a faction's construction budget leaf (造舰) to a Player value"},
            "events": {"usage": "events",     "desc": "print this round's event log (attacks, destroyed ships, razed cities, colonies, stale orders, wars started/ended, story beats)"},
            "story": {"usage": "story [<jq>]", "desc": "print the story chronicle (the unfolding narrative arc) — one beat per JSON line by default, or pipe through a jq filter (e.g. `story .[] | select(.round > 10)`). Each beat carries the round it fired, its id/title/body and participants."},
            "delta": {"usage": "delta [n]",   "desc": "advance n rounds (default 1) and print a compact semantic state diff over that window: new/destroyed ships, city owner/razed/population changes, per-faction resource stockpile deltas, and wars that crossed the threshold. Complements `events` (what happened) with `delta` (what the state changed to)."},
            "save": {"usage": "save <file.ron>", "desc": "write a deterministic checkpoint: the current State plus the PRNG position. Resume later with `load` here or `--start <file>` in a new process; a resumed run reproduces the same future rounds."},
            "load": {"usage": "load <file.ron>", "desc": "replace the in-memory state with a checkpoint saved by `save` / `--save`, restoring the RNG position too (alias resume)."},
            "cities": {"usage": "cities",     "desc": "list cities"},
            "ships":  {"usage": "ships",      "desc": "list ships"},
            "factions": {"usage": "factions", "desc": "list factions"},
            "bodies": {"usage": "bodies",     "desc": "list bodies"},
            "city": {"usage": "city <id>",    "desc": "detail one city"},
            "ship": {"usage": "ship <id>",    "desc": "detail one ship"},
            "faction": {"usage": "faction <id>", "desc": "detail one faction"},
            "body": {"usage": "body <id>",    "desc": "detail one body"},
            "guide": {"usage": "guide",       "desc": "this command catalog"},
            "quit": {"usage": "quit|exit",    "desc": "end the session"}
        },
        "errors": {
            "ERR_QUERY": {"exit_code": 10, "desc": "jq parse/runtime error"},
            "ERR_BAD_ID": {"exit_code": 10, "desc": "non-numeric entity id"},
            "ERR_APPLY": {"exit_code": 10, "desc": "control diff read/parse/apply error"},
            "ERR_SCRIPT": {"exit_code": 10, "desc": "cannot open the --script file"}
        }
    })
    .to_string()
}

fn run_agent_repl(state: &mut State, config: &GameConfig, rng: &mut Prng, input: &mut dyn BufRead) {
    // Intro: a summary radar so the agent starts oriented (state lives in-process).
    print_query(state, config, SUMMARY_JQ);

    loop {
        eprint!("agent> ");
        io::stderr().flush().ok();
        let mut line = String::new();
        let n = input.read_line(&mut line);
        if n.is_err() || n.unwrap_or(0) == 0 {
            break;
        }
        // Normalize: trim CRLF/whitespace, then drop a single leading UTF-8 BOM
        // (a piped byte stream or a script file may begin with one). Blank lines
        // and `#` comments are skipped so scripts stay readable.
        let mut cmd = line.trim();
        if let Some(stripped) = cmd.strip_prefix('\u{feff}') {
            cmd = stripped.trim_start();
        }
        if cmd.is_empty() || cmd.starts_with('#') {
            continue;
        }
        let (head, rest) = cmd
            .split_once(|c: char| c.is_whitespace())
            .unwrap_or((cmd, ""));
        let rest = rest.trim();

        match head {
            // --- query pipeline -------------------------------------------------
            "q" | "query" => {
                if rest.is_empty() {
                    emit(&guide_json());
                } else {
                    print_query(state, config, rest);
                }
            }
            "s" | "summary" => print_query(state, config, SUMMARY_JQ),

            // --- turn control ---------------------------------------------------
            "a" | "advance" => {
                let n: u32 = rest.parse().unwrap_or(1);
                for _ in 0..n {
                    sim::advance(state, config, rng);
                }
                print_query(state, config, SUMMARY_JQ);
            }
            "delta" | "diff" => {
                let n: u32 = rest.parse().unwrap_or(1);
                let before = state.clone();
                for _ in 0..n {
                    sim::advance(state, config, rng);
                }
                emit(&delta_state(&before, state, config).to_string());
            }

            // --- editable control surface --------------------------------------
            "control" => {
                if rest.is_empty() {
                    emit(&web::control_surface(state).to_string());
                } else if let Ok(fid) = rest.parse::<u32>() {
                    emit(&control_for_faction(state, fid).to_string());
                } else {
                    match query::apply_lines(&web::control_surface(state), rest) {
                        Ok(lines) => {
                            if !lines.is_empty() {
                                emit(&lines);
                            }
                        }
                        Err(e) => eprintln!(
                            "{}",
                            json!({"ok": false, "code": "ERR_QUERY", "message": e.to_string()})
                        ),
                    }
                }
            }
            "meta" => print_meta(config, if rest.is_empty() { None } else { Some(rest) }),
            "apply" => {
                if rest.is_empty() {
                    eprintln!(
                        "{}",
                        json!({"ok": false, "code": "ERR_ARGS", "message": "usage: apply <file.json>"})
                    );
                } else {
                    match apply_diff(state, Path::new(rest)) {
                        Ok(()) => emit(&web::control_surface(state).to_string()),
                        Err(e) => eprintln!("{}", json!({"ok": false, "code": "ERR_APPLY", "message": e})),
                    }
                }
            }

            // --- high-level command shortcuts ----------------------------------
            "events" => print_query(state, config, ".events"),
            "story" => {
                // 剧情编年史：每条叙事事件一行 JSON（默认），可跟 jq 过滤整段弧。
                let input = agent::story_value(state);
                let filter = if rest.is_empty() { ".[]" } else { rest };
                match query::apply_lines(&input, filter) {
                    Ok(lines) => {
                        if !lines.is_empty() {
                            emit(&lines);
                        }
                    }
                    Err(e) => eprintln!(
                        "{}",
                        json!({"ok": false, "code": "ERR_QUERY", "message": e.to_string()})
                    ),
                }
            }
            "save" | "save_state" => {
                if rest.is_empty() {
                    eprintln!(
                        "{}",
                        json!({"ok": false, "code": "ERR_ARGS", "message": "usage: save <file.ron>"})
                    );
                } else {
                    match save_checkpoint(Path::new(rest), state, rng) {
                        Ok(()) => emit(&json!({"ok": true, "saved": rest, "round": state.round}).to_string()),
                        Err(e) => eprintln!("{}", json!({"ok": false, "code": "ERR_SAVE", "message": e})),
                    }
                }
            }
            "load" | "resume" => {
                if rest.is_empty() {
                    eprintln!(
                        "{}",
                        json!({"ok": false, "code": "ERR_ARGS", "message": "usage: load <file.ron>"})
                    );
                } else {
                    match load_checkpoint(Path::new(rest)) {
                        Ok((st, rg)) => {
                            *state = st;
                            *rng = rg;
                            print_query(state, config, SUMMARY_JQ);
                        }
                        Err(e) => eprintln!("{}", json!({"ok": false, "code": "ERR_LOAD", "message": e})),
                    }
                }
            }
            "order" => {
                if let Err(e) = cmd_order(state, config, rest) {
                    eprintln!("{}", json!({"ok": false, "code": "ERR_ARGS", "message": e}));
                }
            }
            "budget" => {
                if let Err(e) = cmd_budget(state, config, "investment_budget", rest) {
                    eprintln!("{}", json!({"ok": false, "code": "ERR_ARGS", "message": e}));
                }
            }
            "build" => {
                if let Err(e) = cmd_budget(state, config, "construction_budget", rest) {
                    eprintln!("{}", json!({"ok": false, "code": "ERR_ARGS", "message": e}));
                }
            }

            // --- entity lists ---------------------------------------------------
            "cities" => print_query(state, config, ".cities[] | {id,name,body,owner_name,population,razed,armor,ship_progress}"),
            "ships" => print_query(state, config, ".ships[] | {id,name,class,owner_name,position,hull,hull_max,order}"),
            "factions" => print_query(state, config, ".factions[] | {id,name,resources,wars}"),
            "bodies" => print_query(state, config, ".bodies[] | {id,name,position,settlements:[.settlements[]|{name,total_area}]}"),

            // --- entity detail --------------------------------------------------
            "city" | "ship" | "faction" | "body" => {
                let entity = match head {
                    "city" => "cities",
                    "ship" => "ships",
                    "faction" => "factions",
                    _ => "bodies",
                };
                match rest.parse::<u32>() {
                    Ok(id) => print_query(state, config, &format!(".{entity}[] | select(.id == {id})")),
                    Err(_) => eprintln!(
                        "{}",
                        json!({"ok": false, "code": "ERR_BAD_ID", "message": "expected a numeric id"})
                    ),
                }
            }

            // --- discovery / exit ------------------------------------------------
            "guide" | "help" => emit(&guide_json()),
            "quit" | "exit" | "bye" => break,
            _ => eprintln!(
                "{}",
                json!({"ok": false, "code": "ERR_UNKNOWN_CMD", "message": format!("unknown command '{head}' (use 'guide')")})
            ),
        }
    }
}

fn run_rounds(state: &mut State, config: &GameConfig, rng: &mut Prng, n: u32, save: Option<&Path>) {
    // Agent-only mode: zero-noise JSON Lines on stdout, one object per round
    // (round 0 first, then one per round as the simulation advances).
    emit(&agent::render_state(state, config));
    for _ in 0..n {
        sim::advance(state, config, rng);
        emit(&agent::render_state(state, config));
    }
    // Optionally persist a deterministic checkpoint (state + RNG position).
    if let Some(path) = save {
        if let Err(e) = save_checkpoint(path, state, rng) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_SAVE", "message": e.to_string()}));
            std::process::exit(10);
        }
    }
}
