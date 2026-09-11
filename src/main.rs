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
//! Usage: `planet_x [--seed <s|random>] [--start <path.json>]`
//! `[--apply <file.json>] [--round <n>] [--traj <n>] [--save <path.json>]`
//! `[--meta] [--schema] [--control-schema] [--story] [--control] [--control-plan <faction>]`
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
//! * `--notables [N]` dump the **windowed history layer** (`State::notables`) — the events later
//!              logic needs to look back at within `history.notable_window` rounds (currently
//!              wars), each with a one-line `headline`. Expiry is by design, not loss, so this
//!              layer reports its window instead of a drop count.
//! * `--milestones [N]` dump the **long-lived milestone layer** (`State::milestones`) — the events
//!              later logic needs to look back at over **unbounded** past. By the current
//!              criterion (`GameEvent::salience`) **no variant qualifies**, so this is normally
//!              `count: 0`. That is the criterion working, not a bug: for "how did this city
//!              become what it is", use `--index` + `play/planet_xq` (`q.history()`).
//!              Optional `N` = only the most recent N entries.
//! * `--control` dump the editable control surface (control + scope) — the template
//!              an agent edits into an `--apply` diff.
//! * `--control-schema` dump a machine-readable JSON Schema for the control / `--apply`
//!              diff (auto-derived from the same patch structs `--apply` parses), so an
//!              agent knows what it may write: `control[]` per faction with
//!              ship_orders/budgets/invest_weights/build_weights/loyalty_budget/buildings,
//!              plus the optional `scope`. Mirrors `--schema` (which describes the read
//!              side), this one describes the write side.
//! * `--control-plan [<faction>]` compute a **cost → benefit preview** for a faction's
//!              current control surface: per-round production vs fleet upkeep vs governance,
//!              the commanded construction/investment budgets, the AI's conservative cap,
//!              the net flow, the fleet upkeep you can sustain, and how many rounds before
//!              insolvency. With no faction it runs all factions (a map keyed by faction id).
//!              A pure analytical dry-run (one real `advance` on a clone, no RNG consumed),
//!              so an agent sees the cost of a budget *before* committing it.
//!
//! Determinism: the same seed / checkpoint always reproduces the same trajectory.
//! The tool is purely a generator; the `planet_x_web` binary is the separate,
//! player-facing interactive UI (and is unaffected by these query changes).

use clap::{CommandFactory, Parser};
use planet_x::agent;
use planet_x::config::{load_checkpoint, load_config, load_initial, parse_seed, save_checkpoint};
use planet_x::model::{
    FactionId, GameConfig, GameEvent, RoundInputs, RoundState, RoundView, SCHEMA_VERSION, State,
};
use planet_x::prng::Prng;
use planet_x::{autocontrol, control, projection, sim, world};
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
【控制模型 = 指令】每势力有可控状态 State::control：ship_orders（Idle/Move/Follow/DockCity/\n\
Dock/Colonize；攻击与轰炸不需要行为，射程内自动发生）、doctrine/kiting（行为风格与风筝<->贴脸）、\n\
budget（投资预算）、invest_weights。每个叶子带 mode（三态）：\n\
Inherit（继承上层，缺省）| Auto（系统自动决策）| Player（玩家指令，系统只读）。State::scope 是一棵\n\
作用域树（全局→势力→天体→城市），决定某叶子由谁控制；沿链取第一个不是 Inherit 的层，全链\n\
继承则落到 Auto。agent 用 --apply 写 diff 定向故事。\n\
\n\
【讲故事流程】\n\
- `planet_x --seed 42 --round 240 --index out/`    # 跑一段轨迹 + 投影（lean 主流 + 索引表）\n\
- `planet_xq.load('out').facts`                    # 读主流；`q.join('ships', round=r)` 按 id join\n\
- `planet_x --start s240.json --apply steer.json --round 240`      # 分段续玩 + 定向\n\
- `planet_x --start s240.json --round 0 --index out/` # 续玩前先读「这局最近在打什么」\n\
  （`q.notables()` / `q.history()`；窗口层与里程碑层都在投影里，按 `salience` 列筛）\n\
