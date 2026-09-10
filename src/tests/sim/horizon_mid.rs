//! **中档（T2，49–480 回合）**的 sim 行为用例。
//!
//! 400 回合：「同回合复垦」「出厂风格」；60 回合：编年史 / 参与者 / 战痕地板。
//! 它们要的**模拟时长就是判据本身**，所以不进快档——默认 `cargo nextest run` 不选它们，
//! `cargo nextest run -P mid` 选。

use super::*;

/// **同回合抵消不变量（复垦侧）**。
///
/// 一座城在本回合被拆平之后，**不该被它自己的旧主在本回合复垦**：那对事件对归属的净效果是
/// A→A（只剩人口/建筑被重置），却照样记 `city_razed` + `colony_founded` + `ship_spawned`
/// 三条事件，还白送一艘种子舰——并把它钉成「拆平→复垦→再拆平」的极限环。
///
/// 实测 seed 7 @200 回合，修正前：137 次拆平里 **93 次（68%）** 是这种同回合自我复垦，
/// `水星熔炉基地` 一座城循环 **22 次**、被拆平 32 次；修正后 0 次，该城不再出现在「被拆平
/// 最多」的前五，事件总量 2273 → 1964。
///
/// 这条守卫**必须非空**：局里要真的发生过拆平，否则断言就是空转。删除 `step_resurgence`
/// （D5）之后，同回合复垦只剩「殖民舰恰好当回合抵达」这一条路径，**拆平本身也变少了**
/// （120 回合只剩 13 次）——所以把视野拉到 400 回合，让样本重新够用。
///
/// ⚠ 视野改成**多个种子合计**（而不是只跑 seed 7）：风格轴 + 设计图两个执行者落地之后，
/// seed 7 这条轨迹安静下来了（400 回合 10 次拆平 / 4 艘舰，同一个种子上基线是 57 次 /
/// 33 艘）——**非空这条要求因此不能只押在一个种子上**（那种世界是怎么变的，记在
/// `.agents/notes/control-live-layers.md` §15，是待上层裁决的平衡项，不是这里放宽判据）。
/// 不变式本身对**每一个**种子每一回合都照旧检查，只是样本从三个种子里凑。
#[test]
fn a_city_razed_this_round_is_not_refounded_by_its_own_loser_this_round() {
    let config = load_config();
    let mut razings = 0usize;
    for seed in [1u64, 7, 42] {
        let mut state = default_state(&config, seed);
        let mut rng = crate::prng::Prng::new(seed);
        for _ in 0..400 {
            advance(&mut state, &config, &mut rng);
            // 同一个回合里按事件顺序扫：`city_razed` 由 step_military 发，`colony_founded` 也由
            // step_military 里的殖民路径发（拆平在前、复垦在后），正是要抓的顺序。
            let mut razed: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for e in &state.events {
                match e {
                    GameEvent::CityRazed { city, owner, .. } => {
                        razed.insert(city.clone(), owner.clone());
                        razings += 1;
                    }
                    GameEvent::ColonyFounded { city, owner, .. } => {
                        if let Some(loser) = razed.get(city) {
                            assert_ne!(
                                loser, owner,
                                "seed {seed} 第 {} 回合：{city} 被 {loser} 丢掉后又被**同一个势力**复垦——\
                                 一对净效果为零的事件（拆平在同回合被自己抹掉）",
                                state.round
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    assert!(
        razings >= 20,
        "三个种子各 400 回合只发生 {razings} 次拆平，样本太小，守卫会空转"
    );
}

/// 功能性验证：长局里确实会出现「定制化」舰（资源→组件选择真的被 AI 执行）。
///
/// ⚠ **口径 = 「整局里出现过」，而不是「400 回合末还剩着」**（M2 之后改的）。
/// 原来数的是**末回合快照**，于是这条非空守卫押在一个轨迹事实上：世界是混沌的
/// （任何一处机制改动都会重掷整条轨迹），而舰队在长局里会被打光。实测（400 回合 × seed
/// 7/42，同一个探针在两条树上各跑一次）：
///   * `main`（`23bdb25`）：出厂 206 / 230 条，末回合活舰 **0 / 27**；
///   * 本分支（MOND 掌握度连续化 + 飞船在场渠道）：出厂 105 / 92 条，末回合活舰 **0 / 0**。
/// 两条树上**机制都在正常工作**（出厂的舰基本全都带组件：100/105、87/92），
/// 差别只是「末回合那片场地上还剩几条舰」。所以判据改成**累计**：
/// 只要整局里有任何一个回合存在过「装了组件的活舰」，AI 的选装就被验证过了——
/// 这个口径比原来**更不容易空转**（末回合快照会随轨迹归零，累计不会），
/// 而它检验的仍然是同一件事。
#[test]
fn long_run_produces_customized_ships() {
    let config = load_config();
    let mut customized = 0usize;
    for seed in [7u64, 42] {
        let mut state = default_state(&config, seed);
        let mut rng = Prng::new(seed);
        for _ in 0..400u32 {
            advance(&mut state, &config, &mut rng);
            customized += state
                .ships
                .iter()
                .filter(|s| s.hull > 0.0 && !s.components.is_empty())
                .count();
        }
    }
    assert!(
        customized > 0,
        "customized (component-fitted) ships should appear over a long run, got {customized}"
    );
}

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
