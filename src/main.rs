//! planet_x — a deterministic sandbox trajectory generator for the Planet X game.
//!
//! This is the **agent-facing** tool. It emits zero-noise machine-readable data
//! so an LLM can run a *story*: it produces a linear trajectory (one JSON object
//! per round, all values rounded to 2 decimals), plus the self-describing rules /
//! schema / narrative artifacts. There is **no interactive query REPL** — the
//! agent reads the world via `--index` projections (lean main stream + id-indexed
//! lazy tables) through the `play/planet_xq` Python kit (uv / pandas); `jq` is
//! removed and is no longer a dependency.
//!
//! Usage: `planet_x [--seed <s|random>] [--start <path.ron>]`
//! `[--apply <file.json>] [--round <n>] [--traj <n>] [--save <path.ron>]`
//! `[--meta] [--schema] [--story] [--control]`
//!
//! * `--start`  load an initial `State` from a RON checkpoint (state + RNG); else
//!              generate the default procedural solar system.
//! * `--apply`  overlay a control-state diff (JSON, same shape as the web
//!              `POST /api/command`: `{control:[...], scope:{...}}`) onto the state
//!              before the run. This is how an **agent steers the story**: by
//!              issuing ship orders / budgets / invest weights. A structural
//!              multi-level patch: only the factions/leaves present in the file are
//!              touched, and omitted `value`/`behavior` keep the current value
//!              while an omitted `mode` keeps the current mode.
//! * `--round`  run `n` rounds, emitting one JSON object per round (round 0 first,
//!              then one per round) as JSON Lines. This is the **trajectory body**;
//!              read it back with `--index` + `play/planet_xq` (or `--digest` for a
//!              coarse storyboard) rather than a JSON query tool.
//! * `--traj`   run `n` rounds and emit ONE self-contained JSON document:
//!              `{schema_version, meta, story, trajectory:[...round snapshots...]}` —
//!              a packaged "story pack" with the timeline, the narrative beats and
//!              the rules in a single value.
//! * `--save`   write a deterministic checkpoint (state + RNG) so a later `--start`
//!              resumes identically. Together with `--apply`, this lets an agent run
//!              a story in **segments**: run a bit, apply a steering diff, run more.
//! * `--meta`   dump the game config (rules dictionary: resources raw-key→中文名,
//!              structures/buildings/ships/components specs, tuning constants).
//! * `--schema` dump a machine-readable JSON Schema for the per-round agent view
//!              (derived from the same structs it is rendered from). Use this to
//!              introspect field names rather than memorising them.
//! * `--story`  dump the story chronicle (`State::chronicle`) as a JSON array — the
//!              full narrative arc (round, id, title, body, participants).
//! * `--control` dump the editable control surface (control + scope) — the template
//!              an agent edits into an `--apply` diff.
//!
//! Determinism: the same seed / checkpoint always reproduces the same trajectory.
//! The tool is purely a generator; the `planet_x_web` binary is the separate,
//! player-facing interactive UI (and is unaffected by these query changes).

