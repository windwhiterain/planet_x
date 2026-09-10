//! 雇佣承包市场的单元测试。

use super::*;
use crate::config::load_config;
use crate::world::default_state;

fn fresh(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// 手搓一张雇佣单（避开雇主侧的挂单估算，专测受雇方这一半）。
/// `shipper` 显式传：雇主与受雇方**必须是两家**（自己不能受雇于自己）。
fn contract(
    state: &mut State,
    config: &GameConfig,
    shipper: &str,
    capacity: f64,
    from: &str,
    to: &str,
    mins: f64,
) -> u64 {
    state.contracts.post(
        shipper.into(),
        "碳".into(),
        capacity,
        from.into(),
        to.into(),
        config.freight.share,
        state.round,
        mins,
    )
}

/// **λ 随信誉单调递减**（§C1 的支点：信誉越低越舍不得拿去冒险）。
#[test]
fn the_shadow_price_of_reputation_decreases_with_reputation() {
    let (config, _) = fresh(42);
    let mut prev = f64::INFINITY;
    for i in 0..=40 {
        let rep = i as f64 * 0.1;
        let l = shadow_price(&config, rep);
        assert!(
            l <= prev + 1e-12,
            "λ 必须随信誉单调不增：rep={rep:.1} 时 {l:.3} > {prev:.3}"
        );
        prev = l;
    }
    assert!(
        shadow_price(&config, 0.0) > shadow_price(&config, 4.0) * 5.0,
        "两端要拉开差距"
    );
    assert!(
        shadow_price(&config, 0.0) <= config.freight.shadow_lambda0 + 1e-9,
        "上界是 lambda0"
    );
}

/// **低信誉接不到难单**：同一条深空线，低信誉者的合格度必须显著更低。
#[test]
fn a_low_reputation_carrier_is_rarely_shown_a_hard_contract() {
    let (config, mut state) = fresh(42);
    let id = contract(&mut state, &config, "中国", 3.0, "冥王星", "地球", 2.2);
    let c = state.contracts.get(id).unwrap().clone();
    let low = eligibility(&config, 0.8, &c);
    let high = eligibility(&config, 3.0, &c);
    assert!(high > low, "高信誉的合格度必须更高：{high:.3} vs {low:.3}");
    assert!(low < 0.05, "信誉远低于门槛 ⇒ 极少看见（实为 {low:.4}）");
    assert!(
        high > 0.9,
        "信誉远高于门槛 ⇒ 基本总看得见（实为 {high:.4}）"
    );
}

/// **λ 的两端就是 §C1 那张表**：够不着的活（注定吃差评）低信誉者不赌、高信誉者敢赌；
/// 轻松的活反之。
///
/// 这是本模块符号约定（`D = 报酬 + λ × Δ信誉`）的守卫——若把它写成 §C1 原文的
/// `报酬 − λ × Δ信誉`，这个用例会**反过来**失败（够不着的活对低信誉者反而变香）。
#[test]
fn the_reputation_ladder_makes_newcomers_cautious_and_veterans_greedy() {
    let (config, mut state) = fresh(42);
    // 一张**够不着**的活：要求运力远超这条线上能凑出来的（注定吃差评）。
    let hard = contract(&mut state, &config, "美国", 9999.0, "金星", "地球", 0.0);
    // 一张**稳活**：要求运力小到随手就能达标。
    let easy = contract(&mut state, &config, "美国", 0.01, "金星", "地球", 0.0);
    let (hard, easy) = (
        state.contracts.get(hard).unwrap().clone(),
        state.contracts.get(easy).unwrap().clone(),
    );
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .unwrap()
        .name
        .clone();
    let value = |state: &State, c: &Contract, rep: f64| {
        let mut s = state.clone();
        s.faction_mut("中国").unwrap().reputation = rep;
        // 只留这一艘做候选：这条线上"我能凑出的运力"才可判。
        s.ships.retain(|sh| sh.name == ship);
        decision_value(&s, &config, c, "中国")
    };
    let (d_low, d_high) = (value(&state, &hard, 0.5), value(&state, &hard, 3.5));
    assert!(
        d_high > d_low,
        "够不着的活：高信誉者（λ 小）才敢接。低 {d_low:.2} vs 高 {d_high:.2}"
    );
    let (e_low, e_high) = (value(&state, &easy, 0.5), value(&state, &easy, 3.5));
    assert!(
        e_low > e_high,
        "稳活：低信誉者（λ 大）更想要——他靠履约攒信誉。低 {e_low:.2} vs 高 {e_high:.2}"
    );
    // 而且**够不着的活对信誉更敏感**：信誉才是险活的定价者。
    assert!(
        (d_high - d_low) > (e_high - e_low),
        "够不着的活对信誉的敏感度必须高于稳活：（难 {:.2} vs 稳 {:.2}）",
        d_high - d_low,
        e_high - e_low
    );
}

/// **运力不够就不太愿意接**（用户：「受雇方也会根据当前运力决定是否接受雇佣」）。
///
/// 同一条线、同一份要求：手里船多的势力接单概率必须显著高于只有一条小船的势力。
#[test]
fn a_carrier_short_of_capacity_is_reluctant_to_accept() {
    let (config, mut state) = fresh(42);
    // 清场：只留中国的船当候选（美国一条都没有 ⇒ 对照）。
    state.depots.clear();
    state.contracts.contracts.clear();
    state.ships.retain(|s| s.faction_id == "中国");
    for f in state.factions.iter_mut() {
        f.reputation = 1.0;
    }
    let id = contract(&mut state, &config, "美国", 1.0, "金星", "地球", 0.6);
    let c = state.contracts.get(id).unwrap().clone();
    let rich = accept_chance(&state, &config, &c, "中国");
    // 只剩一条护卫舰（舱容 2）⇒ 能凑的运力小得多。
    let keep = state
        .ships
        .iter()
        .find(|s| s.class == "corvette")
        .unwrap()
        .name
        .clone();
    state.ships.retain(|s| s.name == keep);
    let poor = accept_chance(&state, &config, &c, "中国");
    assert!(
        rich > poor,
        "船多的势力该更愿意接：船多 {rich:.3} vs 只剩一条 {poor:.3}"
    );
    // 一条船都没有 ⇒ 物理上接不了（`match_carriers` 会直接跳过，连骰子都不掷）。
    state.ships.clear();
    assert_eq!(
        available_throughput(&state, &config, "中国", "金星", "地球"),
        0.0
    );
}

/// **撮合只定「谁受雇」，不押船**（用户：「对方派几艘船都无所谓」）。
///
/// 接下之后由 [`assign_hired_ships`] 按缺口派出船——一张单可以有几条，且都是受雇方自己的。
#[test]
fn accepting_an_order_hires_a_faction_and_then_staffs_it_with_ships() {
    let (config, mut state) = fresh(42);
    // 清场：只留中国有积压（雇主），其余势力保持开局舰队（受雇方候选）。
    state.depots.clear();
    state.contracts.contracts.clear();
    state.factions.iter_mut().for_each(|f| f.reputation = 1.0);
    state.depot_add("中国", "金星", "碳", 400.0);
    state.ships.retain(|s| s.faction_id != "中国"); // 中国没有船 ⇒ 只能请人
    let mut rng = crate::prng::Prng::new(42);
    sim::advance(&mut state, &config, &mut rng);
    let taken: Vec<Contract> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_hired())
        .cloned()
        .collect();
    assert!(
        !taken.is_empty(),
        "400 件积压挂出去，该有人接：{:?}",
        state.contracts.contracts
    );
    for c in &taken {
        let carrier = c.carrier.clone().unwrap();
        assert_ne!(carrier, c.shipper, "不能自己接自己的单");
        assert_eq!(
            c.accepted_round,
            Some(state.round),
            "雇佣期从接单那一刻起算"
        );
        assert!(c.expires_round > state.round, "固定期必须在将来");
        assert!(c.review_round > state.round, "第一次考核在一个周期之后");
        // 派上去的船都必须是**受雇方自己**的，而且跑的是**雇主**的路线。
        for ship in state.contracts.ships_of(c.id) {
            let s = state.ship(&ship).expect("派工指向的船必须存在");
            assert_eq!(s.faction_id, carrier, "只能派自己的船");
            let route =
                freight::route_for(&state, &config, &carrier, &ship, &mut crate::model::RoundInputs::default()).expect("接活的舰要有路线");
            assert_eq!(route, (c.from.clone(), c.to.clone()), "跑的是雇主的路线");
            // 每条腿都有一端是雇主的首都（集散地）：集货腿的**终点**是首都，补给腿的**起点**
            // 是首都——「完全禁止瞬移」之后两个方向都是正式的单子。
            assert!(
                route.0 == state.capital_body(&c.shipper)
                    || route.1 == state.capital_body(&c.shipper),
                "每条腿都该有一端是雇主首都，实为 {route:?}"
            );
        }
    }
}