- `planet_x --seed 42 --round 1000 --quiet --save end.json`        # 只要最终 state，不要逐回合轨迹\n\
- 先 `--schema` 查视图字段、`--control-schema` 查 --apply 能写啥、`--meta` 查规则；分析用\n\
  `--index` + `play/planet_xq`，别用 jq。",
    after_help = "agent 专用：stdout 只输出零噪声机器可读 JSON（无颜色/星图/表格/散文）。\n\
--round N 输出 N+1 行 JSON（回合 0 + N 回合；`--every K` 采样、`--quiet` 一行都不出）；\n\
--meta/--schema/--control-schema/--control/--control-plan [<faction>]/--derived 各自输出一个 JSON 值。\n\
分析用 --index + play/planet_xq（历史/编年史也在投影里，不必额外 dump 开关）。"
)]
struct Cli {
    /// 确定性随机种子（数字，或 random / 随机）
    #[arg(long, default_value = "random", value_name = "SEED")]
    seed: String,

    /// 从指定的 checkpoint（state + RNG）.json 文件开始
    #[arg(long, value_name = "PATH")]
    start: Option<PathBuf>,

    /// 运行 n 个回合并把每个回合输出为一行 JSON（回合 0 先），供外部分析（--index/--digest）
    #[arg(long = "round", value_name = "N")]
    round: Option<u32>,

    /// **静默推进**：与 --round 连用时不输出逐回合的完整状态（1000 回合 ≈ 40 MB 的那一份），
    /// 只推进世界。要留东西就配 `--save`（最终 state，0.09 MB）/`--digest K`（故事板）/
    /// `--index DIR`（投影，不受本开关影响）。
    #[arg(long)]
    quiet: bool,

    /// 把一份可控状态 diff（JSON，形同 web 的 POST /api/command：{control,scope}）
    /// 按键 overlay 到状态上，再执行 --round。用于 agent 定向故事。
    #[arg(long, value_name = "FILE")]
    apply: Option<PathBuf>,

    /// （--round 结束后）把当前状态连同 PRNG 位置写入一个 RON checkpoint，
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

    /// 输出可编辑控制面（control + scope）JSON——agent 写 --apply diff 的模板。
    #[arg(long)]
    control: bool,

    /// 输出 control/`--apply` diff 的机器可读 JSON Schema（由解析同一批补丁结构体
    /// 派生）：告诉 agent 能往 --apply 里写哪些字段、各自的类型/含义。与 --schema
    /// 互为镜像（--schema 描述 agent 视图，--control-schema 描述 agent 写入的 diff）。
    #[arg(long)]
    control_schema: bool,

    /// 给势力算**成本→收益预览**（当前控制面下的每回合经济平衡：产出 vs 舰队维护 vs
    /// 治理，含命令的建造/投资预算、AI 保守上限、净流、可养舰队上限、清算前剩余回合）。
    /// 缺省不带参数 = 给所有势力各出一份（map：势力名→剖面）；`--control-plan <faction>`
    /// 只出指定势力。纯分析（干跑一轮真实 advance、无 RNG 消耗），agent 据此在写 diff 前看到
    /// 代价，而不是提交后观崩盘。可和 --apply 连用：先叠加候选 diff 再看它的后果。
    #[arg(long, num_args = 0..=1, value_name = "FACTION")]
    control_plan: Option<Option<String>>,

    /// 降采样步长：与 --round 连用，每 K 回合取一个快照（1 = 每回合）。用于把超长轨迹
    /// 变成可读的粗粒度采样，避免几千行全量 JSON 撑爆上下文。
    /// ⚠ 它只管 **stdout 的轨迹**：`--index` 投影**不受它影响**（实测 `--every 10` 与全量
    /// 都是 169 MB / 8.6 s）——投影要的是逐回合数据，没有可降的采样。
    #[arg(long, value_name = "K", default_value_t = 1)]
    every: u32,

