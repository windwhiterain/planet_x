//! 治理与忠诚的两批用例（⚠ 两处都叫「B1」，说的**不是**同一件事）：
//!
//! 1. **读面批**（`notes/step-intermediates.md` §6 的 B1 批）：把「为什么这座城的忠诚在掉 /
//!    钱花在哪」从**算完就扔**变成读面。规矩是 `pre-post-unify.md` 那三条：观测与过程
//!    **同处一行**、纯追加（不改行为 ⇒ digest 逐字不变）、「这个量在这一档不存在」用中性
//!    缺省或 `Option` **显式**表达。
//! 2. **深空治理**（`tech-system.md` §10 红利表里的 **B1**）：MOND 掌握度把「距离 → 忠诚
//!    衰减」那一项乘掉。用户裁决：「掌握者**不按距离付忠诚衰减**：深处的城不再因为
//!    『离首都太远』而离心、倒戈」。落地是**连续**的一行乘子（`1 − 掌握度`），不是开关：
//!    掌握度每涨一点，深处的离心压力就小一点（凡人 → 指哪打哪是渐变的，见
//!    `sim::step_governance` 的注释）。

use super::*;

/// B1 的核心不变量：**忠诚目标值就是那条忠诚方程的分解**。
///
/// `step_governance` 每回合算出一个目标忠诚 `effective`，再把实际忠诚朝它恢复；这里钉住
/// 分解式本身——`distance + entertainment + 势力行的首都向心项 − 势力行的思潮惩罚`，clamp 后
/// == `effective`——外加一条结构性事实：距离项随离首都的距离**单调不增**。
///
/// ⚠ 后两项按势力算一次，所以它们**只在势力行**（`capital_loyalty_bonus` /
/// `ideology_loyalty_penalty`），城行不重复存：这正是「同一个数只有一个位置」。
#[test]
fn loyalty_target_decomposes_the_loyalty_equation() {
    let (config, mut state) = fresh_world(42);
    let mut sink = RoundSink::default();
    step_governance(&mut state, &config, &mut sink);

    let fid = "中国";
    let cap_pos = state.body_position(&state.capital_body(fid));
    let mut cities: Vec<(CityId, f64)> = state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && !c.razed)
        .map(|c| {
            (
                c.name.clone(),
                dist(state.body_position(&c.body_id), cap_pos),
            )
        })
        .collect();
    assert!(
        cities.len() >= 2,
        "这条守卫要至少两座城才谈得上「随距离单调」"
    );
    cities.sort_by(|a, b| a.1.total_cmp(&b.1));

    // 折进视图：全国项只从**势力行**取（城行没有它们——「同一个数只有一个位置」）。
    let view = observe(&state, &config, &sink);
    let frow = view.factions.get(fid).expect("势力行").clone();
    assert!(
        frow.capital_loyalty_bonus > 0.0,
        "首都在自己的活城上 ⇒ 首都向心项应当为正（拿到 {}）",
        frow.capital_loyalty_bonus
    );

    let mut prev_distance_term = f64::INFINITY;
    for (cid, d) in &cities {
        let t = sink
            .city_loyalty
            .get(cid)
            .unwrap_or_else(|| panic!("{cid} 没有忠诚目标值——捕获漏了这座城"));

        // 1) 勾稽：三项（含势力行那两项）之和 clamp 后就是 `effective`（忠诚朝它恢复的那个值）。
        let sum = t.distance + t.entertainment + frow.capital_loyalty_bonus
            - frow.ideology_loyalty_penalty;
        assert!(
            (t.effective - sum.clamp(0.0, 1.0)).abs() < 1e-12,
            "{cid}: effective={} 与三项之和 {sum} 对不上（clamp 后应为 {}）",
            t.effective,
            sum.clamp(0.0, 1.0)
        );
        assert!((0.0..=1.0).contains(&t.effective), "{cid} 的目标忠诚越界");

        // 2) 城行**不该**重存那两个全国项：它们在势力行上，只有一个位置。
        for (k, v) in [
            ("capital_loyalty_bonus", frow.capital_loyalty_bonus),
            ("ideology_loyalty_penalty", frow.ideology_loyalty_penalty),
        ] {
            assert!(v.is_finite(), "{fid} 的 {k} 应当是有限数");
        }

        // 3) 距离项随距离单调不增——这是「帝国太大管不住」的第一来源。
        assert!(
            t.distance <= prev_distance_term + 1e-12,
            "{cid}: 距首都 {d} AU 的距离项 {} 却比更近的城还高（{prev_distance_term}）",
            t.distance
        );
        prev_distance_term = t.distance;
    }

    // 4) 折进视图以后还是同一个数（「同一个量只有一个位置」）。
    for (cid, _) in &cities {
        let s = sink.city_loyalty.get(cid).expect("上面刚查过");
        let row = &view
            .cities
            .get(cid)
            .expect("视图必须有这座活城")
            .loyalty_target;
        assert_eq!(
            row.effective, s.effective,
            "{cid} 的 effective 折进视图后变了"
        );
        assert_eq!(row.distance, s.distance, "{cid} 的距离项折进视图后变了");
        assert_eq!(
            row.entertainment, s.entertainment,
            "{cid} 的娱乐项折进视图后变了"
        );
    }
}