/// **一张单可以同时跑好几条船；船沉了不算事**（用户：「对方派几艘船都无所谓」、
/// 「船沉没不管，只管统计运输量」）。
#[test]
fn a_contract_runs_any_number_of_ships_and_survives_one_going_down() {
    let (config, mut state) = fresh(42);
    let id = contract(&mut state, &config, "中国", 5.0, "金星", "地球", 0.6);
    let ships: Vec<ShipId> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == "美国")
        .map(|s| s.name.clone())
        .take(2)
        .collect();
    assert!(ships.len() >= 2, "用例前提：美国开局至少两条舰");
    {
        let c = state.contracts.get_mut(id).unwrap();
        c.carrier = Some("美国".into());
        c.accepted_round = Some(0);
        c.expires_round = 99; // 别让「固定期到期」插进来（那个用例另测）
        c.review_round = 99;
    }
    for s in &ships {
        state.contracts.assign(s.clone(), id);
    }
    let rep0 = state.faction("美国").unwrap().reputation;
    // 一艘沉了：`settle_contracts` 抹掉那条派工，**不发事件、不掉信誉**。
    state.ships.retain(|s| s.name != ships[0]);
    settle_contracts(&mut state, &config, &mut crate::model::RoundInputs::default());
    assert_eq!(
        state.contracts.ships_of(id),
        vec![ships[1].clone()],
        "沉掉的那条派工该消失，另一条留着"
    );
    assert!(
        (state.faction("美国").unwrap().reputation - rep0).abs() < 1e-9,
        "船沉本身不该动信誉（考核会说话，别罚两次）"
    );
    assert!(
        !state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::ContractEnded { .. })),
        "合同不该因为沉了一条船就结束（还有别的船在跑）"
    );
}