    /// 粗粒度「编年史窗口」：与 --round 连用，每 K 回合输出**一行语义摘要**（该窗口的
    /// 各势力军力/城市/实力占比、战争、事件类型计数、剧情节拍 id），而不是逐回合全量
    /// 快照——超长轨迹的「故事板」。`--digest 100 --round 3000` → 30 行。
    #[arg(long, value_name = "K")]
    digest: Option<u32>,

    /// 索引投影模式：与 --round N 连用，把 N+1 回合投影成一份**lean 主流**（main.jsonl，
    /// 每回合一行：eager 字段 + **本回合的视图 `view`** + id 数组）+ 按 id 索引的 **lazy 表**
    /// （idx/*.jsonl，ships/cities/bodies 的完整对象）+ **派生表**（idx/faction_process.jsonl、
    /// idx/city_process.jsonl、idx/control.jsonl、idx/scope.jsonl、idx/decisions.jsonl：本回合的
    /// 过程量与控制面，状态里没有）+ **agent 可读
    /// 的投影 schema**（schema.json，声明哪些字段 eager / 哪些 lazy / 哪些 derived 及其
    /// 表/key/join 列/列类型）。重型字段不再内联；agent 用 Python kit（play/planet_xq，
    /// uv 管理）按 id join。确定性：同 seed 复现字节一致。
    #[arg(long, value_name = "DIR")]
    index: Option<PathBuf>,

    /// 输出这一回合的**视图对**：`{round, source, pre, post, [note]}`。
    /// 两个槽都是 `RoundView`（同形）：`pre` = 回合开始时的世界，`post` = 回合结束时的世界
    /// **+ 本回合的过程量**（各势力/各城的产出、舰队维护费、治理成本与覆盖率、市场运费/承运费/
    /// 净进口，以及 AI 的判定流水——这些量不落持久状态，只有这里、`--index` 的过程量表和 web
    /// 的信息树能读到）。
    /// 从 `--start <ckpt>` 读的是**档里存的**那一对（与 `--index` 同一回合的值完全相同，
    /// 不做舍入）；没有档时按当前状态重算（此时 `pre` 与 `post` 相同、过程量全 0），`note` 会说明。
    #[arg(long)]
    derived: bool,
}

