//! Indexed projection: emit a **lean per-round main stream** plus **id-indexed lazy tables**
//! for the heavy entity collections.
//!
//! The problem this solves: future object fields will carry huge data (a ship's full
//! components/hull history, a city's building list, a body's settlements). Inlining them every
//! round bloats the stream. So the emitter splits the world into:
//!
//! * **eager fields** — inline in `main.jsonl` (one lean fact row per round): `round`,
//!   `time_month`, `events`, `chronicle`, `metrics` (the summary), plus the id-arrays
//!   `ship_ids` / `city_ids` / `body_ids`.
//! * **lazy fields** — NOT inline. The main row only carries the id-array; the full objects
//!   live in a table keyed by id (`idx/ships.jsonl`, `idx/cities.jsonl`, `idx/bodies.jsonl`).
//!   The agent fetches them via the Python kit (`planet_xq`) and joins by id.
//!
//! The projection schema (`schema.json`) is written to be **agent-readable**: it declares which
//! fields are eager vs lazy, each lazy field's table / key / id-column / whether it is per-round,
//! and the column types — so an agent can navigate without reverse-engineering the JSON. It is
//! data-driven: add a heavy field to [`LAZY`] and both the emitted tables and the schema follow.
//!
//! Determinism: the emitter only reads the state; the same seed reproduces byte-identical files.

use crate::model::*;
use crate::prng::Prng;
use crate::sim;
use serde_json::json;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

const MAIN: &str = "main.jsonl";
const SCHEMA: &str = "schema.json";
const META: &str = "meta.json";
const IDX_DIR: &str = "idx";

/// Round a float to 2 decimals (token-noise reduction); `+ 0.0` normalizes IEEE `-0.0`.
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0 + 0.0
}

fn idx_file(name: &str) -> String {
    format!("{IDX_DIR}/{name}.jsonl")
}

/// 事件的稳定 id：`"<round>:<seq>"`（`seq` = 本回合内的事件序号）。可排序、确定性、
/// 不需要额外存储；Python 侧用它做 join 键与因果链引用。
fn event_id(round: u32, seq: usize) -> String {
    format!("{round}:{seq}")
}

/// Declaration of one lazy field: which table holds it, how it is keyed, whether it is a
/// per-round snapshot (vs a global master table), and the main-stream id-array column.
struct LazyField {
    name: &'static str,
    table: &'static str,
    key: &'static str,
    id_col: &'static str,
    round: bool,
}

/// The heavy fields that are indexed rather than inlined. Add a field here and the emitter +
/// schema follow automatically. `bodies`/`settlements` are `round == false`: global master tables
/// (a body's orbit and its settlements are essentially static), so they are written once and
/// joined by `body_id` / `settlement_id`.
const LAZY: &[LazyField] = &[
    LazyField { name: "ships", table: "idx/ships.jsonl", key: "ship_id", id_col: "ship_ids", round: true },
    LazyField { name: "cities", table: "idx/cities.jsonl", key: "city_id", id_col: "city_ids", round: true },
    LazyField { name: "factions", table: "idx/factions.jsonl", key: "faction_id", id_col: "faction_ids", round: true },
    // 承包挂单簿：**只留未完成的单子**（等人接的 + 正在履行的），所以它是「此刻在市场上
    // 的运力需求」的权威读面。完成/收回的单子不在这里——它们只留在 `events` 里。
    LazyField { name: "contracts", table: "idx/contracts.jsonl", key: "contract_id", id_col: "contract_ids", round: true },
    // 事件历史：**归一化**的一行一事件（固定列 + 统一参与方槽位），取代此前内联在
    // main.jsonl 里的「serde 直接摊开的 tagged enum」——那种表 74.8% 的单元格是 null、
    // 且 `from`/`to` 一列两义（city_defected 是势力、capital_relocated 是天体）。
    LazyField { name: "events", table: "idx/events.jsonl", key: "event_id", id_col: "event_ids", round: true },
    LazyField { name: "bodies", table: "idx/bodies.jsonl", key: "body_id", id_col: "body_ids", round: false },
    LazyField { name: "settlements", table: "idx/settlements.jsonl", key: "settlement_id", id_col: "settlement_ids", round: false },
];

/// 一棵**派生表**的声明：不是状态里的重型字段，而是引擎算出来的量，agent 必须能 join
/// （`flow`：本回合的产出/维护/治理中间量；`control`/`scope`：控制面的 tidy 行）。
///
/// 与 [`LAZY`] 的区别只有一处：lazy 字段的重型对象**不内联**、靠 main 的 id 数组 join；
/// 派生表的数据**根本不在状态里**（`RoundFlow` 不落持久状态），只能由引擎产出。
struct DerivedTable {
    name: &'static str,
    table: &'static str,
    key: &'static str,
    /// 与 `main.jsonl` 的哪一列 join（`""` = 世界级，每回合一组行，不需要 id 数组）。
    join_on: &'static str,
    round: bool,
}

/// 派生表清单。加一张表要同时改三处：这里、`write_round` 的发射、[`projection_schema`] 的
/// `derived` 条目（测试会断言三者一致）。
const DERIVED: &[DerivedTable] = &[
    DerivedTable { name: "flow", table: "idx/flow.jsonl", key: "faction_id", join_on: "faction_ids", round: true },
    DerivedTable { name: "city_flow", table: "idx/city_flow.jsonl", key: "city_id", join_on: "city_ids", round: true },
    DerivedTable { name: "control", table: "idx/control.jsonl", key: "key", join_on: "faction_ids", round: true },
    DerivedTable { name: "scope", table: "idx/scope.jsonl", key: "key", join_on: "", round: true },
    DerivedTable { name: "decisions", table: "idx/decisions.jsonl", key: "actor", join_on: "faction_ids", round: true },
    // **舰船设计图库**（势力级）：一行 = 一张图。设计图是**结构叶**（`{class, components[],
    // order{}}`），塞进 `control` 表的通用 `value: any` 列会让列类型不稳、Python 侧还要
    // 二次解析 —— 所以给它一张有类型列的专用表（`engine-data-plane.md` §1 的「引擎给答案、
    // Python 只筛」）。⚠ `control` 派生表**不发** `kind="blueprint"` 的行（两份表示 = 漂移
    // 风险）：设计图只住这张表，`control` 表的描述里也写明了这一点。
    DerivedTable { name: "blueprints", table: "idx/blueprints.jsonl", key: "blueprint_id", join_on: "faction_ids", round: true },
];

/// 投影的全部写出端，一次建好再传进 [`write_round`]（参数已经太多，别再往签名里塞）。
struct Writers {
    main: BufWriter<File>,
    events: BufWriter<File>,
    ships: BufWriter<File>,
    cities: BufWriter<File>,
    factions: BufWriter<File>,
    contracts: BufWriter<File>,
    flow: BufWriter<File>,
    city_flow: BufWriter<File>,
    control: BufWriter<File>,
    scope: BufWriter<File>,
    decisions: BufWriter<File>,
    blueprints: BufWriter<File>,
    bodies: BufWriter<File>,
    settlements: BufWriter<File>,
}

impl Writers {
    fn create(dir: &Path) -> Result<Self, String> {
        let open = |name: &str| -> Result<BufWriter<File>, String> {
            File::create(dir.join(idx_file(name))).map(BufWriter::new).map_err(|e| e.to_string())
        };
        Ok(Self {
            main: BufWriter::new(File::create(dir.join(MAIN)).map_err(|e| e.to_string())?),
            events: open("events")?,
            ships: open("ships")?,
            cities: open("cities")?,
            factions: open("factions")?,
            contracts: open("contracts")?,
            flow: open("flow")?,
            city_flow: open("city_flow")?,
            control: open("control")?,
            scope: open("scope")?,
            decisions: open("decisions")?,
            blueprints: open("blueprints")?,
            bodies: open("bodies")?,
            settlements: open("settlements")?,
        })
    }

