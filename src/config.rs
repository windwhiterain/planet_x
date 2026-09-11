//! Shared loading helpers: the game config (`config/game.ron`), an optional
//! initial-state RON file, and seed parsing. Used by both the CLI and the web
//! server so the two entry points behave identically.

use crate::model::{GameConfig, RoundState, State, migrate};
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
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("无法读取配置文件 {}: {e}", path.display()))?;
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
/// 裸 `State` 档（没有 `prng_state`/`round_state` 包装的那种）的**可恢复**读法。
///
/// 与 [`load_checkpoint`] 一样按扩展名选格式（`.json` ⇒ JSON）。给调用方留一条「两个错都能报出来」
/// 的路：`--start` 先按 checkpoint 读，失败了再按裸 state 读，**两个错都要说**——
/// 否则一个格式错会被兜底路径写成「Expected opening `(` for struct State」（RON 的报错），
/// 与真正的原因毫无关系。
pub fn load_state_checked(path: &Path) -> Result<State, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("无法读取 {}: {e}", path.display()))?;
    let mut state: State = match CheckpointFormat::of(path) {
        CheckpointFormat::Ron => ron::from_str(&text).map_err(|e| e.to_string()),
        CheckpointFormat::Json => serde_json::from_str(&text).map_err(|e| e.to_string()),
    }
    .map_err(|e| format!("{} 解析失败: {e}", path.display()))?;
    migrate(&mut state).map_err(|e| format!("{} 迁移失败: {e}", path.display()))?;
    Ok(state)
}

pub fn load_state(path: &Path) -> State {
    match load_state_checked(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{} 初始状态 {e}", "[错误]".red().bold());
            std::process::exit(1);
        }
    }
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
/// 档（checkpoint）的格式：**由扩展名决定**（`.json` ⇒ JSON，其余 ⇒ RON）。
///
/// ## 为什么档要多一个 JSON 格式
///
/// 「合成场景」型用例要**直接改状态**（把这艘舰的船体改成一半、把这个势力的库存清零、把这座城的
/// 人口压到 1……），而 Python 没有像样的 RON 库。状态字段是**字面量**——不像控制面那些叶有
/// `Inherit` 链、必须由引擎解析有效值——所以「按键找人、改个字面量」这件事交给 Python 的
/// `json` 模块最省事：**Rust 侧不用维护一份「哪些字段可改」的清单**（那份清单会无限长下去）。
///
/// 精度：`serde_json` 开了 `float_roundtrip`（见 `Cargo.toml` 的注释）⇒ 解析逐位精确，
/// 配合 serde_json 的 ryu 输出（最短往返表示）⇒ **f64 经 JSON 往返不变**。这条由
/// `play/tests/g1_contract.py` 的「JSON 档与 RON 档推进出同一份投影」盯着。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckpointFormat {
    Ron,
    Json,
}

impl CheckpointFormat {
    /// 按扩展名判：`.json` ⇒ JSON，别的（含没有扩展名）⇒ RON。
    pub fn of(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("json") => Self::Json,
            _ => Self::Ron,
        }
    }
}

pub fn save_checkpoint(path: &Path, round_state: &RoundState, rng: &Prng) -> Result<(), String> {
    let cp = Checkpoint {
        prng_state: rng.state(),
        round_state: round_state.clone(),
    };
    let text = match CheckpointFormat::of(path) {
        CheckpointFormat::Ron => ron::to_string(&cp).map_err(|e| format!("serialize: {e}"))?,
        CheckpointFormat::Json => serde_json::to_string(&cp).map_err(|e| format!("serialize json: {e}"))?,
    };
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// Load a checkpoint, returning the [`RoundState`] and the resumable [`Prng`].
///
/// 格式按扩展名走（见 [`CheckpointFormat`]）：`.json` 走 JSON，其余走 RON。
pub fn load_checkpoint(path: &Path) -> Result<(RoundState, Prng), String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut cp: Checkpoint = match CheckpointFormat::of(path) {
        CheckpointFormat::Ron => ron::from_str(&text).map_err(|e| e.to_string()),
        CheckpointFormat::Json => serde_json::from_str(&text).map_err(|e| e.to_string()),
    }
    .map_err(|e| format!("parse {}: {e}", path.display()))?;
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
pub fn load_initial(path: &Path, seed: u64) -> Result<(State, Prng), String> {
    if let Ok((round_state, rng)) = load_checkpoint(path) {
        return Ok((round_state.state, rng));
    }
    // Not a checkpoint; treat as a bare initial state.
    Ok((load_state_checked(path)?, Prng::new(seed)))
}

/// Parse a `--seed` value: a number, or `random` (case-insensitive).
pub fn parse_seed(v: &str) -> u64 {
    if v.eq_ignore_ascii_case("random") {
        return prng::random_seed();
    }
    v.trim().parse::<u64>().unwrap_or_else(|_| {
        eprintln!(
            "{} 无效的 --seed 值: {v} （用数字或 'random'）",
            "[错误]".red().bold()
        );
        std::process::exit(2);
    })
}

#[cfg(test)]
#[path = "tests/config.rs"]
mod tests;