fn main() {
    // **裸调用**（一个参数都没有）= 想知道这东西能干什么 ⇒ 打 help 并正常退出。
    // 给了参数但没说要干什么（例如只有 `--seed 42`）仍然走下面那条机器可读的 ERR_USAGE
    // ——那条是 agent 的合同，不要用 help 把它盖掉。
    if std::env::args_os().len() == 1 {
        let _ = Cli::command().print_help();
        println!();
        return;
    }
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
    if cli.control_schema {
        emit(&control::control_schema_value().to_string());
        return;
    }

    // Build the world: from a checkpoint (state + RNG) or generated procedurally.
    //
    // `--derived`（以及 `--index`）要的是**档里存下来的**那一对 `pre`/`post`：`--derived`
    // 直接报它，`--index` 用它当**起点回合**的派生态（否则投影一份 checkpoint 会显示
    // 「全世界零产出/零维护」——那些量只有在它是回合结果时才存在）。其余路径继续用
    // `load_initial`（它同时接受 session checkpoint 与裸 state 两种档）。
    let seed = parse_seed(&cli.seed);
    let mut stored: Option<RoundState> = None;
    let (mut state, mut rng) = match &cli.start {
        Some(path) if cli.derived || cli.index.is_some() => match load_checkpoint(path) {
            Ok((rs, prng)) => {
                let s = rs.state.clone();
                stored = Some(rs);
                (s, prng)
            }
            // ⚠ 兜底到 `load_initial` 时**不要把第一个错吞掉**：档的格式错（比如 JSON 档里
            // 有个键读不回来）会被写成「初始状态解析失败: Expected opening `(` for struct State」
            // ——那句 RON 的报错是**兜底路径**发出来的，跟真正的原因没关系。两个错都报出来，
            // 谁在骗人一眼可见。
            Err(checkpoint_err) => match load_initial(path, seed) {
                Ok(got) => got,
                Err(initial_err) => {
                    eprintln!(
                        "{}",
                        json!({
                            "ok": false,
                            "code": "ERR_START",
                            "message": format!(
                                "{} 两种读法都失败。按 checkpoint 读：{checkpoint_err}；按裸 state 读：{initial_err}",
                                path.display()
                            ),
                        })
                    );
                    std::process::exit(10);
                }
            },
        },
        Some(path) => match load_initial(path, seed) {
            Ok(got) => got,
            Err(e) => {
                eprintln!("{}", json!({"ok": false, "code": "ERR_START", "message": e}));
                std::process::exit(10);
            }
        },
        None => (world::default_state(&config, seed), Prng::new(seed)),
    };

    // Overlay a control-state diff (agent steering) before anything runs.
    if let Some(path) = &cli.apply {
        match apply_diff(&mut state, &config, path) {
            Err(e) => {
                eprintln!(
                    "{}",
                    json!({"ok": false, "code": "ERR_APPLY", "message": e})
                );
                std::process::exit(10);
            }
            // 有叶片没落地 → 在 stderr 上报。**这是 agent 唯一能发现「命令其实
            // 没下达」的渠道**：stdout 必须保持零噪声的状态流，而丢弃的常见原因
            // （舰已战沉/改名、城已易主、building 下标换城）恰恰是必须知道的那种。
            // 静默即成功，所以只在真的丢了东西时才说话。
            Ok(report) => {
                if !report.is_clean() {
                    eprintln!(
                        "{}",
                        json!({
                            "ok": true,
                            "code": "WARN_APPLY_SKIPPED",
                            "applied": report.applied,
                            "skipped": report.skipped,
                            "hint": "some diff leaves did not land; the diff itself is valid, the entities it names are not (stale ship/city names, wrong faction, building index from another city).",
                        })
                    );
                }
                // 「写值即接管」的回执：你只写了值、没写 mode，那些叶片从此归你（系统不再改写）。
                // 这不是错误，但 agent 必须知道——它决定了下一回合谁在动它们。
                if !report.took_over.is_empty() {
                    eprintln!(
                        "{}",
                        json!({
                            "ok": true,
                            "code": "NOTE_APPLY_TOOKOVER",
                            "applied": report.applied,
                            "took_over": report.took_over,
                            "hint": "writing a value without `mode` means `mode: Player` (the system stops overwriting that leaf). Pass an explicit mode (`Auto` / `Inherit`) if you only meant to nudge the recorded value.",
                        })
                    );
                }
                // 「删叶」的回执：这些叶片被真的删掉了 ⇒ 有效值**换了来源**（逐舰风格回出厂快照 /
                // 舰队默认回「没有说话」）。删一片本来就不存在的叶是幂等的，不进这个列表。
                if !report.removed.is_empty() {
                    eprintln!(
                        "{}",
                        json!({
                            "ok": true,
                            "code": "NOTE_APPLY_REMOVED",
                            "applied": report.applied,
                            "removed": report.removed,
                            "hint": "those control leaves are gone: what they used to supply now comes from the next layer (per-ship style falls back to the factory record, a fleet default falls back to the scope chain).",
                        })
                    );
                }
            }
        }
    }

    // State-reflecting dumps.
    //
    // `--derived`：这一回合引擎到底算出/消费了什么。
    // **两面分工（B5）**：`pre` = **输入面**（本回合掷出的随机数 + 判定输入，`RoundInputs`）；
    // `post` = **结算面**（回合末的观测 + 本回合过程量，`RoundView`）。两者形状不同是**有意的**：
    // 从前 `pre` 是「回合开始时的观测副本」，与上一回合的 `post` 逐字段相同、零信息量。
    // 有档就报**档里存的**那一对——于是它与 `--index` 的过程量表对同一回合给同一个值
    // （有 `tests/projection_derived.rs` 把这条钉住）；没档（或叠加了 `--apply`，此时状态已变）
    // 只能按当前状态重算，那种情况下没有流量、也没有掷过骰子，`note` 明说，别让人误以为流水为零。
    if cli.derived {
        let from_checkpoint = stored.is_some() && cli.apply.is_none();
        let (pre, post) = match (&stored, cli.apply.is_some()) {
            (Some(rs), false) => (rs.pre.clone(), rs.post.clone()),
            (Some(rs), true) => (rs.pre.clone(), sim::view_from_state(&state, &config)),
            (None, _) => (RoundInputs::default(), sim::view_from_state(&state, &config)),
        };
        let mut v = json!({
            "round": state.round,
            "source": if from_checkpoint { "checkpoint" } else { "state" },
            "pre": pre,
            "post": post,
        });
        if !from_checkpoint {
            v["note"] = json!(
                "这份视图是**按当前状态重算**的，没有档存的「本回合过程量」、也没有本回合的输入面\
                 （掷出的随机数只存在于回合中段）。`post` 里的产出、维护费、治理与判定流水全是 0 / 空，\
                 `pre` 是空的。要看真正的过程量请用 `--index` 投影的过程量表（`faction_process` / \
                 `city_process` / `round_inputs` …），或 `--save` 出来的 checkpoint（它带真正的两面）。"
            );
        }
        emit(&v.to_string());
        return;
    }
    if cli.control {
        emit(&control::control_surface(&state, &config).to_string());
        return;
    }
    match &cli.control_plan {
        Some(Some(fid)) => match autocontrol::control_plan(&state, &config, fid) {
            Some(v) => emit(&v.to_string()),
            None => {
                eprintln!(
                    "{}",
                    json!({"ok": false, "code": "ERR_PLAN_FACTION", "message": format!("--control-plan 未知势力: {fid}（用 --control 或 --schema 看有哪些势力）")})
                );
                std::process::exit(10);
            }
        },
        Some(None) => {
            emit(&json!({"factions": autocontrol::control_plan_all(&state, &config)}).to_string())
        }
        None => {}
    }
    if cli.control_plan.is_some() {
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
        // 从 checkpoint 起跑时，把档里那一对派生态交给投影当**起点回合**的行（见
        // `projection::write_index_seeded`）：那一行的 state 就是那一回合的结果，所以
        // 「产出/维护/治理」应当是那一回合的数，而不是被抹成 0。
        let start_derived = stored.as_ref().map(|rs| (rs.post.clone(), rs.pre.clone()));
        let outcome = match projection::write_index_seeded(
            &mut state,
            &config,
            &mut rng,
            n,
            dir,
            start_derived,
        ) {
            Ok(o) => o,
            Err(e) => {
                eprintln!(
                    "{}",
                    json!({"ok": false, "code": "ERR_INDEX", "message": e})
                );
                std::process::exit(10);
            }
        };
        // 存一份**没丢过程量**的档：`pre`/`post` 直接取投影收尾回合的那一对。此前这里重新
        // `view_from_state`，把本回合的过程量存成了 0——于是同一回合的两个读面
        // （`--index` 的过程量表 vs 档里的 post）会各说各话。
        let round_state = RoundState {
            schema_version: SCHEMA_VERSION,
            state: state.clone(),
            pre: outcome.pre,
            post: outcome.post,
        };
        save_if_requested(cli.save.as_deref(), &round_state, &rng);
        return;
    }

    // Trajectory runs.
    if let Some(n) = cli.round {
        run_rounds(
            &mut state,
            &config,
            &mut rng,
            n,
            cli.every,
            cli.digest,
            cli.quiet,
            cli.save.as_deref(),
        );
        return;
    }

    eprintln!(
        "{}",
        json!({"ok": false, "code": "ERR_USAGE", "message": "nothing to do: specify --round N, or a dump flag (--meta/--schema/--control-schema/--control/--control-plan [<faction>]/--derived). 裸调用（不带任何参数）会打 help。"})
    );
    std::process::exit(10);
}

