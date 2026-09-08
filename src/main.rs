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
//! a `--script` file (`q <jq>`, `summary`, `advance [n]`, `control`, `apply
//! <file>`, `cities`, `city <id>`, `ships`, `faction <id>`, `bodies`, `guide`,
//! `quit` — plus aliases `s`/`a`/`q`), each returning a JSON value —
//! hierarchical, on-demand access instead of a per-round full dump. `control`
//! emits the editable control surface, and `apply <file>` overlays a diff onto
//! the state mid-session.

use clap::Parser;
use planet_x::agent;
use planet_x::config::{load_config, load_state, parse_seed};
use planet_x::model::{GameConfig, State};
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
  按轨道重算。部分天体有定居点 settlement：含有限总面积（settlement_area）、生态容量（人口/面积）、\n\
  建设速度修正、建设资源修正，以及若干资源矿藏（类型 + 面积，限定采矿上限）。\n\
- 城市 city：建在定居点上、由一个势力控制，内有若干连续面积分配的 建筑，并有人口（限制生产效率）、\n\
  防御值 defense（围城伤害累积于此，被攻占后重置）和一条造船队列 ship_build（建造点面积推动进度）。\n\
- 建筑 building：非原子，是一个连续面积分配（计划 area 与实际 deployed），总和不超定居点总面积。三种角色：\n\
  residential 居住点（提供人口容量）、mining 开采点（采对应矿藏资源）、construction 建造点（船坞，造舰）。\n\
- 势力 faction：拥有城市与飞船，库存各资源，并与其它势力两两外交（关系 relations）。\n\
- 飞船 ship：必属某一势力，从城市出厂，在 2D 平面移动，可按指令开火/围城。舰级参数（护甲/攻击/速度/\n\
  攻击距离/建造点/建造成本）由 config 定义：护卫舰 corvette、巡洋舰 cruiser、运输舰 transport。\n\
\n\
【资源】11 种：水冰 water_ice、氦-3 helium3、铀 uranium、钍 thorium、金 gold、铂 platinum、铁 iron、\n\
氢 hydrogen、甲烷 methane、碳 carbon、硅 silicon。\n\
\n\
【每回合演化（sim::advance）】\n\
1. 天体位置按轨道重算。\n\
2. 经济：开采点按 面积×劳动力×生产率 产出资源；人口向住房容量增长（增长比例受 pop_growth 控制）。\n\
3. 建设：建筑按各自投资权重竞争本轮资源预算（默认每资源最多拿出库存的 invest_fraction=0.3 投入建设）；\n\
   连续地把计划面积建成 deployed；船坞按面积×劳动力 累积造船进度，够了就付建造成本造出新舰。\n\
4. 军事：每艘舰按指令 移动/开火/围城。开火削减目标舰 hull；围城削减城市 defense；defense≤0 则城市被\n\
   攻占（守备重置、人口×0.6、改由攻击方控制）。关系 ≤ 战争阈值（war_threshold=-20）即视为敌对（wars）。\n\
5. 外交：攻击/占领会加重敌对（attack_delta/capture_delta）；非战争关系每回合向中性回落（relax_rate）。\n\
\n\
【控制模型 = 指令】每个势力有一份可控状态 State::control：\n\
- ship_orders：本方各舰的 行为（ShipBehavior）：Idle（待命）、Move{position}（前往某位置）、\n\
  TargetShip{ship,attack}（追袭某舰，attack 表示是否开火）、TargetSettlement{city,bombard}（围攻某城）。\n\
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

    // Initial state: from a file, or generated procedurally.
    let mut state = match &cli.start {
        Some(path) => load_state(path),
        None => world::default_state(&config, seed),
    };

    // Apply a control-state diff (agent instructions) before anything else.
    if let Some(path) = &cli.apply {
        if let Err(e) = apply_diff(&mut state, path) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_APPLY", "message": e}));
            std::process::exit(10);
        }
    }

    let mut rng = Prng::new(seed);

    // Batch query: run a jq filter over the agent state and print JSON Lines.
    // Honors --round N by advancing that many rounds first, so the filter runs
    // against the end-of-sim state. Takes precedence over interactive/REPL.
    if let Some(filter) = &cli.query {
        let n = cli.round.unwrap_or(0);
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
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
        Some(n) => run_rounds(&mut state, &config, &mut rng, n),
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
    web::apply_patch(state, &value)
}

/// The built-in `summary` radar: a compact per-round overview.
const SUMMARY_JQ: &str = "{round, time_month, counts:{factions:(.factions|length), cities:(.cities|length), ships:(.ships|length)}, factions:[.factions[]|{id,name,wars}]}";

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
        "schema_version": "1.1",
        "commands": {
            "q":  {"usage": "q <jq>",        "desc": "run a jq filter over the current state; JSON Lines out (alias query)"},
            "summary": {"usage": "summary",   "desc": "compact radar of the current round (alias s)"},
            "advance": {"usage": "advance [n]", "desc": "advance n rounds (default 1), then print summary (alias a)"},
            "control": {"usage": "control",   "desc": "dump the editable control surface (control+scope) as JSON — the template to edit into a diff"},
            "meta": {"usage": "meta [<jq>]",  "desc": "dump the game config (resources raw-key→中文名, buildings/ships specs, economy/combat/diplomacy tuning) — the rules dictionary. With a jq filter, filters the meta value."},
            "apply": {"usage": "apply <file.json>", "desc": "overlay a control diff file onto the state, then print the updated control surface"},
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

            // --- editable control surface --------------------------------------
            "control" => emit(&web::control_surface(state).to_string()),
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

            // --- entity lists ---------------------------------------------------
            "cities" => print_query(state, config, ".cities[] | {id,name,body,owner_name,population,defense,building}"),
            "ships" => print_query(state, config, ".ships[] | {id,name,class,owner_name,position,hull,hull_max,order}"),
            "factions" => print_query(state, config, ".factions[] | {id,name,resources,wars}"),
            "bodies" => print_query(state, config, ".bodies[] | {id,name,position,settlement_area}"),

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

fn run_rounds(state: &mut State, config: &GameConfig, rng: &mut Prng, n: u32) {
    // Agent-only mode: zero-noise JSON Lines on stdout, one object per round
    // (round 0 first, then one per round as the simulation advances).
    emit(&agent::render_state(state, config));
    for _ in 0..n {
        sim::advance(state, config, rng);
        emit(&agent::render_state(state, config));
    }
}
