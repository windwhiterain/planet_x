//! 市场与运输：B3 批——把「为什么是这个价 / 我买到的货为什么少了 / 有货在卖我却没买到 /
//!
//! ## 2026-10（第 7 批）：`freight_gap_is_the_same_ledger_the_engine_posts_contracts_from`
//! 搬去了 g1（新挂 `--call freight_ledger {faction}`）
//!
//! `capacity_ledger` 一本账供两处用（雇主挂单 + 「该不该腾船坞造货船」）⇒ 把它 `pub` 出来、
//! 平铺成 `--call`。判据 = 逐条自洽（`缺口 == max(0, need − own − hired)`，`need > 0`，
//! `own/hired ≥ 0`）**且** `Σ缺口 ÷ Σneed` **就是读面那一列** `factions.haul_gap`
//! （那一列过 `r2` ⇒ 容差 `0.0051`）。两个 seed、12 个势力·回合、28 处缺口全对得上。
//!
//! ## 2026-10（第 7 批）：`trade_block_list_names_the_blocker_and_the_tier` 搬去了 g2
//!
//! 投影 `factions` 新补了一列 **`贸易禁运`** = `{禁运方: 档位}`（以前只有 `--derived` 的
//! `metrics.factions[].trade_blocked_by` 读得到）。判据两半：
//! * **结构**（3 seed 共 **17,712** 条）：没有自己禁运自己、禁运方是真势力、档位在
//!   `war`/`cold`/`coalition` 里，且 `war` 档**两边关系真的 ≤ `combat.war_threshold`**；
//! * **同源复核**：挑一个回合逐条目问新挂的 `--call trade_block_cause`，要求逐字相等。
//!
//! ⚠ `war` 档 = **敌对**（`hostile` = 关系 ≤ 阈值），**不是**「宣战过」：第一版拿
//! `war_started`/`war_ended` 重建去对账，整片假红。
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