/// 治理开销的**两个来源要分得开**：`总开销 = (行政 + 娱乐) × 制裁倍率`。
///
/// 只给合计时，「我把娱乐预算拉满、钱却被行政（距离 × 人口超载）吃掉」在读数上不可见——
/// 这正是 B1 要补的那一问。顺带钉住倍率的定义域（≥1）与它「有活城就一定有行政开销」。
#[test]
fn governance_cost_splits_into_admin_and_entertainment() {
    let (config, mut state) = fresh_world(42);
    let mut sink = RoundSink::default();
    step_governance(&mut state, &config, &mut sink);

    let mut checked = 0usize;
    for f in &state.factions {
        let fid = f.name.clone();
        let Some(g) = sink.governance.get(&fid) else {
            continue; // 零城势力：`step_governance` 直接 continue，读面给中性缺省
        };
        let mult = sanction_cost_mult(&state, &config, &fid);
        assert!(
            (g.total - (g.admin + g.entertainment) * mult).abs() < 1e-9,
            "{fid}: 总开销 {} ≠ (行政 {} + 娱乐 {}) × 制裁倍率 {mult}",
            g.total,
            g.admin,
            g.entertainment
        );
        assert!(g.scale >= 1.0, "{fid} 的人口超载倍率不该小于 1");
        assert!(g.ideology_penalty >= 0.0, "{fid} 的思潮惩罚不该是负数");
        assert!(g.capital_bonus >= 0.0, "{fid} 的首都向心项不该是负数");
        if state.cities.iter().any(|c| c.faction_id == fid && !c.razed) {
            assert!(g.admin > 0.0, "{fid} 有活城却没有行政开销");
            checked += 1;
        }
    }
    assert!(checked >= 2, "只检查到 {checked} 个有活城的势力——守卫太空");
}

/// `pre` 面（不推进、喂空 sink 观测）里 B1 那几列必须是**中性缺省**，不是看起来像事实的假数据：
/// 拆分是 0、倍率是 **1**（与 `coverage` 缺省 1.0 同一条约定——0 会被读成「治理能力归零」）、
/// 迁都是「没评估也没迁」（`reviewed=false` + `Option` 全 `None`）、忠诚目标值全 0。
///
/// 为什么值得单钉：缺省值是**读面契约的一半**——`pre`/`post` 同形的代价就是那几个 0 必须
/// 说话算话（见 `pre-post-unify.md` §3 规矩 2）。
#[test]
fn pre_view_has_neutral_b1_defaults() {
    let (config, state) = fresh_world(42);
    let view = view_from_state(&state, &config);
    let row = view.factions.get("中国").expect("视图里必须有中国");

    assert_eq!(row.governance_admin, 0.0);
    assert_eq!(row.governance_entertainment, 0.0);
    assert_eq!(
        row.governance_scale, 1.0,
        "中性缺省是 1.0——0 会被读成「治理能力归零」，那是另一回事"
    );
    assert_eq!(row.ideology_loyalty_penalty, 0.0);

    assert_eq!(row.capital_loyalty_bonus, 0.0, "没跑治理 ⇒ 首都向心项是 0");
    assert_eq!(row.ideology_loyalty_penalty, 0.0);
    // 首都判定是**稀疏数组**：没推进过 ⇒ 空数组（不是「有行但全是 null」）。
    assert!(
        view.decisions.capital.is_empty(),
        "没推进过就不该有首都判定行，实际 {:?}",
        view.decisions.capital
    );

    assert!(!view.cities.is_empty());
    for (cid, c) in &view.cities {
        assert_eq!(
            c.loyalty_target.effective, 0.0,
            "{cid} 的过程量在 pre 里必须是 0"
        );
        assert_eq!(c.loyalty_target.distance, 0.0);
    }
}

