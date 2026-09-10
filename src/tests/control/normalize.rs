//! 行为叶的规范化：tagged 形式被接受并改写、非法 tag 报出合法清单、默认形式原样保留、`apply_patch` 走同一条规范化。

use super::*;

/// The tagged agent-state `order` form must be accepted and rewritten into
/// the default enum form that the rest of the pipeline expects.
#[test]
fn normalize_behavior_accepts_tagged_form() {
    let cases = [
        (serde_json::json!({"type":"idle"}), serde_json::json!("Idle")),
        (
            serde_json::json!({"type":"follow","ship":"华盛顿"}),
            serde_json::json!({"Follow":{"ship":"华盛顿"}}),
        ),
        (
            serde_json::json!({"type":"dock_city","city":"长三角"}),
            serde_json::json!({"DockCity":{"city":"长三角"}}),
        ),
        (
            serde_json::json!({"type":"move","position":[-0.5,0.3]}),
            serde_json::json!({"Move":{"position":[-0.5,0.3]}}),
        ),
        (
            serde_json::json!({"type":"colonize","body":"地球"}),
            serde_json::json!({"Colonize":{"body":"地球"}}),
        ),
    ];
    for (tagged, expected) in cases {
        let mut v = tagged.clone();
        normalize_behavior(&mut v, "test").expect("legal tag normalizes");
        assert_eq!(v, expected, "tagged input {tagged:?} must normalize to {expected:?}");
    }
}

/// An unknown tag must be **rejected with a self-correcting message**: the
/// removed `target_ship` behavior is the single most likely thing an agent
/// writes (it is what the old manual taught), and the old code let it fall
/// through to serde's `invalid value: map, expected map with a single key`
/// — which names neither the field nor the alternatives.
#[test]
fn unknown_behavior_tag_is_rejected_with_the_legal_tags_and_a_hint() {
    let mut v = serde_json::json!({"type":"target_ship","ship":"华盛顿","attack":true});
    let err = normalize_behavior(&mut v, "中国.ship_orders[0]").expect_err("removed behavior must be rejected");
    for needle in ["target_ship", "合法", "follow", "自动开火"] {
        assert!(err.contains(needle), "error must mention {needle:?}, got: {err}");
    }
    // 未知但也不像旧行为的标签同样被拒（不再静默流过）。
    let mut v = serde_json::json!({"type":"teleport"});
    assert!(normalize_behavior(&mut v, "x").is_err());
}

/// The default form must pass through unchanged.
#[test]
fn normalize_behavior_keeps_default_form() {
    let mut v = serde_json::json!({"Follow":{"ship":"华盛顿"}});
    normalize_behavior(&mut v, "test").expect("default form is untouched");
    assert_eq!(v, serde_json::json!({"Follow":{"ship":"华盛顿"}}));
}

/// Applying a tagged-form diff to a real world must produce the same
/// controllable behavior as the equivalent default-form diff.
#[test]
fn apply_patch_accepts_tagged_ship_order() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    // 长城 = 中国 (faction 3) 的起始护卫舰；华盛顿 = 美国的一艘舰。舰名即唯一 key。
    let tagged = serde_json::json!({
        "control": [{
            "faction_id": "中国",
            "ship_orders": [{"ship": "长城", "behavior": {"type": "follow", "ship": "华盛顿"}, "mode": "Player"}]
        }]
    });
    apply_patch(&mut state, &config, &tagged).expect("tagged diff applies");
    let b = state.ship_behavior("长城".to_string()).expect("长城 has an order");
    assert_eq!(b, ShipBehavior::Follow { ship: "华盛顿".to_string() });
}
