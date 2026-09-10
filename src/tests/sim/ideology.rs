//! 思潮与忠诚：军事信号（只由事件推出）、战争/经济如何推思潮、相似度性质与外交亲和方向、低忠诚倒戈、娱乐设施拉住远城。

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
    let d = |events: &[GameEvent], fid: &str| military_deltas(events).get(fid).copied().unwrap_or(0.0);

    // 1) 互杀：A 的舰打沉 B 的舰，B 的舰同回合也打沉 A 的舰 → **双方各得一分战功**。
    let killer = |ship: &str, faction: &str| Killer {
        ship: ship.to_string(), faction: faction.to_string(), weapon: "kinetic".to_string(),
    };
    let mutual = vec![
        GameEvent::ShipDestroyed {
            ship: "乙舰".into(), owner: "乙".into(), class: "corvette".into(),
            cause: DeathCause::Combat, by: Some(killer("甲舰", "甲")),
        },
        GameEvent::ShipDestroyed {
            ship: "甲舰".into(), owner: "甲".into(), class: "corvette".into(),
            cause: DeathCause::Combat, by: Some(killer("乙舰", "乙")),
        },
    ];
    assert_eq!(d(&mutual, "甲"), 0.0, "甲沉一舰失一分、击沉一舰得一分，净 0");
    assert_eq!(d(&mutual, "乙"), 0.0, "乙同理——旧口径下会有一方拿不到战功");
    // 单方面被击沉：凶手得分，事主扣分。
    let one_sided = vec![GameEvent::ShipDestroyed {
        ship: "乙舰".into(), owner: "乙".into(), class: "corvette".into(),
        cause: DeathCause::Combat, by: Some(killer("甲舰", "甲")),
    }];
    assert_eq!(d(&one_sided, "甲"), 1.0);
    assert_eq!(d(&one_sided, "乙"), -1.0);

    // 2) 欠费报废：失主扣分，**没有人**领功（不是战功）。
    let rusted = vec![GameEvent::ShipDestroyed {
        ship: "锈舰".into(), owner: "丙".into(), class: "corvette".into(),
        cause: DeathCause::UpkeepShortfall, by: None,
    }];
    assert_eq!(d(&rusted, "丙"), -1.0);
    assert_eq!(d(&rusted, "甲"), 0.0, "欠费报废不该被记成任何人的战功");

    // 3) 城被 A 拆平、同回合被 C 复垦：扣分属于**失城方 B**，复垦者 C 不因此得军事分。
    let razed_then_refounded = vec![
        GameEvent::CityRazed {
            city: "城".into(), owner: "乙".into(), fallen_to: "甲".into(),
            by_ship: "甲舰".into(), damage: 9.0, pop_before: 200,
        },
        GameEvent::ColonyFounded {
            city: "城".into(), owner: "丙".into(), body: "木星".into(),
            seeded_ship_class: "corvette".into(), how: FoundingHow::Refounded,
            prev_owner: Some("乙".into()),
        },
    ];
    assert_eq!(d(&razed_then_refounded, "乙"), -1.0, "失城方是乙，不是复垦者");
    assert_eq!(d(&razed_then_refounded, "甲"), 1.0, "拆城方得一分");
    assert_eq!(d(&razed_then_refounded, "丙"), 0.0, "复垦是殖民行为，不进军事轴");

    // 4) 活城易主（离心倒戈）必须与叛乱兜底同分。
    let defect = vec![GameEvent::CityDefected {
        city: "城".into(), from: "乙".into(), to: "甲".into(), loyalty: 0.2,
    }];
    assert_eq!(d(&defect, "乙"), -1.0, "失主必须扣分（与 Revolt 兜底同分）");
    assert_eq!(d(&defect, "甲"), 1.0);
    let revolt = vec![GameEvent::Revolt { city: "城".into(), faction: "乙".into(), loyalty: 0.0 }];
    assert_eq!(d(&revolt, "乙"), -1.0);

    // 5) 新建城（真·殖民）不进军事轴。
    let founded = vec![GameEvent::ColonyFounded {
        city: "新城".into(), owner: "丙".into(), body: "地球".into(),
        seeded_ship_class: "corvette".into(), how: FoundingHow::NewSite, prev_owner: None,
    }];
    assert_eq!(d(&founded, "丙"), 0.0, "殖民归 nature_colony 轴");
}

