//! projection 的单元测试（≤48 回合的读面守卫）。

use super::*;
use crate::config::load_config;
use crate::prng::Prng;
use crate::world::default_state;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// The eager (inline) top-level field names, asserted to be described by [`projection_schema`].
const MAJOR_EAGER: &[&str] = &[
    "round", "time_month", "event_ids", "chronicle", "view", "ship_ids", "city_ids", "faction_ids", "body_ids", "settlement_ids",
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
    // 雇佣挂单簿：表必须存在，且列面与 schema 声明一致（挂单号/雇主/受雇方/要求运力/期限）。
    assert!(s.0.join("idx/contracts.jsonl").exists(), "缺 idx/contracts.jsonl（雇佣挂单簿）");
    let contract_rows = jsonl(&s.0.join("idx/contracts.jsonl"));
    assert!(
        !contract_rows.is_empty(),
        "6 回合内该有挂单（离岸产出落进货栈、自己运不动就挂出去）——空表会让下面的列面守卫空转"
    );
    for row in contract_rows {
        for col in ["contract_id", "shipper", "carrier", "capacity", "expires_round"] {
            assert!(row.get(col).is_some(), "contracts 表缺列 {col}: {row}");
        }
        assert!(
            row.get("ships").map(|v| v.is_array()).unwrap_or(false),
            "contracts 表的 ships 必须是**数组**（一张单可以跑几条船）: {row}"
        );
        assert!(
            row.get("served_rounds").is_some() && row.get("ratio").is_some(),
            "contracts 表要有考核的分母与达标率（served_rounds / ratio）: {row}"
        );
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
    // 思潮 → 集货倾向：**「这个国家为什么少跑运输」必须可读**，而不是只能从行为反推。
    for col in ["freight_lean", "freighter_quota", "freighter_count"] {
        assert!(factions[0].get(col).is_some(), "factions 表缺 {col}（思潮→集货倾向）");
    }
    // 造舰的两条动机（解耦）：也要能从读面上看出「为什么造重舰 / 为什么造货船」。
    for col in ["threat_motive", "haul_gap"] {
        assert!(factions[0].get(col).is_some(), "factions 表缺 {col}（造舰动机）");
    }
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
    // ⚠ 蓝图表是**叶驱动**的：新开局零张图 ⇒ 表里零行。守卫要的是「声明了就必须写出来」，
    // 所以这里先给一个势力建一张图（否则这条断言会以「表没写出来」的假象失败）。
    let (cid, bid) = state
        .cities
        .iter()
        .filter(|c| c.faction_id == "中国")
        .find_map(|c| c.buildings.iter().find(|b| b.is_shipyard()).map(|b| (c.name.clone(), b.id)))
        .expect("中国要有一个建造区");
    state.control.entry("中国".to_string()).or_default().blueprints.insert(
        "守卫样本图".to_string(),
        crate::model::Control::player(crate::model::Blueprint {
            class: "corvette".to_string(),
            components: vec!["kinetic".to_string()],
            order: None,
        }),
    );
    if let Some(city) = state.city_mut(&cid) {
        for b in city.buildings.iter_mut() {
            if b.id == bid {
                b.ship_type = Some("corvette".to_string());
                b.blueprint = Some("守卫样本图".to_string());
            }
        }
    }
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
    for col in [
        "order_leaf_mode", "order_default_mode", "order_effective_mode", "order_effective",
        // 设计图那一轮新增的五列（缺一列 = 读面少一个答案）。
        "order_source", "blueprint", "blueprint_mode", "order_blueprint_mode", "spawned_round",
    ] {
        assert!(ships[0].get(col).is_some(), "ships 表缺 {col}");
    }
    // `cities` 表的内联 `buildings[]` 要能看出「哪个下标在造哪张图」。
    let cities = jsonl(&s.0.join("idx/cities.jsonl"));
    let has_bp_key = cities.iter().any(|c| {
        c["buildings"].as_array().is_some_and(|bs| bs.iter().any(|b| b.get("blueprint").is_some()))
    });
    assert!(has_bp_key, "cities.buildings[] 缺 blueprint 键");
}

/// **蓝图表与读面一致**（§7.4-15）：同一回合，`idx/blueprints.jsonl` 的行集必须与
/// `state.control[*].blueprints` 的键集逐条对应，`mode`/`components`/`class` 逐值相等
/// ——防「两张表各说各话」。
#[test]
fn blueprints_table_matches_the_control_face() {
    let cfg = load_config();
    let mut state = default_state(&cfg, 5);
    let (cid, bid) = state
        .cities
        .iter()
        .filter(|c| c.faction_id == "中国")
        .find_map(|c| c.buildings.iter().find(|b| b.is_shipyard()).map(|b| (c.name.clone(), b.id)))
        .expect("中国要有一个建造区");
    {
        let c = state.control.entry("中国".to_string()).or_default();
        c.blueprints.insert(
            "重甲护卫".to_string(),
            crate::model::Control::player(crate::model::Blueprint {
                class: "corvette".to_string(),
                components: vec!["kinetic".to_string(), "ion_drive".to_string()],
                order: Some(ShipBehavior::Dock { body: "地球".to_string() }),
            }),
        );
        c.blueprints.insert(
            "auto:cruiser".to_string(),
            crate::model::Control::auto(crate::model::Blueprint {
                class: "cruiser".to_string(),
                components: Vec::new(),
                order: None,
            }),
        );
    }
    if let Some(city) = state.city_mut(&cid) {
        for b in city.buildings.iter_mut() {
            if b.id == bid {
                b.ship_type = Some("corvette".to_string());
                b.blueprint = Some("重甲护卫".to_string());
            }
        }
    }
    // 造一艘出自「重甲护卫」的舰，让 `ship_count` 有非零值。
    let pos = state.body_position("地球");
    let bp = "重甲护卫".to_string();
    crate::sim::spawn_ship(&mut state, &cfg, crate::sim::ShipSpawn {
        owner: "中国".to_string(),
        class: "corvette",
        position: pos,
        city: None,
        via: SpawnVia::Shipyard,
        pay_components: false,
        blueprint: Some(&bp),
    });
    let mut rng = Prng::new(5);
    let s = Scratch::new("bp_match");
    write_index(&mut state, &cfg, &mut rng, 0, &s.0).unwrap();

    let rows = jsonl(&s.0.join("idx/blueprints.jsonl"));
    let mut seen: BTreeMap<(String, String), serde_json::Value> = BTreeMap::new();
    for r in &rows {
        seen.insert(
            (r["faction_id"].as_str().unwrap().to_string(), r["blueprint_id"].as_str().unwrap().to_string()),
            r.clone(),
        );
    }
    let mut expected: BTreeSet<(String, String)> = BTreeSet::new();
    for (fid, c) in &state.control {
        for (id, leaf) in &c.blueprints {
            expected.insert((fid.clone(), id.clone()));
            let row = seen
                .get(&(fid.clone(), id.clone()))
                .unwrap_or_else(|| panic!("蓝图表缺 {fid}/{id}"));
            assert_eq!(row["class"].as_str().unwrap(), leaf.value.class, "{fid}/{id} 的 class 不一致");
            assert_eq!(
                row["components"],
                json!(leaf.value.components),
                "{fid}/{id} 的 components 不一致"
            );
            assert_eq!(row["mode"].as_str().unwrap(), leaf.mode.name(), "{fid}/{id} 的 mode 不一致");
            assert_eq!(
                row["effective_mode"].as_str().unwrap(),
                state.blueprint_control(fid, id).name(),
                "{fid}/{id} 的 effective_mode 必须是引擎解析的答案"
            );
            if id == "重甲护卫" {
                assert_eq!(row["ship_count"], json!(1), "本图造了多少艘（引擎算）");
                assert_eq!(row["class_slots"], json!(2), "corvette 的槽位上限");
                assert_eq!(row["order"], json!({"Dock": {"body": "地球"}}), "默认枚举形式（与 control 表一致）");
                assert_eq!(row["launch_waiting"], json!(false), "没有满进度 ⇒ 不在等钱");
            }
            if id == "auto:cruiser" {
                assert!(row["order"].is_null(), "本图对意图没有说话 ⇒ null");
                assert_eq!(row["components"], json!([]));
            }
        }
    }
    assert_eq!(
        seen.keys().cloned().collect::<BTreeSet<_>>(),
        expected,
        "蓝图表与读面的键集必须逐条对应（两张表不许各说各话）"
    );
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

    let flow = jsonl(&s.0.join("idx/faction_process.jsonl"));
    let last_round = state.round;
    let last: Vec<&serde_json::Value> =
        flow.iter().filter(|r| r["round"] == json!(last_round)).collect();
    assert!(!last.is_empty(), "最后一回合应有过程量行");
    let mut checked = 0usize;
    let mut admin_seen = 0usize;
    for row in last {
        let fid = row["faction_id"].as_str().unwrap();
        let expect_upkeep = outcome.post.factions.get(fid).map(|r| r.upkeep).unwrap_or(0.0);
        assert_eq!(
            row["upkeep"].as_f64().unwrap(),
            expect_upkeep,
            "{fid} 的 upkeep 与视图不一致（读了两个不同的数）"
        );
        let expect_prod = outcome
            .post
            .factions
            .get(fid)
            .map(|r| r.production.clone())
            .unwrap_or_default();
        assert_eq!(row["production"], serde_json::to_value(&expect_prod).unwrap(), "{fid} 的 production 不一致");
        // B1：治理的拆分（行政 vs 娱乐）、人口超载倍率、思潮惩罚也必须与视图逐值一致。
        let expect_row = outcome.post.factions.get(fid);
        for (col, got, want) in [
            ("governance_admin", row["governance_admin"].as_f64().unwrap(),
             expect_row.map(|r| r.governance_admin).unwrap_or(0.0)),
            ("governance_entertainment", row["governance_entertainment"].as_f64().unwrap(),
             expect_row.map(|r| r.governance_entertainment).unwrap_or(0.0)),
            ("governance_scale", row["governance_scale"].as_f64().unwrap(),
             expect_row.map(|r| r.governance_scale).unwrap_or(1.0)),
            ("ideology_loyalty_penalty", row["ideology_loyalty_penalty"].as_f64().unwrap(),
             expect_row.map(|r| r.ideology_loyalty_penalty).unwrap_or(0.0)),
            ("capital_loyalty_bonus", row["capital_loyalty_bonus"].as_f64().unwrap(),
             expect_row.map(|r| r.capital_loyalty_bonus).unwrap_or(0.0)),
        ] {
            assert_eq!(got, want, "{fid} 的 {col} 与视图不一致（读了两个不同的数）");
        }
        if row["governance_admin"].as_f64().unwrap_or(0.0) > 0.0 {
            admin_seen += 1;
        }
        checked += 1;
    }
    assert!(checked >= 2, "只检查了 {checked} 个势力的过程量行——守卫太空");
    assert!(admin_seen >= 1, "没有任何势力报出行政开销——新的列等于空转");

    // 城的过程量表同理（挑一个真有产出的城，别拿空表当通过）。
    let city_flow = jsonl(&s.0.join("idx/city_process.jsonl"));
    let with_prod: Vec<&serde_json::Value> = city_flow
        .iter()
        .filter(|r| r["round"] == json!(last_round))
        .filter(|r| r["production"].as_object().map(|o| !o.is_empty()).unwrap_or(false))
        .collect();
    assert!(!with_prod.is_empty(), "最后一回合应有带产出的城（否则这条守卫没在检查任何东西）");
    let mut targets_seen = 0usize;
    for row in with_prod {
        let cid = row["city_id"].as_str().unwrap();
        let expect = outcome
            .post
            .cities
            .get(cid)
            .map(|r| r.production.clone())
            .unwrap_or_default();
        assert_eq!(row["production"], serde_json::to_value(&expect).unwrap(), "{cid} 的产出不一致");
        // B1：忠诚目标值分项（平铺列）与视图里的嵌套对象同源。
        let want_eff = outcome.post.cities.get(cid).map(|r| r.loyalty_target.effective).unwrap_or(0.0);
        assert_eq!(
            row["loyalty_target_effective"].as_f64().unwrap(),
            want_eff,
            "{cid} 的忠诚目标值不一致"
        );
        if want_eff > 0.0 {
            targets_seen += 1;
        }
    }
    assert!(targets_seen >= 1, "没有任何城报出忠诚目标值——新的列等于空转");
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