/// Read a control-state diff file (JSON, same shape as `POST /api/command`,
/// i.e. `{control:[...], scope:{...}}`) and overlay it onto `state` by key.
/// Returns what landed and what was dropped (see [`control::ApplyReport`]).
fn apply_diff(
    state: &mut State,
    config: &GameConfig,
    path: &Path,
) -> Result<control::ApplyReport, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    control::apply_patch(state, config, &value)
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
/// per round sampled at `every` (downsampling: rounds 0, K, 2K, …) — unless `quiet`,
/// in which case the world advances and **nothing** is printed (the point: 1000 回合
/// 的全量轨迹 ≈ 40 MB，而我只要最终 state / 一层摘要)。Each line is JSON Lines
/// queryable by external `jq`. Optionally persist a deterministic checkpoint (state + RNG).
fn run_rounds(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    n: u32,
    every: u32,
    digest: Option<u32>,
    quiet: bool,
    save: Option<&Path>,
) {
    if let Some(window) = digest {
        run_digest(state, config, rng, n, window, save);
        return;
    }
    let every = every.max(1);
    if !quiet {
        emit(&agent::render_state(
            state,
            &sim::view_from_state(state, config),
        )); // round 0 / start
    }
    let mut last_pre = RoundInputs::default();
    let mut last_post = sim::view_from_state(state, config);
    for _ in 0..n {
        // `pre` = 本回合**消费掉**的输入（掷出的随机数）；`post` = 本回合**结算出来**的观测。
        last_post = sim::advance_round(state, config, rng, &mut last_pre);
        if !quiet && state.round % every == 0 {
            emit(&agent::render_state(state, &last_post));
        }
    }
    save_if_requested(
        save,
        &RoundState {
            schema_version: SCHEMA_VERSION,
            state: state.clone(),
            pre: last_pre,
            post: last_post,
        },
        rng,
    );
}