use clap::Parser;
use planet_x::agent;
use planet_x::config::{load_config, load_initial, parse_seed, save_checkpoint};
use planet_x::model::{FactionId, GameConfig, GameEvent, RoundFlow, RoundMetrics, State, SCHEMA_VERSION};
use planet_x::prng::Prng;
use planet_x::{projection, sim, web, world};
use serde_json::json;
use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "planet_x",
    version,
    about = "行星X——太空沙盘轨迹生成器（agent 讲故事用）",
    long_about = "《行星X》是一个回合制太阳系沙盘轨迹生成器：每回合 = 1 个月，整个游戏由一个可确定复现的\n\
State 快照推进，全部数值由 config/game.ron 数据驱动、不硬编码。这个 CLI 是面向 agent 的\n\
零噪声轨迹生成器：无交互查询 REPL——agent 用 **`--index` 投影** + `play/planet_xq`（uv/pandas）\n\
读世界（jq 已移除、不再是依赖）。\n\
\n\
【世界】\n\
- 天体 body：绕太阳 2D 椭圆轨道（近日点/远日点距离、远日点方向、公转周期），每回合重算位置。\n\
- 城市 city：建在定居点 settlement 上、由一势力控制，内有连续面积分配的 建筑。\n\
- 建筑 building：非原子，是连续面积分配（计划 area 与已建 deployed），三种角色：residential\n\
  居住、mining 开采、construction 建造。\n\
- 势力 faction：拥有城市与飞船、库存各资源、两两外交关系。\n\
- 飞船 ship：必属某势力，从城市出厂，可移动/开火/围城；舰级参数（hull/regen/attack/speed/range/\n\
  build_points/cost/upkeep/slots 及各类修正倍率）由 config 定义，spec 五级舰：护卫/驱逐/巡洋/\n\
  航母/战列。\n\
\n\
【每回合演化 sim::advance】天体重算 → 经济（开采/人口）→ 维护 upkeep → 市场（自动兑换富余\n\
矿物）→ 建设（建筑/造舰）→ 军事（移动/开火/围城/攻占）→ 外交（关系漂移/战争/停战）。\n\
\n\
【控制模型 = 指令】每势力有可控状态 State::control：ship_orders（Idle/Move/TargetShip/\n\
TargetSettlement/Dock/Colonize）、budget（投资预算）、invest_weights。每个叶子带 mode：\n\
Ai（系统自动决策）| Player（玩家指令，系统只读）| None（继承上层）。State::scope 是一棵\n\
作用域树（全局→势力→天体→城市），决定某叶子由谁控制。agent 用 --apply 写 diff 定向故事。\n\
\n\
【讲故事流程】\n\
- `planet_x --seed 42 --round 240 --index out/`    # 跑一段轨迹 + 投影（lean 主流 + 索引表）\n\
- `planet_xq.load('out').facts`                    # 读主流；`q.join('ships', round=r)` 按 id join\n\
- `planet_x --start s240.ron --apply steer.json --round 240`      # 分段续玩 + 定向\n\
- `planet_x --traj 240`                            # 一键拿故事封包（含 `.story` 编年史）\n\
- 先 `--schema` 查字段、`--meta` 查规则；分析用 `--index` + `play/planet_xq`，别用 jq。",
    after_help = "agent 专用：stdout 只输出零噪声机器可读 JSON（无颜色/星图/表格/散文）。\n\
--round N 输出 N+1 行 JSON（回合 0 + N 回合）；--traj N 输出一个自包含 story pack；\n\
--meta/--schema/--story/--control 各自输出一个 JSON 值。分析用 --index + play/planet_xq。"
)]
struct Cli {
    /// 确定性随机种子（数字，或 random / 随机）
    #[arg(long, default_value = "random", value_name = "SEED")]
    seed: String,

    /// 从指定的 checkpoint（state + RNG）.ron 文件开始
    #[arg(long, value_name = "PATH")]
    start: Option<PathBuf>,

    /// 运行 n 个回合并把每个回合输出为一行 JSON（回合 0 先），供外部分析（--index/--digest）
    #[arg(long = "round", visible_alias = "rounds", value_name = "N")]
    round: Option<u32>,

    /// 运行 n 个回合，输出一个自包含 JSON 文档 {schema_version, meta, story, trajectory:[...]}
    #[arg(long, value_name = "N")]
    traj: Option<u32>,

    /// 把一份可控状态 diff（JSON，形同 web 的 POST /api/command：{control,scope}）
    /// 按键 overlay 到状态上，再执行 --round/--traj。用于 agent 定向故事。
    #[arg(long, value_name = "FILE")]
    apply: Option<PathBuf>,

    /// （--round/--traj 结束后）把当前状态连同 PRNG 位置写入一个 RON checkpoint，
    /// 以便下次用 --start 确定性续玩（分段讲故事）。
    #[arg(long, value_name = "FILE")]
    save: Option<PathBuf>,

    /// 输出整份游戏配置（resources/buildings/ships/components/economy/combat/diplomacy…）
    /// 为一行 JSON。这是 agent 的规则字典；配合 --index + planet_xq 使用。
    #[arg(long)]
    meta: bool,

