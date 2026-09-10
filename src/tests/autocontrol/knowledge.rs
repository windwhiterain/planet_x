//! 观测舰（角色轴第三态）的单元测试：配额、选靶、期望入伙数、优先级、行为。
//!
//! 这些用例是 §11 验收判据的前半段（后半段是世界级的探针：`tests/tech_probe.rs` 里
//! 「至少一家把掌握度推到 1.0」）。分工：
//! * **本文件**：机制本身（尺子、配额、抽签的期望、优先级、指令）；
//! * **探针**：整个世界跑 600 回合之后，这件事到底发生了没有。

use super::*;
use crate::autocontrol::freight::assign_roles;
use crate::autocontrol::tactics::ai_ship_turn;
use crate::config::load_config;
use crate::world::default_state;
use std::collections::BTreeMap;

fn fresh(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// 某势力此刻在观测的舰（按舰名序）——用例里到处要看它。
fn observers(state: &State, fid: &str) -> Vec<String> {
    let mut v: Vec<String> = state
        .ships
        .iter()
        .filter(|s| {
            s.faction_id == fid && s.hull > 0.0 && state.ship_role(s.name.clone()) == ShipRole::Observe
        })
        .map(|s| s.name.clone())
        .collect();
    v.sort();
    v
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
    let mortal = crate::autocontrol::knowledge::body_weight(&config, 0.0, shallow)
        > crate::autocontrol::knowledge::body_weight(&config, 0.0, deep);
    assert!(mortal, "凡人该蹲前沿边上（深处期望收益更低），实为 {:.2} vs {:.2}",
        crate::autocontrol::knowledge::body_weight(&config, 0.0, shallow),
        crate::autocontrol::knowledge::body_weight(&config, 0.0, deep));
    let master = crate::autocontrol::knowledge::body_weight(&config, 1.0, deep)
        > crate::autocontrol::knowledge::body_weight(&config, 1.0, shallow);
    assert!(master, "指哪打哪的势力该往深处去（p 都是 1 ⇒ 收益就是在场价值）");
    // 中间是**连续**的过渡，不是开关：0.9 的势力在深处的收益已经超过凡人。
    assert!(
        crate::autocontrol::knowledge::body_weight(&config, 0.9, deep)
            > crate::autocontrol::knowledge::body_weight(&config, 0.0, deep)
    );
}

/// **配额**：`学满所需的在场强度 ÷ 每艘在目标深度的价值`，且被「舰队的一半」压住。
///
/// 两条上界的意图不同，用例都要钉住：
/// * `mastery_presence` 是**绝对量**（学满的资格）⇒ 人得一直派够，不能随掌握度缩水；
/// * 「一半」是**政策**（观测是副业）⇒ 小势力不会被抽干。
#[test]
fn observer_quota_targets_mastery_and_never_eats_the_whole_fleet() {
    let (config, state) = fresh(42);
    let fid = "中国";
    // 默认世界：中国有 3 艘舰 ⇒ 配额被「一半」压住（12 ÷ 每艘约 1.5 = 8 艘 > 1.5）。
    let quota = crate::autocontrol::knowledge::observer_quota(&state, &config, fid);
    let fleet = state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0).count() as f64;
    assert!(fleet > 0.0, "用例前提：中国开局有舰");
    assert!(
        (quota - 0.5 * fleet).abs() < 1e-9,
        "三艘舰 ⇒ 配额该被「一半」顶住（{quota:.2} vs 一半 {:.2}）",
        0.5 * fleet
    );
    // 舰队变大 ⇒ 配额改由「学满的资格」决定，且**不再随舰队线性增长**。
    let mut big = state.clone();
    for k in 1..=9 {
        for s in state.ships.iter().filter(|s| s.faction_id == fid).cloned().collect::<Vec<_>>() {
            let mut c = s;
            c.name = format!("{}-{k}", c.name);
            big.ships.push(c);
        }
    }
    let big_fleet =
        big.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0).count() as f64;
    let big_quota = crate::autocontrol::knowledge::observer_quota(&big, &config, fid);
    assert!(big_fleet > 20.0, "用例前提：克隆出一支大舰队（{big_fleet}）");
    assert!(big_quota < 0.5 * big_fleet, "大舰队时上界换成「学满的资格」：{big_quota:.2}");
    assert!(big_quota > 0.0, "还没学满 ⇒ 配额必须为正");
    // **已经到顶（棘轮）⇒ 配额 0**：没有东西可学了。这不是断崖，是棘轮的另一面。
    let mut mastered = state.clone();
    mastered.faction_mut(fid).unwrap().mond_control = 1.0;
    assert_eq!(
        crate::autocontrol::knowledge::observer_quota(&mastered, &config, fid),
        0.0,
        "学满之后没有必要再派人观测"
    );
}

