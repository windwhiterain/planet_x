//! 思潮与忠诚：军事信号（只由事件推出）、战争/经济如何推思潮、外交亲和方向、低忠诚倒戈、娱乐设施拉住远城。
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

/// 军事信号（思潮「和平↔军国」的驱动量）必须**只**由事件历史推出，且**同一现象同分**。
///
/// 这里逐条钉住旧实现的两个真实缺陷：
/// 1. **互杀吞掉战功**：旧口径是「同回合最后一条 `Attack` 的势力」，那要 `state.ship(attacker)`
///    才知道攻击者属于谁——凶手若在本回合也被打沉，它已经不在 `state.ships` 里，于是这次
///    击杀**领不到功**。权威的 `by` 不受影响。
/// 2. **失城方读成了抢城者**：旧口径用 `state.city(city).faction_id` 判「谁丢了城」，而夷平
///    不改归属、同回合稍后的复垦会把它改成新主，于是 −1 记到了**复垦者**头上。
///
/// 另外钉住「`CityDefected`（主路）与 `Revolt`（兜底）必须同分」——它们是同一个触发的两条
/// 分支，旧代码却只给兜底分支扣分。
#[test]
fn military_signal_uses_the_milestones_and_is_branch_agnostic() {
    let d =
        |events: &[GameEvent], fid: &str| military_deltas(events).get(fid).copied().unwrap_or(0.0);

    // 1) 互杀：A 的舰打沉 B 的舰，B 的舰同回合也打沉 A 的舰 → **双方各得一分战功**。
    let killer = |ship: &str, faction: &str| Killer {
        ship: ship.to_string(),
        faction: faction.to_string(),
        weapon: "kinetic".to_string(),
    };
    let mutual = vec![
        GameEvent::ShipDestroyed {
            ship: "乙舰".into(),
            owner: "乙".into(),
            class: "corvette".into(),
            cause: DeathCause::Combat,
            by: Some(killer("甲舰", "甲")),
        },
        GameEvent::ShipDestroyed {
            ship: "甲舰".into(),
            owner: "甲".into(),
            class: "corvette".into(),
            cause: DeathCause::Combat,
            by: Some(killer("乙舰", "乙")),
        },
    ];
    assert_eq!(
        d(&mutual, "甲"),
        0.0,
        "甲沉一舰失一分、击沉一舰得一分，净 0"
    );
    assert_eq!(d(&mutual, "乙"), 0.0, "乙同理——旧口径下会有一方拿不到战功");
    // 单方面被击沉：凶手得分，事主扣分。
    let one_sided = vec![GameEvent::ShipDestroyed {
        ship: "乙舰".into(),
        owner: "乙".into(),
        class: "corvette".into(),
        cause: DeathCause::Combat,
        by: Some(killer("甲舰", "甲")),
    }];
    assert_eq!(d(&one_sided, "甲"), 1.0);
    assert_eq!(d(&one_sided, "乙"), -1.0);

    // 2) 欠费报废：失主扣分，**没有人**领功（不是战功）。
    let rusted = vec![GameEvent::ShipDestroyed {
        ship: "锈舰".into(),
        owner: "丙".into(),
        class: "corvette".into(),
        cause: DeathCause::UpkeepShortfall,
        by: None,
    }];
    assert_eq!(d(&rusted, "丙"), -1.0);
    assert_eq!(d(&rusted, "甲"), 0.0, "欠费报废不该被记成任何人的战功");

    // 3) 城被 A 拆平、同回合被 C 复垦：扣分属于**失城方 B**，复垦者 C 不因此得军事分。
    let razed_then_refounded = vec![
        GameEvent::CityRazed {
            city: "城".into(),
            owner: "乙".into(),
            fallen_to: "甲".into(),
            by_ship: "甲舰".into(),
            damage: 9.0,
            pop_before: 200,
        },
        GameEvent::ColonyFounded {
            city: "城".into(),
            owner: "丙".into(),
            body: "木星".into(),
            seeded_ship_class: "corvette".into(),
            how: FoundingHow::Refounded,
            prev_owner: Some("乙".into()),
        },
    ];
    assert_eq!(
        d(&razed_then_refounded, "乙"),
        -1.0,
        "失城方是乙，不是复垦者"
    );
    assert_eq!(d(&razed_then_refounded, "甲"), 1.0, "拆城方得一分");
    assert_eq!(
        d(&razed_then_refounded, "丙"),
        0.0,
        "复垦是殖民行为，不进军事轴"
    );

    // 4) 活城易主（离心倒戈）必须与叛乱兜底同分。
    let defect = vec![GameEvent::CityDefected {
        city: "城".into(),
        from: "乙".into(),
        to: "甲".into(),
        loyalty: 0.2,
    }];
    assert_eq!(d(&defect, "乙"), -1.0, "失主必须扣分（与 Revolt 兜底同分）");
    assert_eq!(d(&defect, "甲"), 1.0);
    let revolt = vec![GameEvent::Revolt {
        city: "城".into(),
        faction: "乙".into(),
        loyalty: 0.0,
    }];
    assert_eq!(d(&revolt, "乙"), -1.0);

    // 5) 新建城（真·殖民）不进军事轴。
    let founded = vec![GameEvent::ColonyFounded {
        city: "新城".into(),
        owner: "丙".into(),
        body: "地球".into(),
        seeded_ship_class: "corvette".into(),
        how: FoundingHow::NewSite,
        prev_owner: None,
    }];
    assert_eq!(d(&founded, "丙"), 0.0, "殖民归 nature_colony 轴");
}

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
