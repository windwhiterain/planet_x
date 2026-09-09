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
    let mut ships = BufWriter::new(File::create(dir.join(idx_file("ships"))).map_err(|e| e.to_string())?);
    let mut cities = BufWriter::new(File::create(dir.join(idx_file("cities"))).map_err(|e| e.to_string())?);
    let mut factions = BufWriter::new(File::create(dir.join(idx_file("factions"))).map_err(|e| e.to_string())?);
    let mut bodies = BufWriter::new(File::create(dir.join(idx_file("bodies"))).map_err(|e| e.to_string())?);
    let mut settlements = BufWriter::new(File::create(dir.join(idx_file("settlements"))).map_err(|e| e.to_string())?);

    // Global master table: body identity (name/orbit/settlements) — once, at round 0.
    for b in &state.bodies {
        let o = b.orbit;
        writeln!(
            bodies,
            "{}",
            json!({
                "body_id": b.name.clone(),
                "name": b.name,
                "perihelion_distance": o.perihelion_distance,
                "aphelion_distance": o.aphelion_distance,
                "period": o.period,
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
    write_round(&mut main, &mut ships, &mut cities, &mut factions, state, config, &RoundFlow::default())?;
    for _ in 0..rounds {
        let flow = sim::advance(state, config, rng);
        write_round(&mut main, &mut ships, &mut cities, &mut factions, state, config, &flow)?;
    }

    for w in [&mut main, &mut ships, &mut cities, &mut factions, &mut bodies, &mut settlements] {
        w.flush().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Write one round's main line, ship rows, city rows, and faction rows.
fn write_round(
    main: &mut BufWriter<File>,
    ships: &mut BufWriter<File>,
    cities: &mut BufWriter<File>,
    factions: &mut BufWriter<File>,
    state: &State,
    config: &GameConfig,
    flow: &RoundFlow,
) -> Result<(), String> {
    let metrics = sim::round_metrics(state, config, flow);
    let row = json!({
        "round": state.round,
        "time_month": r2(state.time_month),
        "events": state.events,
        "chronicle": state.chronicle,
        "metrics": metrics,
        "ship_ids": state.ships.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "city_ids": state.cities.iter().filter(|c| !c.razed).map(|c| c.name.clone()).collect::<Vec<_>>(),
        "faction_ids": state.factions.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        "body_ids": state.bodies.iter().map(|b| b.name.clone()).collect::<Vec<_>>(),
        "settlement_ids": state.bodies
            .iter()
            .flat_map(|b| b.settlements.iter().map(|s| s.name.clone()))
            .collect::<Vec<_>>(),
    });
    writeln!(main, "{row}").map_err(|e| e.to_string())?;

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
                "columns": {"round":"integer","faction_id":"string","name":"string","symbol":"string","capital_body":"string","alignment":"number","aggression":"number","home_radius":"number","home_attack_mult":"number","home_regen_bonus":"number","resources":"object","relations":"object","city_ids":"array","ship_ids":"array"},
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
            "events":     {"type": "array", "description": "本回合事件（type + 引用 ship/city id：开火/被毁/城夷平/殖民/开战/停战/剧情）。"},
            "chronicle":  {"type": "array", "description": "剧情编年史（round id title body participants，累计叙事）。"},
            "metrics":    {"type": "object", "description": "总结指标（与 --schema 的 Trajectory.metrics 同构）：世界总量/实力占比/霸权/联盟/制裁/交战 + 各势力·各城产出/维护/治理。这是 agent 的轻量决策视图。"},
            "ship_ids":   {"type": "array", "items": {"type": "string"}, "description": "本回合存在的舰 id（=舰名，join ships 表用）。"},
            "city_ids":   {"type": "array", "items": {"type": "string"}, "description": "本回合活城 id（=城名，join cities 表用）。"},
            "faction_ids": {"type": "array", "items": {"type": "string"}, "description": "本回合势力 id（=势力名，join factions 表用）。"},
            "body_ids":   {"type": "array", "items": {"type": "string"}, "description": "天体 id（=天体名，join bodies 表用）。"},
            "settlement_ids": {"type": "array", "items": {"type": "string"}, "description": "全世界定居点 id（=定居点名，join settlements 表用）。"}
        },
        "lazy": lazy,
        "read_order": [
            "先读 schema.json，分清 eager（内联）vs lazy（索引）字段；",
            "读 main.jsonl 的 eager + metrics（轻量决策视图），按需拿 id；",
            "看外交/经济/军力全貌：q.factions(round=r)（势力主表：relations/resources/自有城与舰）；",
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
    use std::path::PathBuf;

    /// The eager (inline) top-level field names, asserted to be described by [`projection_schema`].
    const MAJOR_EAGER: &[&str] = &[
        "round", "time_month", "events", "chronicle", "metrics", "ship_ids", "city_ids", "faction_ids", "body_ids", "settlement_ids",
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
            let ids = row["ship_ids"].as_array().unwrap();
            assert!(!ids.is_empty(), "main 每行要有 ship_ids（join 用）");
        }

        // lazy tables actually written.
        assert!(s.0.join("idx/ships.jsonl").exists());
        assert!(s.0.join("idx/cities.jsonl").exists());
        assert!(s.0.join("idx/factions.jsonl").exists());
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
}
