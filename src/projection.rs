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
    // 事件历史：**归一化**的一行一事件（固定列 + 统一参与方槽位），取代此前内联在
    // main.jsonl 里的「serde 直接摊开的 tagged enum」——那种表 74.8% 的单元格是 null、
    // 且 `from`/`to` 一列两义（city_defected 是势力、capital_relocated 是天体）。
    LazyField { name: "events", table: "idx/events.jsonl", key: "event_id", id_col: "event_ids", round: true },
    LazyField { name: "bodies", table: "idx/bodies.jsonl", key: "body_id", id_col: "body_ids", round: false },
    LazyField { name: "settlements", table: "idx/settlements.jsonl", key: "settlement_id", id_col: "settlement_ids", round: false },
];

/// Emit the index projection of `rounds` rounds (round 0 then `rounds` steps) into `dir`.
/// Round 0 (the start state) uses an empty [`RoundFlow`] (no production yet); each later round
/// uses the flow `sim::advance` captured. Returns `Ok(())` on success.
pub fn write_index(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    rounds: u32,
    dir: &Path,
) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir.join(IDX_DIR)).map_err(|e| e.to_string())?;
    fs::write(dir.join(SCHEMA), projection_schema().to_string()).map_err(|e| e.to_string())?;
    // Static rules dictionary (same renderer as `--meta`), so `load(dir)` = world + rules +
    // join helpers in one directory; the Python kit reads it into `q.meta` + spec tables.
    fs::write(dir.join(META), crate::agent::meta_value(config).to_string()).map_err(|e| e.to_string())?;

    let mut main = BufWriter::new(File::create(dir.join(MAIN)).map_err(|e| e.to_string())?);
    let mut events = BufWriter::new(File::create(dir.join(idx_file("events"))).map_err(|e| e.to_string())?);
    let mut ships = BufWriter::new(File::create(dir.join(idx_file("ships"))).map_err(|e| e.to_string())?);
    let mut cities = BufWriter::new(File::create(dir.join(idx_file("cities"))).map_err(|e| e.to_string())?);
    let mut factions = BufWriter::new(File::create(dir.join(idx_file("factions"))).map_err(|e| e.to_string())?);
    let mut bodies = BufWriter::new(File::create(dir.join(idx_file("bodies"))).map_err(|e| e.to_string())?);
    let mut settlements = BufWriter::new(File::create(dir.join(idx_file("settlements"))).map_err(|e| e.to_string())?);

    // Global master table: body identity (name/orbit/settlements) — once, at round 0.
    for b in &state.bodies {
        let o = &b.orbit;
        writeln!(
            bodies,
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
                settlements,
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
    write_round(&mut main, &mut events, &mut ships, &mut cities, &mut factions, state, config, &sim::derived_from_state(state, config))?;
    for _ in 0..rounds {
        let derived = sim::advance(state, config, rng);
        write_round(&mut main, &mut events, &mut ships, &mut cities, &mut factions, state, config, &derived)?;
    }

    for w in [&mut main, &mut events, &mut ships, &mut cities, &mut factions, &mut bodies, &mut settlements] {
        w.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Write one round's main line, event rows, ship rows, city rows, and faction rows.
fn write_round(
    main: &mut BufWriter<File>,
    events: &mut BufWriter<File>,
    ships: &mut BufWriter<File>,
    cities: &mut BufWriter<File>,
    factions: &mut BufWriter<File>,
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
        "body_ids": state.bodies.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
        "settlement_ids": state.bodies
            .iter()
            .flat_map(|b| b.settlements.iter().map(|s| s.name.clone()))
            .collect::<Vec<_>>(),
    });
    writeln!(main, "{row}").map_err(|e| e.to_string())?;

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
        writeln!(events, "{row}").map_err(|e| e.to_string())?;
    }

    for s in &state.ships {
        let p = ship_panel(config, s);
        writeln!(
            ships,
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
            })
        )
        .map_err(|e| e.to_string())?;
    }

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
            cities,
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
            factions,
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
                "city_ids": city_ids,
                "ship_ids": ship_ids,
            })
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// The **agent-readable** projection schema. It is the single contract both the Rust emitter and
/// the Python kit share: fields with a `lazy` entry are NOT inline in `main.jsonl` (the main row
/// carries their id-array), while `eager` fields are inline. Column types are listed so the agent
/// knows the table shape without guessing.
pub fn projection_schema() -> serde_json::Value {
    let mut lazy = serde_json::Map::new();
    for f in LAZY {
        let entry = match f.name {
            "ships" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "舰的完整对象（class/组件/护甲/护盾/位置/速度 + effective 面板：attack/range/speed/upkeep 等），随回合变化。按 (round, ship_id) 索引。",
                "columns": {"round":"integer","ship_id":"string","faction_id":"string","class":"string","name":"string","x":"number","y":"number","hull":"number","hull_max":"number","shield":"number","shield_max":"number","velocity":"number","components":"array","component_hp":"array","attack":"number","attack_range":"number","speed":"number","accel":"number","hardness":"number","intercept":"number","shield_regen":"number","hull_regen":"number","upkeep":"number"},
            }),
            "cities" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "城的完整对象（人口/忠诚/治理距离/建筑清单/离心风险），随回合变化。按 (round, city_id) 索引。",
                "columns": {"round":"integer","city_id":"string","name":"string","body_id":"string","settlement":"string","faction_id":"string","population":"integer","loyalty":"number","razed":"boolean","deployed_area":"number","building_count":"integer","buildings":"array","gov_distance":"number","revolt_risk":"boolean"},
            }),
            "factions" => json!({
                "table": f.table, "key": f.key, "id_col": f.id_col, "round": f.round,
                "description": "势力的完整对象（库存/resources/relations/意识形态/本土防御 + 它拥有的城与舰），随回合变化。按 (round, faction_id) 索引。这是 agent 看外交 + 经济 + 军力的主表。",
                "columns": {"round":"integer","faction_id":"string","name":"string","symbol":"string","capital_body":"string","alignment":"number","aggression":"number","home_radius":"number","home_attack_mult":"number","home_regen_bonus":"number","ideology":"object","resources":"object","relations":"object","city_ids":"array","ship_ids":"array"},
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

    json!({
        "title": "planet_x 投影：lean 主流 + lazy id 索引表",
        "description": "agent 读 main.jsonl（每回合一行 lean 事实），需要重型明细时按 id 去 lazy 表查。\n· eager 字段直接内联在 main.jsonl 里。\n· lazy 字段**不内联**：main.jsonl 只带它们的 id 数组（ship_ids/city_ids/faction_ids/body_ids），完整对象在 lazy 表里、按 id 索引。\n· 取 lazy 字段：Python kit 里 q.<field>(round=r) 或 q.join('<field>', round=r)；round=r 可省略则返回全量。",
        "generator": "planet_x",
        "schema_version": 1,
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
            "body_ids":   {"type": "array", "items": {"type": "string"}, "description": "天体 id（=天体名，join bodies 表用）。"},
            "settlement_ids": {"type": "array", "items": {"type": "string"}, "description": "全世界定居点 id（=定居点名，join settlements 表用）。"}
        },
        "lazy": lazy,
        "read_order": [
            "先读 schema.json，分清 eager（内联）vs lazy（索引）字段；",
            "读 main.jsonl 的 eager + metrics（轻量决策视图），按需拿 id；",
            "看外交/经济/军力全貌：q.factions(round=r)（势力主表：relations/resources/自有城与舰）；",
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
    /// 根本不发事件：难民夺城（`displace_city_for_refugee`）零事件、`Resurgence` 不带 city。
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
}
