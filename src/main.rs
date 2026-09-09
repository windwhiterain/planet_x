//! planet_x — a deterministic sandbox trajectory generator for the Planet X game.
//!
//! This is the **agent-facing** tool. It emits zero-noise machine-readable data
//! so an LLM can run a *story*: it produces a linear trajectory (one JSON object
//! per round, all values rounded to 2 decimals), plus the self-describing rules /
//! schema / narrative artifacts. There is **no in-process jq** and no interactive
//! query REPL — arbitrary queries over the trajectory are done with **external
//! `jq`** (a required runtime dependency) piped over the emitted JSON Lines.
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
//!              then one per round) as JSON Lines. This is the **trajectory body**:
//!              pipe it into external `jq` to query across time.
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
use planet_x::model::{GameConfig, State, SCHEMA_VERSION};
use planet_x::prng::Prng;
use planet_x::{sim, web, world};
use serde_json::json;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "planet_x",
    version,
    about = "行星X——太空沙盘轨迹生成器（agent 讲故事用）",
    long_about = "《行星X》是一个回合制太阳系沙盘轨迹生成器：每回合 = 1 个月，整个游戏由一个可确定复现的\n\
State 快照推进，全部数值由 config/game.ron 数据驱动、不硬编码。这个 CLI 是面向 agent 的\n\
零噪声轨迹生成器：无进程内 jq、无交互查询 REPL——任意查询用**外部 jq**（必选依赖）对着\n\
输出的 JSON Lines 做。\n\
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
- `planet_x --seed 42 --round 240 > traj.jsonl`   # 跑一段轨迹（JSON Lines）\n\
- `jq -s '[ .[] | {round, nship:(.ships|length)} ]' traj.jsonl`   # 任意时间轴查询\n\
- `planet_x --start s240.ron --apply steer.json --round 240`      # 分段续玩 + 定向\n\
- `planet_x --traj 240 | jq '.story'`                             # 一键拿叙事弧\n\
- 先 `--schema` 查字段、`--meta` 查规则，再写查询，避免凭记忆。",
    after_help = "agent 专用：stdout 只输出零噪声机器可读 JSON（无颜色/星图/表格/散文）。\n\
--round N 输出 N+1 行 JSON（回合 0 + N 回合）；--traj N 输出一个自包含 story pack；\n\
--meta/--schema/--story/--control 各自输出一个 JSON 值。查询一律用外部 jq 对输出做。"
)]
struct Cli {
    /// 确定性随机种子（数字，或 random / 随机）
    #[arg(long, default_value = "random", value_name = "SEED")]
    seed: String,

    /// 从指定的 checkpoint（state + RNG）.ron 文件开始
    #[arg(long, value_name = "PATH")]
    start: Option<PathBuf>,

    /// 运行 n 个回合并把每个回合输出为一行 JSON（回合 0 先），供外部 jq 查询
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
    /// 为一行 JSON。这是 agent 的规则字典；再用外部 jq 过滤。
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

    // Trajectory runs.
    if let Some(n) = cli.traj {
        run_trajectory(&mut state, &config, &mut rng, n, cli.save.as_deref());
        return;
    }
    if let Some(n) = cli.round {
        run_rounds(&mut state, &config, &mut rng, n, cli.save.as_deref());
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

/// Run `n` rounds, emitting one agent-view JSON object per round (round 0 first,
/// then one per round) as JSON Lines — the **trajectory body** that external `jq`
/// queries. Optionally persist a deterministic checkpoint (state + RNG position).
fn run_rounds(state: &mut State, config: &GameConfig, rng: &mut Prng, n: u32, save: Option<&Path>) {
    emit(&agent::render_state(state, config));
    for _ in 0..n {
        sim::advance(state, config, rng);
        emit(&agent::render_state(state, config));
    }
    if let Some(path) = save {
        if let Err(e) = save_checkpoint(path, state, rng) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_SAVE", "message": e.to_string()}));
            std::process::exit(10);
        }
    }
}

/// Run `n` rounds and emit ONE self-contained JSON document:
/// `{schema_version, meta, story, trajectory:[...round snapshots...]}` — a packaged
/// "story pack" with the timeline, the narrative arc and the rules in a single value.
fn run_trajectory(state: &mut State, config: &GameConfig, rng: &mut Prng, n: u32, save: Option<&Path>) {
    let mut snaps = vec![agent::state_json(state, config)];
    for _ in 0..n {
        sim::advance(state, config, rng);
        snaps.push(agent::state_json(state, config));
    }
    let pack = json!({
        "schema_version": SCHEMA_VERSION,
        "meta": agent::meta_value(config),
        "story": agent::story_value(state),
        "trajectory": snaps,
    });
    emit(&pack.to_string());
    if let Some(path) = save {
        if let Err(e) = save_checkpoint(path, state, rng) {
            eprintln!("{}", json!({"ok": false, "code": "ERR_SAVE", "message": e.to_string()}));
            std::process::exit(10);
        }
    }
}