/// 娱乐/福利预算（忠诚度）：一座远离首都的城市，其距离目标忠诚度本应很低；但若
/// 治理势力投入足够的娱乐预算，忠诚度仍能维持/回升，而非立刻爆发离心叛乱。
#[test]
fn entertainment_holds_a_distant_city() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    // 深口袋：让星系矿业(5)付得起治理 + 娱乐开销，覆盖率=1。
    if let Some(f) = state.faction_mut("星系矿业") {
        for k in [
            "铁", "碳", "硅", "水冰", "铀", "铂", "金",
            "氦-3", "钍", "氢", "甲烷",
        ] {
            f.resources.insert(k.to_string(), 100_000.0);
        }
    }
    // 妊神星转运站 (city 19, body 15) 远离矿业首都(泰坦, body 9)，距离目标忠诚度≈0。
    let city19 = state.cities[19].name.clone();
    if let Some(c) = state.city_mut(&city19) {
        c.loyalty = 0.35; // 略高于叛变阈值，但本应继续下滑。
    }
    let loy0 = state.city(&city19).map(|c| c.loyalty).unwrap();
    // 重金投入该城娱乐预算（Player 覆盖）。
    let diff = serde_json::json!({
        "control": [{"faction_id": "星系矿业", "loyalty_budget": [{"city": city19.clone(), "value": 500.0, "mode": "Player"}]}]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("apply loyalty budget");

    advance(&mut state, &config, &mut rng);

    let loy1 = state.city(&city19).map(|c| c.loyalty).unwrap_or(0.0);
    assert!(
        loy1 >= loy0,
        "heavy entertainment funding should keep a distant city loyal (started {loy0}, now {loy1})"
    );
    assert_eq!(
        state.city(&city19).map(|c| c.razed),
        Some(false),
        "a well-funded distant city must not revolt"
    );
}

/// 离心「改旗易帜」：低忠诚城市不再被夷为荒地，而是倒戈到**思潮与旧主最对立**的势力，
/// 城市连同其人口/建筑/控制面一起易主（旧主失去一城、新主获得一城）——这是给旁观/
/// 小势力接盘城市、避免「永久 1 城旁观者」的机制。
#[test]
fn low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // 珠三角 (city 1) 属中国，位于其首都(地球)上——但把忠诚压到叛变阈值之下。
    let city = state.cities[1].name.clone();
    let owner = "中国".to_string();

    // 中国 → 极端（军国+技术+精英+殖民），无国界科学组织 → 相反极，其余全中立。
    // 于是无国界科学组织与中国的思潮距离 = 8（唯一最大），倒戈目标唯一确定。
    let extreme = Ideology { peace_military: 1.0, science_tech: 1.0, people_elite: 1.0, nature_colony: 1.0 };
    let oppose = Ideology { peace_military: -1.0, science_tech: -1.0, people_elite: -1.0, nature_colony: -1.0 };
    if let Some(f) = state.faction_mut("中国") {
        f.ideology = extreme;
    }
    if let Some(f) = state.faction_mut("无国界科学组织") {
        f.ideology = oppose;
    }
    for f in &mut state.factions {
        if f.name != "中国" && f.name != "无国界科学组织" {
            f.ideology = Ideology::default();
        }
    }
    // 忠诚压到叛变阈值之下（0.30）。
    if let Some(c) = state.city_mut(&city) {
        c.loyalty = 0.05;
    }
    let pop_before = state.city(&city).map(|c| c.population).unwrap_or(0);
    let buildings_before = state.city(&city).map(|c| c.buildings.len()).unwrap_or(0);

    advance(&mut state, &config, &mut rng);

    let c = state.city(&city).expect("defected city must survive (not razed)");
    assert_eq!(
        c.faction_id, "无国界科学组织",
        "low-loyalty city must defect to the most ideologically-opposed faction"
    );
    assert!(!c.razed, "defected city must not be razed to blank");
    assert_eq!(c.population, pop_before, "defected city keeps its population");
    assert_eq!(c.buildings.len(), buildings_before, "defected city keeps its buildings");
    // 忠诚在倒戈时被重置为满，随后同回合新主的治理会重新计量；断言它仍高于叛变阈值，
    // 证明这次倒戈给了城市一个「新开始」（没有立刻又叛变/再被夷平）。
    assert!(
        c.loyalty > 0.05,
        "defected city must get a fresh loyalty start (was 0.05, now {}), not stay near zero",
        c.loyalty
    );

    // 事件必须是 CityDefected（旧主→新主），不是 Revolt。
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::CityDefected { city: cid, from, to, .. }
                if *cid == city && *from == owner && *to == "无国界科学组织"
        )),
        "expected a CityDefected event, got {:?}",
        state.events
    );

    // 控制转移：新主(无国界科学组织)的控制面应接管这座城（invest/build 权重按 (城,建筑) 迁入）。
    if let Some(n) = state.control("无国界科学组织".to_string()) {
        let owned_build_keys: bool = state
            .city(&city)
            .map(|c| c.buildings.iter().any(|b| n.build_weights.contains_key(&(city.clone(), b.id))))
            .unwrap_or(false);
        assert!(
            n.invest_weights.keys().any(|(cid, _)| cid == &city) || n.build_weights.keys().any(|(cid, _)| cid == &city),
            "new owner control must include the defected city's buildings"
        );
        let _ = owned_build_keys;
    }
}

