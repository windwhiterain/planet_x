//! 市场与运输：B3 批——把「为什么是这个价 / 我买到的货为什么少了 / 有货在卖我却没买到 /
//! 这趟货为什么没运回来 / 哪处货栈在积压」从**算完就扔**变成读面。
//!
//! 清单与批次见 `.agents/notes/step-intermediates.md` §6（B3）。规矩同 B1/B2：观测与过程
//! **同处一行**、纯追加（行为中性 ⇒ digest 逐字不变）、「这一步没跑」用中性缺省**显式**表达。
//!
//! 这一批的形状裁决有两条，都钉在下面的用例里：
//! * **成交清单的粒度是「一对（买方 × 卖方）一行」**，不是「一对 × 一资源一行」——距离/深度/
//!   关系只由这一对决定，每种矿的成交价就是 `market_price × (rel_mult + freight_rate)`；
//! * **`haul_steps` 一舰一行**，AI 与**玩家指令**两条执行路径都写它（玩家舰不产生判定行）。

use super::*;

/// **成交清单必须与世界的账对得上**：把每一笔的「买方实收」加起来，逐资源等于
/// `view.market_settled`（全世界这一回合收到的量）。
///
/// 这条恒等式同时钉住四件事：① 清单**没漏行**（漏一笔就对不上）；② 一行**没重复计**
/// （同一对买两种矿只占一行，`moved` 各记各的）；③ `moved` 的语义确实是「卖方**交出**的量」
/// （收货要乘 `(1 − loss)`）——把它写成「收到的量」这条就红；
/// ④ 成交清单与 `market_settled` 是同一个回合、同一份快照。
#[test]
fn trades_reconcile_with_the_world_settled_total() {
    let Some((_config, _state, view)) = world_with_a_trade(7, 40) else {
        panic!("seed 7 的 40 回合里一次都没成交——守卫会退化成空转");
    };
    let mut received: ResourceMap = ResourceMap::new();
    for t in &view.market_trades {
        assert!(!t.moved.is_empty(), "{} → {} 这一行没有任何货，不该占位", t.buyer, t.seller);
        assert_ne!(t.buyer, t.seller, "自己跟自己成交？");
        assert!((0.0..=1.0).contains(&t.loss), "丢货比例越界：{}", t.loss);
        for (rt, take) in &t.moved {
            assert!(*take > 0.0, "{} → {} 的 {rt} 报了非正的成交 {} ", t.buyer, t.seller, take);
            *received.entry(rt.clone()).or_insert(0.0) += take * (1.0 - t.loss);
        }
    }
    for (rt, got) in &view.market_settled {
        if *got <= 1e-9 {
            continue;
        }
        let sum = received.get(rt).copied().unwrap_or(0.0);
        assert!(
            (sum - got).abs() < 1e-6,
            "{rt}: 成交清单加起来 {sum}，但世界账上说收到 {got}（清单漏了/重了/语义写反了）"
        );
    }
    // 反向：清单里出现的资源，世界账上必须有量（否则清单在编货）。
    for rt in received.keys() {
        assert!(
            view.market_settled.get(rt).copied().unwrap_or(0.0) > 1e-9,
            "{rt}: 清单里有成交，世界账上却是 0"
        );
    }
}