fn save_if_requested(save: Option<&Path>, round_state: &RoundState, rng: &Prng) {
    if let Some(path) = save {
        if let Err(e) = save_checkpoint(path, round_state, rng) {
            eprintln!(
                "{}",
                json!({"ok": false, "code": "ERR_SAVE", "message": e.to_string()})
            );
            std::process::exit(10);
        }
    }
}

/// Emit one **semantic digest** line per `window`-round window: a coarse
/// per-window summary (each faction's city/ship/fleet totals + power share, the
/// active wars, event-type counts, and the story beats fired). This is the
/// "storyboard" an agent reads for a very long run without wading through
/// thousands of full snapshots. Only complete windows are emitted.
fn run_digest(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    n: u32,
    window: u32,
    save: Option<&Path>,
) {
    let window = window.max(1);
    let mut win_start = state.round;
    let mut events_acc: Vec<GameEvent> = Vec::new();
    let mut prod_acc: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut story_idx = state.chronicle.len();
    let mut last_pre = RoundInputs::default();
    let mut last_post = sim::view_from_state(state, config);
    for _ in 0..n {
        let derived = sim::advance_round(state, config, rng, &mut last_pre);
        events_acc.extend(state.events.iter().cloned());
        // 累计本窗口各方产出（窗口级总开采价值），让 digest 的 `production` 是**窗口总量**，
        // 而非某一点时值。只累有产出的势力（空表 = 这回合没开采，不必在账上多一行 0）。
        for (fid, row) in &derived.factions {
            if row.production.is_empty() {
                continue;
            }
            let v: f64 = row
                .production
                .iter()
                .map(|(k, amt)| amt * config.resources.get(k).map(|r| r.value).unwrap_or(1.0))
                .sum();
            *prod_acc.entry(fid.clone()).or_default() += v;
        }
        if state.round - win_start >= window {
            let story: Vec<String> = state.chronicle[story_idx..]
                .iter()
                .map(|c| c.id.clone())
                .collect();
            // 窗口末态的观测/总结直接取自 `advance` 已算好的那份 `RoundView`
            // （与逐回合 agent 视图同源），不再在 digest 里独立重算一遍。
            let view = &derived;
            emit(
                &digest_value(
                    state,
                    view,
                    &prod_acc,
                    win_start,
                    state.round,
                    &events_acc,
                    &story,
                )
                .to_string(),
            );
            win_start = state.round;
            events_acc.clear();
            prod_acc.clear();
            story_idx = state.chronicle.len();
        }
        last_post = derived;
    }
    save_if_requested(
        save,
        &RoundState {
            schema_version: SCHEMA_VERSION,
            state: state.clone(),
            pre: last_pre,
            post: last_post,
        },
        rng,
    );
}