/// **A/B 同一个世界，只改一个数**：同一个势力、同一座城、同样的库存，只把
/// `mond_control` 从 0 拨到 1，看那座**深处**的城忠诚往哪走。
///
/// 用例把娱乐预算**钉成 0**（`Player` 的叶）：娱乐加成是另一条独立的忠诚来源，
/// 不钉住它、再喂满库存的话，两条支路会一起把忠诚拉回来，就测不出「距离」这一项了。
/// 钉住之后，忠诚的目标只剩 `1 − loyalty_distance × 距离 × (1 − 掌握度)` ＋首都占比 buff。
#[test]
fn a_mond_master_keeps_a_deep_city_loyal_where_a_mortal_loses_it() {
    let (config, base) = fresh_world(42);
    let fid = "中国";
    // 伊克西翁在柯伊伯带（约 30 AU），而中国的首都在地球一带 ⇒ 距离远超
    // `loyalty_range`，凡人的距离目标必然见底（`1 − 0.045 × 23 < 0` ⇒ 钳到 0）。
    let deep = "伊克西翁";

    let run = |control: f64| -> (f64, bool) {
        let mut state = base.clone();
        stock(&mut state, &config, fid, 5000.0);
        state.faction_mut(fid).unwrap().mond_control = control;
        let cid = state
            .cities
            .iter()
            .find(|c| c.faction_id == fid && !c.razed)
            .expect("中国开局有城")
            .name
            .clone();
        // 把它搬到外缘（治理只看「城的天体 → 首都」的距离），并把娱乐预算钉成 0。
        if let Some(c) = state.city_mut(&cid) {
            c.body_id = deep.to_string();
            c.loyalty = 0.5;
        }
        state
            .control_mut(fid.to_string())
            .unwrap()
            .loyalty_budget
            .insert(cid.clone(), Control::player(0.0));
        for _ in 0..5 {
            step_governance(&mut state, &config, &mut RoundSink::default());
        }
        // 城若已倒戈/夷平 ⇒ 不再属于本势力 ⇒ 忠诚按 0 计（那正是「守不住」的极端形态）。
        let still_ours = state
            .city(&cid)
            .filter(|c| c.faction_id == fid && !c.razed)
            .map(|c| c.loyalty)
            .unwrap_or(0.0);
        (
            still_ours,
            state
                .city(&cid)
                .map(|c| c.faction_id == fid)
                .unwrap_or(false),
        )
    };

    let (mortal, mortal_ours) = run(0.0);
    let (master, master_ours) = run(1.0);

    assert!(
        master > mortal + 0.2,
        "掌握者该守得住深处的城：凡人 {mortal:.3}（还在手里 {mortal_ours}） vs \
         掌握 {master:.3}（还在手里 {master_ours}）"
    );
    assert!(master_ours, "掌握者的深空城不该在这一段里丢");
    assert!(
        master > 0.6,
        "掌握者的深空城该往 1.0 回升（5 回合后实测 {master:.3}）"
    );
    assert!(
        mortal < 0.45,
        "凡人的深空城该往 0 掉（实测 {mortal:.3}）——目标忠诚被距离钳到 0"
    );

    // **连续**：掌握度 0.5 落在两端之间（证明这不是「≥ 某个值才生效」的开关）。
    let (half, _) = run(0.5);
    assert!(
        half > mortal + 0.05 && half < master - 0.05,
        "掌握度一半 ⇒ 忠诚该落在两端之间：凡人 {mortal:.3} < 半 {half:.3} < 掌握 {master:.3}"
    );
}

/// 掌握度**只**动「距离」那一项：它不该顺手改掉库存付不出治理费时的暴跌支路
/// （覆盖率 < 1 ⇒ 忠诚按 `loyalty_penalty` 掉，与距离无关）。
#[test]
fn mastery_does_not_pay_the_governance_bill() {
    let (config, base) = fresh_world(42);
    let fid = "中国";
    let run = |control: f64| -> f64 {
        let mut state = base.clone();
        // 一文不名 ⇒ 覆盖率 0 ⇒ 走「欠费暴跌」那条支路。
        state.faction_mut(fid).unwrap().resources.clear();
        state.faction_mut(fid).unwrap().mond_control = control;
        let cid = state
            .cities
            .iter()
            .find(|c| c.faction_id == fid && !c.razed)
            .expect("中国开局有城")
            .name
            .clone();
        if let Some(c) = state.city_mut(&cid) {
            c.loyalty = 1.0;
        }
        for _ in 0..4 {
            step_governance(&mut state, &config, &mut RoundSink::default());
        }
        state.city(&cid).map(|c| c.loyalty).unwrap_or(0.0)
    };
    let mortal = run(0.0);
    let master = run(1.0);
    assert!(
        (mortal - master).abs() < 1e-9,
        "掌握度买的是「守得住」，不是「管得起」——付不出治理费时两侧该一样掉：\
         凡人 {mortal:.3} vs 掌握 {master:.3}"
    );
}