/// **考核 = 按实测吞吐掷好评/差评**（用户：「周期性对评估受雇方的运力是否达标来反馈信誉」）。
///
/// 三件事一起钉：好评概率随达标率**严格递增**（连续的，不是过线才算）；一次考核给信誉
/// 加减振幅固定的量；而**没有「有货可运的回合」时不评**（不能罚它没搬不存在的货）。
#[test]
fn a_review_judges_the_measured_throughput_and_never_punishes_an_idle_depot() {
    let (config, mut state) = fresh(42);
    state.depots.clear(); // 起运货栈先是空的（世界生成可能给中国留下货栈）
    let id = contract(&mut state, &config, "中国", 3.0, "金星", "地球", 0.6);
    {
        let c = state.contracts.get_mut(id).unwrap();
        c.carrier = Some("美国".into());
        c.accepted_round = Some(0);
        c.expires_round = 99; // 只测考核，别让到期插进来
        c.review_round = 0; // 本回合就该考核
    }
    // 好评概率：达标率越高越大，1.0 处正好五五开。
    let mut prev = -1.0;
    for i in 0..=20 {
        let r = i as f64 * 0.1;
        let p = review_chance(&config, r);
        assert!(
            p > prev,
            "好评概率必须随达标率单调增：{r:.1} 时 {p:.3} <= {prev:.3}"
        );
        prev = p;
    }
    assert!(
        (review_chance(&config, 1.0) - 0.5).abs() < 1e-9,
        "恰好达标 = 五五开"
    );
    // 0 个有货回合 ⇒ 不评：信誉一个字都不动，也不发事件。
    let rep0 = state.faction("美国").unwrap().reputation;
    state.contracts.get_mut(id).unwrap().review_round = 0;
    settle_contracts(&mut state, &config, &mut crate::model::RoundInputs::default());
    assert!(
        (state.faction("美国").unwrap().reputation - rep0).abs() < 1e-9,
        "货栈一直没货 ⇒ 不该考核（更不该判它不达标）"
    );
    assert!(
        !state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::ContractReviewed { .. })),
        "无从考核就不该发 `contract_reviewed`"
    );
    // 有货可运 + 交得足 ⇒ 必评，且信誉的变动量恰好是考核幅度（好评或差评二选一）。
    state.depot_add("中国", "金星", "碳", 50.0);
    {
        let c = state.contracts.get_mut(id).unwrap();
        // 巡检会先把这一回合算进分母（货栈有货）⇒ 5 + 1 = 6 个有货回合；
        // 扣掉一个来回的在途宽免（nominal_hold 6 ÷ 要求运力 3 = 2）⇒ 产出期 4 回合，
        // 账上该产出 3 × 4 = 12 件 ⇒ 恰好一个完整账期。
        c.served_rounds = 5;
        c.delivered = 12.0;
        c.review_round = 0;
    }
    settle_contracts(&mut state, &config, &mut crate::model::RoundInputs::default());
    let rep1 = state.faction("美国").unwrap().reputation;
    assert!(
        (rep1 - rep0).abs() - config.freight.reputation_gain < 1e-9,
        "一次考核的振幅必须是 reputation_gain：{rep0:.3} → {rep1:.3}"
    );
    let reviewed = state
        .events
        .iter()
        .find_map(|e| match e {
            GameEvent::ContractReviewed { ratio, .. } => Some(*ratio),
            _ => None,
        })
        .expect("该发一次考核事件");
    assert!(
        (reviewed - 1.0).abs() < 1e-9,
        "事件的达标率该是 1.0，实为 {reviewed:.3}"
    );
}

