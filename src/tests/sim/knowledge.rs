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
    // 停在浅处（深 2 AU ⇒ 每艘 1.5 的在场强度）：**故意够不到 `mastery_presence`**,
    // 这样目标是 < 1 的饱和值，用例测的才是「爬向目标」而不是「学满 snap」。
    park(&mut state, fid, config.mond.radius + 2.0);
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
    park(&mut state, fid, config.mond.radius + 2.0);
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

/// 开局打点与前沿读数：**现在没人白拿**（`config.mond.initial` 是空表），
/// 但机制仍在——表里写了名字就照给。前沿：凡人 30 AU、掌握 1.0 无穷。
#[test]
fn initial_mastery_comes_from_config_and_frontier_reads_it() {
    let (config, state) = fresh_world(42);
    for f in &state.factions {
        assert_eq!(f.mond_control, 0.0, "{} 不该白拿 MOND（用户裁决：特权删掉）", f.name);
    }
    // 机制还在：表里写一个名字就照给（用一份改过的 config，不动世界）。
    let mut loaded = config.clone();
    loaded.mond.initial.insert("行星X崇拜教".to_string(), 1.0);
    let seeded = default_state(&loaded, 42);
    assert_eq!(seeded.faction("行星X崇拜教").unwrap().mond_control, 1.0);

    assert!(mond_frontier(&config, 1.0).is_infinite(), "掌握度 1 ⇒ 前沿无穷（指哪打哪）");
    let mortal = mond_frontier(&config, 0.0);
    let expected = config.mond.radius + config.combat.arrival_eps / config.mond.drift_per_au;
    assert!((mortal - expected).abs() < 1e-9, "凡人前沿 = radius + eps/drift = {expected}");
    assert!(mond_frontier(&config, 0.5) > mortal, "掌握度越高，前沿越往外挪");
}

/// **棘轮**（用户裁决）：「一旦达到 1.0 就不会下降」——到顶之后把舰队撤回太阳系，
/// 掌握度也**一个字节都不掉**。1.0 之下才锈（见上一条用例）。
#[test]
fn mastery_at_one_is_a_ratchet_and_never_rusts() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    state.faction_mut(fid).unwrap().mond_control = 1.0;
    park(&mut state, fid, 1.0); // 舰队在太阳边上，一点都不在带内
    for _ in 0..2000 {
        step_knowledge(&mut state, &config);
    }
    assert_eq!(
        state.faction(fid).unwrap().mond_control,
        1.0,
        "到顶就是永久的：不在场也不许锈"
    );
}

/// **够得着的顶 + 学满即到顶 + 棘轮到顶之后仍然成立**：指数饱和永远到不了 1，
/// 所以「学满」由 `mastery_presence` 定义——在场强度跨过它 ⇒ **直接到 1.0**。
#[test]
fn a_real_deep_presence_reaches_the_top() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    let k = &config.mond.knowledge;
    assert!(
        mond_target(&config, k.mastery_presence - 0.1) < 1.0,
        "差一点就是差的：目标仍 < 1（饱和曲线的延续，不是开关）"
    );
    assert_eq!(mond_target(&config, k.mastery_presence), 1.0, "跨过它就学满");

    // 一支真的常驻深空的编队：把舰摊到深处（深 40 ⇒ 每艘 11 的在场强度）。
    let names: Vec<String> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| s.name.clone())
        .collect();
    assert!(!names.is_empty(), "用例前提：中国开局有舰");
    let deep = config.mond.radius + 40.0;
    for n in &names {
        if let Some(s) = state.ship_mut(n) {
            s.position = [deep, 0.0];
        }
    }
    assert!(
        mond_presence(&state, &config, fid) >= k.mastery_presence,
        "前提：这套阵形的在场强度要够（实测 {}）",
        mond_presence(&state, &config, fid)
    );
    step_knowledge(&mut state, &config);
    let one_round = state.faction(fid).unwrap().mond_control;
    assert!(
        (one_round - 1.0 / k.mastery_rounds as f64).abs() < 1e-12,
        "够格的**一回合**只走一步（{one_round}）——一回合的巧合不该换来永久垄断"
    );
    for _ in 1..k.mastery_rounds {
        step_knowledge(&mut state, &config);
    }
    assert_eq!(
        state.faction(fid).unwrap().mond_control,
        1.0,
        "{} 个够格的回合爬满 ⇒ 正好到顶",
        k.mastery_rounds
    );

    // 到顶之后就算把舰队全部撤回太阳系，也一个字节都不掉（棘轮）。
    park(&mut state, fid, 1.0);
    for _ in 0..2000 {
        step_knowledge(&mut state, &config);
    }
    assert_eq!(state.faction(fid).unwrap().mond_control, 1.0, "到顶是永久的");
}
