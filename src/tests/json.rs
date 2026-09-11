//! json 工具的单元测试。

use super::*;
use crate::config::load_config;
use crate::model::*;
use std::collections::BTreeMap;

/// 元组键（`(城市, 建筑id)`）必须能 dump——这正是 serde_json 直接做会报
/// `key must be a string` 的地方，也是本模块存在的理由。
#[test]
fn tuple_keys_become_strings() {
    let mut state = crate::world::default_state(&load_config(), 42);
    let fid = "中国".to_string();
    let city = state
        .cities
        .iter()
        .find(|c| c.faction_id == fid)
        .expect("a city")
        .name
        .clone();
    let ctrl = state.control.entry(fid.clone()).or_default();
    ctrl.invest_weights
        .insert((city.clone(), 7), Control::player(1.5));
    ctrl.build_weights
        .insert((city.clone(), 7), Control::auto(2.5));

    let v = to_value(&state).expect("a tuple-keyed state must dump to JSON");
    let inv = &v["control"][fid.as_str()]["建设权重"];
    let key = format!("{city}|7");
    assert_eq!(
        inv[&key]["值"],
        serde_json::json!(1.5),
        "tuple key must be `城市|建筑id`"
    );
    assert_eq!(inv[&key]["归属"], serde_json::json!("Player"));
    assert!(
        inv.get("value").is_none(),
        "the tuple key must not collapse into the map"
    );

    // 2026-10：`json::key2` 适配器上线后，**原生** serde_json 也能编这两张元组键的表了
    // （键写成 `名|序号`）——这条断言因此从 `is_err` 翻成 `is_ok`。`to_value` 剩下的价值是
    // 「**任何**古怪键类型（枚举/嵌套元组/整数）都一律字符串化」的通用兜底。
    assert!(
        serde_json::to_value(&state).is_ok(),
        "key2 适配器应当让元组键也过得了原生 serde_json"
    );

    // 更要紧的是**读得回来**：`--save x.json` → `--start x.json` 那条往返（Python 直接改档
    // 就靠它）。适配器写 `名|序号`、读时按 `|` 拆回元组，两边必须对称。
    // 比的是**整份再序列化出来的文本**（比逐字段断言更强，而且不用给 `Control<T>` 加 `PartialEq`；
    // `BTreeMap` 有序 ⇒ 文本确定）。
    let text = serde_json::to_string(&state).expect("state → JSON");
    let back: crate::model::State = serde_json::from_str(&text).expect("JSON → state");
    assert_eq!(
        serde_json::to_string(&back).expect("再编一次"),
        text,
        "JSON 往返回来的状态必须逐字节相同（元组键写 `名|序号`、读按 `|` 拆）"
    );
}

/// 整份模型（State / GameConfig / Derived）都必须可以**无手工投影**地 dump：
/// 任何字段、任何嵌套都在，且键是字符串。这是前端 generic widget 的数据前提。
#[test]
fn whole_models_dump_with_every_field() {
    let config = load_config();
    let state = crate::world::default_state(&config, 42);
    let derived = crate::sim::view_from_state(&state, &config);

    let s = to_value(&state).unwrap();
    for k in [
        "round",
        "time_month",
        "bodies",
        "cities",
        "factions",
        "ships",
        "control",
        "scope",
        "events",
        "chronicle",
        "ship_name_seq",
        "schema_version",
    ] {
        assert!(s.get(k).is_some(), "State field `{k}` must be in the dump");
    }
    let c = to_value(&config).unwrap();
    for k in [
        "economy",
        "name_pool",
        "story",
        "ships",
        "buildings",
        "body_kinds",
    ] {
        assert!(
            c.get(k).is_some(),
            "GameConfig section `{k}` must be in the dump"
        );
    }
    // 视图是**一个对象**（观测 + 本回合过程量同处其中），不再分 `flow` / `metrics` 两段。
    let d = to_value(&derived).unwrap();
    for k in ["factions", "cities", "power_share", "decisions"] {
        assert!(
            d.get(k).is_some(),
            "RoundView field `{k}` must be in the dump"
        );
    }
}

/// 标量/元组/枚举键与「键字符串化」的通用规则。
#[test]
fn key_rules_are_general() {
    let mut ints: BTreeMap<u32, &str> = BTreeMap::new();
    ints.insert(3, "x");
    assert_eq!(to_value(&ints).unwrap(), serde_json::json!({"3": "x"}));

    #[derive(Serialize, PartialEq, Eq, PartialOrd, Ord)]
    enum E {
        A,
        B(u8),
    }
    let mut enums: BTreeMap<E, u8> = BTreeMap::new();
    enums.insert(E::A, 1);
    enums.insert(E::B(3), 2);
    let v = to_value(&enums).unwrap();
    assert_eq!(v["A"], serde_json::json!(1));
    assert_eq!(
        v["B:3"],
        serde_json::json!(2),
        "newtype-variant keys keep their payload"
    );

    let mut nested: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    nested.insert("a", vec![1.0, f64::NAN]);
    assert_eq!(
        to_value(&nested).unwrap(),
        serde_json::json!({"a": [1.0, null]}),
        "NaN has no JSON form"
    );
}
