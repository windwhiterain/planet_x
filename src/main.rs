//! planet_x — a sandbox trajectory generator for the Planet X game.
//!
//! Usage: `planet_x [--seed <random>] [--start <path.ron>] [--round <n>] [--agent] [--query <jq>]`
//!
//! * `--seed`   the deterministic RNG seed, or `random` (the default).
//! * `--start`  load an initial `State` from a RON file; otherwise the default
//!              procedural solar system is generated.
//! * `--round`  run `n` rounds and dump each state snapshot (including the
//!              per-faction controllable state `State::control`) to a `.ron`
//!              file under `trajectory/`.
//! * `--agent`  zero-noise machine output for an LLM agent player: JSON Lines,
//!              no colours, star map, tables or prose.
//! * `--query`  run a jq filter over the agent state and print JSON Lines. With
//!              `--round N` it advances N rounds first, so filters run against
//!              the end-of-sim state (e.g. the surviving marines or captured
//!              cities). Useful for one-shot, composable pipelines.
//!
//! `--agent` without `--round` enters a **query REPL**: commands over stdin
//! (`q <jq>`, `summary`, `advance [n]`, `cities`, `city <id>`, `ships`,
//! `faction <id>`, `bodies`, `guide`, `quit`), each returning a JSON value —
//! hierarchical, on-demand access instead of a per-round full dump.
//!
//! Without `--round`/`--agent` the tool runs interactively: every Enter advances
//! one round and prints the new state in the CLI. After each round the
//! per-faction controllable-state diff (指令 = 对可控制状态的修改) is sparsely
//! printed — in interactive mode below the state, and in `--round` mode inline.

use clap::Parser;
use colored::Colorize;
use planet_x::agent;
use planet_x::config::{self, load_config, load_state, parse_seed};
use planet_x::model::{GameConfig, State};
use planet_x::prng::Prng;
use planet_x::{query, sim, visual, world};
use serde_json::json;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "planet_x",
    version,
    about = "行星X——太空沙盘轨迹生成器",
    after_help = "回合由 Enter 推进；--round 会将每回合状态写入 trajectory/*.ron。每回合后稀疏打印各势力可控状态 diff（= 指令）。--agent 则输出零噪声 JSON Lines 供 LLM agent 读取。"
)]
struct Cli {
    /// 确定性随机种子（数字，或 random / 随机）
    #[arg(long, default_value = "random", value_name = "SEED")]
    seed: String,

    /// 从指定的初始状态 .ron 文件开始
    #[arg(long, value_name = "PATH")]
    start: Option<PathBuf>,

    /// 运行 n 个回合并把各回合状态写成 .ron 轨迹文件，而非交互式推进
    #[arg(long = "round", visible_alias = "rounds", value_name = "N")]
    round: Option<u32>,

    /// 面向 LLM agent 的零噪声输出：stdout 只输出 JSON Lines，
    /// 无颜色/星图/表格/中文散文；浮点四舍五入到 2 位。
    #[arg(long)]
    agent: bool,

    /// (agent/query) 对初始(或 --start 加载的)状态执行一个 jq 过滤并输出
    /// JSON Lines 结果。例如: `planet_x --agent --query '.cities[] | select(.owner_name=="中国") | {name,population}'`
    #[arg(long, value_name = "JQ")]
    query: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    // Colors only when emitting to a terminal (keeps redirected output clean),
    // and never in agent mode (zero-noise machine output).
    if cli.agent || !io::stdout().is_terminal() {
        colored::control::set_override(false);
    }

    let config = load_config();
    let seed = parse_seed(&cli.seed);

    // Initial state: from a file, or generated procedurally.
    let mut state = match &cli.start {
        Some(path) => load_state(path),
        None => world::default_state(&config, seed),
    };

    if !cli.agent {
        println!("{}", "行星X 沙盘轨迹生成器".bold().cyan());
        println!("  种子  : {} -> {}", cli.seed, seed);
        println!("  配置  : {}", config::config_path().display());
        println!("  初始  : {}", cli.start.map_or_else(|| "程序化生成".to_string(), |p| p.display().to_string()));
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
        Some(n) => run_rounds(&mut state, &config, &mut rng, n, seed, cli.agent),
        None if cli.agent => run_agent_repl(&mut state, &config, &mut rng),
        None => run_interactive(&mut state, &config, &mut rng),
    }
}

