//! agent 视图的单元测试。

use super::*;
use crate::config;

/// 风格是**活层**：agent 视图（`state_json`）必须给**有效值**，而不是 `Ship` 上那份
/// 出厂快照——否则 agent 读到 `kiting: 0.0`、以为还是基线，实际舰队默认早已把它推成
/// `-1.0`（又一次「读数不反映真实行为」）。
#[test]
fn state_view_reports_effective_doctrine_and_kiting() {
    let cfg = config::load_config();
    let mut state = crate::world::default_state(&cfg, 7);
    let (fid, name) = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .map(|s| (s.faction_id.clone(), s.name.clone()))
        .expect("中国 has a starting ship");
    let record_kiting = state.ship(&name).unwrap().kiting;
    let record_doctrine = state.ship(&name).unwrap().doctrine;

    let diff = serde_json::json!({
        "control": [{"faction_id": fid,
            "default_kiting": {"kiting": -1.0, "mode": "Player"},
            // `default_doctrine` 是**两轴一片叶**：这片叶还不存在时必须两条轴一起给
            // （只给一条 ⇒ `partial_doctrine_leaf` 拒绝，见 control.rs）。
            "default_doctrine": {"temper": 0.5, "lone_wolf": 0.0, "mode": "Player"}}]
    });
    crate::control::apply_patch(&mut state, &cfg, &diff).expect("默认风格 applies");

    let v = state_json(&state, &crate::sim::derived_from_state(&state, &cfg));
    let row = v["ships"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == serde_json::json!(name))
        .expect("该舰在视图里");
    assert_eq!(row["kiting"], serde_json::json!(-1.0), "视图必须给有效姿态（舰队默认）");
    assert_eq!(row["doctrine"]["temper"], serde_json::json!(0.5), "视图必须给有效风格");
    // 记录值不动（它仍是出厂快照）——这正是"视图不能直接序列化 Ship"的原因。
    assert_eq!(state.ship(&name).unwrap().kiting, record_kiting);
    assert_eq!(state.ship(&name).unwrap().doctrine, record_doctrine);
}

/// `meta_value` 是让 agent 读的「规则字典」。它必须从 config 结构体**派生**，
/// 而不是手写字段清单——此守卫确保新增的 config 字段（尤其是 `combat` 那些）
/// 一定出现在 meta 里，防止再次像 `component_spill` 那样「配置有、meta 无」。
#[test]
fn meta_derives_all_combat_fields_and_keeps_ints() {
    let cfg = config::load_config();
    let m = meta_value(&cfg);
    let combat = m.get("combat").and_then(|v| v.as_object()).expect("combat section");
    for f in [
        "component_spill",
        "component_repair",
        "escort_range",
        "pursuit_range",
        "pd_radius",
    ] {
        assert!(combat.contains_key(f), "meta.combat 缺少 {f}（曾被手写清单漏掉）");
    }
    // 整数型配置字段必须保持整数，不能因圆整变成 `2.0`。
    let slots = &m["ships"]["corvette"]["slots"];
    assert!(slots.is_i64() || slots.is_u64(), "slots 应为整数，实为 {slots}");
}

