//! 剧情：`grant_ship` 真的多出一艘舰队成员。
//!
//! ## 2026-10：能只看数据的那条搬去了 `play/tests/g2_mid.py`
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `story_effects_apply` | g2「剧情的机械后果真的落到读面上（声明的每处关系增减都发生了）」 | 节拍**清单与后果**都从 `meta.story` 读（`{"kind":"relations","a":…,"b":…,"delta":…}`），逐条对着 `factions.关系` 看方向；「prologue 在第 1 回合触发」那半 g2「RoundAt 节拍都触发了」早就压着 |
//!
//! **留在这里的**：`story_grant_ship_spawns_a_fleet_member` —— 它要断言出厂位置
//! **恰好是天体当前位置 + (0.05, 0.05)**，而读面上的 `bodies` 表是**静态**的（只有轨道根数，
//! 没有逐回合位置）⇒ 在 Python 里算那个位置就是把轨道公式抄第二遍（§4 明说不搬）。

use super::*;

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
