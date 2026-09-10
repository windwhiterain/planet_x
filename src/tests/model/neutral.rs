//! 中性值（读面缺省值）的守卫：**声明一处、用一处、发一处**三边必须咬合。
//!
//! 五条测试各钉一边：
//! 1. `every_read_face_field_declares_a_neutral`——用 schemars 遍历读面结构，**新字段必须声明**
//!    （两边集合相等，且中性值的类型与 schema 类型相容）；
//! 2. `struct_defaults_equal_the_declared_neutrals`——Rust 的 `Default` 就是声明的中性值
//!    （serde 缺字段走的就是它，两者不一致 = 「同一份存储两个读者两个值」）；
//! 3. `pre_face_process_fields_equal_their_declared_neutral`——**引擎的实证**：一个真实世界里
//!    「这一步还没跑」的字段，吐出来的值必须逐字段等于声明；
//! 4. `value_consts_match_the_table`——引擎用的具名常量与表同值（常量给人读，表给 schema 读）；
//! 5. `schema_publishes_the_neutral_table`——发出去的 `schema.json` 段等于本表。

use super::*;
use crate::config::load_config;
use crate::model::{LoyaltyTarget, RoundView};
use crate::world::default_state;
use serde_json::Value;

// ── 工具 ──────────────────────────────────────────────────────────────────

/// 按 `cities[].loyalty_target.distance` 这种路径把**所有**匹配到的值收进来
/// （map 段 `[]` 会扇出到每一个值——断言要对每一个值都成立）。
fn collect<'a>(v: &'a Value, path: &str, out: &mut Vec<&'a Value>) {
    let (head, rest) = match path.split_once('.') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    let (name, is_map) = match head.strip_suffix("[]") {
        Some(n) => (n, true),
        None => (head, false),
    };
    let Some(node) = v.get(name) else { return };
    if is_map {
        if let Some(map) = node.as_object() {
            for val in map.values() {
                match rest {
                    Some(r) => collect(val, r, out),
                    None => out.push(val),
                }
            }
        }
    } else {
        match rest {
            Some(r) => collect(node, r, out),
            None => out.push(node),
        }
    }
}

/// 断言路径上的**每一个**值都等于声明的中性值（且至少有一个值——防「路径写错所以没检查到」）。
///
/// `dig_path` 是在这份 JSON 里下钻的路径，`full_path` 是它在 [`READ_FACE_NEUTRALS`] 里的键：
/// 两者只在「拿嵌套结构的 Default 单独检查」时不同（那时下钻用剥了前缀的相对路径）。
fn assert_path_is_neutral(json: &Value, dig_path: &str, full_path: &str, where_: &str) {
    let neutral = neutral_for(full_path).unwrap_or_else(|| panic!("{full_path} 没在表里声明"));
    let mut found = Vec::new();
    collect(json, dig_path, &mut found);
    assert!(
        !found.is_empty(),
        "{where_}: 路径 {dig_path}（= {full_path}）一个值都没取到（路径写错？字段没了？）"
    );
    for v in found {
        assert_eq!(
            *v,
            neutral.to_json(),
            "{where_}: {full_path} 的实测值 {v} ≠ 声明的中性值 {}",
            neutral.to_json()
        );
    }
}

/// 把 schemars 的读面 schema 摊成「叶子路径 → 该字段允许的 JSON 类型」。
fn read_face_leaves() -> Vec<(String, Vec<String>)> {
    let schema = serde_json::to_value(schemars::schema_for!(RoundView)).unwrap();
    let defs = schema.get("definitions").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();
    walk(&schema, &defs, "", &mut out);
    out
}

