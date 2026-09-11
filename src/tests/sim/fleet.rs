//! 舰队行为与默认：逐舰指令的归属、殖民归属、跟随友舰时自动开火只打敌对者。
//!
//! ## 2026-10（第 7 批）：`colonize_keeps_player_ownership` 搬去了 g2 **合成场景 · 殖民**（6 条判据）
//!
//! 起点回合与天体**从长局里扫出来**（开局 22 座城占满 22 个定居点，得等到有城被拆平）：
//! 实测第 29 回合木星空出「木星轨道空间站」⇒ 钉一片玩家 `Colonize{木星}` 叶，第 33 回合建城
//! `大红斑科学站`、`order_effective` 随后变 `Idle`（一次性指令被花掉）、`order_leaf_mode` **全程 Player**。
//! 早退臂（回合 0 目标天体已满）同样只断言「叶片仍是 Player」。
//! ⚠ `colony_founded` 的 `target_id` 是**城名**，天体在 `data.body`。
//!
//! ## 2026-10：能只看数据的那条搬去了 `play/tests/g1_contract.py`
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `advance_populates_round_events` | g1「全新开局的回合 0 没有事件」「推进过就有事件（事件层真的在写）」 | 原用例只断言「回合 0 空 → 跑几回合非空」，读面上这两半都在（`events` 表按 `round` 分组即可） |
//! | `newly_built_ships_have_no_order_of_their_own` | g2「新舰出厂时叶片没有说话 / 新舰归系统」 | 3 seed × 400 回合里 **594 艘**新舰：`order_leaf_mode` 只会是 `Inherit`、`order_effective_mode` 只会是 `Auto`（有人重新加一片「舰队默认指令叶」就会被抓住）。⚠ 原件还断言「叶片里的**值**是占位 `Idle`」——那半读面看不见（AI 同回合就派活，实测 345 艘里只有 30 艘还留着 `Idle`） |
//! | `player_stale_follow_degrades_to_idle_and_does_not_drift` | g2 **合成场景 · 陈旧的跟随**（5 条判据，防空气转在别的舰身上） | 钉一根**从第 0 回合就悬空**的 `Follow`（写面**不校验指令目标**，与悬空图指针正相反）⇒ 退化后的值在 `order_effective`、漂没漂在 `ships.x/y`、留没留痕在 `events[stale_order]` |
//! | `dock_follows_body_and_idle_holds_position` | g2 **合成场景 · 停泊与待命**（4 条判据） | 第 7 批给读面补了派生表 **`body_positions`**（逐回合天体位置，与 `ships.x/y` 同一把绝对坐标尺子）⇒「Dock 有没有朝那个天体去」「Idle 的位置有没有动」都判得了。⚠ 必须钉成 `Player`：AI 会在回合末刚派完 Dock、下一回合开头就改派（实测长局里 `Dock` 的 797 个「两回合同天体」样本**全部原地没动**，就是这种没执行过的叶子） |

use super::*;

/// Follow semantics: `Follow { ship }` escorts/drives the ship — it is a pure
/// movement behavior. Combat is now automatic: when any hostile is inside the
/// ship's own attack range it auto-fires (via the unified base-weight targeting),
/// so a Follow ship still defends itself but never fires at the followed friend.
#[test]
fn follow_ship_auto_attacks_hostile_but_not_the_followed_friend() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // China (3): ship 0 Follows its own friendly ship 1. Co-located at [0,0].
    let ship0 = state.ships[0].name.clone();
    let ship1 = state.ships[1].name.clone();
    let ship3 = state.ships[3].name.clone();
    let ship4 = state.ships[4].name.clone();
    let ship5 = state.ships[5].name.clone();
    let diff = serde_json::json!({
        "control": [{
            "势力": "中国",
            "指令": [{"舰": ship0.clone(), "行为": {"Follow": {"ship": ship1.clone()}}, "归属": "Player"}]
        }]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("apply follow order");

    // Pin positions: follower + followed friend at [0,0]; US (1) enemy
    // destroyer id=3 just inside the corvette attack range (0.4) so the
    // auto-attack can fire. Move the other US ships (4 destroyer, 5 cruiser)
    // far out so only ship 3 engages (its damage 6 won't one-shot the follower's
    // hull 12, letting it retaliate).
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [0.0, 0.0];
    }
    if let Some(s) = state.ship_mut(&ship1) {
        s.position = [0.0, 0.0];
    }
    if let Some(enemy) = state.ship_mut(&ship3) {
        enemy.position = [0.3, 0.0];
    }
    if let Some(s) = state.ship_mut(&ship4) {
        s.position = [50.0, 50.0];
    }
    if let Some(s) = state.ship_mut(&ship5) {
        s.position = [50.0, 50.0];
    }

    // The default world now opens peacefully, so make US (1) explicitly
    // hostile to China (3) for this scenario.
    if let Some(f) = state.faction_mut("中国") {
        f.relations.insert("美国".to_string(), -35.0);
    }
    if let Some(f) = state.faction_mut("美国") {
        f.relations.insert("中国".to_string(), -35.0);
    }

    advance(&mut state, &config, &mut rng);

    // The ship must auto-fire on the hostile, not on the friend.
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::Attack { attacker, target, .. } if attacker == &ship0 && target == &ship3
        )),
        "ship should auto-attack the hostile, got {:?}",
        state.events
    );
    // The followed friend must be unharmed (no attack targeting ship 1).
    assert!(
        !state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::Attack { target, .. } if target == &ship1)),
        "ship must not fire at its own followed friend, got {:?}",
        state.events
    );
    // The order is still a valid Follow (not degraded to Idle).
    assert_eq!(
        state.ship_behavior(ship0.clone()),
        Some(ShipBehavior::Follow {
            ship: ship1.clone()
        })
    );
}
