//! 治理与忠诚的两批用例（⚠ 两处都叫「B1」，说的**不是**同一件事）：
//!
//! ## 2026-10（第 7 批）：`a_mond_master_keeps_a_deep_city_loyal_where_a_mortal_loses_it`
//! 搬去了 g2 **合成场景 · 治理**（3 条判据）
//!
//! 两臂**只差 `MOND 掌握度`**（势力状态字段，直接 `patch`）：把一座城搬到**离首都最远**的天体、
//! 娱乐预算钉 0。凡人 0.50→**0.37**（逐回合下滑）、掌握者 0.50→**0.62**（回升、差 0.25 > 0.2、
//! 城没丢）。「深空天体」取**离首都最远**的那个，不写死地名。
//!
//! ⚠ 同族的 `mastery_does_not_pay_the_governance_bill` **没搬**：它要「覆盖率 0 ⇒ 欠费暴跌支路」，
//! 而**完整回合里够不到**——产出先到账，覆盖率恒 > 0（实测国库清零后忠诚仍稳在 1.0）。
//! 那是 `step_governance` 的单元测。
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

/// **P1-5：Player 的 `welfare_budget` 叶按「总市场价值」读，不是逐资源支付向量**。
///
/// 只在叶里写「碳」，实际治理/娱乐支付仍从全部库存按价值比例扣；因此铁和碳的
/// 支付比例必须几乎相同，而不是「写了碳就只扣碳」。
#[test]
fn player_welfare_budget_is_a_total_value_not_a_payment_vector() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    for f in state.factions.iter_mut() {
        f.resources.clear();
    }
    if let Some(f) = state.faction_mut(fid) {
        f.resources.insert("铁".to_string(), 1_000_000.0);
        f.resources.insert("碳".to_string(), 1_000_000.0);
    }
    {
        let c = state.control_mut(fid.to_string()).expect("中国有 control");
        c.welfare_budget.clear();
        // 叶里只写碳；若它是「支付向量」语义，铁应该一格不掉。
        c.welfare_budget
            .insert("碳".to_string(), Control::player(1.0));
    }
    let before = |s: &State, rt: &str| {
        s.faction(fid)
            .and_then(|f| f.resources.get(rt))
            .copied()
            .unwrap_or(0.0)
    };
    let (iron0, carbon0) = (before(&state, "铁"), before(&state, "碳"));
    step_governance(&mut state, &config, &mut RoundSink::default());
    let (iron1, carbon1) = (before(&state, "铁"), before(&state, "碳"));
    let iron_paid = (iron0 - iron1) / iron0;
    let carbon_paid = (carbon0 - carbon1) / carbon0;
    assert!(
        iron_paid > 0.0 && carbon_paid > 0.0,
        "这一局真的发生了治理/娱乐支付（铁 {iron_paid:.3} / 碳 {carbon_paid:.3}）——守卫不能空转"
    );
    assert!(
        (iron_paid - carbon_paid).abs() < 1e-9,
        "实际支付必须按全部库存价值比例，而不是按 welfare 叶的资源组分：\
         铁 {iron_paid:.6} vs 碳 {carbon_paid:.6}"
    );
}
