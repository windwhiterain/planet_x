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
//!
//! ## 2026-10：三条**只看数据**的判据搬去了 `play/tests/g2_mid.py`
//!
//! `market_trades`（一对一行）+ 主流每回合的 `view.market_settled` + `meta.json` 的 `market`
//! 已经够 ⇒ **零新增序列化**；顺带把样本从「1 seed × 12–60 回合」放大到 **3 seed × 400 回合**：
//!
//! | 数据级判据（g2） | 原 Rust 用例 | 实测（3 seed × 400 回合） |
//! | --- | --- | --- |
//! | 成交清单 ↔ `view.market_settled` 逐回合对账（`moved × (1 − loss)` 求和、双向） | `trades_reconcile_with_the_world_settled_total` | 1,366 笔 / 390 个成交回合、0 例外 |
//! | 价格分解逐项重算（`mond_extra` / `freight_rate`，同一条定义式） | `trade_price_terms_are_self_consistent` | 1,366 笔全咬合；跨天体运费 732 / 同天体零运费 634 |
//! | 买方名次 = 引擎的购买力序（降序、同额按名字升序、名次 0..n-1） | `market_rank_is_the_engines_own_buying_order` | 1,200 个「排过队」的回合全是 0..n-1 的序 |
//!
//! **留在这里的**（按施工图 §4/§5 的两栏对账）：
//! * `trade_block_list_names_the_blocker_and_the_tier`——名单本身在 `factions[].trade_blocked_by`
//!   （读面有），但「与 `trade_block_cause` 同源」那半句要调**引擎函数**；把公式抄进 Python
//!   就是同一个数两个位置（施工图 §4 的「内部契约」）。
//! * `haul_steps_only_cover_ships_that_are_actually_on_a_haul_route`——要拿**回合末的内部
//!   指令叶**（`state.control[*].ship_orders`）反查「这艘舰此刻是不是真的在跑运输」；它判的是
//!   **运输族**（施工图 §5 第 2–4 批：货舱 / 货栈 / 集货腿）的完备性，按批次顺序与那几张表
//!   一起搬。
//! * `freight_gap_is_the_same_ledger_the_engine_posts_contracts_from`——要直接跑
//!   `autocontrol::freight::post_contracts` 并读 `RoundSink.freight_gap`：那本货栈账还没进
//!   读面（施工图 §5 第 3–4 批）。

use super::*;

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
            assert!(
                state.faction(blocker).is_some(),
                "{blocker} 不是这个世界的势力"
            );
            assert!(
                TIERS.contains(&cause.as_str()),
                "没见过这一档：{blocker} → {fid} = {cause}"
            );
            seen.insert(cause.clone());
            if cause == "war" {
                let pair = if fid < blocker {
                    (fid.clone(), blocker.clone())
                } else {
                    (blocker.clone(), fid.clone())
                };
                assert!(
                    view.wars.contains(&pair)
                        || view.wars.contains(&(pair.1.clone(), pair.0.clone())),
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
    assert!(
        entries >= 1,
        "20 回合里一次禁运都没有——这条守卫会退化成空转（见过 {seen:?}）"
    );
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
            let s = state
                .ship(ship)
                .unwrap_or_else(|| panic!("{ship} 不存在了，却是它记了运输动作"));
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
                assert!(
                    step.units() > 0.0,
                    "{ship}: 装卸动作必须真的搬了货（否则该记成 waiting）"
                );
            }
        }
    }
    assert!(
        rounds_with_haul >= 1,
        "12 回合里没有一艘舰跑过运输——用例构造失败（角色没撤掉？）"
    );
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
        let want = if need <= 0.0 {
            0.0
        } else {
            (unc / need).clamp(0.0, 1.0)
        };
        let got = autocontrol::freight::haul_gap(&state, &config, &fid);
        assert!(
            (got - want).abs() < 1e-9,
            "{fid}: haul_gap = {got}，但按读面那本账算是 {want}（两本账漂了）"
        );
        checked += 1;
    }
    assert!(checked >= 2, "只检查到 {checked} 个势力——守卫太空");
    assert!(
        uncovered_seen >= 1,
        "10 回合后一处积压缺口都没有——这条守卫没在检查东西"
    );
}
