//! MOND 知识（科技干线）：**飞船在异常区**是唯一的知识渠道——派进去掌握度就涨，
//! 撤回来它就锈。判据与公式见 `sim::knowledge` 与 `.agents/notes/tech-system.md`。

use super::*;

/// 把某势力所有活舰搬到日心距 `r` 处（正 x 轴）——「在场/不在场」的最短写法。
fn park(state: &mut State, fid: &str, r: f64) {
    let names: Vec<String> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| s.name.clone())
        .collect();
    for n in names {
        if let Some(s) = state.ship_mut(&n) {
            s.position = [r, 0.0];
        }
    }
}

/// 唯一渠道：不在带内的舰**一点也不算**；带内一艘的价值 = `1 + 深度 × depth_weight`。
#[test]
fn presence_comes_only_from_ships_in_the_band() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    park(&mut state, fid, 1.0); // 太阳边上（带内边缘以内）
    assert_eq!(mond_presence(&state, &config, fid), 0.0, "带外的舰不产生知识");
    assert_eq!(mond_target(&config, 0.0), 0.0, "没有在场 ⇒ 目标是 0（道会锈）");

    park(&mut state, fid, 60.0);
    let ships = state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0).count();
    let per_ship = 1.0 + (60.0 - config.mond.radius) * config.mond.knowledge.depth_weight;
    assert!(ships > 0, "用例前提：中国开局有舰");
    assert!(
        (mond_presence(&state, &config, fid) - ships as f64 * per_ship).abs() < 1e-9,
        "在场强度 = Σ(1 + 深度 × 权重)"
    );
    // 深处的一艘 > 浅处的一艘（外缘永远值得派人去）。
    let deep = mond_presence(&state, &config, fid);
    park(&mut state, fid, 29.0);
    assert!(mond_presence(&state, &config, fid) < deep, "同样的舰，浅处观测更不值钱");
}

/// 涨：向「在场强度」决定的目标值靠拢，**指数饱和**（永远不到 1，除非掌握度到顶）。
#[test]
fn control_climbs_toward_the_presence_target_and_stops_there() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    park(&mut state, fid, 60.0);
    let target = mond_target(&config, mond_presence(&state, &config, fid));
    assert!(target > 0.0 && target < 1.0, "目标值必须严格在 (0,1)：饱和而非断崖");

    let before = state.faction(fid).unwrap().mond_control;
    assert_eq!(before, 0.0, "用例前提：凡人从 0 起");
    for _ in 0..60 {
        step_knowledge(&mut state, &config);
    }
    let mid = state.faction(fid).unwrap().mond_control;
    assert!(mid > before, "在场观测必须让掌握度上升（{before} → {mid}）");
    for _ in 0..400 {
        step_knowledge(&mut state, &config);
    }
    let settled = state.faction(fid).unwrap().mond_control;
    assert!(
        (settled - target).abs() < 1e-3,
        "长期停留在场 ⇒ 收敛到目标值 {target}，实际 {settled}"
    );
    assert!(settled <= 1.0, "掌握度必须钳在 [0,1]");
}

/// 锈：把舰队撤出异常区之后，掌握度按同一个速率回落——**知识是活量，不是一次性解锁**。
#[test]
fn control_rusts_back_when_the_fleet_leaves() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    park(&mut state, fid, 60.0);
    for _ in 0..400 {
        step_knowledge(&mut state, &config);
    }
    let learned = state.faction(fid).unwrap().mond_control;
    assert!(learned > 0.5, "先得学到东西，用例才有意义（实得 {learned}）");

    park(&mut state, fid, 1.0); // 撤回来
    for _ in 0..100 {
        step_knowledge(&mut state, &config);
    }
    let rusted = state.faction(fid).unwrap().mond_control;
    assert!(rusted < learned, "不在场就必须锈（{learned} → {rusted}）");
    for _ in 0..2000 {
        step_knowledge(&mut state, &config);
    }
    assert!(
        state.faction(fid).unwrap().mond_control < 0.01,
        "长期不在场 ⇒ 回到凡人（实测 {}）",
        state.faction(fid).unwrap().mond_control
    );
}

/// 开局打点与前沿读数：崇拜教 1.0（指哪打哪 ⇒ 前沿无穷），凡人 0 ⇒ 前沿 30 AU。
#[test]
fn initial_mastery_comes_from_config_and_frontier_reads_it() {
    let (config, state) = fresh_world(42);
    let cult = state.faction("行星X崇拜教").unwrap().mond_control;
    assert_eq!(cult, 1.0, "config.mond.initial 里的势力开局即掌握");
    assert!(mond_frontier(&config, cult).is_infinite(), "掌握度 1 ⇒ 前沿无穷（指哪打哪）");
    let mortal = mond_frontier(&config, 0.0);
    let expected = config.mond.radius + config.combat.arrival_eps / config.mond.drift_per_au;
    assert!((mortal - expected).abs() < 1e-9, "凡人前沿 = radius + eps/drift = {expected}");
    assert!(mond_frontier(&config, 0.5) > mortal, "掌握度越高，前沿越往外挪");
}