    /// 输出 agent 视图的机器可读 JSON Schema（由渲染同一批结构体派生），供 agent 查字段。
    #[arg(long)]
    schema: bool,

    /// 输出剧情编年史（叙事弧：round/id/title/body/participants）为 JSON 数组。
    #[arg(long)]
    story: bool,

    /// 输出可编辑控制面（control + scope）JSON——agent 写 --apply diff 的模板。
    #[arg(long)]
    control: bool,

    /// 降采样步长：与 --round/--traj 连用，每 K 回合取一个快照（1 = 每回合）。用于把
    /// 超长轨迹变成可读的粗粒度采样，避免几千行全量 JSON 撑爆上下文。
    #[arg(long, value_name = "K", default_value_t = 1)]
    every: u32,

    /// 粗粒度「编年史窗口」：与 --round 连用，每 K 回合输出**一行语义摘要**（该窗口的
    /// 各势力军力/城市/实力占比、战争、事件类型计数、剧情节拍 id），而不是逐回合全量
    /// 快照——超长轨迹的「故事板」。`--digest 100 --round 3000` → 30 行。
    #[arg(long, value_name = "K")]
    digest: Option<u32>,

    /// 索引投影模式：与 --round N 连用，把 N+1 回合投影成一份**lean 主流**（main.jsonl，
    /// 每回合一行：eager 字段 + metrics + id 数组）+ 按 id 索引的 **lazy 表**（idx/*.jsonl，
    /// ships/cities/bodies 的完整对象）+ **agent 可读的投影 schema**（schema.json，声明哪些
    /// 字段 eager / 哪些 lazy 及其表/key/列类型）。重型字段不再内联；agent 用 Python kit
    /// （play/planet_xq，uv 管理）按 id join。确定性：同 seed 复现字节一致。
    #[arg(long, value_name = "DIR")]
    index: Option<PathBuf>,
}

fn main() {
    let cli = Cli::parse();

    // Agent-first output: always zero-noise. Redirected output keeps the single
    // JSON Lines stream clean, so disable colours unconditionally.
    colored::control::set_override(false);

    let config = load_config();

    // Config / schema dumps don't need a world — emit and exit.
    if cli.meta {
        emit(&agent::meta_value(&config).to_string());
        return;
    }
    if cli.schema {
        emit(&agent::schema_value().to_string());
        return;
    }

    // Build the world: from a checkpoint (state + RNG) or generated procedurally.
    let seed = parse_seed(&cli.seed);
    let (mut state, mut rng) = match &cli.start {
        Some(path) => load_initial(path, seed),
        None => (world::default_state(&config, seed), Prng::new(seed)),
    };

    // Overlay a control-state diff (agent steering) before anything runs.
    if let Some(path) = &cli.apply {
        if let Err(e) = apply_diff(&mut state, &config, path) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_APPLY", "message": e}));
            std::process::exit(10);
        }
    }

    // State-reflecting dumps.
    if cli.story {
        emit(&agent::story_value(&state).to_string());
        return;
    }
    if cli.control {
        emit(&web::control_surface(&state).to_string());
        return;
    }

    // 索引投影模式：写 lean 主流 + lazy 表，不输出 stdout 单条流。
    if let Some(dir) = &cli.index {
        let Some(n) = cli.round else {
            eprintln!(
                "{}",
                json!({"ok": false, "code": "ERR_INDEX_ROUND", "message": "--index DIR 需要 --round N（投影的回合数）"})
            );
            std::process::exit(10);
        };
        if let Err(e) = projection::write_index(&mut state, &config, &mut rng, n, dir) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_INDEX", "message": e}));
            std::process::exit(10);
        }
        save_if_requested(cli.save.as_deref(), &state, &rng);
        return;
    }

    // Trajectory runs.
    if let Some(n) = cli.traj {
        run_trajectory(&mut state, &config, &mut rng, n, cli.every, cli.save.as_deref());
        return;
    }
    if let Some(n) = cli.round {
        run_rounds(&mut state, &config, &mut rng, n, cli.every, cli.digest, cli.save.as_deref());
        return;
    }

    eprintln!(
        "{}",
        json!({"ok": false, "code": "ERR_USAGE", "message": "nothing to do: specify --round N, --traj N, or a dump flag (--meta/--schema/--story/--control)"})
    );
    std::process::exit(10);
}