/// **期望入伙数 = 缺口**（与集货同一条纪律）。用户裁决的「概率分布而不是硬阈值」在这里的
/// 落实就是这条：不是「每人各掷一次身份」（那样缺口大时会全舰队一起入伙），而是
/// `p = (缺口 + 轮换) × 我的票 ÷ 同侧总票数` ⇒ **平均头数贴着配额**。
#[test]
fn the_observer_headcount_lands_on_the_quota() {
    let (config, base) = fresh(42);
    let fid = "中国";
    // 克隆到一支中等舰队，让配额由「学满的资格」决定（而不是被「一半」顶住）。
    let mut state = base.clone();
    for k in 1..=4 {
        for s in base.ships.iter().filter(|s| s.faction_id == fid).cloned().collect::<Vec<_>>() {
            let mut c = s;
            c.name = format!("{}-{k}", c.name);
            state.ships.push(c);
        }
    }
    let quota = crate::autocontrol::knowledge::observer_quota(&state, &config, fid);
    assert!(quota > 1.0 && quota < 12.0, "用例前提：配额由学满的资格决定（{quota:.2}）");
    // ⚠ 比的是**逐回合的均值**：配额本身会动——每 12 回合按期望在场收益重抽目标天体，
    // 抽到深处（在场价值高）⇒ 每艘顶得上更多 ⇒ 配额低；抽到前沿边上 ⇒ 配额高。拿
    // 某一回合的瞬时值去比 400 回合的均值，比的是两件不同的事。
    let mut sum = 0.0;
    let mut sum_quota = 0.0;
    let rounds = 400;
    for _ in 0..rounds {
        state.round += 1;
        sum_quota += crate::autocontrol::knowledge::observer_quota(&state, &config, fid);
        assign_roles(&mut state, &config);
        sum += observers(&state, fid).len() as f64;
    }
    let mean = sum / rounds as f64;
    let mean_quota = sum_quota / rounds as f64;
    assert!(
        (mean - mean_quota).abs() < 0.5,
        "平均观测头数该贴着配额（逐回合均值 {mean_quota:.2}，实测均值 {mean:.2}）"
    );
    // 轮换：不是把同一批人钉死（岗位任期 ≈ 1 ÷ 0.05 = 20 回合）。
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut last: Vec<String> = Vec::new();
    for _ in 0..120 {
        state.round += 1;
        assign_roles(&mut state, &config);
        let now = observers(&state, fid);
        for n in &now {
            seen.insert(n.clone());
        }
        last = now;
    }
    assert!(
        seen.len() > last.len(),
        "岗位该换手（120 回合里见过 {} 艘，此刻只有 {} 艘）",
        seen.len(),
        last.len()
    );
}

/// **优先级：观测 > 运输 > 战斗**（用户裁决）。同一批舰、同一份积压，观测先挑人，
/// 剩下的才轮得到集货——因为观测是**唯一没有替代品**的角色（运输缺船还能雇人）。
#[test]
fn observers_are_picked_before_haulers() {
    let (config, mut state) = fresh(42);
    let fid = "中国";
    // 一份很大的积压 ⇒ 集货配额很高（4 处货栈 + 中庸倾向 ⇒ 4 条腿）。
    state.depots.clear();
    for b in ["金星", "水星", "火星", "木星"] {
        state.depot_add(fid, b, "碳", 100.0);
    }
    let mut sum_obs = 0.0;
    let mut sum_freight = 0.0;
    let mut sum_quota = 0.0;
    let rounds = 200;
    for _ in 0..rounds {
        state.round += 1;
        sum_quota += crate::autocontrol::knowledge::observer_quota(&state, &config, fid);
        assign_roles(&mut state, &config);
        sum_obs += observers(&state, fid).len() as f64;
        sum_freight += state
            .ships
            .iter()
            .filter(|s| {
                s.faction_id == fid
                    && s.hull > 0.0
                    && state.ship_role(s.name.clone()) == ShipRole::Freight
            })
            .count() as f64;
    }
    let obs = sum_obs / rounds as f64;
    let frt = sum_freight / rounds as f64;
    // 同样比逐回合均值（配额随「重抽目标天体」在动）。
    let quota = sum_quota / rounds as f64;
    let freight_quota = crate::autocontrol::freight::freighter_quota(&state, fid);
    assert!(obs > 0.5, "有观测配额就该真的有人去观测（实测 {obs:.2}）");
    assert!(
        (obs - quota).abs() < 0.6,
        "观测先挑人，且挑满自己的配额（配额 {quota:.2}，实测 {obs:.2}）"
    );
    assert!(frt > 0.5, "剩下的船照常被派去跑运输（实测 {frt:.2}）");
    let fleet = state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0).count() as f64;
    assert!(
        obs + frt <= fleet + 0.01,
        "两种角色互斥（枚举保证），且不该凭空超过舰队规模：{obs:.2} + {frt:.2} vs {fleet}"
    );
    assert!(
        frt <= freight_quota + 1.0,
        "集货拿到的是**剩下的**船，不该超过它自己的配额（{frt:.2} vs {freight_quota:.2}）"
    );
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
    assert_eq!(state.ship_role(ship.clone()), ShipRole::Observe, "玩家钉的观测舰");
    let mut rng = crate::prng::Prng::new(7);
    let mut decisions = Vec::new();
    let focus_of: BTreeMap<FactionId, Option<FactionId>> = BTreeMap::new();
    let mut next_building_id = 0;
    ai_ship_turn(
        &mut state,
        &config,
        &mut rng,
        &ship,
        &focus_of,
        &mut next_building_id,
        &mut decisions,
    );
    let order = state.ship_behavior(ship.clone());
    match order {
        Some(ShipBehavior::Dock { body }) => {
            let depth = sim::dist(state.body_position(&body), [0.0, 0.0]) - config.mond.radius;
            assert!(depth > 0.0, "驻地必须在异常区里（{body} 深 {depth:.2}）");
        }
        other => panic!("观测舰该被派去 Dock 异常区的天体，实为 {other:?}"),
    }
    // 它不是在「打仗」这一档上被派活：决策里没有接战/追击那一类结论。
    assert!(
        decisions.iter().all(|d| !matches!(d.verdict, ShipVerdict::Engage)),
        "观测舰不追敌（照常开火由 auto_combat 负责，不改写指令）"
    );
}