/// **价格分解是「这一对」自己的数**：`freight_rate` 与 `mond_extra` 必须与引擎的两条定义式
/// 逐字咬合（拿**记录下来的** `dist_au`/`depth` 重算一遍，而不是另编一个模型）。
///
/// 这条守卫的理由：读面把这三个数分开给，就是为了让「为什么是这个价」可读——
/// 如果它们之间不再自洽（比如改了配置却只更新一处），读者会照着错的分解去调策略。
/// ⚠ **同一天体上的两家之间 `dist_au = 0` ⇒ 运费 0**（地球上有五座城属于不同势力，
/// 它们之间没有星际运费）——那是真的，不是漏算，所以「非零运费」要跨回合找。
#[test]
fn trade_price_terms_are_self_consistent() {
    let (config, mut state) = fresh_world(7);
    let mut rng = Prng::new(7);
    let m = config.market.clone();
    let mut rows = 0usize;
    let mut seen_freight = 0usize;
    let mut seen_mond = 0usize;
    let mut seen_same_body = 0usize;
    for _ in 0..60 {
        let view = advance(&mut state, &config, &mut rng);
        for t in &view.market_trades {
            rows += 1;
            assert!(t.dist_au >= 0.0 && t.depth >= 0.0, "距离/深度不该是负数：{t:?}");
            assert!(t.rel_mult > 0.0, "关系倍率必须是正的（价格倍率的一部分）");
            assert!((0.0..=1.0).contains(&t.mastery), "掌握度应在 0..1：{}", t.mastery);
            let want_mond = if t.depth > 0.0 {
                m.mond_freight_mult * (t.depth / (t.depth + 1.0))
            } else {
                0.0
            };
            assert!(
                (t.mond_extra - want_mond).abs() < 1e-9,
                "穿带溢价 {} ≠ mond_freight_mult × depth/(depth+1) = {want_mond}",
                t.mond_extra
            );
            let want_freight = m.freight_per_au * t.dist_au * (1.0 + t.mond_extra);
            assert!(
                (t.freight_rate - want_freight).abs() < 1e-9,
                "运费率 {} ≠ freight_per_au × dist × (1 + mond_extra) = {want_freight}",
                t.freight_rate
            );
            // 丢货只可能来自穿带：不穿带的线上必须是 0。
            if t.depth == 0.0 {
                assert_eq!(t.loss, 0.0, "{} → {}：没穿带却有丢货", t.buyer, t.seller);
            }
            if t.freight_rate > 0.0 {
                seen_freight += 1;
            }
            if t.mond_extra > 0.0 {
                seen_mond += 1;
                assert!(t.depth > 0.0, "有穿带溢价却没有深度");
                assert!(t.loss > 0.0, "穿了带却一点货都没丢（除非掌握度到顶）");
            }
            if t.dist_au == 0.0 {
                seen_same_body += 1;
            }
        }
    }
    assert!(rows >= 5, "60 回合只成交 {rows} 笔——守卫太空");
    assert!(seen_freight >= 1, "60 回合里每一笔都零运费——分解式等于空转");
    assert!(
        seen_same_body >= 1,
        "一笔「同天体」贸易都没有——那正是 `dist_au = 0 ⇒ 运费 0` 这一档，不该消失"
    );
    let _ = seen_mond; // 穿带那一档取决于轨迹（深层贸易），不强制出现
}

/// 跑到「市场上真的成交过」的回合：返回那一回合的视图与落定的 state。
///
/// 「到过、成交过」而不是「某个固定回合成交」：固定回合会让守卫随世界轨迹漂移而随机翻车
/// （`decisions_table_matches_the_derived_record` 那条注释记着同一次踩坑）。
fn world_with_a_trade(seed: u64, max_rounds: u32) -> Option<(GameConfig, State, RoundView)> {
    let (config, mut state) = fresh_world(seed);
    let mut rng = Prng::new(seed);
    for _ in 0..max_rounds {
        let view = advance(&mut state, &config, &mut rng);
        if !view.market_trades.is_empty() {
            return Some((config, state, view));
        }
    }
    None
}

/// **买方名次就是引擎排好的那个顺序**：名次是 0..n 的一个排列，且与购买力降序一致
/// （同额按名字升序——这是引擎里写死的 tie-break，别让读者重排一遍）。
#[test]
fn market_rank_is_the_engines_own_buying_order() {
    let (config, mut state) = fresh_world(7);
    let mut rng = Prng::new(7);
    let mut view = view_from_state(&state, &config);
    for _ in 0..12 {
        view = advance(&mut state, &config, &mut rng);
    }
    let mut ranked: Vec<(usize, &FactionId, f64)> = Vec::new();
    for (fid, row) in &view.factions {
        let rank = row
            .market_rank
            .unwrap_or_else(|| panic!("{fid} 没有买方名次——这一回合跑过市场却没排队？"));
        ranked.push((rank, fid, row.purchasing_power));
    }
    assert!(ranked.len() >= 2, "至少要两个势力才谈得上排队");
    ranked.sort_by_key(|(rank, _, _)| *rank);
    for (i, (rank, fid, _)) in ranked.iter().enumerate() {
        assert_eq!(*rank, i, "{fid} 的名次不连续/重复（引擎排完就该是 0..n 的排列）");
    }
    for w in ranked.windows(2) {
        let (_, a, pa) = &w[0];
        let (_, b, pb) = &w[1];
        assert!(
            pa > pb || (pa == pb && a < b),
            "{a}（购买力 {pa}）排在 {b}（{pb}）前面，但购买力更小——名次与购买力不一致"
        );
    }
}