/// 顺着 `$ref` 走到真正的定义节点。
///
/// ⚠ schemars 0.8 在「字段既有 `$ref` 又有文档注释」时会包一层 `allOf`：
/// `{"allOf":[{"$ref":"#/definitions/CapitalFlow"}],"description":"…"}`——不认这层就会把
/// 嵌套结构当成叶子（实测踩过：`decisions` / `capital` / `loyalty_target` 三个都没展开）。
fn resolve<'a>(node: &'a Value, defs: &'a Value) -> &'a Value {
    let mut cur = node;
    let mut hops = 0;
    loop {
        hops += 1;
        assert!(hops < 10, "schema 里的 $ref 成环了");
        if let Some(r) = cur.get("$ref").and_then(|r| r.as_str()) {
            cur = &defs[r.rsplit('/').next().unwrap()];
            continue;
        }
        if let Some(inner) = cur
            .get("allOf")
            .and_then(|a| a.as_array())
            .filter(|a| a.len() == 1)
            .and_then(|a| a[0].get("$ref"))
            .and_then(|r| r.as_str())
        {
            cur = &defs[inner.rsplit('/').next().unwrap()];
            continue;
        }
        break;
    }
    cur
}

fn types_of(node: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(t) = node.get("type") {
        match t {
            Value::String(s) => out.push(s.clone()),
            Value::Array(a) => out.extend(a.iter().filter_map(|v| v.as_str().map(String::from))),
            _ => {}
        }
    }
    // `Option<T>` 在 schemars 里可能是 `anyOf`/`oneOf`（一边 `{"type":"null"}`）。
    for key in ["anyOf", "oneOf"] {
        if let Some(branches) = node.get(key).and_then(|b| b.as_array()) {
            for b in branches {
                out.extend(types_of(b));
            }
        }
    }
    out
}

/// 展开读面 schema：对象逐字段下钻（map 的值用 `[]` 进一层），数组/标量作为叶子。
fn walk(node: &Value, defs: &Value, prefix: &str, out: &mut Vec<(String, Vec<String>)>) {
    let node = resolve(node, defs);
    let types = types_of(node);

    // map（`additionalProperties` 是子 schema）：**容器自己**是一个叶子（空 map 是合法状态，
    // 中性值 `{}`），值是结构时再进一层把值的每个字段摊成叶子。
    if let Some(ap) = node.get("additionalProperties").filter(|a| a.is_object()) {
        let inner = resolve(ap, defs);
        out.push((prefix.to_string(), types));
        if inner.get("properties").is_some() {
            walk(ap, defs, &format!("{prefix}[]"), out);
        }
        return;
    }
    if types.iter().any(|t| t == "object") {
        if let Some(props) = node.get("properties").and_then(|p| p.as_object()) {
            for (name, sub) in props {
                let path = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                walk(sub, defs, &path, out);
            }
            return;
        }
    }
    out.push((prefix.to_string(), types));
}

// ── 1. 覆盖：每个叶子字段都有声明，且类型相容 ──────────────────────────────

#[test]
fn every_read_face_field_declares_a_neutral() {
    let leaves = read_face_leaves();
    let walked_list: Vec<&str> = leaves.iter().map(|(p, _)| p.as_str()).collect();
    assert!(
        leaves.len() >= 45,
        "读面叶子只摊出 {} 个——走 schema 的逻辑可能坏了（摊出来的是 {walked_list:?}）",
        leaves.len()
    );

    let declared: std::collections::BTreeSet<&str> =
        READ_FACE_NEUTRALS.iter().map(|(p, _)| *p).collect();
    let walked: std::collections::BTreeSet<&str> = leaves.iter().map(|(p, _)| p.as_str()).collect();

    let undeclared: Vec<&&str> = walked.difference(&declared).collect();
    assert!(
        undeclared.is_empty(),
        "读面字段没有声明中性值（加字段就要在 `model::neutral::READ_FACE_NEUTRALS` 加一行）：{undeclared:?}"
    );
    let stale: Vec<&&str> = declared.difference(&walked).collect();
    assert!(
        stale.is_empty(),
        "表里这些路径在读面结构里不存在了（字段改名/删了？）：{stale:?}"
    );

    // 类型相容：中性值的种类**必须**是这个字段声明的类型之一——整数档对 `integer`、
    // 浮点档对 `number`、`null`/`object`/`array`/`boolean` 各自对号（这一条防的是
    // 「给整数列补了 0.0」那种把 dtype 带偏的替补值）。
    for (path, types) in &leaves {
        let neutral = neutral_for(path).unwrap();
        let kind = neutral.kind();
        assert!(
            types.iter().any(|t| t == kind),
            "{path} 声明为 {kind}（{neutral:?}），但 schema 说它是 {types:?}"
        );
    }
    // 两个非零中性值必须真的是 One/Null，别被顺手改成 Zero（这是历史坑的正中央）。
    assert_eq!(
        neutral_for("factions[].governance_scale"),
        Some(Neutral::One)
    );
    assert_eq!(
        neutral_for("factions[].governance_coverage"),
        Some(Neutral::One)
    );
    assert_eq!(
        neutral_for("factions[].capital_loyalty_bonus"),
        Some(Neutral::Zero)
    );
    // B2：用工系数的中性值同样是 **1.0**（不缺人手），不是 0——「全城没人上工」是另一回事。
    assert_eq!(neutral_for("cities[].labor"), Some(Neutral::One));
    // 集散地的中性值是 false（这个月的入库路径还没定），不是「它不是首都」。
    assert_eq!(neutral_for("cities[].is_hub"), Some(Neutral::False));
    // 稀疏数组：整条存在或整条缺席，中性值是空数组（条目内部不逐字段声明）。
    assert_eq!(neutral_for("decisions.capital"), Some(Neutral::EmptyArray));
}

