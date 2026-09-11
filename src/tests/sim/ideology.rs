//! 思潮与忠诚：军事信号（只由事件推出）、战争/经济如何推思潮、外交亲和方向、低忠诚倒戈、娱乐设施拉住远城。
//!
//! ## 2026-10（第 7 批）：`military_signal_uses_the_milestones_and_is_branch_agnostic` 搬去了 g1
//!
//! 新挂 `--call military_deltas {events}`（纯函数，只吃事件表；事件形状与状态里 `events` 一致，
//! 内部标记 `type`）。原件的六个用例逐条复现：**互杀双方各得一分**（各 0）、单方面 +1/−1、
//! 欠费报废只扣失主、拆城 ±1 而**复垦者不计分**、**活城易主与叛乱兜底同分**、新建城不计分。
//!
//! **留在这里的**：另外三条要**往世界里注入事件**再跑 `step_ideology`
//! （`--call` 是**纯函数**契约，加「可变调用」是设计改动）⇒ 归 §4 内部契约/手工世界。
//!
//! ## 2026-10（第 7 批）：`low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing`
//! 搬去了 g2 **合成场景 · 低忠诚改旗易帜**（4 条判据）
//!
//! 捏三样（都有身份键 ⇒ 直接改档）：旧主思潮推到极端、一个对照势力推到相反极、其余中立，
//! 再把一座城的忠诚压到阈值下。**「倒向谁」不写死**：拿 `--call ideology_similarity` 把
//! 「旧主 × 每个势力」的相似度都算一遍，判据要求倒戈目标就是**相似度最低**的那一个
//! （实测 `0.0` vs 其余 `0.5`）。另加「没被夷平 + 人口/建筑都在」。
//!
//! ⚠ 相似度必须从**场景自己的**投影读（补丁落地后那份）——第一版读了对照局，判据只是
//! **碰巧**还是那一家。
//!
//! ## 2026-10（第 7 批）：`entertainment_holds_a_distant_city` 搬去了 g2 **合成场景 · 重金娱乐拉住远城**（4 条判据）
//!
//! 两臂**只差有没有那份福利预算**（同一座城 = 该势力 `gov_distance` 最大的那座、同样起点
//! 忠诚度、同样满仓国库）：重金臂 `[0.35, 0.39, 0.43, 0.46, 0.49, 0.52]` 逐回合不降，
//! 对照臂 `[0.35, 0.33, 0.32, 0.31, …]` 真的往下走（防空转）。
//! 国库/忠诚度走 `h.scenario(patch=…)`，福利两片叶走 `--apply`——与原件那份 diff 同形。
//!
//! ## 2026-10（第 7 批）：`ideology_similarity_ranges_and_is_monotonic` 搬去了 g1
//!
//! 新挂了 `--call ideology_similarity`（键名与读面 `factions.思潮` 一致）⇒ 判据比原件**更强**：
//! 同 = 1、全对极 = 0、恒在 `[0,1]`，外加**对称**与沿轴**单调**（原件只比了「自己 ≥ 别人」）。

use super::*;

/// 思潮：战争得利把「和平↔军国」推向军国端。
#[test]
fn ideology_military_win_drives_toward_militarism() {
    let (config, mut state) = fresh_world(42);
    let fname = state.factions[2].name.clone(); // 欧盟（开局有舰，且初始偏和平端）
    let my_ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fname)
        .map(|s| s.name.clone())
        .expect("a ship");
    let enemy = state
        .ships
        .iter()
        .find(|s| s.faction_id != fname)
        .map(|s| (s.name.clone(), s.faction_id.clone()))
        .expect("enemy ship");
    let start = state.faction(&fname).unwrap().ideology.peace_military;

    // 注入一回合「战争得利」：我方舰击毁一艘敌舰。击毁归属由 Attack→ShipDestroyed 反推。
    state.events.push(GameEvent::Attack {
        attacker: my_ship.clone(),
        target: enemy.0.clone(),
        damage: 10.0,
        shots: Vec::new(),
    });
    state.events.push(GameEvent::ShipDestroyed {
        ship: enemy.0.clone(),
        owner: enemy.1.clone(),
        class: "corvette".to_string(),
        cause: DeathCause::Combat,
        by: None,
    });
    step_ideology(&mut state, &config, &RoundSink::default());

    let after = state.faction(&fname).unwrap().ideology.peace_military;
    assert!(
        after > start,
        "war victory must push 和平↔军国 toward 军国: start={start} after={after}"
    );
}