/// **禁运三档**：`trade_blocked_by` 是一张「谁 + 为什么」的名单，三个原因都来自
/// `trade_block_cause`（战争 / 关系冷 / 联盟封锁），且**战争那一档必须在 `view.wars` 里**。
#[test]
fn trade_block_list_names_the_blocker_and_the_tier() {
    let (config, mut state) = fresh_world(7);
    let mut rng = Prng::new(7);
    let mut view = view_from_state(&state, &config);
    for _ in 0..20 {
        view = advance(&mut state, &config, &mut rng);
    }
    const TIERS: [&str; 3] = ["war", "cold", "coalition"];
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = 0usize;
    for (fid, row) in &view.factions {
        for (blocker, cause) in &row.trade_blocked_by {
            entries += 1;
            assert_ne!(blocker, fid, "{fid} 把自己列进禁运名单了");
            assert!(state.faction(blocker).is_some(), "{blocker} 不是这个世界的势力");
            assert!(TIERS.contains(&cause.as_str()), "没见过这一档：{blocker} → {fid} = {cause}");
            seen.insert(cause.clone());
            if cause == "war" {
                let pair = if fid < blocker {
                    (fid.clone(), blocker.clone())
                } else {
                    (blocker.clone(), fid.clone())
                };
                assert!(
                    view.wars.contains(&pair) || view.wars.contains(&(pair.1.clone(), pair.0.clone())),
                    "{blocker} 与 {fid} 报的是战争禁运，但 `view.wars` 里没有这一对"
                );
            }
            // 名单与判据同源：拿引擎的函数复核一遍（这正是「别在读面另编一套」的检查）。
            let want = trade_block_cause(&state, &config, blocker, fid);
            assert_eq!(
                want.map(|c| c.to_string()).as_deref(),
                Some(cause.as_str()),
                "{blocker} → {fid} 的原因与引擎判据不一致"
            );
        }
    }
    assert!(entries >= 1, "20 回合里一次禁运都没有——这条守卫会退化成空转（见过 {seen:?}）");
}

/// **每艘在跑运输的舰都有一步记录**：`haul_steps` 的键集恰好是「本回合按 `Haul` 跑过的舰」。
///
/// 为什么这条值得钉死：`waiting`/`en_route` **既不落 State 也不发事件**，这张表是它们唯一
/// 的读法——漏一艘，那艘舰的「为什么没运回来」就查不到了；多一艘，读者会以为它跑了运输。
/// ⚠ 玩家舰也走这条（`step_military` 的指令分支），所以键集不能只对 AI 舰。
///
/// ⚠ 用例必须**撤掉 `fresh_world` 那条「全员战舰」的默认角色**——那是给测别的东西的用例用的
/// （见 `src/tests/sim/mod.rs` 的 `fresh_world` 注释），钉着它一艘运输舰都不会有。
/// 判据用**回合末**的指令叶：AI 分支在同一回合里把这次跑的路线写进叶（下一回合接着跑），
/// 玩家分支的叶本来就是玩家给的——所以「记了运输动作的舰，此刻的指令必须是运输」。
/// （反过来不成立：AI 这一回合可能放弃旧线去跑新线，所以不能要求「叶是运输的舰都记了」。）
#[test]
fn haul_steps_only_cover_ships_that_are_actually_on_a_haul_route() {
    let (config, mut state) = fresh_world(7);
    // 把舰队默认角色从「全员战舰」改成运输：定编与派单这条路才会真的跑起来。
    for c in state.control.values_mut() {
        c.default_role = Some(Control::player(ShipRole::Freight));
    }
    let mut rng = Prng::new(7);
    let mut view;
    let mut rounds_with_haul = 0usize;
    let mut steps_seen = std::collections::BTreeSet::new();
    for _ in 0..12 {
        view = advance(&mut state, &config, &mut rng);
        if !view.haul_steps.is_empty() {
            rounds_with_haul += 1;
        }
        for (ship, step) in &view.haul_steps {
            // ① 没有幽灵行：每一艘记了动作的舰，回合末的指令确实是「跑运输」。
            let s = state.ship(ship).unwrap_or_else(|| panic!("{ship} 不存在了，却是它记了运输动作"));
            assert!(s.hull > 0.0, "{ship} 已经沉了，却还记着运输动作");
            let order = state
                .control(s.faction_id.clone())
                .and_then(|c| c.ship_orders.get(ship))
                .map(|l| l.value.clone());
            assert!(
                matches!(order, Some(ShipBehavior::Haul { .. })),
                "第 {} 回合：{ship} 记了运输动作，但它的指令是 {order:?}（幽灵行）",
                state.round
            );
            steps_seen.insert(step.step().to_string());
            assert_eq!(
                step.step(),
                match step {
                    HaulStep::Loaded { .. } => "loaded",
                    HaulStep::Delivered { .. } => "delivered",
                    HaulStep::Waiting { .. } => "waiting",
                    HaulStep::EnRoute { .. } => "en_route",
                },
                "{ship} 的变体名与 serde 判别式不一致"
            );
            assert!(
                state.body(step.body()).is_some(),
                "{ship} 的运输动作指着不存在的天体 {}",
                step.body()
            );
            if matches!(step, HaulStep::Waiting { .. } | HaulStep::EnRoute { .. }) {
                assert_eq!(step.units(), 0.0, "{ship}: 等待/在途不该报出搬动的件数");
                assert!(!step.into_pool(), "{ship}: 没卸货却报「进了首都池」");
            }
            if matches!(step, HaulStep::Loaded { .. } | HaulStep::Delivered { .. }) {
                assert!(step.units() > 0.0, "{ship}: 装卸动作必须真的搬了货（否则该记成 waiting）");
            }
        }
    }
    assert!(rounds_with_haul >= 1, "12 回合里没有一艘舰跑过运输——用例构造失败（角色没撤掉？）");
    assert!(
        steps_seen.contains("loaded") || steps_seen.contains("delivered"),
        "一次装卸都没发生过（见过 {steps_seen:?}）——用例跑得太短"
    );
}