fn print_state(state: &State, config: &GameConfig) {
    println!("{}", visual::render_map(state));
    println!("{}", visual::render_summary(state, config));
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
        "schema_version": "1.0",
        "commands": {
            "q":  {"usage": "q <jq>",        "desc": "run a jq filter over the current state; JSON Lines out"},
            "summary": {"usage": "summary",   "desc": "compact radar of the current round"},
            "advance": {"usage": "advance [n]", "desc": "advance n rounds (default 1), then print summary"},
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
            "ERR_BAD_ID": {"exit_code": 10, "desc": "non-numeric entity id"}
        }
    })
    .to_string()
}

fn run_agent_repl(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    // Intro: a summary radar so the agent starts oriented (state lives in-process).
    print_query(state, config, SUMMARY_JQ);

    let stdin = io::stdin();
    let mut handle = stdin.lock();
    loop {
        eprint!("agent> ");
        io::stderr().flush().ok();
        let mut line = String::new();
        let n = handle.read_line(&mut line);
        if n.is_err() || n.unwrap_or(0) == 0 {
            break;
        }
        let cmd = line.trim();
        if cmd.is_empty() {
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
            "summary" => print_query(state, config, SUMMARY_JQ),

            // --- turn control ---------------------------------------------------
            "advance" => {
                let n: u32 = rest.parse().unwrap_or(1);
                for _ in 0..n {
                    sim::advance(state, config, rng);
                }
                print_query(state, config, SUMMARY_JQ);
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

fn run_interactive(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    print_state(state, config);
    let stdin = io::stdin();
    let mut handle = stdin.lock();
    loop {
        print!("\n{} ", "[回车=下一回合, q=退出] >".bright_black());
        io::stdout().flush().ok();
        let mut line = String::new();
        let n = handle.read_line(&mut line);
        if n.is_err() || n.unwrap_or(0) == 0 {
            break;
        }
        let cmd = line.trim().to_lowercase();
        if cmd == "q" || cmd == "quit" || cmd == "exit" {
            break;
        }
        let before = state.control.clone();
        sim::advance(state, config, rng);
        println!("\n{}", "─".repeat(72).dimmed());
        print_state(state, config);
        // 各势力可控状态 diff（指令）稀疏打印在 state 下方。
        println!("{}", visual::render_control_diff(&before, &state.control, state, config));
    }
    println!();
}

fn run_rounds(state: &mut State, config: &GameConfig, rng: &mut Prng, n: u32, seed: u64, agent_mode: bool) {
    // Agent mode: zero-noise JSON Lines on stdout, no trajectory files, no
    // banner (already suppressed upstream). Round 0 first, then one per round.
    if agent_mode {
        emit(&agent::render_state(state, config));
        for _ in 0..n {
            sim::advance(state, config, rng);
            emit(&agent::render_state(state, config));
        }
        return;
    }

    use ron::ser::PrettyConfig;

    let dir = PathBuf::from("trajectory");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{} 无法创建输出目录 {}: {e}", "[错误]".red().bold(), dir.display());
        std::process::exit(1);
    }

    let write_state = |state: &State, round: u32| -> io::Result<()> {
        let text = ron::ser::to_string_pretty(state, PrettyConfig::default())
            .map_err(|e| io::Error::other(e.to_string()))?;
        let path = dir.join(format!("state_round_{:04}.ron", round));
        std::fs::write(path, text)
    };

    // Round 0 is the start state.
    if let Err(e) = write_state(state, 0) {
        eprintln!("{} 写出初始状态失败: {e}", "[错误]".red().bold());
        std::process::exit(1);
    }

    let mut idx = 0u32;
    for _ in 0..n {
        let before = state.control.clone();
        sim::advance(state, config, rng);
        idx += 1;
        if let Err(e) = write_state(state, idx) {
            eprintln!("{} 写出回合 {} 失败: {e}", "[错误]".red().bold(), idx);
            std::process::exit(1);
        }
        // 指令 = 可控状态 diff：稀疏打印（无变化则打印空）。
        let diff = visual::render_control_diff(&before, &state.control, state, config);
        if diff.is_empty() {
            println!("[回合 {idx}] （各势力可控状态无变化）");
        } else {
            println!("[回合 {idx}]{}", diff);
        }
    }

    // Reproducibility record.
    if let Err(e) = std::fs::write(dir.join("seed.txt"), format!("{seed}\n")) {
        eprintln!("{} 写出 seed.txt 失败: {e}", "[错误]".red().bold());
    }

    println!();
    println!(
        "{} 已生成 {n} 个回合的轨迹（含回合 0 初始状态，共 {} 个快照）",
        "[完成]".green().bold(),
        idx + 1
    );
    println!("输出目录: {}", dir.display());
}
