//! 齐射的事件层：被点防吃光的齐射也必须留一条事件。
//!
//! ## 2026-10（第 7 批）：两条搬去了 g2 `combat_report`/`combat_checks`
//!
//! | 原用例 | 判据 |
//! | --- | --- |
//! | `the_event_breakdown_adds_up_to_the_aggregate_damage` | **齐射一级**：`Σ shots[].damage == magnitude`（3 seed 共 561 条齐射），且逐发都在取值域里（命中折减 ∈ [0.2,1]、护盾吸收 ∈ [0,1]、护甲减伤 ∈ [0,0.85]、防御倍率 > 0、火力分配 > 0、`target_hull_before > 0`） |
//! | `a_salvo_aimed_only_at_a_corpse_does_not_emit_an_attack` | 每条 `attack` **至少有一发真打出去**（全是被跳过的发 = 不该发事件）；防空转 = 真出现过被跳过的发（3 seed 里 2 发） |
//!
//! **留在这里的**：`a_fully_intercepted_salvo_still_leaves_an_event` —— 它要**构造**「两层点防
//! 吃光一发导弹」，而读面上只看得见结果：实测 7 seed × 1000 回合里这种齐射一共只有 **2 条**
//! （多数种子一条都没有）⇒ 「一条都没发生」时判据无法区分「机制坏了」与「没打过这种仗」。
//!
//! 这一批动的不是读面（`RoundView` 一个字段没加），而是 [`GameEvent::Attack`]：它长出 `shots`
//! ——每一发的完整分解（选择三项分 + 命中/点防/护盾/护甲/破甲）。同时**放宽了发事件的闸**：
//! 从前「总伤害 > 0 才发」，现在「真朝一个活目标打过一发就发」——因为「齐射被点防吃光」正是
//! 伤害为 0 的那一格，而它在旧规则下**什么事件都不留**。
//!
//! 用例钉住三件事：① 逐发之和 = 聚合伤害；② 被吃光的齐射也留事件；③ 世界行为没变
//! （关系调整与「交火」判据都还只认真打出来的伤害）。

use super::*;

/// 一对互相敌对的舰（射手 = `ships[0]` 中国；目标 = `ships[3]` 美国），摆进彼此的射程内。
///
/// 组件与位置都由用例指定：`fire` 只看 plan，所以这里能精确造出「被点防吃光」的情形
/// （两层 `point_defense` 的拦截量 12 > 一发导弹的 9）。
fn duel(seed: u64, shooter_components: &[&str], target_components: &[&str]) -> (GameConfig, State, ShipId, ShipId) {
    let (config, mut state) = fresh_world(seed);
    let shooter = state.ships[0].name.clone();
    let victim = state.ships[3].name.clone();
    // 默认世界开局和平 ⇒ 显式让两家敌对（同 `fleet.rs` 的手工场景）。
    for (a, b) in [("中国", "美国"), ("美国", "中国")] {
        if let Some(f) = state.faction_mut(a) {
            f.relations.insert(b.to_string(), -35.0);
        }
    }
    if let Some(s) = state.ship_mut(&shooter) {
        s.components = shooter_components.iter().map(|c| c.to_string()).collect();
        // ⚠ 必须按**组件自己的满值**填 `component_hp`：有效性 = `hp ÷ 满值`，填 1.0 会让
        // 一件满血组件只发挥几分之一（第一版就踩了这个：两层点防只剩 0.67 拦截量）。
        s.component_hp = s
            .components
            .iter()
            .map(|c| crate::model::component_integrity(&config, c))
            .collect();
        s.position = [0.0, 0.0];
    }
    if let Some(s) = state.ship_mut(&victim) {
        s.components = target_components.iter().map(|c| c.to_string()).collect();
        s.component_hp = s
            .components
            .iter()
            .map(|c| crate::model::component_integrity(&config, c))
            .collect();
        s.position = [0.3, 0.0];
    }
    (config, state, shooter, victim)
}

fn order(weapon: usize, target: &ShipId) -> crate::sim::FireOrder {
    crate::sim::FireOrder {
        weapon,
        target: target.clone(),
        score_basic: 1.0,
        score_temper: 0.0,
        score_spread: 1.0,
    }
}

/// **被点防吃光的齐射也必须留一条事件**：这是 B4 放宽发事件闸的唯一理由。
///
/// 旧规则（总伤害 > 0 才发）下，「我的导弹齐射为什么全被拦下了」在任何读面都查不到：
/// 拦截量线性吃掉全部伤害 ⇒ `damage = 0` ⇒ 一条事件都没有。
#[test]
fn a_fully_intercepted_salvo_still_leaves_an_event() {
    let (config, mut state, shooter, victim) = duel(42, &["missile"], &["point_defense", "point_defense"]);
    crate::sim::fire(&mut state, &config, &shooter, &[order(0, &victim)]);

    let (damage, shots) = state
        .events
        .iter()
        .find_map(|e| match e {
            GameEvent::Attack { damage, shots, .. } => Some((*damage, shots.clone())),
            _ => None,
        })
        .expect("被拦光的齐射必须留下一条 Attack 事件（这正是 C5 要回答的那一格）");
    assert_eq!(damage, 0.0, "两层点防 12 > 一发导弹 9 ⇒ 打不出伤害");
    assert_eq!(shots.len(), 1, "一件武器一发 = 一条逐发记录");
    let s = &shots[0];
    assert!(!s.skipped, "目标是活的，这一发真打出去了");
    assert!(s.in_range, "摆在同一位置上，必须在射程内");
    assert!(s.pd >= 9.0, "两层点防的拦截量应当盖过一发导弹（实为 {}）", s.pd);
    assert!(
        s.pd_absorbed > 0.0,
        "被点防吃掉的伤害必须为正（实为 {}）",
        s.pd_absorbed
    );
    assert_eq!(s.damage, 0.0, "全被拦下 ⇒ 这一发不打任何伤害");
    assert_eq!(s.hull_pen, 0.0);
    assert_eq!(s.absorbed, 0.0);
}