    fn flush_all(&mut self) -> Result<(), String> {
        for w in [
            &mut self.main, &mut self.events, &mut self.ships, &mut self.cities, &mut self.factions,
            &mut self.contracts,
            &mut self.flow, &mut self.city_flow, &mut self.control, &mut self.scope,
            &mut self.blueprints, &mut self.bodies, &mut self.settlements,
        ] {
            w.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// 投影产出的**收尾态**：本回合的 `pre`（推进前的观测）与 `post`（推进后的观测 + 流量）。
///
/// 返回它们是为了 `--index --save` 能存下一份**没丢掉流量**的 checkpoint——否则
/// `--index` 路径存出来的档里 `post.flow` 是空的，「同一回合两个读面各说各话」。
pub struct IndexOutcome {
    pub pre: Derived,
    pub post: Derived,
}

/// Emit the index projection of `rounds` rounds (round 0 then `rounds` steps) into `dir`.
/// Round 0 (the start state) uses an empty [`RoundFlow`] (no production yet); each later round
/// uses the flow `sim::advance` captured.
pub fn write_index(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    rounds: u32,
    dir: &Path,
) -> Result<IndexOutcome, String> {
    write_index_seeded(state, config, rng, rounds, dir, None)
}

/// [`write_index`]，但允许把**起点回合的派生态**交进来。
///
/// `--start <ckpt> --index` 时档里存着**产生当前状态的那一回合**的派生态：那一行的 state 就是
/// 那一回合的结果，所以用档里的 `post` 比用 `derived_from_state`（`flow` 恒空、`metrics` 里
/// 的产出/维护/治理全被抹成 0）**更真**——否则「投影一份 checkpoint」会让 agent 看到「全世界
/// 零产出、零维护」，而真相是这些量只在它是回合结果时才有。全新开局（`--seed`）没有这一对，
/// 传 `None`（回合 0 就是初始世界，没有流量）。
pub fn write_index_seeded(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    rounds: u32,
    dir: &Path,
    start: Option<Derived>,
) -> Result<IndexOutcome, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir.join(IDX_DIR)).map_err(|e| e.to_string())?;
    fs::write(dir.join(SCHEMA), projection_schema().to_string()).map_err(|e| e.to_string())?;
    // Static rules dictionary (same renderer as `--meta`), so `load(dir)` = world + rules +
    // join helpers in one directory; the Python kit reads it into `q.meta` + spec tables.
    fs::write(dir.join(META), crate::agent::meta_value(config).to_string()).map_err(|e| e.to_string())?;

    let mut w = Writers::create(dir)?;

    // Global master table: body identity (name/orbit/settlements) — once, at round 0.
    for b in &state.bodies {
        let o = &b.orbit;
        writeln!(
            w.bodies,
            "{}",
            json!({
                "body_id": b.name.clone(),
                "name": b.name,
                "perihelion_distance": o.perihelion_distance,
                "aphelion_distance": o.aphelion_distance,
                "period": o.period,
                // 母天体 id：None = 环绕太阳（日心行星），Some(名) = 该天体的卫星。
                "parent": o.parent,
                "x": r2(b.position[0]),
                "y": r2(b.position[1]),
                "settlement_count": b.settlements.len(),
            })
        )
        .map_err(|e| e.to_string())?;
    }

    // Global master table: each body's 定居点 (site name/area/capacity/resources) — once. The
    // heavy site collection is not inlined into `bodies`; the agent joins `settlements` by id.
    for b in &state.bodies {
        for (i, s) in b.settlements.iter().enumerate() {
            writeln!(
                w.settlements,
                "{}",
                json!({
                    "settlement_id": s.name.clone(),
                    "body_id": b.name.clone(),
                    "index": i,
                    "name": s.name.clone(),
                    "total_area": s.total_area,
                    "ecological_capacity": s.ecological_capacity,
                    "construction_speed_mod": s.construction_speed_mod,
                    "construction_resource_mod": s.construction_resource_mod,
                    "resources": s.resources,
                })
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // Round 0 (start state) then each advancing round.
    //
    // 起点那一行的派生态：有档就用档里那一对（见 [`write_index_seeded`]），没有就按当前状态重算。
    let (mut pre, mut post) = match start {
        Some(s) => (s.clone(), s),
        None => {
            let d = sim::derived_from_state(state, config);
            (d.clone(), d)
        }
    };
    write_round(&mut w, state, config, &post)?;
    for _ in 0..rounds {
        // `pre` = 本回合开头的观测（随机决策尚未落地）；`post` = 结尾的观测 + 本回合流量。
        pre = sim::derived_from_state(state, config);
        post = sim::advance(state, config, rng);
        write_round(&mut w, state, config, &post)?;
    }

    w.flush_all()?;
    Ok(IndexOutcome { pre, post })
}

/// Write one round's main line, event rows, ship rows, city rows, faction rows, and the
/// derived tables (`flow` / `city_flow` / `control` / `scope`).
fn write_round(
    w: &mut Writers,
    state: &State,
    config: &GameConfig,
    derived: &Derived,
) -> Result<(), String> {
    let metrics = &derived.metrics;
    let row = json!({
        "round": state.round,
        "time_month": r2(state.time_month),
        "chronicle": state.chronicle,
        "metrics": metrics,
        "event_ids": (0..state.events.len())
            .map(|i| event_id(state.round, i))
            .collect::<Vec<_>>(),
        "ship_ids": state.ships.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        // **全部**城（含已夷平的空白城）：投影表 `cities` 里本来就有 razed 行，这里若把
        // razed 城筛掉，`q.join('cities')` 就会与 `q.cities()` 不一致——而「被夷平的城」
        // 恰恰是历史查询最关心的实体。是否算「活城」交给查询方看 `razed` 列。
        "city_ids": state.cities.iter().map(|c| c.name.clone()).collect::<Vec<_>>(),
        "faction_ids": state.factions.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        // 承包挂单簿（join `contracts` 表用）。挂单号是 `u64`（不是实体名），所以这里是
        // 数字数组——与本表其它 id 数组（都是名字）不同，别把它当实体 id 用。
        "contract_ids": state.contracts.contracts.iter().map(|c| c.id).collect::<Vec<_>>(),
        "body_ids": state.bodies.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
        "settlement_ids": state.bodies
            .iter()
            .flat_map(|b| b.settlements.iter().map(|s| s.name.clone()))
            .collect::<Vec<_>>(),
    });
    writeln!(w.main, "{row}").map_err(|e| e.to_string())?;

    // 本回合事件的**归一化行**（见 `GameEvent::history_row`）：固定列、无同名多义，
    // 参与方在统一的 (actor/target/extra) 槽位里。Python 侧因此可以按任意实体 join
    // 「这座城 / 这艘舰 / 这个势力的历史」，而不需要知道任何 variant 的字段布局。
    for (i, e) in state.events.iter().enumerate() {
        let h = e.history_row();
        // 行内容以 `EventRow` 的**序列化结果**为准（唯一来源）：将来给 `EventRow` 加字段，
        // 表里会自动多一列——不会像手写 `json!` 那样悄悄漏掉（本列 `headline` 就是这么
        // 差点漏掉的）。投影自己只需补几个键，并把 `magnitude` 规整到两位小数。
        let mut row = serde_json::to_value(&h).map_err(|e| e.to_string())?;
        let obj = row.as_object_mut().ok_or("EventRow 必须是 JSON 对象")?;
        obj.insert("round".to_string(), json!(state.round));
        obj.insert("seq".to_string(), json!(i));
        obj.insert("event_id".to_string(), json!(event_id(state.round, i)));
        obj.insert("magnitude".to_string(), json!(r2(h.magnitude)));
        // `weight` 是**纯显示用**的排序键（`GameEvent::weight`，穷尽 match 声明、不进 config）。
        // 它必须投影出来：`salience` 是**分层判据**（后续计算要访问哪段历史），**不是重要性**——
        // 按它挑「值得读的事件」会挑错（里程碑层清空后会挑到空集）。给人看的排序走这一列。
        obj.insert("weight".to_string(), json!(e.weight()));
        writeln!(w.events, "{row}").map_err(|e| e.to_string())?;
    }

    for s in &state.ships {
        let p = ship_panel(config, s);
        writeln!(
            w.ships,
            "{}",
            json!({
                "round": state.round,
                "ship_id": s.name.clone(),
                "faction_id": s.faction_id,
                "class": s.class,
                "name": s.name,
                "x": r2(s.position[0]),
                "y": r2(s.position[1]),
                "hull": r2(s.hull),
                "hull_max": r2(s.hull_max),
                "shield": r2(s.shield),
                "shield_max": r2(s.shield_max),
                "velocity": r2(s.velocity),
                "components": s.components,
                "component_hp": s.component_hp.iter().map(|v| r2(*v)).collect::<Vec<_>>(),
                "attack": r2(p.attack),
                "attack_range": r2(p.attack_range),
                "speed": r2(p.speed),
                "accel": r2(p.accel),
                "hardness": r2(p.hardness),
                "intercept": r2(p.intercept),
                "shield_regen": r2(p.shield_regen),
                "hull_regen": r2(p.hull_regen),
                "upkeep": r2(p.upkeep),
                // —— 指令归属的**引擎解析结果**（别让 Python 自己重实现链：那是漂移源）——
                // `order_leaf_mode`：本舰叶片自己的表态（没有叶片 = Inherit）；
                // `order_default_mode`：势力级舰队默认的表态；
                // `order_effective_mode`：`State::ship_control` 的答案（叶 → 默认 → 势力 → 全局）；
                // `order_effective`：`State::ship_behavior` 的答案（有效指令；null = 无人说话）。
                "order_leaf_mode": leaf_mode_of(state, &s.faction_id, &s.name),
                "order_default_mode": default_mode_of(state, &s.faction_id),
                "order_effective_mode": state.ship_control(s.name.clone()),
                "order_effective": state.ship_behavior(s.name.clone()),
                // —— 设计图（出厂规格）在**这艘舰**上的三个读数 ——
                // `blueprint`：本舰出厂所用图名（快照的溯源；null = 无图）。
                // `blueprint_mode`：那张图**在势力库里的叶表态**（缺图 = Inherit）。
                // `order_blueprint_mode`：图给新舰的默认意图（`order`）那一层的表态：
                //   缺图 / 图上没写 order = Inherit（= 这一层没有说话）。⚠ 它是**图叶自己**
                //   的表态，不是链解析结果——所以你可能在这里看到 Inherit，而
                //   `order_effective_mode` 却是 Player（那来自舰队默认叶或作用域）。
                "blueprint": s.blueprint,
                "blueprint_mode": blueprint_mode_of(state, &s.faction_id, s.blueprint.as_deref()),
                "order_blueprint_mode": order_blueprint_mode_of(state, &s.faction_id, s.blueprint.as_deref()),
                // **这条有效意图是谁供的值**（Q2=(b) 的出处列）：leaf / blueprint:<图名> /
                // fleet_default / scope / record。`scope`/`record` 在**指令链上不会出现**
                // （作用域不携带值、指令没有出厂记录值），见 `column_docs`。
                // ⚠ 它把「叶**不存在**」与「叶写着 `Inherit`」分开报：后者报 `leaf`
                // （那时值真的来自那片叶），前者才可能落到 `blueprint:*`/`fleet_default`。
                "order_source": state.ship_behavior_source(s.name.clone()).map(|src| src.label()),
                // 下水回合（编制表的确定性 tie-break：「同分取最老的」）。旧档缺字段 ⇒ null
                // = **未知**（读者要回落名字序，不能当成第 0 回合）。
                "spawned_round": s.spawned_round,
                // 三条**风格轴**的有效值（叶 → 舰队默认 → 舰上记录值）。风格是活层，
                // `Ship.doctrine`/`Ship.kiting`/`Ship.freighter` 只是记录值——这里给的是
                // 引擎解析后的答案。第三条轴（角色）与前两条的唯一差别：**AI 会写它**
                // （按积压定编），所以 `freighter_mode` 还会告诉你那片叶归谁。
                "doctrine": state.ship_doctrine(s.name.clone()),
                "kiting": state.ship_kiting(s.name.clone()),
                "freighter": state.ship_freighter(s.name.clone()),
                "freighter_mode": state.ship_freighter_control(s.name.clone()),
            })
        )
        .map_err(|e| e.to_string())?;
    }

    // 资源价值（与 `sim` 同一把尺子）：产地货栈按它折算成可比的价值列。
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    for c in &state.cities {
        let deployed: f64 = c.buildings.iter().map(|b| b.deployed).sum();
        let buildings: Vec<serde_json::Value> = c
            .buildings
            .iter()
            .map(|b| {
                json!({
                    "id": b.id,
                    "kind": b.kind,
                    "resource": b.resource,
                    "ship_type": b.ship_type,
                    // **设计图指针**（原样输出：指向一张已被删除/改名的图时，它照样出现在这里
                    // ——配合「进度停攒」你就能一眼看出「这个区停产了，因为图没了」，见 Q10(a)）。
                    "blueprint": b.blueprint,
                    "structure": b.structure,
                    "area": r2(b.area),
                    "deployed": r2(b.deployed),
                    "armor": r2(b.armor),
                })
            })
            .collect();
        writeln!(
            w.cities,
            "{}",
            json!({
                "round": state.round,
                "city_id": c.name.clone(),
                "name": c.name,
                "body_id": c.body_id,
                "settlement": c.settlement.clone(),
                "faction_id": c.faction_id,
                "population": c.population,
                "loyalty": r2(c.loyalty),
                "razed": c.razed,
                "deployed_area": r2(deployed),
                "building_count": c.buildings.len(),
                "buildings": buildings,
                // 治理到首都的距离（AU，游戏规则：距 capital_body 越远治理越费、忠诚越低）。
                // 由模拟算出（复用 agent::governance_distance），agent 只读；夷平城无主，置 0。
                "gov_distance": r2(if c.razed { 0.0 } else { crate::agent::governance_distance(state, &c.faction_id, &c.body_id) }),
                // **产地货栈**：这座城所在天体上、属于该势力的冻结存货（按资源价值计）。
                // 非首都产出不会直接进势力库存——它先落在这里，要靠船运回首都才可用
                // （见 `.agents/notes/freight-collection.md`）。所以这一列是「这里压了多少
                // 运不出去的货」：>0 且长期不动 = 这座城市接不上运输。
                "depot_value": r2(if c.razed {
                    0.0
                } else {
                    state
                        .depot(&c.faction_id, &c.body_id)
                        .map(|m| m.iter().map(|(rt, amt)| amt * value_of(rt)).sum::<f64>())
                        .unwrap_or(0.0)
                }),
                // 离心风险：忠诚低于叛变阈值即爆发 Revolt（夷平为空白）。模拟算好的信号。
                "revolt_risk": !c.razed && c.loyalty <= config.governance.loyalty_revolt,
            })
        )
        .map_err(|e| e.to_string())?;
    }

    for f in &state.factions {
        let city_ids: Vec<String> = state
            .cities
            .iter()
            .filter(|c| !c.razed && c.faction_id == f.name)
            .map(|c| c.name.clone())
            .collect();
        let ship_ids: Vec<String> = state
            .ships
            .iter()
            .filter(|s| s.hull > 0.0 && s.faction_id == f.name)
            .map(|s| s.name.clone())
            .collect();
        // 前沿（p = 1 的日心距）：掌握度到顶时是无穷 ⇒ 用 `null` 表示（JSON 没有 Infinity）。
        let frontier = crate::sim::mond_frontier(config, f.mond_control);
        let frontier_json =
            if frontier.is_finite() { json!(r2(frontier)) } else { serde_json::Value::Null };
        let ships_in_band = state
            .ships
            .iter()
            .filter(|s| {
                s.hull > 0.0
                    && s.faction_id == f.name
                    && crate::sim::dist(s.position, [0.0, 0.0]) > config.mond.radius
            })
            .count();
        writeln!(
            w.factions,
            "{}",
            json!({
                "round": state.round,
                "faction_id": f.name.clone(),
                "name": f.name,
                "symbol": f.symbol,
                "capital_body": state.capital_body(&f.name),
                "alignment": r2(f.alignment),
                "aggression": r2(f.aggression),
                "home_radius": r2(f.home_radius),
                "home_attack_mult": r2(f.home_attack_mult),
                "home_regen_bonus": r2(f.home_regen_bonus),
                "ideology": {
                    "peace_military": r2(f.ideology.peace_military),
                    "science_tech": r2(f.ideology.science_tech),
                    "people_elite": r2(f.ideology.people_elite),
                    "nature_colony": r2(f.ideology.nature_colony),
                },
                "resources": f.resources,
                "relations": f.relations,
                // 信誉（承包市场的准入资产，势力级）。它是**唯一的抵押品**：承运人不赔货值
                // （Q1(b)），托运方靠这一列决定敢不敢把货交给它。
                "reputation": r2(f.reputation),
                // **思潮 → 集货倾向**（用户裁决：由国家思潮决定舰船倾向于运输还是战斗）：
                // `freight_lean` 是「愿意投在集货上的头数倍数」（中庸 = 1.0，军国 < 1、
                // 和平/殖民 > 1），`freighter_quota` 是「目标运输舰条数」= 需求 × 倾向。
                // 有这两列，「这个国家为什么少跑运输」是可读的，而不是只能从行为反推。
                "freight_lean": r2(crate::autocontrol::freight::freight_lean(state, &f.name)),
                "freighter_quota": r2(crate::autocontrol::freight::freighter_quota(state, &f.name)),
                // **造舰的两条动机**（用户裁决：解耦）——
                // `threat_motive`：敌对国比自己强多少（造战斗舰）；
                // `haul_gap`：集货运力**搬不动的比例**（造货船；已雇到的部分不算缺口）。
                // 有这两列，「这个国家为什么在造重舰 / 为什么在造船坞运货」是可读的。
                "threat_motive": r2(crate::autocontrol::shipbuilding::threat_motive(state, config, &f.name)),
                "haul_gap": r2(crate::autocontrol::freight::haul_gap(state, config, &f.name)),
                // **MOND 掌握度**（0..1）——科技体系的干线：它连续地决定异常区里
                // 「一次导航尝试的胜算」（`0` = 凡人、`1` = 指哪打哪），因此这一列是
                // 「谁能去多深」的唯一读数。另两列给出它的**来路**与**结论**：
                // `mond_ships_in_band` = 此刻在异常区里的自己的活舰数（唯一的知识渠道），
                // `mond_frontier_au` = 一次到位的最远日心距。
                "mond_control": r2(f.mond_control),
                "mond_ships_in_band": ships_in_band,
                "mond_frontier_au": frontier_json,
                // 此刻实际在跑运输的舰数（有效角色为 true；含玩家钉住与舱里有货的）。
                "freighter_count": state
                    .ships
                    .iter()
                    .filter(|s| s.hull > 0.0 && s.faction_id == f.name && state.ship_freighter(s.name.clone()))
                    .count(),
                "city_ids": city_ids,
                "ship_ids": ship_ids,
            })
        )
        .map_err(|e| e.to_string())?;
    }

    // 雇佣挂单簿（**只留没结束的合同**：等人接的 + 还在雇佣期内的）。`carrier` 为 null
    // 表示还在挂单簿上等人接；`ships` 是**此刻在替这张单跑的舰**（可以是零条、也可以多条
    // ——用户：「对方派几艘船都无所谓」）。结束的合同不在这张表里，去 `events` 里查。
    for c in &state.contracts.contracts {
        writeln!(
            w.contracts,
            "{}",
            json!({
                "round": state.round,
                "contract_id": c.id,
                "shipper": c.shipper,
                "carrier": c.carrier,
                "resource": c.resource,
                // 要求的运力（单位/回合）——不是「要搬多少件」（雇佣形态）。
                "capacity": r2(c.capacity),
                "delivered": r2(c.delivered),
                // 考核的分母：本期「起运货栈有货」的回合数。
                "served_rounds": c.served_rounds,
                // 扣除在途宽免之后的**产出期**（考核真正用的分母）。
                "output_rounds": r2(c.output_rounds(config)),
                // 实测吞吐达标率（1.0 = 一个考核周期搬回一舱货 = 一条参考船的水准）；
                // null = **还不到看账的时候**（账上的产出还不满一个货舱）。
                "ratio": c.throughput_ratio(config).map(r2),
                "from": c.from,
                "to": c.to,
                "share": r2(c.share),
                "min_reputation": r2(c.min_reputation),
                // 此刻在跑这张单的舰（`assignments` 的反查；空数组 = 还没派人）。
                "ships": state.contracts.ships_of(c.id),
                "posted_round": c.posted_round,
                "accepted_round": c.accepted_round,
                "expires_round": c.expires_round,
                "review_round": c.review_round,
            })
        )
        .map_err(|e| e.to_string())?;
    }

    // —— 派生表：本回合的流量中间量（引擎内部算过、但不落持久状态的量）——
    // **不做 r2 舍入**：`--derived` 直接序列化同一个 `Derived`，两个读面必须给出相同的 JSON
    // 值（这是"同一回合两个读面不许各说各话"的可检查形式）。
    for f in &state.factions {
        let prod = derived.flow.faction_production.get(&f.name).cloned().unwrap_or_default();
        // 治理：**本回合没跑治理步骤**的势力（零城势力——`step_governance` 在 `cities.is_empty()`
        // 时直接 `continue`，`src/sim.rs:1897`）在 `flow.governance` 里根本没有键。这里补的默认值
        // 必须是**引擎自己的约定**（`src/sim.rs:1926`：`governance_total ≈ 0 ⇒ coverage = 1.0`），
        // 不能图省事用 `GovernanceFlow::default()` 的 0.0 —— 否则同一回合的两个读面会各说各话：
        // `flow.jsonl` 说「覆盖 0%」（读起来像治理崩了），而 `metrics.factions[].governance_coverage`
        // 说 100%。零城势力的正确语义是「无账可付」，不是「付不起」。
        let gov = derived.flow.governance.get(&f.name);
        writeln!(
            w.flow,
            "{}",
            json!({
                "round": state.round,
                "faction_id": f.name.clone(),
                "production": prod,
                "upkeep": derived.flow.upkeep.get(&f.name).copied().unwrap_or(0.0),
                "governance_total": gov.map(|g| g.total).unwrap_or(0.0),
                "governance_coverage": gov.map(|g| g.coverage).unwrap_or(1.0),
            })
        )
        .map_err(|e| e.to_string())?;
    }
    for c in &state.cities {
        let prod = derived.flow.city_production.get(&c.name).cloned().unwrap_or_default();
        writeln!(
            w.city_flow,
            "{}",
            json!({
                "round": state.round,
                "city_id": c.name.clone(),
                "body_id": c.body_id,
                "faction_id": c.faction_id,
                "razed": c.razed,
                "production": prod,
            })
        )
        .map_err(|e| e.to_string())?;
    }

    // —— 控制面：把 `--control` 那块（读面即写面）也摊成 tidy 行，好让 Python 一次 join 到 ——
    //
    // 列的语义（每列只有一个含义）：`kind` = 叶的种类；`key` = 该叶的键（舰名/资源名/城名，
    // 舰队默认与首都为 `""`）；`sub` = **仅**权重叶的建筑下标（其余 kind 为 null）。
    // `value`/`mode` 是叶自己的值与三态归属——**不是**有效值：有效值看 `ships` 表的
    // `order_effective*` 列（引擎解析），别在 Python 里重实现链。
    for (fid, c) in &state.control {
        let mut row = |kind: &str, key: serde_json::Value, sub: serde_json::Value, value: serde_json::Value, mode: ControlMode| -> Result<(), String> {
            writeln!(
                w.control,
                "{}",
                json!({"round": state.round, "faction_id": fid, "kind": kind, "key": key, "sub": sub, "value": value, "mode": mode})
            )
            .map_err(|e| e.to_string())
        };
        for (ship, leaf) in &c.ship_orders {
            row("ship_order", json!(ship), json!(null), json!(leaf.value), leaf.mode)?;
        }
        if let Some(d) = &c.default_ship_order {
            row("default_ship_order", json!(""), json!(null), json!(d.value), d.mode)?;
        }
        // —— 风格三轴的六片叶（`control-live-layers.md` §3 那条候选 + 运输分支的角色轴）——
        //
        // 漏掉它们的后果很具体：Python 侧只能从 `ships` 表的 `doctrine`/`kiting`/`freighter`
        // （**有效值**）看结果，看不到这些叶**自己的值与自己的表态**——于是「这艘舰的风格/角色
        // 是它自己钉的，还是跟着舰队默认走的」在表里查不出来（web 的 `effectiveMode()` 正是
        // 靠这个区分）。`value` 列是 `any`：doctrine 是 `{temper, lone_wolf}` 对象，
        // kiting 是数字，freighter 是布尔。
        for (ship, leaf) in &c.ship_doctrine {
            row("ship_doctrine", json!(ship), json!(null), json!(leaf.value), leaf.mode)?;
        }
        for (ship, leaf) in &c.ship_kiting {
            row("ship_kiting", json!(ship), json!(null), json!(leaf.value), leaf.mode)?;
        }
        for (ship, leaf) in &c.ship_freighter {
            row("ship_freighter", json!(ship), json!(null), json!(leaf.value), leaf.mode)?;
        }
        if let Some(d) = &c.default_doctrine {
            row("default_doctrine", json!(""), json!(null), json!(d.value), d.mode)?;
        }
        if let Some(d) = &c.default_kiting {
            row("default_kiting", json!(""), json!(null), json!(d.value), d.mode)?;
        }
        if let Some(d) = &c.default_freighter {
            row("default_freighter", json!(""), json!(null), json!(d.value), d.mode)?;
        }
        for (res, leaf) in &c.investment_budget {
            row("investment_budget", json!(res), json!(null), json!(leaf.value), leaf.mode)?;
        }
        for (res, leaf) in &c.construction_budget {
            row("construction_budget", json!(res), json!(null), json!(leaf.value), leaf.mode)?;
        }
        for ((city, b), leaf) in &c.invest_weights {
            row("invest_weight", json!(city), json!(b), json!(leaf.value), leaf.mode)?;
        }
        for ((city, b), leaf) in &c.build_weights {
            row("build_weight", json!(city), json!(b), json!(leaf.value), leaf.mode)?;
        }
        for (city, leaf) in &c.loyalty_budget {
            row("loyalty_budget", json!(city), json!(null), json!(leaf.value), leaf.mode)?;
        }
        if let Some(cap) = &c.capital {
            row("capital", json!(""), json!(null), json!(cap.value), cap.mode)?;
        }
    }
    // —— 设计图库：每回合 × 每势力 × 每张图一行 ——
    //
    // 这是**结构叶**的专用表（`control` 表那边刻意不发 `kind="blueprint"` 的行，免得出现
    // 两份表示互相漂移）。列的口径：
    // * `mode` = 图叶**自己的**表态；`effective_mode` = `State::blueprint_control` 的答案
    //   （图叶 → 势力 scope → 全局；全继承 ⇒ Auto）。引擎解析，Python 别重算。
    // * `ship_count` = 世界上有多少艘舰出自这张图（引擎算）。
    // * `class_slots` / `component_cost` 是**配置表的派生量**：不落状态，省得每个配方自己
    //   去 join `meta.json`。
    // * `launch_waiting` = Q4(b) 的**可见标记**：这张（玩家归属的）图此刻「进度已经攒够
    //   `build_points` 却没下水」——因为买不起它的选装，进度在继续攒。
    for (fid, c) in &state.control {
        for (id, leaf) in &c.blueprints {
            let ship_count = state
                .ships
                .iter()
                .filter(|s| s.faction_id == *fid && s.blueprint.as_deref() == Some(id.as_str()))
                .count();
            let mut component_cost: ResourceMap = ResourceMap::new();
            for comp in &leaf.value.components {
                if let Some(cs) = config.components.get(comp) {
                    for (rt, amt) in &cs.cost {
                        *component_cost.entry(rt.clone()).or_insert(0.0) += *amt;
                    }
                }
            }
            writeln!(
                w.blueprints,
                "{}",
                json!({
                    "round": state.round,
                    "faction_id": fid,
                    "blueprint_id": id,
                    "class": leaf.value.class,
                    // 选装：**全量**输出（空数组 = 交给生成器现算），与 `--control` 一致。
                    "components": leaf.value.components,
                    // `order` = 本图给新舰的默认意图（**默认枚举形式**，与 `control` 表一致；
                    // null = 本图对意图没有说话）。
                    "order": leaf.value.order,
                    "mode": leaf.mode,
                    "effective_mode": state.blueprint_control(fid, id),
                    "ship_count": ship_count,
                    // 该舰级的槽位上限（配置表派生量）。
                    "class_slots": config.ships.get(&leaf.value.class).map(|s| s.slots),
                    // 选装的一次性成本（造一艘要额外花的组件钱；配置表派生量）。
                    "component_cost": component_cost,
                    // 本回合这张图是否在「等钱」（进度满了却不下水，Q4(b) 的可见标记）。
                    "launch_waiting": crate::sim::blueprint_launch_waiting(state, config, fid, id),
                })
            )
            .map_err(|e| e.to_string())?;
        }
    }

    // `scope`：只发**显式表态**的节点（`Inherit` = 这一层没有说话，不必占行）。
    writeln!(
        w.scope,
        "{}",
        json!({"round": state.round, "level": "global", "key": "", "mode": state.scope.global})
    )
    .map_err(|e| e.to_string())?;
    for (fid, m) in &state.scope.factions {
        writeln!(w.scope, "{}", json!({"round": state.round, "level": "faction", "key": fid, "mode": m}))
            .map_err(|e| e.to_string())?;
    }
    for (bid, m) in &state.scope.bodies {
        writeln!(w.scope, "{}", json!({"round": state.round, "level": "body", "key": bid, "mode": m}))
            .map_err(|e| e.to_string())?;
    }
    for (cid, m) in &state.scope.cities {
        writeln!(w.scope, "{}", json!({"round": state.round, "level": "city", "key": cid, "mode": m}))
            .map_err(|e| e.to_string())?;
    }

    // —— 判定：本回合 **AI 选了什么、为什么**（`Derived.flow.decisions`）——
    //
    // 两种 `kind` 共用一组固定列；`actor` = 谁（舰名 / 船坞所在城）是 join 键。共用列之外
    // 的差异（逐舰判定的输入 vs 改装的前后舰级）都进 `detail` 对象——这样 Python 侧列类型
    // 稳定，而各 kind 的专属信息不丢。
    //
    // ⚠ 空白是**有信息**的：`verdict: "hold"` 行 = 这回合 AI 没给这艘舰派活（叶上那条值
    // 可能是很久以前的），不是"它在待命"。
    for d in &derived.flow.decisions.ships {
        writeln!(
            w.decisions,
            "{}",
            json!({
                "round": state.round,
                "faction_id": d.faction,
                "kind": "ship_order",
                "actor": d.ship,
                "verdict": d.verdict,
                "target": d.target,
                "detail": {
                    "hull_ratio": d.hull_ratio,
                    "retreat_hull": d.retreat_hull,
                    "kiting": d.kiting,
                    "enemy_in_range": d.enemy_in_range,
                    "after_move": d.after_move,
                    "destination": d.destination,
                    "order": d.order,
                },
            })
        )
        .map_err(|e| e.to_string())?;
    }
    for r in &derived.flow.decisions.retools {
        writeln!(
            w.decisions,
            "{}",
            json!({
                "round": state.round,
                "faction_id": r.faction,
                "kind": "retool",
                "actor": r.city,
                "verdict": "retool",
                "target": r.to,
                "detail": {"building": r.building, "from": r.from},
            })
        )
        .map_err(|e| e.to_string())?;
    }
    // 风格轴的**执行者**（`autocontrol::style`）：`Auto` 风格叶不是"值冻结"，它每回合被
    // 概率触发、朝战况目标走一步分布步长——所以"改了哪条轴、朝哪儿改、为什么"必须能回答。
    for s in &derived.flow.decisions.styles {
        writeln!(
            w.decisions,
            "{}",
            json!({
                "round": state.round,
                "faction_id": s.faction,
                "kind": "style_retune",
                "actor": s.ship,
                "verdict": s.axis,
                "target": serde_json::Value::Null,
                "detail": {
                    "from": s.from,
                    "to": s.to,
                    "goal": s.target,
                    "drivers": s.drivers,
                },
            })
        )
        .map_err(|e| e.to_string())?;
    }
    // 设计图的**执行者**（`autocontrol::blueprints`）：建图/重估/复用/回收。
    for d in &derived.flow.decisions.blueprints {
        writeln!(
            w.decisions,
            "{}",
            json!({
                "round": state.round,
                "faction_id": d.faction,
                "kind": "blueprint",
                "actor": d.blueprint,
                "verdict": d.action,
                "target": d.class,
                "detail": {
                    "theme": d.theme,
                    "components": d.components,
                    "city": d.city,
                    "building": d.building,
                },
            })
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// 本舰叶片自己的表态（没有叶片 = `Inherit`，即"这一层没有说话"）。
fn leaf_mode_of(state: &State, fid: &FactionId, ship: &ShipId) -> ControlMode {
    state
        .control(fid.clone())
        .and_then(|c| c.ship_orders.get(ship))
        .map(|l| l.mode)
        .unwrap_or_default()
}

/// 势力级**舰队默认指令**的表态（没有这片叶 = `Inherit`）。
fn default_mode_of(state: &State, fid: &FactionId) -> ControlMode {
    state
        .control(fid.clone())
        .and_then(|c| c.default_ship_order.as_ref())
        .map(|d| d.mode)
        .unwrap_or_default()
}

/// 本舰出厂那张图**在势力库里的叶表态**（没有图 / 图不存在 = `Inherit` = 这一层没有说话）。
fn blueprint_mode_of(state: &State, fid: &FactionId, bp: Option<&str>) -> ControlMode {
    bp.and_then(|id| {
        state
            .control(fid.clone())
            .and_then(|c| c.blueprints.get(id))
            .map(|l| l.mode)
    })
    .unwrap_or_default()
}

/// 本舰出厂图上**意图那一层**的表态：图上写了 `order` 就是叶自己的表态，没写（或缺图）
/// 就是 `Inherit`（Q1(c)：图的意图轴默认沉默）。
///
/// ⚠ 这是**图叶自己的**表态，不是链解析结果：`order_effective_mode`（`State::ship_control`）
/// 才是「谁说了算」。两者可能不一致，而且那正是有信息的地方（例如图叶是 `Inherit`、
/// 但舰队默认叶是 `Player` ⇒ 图上这层说话与否都不影响结果）。
fn order_blueprint_mode_of(state: &State, fid: &FactionId, bp: Option<&str>) -> ControlMode {
    bp.and_then(|id| {
        state
            .control(fid.clone())
            .and_then(|c| c.blueprints.get(id))
            .filter(|l| l.value.order.is_some())
            .map(|l| l.mode)
    })
    .unwrap_or_default()
}

/// The **agent-readable** projection schema. It is the single contract both the Rust emitter and
/// the Python kit share: fields with a `lazy` entry are NOT inline in `main.jsonl` (the main row
/// carries their id-array), while `eager` fields are inline. Column types are listed so the agent
/// knows the table shape without guessing.
pub fn projection_schema() -> serde_json::Value {
    let mut lazy = serde_json::Map::new();
    for f in LAZY {        let entry = match f.name {
            "ships" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "舰的完整对象（class/组件/护甲/护盾/位置/速度 + effective 面板：attack/range/speed/upkeep 等 + 指令归属的引擎解析结果 order_*），随回合变化。按 (round, ship_id) 索引。",
                "columns": {"round":"integer","ship_id":"string","faction_id":"string","class":"string","name":"string","x":"number","y":"number","hull":"number","hull_max":"number","shield":"number","shield_max":"number","velocity":"number","components":"array","component_hp":"array","attack":"number","attack_range":"number","speed":"number","accel":"number","hardness":"number","intercept":"number","shield_regen":"number","hull_regen":"number","upkeep":"number","order_leaf_mode":"string","order_default_mode":"string","order_effective_mode":"string","order_effective":"object","order_source":"string","doctrine":"object","kiting":"number","freighter":"boolean","freighter_mode":"string","blueprint":"string","blueprint_mode":"string","order_blueprint_mode":"string","spawned_round":"integer"},
                "column_docs": {
                    "order_leaf_mode": "本舰**叶片自己**的表态（没有叶片 = Inherit）。",
                    "order_default_mode": "势力级**舰队默认指令**的表态（没有这片叶 = Inherit）。",
                    "order_effective_mode": "**有效归属**：`State::ship_control` 的答案（叶 → **出厂图**（图上真写了 `order` 时）→ 舰队默认 → 势力 scope → 全局 scope，最具体的有意见者胜；全继承 ⇒ Auto）。",
                    "order_effective": "**有效指令**：`State::ship_behavior` 的答案（null = 没有任何一层说话，调用方按 Idle 兜底）。注意「叶 Inherit + 舰队默认不是 Player」时会回落到叶上的记录值——这是引擎的既有取值规则，Python 侧不要自己重算。",
                    "order_source": "**这条有效意图是谁供的值**（`State::ship_behavior_source`）：`leaf`（本舰的指令叶存在——`mode` 是 `Inherit` 也算）/ `blueprint:<图名>`（值来自本舰出厂那张图上的 `order`）/ `fleet_default`（势力级舰队默认叶）/ `scope` / `record`。⚠ 后两个取值在**指令链上不会出现**（作用域节点只表态『谁负责』、不携带值；指令没有出厂记录值——那是 `doctrine`/`kiting`/`freighter` 三轴的兜底），列在取值域里是为了让枚举与控制属性的层次链一一对应，不是漏了分支。⚠ 它把「叶**不存在**」与「叶写着 `Inherit`」分开报：后者报 `leaf`（那时值真的来自那片叶，`leaf.map(|l| l.value)`），只有叶不存在才可能落到 `blueprint:*`/`fleet_default`。",
                    "doctrine": "**有效行为风格**（`State::ship_doctrine`：叶 → 舰队默认 → 舰上记录值）——{temper, lone_wolf}，各取 [-1,1]。舰上的 `Ship.doctrine` 只是出厂快照/AI 流水，不是这里。",
                    "kiting": "**有效风筝<->贴脸姿态**（`State::ship_kiting`，同一条链），[-1,1]，0 = 基线。",
                    "freighter": "**有效角色**（`State::ship_freighter`，同一条链）：`true` = 运输舰（自动控制给它排集货路线），`false` = 战舰（找仗打）。它**只管自动控制派哪种活**——不解除武装，运输舰照样自动开火、照样按 `kiting` 软移动。",
                    "freighter_mode": "角色那片叶的**有效归属**（`State::ship_freighter_control`）：Auto = 这条结论是自动控制写的（它每回合按积压定编），Player = 玩家钉的、AI 不碰。",
                    "blueprint": "本舰**出厂所用**的设计图名（null = 无图：旧档 / 开局预置舰队 / 剧情赠舰）。⚠ 它是**快照的溯源**——不代表本舰的选装会随图变化（`components` 是出厂快照）；join `derived.blueprints` 的 `blueprint_id` 看那张图的详情。",
                    "blueprint_mode": "那张图**在势力库里的叶表态**（Inherit/Auto/Player；缺图 = Inherit）。有效归属看蓝图表 `effective_mode`。",
                    "order_blueprint_mode": "图上**意图那一层**的表态：图上写了 `order` 就是叶自己的表态，没写（或缺图）= Inherit（Q1(c)：图的意图轴默认沉默）。⚠ 这是**图叶自己**的表态，不是链解析结果——与 `order_effective_mode` 不一致是正常的（例如图叶 Inherit、舰队默认叶 Player）。",
                    "spawned_round": "本舰**下水所在回合**（null = 旧档缺字段 ⇒ **未知**）。用途：编制表/花名册的确定性 tie-break（同分取最老的）——遇到 null 要**回落名字序**，不能当成第 0 回合。",
                },
            }),
            "cities" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "城的完整对象（人口/忠诚/治理距离/建筑清单/离心风险），随回合变化。按 (round, city_id) 索引。",
                "columns": {"round":"integer","city_id":"string","name":"string","body_id":"string","settlement":"string","faction_id":"string","population":"integer","loyalty":"number","razed":"boolean","deployed_area":"number","building_count":"integer","buildings":"array","gov_distance":"number","depot_value":"number","revolt_risk":"boolean"},
            }),
            "factions" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "势力的完整对象（库存/resources/relations/意识形态/本土防御 + 它拥有的城与舰 + 信誉），随回合变化。按 (round, faction_id) 索引。这是 agent 看外交 + 经济 + 军力的主表。",
                "columns": {"round":"integer","faction_id":"string","name":"string","symbol":"string","capital_body":"string","alignment":"number","aggression":"number","home_radius":"number","home_attack_mult":"number","home_regen_bonus":"number","ideology":"object","resources":"object","relations":"object","reputation":"number","mond_control":"number","mond_ships_in_band":"integer","mond_frontier_au":"number|null","freight_lean":"number","freighter_quota":"number","freighter_count":"integer","threat_motive":"number","haul_gap":"number","city_ids":"array","ship_ids":"array"},
                "column_docs": {
                    "reputation": "**信誉**（势力级全局单值，雇佣市场的准入资产）：受雇方**不赔货值**，干砸了只掉它，而雇主按它决定敢不敢把线交给它、要不要续约——所以它是这条腿上**唯一的抵押品**，低信誉者结构上接不到贵活/难活。它**只由雇主的周期考核产生**（`contract_reviewed`：按实测吞吐掷好评/差评，各 ±`freight.reputation_gain`），不随回合自然衰减。中性值 1.0（没有任何雇佣履历）。",
                    "mond_control": "**MOND 掌握度**（0..1，科技体系的干线）：`0` = 牛顿近似的凡人、`1` = 指哪打哪（今天的崇拜教）。它**连续地**决定异常区内「一次导航尝试的胜算」`p = min(1, (arrival_eps/(depth×drift_per_au×(1−它)))^(1/shape))`，于是前沿（p = 1 的日心距）`= 28 + 0.06/(0.03×(1−它))` AU：0 → 30.0、0.35 → 31.1、0.70 → 34.7、0.90 → 48.0。开局值来自 `config.mond.initial`；之后由 `sim::step_knowledge` 按**飞船在异常区的在场强度**驱动（用户裁决：先只做这一条渠道）——**它是活知识**：把舰队撤回来，它会慢慢锈回凡人。",
                    "mond_ships_in_band": "此刻自己有**多少艘活舰在异常区里**（日心距 > `mond.radius`）——这是掌握度**唯一**的知识来源（第一版）。`mond_control` 在涨还是锈，看这一列就是答案。",
                    "mond_frontier_au": "**前沿海拔**（AU）：一次导航尝试就能精确到位（p = 1）的最远日心距 = `radius + arrival_eps/(drift_per_au×(1−mond_control))`。前沿**之外**不是「进不去」，而是「期望要试 `1/p` 次」；掌握度到顶时为 `null`（无穷远，指哪打哪）。",
                    "freight_lean": "**思潮 → 集货倾向**（用户裁决：由国家思潮决定舰船倾向于运输还是战斗）= `2σ(−1.5 × 尚武度)`，**尚武度 = +和平↔军国 − 自然↔殖民**（两轴同权反号，写死在 `autocontrol::freight`）。中庸 = 1.0 = 旧的硬定编；**军国 < 1**（宁可缺货、宁可雇人也要把船留在战线上）、**和平/殖民 > 1**（殖民要给远方殖民地送补给 ⇒ 多跑运输）。",
                    "freighter_quota": "**目标运输舰条数**（连续量）= `需求 × freight_lean`，需求 = 有积压的货栈数。自动控制按「目标 − 现状」这个**缺口抽签**派人（概率 = 缺口 × 本舰的票 ÷ 同侧总票数，票按运力效率 ⇒ 期望入伙数正好是缺口）。**没有积压 ⇒ 配额 0 ⇒ 全员战舰**。",
                    "threat_motive": "**造战斗舰的动机**（0..1）= 敌对国比自己强多少：`σ((Σ_j 敌对度_j × (实力_j − 自己实力) ÷ 自己实力 − 1) ÷ 0.5)`。实力用均势外交那把尺子（城 + 舰体占比）；**只有比自己强的才算威胁** ⇒ 压得住场子的势力不会因为「在打仗」就继续堆旗舰（众弱结盟的备战动机 > 霸权的）。它取代了旧的「是否处于战争」这个布尔。",
                    "haul_gap": "**造货船的需求**（0..1）= 集货运力**搬不动的比例**：`Σ(要求运力 − 自有 − 已雇) ÷ Σ要求运力`，逐货栈夹到 ≥0 再求和。**已雇到的不算缺口**（雇得到人就不必自己造船）。它推动 `retool_haulers` 腾一个船坞改产货船——**与 `threat_motive` 各占一个船坞，互不淹没**（这就是两条造舰动机的「解耦」）。",
                    "freighter_count": "**此刻实际在跑运输的舰数**（有效角色为 true：含玩家钉住的、舱里载着货的、正在执行承包单的）。把这一列与 `freighter_quota` 对比，就能分辨「思潮不让跑」（配额低）与「没人可派」（配额高但舰不够）。",
                },
            }),
            "contracts" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "**雇佣运力挂单簿**：一行一单，只含**没结束**的合同（等人接的 + 还在雇佣期内的）。结束的合同不在这里——“怎么结束的”去 `events` 里按 `contract_ended` 查。按 (round, contract_id) 索引。单子要求的是**运力**（单位/回合），不是一票货：受雇方自己决定派几条船来跑（`ships` 可以为空、也可以多条）。",
                "columns": {"round":"integer","contract_id":"integer","shipper":"string","carrier":"string","resource":"string","capacity":"number","delivered":"number","served_rounds":"integer","ratio":"number","from":"string","to":"string","share":"number","min_reputation":"number","ships":"array","posted_round":"integer","accepted_round":"integer","expires_round":"integer","review_round":"integer"},
                "column_docs": {
                    "shipper": "雇主（挂单的人）。",
                    "carrier": "受雇方；**null = 还在挂单簿上等人接**（这是本表最常用的一列：它是「市场上还没被吃掉的运力需求」）。",
                    "ships": "**此刻在替这张单跑的舰名数组**（空数组 = 还没派人，或多条船组队）。派几条船、派哪条，是**受雇方的内部事务**（用户：「对方派几艘船都无所谓」）——它跑的是**雇主**的路线：起运在雇主货栈、目的在雇主首都，与受雇方自己的集货路线**方向不同**。",
                    "min_reputation": "雇主定的**信誉门槛**（挂单时按难度与货值算好并冻结）：合格度 = σ((受雇方信誉 − 这一列) ÷ 宽度)。**不是硬闸**——低信誉者极少被选中，而非绝无可能。Q1(b) 之后这是雇主唯一的自我保护（受雇方不赔货值）。**到期续约用的是同一个闸**。",
                    "capacity": "**要求的运力**（单位/回合）= 一条参考船在这条线上的吞吐（`nominal_hold ÷ 参考往返回合数`）。未接单时每回合被改成此刻的缺口（雇主自己搬不动的部分），接单后**冻结**成承诺。",
                    "delivered": "本雇佣期内**已从雇主货栈搬走**的量（含受雇方自留的抽成——抽成是搬运费，不该从运力里扣）。",
                    "served_rounds": "考核的**分母**：本期「起运货栈有货」的回合数。没货可运的回合不算在受雇方头上。",
                    "ratio": "**实测吞吐达标率** = `delivered ÷ (capacity × served_rounds)`：1.0 = 恰好是一条参考船的水准（一个考核期搬回一舱货）。null = 本期还没有有货可运的回合 ⇒ 无从考核（不是考零分）。雇主按它掷骰子给好评/差评。",
                    "from/to": "起运天体（雇主的产地货栈）→ 目的天体（照公理“首都即集散地”，`to` 永远是雇主首都）。",
                    "share": "受雇方**抽成**比例：交付时从货里自留，其余进雇主首都池。没有货币转移——报酬就是它没交出去的那部分货。没人接的单子每个考核周期抬一档（上限 `freight.share_max`）。",
                    "posted_round": "**本轮叫价的起点**：一个考核周期没人接就抬一档抽成并把这一列挪到当时回合（免得一挂出来就连续加价）。",
                    "accepted_round/expires_round/review_round": "雇佣起算回合 / 固定期到期回合 / 下次考核回合。期限与考核周期都从**航程**算（一个考核周期 = 这条线的一个往返），所以不同航线的刻度差一个数量级。",
                },
            }),
            "events" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "事件历史（稀疏里程碑）：一行一事件，**归一化固定列**——参与方一律走 (actor_kind, actor_id, target_kind, target_id) 主槽位 + `extra` 长表（role/kind/id），因此任意实体（城/舰/势力/天体）都能用同一个查询形状 join 它自己的历史，不需要知道任何事件类型的字段布局。variant 专属载荷统一收进 `data` 一个对象列（一列只承载一种类型：没有\"同时是标量和列表\"的列，也没有 `from`/`to` 这种一列两义的同名列）。\n统计建议：**先按类型取**（`q.events(type='city_razed')` → 该类型的字段是稠密的），全集帧只用于计数/扫描。",
                "columns": {
                    "round":"integer","seq":"integer","event_id":"string","type":"string","salience":"string",
                    "weight":"integer",
                    "actor_kind":"string","actor_id":"string","target_kind":"string","target_id":"string",
                    "extra":"array","magnitude":"number","headline":"string","data":"object"
                },
                "column_docs": {
                    "event_id": "稳定 id `<round>:<seq>`，join/因果引用用。",
                    "type": "事件类型（与 Rust `GameEvent::kind()` / serde 判别式逐字一致）。",
                    "salience": "**分层**，判据 = 后续计算需要访问哪一段历史（见 `Salience`）：`milestone`（无限过去）/ `notable`（一定窗口）/ `detail`（只需前一帧、或没有读者，仅为 agent 分析而记录）。**这一列不是「重要性」**——按重要性挑事件请用 `weight`。",
                    "weight": "**纯显示用**的排序键（`GameEvent::weight`）：**0–9 的序数阶梯**（9=开战/停战/结盟/迁都，8=城市易主或毁灭，7=势力重建/剧情，5=舰存亡，2=撤退/指令降级，0=逐发流水）。`salience` 回答「谁要回看它」，`weight` 回答「人读起来重不重要」——两者刻意分开：给 agent/人挑「值得读的事件」用这一列（`q.storyboard()` 默认取 `>= 8`），**别看 `salience`**。",
                    "actor_kind/actor_id": "动作发起方（如 city_razed 的 actor 是拆城的**势力**）——没有发起方时为 null。",
                    "target_kind/target_id": "动作直接对象（如被围的**城**、被击毁的**舰**）。",
                    "extra": "其余参与方长表 [{role, kind, id}]，role ∈ actor/target/victim/beneficiary/third；如 city_razed 里 by_ship（补刀的舰）、ship_destroyed 里的凶手与旧主。",
                    "magnitude": "统一数值强度（伤害；无伤害事件为 0），便于 groupby().sum()。",
                    "headline": "**人读的一句话**（`GameEvent::headline` 的唯一产物，与 CLI `--milestones`/`--digest` 同源）。它自足（只读事件自身字段，不回查 state），所以对已归档的历史同样成立；`participants()` 列出的每个 id 都逐字出现在这句话里。机器查询仍走 actor_*/target_*/data。",
                    "data": "该事件类型的专属载荷（可读名/舰级/死因/忠诚度/复垦方式…），固定只用这一个对象列。",
                },
            }),
            "bodies" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "天体主表（name/轨道/定居点数），几乎不变，全局一次。按 body_id 索引。",
                "columns": {"body_id":"string","name":"string","perihelion_distance":"number","aphelion_distance":"number","period":"number","x":"number","y":"number","settlement_count":"integer"},
            }),
            "settlements" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "定居点主表（每个天体上的空间位：名字/面积/生态容量/建设修正/资源矿藏），几乎不变，全局一次。按 body_id 过滤 + settlement_id 索引。",
                "columns": {"settlement_id":"string","body_id":"string","index":"integer","name":"string","total_area":"number","ecological_capacity":"number","construction_speed_mod":"number","construction_resource_mod":"number","resources":"array"},
            }),
            _ => continue,
        };
        lazy.insert(f.name.to_string(), entry);
    }

    // 派生表（`DERIVED`）：不是状态里的重型字段，而是**引擎算出来的量**（本回合流量中间量、
    // 控制面）。join 键是 main 已有的 id 数组（或世界级 row），所以它们不进 `LAZY`。
    let mut derived_tables = serde_json::Map::new();
    for t in DERIVED {
        let entry = match t.name {
            "flow" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**本回合的流量中间量**（`Derived.flow`）：各势力本回合各资源产出、舰队维护费、治理总成本/覆盖率。这些量由各 step 计算并应用、**不落到持久状态**，所以除了这张表没有别的读法。与 `planet_x --derived` 的值逐字一致（不做舍入）。",
                "columns": {"round":"integer","faction_id":"string","production":"object","upkeep":"number","governance_total":"number","governance_coverage":"number"},
                "column_docs": {
                    "production": "本回合该势力各资源产出（resource → 数量）。**没有产出也给 `{}`**（不是 null），这样 Python 侧列类型稳定。注意同一批数在主流 `metrics.factions[<势力>].production` 里也有一份（嵌套对象）；这张表是它的**可 join 平铺版**。",
                    "upkeep": "本回合该势力的舰队维护费（市场价值）。这是「预算压顶」判据的分子，`--control-plan` 的 `fleet_upkeep_cap` 是引擎给出的上限读数。",
                    "governance_total": "本回合治理总开销（行政 + 娱乐，含制裁倍率）。",
                    "governance_coverage": "治理覆盖率 0..1（覆盖不住就是离心风险的来源）。",
                },
            }),
            "city_flow" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**本回合各城的开采产出**（`Derived.flow.city_production`）：按 (round, city_id) 索引。**含已夷平的空白城**（`razed` 列筛，产出为 `{}`），与 `cities` 表逐行一致。同一批数在主流 `metrics.city_production` 里也有一份（但那张表跳过了 razed 城）。",
                "columns": {"round":"integer","city_id":"string","body_id":"string","faction_id":"string","razed":"boolean","production":"object"},
            }),
            "control" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**控制面的 tidy 行**：每个叶片一行（舰指令 / 舰队默认指令 / 预算 / 权重 / 娱乐预算 / 首都）。值就是 `--control` 里那片叶的值，**不是**有效值——有效值见 ships 表的 `order_effective*` 列（引擎解析，别在 Python 里重实现链）。⚠ **设计图不在本表**：它是结构叶（`{class, components[], order{}}`），住在 `derived.blueprints`（`value: any` 列塞不下结构，两张表示还会漂移）。",
                "columns": {"round":"integer","faction_id":"string","kind":"string","key":"string","sub":"integer","value":"any","mode":"string"},
                "column_docs": {
                    "kind": "叶的种类：ship_order / ship_doctrine / ship_kiting / ship_freighter / default_ship_order / default_doctrine / default_kiting / default_freighter / investment_budget / construction_budget / invest_weight / build_weight / loyalty_budget / capital。",
                    "key": "该叶的键：舰名 / 资源名 / 城名；`default_ship_order`/`default_doctrine`/`default_kiting`/`default_freighter` 与 `capital` 为 `\"\"`。",
                    "sub": "**仅**权重叶（invest_weight / build_weight）的建筑下标（城内唯一，见 name-as-unique-key 的裁决）；其余 kind 为 null。",
                    "value": "叶**自己的**值（不是有效值）：指令是行为对象、`ship_doctrine`/`default_doctrine` 是 `{temper, lone_wolf}`、`ship_kiting`/`default_kiting` 是数字、`ship_freighter`/`default_freighter` 是布尔、预算是数字、`capital` 是城名。要有效值请读 `ships` 表的 `order_effective*`/`doctrine`/`kiting`/`freighter` 列。",
                    "mode": "三态归属：Inherit（这一层没有说话）/ Auto（系统决定）/ Player（玩家决定）。写值即接管：diff 里只写值不写 mode ⇒ mode 变 Player。",
                },
            }),
            "scope" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**作用域树的显式表态**：谁负责 AI 决策（全局 / 势力 / 天体 / 城）。只发显式节点——`Inherit` 等于「这一层没有说话」，不占行。舰的归属链是 叶 → 出厂图 → 舰队默认 → 势力 → 全局。",
                "columns": {"round":"integer","level":"string","key":"string","mode":"string"},
                "column_docs": {
                    "level": "节点层级：global / faction / body / city（`global` 的 key 为 `\"\"`）。",
                },
            }),
            "blueprints" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**舰船设计图库**（势力级）：一行 = 一张图。设计图是「还不存在的舰」的出厂规格——建造区指向一张图，下水时把图印成一艘舰（`components` 是**快照**，改图**不**改已下水的舰）。**图 = 出厂规格（装什么），`mode` = 谁可以改这张图**：`components` 非空就按它装配（与归属无关），空数组 = 交给 `choose_loadout` 在出厂时现算。`Auto` 图的执行者是 `autocontrol::blueprints`（AI 自己建图/重估/去重复用/回收）+ `retool_shipyards`（舰级重估）。⚠ 设计图**不在** `derived.control` 表里（那是标量形状的叶；两张表示 = 漂移风险）——它就住这张表，`ships.blueprint` 与 `cities.buildings[].blueprint` join 它。⚠ 它也**不在** `Derived`（`--derived`）里：它是**状态**的纯函数（每回合从 `state.control[*].blueprints` 现算），所以「`--derived` 与 `--index` 必须给同一份数」那条约束**不适用于这张表**。",
                "columns": {"round":"integer","faction_id":"string","blueprint_id":"string","class":"string","components":"array","order":"object","mode":"string","effective_mode":"string","ship_count":"integer","class_slots":"integer","component_cost":"object","launch_waiting":"boolean"},
                "column_docs": {
                    "blueprint_id": "图名（势力内的唯一 key）。`ships` 表的 `blueprint` 列与 `cities.buildings[].blueprint` 都 join 它。图名会换代（改名 = 删旧建新）⇒ 指向不存在的图**必须**响亮报 `no_such_blueprint`（apply 时），绝不静默回落生成器。",
                    "class": "舰级（口径 A：必须 == 该建造区的 `ship_type`，否则 apply 报 `blueprint_class_mismatch`）。",
                    "components": "选装表（组件 id，顺序 = 槽位）。空数组 = 交给 `choose_loadout` 生成器；非空 ⇒ **出厂就按它装配**（与图的归属无关：归属只管「谁能改这张图」）。",
                    "order": "本图给**新舰**的默认意图（`ShipBehavior`，默认枚举形式；null = 本图对意图没有说话）。⚠ 链上只在该图的归属解析为 `Player` 时取值。AI 建的图**从不**写它（建图 ≠ 表态，Q1(c)）。",
                    "mode": "图叶**自己的**表态：Inherit（没有说话——**AI 建的图就是这个**：流水，不是表态）/ Auto（系统可重估：`retool_shipyards` 改舰级、`autocontrol::blueprints` 重估选装）/ Player（系统不许动）。",
                    "effective_mode": "**有效归属**（`State::blueprint_control`：图叶 → 势力 scope → 全局；全继承 ⇒ Auto）。引擎解析，别在 Python 里重算。",
                    "ship_count": "世界上 `Ship.blueprint == blueprint_id` 的舰数（引擎算）。",
                    "class_slots": "该舰级的槽位上限（配置表 `ShipSpec.slots` 的派生量，省得每个配方自己 join meta.json）。null = 舰级不在配置表里（正常状态不会出现）。",
                    "component_cost": "这张图的选装**一次性成本**（Σ组件 `cost`，配置表派生量）。造一艘的总花费还要加上舰级的 `build_cost`（那是进度池的账）。",
                    "launch_waiting": "**进度满了却不下水的可见标记**（用户裁决 Q4(b)）：本回合这张图在某个挂了它的城里，该舰级的进度已经 ≥ `build_points` 却没下水——因为**买不起**它的选装（只对 **Player 归属**的图可能为 true；Auto 图与无图保持旧行为）。进度**不会丢**，下回合攒够钱就下水。",
                },
            }),
            "decisions" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**本回合 AI 的判定**（`Derived.flow.decisions`）：逐舰「选了什么、当时的关键输入是多少」+ 船坞改装的「从什么改成什么」+ **风格重估**（`Auto` 风格叶的执行者每改一条轴一行）+ **设计图**（AI 建图/重估/复用/回收）。这些判定**既不发事件、也不落持久状态**（指令叶只留结果），所以除了这张表和 `planet_x --derived` 没有别的读法——它回答的是「我的舰为什么跑到那儿去送死」「这条风格轴为什么会变」「这张图是谁画的」。空白有意义：`verdict=\"hold\"` = 这回合 AI 没给这艘舰派活。",
                "columns": {"round":"integer","faction_id":"string","kind":"string","actor":"string","verdict":"string","target":"string","detail":"object"},
                "column_docs": {
                    "kind": "判定的种类：ship_order（逐舰行为判定）/ retool（船坞改装）/ style_retune（风格轴重估：`Auto` 风格叶的执行者）/ blueprint（设计图：AI 建图/重估/复用/回收）。",
                    "actor": "作判定的一方：舰名（ship_order / style_retune）/ 船坞所在城名（retool）/ **图名**（blueprint）。",
                    "verdict": "ship_order：withdraw（自保撤退）/ engage（接战）/ colonize（殖民复垦）/ bombard（就地轰炸）/ move（常规机动）/ haul（运输：跑集货路线，装/卸/在途都记成它）/ **hold（没派活）**；retool 固定为 retool；style_retune：temper / lone_wolf / kiting（**哪条轴**被改）；blueprint：created / retuned / reused / reaped。",
                    "target": "判定的对象：舰名（接战/撤退到首都）／城名（轰炸）／天体名（殖民）／新舰级（retool）／**舰级**（blueprint）；纯位置机动与 style_retune 为 null（看 `detail`）。",
                    "detail": "该 kind 的专属事实。ship_order：`hull_ratio`/`retreat_hull`（撤退判定的两个输入）、`kiting`（当时的有效风筝距离）、`enemy_in_range`、`after_move`（这次判定是否发生在移动之后——**一艘舰一回合最多两行**：先机动、到位后再判一次）、`destination`（驶向的坐标）、`order`（实际写回指令叶的行为，null = 没写叶）。retool：`from`（改装前舰级）、`building`（船坞在该城内的建筑下标，只在城内唯一）。style_retune：`from`/`to`（这条轴改动前后的值）、`goal`（这次重估朝它走的**战况目标**）、`drivers`（当时读到的战况输入：temper 是 war/win/damage/withdraw，lone_wolf 是 neighbors，kiting 是 power/hardness/hurt）。blueprint：`theme`（设计主题）/ `components`（落到图上的选装）/ `city`+`building`（这次决策发生在哪个建造区；reaped 为 null）。",
                },
            }),
            _ => continue,
        };
        derived_tables.insert(t.name.to_string(), entry);
    }

    json!({
        "title": "planet_x 投影：lean 主流 + lazy id 索引表 + 派生表",
        "description": "agent 读 main.jsonl（每回合一行 lean 事实），需要重型明细时按 id 去 lazy 表查，需要引擎算出来的量（本回合流量、控制面）时读派生表。\n· eager 字段直接内联在 main.jsonl 里。\n· lazy 字段**不内联**：main.jsonl 只带它们的 id 数组（ship_ids/city_ids/faction_ids/body_ids/contract_ids），完整对象在 lazy 表里、按 id 索引。\n· 取 lazy 字段：Python kit 里 q.<field>(round=r) 或 q.join('<field>', round=r)；round=r 可省略则返回全量。\n· 派生表（derived）：数据**不在状态里**（引擎内部中间量/控制面），按 join_on 指的 main 列 join。",
        "generator": "planet_x",
        "schema_version": 2,
        "main_stream": MAIN,
        "meta": META,
        "eager": {
            "round":      {"type": "integer", "description": "回合号（月）。"},
            "time_month": {"type": "number", "description": "累计时间（月）。"},
            "event_ids":  {"type": "array", "items": {"type": "string"}, "description": "本回合事件 id（`<round>:<seq>`，join events 表用）。事件本体不再内联——见 lazy.events。"},
            "chronicle":  {"type": "array", "description": "剧情编年史（round id title body participants，累计叙事）。"},
            "metrics":    {"type": "object", "description": "总结指标（与 --schema 的 Trajectory.metrics 同构）：世界总量/实力占比/霸权/联盟/制裁/交战 + 各势力·各城产出/维护/治理。这是 agent 的轻量决策视图。"},
            "ship_ids":   {"type": "array", "items": {"type": "string"}, "description": "本回合存在的舰 id（=舰名，join ships 表用）。"},
            "city_ids":   {"type": "array", "items": {"type": "string"}, "description": "本回合**全部**城 id（=城名，join cities 表用）。含已夷平的空白城（razed 列筛）；与 cities 表逐行一致。"},
            "faction_ids": {"type": "array", "items": {"type": "string"}, "description": "本回合势力 id（=势力名，join factions 表用）。"},
            "contract_ids": {"type": "array", "items": {"type": "integer"}, "description": "本回合**未完成**的承包单号（join contracts 表用）。注意它是**数字**而不是实体名——挂单号由 `ContractState::next_id` 分配、单调递增不复用，所以历史事件里的单号永远指得准。"},
            "body_ids":   {"type": "array", "items": {"type": "string"}, "description": "天体 id（=天体名，join bodies 表用）。"},
            "settlement_ids": {"type": "array", "items": {"type": "string"}, "description": "全世界定居点 id（=定居点名，join settlements 表用）。"}
        },
        "lazy": lazy,
        "derived": derived_tables,
        "read_order": [
            "先读 schema.json，分清 eager（内联）/ lazy（索引）/ derived（引擎算出来的量）三类字段；",
            "读 main.jsonl 的 eager + metrics（轻量决策视图），按需拿 id；",
            "看外交/经济/军力全貌：q.factions(round=r)（势力主表：relations/resources/自有城与舰）；",
            "要「这回合产出/维护/治理到底是多少」：读 derived.flow / derived.city_flow（引擎内部中间量，状态里没有）；",
            "要「谁在控制什么」：读 derived.control（每个叶片一行）+ derived.scope（显式作用域节点），舰的有效指令看 ships 表的 order_effective* 列；",
            "要「AI 为什么这么决定」：读 derived.decisions（逐舰判定 withdraw/engage/colonize/bombard/move/hold + 当时的关键输入，以及船坞改装）——它既不发事件也不落状态，只有这里能读到；",
            "查「某城/某舰/某势力发生过什么」：用事件历史表——q.history('city', 城名) / q.history('ship', 舰名)（归一化参与方槽位，任意实体都能 join），或按类型直取稠密帧 q.events(type='city_razed')；",
            "要某舰/某城/某天体的完整对象时，用 Python kit 按 id join：q.ships(round=r) / q.join('ships', round=r)；",
            "要规则（舰级/建筑/组件/资源价值）时读 meta.json：Python kit 里 q.meta / q.ships_spec() / q.buildings_spec() / q.components_spec() / q.resource_value() —— 规则表可当 DataFrame 与 facts join。"
        ]
    })
}

#[cfg(test)]
#[path = "tests/projection/mod.rs"]
mod tests;