/// **禁运同样挡雇佣**（Q4）：被封锁的势力**既接不到**这条线上的活，也不该把自己的
/// 运力借给封锁它的人——商品市场的 `trade_blocked` 判的是「根本不卖给你」，而雇对方的
/// 船运货比卖矿更直接。
#[test]
fn a_blockade_keeps_a_faction_out_of_the_hiring_market() {
    let (config, mut state) = fresh(42);
    // 清场：只留中国有积压（雇主）当待雇方；中国没有船 ⇒ 只能请人。
    state.depots.clear();
    state.contracts.contracts.clear();
    state.factions.iter_mut().for_each(|f| f.reputation = 1.0);
    state.depot_add("中国", "金星", "碳", 400.0);
    state.ships.retain(|s| s.faction_id != "中国");
    let id = contract(&mut state, &config, "中国", 3.0, "金星", "地球", 0.0);
    // 把所有势力之间的封锁全打开（关系拉到冰点）⇒ 没人能接。
    for f in state.factions.iter_mut() {
        for v in f.relations.values_mut() {
            *v = config.market.embargo_relation - 10.0;
        }
    }
    let mut rng = crate::prng::Prng::new(42);
    sim::advance(&mut state, &config, &mut rng);
    assert!(
        state.contracts.get(id).expect("单子还在").is_open(),
        "全星系互相封锁 ⇒ 不该有人接下这条线上的活：{:?}",
        state.contracts.get(id)
    );
    // 关系修好之后（同一张单、同一个回合数）就有人接了——证明上面那条不是因为
    // 别的原因（没船、门槛、自评）而没人接。
    let mut state2 = fresh(42).1;
    state2.depots.clear();
    state2.contracts.contracts.clear();
    state2.factions.iter_mut().for_each(|f| f.reputation = 1.0);
    state2.depot_add("中国", "金星", "碳", 400.0);
    state2.ships.retain(|s| s.faction_id != "中国");
    let id2 = contract(&mut state2, &config, "中国", 3.0, "金星", "地球", 0.0);
    for f in state2.factions.iter_mut() {
        for v in f.relations.values_mut() {
            *v = 50.0;
        }
    }
    let mut rng2 = crate::prng::Prng::new(42);
    sim::advance(&mut state2, &config, &mut rng2);
    assert!(
        state2.contracts.get(id2).expect("单子还在").is_hired(),
        "关系正常时该有人接（否则上一条断言是空转的）"
    );
}

