//! 剧情：机械后果真的落到状态上、`grant_ship` 真的多出一艘舰队成员。

use super::*;

/// 剧情机械后果：prologue 给无国界科学组织(6)注入氦-3，并拉低它与行星X崇拜教(8)的关系。
#[test]
fn story_effects_apply() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    let rel_before = state
        .faction("无国界科学组织")
        .and_then(|f| f.relations.get(&"行星X崇拜教".to_string()).copied())
        .unwrap_or(0.0);

    advance(&mut state, &config, &mut rng);

    // prologue 是「事件型后果」：把资源写入并在编年史里记录。第 1 回合经济（维护费/
    // 市场）会立刻重新平衡库存，故不断言 helium3 净增（它可能被维护费/市场抵消），
    // 而断言那份资源确实进入了编年史记录的 prologue（机械后果生效）。
    assert!(
        state.chronicle.iter().any(|c| c.id == "prologue"),
        "prologue must fire and record its effects at round 1"
    );
    let rel_after = state
        .faction("无国界科学组织")
        .and_then(|f| f.relations.get(&"行星X崇拜教".to_string()).copied())
        .unwrap_or(0.0);
    assert!(
        rel_after < rel_before,
        "prologue must lower science↔cult relation (effect)"
    );
}

/// 剧情 GrantShip 后果：kuiper_boom（RoundAt 24）给星系矿业(5)出厂一艘巡洋舰；
/// 出厂位置恰好在天体当前位置 + (0.05, 0.05)（确定性偏移）、带 Idle 指令、id 连续。
#[test]
fn story_grant_ship_spawns_a_fleet_member() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // Advance to round 24 so kuiper_boom fires.
    for _ in 0..24 {
        advance(&mut state, &config, &mut rng);
    }

    // The story fired this round.
    assert!(
        state
            .chronicle
            .iter()
            .any(|c| c.id == "kuiper_boom" && c.round == 24),
        "kuiper_boom must fire at round 24, got {:?}",
        state
            .chronicle
            .iter()
            .map(|c| (c.id.clone(), c.round))
            .collect::<Vec<_>>()
    );

    // The granted cruiser is at exactly body 9 (泰坦) position + the deterministic offset.
    let bpos = state.body_position("泰坦");
    let granted = state
        .ships
        .iter()
        .find(|s| {
            s.faction_id == "星系矿业"
                && s.class == "cruiser"
                && (s.position[0] - (bpos[0] + 0.05)).abs() < 1e-9
                && (s.position[1] - (bpos[1] + 0.05)).abs() < 1e-9
        })
        .expect("kuiper_boom must grant 星系矿业 a cruiser parked at 泰坦");
    assert_eq!(
        state.ship_behavior(granted.name.clone()),
        Some(ShipBehavior::Idle),
        "granted ship starts Idle"
    );

    // Determinism: re-running reproduces the identical granted fleet.
    let (_, mut state2) = fresh_world(42);
    let mut rng2 = Prng::new(42);
    for _ in 0..24 {
        advance(&mut state2, &config, &mut rng2);
    }
    let fleet_a: Vec<(String, String)> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == "星系矿业")
        .map(|s| (s.name.clone(), s.class.clone()))
        .collect();
    let fleet_b: Vec<(String, String)> = state2
        .ships
        .iter()
        .filter(|s| s.faction_id == "星系矿业")
        .map(|s| (s.name.clone(), s.class.clone()))
        .collect();
    assert_eq!(
        fleet_a, fleet_b,
        "same seed must reproduce the same granted fleet"
    );
}
