//! 首都：亡城强迁到人口最高活城、AI 周期性评估迁都、玩家钉的首都 AI 不许覆盖。

use super::*;

/// 迁都-亡城强迁：首都天体上已无本势力活城 → 自动切到**人口最高的活城**。
#[test]
fn capital_destroyed_auto_relocates_to_highest_population_city() {
    let (config, mut state) = fresh_world(42);
    // 中国初始首都=地球，其上活城 长三角(1400)/珠三角(1100)。把这两城夷平 → 首都亡。
    assert_eq!(state.capital_body("中国"), "地球");
    for cid in ["长三角".to_string(), "珠三角".to_string()] {
        if let Some(c) = state.city_mut(&cid) {
            c.razed = true;
        }
    }

    step_capital(&mut state, &config);

    // 剩余中国活城：水星熔炉基地(220,水星)、金星浮空之城(260,金星)。人口最高=金星浮空之城。
    assert_eq!(
        state.capital_body("中国"),
        "金星",
        "capital must snap to the highest-population remaining city (金星)"
    );
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::CapitalRelocated { faction, to, reason, .. } if faction == "中国" && to == "金星" && reason == "destroyed"
        )),
        "a destroyed-capital relocation event must be recorded, got {:?}",
        state.events
    );
}

/// 迁都-周期 AI 评估：首都 Population 中心更优（总治理距离成本显著更低）时，AI 迁过去。
#[test]
fn ai_periodic_review_relocates_capital_to_population_center() {
    let (mut config, mut state) = fresh_world(42);
    // 收窄治理可达半径 + 降低迁都门槛，让内行星间的距离差能体现「更优」。
    config.governance.admin_range = 0.05;
    config.governance.capital_relocate_threshold = 0.1;
    assert_eq!(config.governance.capital_review_every, 12);

    // 交圈数设为评估周期（12）：非 Player 首都在评估轮迁到人口中心。
    state.round = 12;
    // 把中国首都先钉到 水星（较远），**显式写 `mode: Auto`** 让 AI 继续评估：
    // 「写值即接管」之后，只写 value 会被当成玩家的首都（mode=Player），
    // 那样这条测试考的就不再是 AI 评估了。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "capital": {"value": "水星", "mode": "Auto"}}]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("set far capital");
    assert_eq!(state.capital_body("中国"), "水星");

    step_capital(&mut state, &config);

    // 中国人口最繁华城=长三角(1400,地球)；迁到地球显著降低总治理距离成本。
    assert_eq!(
        state.capital_body("中国"),
        "地球",
        "AI review should relocate the capital to the population center (地球)"
    );
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::CapitalRelocated { faction, reason, .. } if faction == "中国" && reason == "ai_review"
        )),
        "an AI-review relocation event must be recorded, got {:?}",
        state.events
    );
}

/// 迁都-Player 标记：mode=Player 的首都在评估轮不被 AI 覆盖（除非亡城硬规则）。
#[test]
fn player_capital_not_overridden_by_ai_review() {
    let (config, mut state) = fresh_world(42);
    // 玩家把首都迁到 水星 并标 Player；中国在 水星 仍有活城（水星熔炉基地），非亡城。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "capital": {"value": "水星", "mode": "Player"}}]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("player move capital");
    assert_eq!(state.capital_body("中国"), "水星");
    assert_eq!(state.capital_control("中国"), ControlMode::Player);

    // 评估轮：Player 控制的首都不被周期迁移覆盖。
    state.round = 12;
    step_capital(&mut state, &config);

    assert_eq!(
        state.capital_body("中国"),
        "水星",
        "a Player-chosen capital must survive the periodic AI review"
    );
    assert!(
        !state.events.iter().any(|e| matches!(
            e,
            GameEvent::CapitalRelocated { faction, .. } if faction == "中国"
        )),
        "no relocation may fire for a Player-owned capital, got {:?}",
        state.events
    );
}