/// **货栈运力账就是引擎挂单用的那一本**：逐势力的 `Σuncovered ÷ Σneed` 必须等于
/// `haul_gap`（投影 `factions` 表里那一列），而账里的每一处货栈都要真的有积压。
///
/// 这条同时说明「读面为什么把账摊到每一处货栈」：势力级只有一个比值，看不到是哪处在积压。
#[test]
fn freight_gap_is_the_same_ledger_the_engine_posts_contracts_from() {
    let (config, mut state) = fresh_world(7);
    let mut rng = Prng::new(7);
    for _ in 0..10 {
        advance(&mut state, &config, &mut rng);
    }
    // 直接跑挂单那一步：它算的那本账就是读面记的那本（紧接着读 `haul_gap` 时 state 还没变）。
    let mut sink = RoundSink::default();
    autocontrol::freight::post_contracts(&mut state, &config, &mut sink);

    let mut checked = 0usize;
    let mut uncovered_seen = 0usize;
    for f in &state.factions {
        let fid = f.name.clone();
        let ledger = sink.freight_gap.get(&fid).cloned().unwrap_or_default();
        for (body, g) in &ledger {
            assert!(g.need > 0.0, "{fid} 的 {body}: 零需求的货栈不该占键");
            assert!(g.own >= 0.0 && g.hired >= 0.0, "{fid} 的 {body}: 负运力？");
            assert!(
                g.uncovered >= -1e-9,
                "{fid} 的 {body}: 缺口是负数（{}）——它必须是 max(0, need − own − hired)",
                g.uncovered
            );
            let want = (g.need - g.own - g.hired).max(0.0);
            assert!(
                (g.uncovered - want).abs() < 1e-9,
                "{fid} 的 {body}: 缺口 {} ≠ need − own − hired = {want}",
                g.uncovered
            );
            if g.uncovered > 1e-9 {
                uncovered_seen += 1;
            }
        }
        // 势力级的总账（引擎自己那把尺子）必须等于这本账的和。
        let need: f64 = ledger.values().map(|g| g.need).sum();
        let unc: f64 = ledger.values().map(|g| g.uncovered).sum();
        let want = if need <= 0.0 { 0.0 } else { (unc / need).clamp(0.0, 1.0) };
        let got = autocontrol::freight::haul_gap(&state, &config, &fid);
        assert!(
            (got - want).abs() < 1e-9,
            "{fid}: haul_gap = {got}，但按读面那本账算是 {want}（两本账漂了）"
        );
        checked += 1;
    }
    assert!(checked >= 2, "只检查到 {checked} 个势力——守卫太空");
    assert!(uncovered_seen >= 1, "10 回合后一处积压缺口都没有——这条守卫没在检查东西");
}
