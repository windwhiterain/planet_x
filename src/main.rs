//! planet_x — a sandbox trajectory generator for the Planet X game.
//!
//! Usage: `planet_x [--seed <random>] [--start <path.ron>] [--round <n>]`
//!
//! * `--seed`   the deterministic RNG seed, or `random` (the default).
//! * `--start`  load an initial `State` from a RON file; otherwise the default
//!              procedural solar system is generated.
//! * `--round`  run `n` rounds and dump each state snapshot (including the
//!              per-faction controllable state `State::control`) to a `.ron`
//!              file under `trajectory/`.
//!
//! Without `--round` the tool runs interactively: every Enter advances one
//! round and prints the new state in the CLI. After each round the per-faction
//! controllable-state diff (指令 = 对可控制状态的修改) is sparsely printed —
//! in interactive mode below the state, and in `--round` mode inline.

mod model;
mod prng;
mod sim;
mod visual;
mod world;

use clap::Parser;
use colored::Colorize;
use model::{GameConfig, State};
use prng::Prng;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "planet_x",
    version,
    about = "行星X——太空沙盘轨迹生成器",
    after_help = "回合由 Enter 推进；--round 会将每回合状态写入 trajectory/*.ron。每回合后稀疏打印各势力可控状态 diff（= 指令）。"
)]
struct Cli {
    /// 确定性随机种子（数字，或 random / 随机）
    #[arg(long, default_value = "random", value_name = "SEED")]
    seed: String,

    /// 从指定的初始状态 .ron 文件开始
    #[arg(long, value_name = "PATH")]
    start: Option<PathBuf>,

    /// 运行 n 个回合并把各回合状态写成 .ron 轨迹文件，而非交互式推进
    #[arg(long, value_name = "N")]
    round: Option<u32>,
}

fn main() {
    // Colors only when emitting to a terminal (keeps redirected output clean).
    if !io::stdout().is_terminal() {
        colored::control::set_override(false);
    }

    let cli = Cli::parse();

    let config = load_config();
    let seed = parse_seed(&cli.seed);

    // Initial state: from a file, or generated procedurally.
    let mut state = match &cli.start {
        Some(path) => load_state(path),
        None => world::default_state(&config, seed),
    };

    println!("{}", "行星X 沙盘轨迹生成器".bold().cyan());
    println!("  种子  : {} -> {}", cli.seed, seed);
    println!("  配置  : {}", config_path().display());
    println!("  初始  : {}", cli.start.map_or_else(|| "程序化生成".to_string(), |p| p.display().to_string()));

    let mut rng = Prng::new(seed);

    match cli.round {
        Some(n) => run_rounds(&mut state, &config, &mut rng, n, seed),
        None => run_interactive(&mut state, &config, &mut rng),
    }
}

fn config_path() -> PathBuf {
    std::env::var_os("PLANET_X_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/game.ron"))
}

fn load_config() -> GameConfig {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        eprintln!("{} 无法读取配置文件 {}: {e}", "[错误]".red().bold(), path.display());
        eprintln!("请提供 config/game.ron，或设置 PLANET_X_CONFIG 环境变量。");
        std::process::exit(1);
    });
    ron::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{} 配置文件 {} 解析失败: {e}", "[错误]".red().bold(), path.display());
        std::process::exit(1);
    })
}

fn load_state(path: &Path) -> State {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{} 无法读取初始状态 {}: {e}", "[错误]".red().bold(), path.display());
        std::process::exit(1);
    });
    ron::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{} 初始状态 {} 解析失败: {e}", "[错误]".red().bold(), path.display());
        std::process::exit(1);
    })
}

fn parse_seed(v: &str) -> u64 {
    if v.eq_ignore_ascii_case("random") {
        return prng::random_seed();
    }
    v.trim().parse::<u64>().unwrap_or_else(|_| {
        eprintln!("{} 无效的 --seed 值: {v} （用数字或 'random'）", "[错误]".red().bold());
        std::process::exit(2);
    })
}

fn print_state(state: &State, config: &GameConfig) {
    println!("{}", visual::render_map(state));
    println!("{}", visual::render_summary(state, config));
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

fn run_rounds(state: &mut State, config: &GameConfig, rng: &mut Prng, n: u32, seed: u64) {
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
