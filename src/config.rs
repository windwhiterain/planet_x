//! Shared loading helpers: the game config (`config/game.ron`), an optional
//! initial-state RON file, and seed parsing. Used by both the CLI and the web
//! server so the two entry points behave identically.

use crate::model::{migrate, GameConfig, State};
use crate::prng::{self, Prng};
use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Config file path: `$PLANET_X_CONFIG` or the default `config/game.ron`.
pub fn config_path() -> PathBuf {
    std::env::var_os("PLANET_X_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config/game.ron"))
}

/// Load and parse the `GameConfig` from the config path. Exits on error.
pub fn load_config() -> GameConfig {
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

/// Load a `State` initial snapshot from a RON file. Exits on error.
pub fn load_state(path: &Path) -> State {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{} 无法读取初始状态 {}: {e}", "[错误]".red().bold(), path.display());
        std::process::exit(1);
    });
    let mut state: State = ron::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{} 初始状态 {} 解析失败: {e}", "[错误]".red().bold(), path.display());
        std::process::exit(1);
    });
    // 显式迁移到当前 schema 版本；无法迁移/版本过新则报错退出，而不是静默错载。
    if let Err(e) = migrate(&mut state) {
        eprintln!("{} 初始状态 {} 迁移失败: {e}", "[错误]".red().bold(), path.display());
        std::process::exit(1);
    }
    state
}

/// A serializable session checkpoint: the full `State` plus the exact PRNG
/// position, so a saved run can be resumed deterministically.
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    pub prng_state: u64,
    pub state: State,
}

/// Serialize the current `State` to a RON file (no RNG position). Use
/// [`save_checkpoint`] when the run must resume deterministically.
pub fn save_state(path: &Path, state: &State) -> Result<(), String> {
    let text = ron::to_string(state).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Serialize a session checkpoint (state + RNG position) to a RON file.
pub fn save_checkpoint(path: &Path, state: &State, rng: &Prng) -> Result<(), String> {
    let cp = Checkpoint { prng_state: rng.state(), state: state.clone() };
    let text = ron::to_string(&cp).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Load a checkpoint, returning the `State` and the resumable `Prng`.
pub fn load_checkpoint(path: &Path) -> Result<(State, Prng), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut cp: Checkpoint = ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    // 显式迁移到当前 schema 版本；无法迁移/版本过新则报错。
    migrate(&mut cp.state)?;
    Ok((cp.state, Prng::from_state(cp.prng_state)))
}

/// `--start` style initial load: prefers a session checkpoint (state + RNG), and
/// falls back to a bare `State` RON file (fresh RNG from `seed`).
pub fn load_initial(path: &Path, seed: u64) -> (State, Prng) {
    if let Ok((state, rng)) = load_checkpoint(path) {
        return (state, rng);
    }
    // Not a checkpoint; treat as a bare initial state.
    (load_state(path), Prng::new(seed))
}

/// Parse a `--seed` value: a number, or `random` (case-insensitive).
pub fn parse_seed(v: &str) -> u64 {
    if v.eq_ignore_ascii_case("random") {
        return prng::random_seed();
    }
    v.trim().parse::<u64>().unwrap_or_else(|_| {
        eprintln!("{} 无效的 --seed 值: {v} （用数字或 'random'）", "[错误]".red().bold());
        std::process::exit(2);
    })
}
