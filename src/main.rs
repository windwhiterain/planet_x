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

/// Structured command catalog returned by `guide`/`help` (progressive discovery).
fn guide_json() -> String {
    json!({
        "schema_version": "1.1",
        "commands": {
            "q":  {"usage": "q <jq>",        "desc": "run a jq filter over the current state; JSON Lines out (alias query)"},
            "summary": {"usage": "summary",   "desc": "compact radar of the current round (alias s)"},
            "advance": {"usage": "advance [n]", "desc": "advance n rounds (default 1), then print summary (alias a)"},
            "control": {"usage": "control",   "desc": "dump the editable control surface (control+scope) as JSON — the template to edit into a diff"},
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
