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
mod tests {
    use super::*;
    use crate::model::{
        DeathCause, FoundingHow, GameEvent, Killer, RoundState, SpawnVia, State, SCHEMA_VERSION,
    };
    use crate::sim;
    use crate::world;

    /// `--save` 写出的 checkpoint，`--start` 必须能原样读回来，而且**接着跑要和一路跑到底
    /// 完全一致**。这是「分段讲故事」的全部基础（agent 玩这个游戏的基本 loop）。
    ///
    /// 之前**完全没有测试**覆盖它，于是 `GameEvent` 里加了一个嵌在内部标签 enum 内的单元
    /// enum 之后（见 [`crate::model`] 的 `stringly_unit_enum`），整份 checkpoint 静默读不
    /// 回来；而 `load_initial` 会把「checkpoint 解析失败」咽掉、退回当裸 `State` 解析，最后
    /// 报成一句指向文件末尾的「missing field `round` in `State`」——离真正的原因十万八千里。
    #[test]
    fn checkpoint_survives_save_and_resume_identically() {
        let config = load_config();
        let seed = 7u64;
        let dir = std::env::temp_dir().join(format!("planet_x_ckpt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.ron");

        // 一路跑到底（参照组）。
        let mut straight = world::default_state(&config, seed);
        let mut rng_straight = crate::prng::Prng::new(seed);
        for _ in 0..6 {
            sim::advance(&mut straight, &config, &mut rng_straight);
        }

        // 分段：跑 3 回合 → 存 → 读回来 → 再跑 3 回合。
        let mut a = world::default_state(&config, seed);
        let mut rng_a = crate::prng::Prng::new(seed);
        for _ in 0..3 {
            sim::advance(&mut a, &config, &mut rng_a);
        }
        let derived = sim::derived_from_state(&a, &config);
        let rs = RoundState {
            schema_version: SCHEMA_VERSION,
            state: a.clone(),
            pre: derived.clone(),
            post: derived,
        };
        save_checkpoint(&path, &rs, &rng_a).expect("save_checkpoint");

        let (loaded, mut rng_b) = load_checkpoint(&path).expect("checkpoint 必须能读回来");
        assert_eq!(loaded.state.round, a.round);
        assert_eq!(
            loaded.state.milestones.entries.len(),
            a.milestones.entries.len(),
            "长存里程碑必须随 checkpoint 一起活下来"
        );
        assert_eq!(rng_b.state(), rng_a.state(), "RNG 位置必须原样恢复");

        let mut resumed = loaded.state;
        for _ in 0..3 {
            sim::advance(&mut resumed, &config, &mut rng_b);
        }
        assert_eq!(resumed.round, straight.round);
        assert_eq!(
            crate::agent::render_state(&resumed, &sim::derived_from_state(&resumed, &config)),
            crate::agent::render_state(&straight, &sim::derived_from_state(&straight, &config)),
            "分段续玩必须与一路跑到底逐字节一致"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **每个** `GameEvent` 变体都要能过一遍 RON 往返。
    ///
    /// 这个守卫是为了钉住一类**结构性**缺陷：单元 enum（`DeathCause`/`SpawnVia`/`FoundingHow`）
    /// 嵌在内部标签 enum 里时，若序列化成裸标识符，RON 读不回来（见 `stringly_unit_enum`）。
    /// 单看某一个变体测不出来——必须**逐个变体**试。
    ///
    /// 「有没有漏掉新变体」由 `variant_checklist` 的穷尽 match 提醒：新增 variant 时那里会
    /// 编译失败，照着把样本加进 `samples()` 即可（与 `history_row`/`kind`/`headline` 同一套
    /// 「漏不掉」纪律）。
    #[test]
    fn every_game_event_variant_round_trips_through_ron() {
        let samples = samples();
        for ev in &samples {
            variant_checklist(ev); // 穷尽 match：新增变体时这里编译失败
            let text = ron::to_string(ev).expect("serialize");
            let back: GameEvent = ron::from_str(&text)
                .unwrap_or_else(|e| panic!("{} 无法从 RON 读回: {e}\n  {text}", ev.kind()));
            assert_eq!(&back, ev, "{} 往返后不相等", ev.kind());
            // JSON 侧形状同样必须稳定（agent 视图/投影吃的是这一份）。
            let js = serde_json::to_string(ev).expect("json");
            let back_js: GameEvent = serde_json::from_str(&js).expect("json round-trip");
            assert_eq!(&back_js, ev);
        }
        assert!(samples.len() >= 18, "样本数 {} 应覆盖全部变体", samples.len());
    }

    /// 全部 18 个 `GameEvent` 变体各一个样本。
    fn samples() -> Vec<GameEvent> {
        vec![
            GameEvent::Attack { attacker: "a".into(), target: "b".into(), damage: 1.5 },
            GameEvent::ShipDestroyed {
                ship: "s".into(), owner: "f".into(), class: "corvette".into(),
                cause: DeathCause::Combat,
                by: Some(Killer { ship: "k".into(), faction: "g".into(), weapon: "kinetic".into() }),
            },
            // 无凶手的战沉（测试/兜底路径）与**非战沉**（锈蚀）都要覆盖。
            GameEvent::ShipDestroyed {
                ship: "s".into(), owner: "f".into(), class: "corvette".into(),
                cause: DeathCause::UpkeepShortfall, by: None,
            },
            GameEvent::ShipDestroyed {
                ship: "s".into(), owner: "f".into(), class: "corvette".into(),
                cause: DeathCause::Scrapped, by: None,
            },
            GameEvent::Siege { attacker: "a".into(), city: "c".into(), damage: 2.0 },
            GameEvent::CityRazed {
                city: "c".into(), owner: "f".into(), fallen_to: "g".into(),
                by_ship: "a".into(), damage: 9.0, pop_before: 120,
            },
            GameEvent::ShipSpawned {
                ship: "s".into(), owner: "f".into(), class: "corvette".into(),
                city: Some("c".into()), via: SpawnVia::Shipyard,
            },
            GameEvent::ShipSpawned {
                ship: "s".into(), owner: "f".into(), class: "corvette".into(),
                city: None, via: SpawnVia::Story,
            },
            GameEvent::ShipSpawned {
                ship: "s".into(), owner: "f".into(), class: "corvette".into(),
                city: Some("c".into()), via: SpawnVia::Resurgence,
            },
            GameEvent::ColonyFounded {
                city: "c".into(), owner: "f".into(), body: "地球".into(),
                seeded_ship_class: "corvette".into(), how: FoundingHow::NewSite, prev_owner: None,
            },
            GameEvent::ColonyFounded {
                city: "c".into(), owner: "f".into(), body: "地球".into(),
                seeded_ship_class: "corvette".into(), how: FoundingHow::Refounded,
                prev_owner: Some("g".into()),
            },
            GameEvent::StaleOrder { ship: "s".into(), reason: "gone".into() },
            GameEvent::Withdraw { ship: "s".into(), to_body: "地球".into() },
            GameEvent::WarStarted { a: "f".into(), b: "g".into() },
            GameEvent::WarEnded { a: "f".into(), b: "g".into() },
            GameEvent::Story { id: "p".into(), title: "序章".into(), participants: vec!["f".into()] },
            GameEvent::Resurgence {
                faction: "f".into(), body: "地球".into(), ship: "s".into(), city: "c".into(),
            },
            GameEvent::Revolt { city: "c".into(), faction: "f".into(), loyalty: 0.0 },
            GameEvent::CityDefected {
                city: "c".into(), from: "f".into(), to: "g".into(), loyalty: 0.2,
            },
            GameEvent::CityOverrun { city: "c".into(), from: "f".into(), to: "g".into() },
            GameEvent::CoalitionFormed { hegemon: "f".into(), members: vec!["g".into(), "h".into()] },
            GameEvent::CoalitionEnded { hegemon: "f".into(), members: vec!["g".into()] },
            GameEvent::CapitalRelocated {
                faction: "f".into(), from: "地球".into(), to: "火星".into(),
                reason: "destroyed".into(),
            },
        ]
    }

    /// 穷尽 match：**新增 `GameEvent` 变体时这里会编译失败**，提醒把样本补进 [`samples`]。
    fn variant_checklist(e: &GameEvent) {
        match e {
            GameEvent::Attack { .. }
            | GameEvent::ShipDestroyed { .. }
            | GameEvent::Siege { .. }
            | GameEvent::CityRazed { .. }
            | GameEvent::ShipSpawned { .. }
            | GameEvent::ColonyFounded { .. }
            | GameEvent::StaleOrder { .. }
            | GameEvent::Withdraw { .. }
            | GameEvent::WarStarted { .. }
            | GameEvent::WarEnded { .. }
            | GameEvent::Story { .. }
            | GameEvent::Resurgence { .. }
            | GameEvent::Revolt { .. }
            | GameEvent::CityDefected { .. }
            | GameEvent::CityOverrun { .. }
            | GameEvent::CoalitionFormed { .. }
            | GameEvent::CoalitionEnded { .. }
            | GameEvent::CapitalRelocated { .. } => {}
        }
    }

    /// 单元 enum 的 **JSON 形状必须与改造前逐字一致**（agent 视图、`idx/events.jsonl` 的
    /// `data.cause`、WebUI 都吃这一份）。字符串化只是为了修 RON，不该动 JSON。
    #[test]
    fn unit_enums_keep_their_json_shape() {
        assert_eq!(serde_json::to_string(&DeathCause::UpkeepShortfall).unwrap(), "\"upkeep_shortfall\"");
        assert_eq!(serde_json::to_string(&DeathCause::Combat).unwrap(), "\"combat\"");
        assert_eq!(serde_json::to_string(&DeathCause::Scrapped).unwrap(), "\"scrapped\"");
        assert_eq!(serde_json::to_string(&SpawnVia::Shipyard).unwrap(), "\"shipyard\"");
        assert_eq!(serde_json::to_string(&SpawnVia::Story).unwrap(), "\"story\"");
        assert_eq!(serde_json::to_string(&SpawnVia::Resurgence).unwrap(), "\"resurgence\"");
        assert_eq!(serde_json::to_string(&FoundingHow::NewSite).unwrap(), "\"new_site\"");
        assert_eq!(serde_json::to_string(&FoundingHow::Refounded).unwrap(), "\"refounded\"");
        // 认不出的标签要**明确报错**，不退回默认变体。
        assert!(serde_json::from_str::<DeathCause>("\"nonsense\"").is_err());
    }

    /// 长存里程碑：只收里程碑、跨回合不丢、可按实体查史；截断时**如实记账**。
    #[test]
    fn milestones_collects_milestones_and_reports_truncation() {
        let config = load_config();
        let seed = 7u64;
        let mut state = world::default_state(&config, seed);
        let mut rng = crate::prng::Prng::new(seed);
        for _ in 0..20 {
            sim::advance(&mut state, &config, &mut rng);
        }
        let n = state.milestones.entries.len();
        assert!(n >= 5, "20 回合只攒了 {n} 条里程碑，里程碑可能没在记");

        // 里程碑 = 全部里程碑事件（逐回合 events 里的 milestone 之和），一条不多一条不少，
        // 且**逐发流水永不入账**。
        let mut expected = 0usize;
        let mut replay = world::default_state(&config, seed);
        let mut rng2 = crate::prng::Prng::new(seed);
        for _ in 0..20 {
            sim::advance(&mut replay, &config, &mut rng2);
            expected += replay
                .events
                .iter()
                .filter(|e| e.salience() == crate::model::Salience::Milestone)
                .count();
        }
        assert_eq!(n, expected, "里程碑条数必须等于里程碑事件总数");
        assert!(
            !state.milestones.entries.iter().any(|e| matches!(
                e.event,
                GameEvent::Attack { .. } | GameEvent::Siege { .. }
            )),
            "逐发流水不该进长存里程碑"
        );
        // 回合号必须是真的（跨回合累计，不是「全是最后一回合」）。
        let rounds: std::collections::BTreeSet<u32> =
            state.milestones.entries.iter().map(|e| e.round).collect();
        assert!(rounds.len() >= 5, "里程碑应跨多个回合，实际只覆盖 {rounds:?}");

        // 按实体查史：随便挑一个出现过的城，它的历史必须非空且都点到它的名。
        if let Some(entry) = state.milestones.entries.iter().find(|e| {
            e.event.participants().iter().any(|p| p.kind == crate::model::EntityKind::City)
        }) {
            let cid = entry
                .event
                .participants()
                .into_iter()
                .find(|p| p.kind == crate::model::EntityKind::City)
                .unwrap()
                .id;
            let hist = state.milestones.history_of(crate::model::EntityKind::City, &cid);
            assert!(!hist.is_empty());
            assert!(hist.iter().all(|e| e.event.headline().contains(&cid)));
        } else {
            panic!("20 回合里没有任何涉及城市的事件，样本无效");
        }

        // 截断：**可见**。丢弃量与丢弃到的回合都要记下来。
        let mut trimmed = state.milestones.clone();
        let cap = 10;
        trimmed.trim(cap);
        assert_eq!(trimmed.entries.len(), cap);
        assert_eq!(trimmed.dropped, (n - cap) as u64);
        assert!(!trimmed.is_complete());
        assert!(trimmed.dropped_through_round > 0);
        // 无损配置（0）不动任何东西。
        let mut intact = state.milestones.clone();
        intact.trim(0);
        assert_eq!(intact.entries.len(), n);
        assert!(intact.is_complete());
    }

    /// 载入一个**裸 `State`** 的 RON 也要能过（`--start` 的另一条路径）。
    #[test]
    fn bare_state_round_trips_through_ron() {
        let config = load_config();
        let seed = 42u64;
        let mut state = world::default_state(&config, seed);
        let mut rng = crate::prng::Prng::new(seed);
        for _ in 0..5 {
            sim::advance(&mut state, &config, &mut rng);
        }
        let text = ron::to_string(&state).expect("serialize");
        let back: State = match ron::from_str(&text) {
            Ok(v) => v,
            Err(e) => panic!("裸 State 无法从 RON 读回: {e}"),
        };
        assert_eq!(back.round, state.round);
        assert_eq!(back.events.len(), state.events.len());
        assert_eq!(back.milestones.entries.len(), state.milestones.entries.len());
    }
}