/// 规则字典 `meta` 必须**覆盖每一个配置段**（它从 config 派生，不手写）。任何 config
/// 字段都必须出现在对应的 meta 段里——这直接拦住「配置加了字段、meta 忘了加」这类漂移
/// （正是当初 `combat.component_spill` 消失的那种 bug）。
#[test]
fn meta_value_covers_every_config_section() {
    fn key_set(v: &serde_json::Value) -> std::collections::BTreeSet<String> {
        v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
    }
    fn assert_covers(m: &serde_json::Value, key: &str, cfgv: &serde_json::Value) {
        let meta_sec = m.get(key).unwrap_or_else(|| panic!("meta 缺 {key} 段"));
        let cfg_keys = key_set(cfgv);
        let meta_keys = key_set(meta_sec);
        let missing: Vec<_> = cfg_keys.difference(&meta_keys).cloned().collect();
        assert!(missing.is_empty(), "meta.{key} 遗漏 config 字段: {missing:?}");
    }
    let cfg = config::load_config();
    let m = meta_value(&cfg);
    // 结构体段：meta 用 config_json 全量派生，key 集合必须覆盖 config 的集合。
    assert_covers(&m, "economy", &serde_json::to_value(&cfg.economy).unwrap());
    assert_covers(&m, "combat", &serde_json::to_value(&cfg.combat).unwrap());
    assert_covers(&m, "diplomacy", &serde_json::to_value(&cfg.diplomacy).unwrap());
    assert_covers(&m, "market", &serde_json::to_value(&cfg.market).unwrap());
    assert_covers(&m, "freight", &serde_json::to_value(&cfg.freight).unwrap());
    assert_covers(&m, "governance", &serde_json::to_value(&cfg.governance).unwrap());
    assert_covers(&m, "mond", &serde_json::to_value(&cfg.mond).unwrap());
    assert_covers(&m, "balance", &serde_json::to_value(&cfg.balance).unwrap());
    // 字符串键表段：meta 的段 key == config 表的 key。
    assert_covers(&m, "structures", &serde_json::to_value(&cfg.structures).unwrap());
    assert_covers(&m, "buildings", &serde_json::to_value(&cfg.buildings).unwrap());
    assert_covers(&m, "ships", &serde_json::to_value(&cfg.ships).unwrap());
    assert_covers(&m, "components", &serde_json::to_value(&cfg.components).unwrap());
}

/// agent 视图（`state_json` 发射的 JSON）必须被 `schema_value()` **自描述**：发射的
/// 顶层 key 都出现在 schema 的 `properties` 里，且实体数组被声明。因为 schema 与 JSON
/// 都由同一批权威类型（`Trajectory` + 模型类型）派生，这个守卫把「schema↔发射不一致」
/// 显式化，防止未来有人用另一个生产者替换掉 `state_json`。
#[test]
fn agent_view_is_self_described_by_schema() {
    let cfg = config::load_config();
    let state = crate::world::default_state(&cfg, 42);
    let v = state_json(&state, &crate::sim::derived_from_state(&state, &cfg));
    let schema = schema_value();
    let props = schema.get("properties").and_then(|p| p.as_object()).expect("schema.properties");
    let emit_keys = v
        .as_object()
        .map(|o| o.keys().cloned().collect::<std::collections::BTreeSet<_>>())
        .unwrap_or_default();
    let schema_keys = props.keys().cloned().collect::<std::collections::BTreeSet<_>>();
    let missing: Vec<_> = emit_keys.difference(&schema_keys).cloned().collect();
    assert!(missing.is_empty(), "agent 视图发射了 schema 未描述的字段: {missing:?}");
    for k in ["bodies", "cities", "factions", "ships", "events", "chronicle", "metrics"] {
        assert!(props.contains_key(k), "schema 缺字段 {k}");
    }
    // 总结指标 `metrics` 必须真的出现在发射的 JSON 里（不是空壳），且是实体视图的一部分。
    assert!(
        v.get("metrics").is_some_and(|m| m.get("factions").is_some()),
        "agent 视图必须携带 metrics 总结（含各势力聚合）"
    );
    // metrics 必须携带新增的**流量**字段（产出/维护/治理 + 每城产出）。
    let m = &v["metrics"];
    let sample_fac = m
        .get("factions")
        .and_then(|f| f.as_object())
        .and_then(|o| o.values().next())
        .cloned()
        .unwrap_or_default();
    for k in ["production_value", "production", "upkeep", "governance_cost", "governance_coverage"] {
        assert!(sample_fac.get(k).is_some(), "metrics.factions 缺流量字段 {k}");
    }
    assert!(m.get("city_production").is_some(), "metrics 缺每城产出 city_production");
}
