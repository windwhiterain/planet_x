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
            bodies: open("bodies")?,
            settlements: open("settlements")?,
        })
    }

    fn flush_all(&mut self) -> Result<(), String> {
        for w in [
            &mut self.main, &mut self.events, &mut self.ships, &mut self.cities, &mut self.factions,
            &mut self.contracts,
            &mut self.flow, &mut self.city_flow, &mut self.control, &mut self.scope,
            &mut self.bodies, &mut self.settlements,
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
                "city_ids": city_ids,
                "ship_ids": ship_ids,
            })
        )
        .map_err(|e| e.to_string())?;
    }

    // 承包挂单簿（**只留未完成的单子**：等人接的 + 正在履行的）。`carrier` 为 null
    // 表示还在挂单簿上等人接。完成的单子不在这张表里——「成交了」只留在 `events`。
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
                "amount": r2(c.amount),
                "delivered": r2(c.delivered),
                "outstanding": r2(c.outstanding()),
                "from": c.from,
                "to": c.to,
                "share": r2(c.share),
                "posted_round": c.posted_round,
                "deadline": c.deadline,
                "late": state.round > c.deadline,
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
                "columns": {"round":"integer","ship_id":"string","faction_id":"string","class":"string","name":"string","x":"number","y":"number","hull":"number","hull_max":"number","shield":"number","shield_max":"number","velocity":"number","components":"array","component_hp":"array","attack":"number","attack_range":"number","speed":"number","accel":"number","hardness":"number","intercept":"number","shield_regen":"number","hull_regen":"number","upkeep":"number","order_leaf_mode":"string","order_default_mode":"string","order_effective_mode":"string","order_effective":"object","doctrine":"object","kiting":"number","freighter":"boolean","freighter_mode":"string"},
                "column_docs": {
                    "order_leaf_mode": "本舰**叶片自己**的表态（没有叶片 = Inherit）。",
                    "order_default_mode": "势力级**舰队默认指令**的表态（没有这片叶 = Inherit）。",
                    "order_effective_mode": "**有效归属**：`State::ship_control` 的答案（叶 → 舰队默认 → 势力 scope → 全局 scope，最具体的有意见者胜；全继承 ⇒ Auto）。",
                    "order_effective": "**有效指令**：`State::ship_behavior` 的答案（null = 没有任何一层说话，调用方按 Idle 兜底）。注意「叶 Inherit + 舰队默认不是 Player」时会回落到叶上的记录值——这是引擎的既有取值规则，Python 侧不要自己重算。",
                    "doctrine": "**有效行为风格**（`State::ship_doctrine`：叶 → 舰队默认 → 舰上记录值）——{temper, lone_wolf}，各取 [-1,1]。舰上的 `Ship.doctrine` 只是出厂快照/AI 流水，不是这里。",
                    "kiting": "**有效风筝<->贴脸姿态**（`State::ship_kiting`，同一条链），[-1,1]，0 = 基线。",
                    "freighter": "**有效角色**（`State::ship_freighter`，同一条链）：`true` = 运输舰（自动控制给它排集货路线），`false` = 战舰（找仗打）。它**只管自动控制派哪种活**——不解除武装，运输舰照样自动开火、照样按 `kiting` 软移动。",
                    "freighter_mode": "角色那片叶的**有效归属**（`State::ship_freighter_control`）：Auto = 这条结论是自动控制写的（它每回合按积压定编），Player = 玩家钉的、AI 不碰。",
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
                "columns": {"round":"integer","faction_id":"string","name":"string","symbol":"string","capital_body":"string","alignment":"number","aggression":"number","home_radius":"number","home_attack_mult":"number","home_regen_bonus":"number","ideology":"object","resources":"object","relations":"object","reputation":"number","city_ids":"array","ship_ids":"array"},
                "column_docs": {
                    "reputation": "**信誉**（势力级，承包市场的准入资产）：承运人**不赔货值**，砸单只掉它，而托运方按它决定敢不敢把货交给你——所以它是这条腿上**唯一的抵押品**，低信誉者结构上接不到贵单/难单。中性值 1.0（没有任何承包履历）。公开值，不随回合自然衰减（涨跌都来自明确行为：按时交付/超期/丢货）。",
                },
            }),
            "contracts" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "**承包挂单簿**：一行一单，只含**未完成**的单子（等人接的 + 正在履行的）。完成的单子不在这里——“成交了/怎么结束的”去 `events` 里按类型查（`contract_posted` 及后续类型）。按 (round, contract_id) 索引。",
                "columns": {"round":"integer","contract_id":"integer","shipper":"string","carrier":"string","resource":"string","amount":"number","delivered":"number","outstanding":"number","from":"string","to":"string","share":"number","posted_round":"integer","deadline":"integer","late":"boolean"},
                "column_docs": {
                    "shipper": "托运方（挂单的人）。",
                    "carrier": "承运方；**null = 还在挂单簿上等人接**（这是本表最常用的一列：它是「市场上还没被吃掉的运力需求」）。",
                    "amount": "挂单总量（单位）。",
                    "delivered": "**已交付给托运方**的量——不含承运人自留的抽成（见 `share`）。",
                    "outstanding": "还差多少没送到 = `amount - delivered`。",
                    "from/to": "起运天体（托运方的产地货栈）→ 目的天体（照公理“首都即集散地”，`to` 永远是托运方首都）。",
                    "share": "承运人**抽成**比例：交付时从货里自留，其余进托运方首都池。没有货币转移——报酬就是它没交出去的那部分货。",
                    "posted_round/deadline": "挂单回合与截止回合。挂单时按**参考巡航速度**估出的宽裕时限定死（同一张单的时限不随接单者而变）。",
                    "late": "此刻是否已过截止期。**超期不作废**（只扣一次信誉，货照运、抽成照拿）——所以 `late=true` 的单子仍在履行中。",
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
                "description": "**控制面的 tidy 行**：每个叶片一行（舰指令 / 舰队默认指令 / 预算 / 权重 / 娱乐预算 / 首都）。值就是 `--control` 里那片叶的值，**不是**有效值——有效值见 ships 表的 `order_effective*` 列（引擎解析，别在 Python 里重实现链）。",
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
                "description": "**作用域树的显式表态**：谁负责 AI 决策（全局 / 势力 / 天体 / 城）。只发显式节点——`Inherit` 等于「这一层没有说话」，不占行。舰的归属链是 叶 → 舰队默认 → 势力 → 全局。",
                "columns": {"round":"integer","level":"string","key":"string","mode":"string"},
                "column_docs": {
                    "level": "节点层级：global / faction / body / city（`global` 的 key 为 `\"\"`）。",
                },
            }),
            "decisions" => json!({
                "table": t.table, "key": t.key, "join_on": t.join_on, "round": t.round,
                "description": "**本回合 AI 的判定**（`Derived.flow.decisions`）：逐舰「选了什么、当时的关键输入是多少」+ 船坞改装的「从什么改成什么」。这些判定**既不发事件、也不落持久状态**（指令叶只留结果），所以除了这张表和 `planet_x --derived` 没有别的读法——它回答的是「我的舰为什么跑到那儿去送死」。空白有意义：`verdict=\"hold\"` = 这回合 AI 没给这艘舰派活。",
                "columns": {"round":"integer","faction_id":"string","kind":"string","actor":"string","verdict":"string","target":"string","detail":"object"},
                "column_docs": {
                    "kind": "判定的种类：ship_order（逐舰行为判定）/ retool（船坞改装）。",
                    "actor": "作判定的一方：舰名（ship_order）/ 船坞所在城名（retool）。",
                    "verdict": "ship_order：withdraw（自保撤退）/ engage（接战）/ colonize（殖民复垦）/ bombard（就地轰炸）/ move（常规机动）/ haul（运输：跑集货路线，装/卸/在途都记成它）/ **hold（没派活）**；retool 固定为 retool。",
                    "target": "判定的对象：舰名（接战/撤退到首都）／城名（轰炸）／天体名（殖民）／新舰级（retool）；纯位置机动为 null（看 `detail.destination`）。",
                    "detail": "该 kind 的专属事实。ship_order：`hull_ratio`/`retreat_hull`（撤退判定的两个输入）、`kiting`（当时的有效风筝距离）、`enemy_in_range`、`after_move`（这次判定是否发生在移动之后——**一艘舰一回合最多两行**：先机动、到位后再判一次）、`destination`（驶向的坐标）、`order`（实际写回指令叶的行为，null = 没写叶）。retool：`from`（改装前舰级）、`building`（船坞在该城内的建筑下标，只在城内唯一）。",
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
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::prng::Prng;
    use crate::world::default_state;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;

    /// The eager (inline) top-level field names, asserted to be described by [`projection_schema`].
    const MAJOR_EAGER: &[&str] = &[
        "round", "time_month", "event_ids", "chronicle", "metrics", "ship_ids", "city_ids", "faction_ids", "body_ids", "settlement_ids",
    ];

    /// A scratch dir for one test, removed on drop.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!("planet_x_proj_{}_{tag}", std::process::id()));
            let _ = fs::remove_dir_all(&d);
            fs::create_dir_all(&d).unwrap();
            Self(d)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn jsonl(path: &Path) -> Vec<serde_json::Value> {
        let text = fs::read_to_string(path).unwrap();
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    /// The main stream must be **lean**: no heavy object collections inline — only eager fields +
    /// metrics + the id-arrays. The heavy fields live in the indexed tables. And all claimed lazy
    /// tables are actually written with the expected keys/columns.
    #[test]
    fn projection_writes_lean_main_and_indexed_tables() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 42);
        let mut rng = Prng::new(42);
        let s = Scratch::new("lean");
        write_index(&mut state, &cfg, &mut rng, 6, &s.0).unwrap();

        // schema.json: agent-readable eager/lazy contract.
        let schema: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(s.0.join("schema.json")).unwrap()).unwrap();
        let lazy = schema["lazy"].as_object().unwrap();
        for f in LAZY {
            assert!(lazy.contains_key(f.name), "schema.lazy 缺 {}", f.name);
            assert_eq!(lazy[f.name]["key"], f.key, "schema.lazy.{}.key 错", f.name);
            assert_eq!(lazy[f.name]["table"], f.table, "schema.lazy.{}.table 错", f.name);
        }
        let eager = schema["eager"].as_object().unwrap();
        for k in MAJOR_EAGER {
            assert!(eager.contains_key(*k), "schema.eager 缺 {k}");
        }

        // main.jsonl: 7 lines for `--round 6` (round 0 + 6), and each is lean.
        let main = jsonl(&s.0.join("main.jsonl"));
        assert_eq!(main.len(), 7, "main.jsonl should have round 0 + 6 rounds");
        assert_eq!(main[0]["round"], 0);
        for row in &main {
            for obj in ["ships", "cities", "factions", "bodies", "settlements"] {
                assert!(!row.as_object().unwrap().contains_key(obj), "main 不应内联 {obj}");
            }
            // 事件已改为 lazy：主流只带 event_ids，不再内联 events。
            assert!(!row.as_object().unwrap().contains_key("events"), "main 不应内联 events（已 lazy 化）");
            assert!(row["contract_ids"].is_array(), "main 每行要有 contract_ids（join contracts 用）");
            let ids = row["ship_ids"].as_array().unwrap();
            assert!(!ids.is_empty(), "main 每行要有 ship_ids（join 用）");
            assert!(row["event_ids"].is_array(), "main 每行要有 event_ids（join events 用）");
        }

        // lazy tables actually written.
        assert!(s.0.join("idx/ships.jsonl").exists());
        assert!(s.0.join("idx/cities.jsonl").exists());
        assert!(s.0.join("idx/factions.jsonl").exists());
        assert!(s.0.join("idx/events.jsonl").exists());
        assert!(s.0.join("idx/bodies.jsonl").exists());
        assert!(s.0.join("idx/settlements.jsonl").exists());
        // 承包挂单簿：表必须存在，且列面与 schema 声明一致（挂单号/托运方/承运方/截止期）。
        assert!(s.0.join("idx/contracts.jsonl").exists(), "缺 idx/contracts.jsonl（承包挂单簿）");
        let contract_rows = jsonl(&s.0.join("idx/contracts.jsonl"));
        assert!(
            !contract_rows.is_empty(),
            "6 回合内该有挂单（离岸产出落进货栈、自己运不动就挂出去）——空表会让下面的列面守卫空转"
        );
        for row in contract_rows {
            for col in ["contract_id", "shipper", "carrier", "amount", "deadline"] {
                assert!(row.get(col).is_some(), "contracts 表缺列 {col}: {row}");
            }
            assert!(row.get("outstanding").is_some(), "contracts 表要有 outstanding（还差多少没送到）");
        }
        let ships = jsonl(&s.0.join("idx/ships.jsonl"));
        assert!(!ships.is_empty());
        assert!(ships[0].get("ship_id").is_some(), "ships 表要有 ship_id 列");
        assert!(ships[0].get("components").is_some(), "ships 表要有 components 列");
        // factions table: has relations + resources, and its own city/ship id lists.
        let factions = jsonl(&s.0.join("idx/factions.jsonl"));
        assert!(!factions.is_empty());
        assert!(factions[0].get("faction_id").is_some(), "factions 表要有 faction_id 列");
        assert!(factions[0].get("relations").is_some(), "factions 表要有 relations");
        assert!(factions[0].get("resources").is_some(), "factions 表要有 resources（库存）");
        // cities table: governance distance + revolt-risk are game-derived but emitted for the agent.
        let cities = jsonl(&s.0.join("idx/cities.jsonl"));
        assert!(!cities.is_empty());
        assert!(cities[0].get("gov_distance").is_some(), "cities 表要有 gov_distance（治理距离）");
        assert!(
            cities[0].get("depot_value").is_some(),
            "cities 表要有 depot_value（产地货栈：压在产地、还没运回首都的存货价值）"
        );
        assert!(cities[0].get("revolt_risk").is_some(), "cities 表要有 revolt_risk（离心风险）");

        // events table: 归一化固定列（一行一事件），参与方走统一槽位。
        let events = jsonl(&s.0.join("idx/events.jsonl"));
        assert!(!events.is_empty(), "6 回合后应有事件");
        for col in ["round", "seq", "event_id", "type", "salience", "actor_kind", "actor_id",
                    "target_kind", "target_id", "extra", "magnitude", "headline", "data"] {
            assert!(events[0].get(col).is_some(), "events 表要有 {col} 列");
        }
        // 归一化的意义：**没有任何一列是 variant 专属字段**，否则又会回到「同名多义」
        // （`from`/`to` 一列两义）与「同角色多名」（faction/owner/fallen_to/from/to）。
        for forbidden in ["from", "to", "a", "b", "attacker", "target", "city", "ship", "body", "owner"] {
            assert!(events[0].get(forbidden).is_none(), "events 表不应有 variant 专属列 {forbidden}（应进 data/统一槽位）");
        }
        assert!(events.iter().all(|e| e["salience"].is_string()), "salience 必须是字符串分级");
        // 每个事件至少有一个被命名的实体（否则它无法被任何实体 join 到）。
        assert!(
            events.iter().all(|e| e["actor_id"].is_string() || e["target_id"].is_string()
                || !e["extra"].as_array().map(|a| a.is_empty()).unwrap_or(true)),
            "每个事件至少要有一个参与方实体"
        );
    }

    /// **同回合归属翻转不变量**：一个回合内，同一座城不能易主两次。
    ///
    /// 这条不变量是「僵尸势力夺城—倒戈振荡」的**结构性**约束：`step_governance`（离心倒戈）
    /// 先跑，`step_resurgence`（难民夺城）后跑，后者若把前者刚放手的那座城夺回来，两个步进
    /// 就在同一回合里**正好互相抵消**——净效果为零，却照样记两条里程碑、白造一艘种子舰。
    /// 实测 seed 7 的 `冥王星前哨` 就是这样被钉进 4 回合一轮的死循环（60 回合 41 次夺城）。
    ///
    /// 「活城易主」= `city_defected` / `city_overrun` / `colony_founded`（城活着，换了主人或
    /// 从空白重新立起来）。三者之和每 `(回合, 城)` 最多 1 条。
    ///
    /// 注意：`city_razed` → `colony_founded`（被夷平后同回合复垦）**不算**违规——城经过了
    /// 「死亡」这个中间态，是两件不同的事（先被拆平、再被重建），两条事件都是真的。
    #[test]
    fn no_city_changes_owner_twice_in_one_round() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 7);
        let mut rng = Prng::new(7);
        let s = Scratch::new("no_double_flip");
        write_index(&mut state, &cfg, &mut rng, 120, &s.0).unwrap();

        let events = jsonl(&s.0.join("idx/events.jsonl"));
        let mut flips: BTreeMap<(u32, String), Vec<String>> = BTreeMap::new();
        for e in &events {
            let ty = e["type"].as_str().unwrap_or_default();
            if !matches!(ty, "city_defected" | "city_overrun" | "colony_founded") {
                continue;
            }
            let city = e["data"]["city"].as_str().unwrap_or_default().to_string();
            let round = e["round"].as_u64().unwrap_or_default() as u32;
            flips.entry((round, city)).or_default().push(e["headline"].as_str().unwrap_or("").to_string());
        }
        let bad: Vec<_> = flips.iter().filter(|(_, v)| v.len() > 1).collect();
        assert!(
            bad.is_empty(),
            "有 {} 座城在同一回合里易主了两次（净效果为零的自相抵消）：{:#?}",
            bad.len(),
            bad.iter().take(5).collect::<Vec<_>>()
        );
        assert!(flips.len() >= 5, "只观察到 {} 次活城易主，样本太稀——守卫可能是空转", flips.len());
    }

    /// **标题必须点到名**：`GameEvent::participants()` 列出的每一个实体 id，都要**逐字出现**
    /// 在 `headline()` 里（单行、非空）。
    ///
    /// 这把「索引指向谁」和「人读到的句子说的是谁」钉在一起：查询 join 到的实体，一定能在
    /// 那句话里看见；反之标题里出现的实体也不会是索引之外的幽灵。同时钉住「标题自足」——
    /// 它只读事件自身的字段，所以对归档的老历史同样成立。
    #[test]
    fn headline_names_every_participant() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 7);
        let mut rng = Prng::new(7);
        let s = Scratch::new("headline_names");
        write_index(&mut state, &cfg, &mut rng, 80, &s.0).unwrap();

        let events = jsonl(&s.0.join("idx/events.jsonl"));
        let mut checked = 0usize;
        for e in &events {
            let h = e["headline"].as_str().unwrap_or_default();
            assert!(!h.is_empty(), "{} 没有标题", e["type"]);
            assert!(!h.contains('\n'), "标题必须单行: {h:?}");
            let mut ids: Vec<String> = Vec::new();
            if let Some(v) = e["actor_id"].as_str() { ids.push(v.to_string()); }
            if let Some(v) = e["target_id"].as_str() { ids.push(v.to_string()); }
            for p in e["extra"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(v) = p["id"].as_str() { ids.push(v.to_string()); }
            }
            for id in &ids {
                assert!(h.contains(id.as_str()), "标题 {h:?} 没提到参与方 {id:?}（{}）", e["type"]);
                checked += 1;
            }
        }
        assert!(checked >= 50, "只校验了 {checked} 个参与方名字，守卫可能是空转");
    }

    /// **失城方必须在夷平那一刻记下**：`city_razed.owner` 是夷平时的持有者，**不是**同回合
    /// 后来复垦者的名字。
    ///
    /// 这正是 `step_ideology` 那段「事后回读」栽的坑：夷平不改 `faction_id`（空白城保留最后
    /// 主人的 diaspora claim），而同回合稍后的 `reseed_city` 会把它改成新主，于是事后再读
    /// 只会读到**抢城的人**。活体样本 seed 7 r24：大红斑科学站被欧盟夷平、同回合被无国界
    /// 科学组织复垦，战功被记到了抢城者头上。
    ///
    /// **样本改为确定性构造**：原先靠长局恰好撞上「被 A 夷平、同回合被 B 复垦」，而唯一大量
    /// 产生这种巧合的 `step_resurgence` 已删除（D5），长局样本随之消失、守卫空转。现在直接用
    /// 真实漏斗造出这个巧合（`raze_city` → `reseed_city`，同一回合），再让投影把 round 0 的
    /// 事件落盘校验——**守卫再也不会空转**。
    #[test]
    fn city_razed_records_the_loser_not_the_refounder() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 7);
        let mut rng = Prng::new(7);
        let s = Scratch::new("razed_loser");

        // 挑一座活城（失城方 A）与一支别的势力（抢城方 B）。
        let (city, loser) = state
            .cities
            .iter()
            .find(|c| !c.razed)
            .map(|c| (c.name.clone(), c.faction_id.clone()))
            .expect("世界生成必须至少有一座活城");
        let founder = state
            .factions
            .iter()
            .map(|f| f.name.clone())
            .find(|f| f != &loser)
            .expect("世界必须至少有两个势力");
        let body = state.city(&city).map(|c| c.body_id.clone()).expect("city");

        // 同一回合：先被 A 的对头夷平，再被 B 复垦（= 那个会写错战功的巧合）。
        crate::sim::raze_city(
            &mut state,
            &city,
            crate::sim::RazeCause::Bombardment {
                by_ship: "测试舰".to_string(),
                by_faction: founder.clone(),
                damage: 1.0,
            },
        );
        let mut next_building = state
            .cities
            .iter()
            .flat_map(|c| c.buildings.iter().map(|b| b.id))
            .max()
            .map_or(0, |m| m + 1);
        let class = state
            .ships
            .iter()
            .find(|sh| sh.faction_id == founder)
            .map(|sh| sh.class.clone())
            .unwrap_or_else(|| "corvette".to_string());
        assert!(
            crate::sim::reseed_city(&mut state, &cfg, &city, &founder, &class, &mut next_building),
            "复垦应当成功（同回合制造出「夷平 → 被别家复垦」这个巧合）"
        );
        let _ = body;

        // 只落盘 round 0（不要推进回合，否则 advance 会清空本回合事件）。
        write_index(&mut state, &cfg, &mut rng, 0, &s.0).unwrap();

        let events = jsonl(&s.0.join("idx/events.jsonl"));
        let mut razed_with_revival = 0usize;
        for e in &events {
            if e["type"] != "city_razed" {
                continue;
            }
            let round = e["round"].as_u64().unwrap();
            let city = e["data"]["city"].as_str().unwrap();
            let owner = e["data"]["owner"].as_str().unwrap();
            let fallen_to = e["data"]["fallen_to"].as_str().unwrap();
            assert_ne!(owner, fallen_to, "夷平一座城不该由它的持有者自己造成（{city}）");
            assert_eq!(owner, loser, "city_razed.owner 必须是失城方，而不是抢城者");
            // 同回合、同一座城的复垦者若存在，必然**不是** owner 被写成的那个名字。
            for f in &events {
                if f["type"] != "colony_founded" || f["round"].as_u64() != Some(round) {
                    continue;
                }
                if f["data"]["city"].as_str() != Some(city) {
                    continue;
                }
                let fdr = f["data"]["owner"].as_str().unwrap_or_default();
                let prev = f["data"]["prev_owner"].as_str().unwrap_or_default();
                assert_eq!(prev, owner, "{city} 同回合被 {fdr} 复垦，prev_owner 应等于失城方 {owner}");
                if fdr != owner {
                    razed_with_revival += 1;
                }
            }
        }
        assert!(
            razed_with_revival >= 1,
            "确定性样本里没有「被 A 夷平、同回合被 B 复垦」的城——这条守卫没能真的验到那个坑"
        );
    }

    /// The projection is deterministic: the same seed → byte-identical `main.jsonl`.
    #[test]
    fn projection_is_deterministic() {
        let cfg = load_config();
        let run = |tag: &str| -> Vec<u8> {
            let mut state = default_state(&cfg, 42);
            let mut rng = Prng::new(42);
            let s = Scratch::new(tag);
            write_index(&mut state, &cfg, &mut rng, 20, &s.0).unwrap();
            fs::read(s.0.join("main.jsonl")).unwrap()
        };
        assert_eq!(run("a"), run("b"), "same seed must reproduce identical main.jsonl");
    }

    /// The event milestones is deterministic too: same seed → byte-identical `idx/events.jsonl`.
    ///
    /// NOTE: `Scratch` 的目录名是「进程 id + tag」，而 cargo 的测试是**同进程多线程并行**的，
    /// 所以 tag 必须在全文件内唯一——与 `projection_is_deterministic` 共用 "a"/"b" 会撞目录。
    #[test]
    fn event_milestones_is_deterministic() {
        let cfg = load_config();
        let run = |tag: &str| -> Vec<u8> {
            let mut state = default_state(&cfg, 42);
            let mut rng = Prng::new(42);
            let s = Scratch::new(tag);
            write_index(&mut state, &cfg, &mut rng, 20, &s.0).unwrap();
            fs::read(s.0.join("idx/events.jsonl")).unwrap()
        };
        assert_eq!(run("milestones_a"), run("milestones_b"), "same seed must reproduce identical event milestones");
    }

    /// **完备性守卫**：密集快照里可见的每一次「城的归属 / 存亡」变化，都必须有一条**命名
    /// 了这座城**的事件来解释。
    ///
    /// 这正是此前最痛的那个洞——「城市易主了，就近是什么事件导致？」答不上来，因为若干路径
    /// 根本不发事件：难民夺城（`displace_city_for_refugee`）零事件（`SpawnVia::Resurgence`
    /// 与 `CityOverrun` 已在贸易分支删除——它们不再有任何生产者，留着只会骗下一个读者）。
    /// 稀疏历史安全的前提就是「能证明自己什么都没丢」；这条测试就是那个证明。它同时是
    /// 「事件系统不许退化成顺手记的副产品」的回归防线。
    #[test]
    fn every_city_state_change_is_explained_by_an_event() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 42);
        let mut rng = Prng::new(42);
        let s = Scratch::new("reconcile");
        write_index(&mut state, &cfg, &mut rng, 120, &s.0).unwrap();

        // 每回合「被事件命名过的实体」集合：(kind, id)。
        let mut named: BTreeMap<u32, BTreeSet<(String, String)>> = BTreeMap::new();
        for e in jsonl(&s.0.join("idx/events.jsonl")) {
            let round = e["round"].as_u64().unwrap() as u32;
            let set = named.entry(round).or_default();
            for (kc, ic) in [("actor_kind", "actor_id"), ("target_kind", "target_id")] {
                if let (Some(k), Some(i)) = (e[kc].as_str(), e[ic].as_str()) {
                    set.insert((k.to_string(), i.to_string()));
                }
            }
            for p in e["extra"].as_array().cloned().unwrap_or_default() {
                if let (Some(k), Some(i)) = (p["kind"].as_str(), p["id"].as_str()) {
                    set.insert((k.to_string(), i.to_string()));
                }
            }
        }

        // 逐回合对比密集快照：任何 (faction_id, razed) 变化都必须被解释。
        // `prev`/`cur` 分开两张表：文件按回合成块，遇到新回合才把 cur 提升为 prev。
        let mut prev: BTreeMap<String, (String, bool)> = BTreeMap::new();
        let mut cur: BTreeMap<String, (String, bool)> = BTreeMap::new();
        let mut cur_round = u32::MAX;
        let mut unexplained: Vec<String> = Vec::new();
        // 真正被检查到的变化次数——**守卫必须非空**：一个什么都没检查的绿灯等于没有守卫。
        let mut checked = 0usize;
        for c in jsonl(&s.0.join("idx/cities.jsonl")) {
            let round = c["round"].as_u64().unwrap() as u32;
            if round != cur_round {
                prev = std::mem::take(&mut cur);
                cur_round = round;
            }
            let cid = c["city_id"].as_str().unwrap().to_string();
            let now = (
                c["faction_id"].as_str().unwrap_or_default().to_string(),
                c["razed"].as_bool().unwrap_or(false),
            );
            if let Some(p) = prev.get(&cid) {
                if p != &now {
                    checked += 1;
                    let explained = named
                        .get(&round)
                        .map(|set| set.contains(&("city".to_string(), cid.clone())))
                        .unwrap_or(false);
                    if !explained {
                        unexplained.push(format!("r{round} 城 {cid}: {p:?} → {now:?}"));
                    }
                }
            }
            cur.insert(cid, now);
        }
        assert!(
            checked >= 5,
            "该窗口内只看到 {checked} 次城状态变化——守卫几乎没在检查东西，请换更长窗口/种子"
        );
        assert!(
            unexplained.is_empty(),
            "存在**未被任何事件解释**的城状态变化（历史不完备，agent 会答不出「为什么易主」）：\n· {}",
            unexplained.join("\n· ")
        );
        println!("完备性守卫：{checked} 次城状态变化全部有事件解释");
    }

    /// 把 `idx/ships.jsonl` 读成 `round -> 该回合存在的舰 id 集合`。
    ///
    /// 注意：`idx/ships.jsonl` 只在舰**活着**时写行（模拟里 `retain(hull > 0)` 不留尸行），
    /// 所以「一艘舰不见了」这件事**只能由事件表解释**——这正是这条守卫要钉住的不变量。
    fn ships_by_round(dir: &Path) -> BTreeMap<u32, BTreeSet<String>> {
        let mut out: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
        for r in jsonl(&dir.join("idx/ships.jsonl")) {
            let round = r["round"].as_u64().unwrap() as u32;
            if let Some(s) = r["ship_id"].as_str() {
                out.entry(round).or_default().insert(s.to_string());
            }
        }
        out
    }

    /// 某类事件的**目标舰**按回合集合（`ship_destroyed` / `ship_spawned` 的 `target_id` 即舰名）。
    fn event_ships(dir: &Path, ty: &str) -> BTreeMap<u32, BTreeSet<String>> {
        let mut out: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
        for e in jsonl(&dir.join("idx/events.jsonl")) {
            if e["type"] != ty {
                continue;
            }
            let round = e["round"].as_u64().unwrap() as u32;
            if let Some(s) = e["target_id"].as_str() {
                out.entry(round).or_default().insert(s.to_string());
            }
        }
        out
    }

    /// **舰的存亡对账**：密集表里「出现 / 消失」的每一艘舰都必须被事件解释——
    /// 消失 = 死因事件（`ship_destroyed`，带 `cause`/`by`），出现 = 造舰事件
    /// （`ship_spawned`，带 `via`）。任何一头缺失都意味着**某条路径漏了 `kill_ship` /
    /// `spawn_ship` 漏斗**。
    ///
    /// 这条守卫是 Stage B 的直接收获：它一上线就抓出 `step_resurgence` 的种子舰
    /// **完全不发造舰事件**（实测 63 次出生里 46 次无解释）——那是一条与「城市易主查不到
    /// 原因」完全同源的漏洞，只是藏在舰这一侧。
    #[test]
    fn every_ship_state_change_is_explained_by_an_event() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 42);
        let mut rng = Prng::new(42);
        let s = Scratch::new("reconcile_ships");
        write_index(&mut state, &cfg, &mut rng, 120, &s.0).unwrap();

        let alive = ships_by_round(&s.0);
        let dead = event_ships(&s.0, "ship_destroyed");
        let born = event_ships(&s.0, "ship_spawned");
        let max_round = alive.keys().copied().max().unwrap_or(0);

        let (mut checked_death, mut checked_birth) = (0usize, 0usize);
        let mut unexplained: Vec<String> = Vec::new();
        for r in 1..=max_round {
            let empty = BTreeSet::new();
            let prev = alive.get(&(r - 1)).unwrap_or(&empty);
            let cur = alive.get(&r).unwrap_or(&empty);
            for sid in prev.difference(cur) {
                checked_death += 1;
                if !dead.get(&r).map(|d| d.contains(sid)).unwrap_or(false) {
                    unexplained.push(format!("r{r} 舰 {sid} 消失但没有死因事件"));
                }
            }
            for sid in cur.difference(prev) {
                checked_birth += 1;
                if !born.get(&r).map(|d| d.contains(sid)).unwrap_or(false) {
                    unexplained.push(format!("r{r} 舰 {sid} 出现但没有造舰事件"));
                }
            }
        }
        assert!(
            checked_death >= 5 && checked_birth >= 5,
            "窗口内只看到 {checked_death} 次死亡 / {checked_birth} 次出生——守卫几乎没在检查东西"
        );
        assert!(
            unexplained.is_empty(),
            "存在**未被任何事件解释**的舰存亡变化（漏斗有漏；agent 会答不出「这艘舰哪去了/哪来的」）：\n· {}",
            unexplained.join("\n· ")
        );
        println!("完备性守卫：{checked_death} 次舰死亡 / {checked_birth} 次舰出生全部有事件解释");
    }

    /// **派生表契约**：`DERIVED` 声明的每张表都要真的写出来、要在 `schema.derived` 里有条目、
    /// 它的 `join_on` 必须是 `main.jsonl` 真有的列（否则 Python 侧 join 会静默错）。加表忘了
    /// 改 schema 是这套数据面最容易犯的错，所以这里三处一起钉。
    #[test]
    fn derived_tables_are_written_and_declared() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 7);
        let mut rng = Prng::new(7);
        let s = Scratch::new("derived_contract");
        write_index(&mut state, &cfg, &mut rng, 3, &s.0).unwrap();

        let schema: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(s.0.join("schema.json")).unwrap()).unwrap();
        let derived = schema["derived"].as_object().unwrap();
        assert_eq!(derived.len(), DERIVED.len(), "schema.derived 的条目数应等于 DERIVED");
        let main = jsonl(&s.0.join("main.jsonl"));
        let row0 = main[0].as_object().unwrap();
        for t in DERIVED {
            assert!(derived.contains_key(t.name), "schema.derived 缺 {}", t.name);
            assert_eq!(derived[t.name]["table"], t.table, "schema.derived.{}.table 错", t.name);
            assert_eq!(derived[t.name]["join_on"], t.join_on, "schema.derived.{}.join_on 错", t.name);
            assert!(
                !jsonl(&s.0.join(t.table)).is_empty(),
                "派生表 {} 没有写出来（{}）",
                t.name,
                t.table
            );
            if !t.join_on.is_empty() {
                assert!(row0.contains_key(t.join_on), "main.jsonl 缺 join 列 {}", t.join_on);
            }
        }
        // ships 表的指令归属列：引擎解析的结果必须在表里（Python 不该自己重实现链）。
        let ships = jsonl(&s.0.join("idx/ships.jsonl"));
        for col in ["order_leaf_mode", "order_default_mode", "order_effective_mode", "order_effective"] {
            assert!(ships[0].get(col).is_some(), "ships 表缺 {col}");
        }
    }

    /// 派生表里的数就是 `Derived` 里的数（**不做舍入**）：`--index` 的 `derived.flow` 与
    /// `planet_x --derived` 读同一份 `Derived`，两个读面必须给同一个值。
    #[test]
    fn flow_table_matches_the_derived_record() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 11);
        let mut rng = Prng::new(11);
        let s = Scratch::new("flow_match");
        let outcome = write_index(&mut state, &cfg, &mut rng, 8, &s.0).unwrap();

        let flow = jsonl(&s.0.join("idx/flow.jsonl"));
        let last_round = state.round;
        let last: Vec<&serde_json::Value> =
            flow.iter().filter(|r| r["round"] == json!(last_round)).collect();
        assert!(!last.is_empty(), "最后一回合应有 flow 行");
        let mut checked = 0usize;
        for row in last {
            let fid = row["faction_id"].as_str().unwrap();
            let expect_upkeep = outcome.post.flow.upkeep.get(fid).copied().unwrap_or(0.0);
            assert_eq!(
                row["upkeep"].as_f64().unwrap(),
                expect_upkeep,
                "{fid} 的 upkeep 与 Derived.flow 不一致（读了两个不同的数）"
            );
            let expect_prod = outcome
                .post
                .flow
                .faction_production
                .get(fid)
                .cloned()
                .unwrap_or_default();
            assert_eq!(row["production"], serde_json::to_value(&expect_prod).unwrap(), "{fid} 的 production 不一致");
            checked += 1;
        }
        assert!(checked >= 2, "只检查了 {checked} 个势力的 flow 行——守卫太空");

        // city_flow 同理（挑一个真有产出的城，别拿空表当通过）。
        let city_flow = jsonl(&s.0.join("idx/city_flow.jsonl"));
        let with_prod: Vec<&serde_json::Value> = city_flow
            .iter()
            .filter(|r| r["round"] == json!(last_round))
            .filter(|r| r["production"].as_object().map(|o| !o.is_empty()).unwrap_or(false))
            .collect();
        assert!(!with_prod.is_empty(), "最后一回合应有带产出的城（否则这条守卫没在检查任何东西）");
        for row in with_prod {
            let cid = row["city_id"].as_str().unwrap();
            let expect = outcome
                .post
                .flow
                .city_production
                .get(cid)
                .cloned()
                .unwrap_or_default();
            assert_eq!(row["production"], serde_json::to_value(&expect).unwrap(), "{cid} 的产出不一致");
        }
    }

    /// 控制面表：每个叶片一行，`mode` 与状态里的一致；`capital` 这种可空叶也在。
    #[test]
    fn control_table_holds_every_leaf() {
        let cfg = load_config();
        let mut state = default_state(&cfg, 7);
        // 造几片叶：一个玩家叶、一个势力级默认、一笔预算。
        let fid = state.factions[0].name.clone();
        let ship = state.ships.iter().find(|s| s.faction_id == fid).map(|s| s.name.clone());
        let mut c = crate::model::ControllableState::default();
        if let Some(ship) = &ship {
            c.ship_orders.insert(
                ship.clone(),
                Control { value: ShipBehavior::Idle, mode: ControlMode::Player },
            );
        }
        c.default_ship_order = Some(Control { value: ShipBehavior::Idle, mode: ControlMode::Player });
        // 风格四片叶（`control-live-layers.md` §3 那条候选）：四片都要出现在表里——
        // 少了它们，「这艘舰的风格是它自己钉的，还是跟着舰队默认走」在表里就查不出来。
        if let Some(ship) = &ship {
            c.ship_doctrine.insert(
                ship.clone(),
                Control { value: crate::model::ShipDoctrine { temper: 0.71, lone_wolf: -0.25 }, mode: ControlMode::Player },
            );
            c.ship_kiting.insert(ship.clone(), Control { value: -0.6, mode: ControlMode::Player });
        }
        c.default_doctrine = Some(Control {
            value: crate::model::ShipDoctrine { temper: 0.25, lone_wolf: 0.5 },
            mode: ControlMode::Auto,
        });
        c.default_kiting = Some(Control { value: 0.2, mode: ControlMode::Player });
        c.construction_budget.insert(
            "铁".to_string(),
            Control { value: 3.5, mode: ControlMode::Player },
        );
        state.control.insert(fid.clone(), c);
        state.scope.factions.insert(fid.clone(), ControlMode::Player);

        let mut rng = Prng::new(7);
        let s = Scratch::new("control_table");
        write_index(&mut state, &cfg, &mut rng, 0, &s.0).unwrap();

        let rows = jsonl(&s.0.join("idx/control.jsonl"));
        let has = |kind: &str| rows.iter().any(|r| r["kind"] == json!(kind) && r["faction_id"] == json!(fid));
        assert!(has("default_ship_order"), "缺舰队默认指令行");
        assert!(has("construction_budget"), "缺预算行");
        // 风格四片叶：值与**自己的** mode 都要在（不是有效值、不是有效归属）。
        let doc = rows
            .iter()
            .find(|r| r["kind"] == json!("ship_doctrine") && r["key"] == json!(ship.clone().unwrap_or_default()))
            .expect("缺逐舰风格叶行");
        assert_eq!(doc["value"], json!({"temper": 0.71, "lone_wolf": -0.25}), "风格叶的值应是叶自己的值");
        assert_eq!(doc["mode"], json!("Player"));
        assert!(
            rows.iter().any(|r| r["kind"] == json!("ship_kiting") && r["value"] == json!(-0.6)),
            "缺逐舰风筝姿态叶行"
        );
        let dd = rows.iter().find(|r| r["kind"] == json!("default_doctrine")).expect("缺舰队默认风格行");
        assert_eq!(dd["mode"], json!("Auto"), "势力级默认风的 mode 也要如实带出来");
        assert!(has("default_kiting"), "缺舰队默认风筝姿态行");
        if let Some(ship) = &ship {
            let row = rows
                .iter()
                .find(|r| r["kind"] == json!("ship_order") && r["key"] == json!(ship))
                .expect("缺该舰的指令叶行");
            assert_eq!(row["mode"], json!("Player"), "叶的 mode 应与状态一致");
        }
        // scope 表：显式节点一行（global 恒定 + 我们刚钉的势力）。
        let scope = jsonl(&s.0.join("idx/scope.jsonl"));
        assert!(scope.iter().any(|r| r["level"] == json!("global")), "scope 缺 global 行");
        assert!(
            scope.iter().any(|r| r["level"] == json!("faction") && r["key"] == json!(fid) && r["mode"] == json!("Player")),
            "scope 缺该势力的显式表态"
        );

        // ships 表的有效归属列必须与 `State::ship_control` 一致（这是"引擎给答案，Python 不重算"）。
        let ships = jsonl(&s.0.join("idx/ships.jsonl"));
        let mut checked = 0usize;
        for row in ships.iter().filter(|r| r["round"] == json!(0)) {
            let name = row["ship_id"].as_str().unwrap();
            assert_eq!(
                row["order_effective_mode"],
                serde_json::to_value(state.ship_control(name.to_string())).unwrap(),
                "{name} 的 order_effective_mode 与 State::ship_control 不一致"
            );
            checked += 1;
        }
        assert!(checked > 0, "ships 表里一艘舰都没有——守卫没在检查东西");
    }
}