fn r2(v: f64) -> f64 {
    // + 0.0 把 IEEE 的 -0.0 规整成 +0.0（round 保留负零），避免 agent 看到 `-0.0`。
    (v * 100.0).round() / 100.0 + 0.0
}

/// The coarse per-window summary value. State-derived fields (world totals, per-faction
/// city/ship/fleet, power share, hegemon/coalition/sanction, wars) are taken directly from
/// the [`RoundView`] `advance` computed at this window's end — the same numbers
/// the per-round agent view carries — so the digest can never drift from the simulation.
/// Only the window-accumulated fields (event counts, story beats) are built here.
fn digest_value(
    state: &State,
    view: &RoundView,
    production: &BTreeMap<FactionId, f64>,
    from: u32,
    to: u32,
    events_acc: &[GameEvent],
    story: &[String],
) -> serde_json::Value {
    let factions: Vec<serde_json::Value> = view
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
            // 键名保持 `cities`/`ships`（digest 是**给窗口级故事板读的粗粒度摘要**，
            // 键名不必跟视图内部改名——保持它，digest 的字节基线才不动）。
            "cities": view.city_count,
            "ships": view.ship_count,
            "fleet_value": r2(view.fleet_value),
            "population": view.population,
        },
        "factions": factions,
        "power_share": view.power_share.iter().map(|(k, v)| (k.clone(), r2(*v))).collect::<BTreeMap<_, _>>(),
        "hegemon": view.hegemon,
        "coalition_members": view.coalition_members,
        "sanctioned": view.sanctioned,
        "wars": view.wars,
        "events": event_counts(events_acc),
        "top_events": top_events(events_acc),
        "story": story,
    })
}

/// 一个窗口内**最重要**的若干条事件，渲染成 `{round, headline}`——digest 的「故事板」。
///
/// 这里**刻意不按 [`Salience`] 过滤**：分层判据是「后续计算需要访问哪一段历史」，与「人读起来
/// 重不重要」无关——按它过滤会让故事板在里程碑层清空后**静默变空**。挑选改用
/// [`GameEvent::weight`]（**纯显示用**的排序键：穷尽 match 声明、不进 config，正是因为它只服务
/// 于「给人看」）。窗口内多于 [`TOP_EVENTS`] 条时取最重的若干条，**展示顺序仍按时间**
/// （挑选用权重，读起来用时间）；被略过的条数如实给出，不假装这就是全部。
///
/// 这里只做「挑选 + 渲染」，句子本身来自唯一的 [`GameEvent::headline`]——CLI、投影表的
/// `headline` 列、以及 agent 读到的都是同一句话。
fn top_events(events: &[GameEvent]) -> serde_json::Value {
    let mut picked: Vec<usize> = (0..events.len()).collect();
    let total = picked.len();
    // 稳定排序：权重降序，同权重保持原有的时间先后。
    picked.sort_by_key(|&i| std::cmp::Reverse(events[i].weight()));
    picked.truncate(TOP_EVENTS);
    // 展示按时间序（挑选用权重，读起来用时间）。
    picked.sort_unstable();
    let shown: Vec<serde_json::Value> = picked
        .iter()
        .map(|&i| json!({"headline": events[i].headline()}))
        .collect();
    json!({"total": total, "shown": shown.len(), "skipped": total - shown.len(), "events": shown})
}

/// `--digest` 每个窗口最多列几条事件（见 [`top_events`]）。
const TOP_EVENTS: usize = 24;

/// Count the GameEvent variants in a window, keyed by their `type` label.
fn event_counts(events: &[GameEvent]) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    for e in events {
        // 类型名走单一权威 `GameEvent::kind()`（此前这里手抄了一份 variant → label 的
        // match，既与 serde 判别式有漂移风险，也会在新增 variant 时漏掉）。
        *m.entry(e.kind().to_string()).or_insert(0) += 1;
    }
    m
}
