//! Shared loading helpers: the game config (`config/game.ron`), an optional
//! initial-state RON file, and seed parsing. Used by both the CLI and the web
//! server so the two entry points behave identically.

use crate::model::{migrate, GameConfig, RoundState, State};
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

/// Load and parse the `GameConfig` from an explicit path (no `std::process::exit`,
/// no dependency on the process cwd). [`load_config`] is the exit-on-error wrapper
/// the binaries use; tests (e.g. in the `planet_x_web` crate, whose cwd is `web/`)
/// use this one with `CARGO_MANIFEST_DIR`-anchored paths.
pub fn load_config_from(path: &Path) -> Result<GameConfig, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("无法读取配置文件 {}: {e}", path.display()))?;
    ron::from_str(&text).map_err(|e| format!("配置文件 {} 解析失败: {e}", path.display()))
}

/// Load and parse the `GameConfig` from the config path. Exits on error.
pub fn load_config() -> GameConfig {
    let path = config_path();
    load_config_from(&path).unwrap_or_else(|e| {
        eprintln!("{} {e}", "[错误]".red().bold());
        eprintln!("请提供 config/game.ron，或设置 PLANET_X_CONFIG 环境变量。");
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

/// A serializable session checkpoint: the full [`RoundState`] (canonical world +
/// this round's `pre`/`post` derived records) plus the exact PRNG position, so a
/// saved run can be resumed deterministically.
#[derive(Serialize, Deserialize)]
pub struct Checkpoint {
    pub prng_state: u64,
    pub round_state: RoundState,
}

/// Serialize the current `State` to a RON file (no RNG position). Use
/// [`save_checkpoint`] when the run must resume deterministically.
pub fn save_state(path: &Path, state: &State) -> Result<(), String> {
    let text = ron::to_string(state).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Serialize a session checkpoint (round_state + RNG position) to a RON file.
pub fn save_checkpoint(path: &Path, round_state: &RoundState, rng: &Prng) -> Result<(), String> {
    let cp = Checkpoint { prng_state: rng.state(), round_state: round_state.clone() };
    let text = ron::to_string(&cp).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Load a checkpoint, returning the [`RoundState`] and the resumable [`Prng`].
pub fn load_checkpoint(path: &Path) -> Result<(RoundState, Prng), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut cp: Checkpoint = ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    // 显式迁移到当前 schema 版本；无法迁移/版本过新则报错。
    migrate(&mut cp.round_state.state)?;
    // 包装层（`RoundState`）自己那个版本号也跟着推到当前值：它以前只是**写出去**、从没被校准，
    // 于是「旧档 + 新二进制」读完会出现 `state.schema_version = 10` 而包装层还写着 9 ——
    // 一个只在 `--save` 之后才被发现的自相矛盾。迁移的语义仍以 `State` 为准（它才是被迁移的那个）。
    cp.round_state.schema_version = crate::model::SCHEMA_VERSION;
    Ok((cp.round_state, Prng::from_state(cp.prng_state)))
}

/// `--start` style initial load: prefers a session checkpoint ([`RoundState`] + RNG),
/// and falls back to a bare `State` RON file (fresh RNG from `seed`). Returns the
/// canonical `State` and the resumable RNG; the checkpoint's `pre`/`post` derived records
/// are ignored for continuation (the next `advance` recomputes its own `pre`).
pub fn load_initial(path: &Path, seed: u64) -> (State, Prng) {
    if let Ok((round_state, rng)) = load_checkpoint(path) {
        return (round_state.state, rng);
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

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;

