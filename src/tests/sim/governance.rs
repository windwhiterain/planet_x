//! **B1 深空治理**的单元测试：MOND 掌握度把「距离 → 忠诚衰减」这一项乘掉。
//!
//! 用户裁决（`tech-system.md` §10 七条 MOND 红利里**只做这一条**）：
//! 「掌握者**不按距离付忠诚衰减**：深处的城不再因为『离首都太远』而离心、倒戈」。
//! 落地是**连续**的一行乘子（`1 − 掌握度`），不是开关：掌握度每涨一点，深处的离心压力
//! 就小一点（凡人 → 指哪打哪是渐变的，见 `sim::step_governance` 的注释）。

use super::*;

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
        (still_ours, state.city(&cid).map(|c| c.faction_id == fid).unwrap_or(false))
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