/// **固定期到期 ⇒ 按信誉决定续约还是换人**，用的是当初那条准入闸。
#[test]
fn an_expired_term_is_renewed_or_switched_on_the_same_gate_that_hired_it() {
    let (config, state) = fresh(42);
    let setup = |rep: f64| {
        let mut st = state.clone();
        st.depots.clear();
        st.depot_add("中国", "金星", "碳", 50.0);
        let id = contract(&mut st, &config, "中国", 3.0, "金星", "地球", 0.6);
        {
            let c = st.contracts.get_mut(id).unwrap();
            c.carrier = Some("美国".into());
            c.accepted_round = Some(0);
            c.expires_round = st.round; // 本回合到期
            c.review_round = st.round;
            c.served_rounds = 1;
            c.delivered = 3.0;
        }
        st.faction_mut("美国").unwrap().reputation = rep;
        settle_contracts(&mut st, &config, &mut crate::model::RoundInputs::default());
        (id, st)
    };
    // 信誉高于门槛（0.6 上下）⇒ 大概率续约：合同留在簿上、仍是同一受雇方、进度清零。
    let (id, st) = setup(4.0);
    let c = st.contracts.get(id).expect("续约 ⇒ 合同还在簿上");
    if c.is_hired() {
        assert_eq!(c.carrier.as_deref(), Some("美国"), "续约是同一份关系继续");
        assert_eq!(c.delivered, 0.0, "新一期从零开始记");
        assert!(c.expires_round > st.round, "固定期重新起算");
    } else {
        // 掷骰子偶尔会判成换人——那也是合法结果，但必须**回到挂单簿**而不是消失。
        assert_eq!(st.contracts.get(id).unwrap().carrier, None);
    }
    // 信誉远低于门槛 ⇒ 大概率换人：合同**回到挂单簿**（还能被别人接），并发事件。
    let (id, st) = setup(0.0);
    let c = st.contracts.get(id).expect("换人只是回到挂单簿，不是作废");
    if !c.is_hired() {
        assert_eq!(c.delivered, 0.0, "回到挂单簿 ⇒ 本期进度清掉");
        assert_eq!(c.accepted_round, None);
        assert!(
            st.events.iter().any(|e| matches!(
                e,
                GameEvent::ContractEnded { reason, .. } if reason == "term"
            )),
            "换人该发 `contract_ended`"
        );
    }
}

/// **受雇方缺船时提前结束雇佣**（用户：「是否提前结束雇佣」），而且**离开前要结清这一期**。
///
/// 不结清的话，「这一期干砸了」的最优解就是赶在考核之前跑掉——那不是市场，是逃单。
#[test]
fn a_carrier_short_of_ships_quits_and_settles_the_period_it_served() {
    let (config, mut state) = fresh(42);
    // 中国的积压很多（自家缺船），美国受雇替它跑一条线，并且美国自己也有积压。
    state.depots.clear();
    state.contracts.contracts.clear();
    state.depot_add("美国", "水星", "铁", 500.0); // 美国自家缺船（一处积压配一条船）
    let id = contract(&mut state, &config, "中国", 3.0, "金星", "地球", 0.0);
    {
        let c = state.contracts.get_mut(id).unwrap();
        c.shipper = "中国".into();
        c.carrier = Some("美国".into());
        c.accepted_round = Some(0);
        c.expires_round = 99;
        c.review_round = 99;
        c.served_rounds = 6; // 账期已经够长（> 2×宽免）⇒ 这一期可评
        c.delivered = 0.0; // 这一期一件没交（达标率 0 ⇒ 该吃差评）
    }
    state.depot_add("中国", "金星", "碳", 50.0); // 起运货栈有货
    // 把美国的船全调走，让它**一条能用的船都没有** ⇒ 缺口 = 1（一处积压）。
    state.ships.retain(|s| s.faction_id != "美国");
    let mut rng = crate::prng::Prng::new(42);
    let mut ended = false;
    let mut reviewed = false;
    for _ in 0..40 {
        sim::advance(&mut state, &config, &mut rng);
        for e in &state.events {
            match e {
                GameEvent::ContractEnded { reason, .. } if reason == "recalled" => ended = true,
                GameEvent::ContractReviewed { .. } => reviewed = true,
                _ => {}
            }
        }
        if ended {
            break;
        }
    }
    assert!(ended, "自家缺船时它该退掉手上的雇佣（40 回合内必发生）");
    assert!(reviewed, "提前结束前必须结清这一期（否则逃单就是最优解）");
}