// ── 2. Rust 的 Default 就是声明的中性值（serde 缺字段走它）──────────────

#[test]
fn struct_defaults_equal_the_declared_neutrals() {
    // 整份视图的顶层标量 / 空集合：`RoundView::default()` 必须逐字段等于声明。
    let view = serde_json::to_value(RoundView::default()).unwrap();
    for (path, _) in READ_FACE_NEUTRALS {
        // 空 map 里取不到 `factions[]...`，那些路径由下面两个嵌套结构的 Default 覆盖。
        if path.starts_with("factions[].") || path.starts_with("cities[].") {
            continue;
        }
        assert_path_is_neutral(&view, path, path, "RoundView::default()");
    }

    // 嵌套结构（B1 的忠诚目标值）：它的 Default 就是「没跑这一步」的样子。
    let light = serde_json::to_value(LoyaltyTarget::default()).unwrap();
    for (path, _) in READ_FACE_NEUTRALS {
        if let Some(rest) = path.strip_prefix("cities[].loyalty_target.") {
            assert_path_is_neutral(&light, rest, path, "LoyaltyTarget::default()");
        }
    }

    // B2 的每舰级造舰行（map 的**值**结构）：缺一个键时读到的是「本城没这个舰级的建造区」，
    // 而一旦有键，两个叶子各自按声明填（`Default` = 两项都是 0）。
    let line = serde_json::to_value(crate::model::BuildLine::default()).unwrap();
    for (path, _) in READ_FACE_NEUTRALS {
        if let Some(rest) = path.strip_prefix("cities[].build[].") {
            assert_path_is_neutral(&line, rest, path, "BuildLine::default()");
        }
    }
}

// ── 3. 引擎实证：`pre` 面（这一步还没跑）吐出来的就是声明 ─────────────────

/// 「这一步还没跑 / 这件事没发生」时必须是中性值的那些路径——**过程量**。
/// 观测量（人口/忠诚/世界总量/价格…）不在这个列表里：它们的声明是「万一缺键该怎么读」的
/// **定义**，不是「一个空世界的观测值」（见 `neutral` 模块文档里那段区分）。
const PROCESS_PATHS: &[&str] = &[
    "market_settled",
    "market_offered",
    "decisions.ships",
    "decisions.retools",
    "decisions.styles",
    "decisions.blueprints",
    "factions[].production",
    "factions[].production_value",
    "factions[].upkeep",
    "factions[].governance_cost",
    "factions[].governance_coverage",
    "factions[].governance_admin",
    "factions[].governance_entertainment",
    "factions[].governance_scale",
    "factions[].ideology_loyalty_penalty",
    "factions[].capital_loyalty_bonus",
    "decisions.capital",
    "factions[].freight_paid",
    "factions[].carrier_income",
    "factions[].net_import",
    "cities[].production",
    "cities[].production_value",
    "cities[].loyalty_target.distance",
    "cities[].loyalty_target.entertainment",
    "cities[].loyalty_target.effective",
    // B2（钱去哪了）：这八个都是「这一步还没跑」⇒ 中性值。⚠ 用工系数与集散地**特别容易写错**：
    // 前者的中性值是 1.0（不缺人手，不是「没人上工」），后者的中性值是 `false`（这个月的入库
    // 路径还没定，不是「它不是首都」——要后者请拿 `control` 的 `capital` 叶比 `body_id`）。
    "factions[].investment_spent",
    "factions[].construction_spent",
    "factions[].upkeep_unpaid",
    "factions[].fleet_rust",
    "cities[].labor",
    "cities[].housing_capacity",
    "cities[].is_hub",
    "cities[].build",
    // B3（市场与运输）：本回合的结算事实 + 市场里的位置 + 集货运力账。
    // ⚠ `market_rank` 的中性值是 **`null`**（还没排队），不是 0（那是「第一个挑」）；
    // `freight_gap` 只把**容器**列进来（`pre` 里是 `{}`，条目内部一个值都取不到——列叶子会红）。
    "market_trades",
    "haul_steps",
    "factions[].purchasing_power",
    "factions[].market_rank",
    "factions[].freight_gap",
];