/// 思潮：战争得利把「和平↔军国」推向军国端。
#[test]
fn ideology_military_win_drives_toward_militarism() {
    let (config, mut state) = fresh_world(42);
    let fname = state.factions[2].name.clone(); // 欧盟（开局有舰，且初始偏和平端）
    let my_ship = state.ships.iter().find(|s| s.faction_id == fname).map(|s| s.name.clone()).expect("a ship");
    let enemy = state.ships.iter().find(|s| s.faction_id != fname).map(|s| (s.name.clone(), s.faction_id.clone())).expect("enemy ship");
    let start = state.faction(&fname).unwrap().ideology.peace_military;

    // 注入一回合「战争得利」：我方舰击毁一艘敌舰。击毁归属由 Attack→ShipDestroyed 反推。
    state.events.push(GameEvent::Attack { attacker: my_ship.clone(), target: enemy.0.clone(), damage: 10.0 });
    state.events.push(GameEvent::ShipDestroyed {
        ship: enemy.0.clone(),
        owner: enemy.1.clone(),
        class: "corvette".to_string(),
        cause: DeathCause::Combat,
        by: None,
    });
    step_ideology(&mut state, &config, &RoundFlow::default());

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
    let mut flow = RoundFlow::default();
    flow.upkeep.insert(fname.clone(), 100.0);
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
            assert!(v.is_finite() && (-1.0..=1.0).contains(&v), "{k} out of bounds: {v}");
        }
    }
}

/// 思潮相似度函数：同=1，全对极=0，中庸=0.5；单调随轴距离下降。
#[test]
fn ideology_similarity_ranges_and_is_monotonic() {
    let a = Ideology { peace_military: 0.5, science_tech: -0.3, people_elite: 0.2, nature_colony: 0.4 };
    let b = Ideology { peace_military: -0.5, science_tech: 0.3, people_elite: -0.2, nature_colony: -0.4 };
    let same = Ideology { peace_military: 0.5, science_tech: -0.3, people_elite: 0.2, nature_colony: 0.4 };
    assert_eq!(ideology_similarity(&a, &same), 1.0, "identical ideologies have unit similarity");
    assert!(ideology_similarity(&a, &a) >= ideology_similarity(&a, &b), "similarity is monotonic in distance");
    assert!((0.0..=1.0).contains(&ideology_similarity(&a, &b)));
    assert_eq!(ideology_similarity(&a, &a), 1.0);
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
        step_diplomacy(&mut state, &config, &mut rng);
        relation(&state, &a, &b)
    };

    // 全同极（相似度=1）vs 全对极（相似度=0）：同 seed、同 alignment、同起始关系，
    // 唯一的差别就是思潮相似度 → 相似的一方关系必须更友好。
    let same_pos = Ideology { peace_military: 1.0, science_tech: 1.0, people_elite: 1.0, nature_colony: 1.0 };
    let opposite = Ideology { peace_military: -1.0, science_tech: -1.0, people_elite: -1.0, nature_colony: -1.0 };
    let r_same = run(same_pos, same_pos);
    let r_opp = run(same_pos, opposite);
    assert!(
        r_same > r_opp,
        "similar ideologies must rest friendlier than opposite ones: same={r_same} opp={r_opp}"
    );
}
