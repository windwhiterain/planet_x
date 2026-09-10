//! 治理与忠诚：B1 批——把「为什么这座城的忠诚在掉 / 钱花在哪」从**算完就扔**变成读面。
//!
//! 清单与批次见 `.agents/notes/step-intermediates.md` §6（B1）。这一批的规矩是
//! `pre-post-unify.md` 那三条：观测与过程**同处一行**、纯追加（不改行为 ⇒ digest 逐字不变）、
//! 「这个量在这一档不存在」用中性缺省或 `Option` **显式**表达。

use super::*;

/// B1 的核心不变量：**忠诚目标值就是那条忠诚方程的分解**。
///
/// `step_governance` 每回合算出一个目标忠诚 `effective`，再把实际忠诚朝它恢复；这里钉住
/// 分解式本身（四项之和 clamp 后 == `effective`），外加两条结构性事实：首都向心项与思潮惩罚
/// 是**全国同值**，距离项随离首都的距离**单调不增**。
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
        .map(|c| (c.name.clone(), dist(state.body_position(&c.body_id), cap_pos)))
        .collect();
    assert!(cities.len() >= 2, "这条守卫要至少两座城才谈得上「随距离单调」");
    cities.sort_by(|a, b| a.1.total_cmp(&b.1));

    let mut prev_distance_term = f64::INFINITY;
    let mut national: Option<(f64, f64)> = None;
    for (cid, d) in &cities {
        let t = sink
            .city_loyalty
            .get(cid)
            .unwrap_or_else(|| panic!("{cid} 没有忠诚目标值——捕获漏了这座城"));

        // 1) 勾稽：四项之和 clamp 后就是 `effective`（忠诚朝它恢复的那个值）。
        let sum = t.distance + t.entertainment + t.capital_share - t.ideology_penalty;
        assert!(
            (t.effective - sum.clamp(0.0, 1.0)).abs() < 1e-12,
            "{cid}: effective={} 与四项之和 {sum} 对不上（clamp 后应为 {}）",
            t.effective,
            sum.clamp(0.0, 1.0)
        );
        assert!((0.0..=1.0).contains(&t.effective), "{cid} 的目标忠诚越界");

        // 2) 全国同值：首都向心项与思潮惩罚按**势力**算，每座城读到的必须是同一个数。
        match national {
            None => national = Some((t.capital_share, t.ideology_penalty)),
            Some((cs, ip)) => {
                assert_eq!(t.capital_share, cs, "{cid} 的首都向心项与同势力其它城不一致");
                assert_eq!(t.ideology_penalty, ip, "{cid} 的思潮惩罚与同势力其它城不一致");
            }
        }

        // 3) 距离项随距离单调不增——这是「帝国太大管不住」的第一来源。
        assert!(
            t.distance <= prev_distance_term + 1e-12,
            "{cid}: 距首都 {d} AU 的距离项 {} 却比更近的城还高（{prev_distance_term}）",
            t.distance
        );
        prev_distance_term = t.distance;
    }

    // 4) 折进视图以后还是同一个数（「同一个量只有一个位置」）：`observe` 拿同一个 sink 折一遍。
    let view = observe(&state, &config, &sink);
    for (cid, _) in &cities {
        let s = sink.city_loyalty.get(cid).expect("上面刚查过");
        let row = &view.cities.get(cid).expect("视图必须有这座活城").loyalty_target;
        assert_eq!(row.effective, s.effective, "{cid} 的 effective 折进视图后变了");
        assert_eq!(row.distance, s.distance, "{cid} 的距离项折进视图后变了");
        assert_eq!(row.ideology_penalty, s.ideology_penalty, "{cid} 的思潮项折进视图后变了");
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

    assert!(!row.capital.reviewed, "没推进过就不该说「评估过」");
    assert!(row.capital.candidate.is_none(), "没评估 ⇒ 候选是 None，不是编一个城名");
    assert!(row.capital.current_cost.is_none(), "没评估 ⇒ 成本是 None，不是 0");
    assert!(row.capital.candidate_cost.is_none());
    assert!(row.capital.relocated_from.is_none() && row.capital.relocated_to.is_none());
    assert_eq!(row.capital.relocate_loyalty_cost, 0.0);

    assert!(!view.cities.is_empty());
    for (cid, c) in &view.cities {
        assert_eq!(c.loyalty_target.effective, 0.0, "{cid} 的过程量在 pre 里必须是 0");
        assert_eq!(c.loyalty_target.distance, 0.0);
    }
}