/// Read a control-state diff file (JSON, same shape as `POST /api/command`,
/// i.e. `{control:[...], scope:{...}}`) and overlay it onto `state` by key.
fn apply_diff(state: &mut State, config: &GameConfig, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    web::apply_patch(state, config, &value)
}

/// Write a line to stdout, ignoring broken-pipe errors so piping into `jq` (or an
/// agent that closes the pipe early) exits quietly instead of panicking.
fn emit(s: &str) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{s}");
    let _ = out.flush();
}

/// Run `n` rounds. Depending on `digest`, either emit one semantic digest line per
/// `digest`-round window (a coarse "storyboard"), or emit one agent-view JSON object
/// per round sampled at `every` (downsampling: rounds 0, K, 2K, …). Each line is
/// JSON Lines queryable by external `jq`. Optionally persist a deterministic
/// checkpoint (state + RNG position).
fn run_rounds(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    n: u32,
    every: u32,
    digest: Option<u32>,
    save: Option<&Path>,
) {
    if let Some(window) = digest {
        run_digest(state, config, rng, n, window, save);
        return;
    }
    let every = every.max(1);
    emit(&agent::render_state(state, config, &RoundFlow::default())); // round 0 / start
    for _ in 0..n {
        let flow = sim::advance(state, config, rng);
        if state.round % every == 0 {
            emit(&agent::render_state(state, config, &flow));
        }
    }
    save_if_requested(save, state, rng);
}

/// Run `n` rounds and emit ONE self-contained JSON document:
/// `{schema_version, meta, story, trajectory:[...round snapshots...]}` — a packaged
/// "story pack" with the timeline, the narrative arc and the rules in a single value.
/// `every > 1` downsamples the `trajectory` array (round 0 then every K-th).
fn run_trajectory(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    n: u32,
    every: u32,
    save: Option<&Path>,
) {
    let every = every.max(1);
    let mut snaps = vec![agent::state_json(state, config, &RoundFlow::default())];
    for _ in 0..n {
        let flow = sim::advance(state, config, rng);
        if state.round % every == 0 {
            snaps.push(agent::state_json(state, config, &flow));
        }
    }
    let pack = json!({
        "schema_version": SCHEMA_VERSION,
        "meta": agent::meta_value(config),
        "story": agent::story_value(state),
        "trajectory": snaps,
    });
    emit(&pack.to_string());
    save_if_requested(save, state, rng);
}

fn save_if_requested(save: Option<&Path>, state: &State, rng: &Prng) {
    if let Some(path) = save {
        if let Err(e) = save_checkpoint(path, state, rng) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_SAVE", "message": e.to_string()}));
            std::process::exit(10);
        }
    }
}

/// Emit one **semantic digest** line per `window`-round window: a coarse
/// per-window summary (each faction's city/ship/fleet totals + power share, the
/// active wars, event-type counts, and the story beats fired). This is the
/// "storyboard" an agent reads for a very long run without wading through
/// thousands of full snapshots. Only complete windows are emitted.
fn run_digest(state: &mut State, config: &GameConfig, rng: &mut Prng, n: u32, window: u32, save: Option<&Path>) {
    let window = window.max(1);
    let mut win_start = state.round;
    let mut events_acc: Vec<GameEvent> = Vec::new();
    let mut prod_acc: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut story_idx = state.chronicle.len();
    for _ in 0..n {
        let flow = sim::advance(state, config, rng);
        events_acc.extend(state.events.iter().cloned());
        // 累计本窗口各方产出（窗口级总开采价值），让 digest 的 `production` 是**窗口总量**，
        // 而非某一点时值。
        for (fid, res) in &flow.faction_production {
            let v: f64 = res
                .iter()
                .map(|(k, amt)| amt * config.resources.get(k).map(|r| r.value).unwrap_or(1.0))
                .sum();
            *prod_acc.entry(fid.clone()).or_default() += v;
        }
        if state.round - win_start >= window {
            let story: Vec<String> =
                state.chronicle[story_idx..].iter().map(|c| c.id.clone()).collect();
            // 窗口末态的「总结指标」直接取自同源的 step 计算（与逐回合 agent 视图一致），
            // 不再在 digest 里独立重算一遍。
            let metrics = sim::round_metrics(state, config, &flow);
            emit(&digest_value(state, &metrics, &prod_acc, win_start, state.round, &events_acc, &story).to_string());
            win_start = state.round;
            events_acc.clear();
            prod_acc.clear();
            story_idx = state.chronicle.len();
        }
    }
    save_if_requested(save, state, rng);
}

