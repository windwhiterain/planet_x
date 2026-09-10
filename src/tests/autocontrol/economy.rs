//! 经济决策的单元测试。

use super::*;
use crate::config::load_config;
use crate::world::default_state;

/// Build the config + a fresh deterministic world (round 0).
fn fresh_world(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// Guard: the cost→benefit preview reports the current economy balance
/// (`net = production − upkeep − governance`) and, when the agent commands an
/// over-committed construction budget, flags it *before* it collapses — the
/// reported "建造预算拉满 → 维护 > 产出 → 城清零" footgun.
#[test]
fn control_plan_balances_and_flags_over_committed_construction() {
    let (config, mut state) = fresh_world(42);

    // Baseline: 中国 self-sustaining at the start.
    let plan = control_plan(&state, &config, "中国").expect("faction exists");
    let p = plan["production_value"].as_f64().unwrap();
    let u = plan["upkeep"].as_f64().unwrap();
    let g = plan["governance_cost"].as_f64().unwrap();
    let net = plan["net_flow"].as_f64().unwrap();
    assert!(
        (net - (p - u - g)).abs() < 0.05,
        "net must ≈ production − upkeep − governance"
    );
    assert_eq!(plan["verdict"].as_str().unwrap(), "healthy");
    assert!(!plan["over_committed_construction"].as_bool().unwrap());
    assert!(plan["rounds_before_insolvent"].is_null());

    // Command a huge construction budget on a held resource with mode=Player.
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "construction_budget": [
            {"resource": "铁", "value": 10000.0, "mode": "Player"},
            {"resource": "碳", "value": 10000.0, "mode": "Player"}
        ]}]
    });
    crate::control::apply_patch(&mut state, &config, &diff)
        .expect("apply construction over-commit");

    let plan2 = control_plan(&state, &config, "中国").expect("faction exists");
    assert!(
        plan2["construction_budget_value"].as_f64().unwrap() > 0.0,
        "commanded construction budget must be non-zero"
    );
    assert!(
        plan2["over_committed_construction"].as_bool().unwrap(),
        "over-committed construction must be flagged"
    );
    assert_ne!(plan2["verdict"].as_str().unwrap(), "healthy");
}