/// **关系结束不能把在途的那票货顺手变成受雇方自己的**（用户裁决下的一个真陷阱）。
///
/// 货卸到哪儿由**派工记录**决定（`sim::cargo_owner` 认的就是它）：关系一结束就连船带货
/// "还给"受雇方，那票从**雇主货栈**装走的货就会卸进**受雇方自己**的池子——等于把雇主的
/// 货偷走。所以 `end_contract` 只放空舱的船；满载的船把这趟跑完、卸完变空之后才放掉。
#[test]
fn quitting_never_hands_the_cargo_in_transit_to_the_carrier() {
    let (config, mut state) = fresh(42);
    let share = config.freight.share;
    state.depots.clear();
    state.contracts.contracts.clear();
    state.depot_add("中国", "金星", "碳", 10.0);
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "美国" && s.class == "destroyer")
        .expect("美国开局有驱逐舰")
        .name
        .clone();
    let class = state.ship(&ship).unwrap().class.clone();
    let id = state.contracts.post(
        "中国".into(),
        "碳".into(),
        3.0,
        "金星".into(),
        "地球".into(),
        share,
        0,
        0.0,
    );
    {
        let c = state.contracts.get_mut(id).unwrap();
        c.carrier = Some("美国".into());
        c.accepted_round = Some(0);
        c.expires_round = 99;
        c.review_round = 99;
    }
    state.contracts.assign(ship.clone(), id);
    // 停在雇主货栈的泊位上装货（装的是**中国**的货）。
    let vpos = state.body_position("金星");
    state.ship_mut(&ship).unwrap().position = vpos;
    let loaded = match sim::haul_step(&mut state, &config, &ship, &class, "金星", "地球", &mut crate::model::RoundInputs::default()) {
        sim::HaulStep::Loaded { units, .. } => units,
        other => panic!("停在雇主货栈上该装货，实为 {other:?}"),
    };
    // 受雇方**提前结束雇佣**（抽手）。
    end_contract(&mut state, id, "recalled");
    assert_eq!(
        state.contracts.assignment_of(&ship),
        Some(id),
        "满载的船不许放——放了那票货就成了受雇方自己的"
    );
    // 卸到中国首都：抽成归美国，余数必须进**中国**的池子。
    let (cn0, us0) = (
        state
            .faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
        state
            .faction("美国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
    );
    state.ship_mut(&ship).unwrap().position = state.body_position("地球");
    assert!(
        matches!(
            sim::haul_step(&mut state, &config, &ship, &class, "金星", "地球", &mut crate::model::RoundInputs::default()),
            sim::HaulStep::Delivered {
                into_pool: true,
                ..
            }
        ),
        "目的 = 雇主首都 ⇒ 该进池子"
    );
    let (cn1, us1) = (
        state
            .faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
        state
            .faction("美国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
    );
    assert!(
        (cn1 - cn0 - (loaded - loaded * share)).abs() < 1e-9,
        "雇主该收到 {} 件（装的 {loaded} 减去抽成），实收 {:.3}",
        loaded - loaded * share,
        cn1 - cn0
    );
    assert!(
        (us1 - us0 - loaded * share).abs() < 1e-9,
        "受雇方只该拿抽成 {:.3}，实收 {:.3}",
        loaded * share,
        us1 - us0
    );
    // 卸完变空 ⇒ 巡检放掉这条派工（它回去跑自己的线）。
    settle_contracts(&mut state, &config, &mut crate::model::RoundInputs::default());
    assert_eq!(
        state.contracts.assignment_of(&ship),
        None,
        "在途那一趟跑完就该放回去跑自己的线"
    );
}

/// **端到端**：受雇方真的把货搬到了雇主首都，**报酬就是它自留的那部分货**，
/// 而雇主收到的是扣掉抽成的量；**交付本身不动信誉**（信誉只由考核产生）。
///
/// 这一条把整条腿走完：挂单（手搓）→ 受雇 → 派工 → 装（**从雇主的货栈装**）→ 飞 →
/// 卸进**雇主的池子** → 抽成进受雇方自己的池子。
#[test]
fn a_hired_ship_delivers_to_the_employer_and_pays_itself_in_cargo() {
    let (config, mut state) = fresh(42);
    let share = config.freight.share;
    // --- 布景：中国在金星积压 12 件碳、自己没有船；美国派一艘驱逐舰去运 ---
    state.depots.clear();
    state.contracts.contracts.clear();
    state.ships.retain(|s| s.faction_id != "中国");
    state.depot_add("中国", "金星", "碳", 12.0);
    // 把所有人关系拉正：这一条测的是**运输腿**，不是战争。实测过不这么做会怎样——
    // 美国的驱逐舰在第 2 回合被打沉，于是"交付"这件事根本没发生。
    for f in state.factions.iter_mut() {
        for v in f.relations.values_mut() {
            *v = 50.0;
        }
        // 也把家底垫厚：否则第 2 回合就会因为**维护费付不出**而把船报废
        //（实测 `ShipDestroyed cause=UpkeepShortfall`）——那同样是本用例之外的事。
        f.resources.insert("铁".into(), 2000.0);
        f.resources.insert("碳".into(), 2000.0);
    }
    for s in state.ships.iter_mut() {
        s.hull = s.hull_max; // 满血出战，免得被自保撤退打断
    }
    let carrier_ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "美国" && s.class == "destroyer")
        .expect("美国开局有驱逐舰")
        .name
        .clone();
    // 把受雇的舰**直接摆到金星的泊位上**（否则要先飞几个回合，用例说不清是谁的功劳）。
    let at_venus = state.body_position("金星");
    {
        let s = state.ship_mut(&carrier_ship).unwrap();
        s.position = at_venus;
        s.velocity = 0.0;
    }
    let id = state.contracts.post(
        "中国".into(),
        "碳".into(),
        3.0,
        "金星".into(),
        "地球".into(),
        share,
        0,
        0.0,
    );
    {
        let c = state.contracts.get_mut(id).unwrap();
        c.carrier = Some("美国".into());
        c.accepted_round = Some(0);
        c.expires_round = 99;
        c.review_round = 99;
    }
    state.contracts.assign(carrier_ship.clone(), id);
    let rep_us0 = state.faction("美国").unwrap().reputation;
    let mut rng = crate::prng::Prng::new(42);
    let mut delivered_events = 0usize;
    // 事件流只保留**本回合**（`advance` 开头清空），所以跨回合的量要在这里累加。
    let (mut paid, mut cut) = (0.0, 0.0);
    for _ in 0..8 {
        sim::advance(&mut state, &config, &mut rng);
        for e in &state.events {
            if let GameEvent::ContractDelivered {
                contract,
                amount,
                cut: c,
                ..
            } = e
            {
                if *contract == id {
                    delivered_events += 1;
                    paid += amount;
                    cut += c;
                }
            }
        }
    }
    assert!(delivered_events > 0, "八回合内该跑完一趟（金星→地球很近）");
    // 账要**按事件流**核（池子同时在被维护费/建造花掉，直接比池子的差值是脆的）。
    //
    // 口径与「一票货」形态不同：那时合同上写着「运 12 件」，交付总量有**上界**；
    // 现在单子要的是**运力**（单位/回合），只要金星还有货、船还在跑，它就会一直搬
    // ——所以这里能核的是**分成比例**（抽成制 Q10，这一条才是机制不变量），不是某个绝对数。
    assert!(
        paid > 0.0 && cut > 0.0,
        "该有交付：雇主实收 {paid:.3} / 受雇方自留 {cut:.3}"
    );
    assert!(
        (paid / (paid + cut) - (1.0 - share)).abs() < 1e-9,
        "抽成比例必须恰好是 85%（实收 {paid:.3} / 自留 {cut:.3}）"
    );
    // 货真的从**雇主的货栈**搬走了。
    let left: f64 = state
        .depots
        .get(&("中国".to_string(), "金星".to_string()))
        .map(|m| m.values().sum())
        .unwrap_or(0.0);
    assert!(left < 12.0, "受雇的船该把货装走：金星货栈还剩 {left:.2} 件");
    // **交付不动信誉**：雇佣形态下信誉只由考核产生（这个用例里 review_round=99，不评）。
    assert!(
        (state.faction("美国").unwrap().reputation - rep_us0).abs() < 1e-9,
        "交付本身不该改信誉（{rep_us0:.3} → {:.3}）",
        state.faction("美国").unwrap().reputation
    );
    // 进度记在合同上（用于考核的达标率）。
    let c = state.contracts.get(id).expect("合同还在雇佣期内");
    assert!(c.delivered > 0.0, "交付要记进度（实为 {:.2}）", c.delivered);
}
