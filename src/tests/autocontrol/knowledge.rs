//! 观测舰（角色轴第三态）的单元测试：**三个动机怎么抢一支舰队**、选靶、期望入伙数、行为。
//!
//! 用户裁决（第二轮追加）：「加硬阈值只能说明动机设计的不够好，把资源堆积的运输动机和战争
//! 威胁动机覆盖了，**不能加阈值要自然**」。所以这一份用例的重点是**挤压关系**：
//! 积压涨 ⇒ 观测分到的少；威胁涨 ⇒ 观测分到的少；思潮（科学↔技术）决定观测那一支值多少。
//! 分工：**本文件**测机制本身；**探针**（`tests/tech_probe.rs`）测整个世界跑 600 回合之后
//! 到底有没有人学会 MOND。

use super::*;
use crate::autocontrol::freight::{assign_roles, observer_quota, role_quotas};
use crate::autocontrol::knowledge::{observe_claim, observe_lean};
use crate::autocontrol::tactics::ai_ship_turn;
use crate::config::load_config;
use crate::world::default_state;
use std::collections::BTreeMap;

fn fresh(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// 某势力此刻在观测的舰（按舰名序）。
fn observers(state: &State, fid: &str) -> Vec<String> {
    let mut v: Vec<String> = state
        .ships
        .iter()
        .filter(|s| {
            s.faction_id == fid
                && s.hull > 0.0
                && state.ship_role(s.name.clone()) == ShipRole::Observe
        })
        .map(|s| s.name.clone())
        .collect();
    v.sort();
    v
}

/// 把某势力的舰队克隆 `times` 倍（名字加后缀）——用例需要一支**够大的**舰队，
/// 否则「各支能分到多少」会被舰队规模顶住，看不出动机之间的挤压。
fn grow_fleet(state: &mut State, fid: &str, times: usize) {
    let base: Vec<Ship> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .cloned()
        .collect();
    for k in 1..=times {
        for s in &base {
            let mut c = s.clone();
            c.name = format!("{}-{k}", c.name);
            state.ships.push(c);
        }
    }
}

/// **尺子**：`期望在场收益 = p(深度, 掌握度) × (1 + 深度 × depth_weight)`。
///
/// 这条用例钉住的是 §11 初稿的**一处修正**：初稿写的是「取 `深度 × p` 最大者」，而实测
/// 两者在凡人这一端**给出相反的答案**——凡人时 `创神星`（深 10.2、p = 0.20）的 `深度 × p`
/// 与 `海王星`（深 2、p = 1.0）打平，于是抽签会把人派去一个大概率迷航的深空目标。
/// 按**在场收益**算：海王星 `1.5` vs 创神星 `0.71` ⇒ 凡人先蹲前沿边上；
/// 掌握度到顶时每个天体的 p 都是 1 ⇒ 收益 = 在场价值 ⇒ 最深的天体最重。
/// 所以这条尺子**自己就会随掌握度往外迁移**，不需要任何「什么时候该换目标」的开关。
#[test]
fn expected_presence_gain_keeps_mortals_near_the_frontier_and_masters_deep() {
    let (config, _state) = fresh(42);
    let shallow = 2.0; // 海王星一带（30 AU）
    let deep = 10.2; // 创神星/阋神星一带（约 38 AU）
    let w = |c: f64, d: f64| crate::autocontrol::knowledge::body_weight(&config, c, d);
    assert!(
        w(0.0, shallow) > w(0.0, deep),
        "凡人该蹲前沿边上（深处期望收益更低）：{:.2} vs {:.2}",
        w(0.0, shallow),
        w(0.0, deep)
    );
    assert!(
        w(1.0, deep) > w(1.0, shallow),
        "指哪打哪的势力该往深处去（p 都是 1 ⇒ 收益 = 在场价值）"
    );
    // 中间是**连续**的过渡，不是开关。
    assert!(w(0.9, deep) > w(0.0, deep));
}

/// **三支力量抢舰队**（用户裁决：不许加阈值，要自然）——这条用例把三处挤压关系一次钉住：
///
/// 1. **没有任何角色上限**：没有积压、没有威胁时，观测能拿到**超过一半**的舰队
///    （第一版那条「最多占一半」写死的上限会在这里翻红——它正是被否掉的东西）；
/// 2. **资源堆积的运输动机能把它顶回去**：堆几处货栈 ⇒ 运输主张变大 ⇒ 观测分到的变少；
/// 3. **战争威胁动机能把它顶回去**：把一支比自己强的敌军摆到开战关系上 ⇒ 战舰那一份吃掉
///    余量 ⇒ 观测（和运输）一起缩。
#[test]
fn the_three_motives_share_the_fleet_with_no_cap_anywhere() {
    let (config, base) = fresh(42);
    let fid = "中国";
    let mut calm = base.clone();
    grow_fleet(&mut calm, fid, 4); // 15 艘：够大，配额不会被舰队规模顶住
    let fleet = |s: &State| {
        s.ships
            .iter()
            .filter(|x| x.faction_id == fid && x.hull > 0.0)
            .count() as f64
    };
    let n = fleet(&calm);
    assert!(n >= 15.0, "用例前提：克隆出一支大舰队（{n}）");
    assert!(
        crate::autocontrol::shipbuilding::threat_motive(&calm, &config, fid) < 0.2,
        "用例前提：这一支没有威胁"
    );

    // ① 无积压、无威胁 ⇒ 观测能拿走**过半**的舰队。⚠ 这里用**小舰队**：观测的主张是
    //    `学满所需头数 × 思潮倾向`（在 15 艘的舰队里它可能只有 5–6 艘，于是「全额兑现」而
    //    不到一半）——「没有上限」这件事在小舰队上才看得出来：3 艘里能有 2 艘去观测，
    //    而第一版那条写死的「最多占一半」会把这里卡在 1.5 艘。
    let small = base.clone();
    let small_n = small
        .ships
        .iter()
        .filter(|x| x.faction_id == fid && x.hull > 0.0)
        .count() as f64;
    let (_, sm_frt, sm_obs) = role_quotas(&small, &config, fid);
    assert!(small_n >= 3.0, "用例前提：中国开局有舰（{small_n}）");
    assert!(
        sm_obs > 0.5 * small_n,
        "没有别的动机时，观测该能拿到过半的舰队（{sm_obs:.2} / {small_n}）\
         ——这正是「一律最多一半」那条上限做不到的事"
    );
    assert!(sm_frt + sm_obs <= small_n + 1e-9);

    // ① b 大舰队：主张装得下预算 ⇒ **全额兑现**（不封顶，也不被舰队规模硬压）。
    let (war, frt, obs) = role_quotas(&calm, &config, fid);
    let claim = observe_claim(&calm, &config, fid);
    assert!(
        (obs - claim).abs() < 1e-9,
        "装得下就各得其所：观测配额该等于它的主张（{obs:.2} vs 主张 {claim:.2}）"
    );
    assert!(
        war + frt + obs <= n + 1e-9,
        "三支加起来不该超过舰队：{war:.2}+{frt:.2}+{obs:.2} vs {n}"
    );
    assert!(war / n > 0.2, "常备军那一份仍在（{:.2}）", war / n);

    // ② 堆货栈 ⇒ 运输主张变大。⚠ 要点满到**主张超过预算**才会挤压：水位配给的规矩是
    //    「装得下就各得其所」——只有加起来超了，才会按相对大小成比例缩水。所以这里堆够
    //    十处货栈，并**断言前提**（主张之和确实超了预算），免得这条用例悄悄退化成空转。
    let mut stocked = calm.clone();
    let bodies = [
        "金星",
        "水星",
        "火星",
        "木星",
        "土星",
        "天王星",
        "海王星",
        "冥王星",
        "卡戎",
        "谷神星",
    ];
    for (i, body) in bodies.iter().enumerate() {
        stocked.depot_add(fid, body, "碳", 500.0 * (i as f64 + 1.0));
    }
    let freight_claim = crate::autocontrol::freight::freighter_quota(&stocked, &config, fid);
    let observe_claim_v = observe_claim(&stocked, &config, fid);
    assert!(
        freight_claim + observe_claim_v > (war + frt + obs - war) + 1e-9,
        "用例前提：两支主张加起来要超过可分预算才会挤压（主张 {:.2} vs 预算 {:.2}）",
        freight_claim + observe_claim_v,
        frt + obs
    );
    let (_, frt2, obs2) = role_quotas(&stocked, &config, fid);
    assert!(frt2 > frt, "积压上来，运输该多分到（{frt:.2} → {frt2:.2}）");
    assert!(
        obs2 < obs,
        "**资源堆积的运输动机该把观测顶回去**：{obs:.2} → {obs2:.2}"
    );

    // ③ 一支比自己强的敌军摆到开战关系上 ⇒ 战舰那一份吃掉余量 ⇒ 观测（与运输）再缩。
    let mut threatened = stocked.clone();
    // 40 倍：威胁读的是 `faction_power_share`（城 + 舰体占全星系的比例），要把 15 艘舰的
    // 中国压成弱势，对手得真的**大一个数量级**（×6 只有 0.16，因为中国的城与舰本来就多）。
    grow_fleet(&mut threatened, "美国", 40);
    for f in ["中国", "美国"] {
        let other = if f == "中国" { "美国" } else { "中国" };
        threatened
            .faction_mut(f)
            .unwrap()
            .relations
            .insert(other.to_string(), -200.0);
    }
    let threat = crate::autocontrol::shipbuilding::threat_motive(&threatened, &config, fid);
    assert!(
        threat > 0.5,
        "用例前提：这一支确实被强敌压着（威胁 {threat:.2}）"
    );
    let (war3, frt3, obs3) = role_quotas(&threatened, &config, fid);
    assert!(
        war3 > war,
        "威胁上来，战舰那一份该变大（{war:.2} → {war3:.2}）"
    );
    assert!(
        obs3 < obs2,
        "**战争威胁动机该把观测顶回去**：{obs2:.2} → {obs3:.2}"
    );
    assert!(
        frt3 < frt2,
        "余量被战舰吃掉，运输也一起缩（{frt2:.2} → {frt3:.2}）"
    );
    assert!(
        war3 + frt3 + obs3 <= n + 1e-9,
        "极端威胁下也该装得进舰队：{war3:.2}+{frt3:.2}+{obs3:.2} vs {n}"
    );
}

/// **思潮决定观测那一支值多少**（与集货的 `freight_lean` 同形）：科学端 > 中庸 > 技术端。
/// 方向与 `governance` 那条「科学端 + 舰不在异常区 = 言行不符」的忠诚惩罚**同向**。
#[test]
fn the_science_axis_decides_how_much_the_fleet_wants_to_observe() {
    let (config, base) = fresh(42);
    let fid = "中国";
    let claim = |sci: f64| -> f64 {
        let mut st = base.clone();
        st.faction_mut(fid).unwrap().ideology.science_tech = sci;
        observe_claim(&st, &config, fid)
    };
    let (science, neutral, tech) = (claim(-1.0), claim(0.0), claim(1.0));
    assert!(
        science > neutral && neutral > tech,
        "科学端 > 中庸 > 技术端：{science:.2} / {neutral:.2} / {tech:.2}"
    );
    let lean = |sci: f64| -> f64 {
        let mut st = base.clone();
        st.faction_mut(fid).unwrap().ideology.science_tech = sci;
        observe_lean(&st, fid)
    };
    assert!((lean(0.0) - 1.0).abs() < 1e-9, "中庸（0）⇒ 倾向正好 1.0");
    assert!(
        lean(-1.0) > 1.0 && lean(1.0) < 1.0,
        "科学端 > 1、技术端 < 1：{:.2} / {:.2}",
        lean(-1.0),
        lean(1.0)
    );
    assert!(
        science > 1.3 * tech,
        "两端该拉开可见的差距（{:.2}×）",
        science / tech
    );
}

/// **学满之后不再主张**：掌握度到顶（棘轮）⇒ 观测主张 0 ⇒ 配额 0。
/// 这不是阈值断崖，而是 `sim::step_knowledge` 里那条棘轮的同一句话（到顶不再变化 ⇒ 也没必要派人）。
#[test]
fn a_mastered_faction_claims_nothing_and_keeps_its_ships_at_war() {
    let (config, mut state) = fresh(42);
    let fid = "中国";
    state.faction_mut(fid).unwrap().mond_control = 1.0;
    assert_eq!(observe_claim(&state, &config, fid), 0.0);
    // ⚠ **运输那一支现在有非零基线**：每个建造区都要常住一套最低可用选装
    //   （`freight::site_standing`）⇒ **楼全建完、只剩船坞在干活**的天体也有「要动的货」
    //   （进口腿）。所以「没有积压」不再等于「运输主张 0」——要看的是**有没有货要动**，
    //   而「一件货都没有」= 首都池也空（`lanes` 的进口腿要求首都能拿得出货）。
    for f in state.factions.iter_mut() {
        f.resources.clear();
    }
    let (war, frt, obs) = role_quotas(&state, &config, fid);
    assert_eq!(obs, 0.0, "学满 ⇒ 观测配额 0");
    assert_eq!(frt, 0.0, "一件货都没有 ⇒ 运输配额 0");
    let fleet = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .count() as f64;
    assert_eq!(war, fleet, "两支都没主张 ⇒ 全军留在战位");
}



/// **行为**：观测舰这一回合的活是 `Dock` 到异常区里的目标天体（跟着天体走 ⇒ 一直待在带里，
/// 而 `sim::mond_presence` 只认「此刻在带内的活舰」）。它**不解除武装**——照常开火。
#[test]
fn an_observer_is_ordered_to_dock_at_a_body_inside_the_anomaly() {
    let (config, mut state) = fresh(42);
    let fid = "中国";
    // 把它的一艘舰钉成**玩家指定的观测舰**（`Player` 的叶 ⇒ AI 不重估，专测行为那一半）。
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid && s.hull > 0.0)
        .expect("中国开局有舰")
        .name
        .clone();
    state
        .control_mut(fid.to_string())
        .unwrap()
        .ship_role
        .insert(ship.clone(), Control::player(ShipRole::Observe));
    assert_eq!(
        state.ship_role(ship.clone()),
        ShipRole::Observe,
        "玩家钉的观测舰"
    );
    let mut rng = crate::prng::Prng::new(7);
    let mut decisions = Vec::new();
    let focus_of: BTreeMap<FactionId, Option<FactionId>> = BTreeMap::new();
    let mut next_building_id = 0;
    let mut haul_steps = BTreeMap::new();
    ai_ship_turn(
        &mut state,
        &config,
        &mut rng,
        &ship,
        &focus_of,
        &mut next_building_id,
        &mut decisions,
        &mut haul_steps,
        &mut crate::model::RoundInputs::default(),
    );
    match state.ship_behavior(ship.clone()) {
        Some(ShipBehavior::Dock { body }) => {
            let depth = sim::dist(state.body_position(&body), [0.0, 0.0]) - config.mond.radius;
            assert!(depth > 0.0, "驻地必须在异常区里（{body} 深 {depth:.2}）");
        }
        other => panic!("观测舰该被派去 Dock 异常区的天体，实为 {other:?}"),
    }
    assert!(
        decisions
            .iter()
            .all(|d| !matches!(d.verdict, ShipVerdict::Engage)),
        "观测舰不追敌（照常开火由 auto_combat 负责，不改写指令）"
    );
}
