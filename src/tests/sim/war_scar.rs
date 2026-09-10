//! **记恨地板（战争疤痕）自身的形状**——纯单元（不推进回合，所以住快档）。
//!
//! 开战之后 `war_scar_rounds` 回合内，这一对势力的关系被压在一道线性衰减的地板下：
//! 随年龄抬高、窗口内始终是敌意、出了窗口**彻底消失**（不再影响任何计算——这正是它属于
//! **窗口层**而不是里程碑层的原因），而且只属于开战的那一对、与势力顺序无关。
//!
//! 形状要一个**手工摆出来的世界**（`HistoryEntry` 里塞一条 `WarStarted`）+ 直接调
//! `war_scar_floor` ⇒ 它搬不去 Python（那边只能看跑出来的数据），所以留在这里。
//!
//! **「地板真的是一道地板」那一半**（真实长局里没有任何一场战争短于地板承诺的回合数）
//! 已搬到 `play/tests/g2_mid.py`：它把 `war_started`/`war_ended` 配对算时长，再与
//! `meta.json` 里的 `war_scar_rounds`/`war_scar_relation`/`war_threshold` 算出的最短回合比。
//! 那一条**曾经失败过**（最短 6 回合）：`step_balance_of_power` 的「合纵」走另一个关系
//! 写入者，绕过了只在外交漂移里套用的地板。修法是让地板进入**关系写入的唯一漏斗**
//! （`set_relation_sym` / `adjust_relation`），而不是在测试里放宽断言。

use super::*;

/// 地板的形状：满额起点、随年龄单调抬高、窗口内仍带着敌意、出窗口消失、只属于那一对、
/// 与势力顺序无关；以及「地板抬过交战阈值所需的年龄 = 战争最短回合数」这个换算。
#[test]
fn war_scar_floor_shape_decays_over_its_window() {
    let config = load_config();
    let span = config.diplomacy.war_scar_rounds;
    let base = config.diplomacy.war_scar_relation;
    let thr = config.combat.war_threshold;
    assert!(span > 0, "war_scar_rounds 应当开启");
    assert!(
        base < thr,
        "疤痕初值必须低于交战阈值（{base} vs {thr}），否则压不住言和"
    );

    let mut s = default_state(&config, 1);
    s.round = 10;
    s.notables.entries.push(crate::model::HistoryEntry {
        round: 10,
        event: GameEvent::WarStarted {
            a: "甲".into(),
            b: "乙".into(),
        },
    });
    let at = |age: u32| {
        let mut t = s.clone();
        t.round = 10 + age;
        war_scar_floor(&t, &config, "甲", "乙")
    };
    assert_eq!(at(0), Some(base), "刚开战必须是满额敌意");
    assert!(at(1).unwrap() > at(0).unwrap(), "地板必须随年龄单调抬高");
    assert!(at(span - 1).unwrap() < 0.0, "窗口内应当仍然带着敌意");
    assert_eq!(at(span), None, "出了 war_scar_rounds 之后疤痕必须彻底消失");
    assert_eq!(
        war_scar_floor(&s, &config, "甲", "丙"),
        None,
        "疤痕只属于开战的那一对，不牵连第三方"
    );
    assert_eq!(
        war_scar_floor(&s, &config, "乙", "甲"),
        at(0),
        "疤痕与势力顺序无关（必须无序匹配）"
    );

    // 地板抬过交战阈值所需的最小年龄 = 战争最短回合数（`g2_mid.py` 那条判据用的就是它）。
    let min_age = (0..=span)
        .find(|a| base * (1.0 - (*a as f64) / (span as f64)) > thr)
        .expect("疤痕必须最终抬过交战阈值，否则战争永远结束不了");
    assert!(
        min_age > 0 && min_age <= span,
        "最短战争 {min_age} 回合应当落在窗口 1..={span} 内"
    );
}
