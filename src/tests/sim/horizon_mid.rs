//! **中档（T2，49–480 回合）**的 sim 行为用例：60 回合一类（编年史 / 参与者 / 战痕地板）。
//! 它们要的**模拟时长就是判据本身**，所以不进快档——默认 `cargo nextest run` 不选它们，
//! `cargo nextest run -P mid` 选。
//!
//! ## 两条 400 回合的用例已搬到 Python 侧（`play/tests/g2_mid.py`）
//!
//! 2026-10（用户裁决：*「测试应该和游戏二进制解耦，直接测跑出来的数据」*）：
//!
//! * `a_city_razed_this_round_is_not_refounded_by_its_own_loser_this_round` —— 判据完全在
//!   **事件层**（`city_razed` 的 `data.owner` 与同回合 `colony_founded` 的 actor 比先后与归属），
//!   读面就够；现在住 `g2_mid.py`「被拆平的城不在同回合被旧主复垦」（同种子 `[1,7,42]`、
//!   同 400 回合、同「拆平数 ≥ 20」的防空转下限）。
//! * `long_run_produces_customized_ships` —— 判据在**舰表**（`hull > 0` + `components` 非空），
//!   同样口径（「整局里出现过」而不是「末回合还剩着」）。
//!
//! 留在 Rust 的是需要 crate 内部量/夹具的（战痕地板要读 `war_scar_rounds` 的关系序列、
//! 编年史要 `fresh_world` 夹具）：搬不动的两栏对账见 `.agents/notes/test-decoupled-suite.md`。

use super::*;

/// 剧情编年史：RoundAt 节拍按回合触发、编年史按发生先后单调增长、id 唯一，且
/// 同一种子完全确定（重跑逐字节一致）。
#[test]
fn story_chronicle_grows_deterministically() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    // Round-at beats: prologue fires round 1, planet_x_arrives round 60.
    for _ in 0..60 {
        advance(&mut state, &config, &mut rng);
    }
    let ids: Vec<&str> = state.chronicle.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"prologue"), "prologue (RoundAt 1) must fire");
    assert!(
        ids.contains(&"planet_x_arrives"),
        "planet_x_arrives (RoundAt 60) must fire"
    );

    // The chronicle records the round it fired, in non-decreasing order.
    let rounds: Vec<u32> = state.chronicle.iter().map(|c| c.round).collect();
    let mut sorted = rounds.clone();
    sorted.sort_unstable();
    assert_eq!(rounds, sorted, "chronicle must be sorted by firing round");

    // ids are unique (each event fires once).
    let mut dedup = ids.clone();
    dedup.sort_unstable();
    let before_n = dedup.len();
    dedup.dedup();
    assert_eq!(before_n, dedup.len(), "each story id fires at most once");

    // Determinism: re-running the same seed reproduces the identical chronicle.
    let (_, mut state2) = fresh_world(42);
    let mut rng2 = Prng::new(42);
    for _ in 0..60 {
        advance(&mut state2, &config, &mut rng2);
    }
    assert_eq!(
        state
            .chronicle
            .iter()
            .map(|c| (c.round, c.id.clone(), c.title.clone()))
            .collect::<Vec<_>>(),
        state2
            .chronicle
            .iter()
            .map(|c| (c.round, c.id.clone(), c.title.clone()))
            .collect::<Vec<_>>(),
        "same seed must produce the same story arc"
    );
}