#[test]
fn pre_face_process_fields_equal_their_declared_neutral() {
    let config = load_config();
    let state = default_state(&config, 42);
    // `view_from_state` = 喂一个空 sink 的观测 ⇒ 正是「`pre` 面」的形状（`--start` 载入 / 回合 0）。
    let view = serde_json::to_value(crate::sim::view_from_state(&state, &config)).unwrap();

    // 下限只是防空转（原本 ~32 条；`capital` 判定搬进稀疏数组、两个全国项上移势力行后少了几条）。
    assert!(
        PROCESS_PATHS.len() >= 20,
        "过程量清单短了——守卫会退化成空转"
    );
    for path in PROCESS_PATHS {
        assert!(
            neutral_for(path).is_some(),
            "{path} 在过程量清单里，却没在 READ_FACE_NEUTRALS 里声明"
        );
        assert_path_is_neutral(&view, path, path, "引擎的 pre 面");
    }
}

// ── 4. 具名常量与表同值（常量给人读，表给 schema 读）──────────────────────

#[test]
fn value_consts_match_the_table() {
    assert_eq!(
        neutral_for("factions[].governance_coverage"),
        Some(Neutral::One),
        "覆盖率常量与表不一致"
    );
    assert_eq!(
        neutral_for("factions[].governance_scale"),
        Some(Neutral::One),
        "超载倍率常量与表不一致"
    );
    assert_eq!(value::GOVERNANCE_COVERAGE, 1.0);
    assert_eq!(value::GOVERNANCE_SCALE, 1.0);
    assert_eq!(
        neutral_for("cities[].labor"),
        Some(Neutral::One),
        "用工系数常量与表不一致"
    );
    assert_eq!(value::CITY_LABOR, 1.0);
    assert_eq!(
        Neutral::One.to_json(),
        serde_json::json!(value::GOVERNANCE_SCALE),
        "表里的「1」与引擎常量必须是同一个值"
    );
}

// ── 5. 发出去的 schema 段等于本表 ─────────────────────────────────────────

#[test]
fn schema_publishes_the_neutral_table() {
    let schema = crate::projection::projection_schema();
    let section = schema
        .get("neutral")
        .expect("schema.json 必须有 neutral 段");
    assert_eq!(section["root"], serde_json::json!("view"));
    let desc = section["description"].as_str().expect("neutral 段要有说明");
    assert!(
        desc.contains("不要自己编缺省"),
        "说明里必须点明「缺键按这里补、别自己编」——那正是历史坑的成因"
    );
    let published = section["fields"]
        .as_object()
        .expect("neutral.fields 是对象");
    assert_eq!(
        published.len(),
        READ_FACE_NEUTRALS.len(),
        "发出的字段数与声明数不一致（发布路径漂了）"
    );
    for (path, neutral) in READ_FACE_NEUTRALS {
        assert_eq!(
            published.get(*path),
            Some(&neutral.to_json()),
            "schema.json 里 {path} 的中性值与声明不一致"
        );
    }
}