/// 思潮：经济转负把「人民↔精英」推向人民端；且所有轴恒可有界、有限。
#[test]
fn ideology_economy_bad_drives_toward_populism_and_stays_bounded() {
    let (config, mut state) = fresh_world(42);
    let fname = state.factions[1].name.clone();
    let start = state.faction(&fname).unwrap().ideology.people_elite;

    // 经济转负：净流 = 产出(0) − 维护(100) − 治理(0) < 0 → 人民（民粹反弹）。
    let mut flow = RoundSink::default();
    flow.upkeep.entry(fname.clone()).or_default().total = 100.0;
    step_ideology(&mut state, &config, &flow);

    let after = state.faction(&fname).unwrap().ideology.people_elite;
    assert!(
        after < start,
        "economic bust must push 人民↔精英 toward 人民: start={start} after={after}"
    );
    // 所有势力的所有轴都应是有界、有限的。
    for f in &state.factions {
        let i = &f.ideology;
        for (k, v) in [
            ("peace_military", i.peace_military),
            ("science_tech", i.science_tech),
            ("people_elite", i.people_elite),
            ("nature_colony", i.nature_colony),
        ] {
            assert!(
                v.is_finite() && (-1.0..=1.0).contains(&v),
                "{k} out of bounds: {v}"
            );
        }
    }
}

/// 思潮相似度影响外交：其它条件相同（同 seed、同 alignment、同起始关系、噪声关闭）下，
/// 思潮越像 → 静息亲和越高 → 关系向更友好靠拢；思潮越对立 → 越向敌对靠拢。
#[test]
fn ideology_similarity_shifts_diplomatic_affinity_directionally() {
    let run = |ideo_a: Ideology, ideo_b: Ideology| -> f64 {
        let (mut config, mut state) = fresh_world(42);
        // 关掉噪声，让关系变化只反映静息亲和的差异（确定性）。
        config.diplomacy.noise = 0.0;
        let a = state.factions[0].name.clone();
        let b = state.factions[1].name.clone();
        {
            let fa = state.faction_mut(&a).unwrap();
            fa.alignment = 0.0; // 隔离 alignment：只留思潮相似度的独立影响
            fa.ideology = ideo_a;
            fa.relations.insert(b.clone(), 0.0);
            let fb = state.faction_mut(&b).unwrap();
            fb.alignment = 0.0;
            fb.ideology = ideo_b;
            fb.relations.insert(a.clone(), 0.0);
        }
        let mut rng = Prng::new(42);
        step_diplomacy(&mut state, &config, &mut rng, &mut RoundSink::default());
        relation(&state, &a, &b)
    };

    // 全同极（相似度=1）vs 全对极（相似度=0）：同 seed、同 alignment、同起始关系，
    // 唯一的差别就是思潮相似度 → 相似的一方关系必须更友好。
    let same_pos = Ideology {
        peace_military: 1.0,
        science_tech: 1.0,
        people_elite: 1.0,
        nature_colony: 1.0,
    };
    let opposite = Ideology {
        peace_military: -1.0,
        science_tech: -1.0,
        people_elite: -1.0,
        nature_colony: -1.0,
    };
    let r_same = run(same_pos, same_pos);
    let r_opp = run(same_pos, opposite);
    assert!(
        r_same > r_opp,
        "similar ideologies must rest friendlier than opposite ones: same={r_same} opp={r_opp}"
    );
}