fn r2(v: f64) -> f64 {
    // + 0.0 把 IEEE 的 -0.0 规整成 +0.0（round 保留负零），避免 agent 看到 `-0.0`。
    (v * 100.0).round() / 100.0 + 0.0
}

/// The coarse per-window summary value. State-derived fields (world totals, per-faction
/// city/ship/fleet, power share, hegemon/coalition/sanction, wars) are taken directly from
/// the [`RoundMetrics`] the step functions computed at this window's end — the same numbers
/// the per-round agent view carries — so the digest can never drift from the simulation.
/// Only the window-accumulated fields (event counts, story beats) are built here.
fn digest_value(
    state: &State,
    metrics: &RoundMetrics,
    production: &BTreeMap<FactionId, f64>,
    from: u32,
    to: u32,
    events_acc: &[GameEvent],
    story: &[String],
) -> serde_json::Value {
    let factions: Vec<serde_json::Value> = metrics
        .factions
        .iter()
        .map(|(fid, m)| {
            json!({
                "id": fid,
                "name": state.faction(fid).map(|f| f.name.clone()).unwrap_or_default(),
                "city_count": m.city_count,
                "ship_count": m.ship_count,
                "fleet_value": r2(m.fleet_value),
                "population": m.population,
                "market_value": r2(m.market_value),
                "at_war": m.at_war,
                // 窗口累计开采产出（价值），由逐回合 flow 累加而来，与模拟一致。
                "production": r2(production.get(fid).copied().unwrap_or(0.0)),
                "upkeep": r2(m.upkeep),
                "governance_cost": r2(m.governance_cost),
                "governance_coverage": r2(m.governance_coverage),
            })
        })
        .collect();
    json!({
        "from": from,
        "to": to,
        "rounds": to.saturating_sub(from),
        "world": {
            "cities": metrics.cities,
            "ships": metrics.ships,
            "fleet_value": r2(metrics.fleet_value),
            "population": metrics.population,
        },
        "factions": factions,
        "power_share": metrics.power_share.iter().map(|(k, v)| (k.clone(), r2(*v))).collect::<BTreeMap<_, _>>(),
        "hegemon": metrics.hegemon,
        "coalition_members": metrics.coalition_members,
        "sanctioned": metrics.sanctioned,
        "wars": metrics.wars,
        "events": event_counts(events_acc),
        "story": story,
    })
}

/// Count the GameEvent variants in a window, keyed by their `type` label.
fn event_counts(events: &[GameEvent]) -> BTreeMap<String, u32> {
    use GameEvent::*;
    let mut m = BTreeMap::new();
    for e in events {
        let label = match e {
            Attack { .. } => "attack",
            ShipDestroyed { .. } => "ship_destroyed",
            Siege { .. } => "siege",
            CityRazed { .. } => "city_razed",
            ShipSpawned { .. } => "ship_spawned",
            ColonyFounded { .. } => "colony_founded",
            StaleOrder { .. } => "stale_order",
            Withdraw { .. } => "withdraw",
            WarStarted { .. } => "war_started",
            WarEnded { .. } => "war_ended",
            Story { .. } => "story",
            Resurgence { .. } => "resurgence",
            Revolt { .. } => "revolt",
            CoalitionFormed { .. } => "coalition_formed",
            CoalitionEnded { .. } => "coalition_ended",
        };
        *m.entry(label.to_string()).or_insert(0) += 1;
    }
    m
}
