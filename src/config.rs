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


    /// `GameEvent::weight` 的**量纲**必须保持文档承诺的 **0–9 序数阶梯**，不是 0–100 分数。
    ///
    /// 这条守卫存在的理由是一个真实踩过的坑：`q.storyboard()` 曾把「值得读」的门槛写成 **60**
    /// （照「0–100 分数」的错觉），于是**静默返回空表**——测试全绿，故事板是空的。任何把量纲
    /// 拉大的改动都会让下游那个 `>= 8` 的门槛失去意义，所以它必须在这里红，而不是在 Python 里静默。
    #[test]
    fn weight_ladder_stays_a_documented_zero_to_nine_scale() {
        let by_kind = |k: &str| {
            samples()
                .into_iter()
                .find(|e| e.kind() == k)
                .unwrap_or_else(|| panic!("样本里没有 {k}"))
                .weight()
        };
        for ev in samples() {
            assert!(
                ev.weight() <= 9,
                "{} 的 weight={} 超出文档承诺的 0–9 量纲（q.storyboard 的门槛是 >= 8）",
                ev.kind(),
                ev.weight()
            );
        }
        // 阶梯本身：逐发流水 < 撤退 < 舰存亡 < 剧情 <= 城市易主 <= 世界格局。
        // （「重建」那一档随 `step_resurgence` 删除而消失——现在**没有**任何「势力复活」
        // 事件：重建走殖民舰，记的是 `colony_founded`，属于城市易主档。）
        assert!(by_kind("attack") < by_kind("withdraw"), "逐发流水必须在阶梯底部");
        assert!(by_kind("withdraw") < by_kind("ship_destroyed"));
        assert!(by_kind("ship_destroyed") < by_kind("story"));
        assert!(by_kind("story") <= by_kind("city_razed"));
        assert!(by_kind("city_razed") <= by_kind("war_started"));
        assert_eq!(by_kind("attack"), 0, "逐发流水 = 0");
        assert_eq!(by_kind("war_started"), 9, "世界格局级必须是阶梯顶端");
        // 门槛 8 必须真的切出一部分、又不能等于全量（否则故事板要么空、要么等于没筛）。
        let w: Vec<u8> = samples().iter().map(|e| e.weight()).collect();
        assert!(w.iter().any(|&x| x >= 8), "门槛 8 切不出任何东西");
        assert!(w.iter().any(|&x| x < 8), "门槛 8 等于全量，等于没筛");
    }
    /// 全部 `GameEvent` 变体各一个样本。
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
            GameEvent::Revolt { city: "c".into(), faction: "f".into(), loyalty: 0.0 },
            GameEvent::CityDefected {
                city: "c".into(), from: "f".into(), to: "g".into(), loyalty: 0.2,
            },
            GameEvent::CoalitionFormed { hegemon: "f".into(), members: vec!["g".into(), "h".into()] },
            GameEvent::CoalitionEnded { hegemon: "f".into(), members: vec!["g".into()] },
            GameEvent::CapitalRelocated {
                faction: "f".into(), from: "地球".into(), to: "火星".into(),
                reason: "destroyed".into(),
            },
            GameEvent::CargoLoaded {
                ship: "长征".into(), faction: "中国".into(), owner: "中国".into(),
                body: "金星".into(),
                cargo: [("碳".to_string(), 4.0)].into_iter().collect(),
            },
            GameEvent::CargoDelivered {
                ship: "长征".into(), faction: "中国".into(), owner: "中国".into(),
                body: "地球".into(),
                cargo: [("碳".to_string(), 4.0)].into_iter().collect(),
                into_pool: true,
            },
            GameEvent::ContractPosted {
                contract: 0, shipper: "中国".into(), resource: "碳".into(), capacity: 3.0,
                from: "金星".into(), to: "地球".into(), share: 0.15,
            },
            GameEvent::ContractAccepted {
                contract: 0, shipper: "中国".into(), carrier: "美国".into(),
                resource: "碳".into(), capacity: 3.0, from: "金星".into(), to: "地球".into(),
            },
            GameEvent::ContractDelivered {
                contract: 0, shipper: "中国".into(), carrier: "美国".into(), ship: "自由号".into(),
                resource: "碳".into(), amount: 10.2, cut: 1.8,
            },
            GameEvent::ContractReviewed {
                contract: 0, shipper: "中国".into(), carrier: "美国".into(),
                ratio: 1.2, good: true, delta: 0.08,
            },
            GameEvent::ContractEnded {
                contract: 0, shipper: "中国".into(), carrier: "美国".into(),
                reason: "term".into(),
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
            | GameEvent::Revolt { .. }
            | GameEvent::CityDefected { .. }
            | GameEvent::CoalitionFormed { .. }
            | GameEvent::CoalitionEnded { .. }
            | GameEvent::CapitalRelocated { .. }
            | GameEvent::CargoLoaded { .. }
            | GameEvent::CargoDelivered { .. }
            | GameEvent::ContractPosted { .. }
            | GameEvent::ContractAccepted { .. }
            | GameEvent::ContractDelivered { .. }
            | GameEvent::ContractReviewed { .. }
            | GameEvent::ContractEnded { .. } => {}
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
        assert_eq!(serde_json::to_string(&FoundingHow::NewSite).unwrap(), "\"new_site\"");
        assert_eq!(serde_json::to_string(&FoundingHow::Refounded).unwrap(), "\"refounded\"");
        // 认不出的标签要**明确报错**，不退回默认变体。
        assert!(serde_json::from_str::<DeathCause>("\"nonsense\"").is_err());
    }

    /// **分层判据的守卫**：分层按「后续计算需要访问哪一段历史」定，不按「重要性」。
    ///
    /// 这条测试钉住当前判据的两个结论，防止它们被无声地改回去：
    /// 1. **里程碑层为空**——没有任何 variant 有「无限过去」读者（见 `GameEvent::salience`）。
    ///    谁要是凭「这个事件听起来重要」把 variant 提升进来，这里会红。
    /// 2. **窗口层只装战争**，且**真的只保留窗口内的回合**——窗口裁剪是设计，不是丢失。
    #[test]
    fn history_layers_are_assigned_by_reader_need_not_importance() {
        let config = load_config();
        let seed = 7u64;
        let window = config.history.notable_window;
        assert!(window > 0, "窗口默认值应当是有限的（0 = 不裁剪，会让判据失去意义）");
        let mut state = world::default_state(&config, seed);
        let mut rng = crate::prng::Prng::new(seed);
        for _ in 0..20 {
            sim::advance(&mut state, &config, &mut rng);
        }

        // 1. 里程碑层按当前判据是空的。
        assert!(
            state.milestones.entries.is_empty(),
            "里程碑层应当为空（没有无限过去的读者），实际有 {} 条——\
             若有新读者出现，请同时更新 GameEvent::salience 的读者盘点表",
            state.milestones.entries.len()
        );
        assert!(state.milestones.is_complete(), "空的里程碑层不应声称丢过东西");

        // 2. 窗口层：非空、只装战争、且全部落在窗口内。
        let nb = &state.notables.entries;
        assert!(!nb.is_empty(), "20 回合里没记下任何窗口事件，窗口层可能没在记");
        assert!(
            nb.iter().all(|e| matches!(
                e.event,
                GameEvent::WarStarted { .. } | GameEvent::WarEnded { .. }
            )),
            "窗口层当前只该装战争（唯一的窗口读者是记恨地板）"
        );
        let oldest = state.round.saturating_sub(window as u32 - 1);
        assert!(
            nb.iter().all(|e| e.round >= oldest),
            "窗口层里出现了窗口外的记录：最早 {}，窗口下界 {oldest}",
            nb.iter().map(|e| e.round).min().unwrap_or(0)
        );

        // 与「逐回合重放 + 滑窗」的朴素算法对齐：条数必须相等。
        let mut replay = world::default_state(&config, seed);
        let mut rng2 = crate::prng::Prng::new(seed);
        let mut expected: Vec<u32> = Vec::new();
        for _ in 0..20 {
            sim::advance(&mut replay, &config, &mut rng2);
            let r = replay.round;
            for e in &replay.events {
                if e.salience() == crate::model::Salience::Notable {
                    expected.push(r);
                }
            }
        }
        expected.retain(|r| *r >= oldest);
        assert_eq!(
            nb.len(),
            expected.len(),
            "窗口层的条数必须等于「窗口内的 Notable 事件总数」"
        );
    }

    /// 里程碑层的机械行为：**截断可见**（丢弃量与丢弃到的回合都记下来），`0` = 无损。
    ///
    /// 这一层按当前判据没有生产者，所以这里直接构造条目来测机制——测的是 `trim` 本身，
    /// 与「谁该进来」那个判据问题无关。
    #[test]
    fn milestones_trim_reports_truncation_visibly() {
        let mk = |round: u32| crate::model::HistoryEntry {
            round,
            event: GameEvent::WarStarted { a: "甲".into(), b: "乙".into() },
        };
        let mut ms = crate::model::Milestones::default();
        for r in 1..=25u32 {
            ms.entries.push(mk(r));
        }

        // 无损配置（0）不动任何东西。
        let mut intact = ms.clone();
        intact.trim(0);
        assert_eq!(intact.entries.len(), 25);
        assert!(intact.is_complete());

        // 截断：留下最新 cap 条，丢掉的最旧那批的回合号要被记下来。
        let mut trimmed = ms.clone();
        trimmed.trim(10);
        assert_eq!(trimmed.entries.len(), 10);
        assert_eq!(trimmed.dropped, 15);
        assert!(!trimmed.is_complete());
        assert_eq!(trimmed.dropped_through_round, 15, "丢到第 15 回合");
        assert_eq!(trimmed.entries.first().unwrap().round, 16);
    }

    /// 窗口层的机械行为：只收窗口内、`window = 1` 等价于「只看本回合」、`0` = 不裁剪。
    #[test]
    fn notables_keep_exactly_the_window() {
        let mk = |round: u32| crate::model::HistoryEntry {
            round,
            event: GameEvent::WarStarted { a: "甲".into(), b: "乙".into() },
        };
        let mut nb = crate::model::Notables::default();
        for r in 1..=25u32 {
            nb.entries.push(mk(r));
        }

        // window = 5，当前回合 25 → 保留 [21, 25]。
        nb.trim(25, 5);
        assert_eq!(nb.entries.len(), 5);
        assert_eq!(nb.entries.first().unwrap().round, 21);
        assert_eq!(nb.entries.last().unwrap().round, 25);

        // 本回合仍在窗口内：window = 1 → 只剩当前回合（不是「空」）。
        let mut one = crate::model::Notables::default();
        for r in 1..=25u32 {
            one.entries.push(mk(r));
        }
        one.trim(25, 1);
        assert_eq!(one.entries.len(), 1);
        assert_eq!(one.entries[0].round, 25);

        // 0 = 不裁剪。
        let mut all = crate::model::Notables::default();
        for r in 1..=25u32 {
            all.entries.push(mk(r));
        }
        all.trim(25, 0);
        assert_eq!(all.entries.len(), 25);
    }

    /// `push` 的分层过滤：每一层只吃自己那一档，其余直接忽略。
    #[test]
    fn each_layer_only_takes_its_own_salience() {
        use crate::model::{Notables, Salience};
        let war = GameEvent::WarStarted { a: "甲".into(), b: "乙".into() };
        let raze = GameEvent::CityRazed {
            city: "城".into(),
            owner: "甲".into(),
            fallen_to: "乙".into(),
            by_ship: "舰".into(),
            damage: 1.0,
            pop_before: 1,
        };
        assert_eq!(war.salience(), Salience::Notable);
        assert_eq!(raze.salience(), Salience::Detail, "城市事件当前没有窗口/无限读者");

        let mut nb = Notables::default();
        nb.push(3, war.clone());
        nb.push(3, raze.clone());
        assert_eq!(nb.entries.len(), 1, "窗口层只收 Notable");
        assert!(matches!(nb.entries[0].event, GameEvent::WarStarted { .. }));

        let mut ms = crate::model::Milestones::default();
        ms.push(3, war);
        ms.push(3, raze);
        assert!(ms.entries.is_empty(), "里程碑层当前没有任何 variant 属于它");
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
        assert_eq!(back.notables.entries.len(), state.notables.entries.len());
        assert_eq!(
            back.notables.entries.iter().map(|e| e.round).collect::<Vec<_>>(),
            state.notables.entries.iter().map(|e| e.round).collect::<Vec<_>>(),
            "窗口层的回合号必须逐条读回来（记恨地板靠它算年龄）"
        );
    }
}