/// 剧情参与方是「具体的」：事件型触发把本回合事件的实际对象写进编年史
/// （谁与谁开战、哪座城被夷平、谁建立了殖民地），而不是泛化的空标签。
#[test]
fn story_participants_are_concrete() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    for _ in 0..60 {
        advance(&mut state, &config, &mut rng);
    }
    let find = |id: &str| state.chronicle.iter().find(|c| c.id == id);
    if let Some(war) = find("first_war") {
        assert_eq!(
            war.participants.len(),
            2,
            "first_war names the two belligerents, got {:?}",
            war.participants
        );
        assert!(war.participants.iter().all(|p| !p.is_empty()));
    }
    if let Some(razed) = find("first_raze") {
        assert!(
            razed.participants.len() >= 2,
            "first_raze names the city and the razer, got {:?}",
            razed.participants
        );
    }
    if let Some(colon) = find("first_colony") {
        assert!(
            colon.participants.len() >= 2,
            "first_colony names the colonizer and the body, got {:?}",
            colon.participants
        );
    }
    if let Some(cn) = find("cn_us_rivalry") {
        assert!(
            cn.participants.contains(&"中国".to_string()),
            "cn_us_rivalry names 中国, got {:?}",
            cn.participants
        );
        assert!(
            cn.participants.contains(&"美国".to_string()),
            "cn_us_rivalry names 美国, got {:?}",
            cn.participants
        );
    }
    // RoundAt beats keep exactly their static participants (no event to enrich).
    if let Some(pro) = find("prologue") {
        assert_eq!(
            pro.participants,
            vec!["无国界科学组织".to_string(), "行星X崇拜教".to_string()]
        );
    }
}

/// **记恨地板（战争疤痕）**：开战之后 `war_scar_rounds` 回合内，这一对势力的关系被压在一道
/// 线性衰减的地板下——于是「刚开战就当回合言和」不可能。
///
/// 这条测试钉住两件事：
/// 1. **地板自身的形状**：随年龄抬高、窗口内始终是敌意、出了 `war_scar_rounds` 彻底消失
///    （窗口过期 = 不再影响任何计算，这正是它属于窗口层而不是里程碑层的原因）。
/// 2. **地板真的是一道地板**：用真实长局验证「没有任何一场战争短于地板承诺的回合数」。
///    这一条曾经**失败过**（最短 6 回合）：`step_balance_of_power` 的「合纵」走另一个关系
///    写入者，绕过了只在外交漂移里套用的地板。修法是让地板进入**关系写入的唯一漏斗**
///    （`set_relation_sym` / `adjust_relation`），而不是在这个测试里放宽断言。
#[test]
fn war_scar_floor_makes_a_real_floor_on_war_duration() {
    let config = load_config();
    let span = config.diplomacy.war_scar_rounds;
    let base = config.diplomacy.war_scar_relation;
    let thr = config.combat.war_threshold;
    assert!(span > 0, "war_scar_rounds 应当开启");
    assert!(
        base < thr,
        "疤痕初值必须低于交战阈值（{base} vs {thr}），否则压不住言和"
    );

    // 1. 地板形状：只属于开战的那一对，随年龄抬高，到 span 之后消失。
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

    // 地板抬过交战阈值所需的最小年龄 = 战争最短回合数。
    let min_age = (0..=span)
        .find(|a| base * (1.0 - (*a as f64) / (span as f64)) > thr)
        .expect("疤痕必须最终抬过交战阈值，否则战争永远结束不了");

    // 2. 真实长局：没有一场战争短于 min_age。
    let mut state = default_state(&config, 7);
    let mut rng = crate::prng::Prng::new(7);
    let mut open: std::collections::BTreeMap<(String, String), u32> =
        std::collections::BTreeMap::new();
    let mut shortest = u32::MAX;
    let mut episodes = 0usize;
    for _ in 0..60 {
        advance(&mut state, &config, &mut rng);
        let round = state.round;
        for e in &state.events {
            let pair = |a: &String, b: &String| {
                if a <= b {
                    (a.clone(), b.clone())
                } else {
                    (b.clone(), a.clone())
                }
            };
            match e {
                GameEvent::WarStarted { a, b } => {
                    open.entry(pair(a, b)).or_insert(round);
                }
                GameEvent::WarEnded { a, b } => {
                    if let Some(start) = open.remove(&pair(a, b)) {
                        episodes += 1;
                        shortest = shortest.min(round - start);
                    }
                }
                _ => {}
            }
        }
    }
    assert!(episodes >= 5, "60 回合里只打完 {episodes} 场战争，样本太小");
    assert!(
        shortest >= min_age,
        "最短战争 {shortest} 回合 < 地板承诺的 {min_age} 回合——\
         说明有某个关系写入者绕过了地板（见 set_relation_sym 的说明）"
    );
}
