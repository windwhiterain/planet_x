//! **雇佣运力市场**：接单、派工、考核、续约、解约。
//!
//! 雇主侧（挂单）在 [`crate::autocontrol::freight`] 的 `post_contracts`；这一半是**受雇方**
//! 与**雇主的验货**。合起来就是 `.agents/notes/freight-collection.md` §4 的雇佣形态，
//! 用户裁决：**单子以雇佣船的形式**、雇主挂单逻辑与派自己的船同源、单子要求的是**运力**、
//! 雇主**周期性考核**并据此反馈信誉、再按信誉决定是否切换受雇方、受雇方按自己的运力
//! 决定接不接 / 是否提前结束。加上更早的：**Q1(b)** 不赔货值只掉信誉 / **Q3+P4** 信誉是
//! **势力级的全局单值** / **Q10** 报酬是抽成 / **Q4** 禁运同样挡雇佣 / **船沉没不管**。
//!
//! # 一张单 ≠ 一条船（用户：「对方派几艘船都无所谓」）
//!
//! [`match_carriers`] 只决定**谁受雇**（势力级），**不押任何船**；派几条船、派哪条船是
//! [`assign_hired_ships`] 每回合按**缺口**抽签的事。于是：
//!
//! * 一张单可以同时跑好几条船（舱容小的船组队也能顶上一个运力要求）；
//! * 船被打沉**不需要任何特判**：那条派工关系下一回合自然消失，考核看的是**运输量**
//!   （用户：「船沉没不管，只管统计运输量」）——旧形态里那条「押上的舰没了 ⇒ 合同回挂单簿
//!   + 掉一次信誉」的机制**整个删掉**，因为它把「船沉了」这件事罚了两次（考核会照样判它
//!   交付不足）。
//!
//! # 三道闸，全是抽签（遵 `AGENTS.md`：概率分布，不设硬阈值）
//!
//! 1. **看得见吗**（[`eligibility`]）：`σ((信誉 − 门槛) ÷ 宽度)`——低信誉者**极少**接到活。
//! 2. **愿意接吗**（[`accept_chance`]）：`σ(决策值 ÷ 温度)`。决策值 = **这一期的报酬**
//!    `+ λ(信誉) × 预期信誉变化`，其中预期信誉变化由**我这条线上能凑出多少运力**决定
//!    （[`decision_value`]）：凑不够 ⇒ 接了就等着吃差评 ⇒ 低信誉者（λ 大）不接。
//!    这正是用户说的「受雇方根据当前运力决定是否接受雇佣」。
//! 3. **谁接**：多个愿意接 ⇒ **按信誉加权抽签**（信誉 = 竞争力，但不独占）。
//!
//! 另有两条**对称**的闸（同一把尺子的两面，[`assign_hired_ships`]）：自己缺船时
//! **既不再接新的活、也开始退掉旧的活**——`σ((缺几条船 − recall_bar) ÷ recall_width)`。
//!
//! # 决策值：`D = 报酬 + λ(信誉) × 预期信誉变化`（**注意符号**）
//!
//! §C1 原文把判据写成 `预期收益 ≥ λ(信誉) × 预期信誉变化`。实现时发现**那个符号是错的**：
//! 信誉变化为**负**（这单多半要砸）时，右边变成负数，不等式恒成立——于是「越危险的单
//! 越该接」，正好把设计想要的东西写反了。有意义的形态是**把信誉按影子价格折进价值里**：
//!
//! * 稳单（`Δ信誉 > 0`）：低信誉者（λ 大）拿到的附加值**大** ⇒ 靠稳妥履约攒信誉；
//! * 险单（`Δ信誉 < 0`）：λ 大时被**扣**得很痛 ⇒ 低信誉者不赌；
//! * 高信誉者 λ 已很小 ⇒ 稳单险单对他几乎是同一个数 ⇒ **目标变成多赚钱**。
//!
//! **λ 的量纲**：一期的报酬 ÷ 一期的信誉增量（见 `config.freight.shadow_lambda0`）。
//!
//! # 没有「养船成本」项
//!
//! §C1 的判据里没有成本，实现也不再添：本作的 `upkeep` 是**每回合都付**的持续开销，
//! 舰闲着也在付。所以「这次航行的边际成本」在模型里就是**空闲舰的机会成本 ≈ 0**，
//! 把整船维护费摊到单趟上会让所有远单（柯伊伯带往返几十回合）在算术上永远不划算
//! ——而那正是设计要它**可行但有风险**的地方（§1 的实测结论）。风险由信誉项承担。

use crate::autocontrol::freight;
use crate::model::*;
use crate::sim;

/// 逻辑斯蒂函数 `σ(x) = 1/(1+e^-x)`：把任意实数压到 `(0,1)`。
///
/// **本模块所有闸门都用它**（遵 `AGENTS.md`：默认用概率分布，不设硬阈值）：
/// 硬阈值会造出「跨过一条线就完全反过来」的不合理观感，σ 给的是**连续的难度**。
pub(crate) fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x.clamp(-30.0, 30.0)).exp())
}

/// 信誉的**影子价格** `λ(信誉)`：单调**递减**。
///
/// 信誉越低越稀缺 ⇒ 一点信誉越值钱 ⇒ 越舍不得拿它去冒险（只接稳单）。
/// 它是 §C1 那条循环（新人攒信誉 → 够门槛接难单 → 失手回落 → 自动退回谨慎档）的支点：
/// 两端各有进入门槛、也各有退出代价，所以既不会全员躺平接易单，也不会全员赌命接难单。
pub fn shadow_price(config: &GameConfig, reputation: f64) -> f64 {
    let f = &config.freight;
    f.shadow_lambda0 * sigmoid((f.shadow_mid - reputation) / f.shadow_width.max(1e-9))
}

/// **合格度**（雇主那一侧的闸）：`σ((信誉 − 门槛) ÷ 宽度)` ∈ `(0,1)`。
///
/// 它是**被看见的概率**：每回合对每个 `(单, 受雇方)` 掷一次派生骰子（[`sim::derived_roll`]，
/// 不消费主 `Prng`），落在合格度之内这份活才对它可见，否则它这一回合"没听说这单"。
/// 于是低信誉者**极少**接到活，但**不是**被永久排除——攒够信誉自然就看得见了。
///
/// 固定期到期后的**续约闸也是它**（[`settle_contracts`]）：雇主不需要为「换人」再造一个阈值
/// ——「你当初是怎么被选上的，现在就按同一条线续」。
pub fn eligibility(config: &GameConfig, reputation: f64, c: &Contract) -> f64 {
    sigmoid((reputation - c.min_reputation) / config.freight.gate_width.max(1e-9))
}

/// 本势力此刻能投到**某条线**上的空闲运力之和（单位/回合）。
///
/// 口径 = [`freight::trip_throughput`]（舱容 × 每回合能跑几趟，**距离已经折在里面**）之和。
/// 三处排除，各有理由：舱里有货的舰得先把自己那票送完；已经替别人跑的舰一次只能跑一条线；
/// 玩家开的舰（`ship_control != Auto`）AI 不替玩家派活。
///
/// 它是「受雇方根据当前运力决定是否接受雇佣」那把尺子——**加总**而不是挑一条船，
/// 因为一份单要的是**运力**，几条小船凑起来也算数（用户：「对方派几艘船都无所谓」）。
pub fn available_throughput(
    state: &State,
    config: &GameConfig,
    fid: &str,
    from: &str,
    to: &str,
) -> f64 {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .filter(|s| state.ship_control(s.name.clone()) == ControlMode::Auto)
        .filter(|s| state.contracts.assignment_of(&s.name).is_none())
        .filter(|s| s.cargo.is_empty())
        .map(|s| freight::trip_throughput(state, config, s, from, to))
        .sum()
}

/// 受雇方接下这单的**决策值** `D`（价值单位）。符号约定见模块文档。
///
/// * **报酬** = 抽成 × **一期的运力**（`capacity × 考核周期` = 一个来回该搬回的货）× 单位价值。
///   按**一期**算而不是按整期：与信誉风险的刻度对齐——一次考核加减 [`GameConfig`] 里的
///   `reputation_gain`，所以两边是「同一期中」的报酬与风险。
/// * **能不能达标** `p_ok = σ((我能凑出的运力 − 要求运力) ÷ (要求运力 × review_width))`：
///   勉强够用的就五五开，绰绰有余的几乎必然达标。旧形态这里算的是「时间够不够」
///   （截止期 ÷ 要跑几回合），雇佣形态里没有截止期，换成**运力够不够**。
/// * **预期信誉变化** = `reputation_gain × (2 p_ok − 1)`：达标无望时它是负的，乘上 λ 就是
///   「接这单会砸掉多少名声」的价钱。
/// * **λ** 见 [`shadow_price`]。
pub fn decision_value(state: &State, config: &GameConfig, c: &Contract, fid: &str) -> f64 {
    let terms = hire_terms(state, config, &c.from, &c.to);
    let unit = config
        .resources
        .get(&c.resource)
        .map(|r| r.value)
        .unwrap_or(1.0);
    // 一期的报酬：这条线一个考核期该搬回的货（= capacity × interval = nominal_hold）× 抽成 × 单价。
    let reward = c.share * c.capacity * terms.interval as f64 * unit;
    let mine = available_throughput(state, config, fid, &c.from, &c.to);
    let width = (c.capacity * config.freight.review_width.max(1e-9)).max(1e-9);
    let p_ok = sigmoid((mine - c.capacity) / width);
    let d_rep = config.freight.reputation_gain * (2.0 * p_ok - 1.0);
    let rep = state.faction(fid).map(|f| f.reputation).unwrap_or(0.0);
    reward + shadow_price(config, rep) * d_rep
}

/// **接单概率** = `σ(D ÷ 温度)`：不是「划一条线过或不过」，而是**越划算越可能接**。
pub fn accept_chance(state: &State, config: &GameConfig, c: &Contract, fid: &str) -> f64 {
    let d = decision_value(state, config, c, fid);
    sigmoid(d / config.freight.match_temperature.max(1e-6))
}

/// **每回合的撮合**：把挂单簿上还没人接的单派给愿意接、也够格接的受雇方。
///
/// 逐单处理、**按单号序**（先挂的先被接：一张单不会因为不断有新单出现而永远轮不到）。
/// 每张单三步，全部掷**派生骰子**（`(势力, 单号, 回合, 用途)`，不消费主 `Prng` 流）：
///
/// 1. **看得见吗**：雇主的门槛（[`eligibility`]）——一条船都派不出的势力直接跳过（不掷骰）；
/// 2. **愿意接吗**：自评闸（[`accept_chance`]）；
/// 3. **谁接**：多个愿意接 ⇒ **按信誉加权抽签**（信誉 = 竞争力，但不独占）。
///
/// 中选者只是**受雇**（`carrier` 落定 + 雇佣期起算）：**不押船**。派工是下一步
/// [`assign_hired_ships`] 的事——那张单要几条船、派哪几条，是受雇方自己的内部事务。
pub(crate) fn match_carriers(
    state: &mut State,
    config: &GameConfig,
    inputs: &mut RoundInputs,
) {
    let mut fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort(); // 确定性：候选顺序不依赖势力表的排列
    let open: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_open())
        // 还挂着船的未接单**不对外招募**：那几条船正在把上一段关系的最后一趟货运完
        //（见 `end_contract` 的「舱里有货的船不放」）。再招一家人来跑同一条线，会让
        // 「这一趟是谁跑的」变得没法记账。
        .filter(|c| state.contracts.ships_of(c.id).is_empty())
        .map(|c| c.id)
        .collect();
    for id in open {
        // 每单重新取一遍状态：前面成交的单会占掉运力（`available_throughput` 看得见）。
        let Some(c) = state.contracts.get(id).cloned() else {
            continue;
        };
        if !c.is_open() {
            continue;
        }
        let round = state.round;
        let key = format!("契约{id}");
        // 1) + 2)：谁看得见、谁愿意接。
        let mut willing: Vec<(FactionId, f64)> = Vec::new();
        for fid in &fids {
            if *fid == c.shipper {
                continue; // 自己给自己运不算雇佣（挂单的意义就是请人）
            }
            // **禁运同样挡雇佣**（Q4，用户裁决）：全面禁运是「根本不卖给你」——那就不该
            // 还能雇对方的船来搬货（比不卖矿更狠的一条，见 `.agents/notes/trade-and-sanctions.md`）。
            // 判据与商品市场**同源**（`sim::trade_blocked`），所以「为什么断供」在两处一致。
            if sim::trade_blocked(state, config, &c.shipper, fid) {
                continue;
            }
            if available_throughput(state, config, fid, &c.from, &c.to) <= 0.0 {
                continue; // 一条船都派不出来 ⇒ 物理上接不了（不是「不太愿意」）
            }
            let rep = state.faction(fid).map(|f| f.reputation).unwrap_or(0.0);
            // **两道闸门都记账**（B5）：低信誉者「没听说这单」与「听说了但不想接」是两件
            // 不同的事——只记事件的话，读面只看得见「最后谁接了」，看不见「谁根本没听说」。
            let gate = eligibility(config, rep, &c);
            let gate_roll = sim::derived_roll(fid, &key, round, "gate");
            inputs.record_gate(
                "gate",
                fid,
                &key,
                gate_roll,
                gate,
                if gate_roll < gate { "heard" } else { "unheard" },
            );
            if gate_roll >= gate {
                continue; // 这一回合没"听说"这单（低信誉者极少看见）
            }
            let accept = accept_chance(state, config, &c, fid);
            let accept_roll = sim::derived_roll(fid, &key, round, "accept");
            inputs.record_gate(
                "accept",
                fid,
                &key,
                accept_roll,
                accept,
                if accept_roll < accept {
                    "willing"
                } else {
                    "unwilling"
                },
            );
            if accept_roll >= accept {
                continue; // 看见了但不想接（越不划算越可能不接）
            }
            willing.push((fid.clone(), rep.max(1e-3)));
        }
        if willing.is_empty() {
            continue;
        }
        // 3) 多家愿意接 ⇒ 按信誉加权抽签：信誉高者更可能拿到，但不是必然。
        let total: f64 = willing.iter().map(|(_, w)| *w).sum();
        let pick_roll = sim::derived_roll("", &key, round, "pick");
        let mut x = pick_roll * total;
        let mut chosen = willing.last().cloned().expect("willing 非空");
        for (fid, w) in &willing {
            if x < *w {
                chosen = (fid.clone(), *w);
                break;
            }
            x -= w;
        }
        let (carrier, _) = chosen;
        // **候选池（B5c）**：愿意接的每家各占多少权重——「为什么是它拿到了」= 信誉加权。
        let pool: Vec<crate::model::PoolEntry> = willing
            .iter()
            .map(|(f, w)| crate::model::PoolEntry {
                name: f.clone(),
                weight: *w,
            })
            .collect();
        inputs.record_draw("pick", &carrier, &key, pick_roll, total, &carrier).pool = pool;
        let terms = hire_terms(state, config, &c.from, &c.to);
        if let Some(cc) = state.contracts.get_mut(id) {
            cc.carrier = Some(carrier.clone());
            cc.accepted_round = Some(round);
            cc.expires_round = round.saturating_add(terms.term);
            cc.review_round = round.saturating_add(terms.interval);
            cc.delivered = 0.0;
            cc.served_rounds = 0;
        }
        sim::ev(
            state,
            GameEvent::ContractAccepted {
                contract: id,
                shipper: c.shipper.clone(),
                carrier,
                resource: c.resource.clone(),
                capacity: c.capacity,
                from: c.from.clone(),
                to: c.to.clone(),
            },
        );
    }
}

/// **本势力此刻能派去跑雇佣线的空闲舰**（按舰名序 ⇒ 确定性；**只含空舱的**）。
///
/// 舱里有货的舰不算空闲：它得先把自己那票送完（否则货烂在舱里，而且那票货的**归属**
/// 已经定了——`sim::cargo_owner` 认的是派工记录，中途改派会把别人的货卸进自己的池子）。
fn idle_ships(state: &State, fid: &str) -> Vec<ShipId> {
    let mut v: Vec<ShipId> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .filter(|s| state.ship_control(s.name.clone()) == ControlMode::Auto)
        .filter(|s| s.cargo.is_empty())
        .filter(|s| state.contracts.assignment_of(&s.name).is_none())
        .map(|s| s.name.clone())
        .collect();
    v.sort();
    v
}

/// 某张单**此刻到位的运力**（在跑的舰的吞吐之和，单位/回合）。
fn hired_throughput(state: &State, config: &GameConfig, c: &Contract) -> f64 {
    state
        .contracts
        .ships_of(c.id)
        .iter()
        .filter_map(|s| state.ship(s.as_str()))
        .filter(|s| s.hull > 0.0)
        .map(|s| freight::trip_throughput(state, config, s, &c.from, &c.to))
        .sum()
}

/// 本势力的**船只余缺**（船数尺度）：`(多出来几条, 缺几条)`。
///
/// 口径是**整支舰队**（`Auto` 开的、动得了的舰，**不管此刻有没有借出去**），与「有积压的
/// 货栈数」（既有的定编规则 Q8）比：
///
/// * `spare > 0` ⇒ 有富余的船，**可以借出去**（[`assign_hired_ships`] 第一段）；
/// * `deficit > 0` ⇒ 自己的线都跑不过来，**该把手上的雇佣退掉**（第二段）。
///
/// **为什么按整支舰队算、而不是按「此刻没派工的船」算**：后者会让借出这个动作本身改变
/// 结论——把唯一一条富余的船借出去，下一回合就"变成缺船"，于是退约、船回来、再借出去，
/// 来回震荡（实测 seed 7 在 400 回合里出现过 263 次这种退约）。按舰队算，**借出不再改变
/// 余缺**，而「自己缺不缺船」本来就与「借没借出去」无关：`需求运力 − 自有运力` 才是雇主
/// 那边同一把尺子的另一面（`freight::post_contracts`）。
fn own_ship_balance(state: &State, config: &GameConfig, fid: &str) -> (f64, f64) {
    let needed = freight::needed_haulers(state, config, fid);
    let fleet = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .filter(|s| state.ship_control(s.name.clone()) == ControlMode::Auto)
        .filter(|s| freight::freight_tonnage(config, s) > 0.0)
        .count() as f64;
    ((fleet - needed).max(0.0), (needed - fleet).max(0.0))
}

/// **每回合的派工**：受雇方把自己的空闲船派到手上的雇佣单上，缺船时又<b>收回</b>它们。
///
/// # 一件事的两面（用户裁决的落点）
///
/// > 受雇方也会根据当前运力决定是否接受雇佣，是否提前结束雇佣。
///
/// 两面共用**同一把尺子**：`p = σ((余缺 − recall_bar) ÷ recall_width)`，其中余缺按
/// [整支舰队](own_ship_balance)算。
///
/// * **补人**（第一段）：这张单还缺运力 ⇒ 每艘空闲船以
///   `(缺口 ÷ 要求运力) × 有富余的概率` 被派上去。`缺口 → 0` 时概率自动归零 ⇒ **不会
///   过度承诺**（不需要再写一条「派够了就别派」的规则）；自家没有富余的船时概率变小 ⇒
///   空闲船**优先留给自己**（先顾自己——这不是特判，而是同一个 σ 的另一侧）。
/// * **抽手**（第二段）：自家缺 `k` 条船 ⇒ 每回合想退掉 `k` 份合约，按 `p` 掷骰，
///   退的是**最不划算的那几份**（`share × capacity` 小者先退）。
///   一次只退「缺的那几份」而不是「全部」——否则缺一条船会让它一口气退掉所有合约（踩踏）。
///
/// # 提前结束也要**结清这一期**
///
/// 抽手的人在离开前会**当场被考核一次**（[`review_contract`]）：不然「这一期干砸了」的
/// 最优解就是赶在考核之前跑掉——那不是市场，那是逃单。
///
/// **舱里有货的船不放**（见 [`end_contract`]）：那票货的货主是雇主，人可以先走、货得送到。
///
/// 顺序上：**先补人再抽手**（这一回合刚接下的活也应该有机会立刻派人）。每个势力按名字序、
/// 每张单按单号序处理 ⇒ 与势力表的排列无关，确定性。
pub(crate) fn assign_hired_ships(
    state: &mut State,
    config: &GameConfig,
    inputs: &mut RoundInputs,
) {
    let mut fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort();
    let unit = config.freight.recall_width.max(1e-9);
    let bar = config.freight.recall_bar;
    for fid in fids {
        let idle = idle_ships(state, &fid);
        let round = state.round;
        let (spare, deficit) = own_ship_balance(state, config, &fid);
        let lend = sigmoid((spare - bar) / unit);
        // --- 第一段：补人 ---------------------------------------------------
        let held: Vec<u64> = state
            .contracts
            .carried_by(&fid)
            .iter()
            .map(|c| c.id)
            .collect();
        let mut free: Vec<ShipId> = idle;
        for id in held {
            let Some(c) = state.contracts.get(id).cloned() else {
                continue;
            };
            if c.capacity <= 1e-9 {
                continue;
            }
            for ship in free.clone() {
                let shortfall = (c.capacity - hired_throughput(state, config, &c)).max(0.0);
                if shortfall <= 1e-9 {
                    break; // 已经够了：不再派人（缺口为 0 ⇒ 概率也为 0，这里只是省一趟计算）
                }
                let p = (shortfall / c.capacity).clamp(0.0, 1.0) * lend;
                let key = format!("派工{id}");
                let roll = sim::derived_roll(&fid, &key, round, &ship);
                inputs.record_gate(
                    "assign",
                    &fid,
                    &format!("{key}:{ship}"),
                    roll,
                    p,
                    if roll < p { "assigned" } else { "skipped" },
                );
                if roll < p {
                    state.contracts.assign(ship.clone(), id);
                    free.retain(|s| *s != ship);
                }
            }
        }
        // --- 第二段：抽手（提前结束雇佣）--------------------------------------
        // 每缺一条船，就想退掉**一份**最不划算的合约（`share × capacity` 小者先退）。
        let mut quota = deficit.round() as u32;
        if quota == 0 {
            continue;
        }
        let quit = sigmoid((deficit - bar) / unit);
        let mut ranked: Vec<(u64, f64)> = state
            .contracts
            .carried_by(&fid)
            .iter()
            .map(|c| (c.id, c.share * c.capacity))
            .collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        for (id, _) in ranked {
            if quota == 0 {
                break;
            }
            let key = format!("退约{id}");
            let roll = sim::derived_roll(&fid, &key, round, "quit");
            inputs.record_gate(
                "quit",
                &fid,
                &key,
                roll,
                quit,
                if roll < quit { "quit" } else { "kept" },
            );
            if roll >= quit {
                continue;
            }
            review_contract(state, config, id, inputs); // 离开前结清这一期
            end_contract(state, id, "recalled");
            quota -= 1;
        }
    }
}

/// **交付记账**：受雇方的一条船在雇主的天体卸下 `loaded` 单位（**卸出舱的总量**，含自留的抽成）。
///
/// 只做两件事：
/// 1. 记进度（`delivered += loaded`）——**是卸出舱的总量**，不是雇主实收的量。抽成是搬运费，
///    不能从**运力**里扣：否则每一趟都天生差 15%，而要求运力是按「一条参考船」定的，
///    那 15% 会变成系统性的「不达标」；
/// 2. 发 `contract_delivered` 事件（带上受雇方自留的抽成：那是它的**全部报酬**）。
///
/// **这里不动信誉**：雇佣形态下信誉只由雇主的**周期考核**产生（用户裁决）。旧形态把涨信誉
/// 挂在每一次交付上，于是「搬得多」直接等于「名声好」——那让雇主失去了「我雇的这条线到底
/// 有没有达标」这个判断，而这正是新形态要问的问题。
pub(crate) fn on_delivery(state: &mut State, ship_id: &str, loaded: f64, cut: f64) {
    let Some(id) = state.contracts.assignment_of(ship_id) else {
        return;
    };
    let Some(c) = state.contracts.get(id).cloned() else {
        return;
    };
    let Some(carrier) = c.carrier.clone() else {
        return;
    };
    if let Some(cc) = state.contracts.get_mut(id) {
        cc.delivered += loaded;
    }
    sim::ev(
        state,
        GameEvent::ContractDelivered {
            contract: id,
            shipper: c.shipper,
            carrier,
            ship: ship_id.to_string(),
            resource: c.resource,
            amount: (loaded - cut).max(0.0),
            cut,
        },
    );
}

/// 改一个势力的信誉（夹在 `[0, reputation_max]`）。
///
/// **下限是 0 而不是负数**：信誉没有「欠债」的语义——它是准入资产，不是钱包。
/// 归零者仍然可以接活（门槛是 σ 软化的），只是几乎接不到贵的难的。
fn add_reputation(state: &mut State, config: &GameConfig, fid: &str, delta: f64) {
    if let Some(f) = state.factions.iter_mut().find(|f| f.name == fid) {
        f.reputation = (f.reputation + delta).clamp(0.0, config.freight.reputation_max.max(0.0));
    }
}

/// **好评概率**：达标率越高越可能好评——`σ((达标率 − 1) ÷ review_width)`。
///
/// 1.0 = 恰好是一条参考船的水准 ⇒ 五五开（雇主既不觉得吃亏也不觉得占了便宜）。
/// 它是**连续**的：0.9 与 1.1 的差别只是概率的差别，不是「过线/没过线」的断崖
/// （遵 `AGENTS.md`）。
pub fn review_chance(config: &GameConfig, ratio: f64) -> f64 {
    sigmoid((ratio - 1.0) / config.freight.review_width.max(1e-9))
}

/// **雇主的周期考核**：验这一期的**实测吞吐**，掷一次骰子给好评或差评，据此改信誉。
///
/// * **达标率** = [`Contract::throughput_ratio`] = `实交 ÷ (要求运力 × 有货回合数)`。
///   分母是**有货可运的回合数**而不是「过了几个回合」：没货可运的回合不该算在受雇方头上
///   （那会变成罚它没搬不存在的货）。一个「有货可运回合」都没有 ⇒ **这一期不评**
///   （不发事件、不动信誉）——不是考零分。
/// * **好评/差评是掷出来的**（[`review_chance`] 是概率）：期望随达标率严格递增，但
///   一条好船偶尔也会吃差评、一条烂船偶尔也会蒙到好评。信誉因此是**市场信号**而不是判决书。
/// * 事件带上 `ratio` / `good` / `delta`，所以「为什么它的信誉掉了」在流水账里查得到。
///
/// 返回是否真的评了（供调用处决定要不要排下一次）。
fn review_contract(
    state: &mut State,
    config: &GameConfig,
    id: u64,
    inputs: &mut RoundInputs,
) -> bool {
    let Some(c) = state.contracts.get(id).cloned() else {
        return false;
    };
    let Some(carrier) = c.carrier.clone() else {
        return false;
    };
    let Some(ratio) = c.throughput_ratio(config) else {
        return false;
    };
    let chance = review_chance(config, ratio);
    let review_key = format!("考核{id}");
    let roll = sim::derived_roll(&c.shipper, &review_key, state.round, "review");
    inputs.record_gate(
        "review",
        &c.shipper,
        &review_key,
        roll,
        chance,
        if roll < chance { "good" } else { "poor" },
    );
    let good = roll < chance;
    let delta = if good {
        config.freight.reputation_gain
    } else {
        -config.freight.reputation_gain
    };
    add_reputation(state, config, &carrier, delta);
    sim::ev(
        state,
        GameEvent::ContractReviewed {
            contract: id,
            shipper: c.shipper,
            carrier,
            ratio,
            good,
            delta,
        },
    );
    true
}

/// **结束一份雇佣关系**：合同**回到挂单簿**（`carrier = None` + 清掉本期进度），
/// 受雇方的空闲船全部放回去跑它自己的线，并发 `contract_ended`。
///
/// 三种由来见 [`GameEvent::ContractEnded`]：固定期到期不续约（`term`）、整期没有产出
/// （`no_output`）、受雇方自己缺船提前结束（`recalled`）。**都不是「违约」**：雇佣形态下
/// 没有超期也没有丢单，代价只有考核掉的那些信誉。
///
/// # **舱里有货的船不放**
///
/// 那票货是从**雇主的**货栈装的，卸到哪儿由派工记录决定（`sim::cargo_owner` 认的就是它）。
/// 关系结束的那一刻如果连货带船一起"还给"受雇方，那票货就成了**受雇方自己的**——卸进它
/// 自己的池子，等于把雇主的货偷走。所以：
///
/// * **放掉空舱的船**（它们本来就在替别人跑，早该回去跑自己的线）；
/// * **留着满载的船**——它把这趟跑完、卸进雇主的池子；卸完变空之后由巡检放掉
///   （见 [`settle_contracts`] 第 0 步），那期间那张单**不再对外招募**
///   （[`match_carriers`] 跳过还挂着船的未接单），所以不会出现「两个受雇方跑同一张单」。
fn end_contract(state: &mut State, id: u64, reason: &str) {
    let Some(c) = state.contracts.get(id).cloned() else {
        return;
    };
    let Some(carrier) = c.carrier.clone() else {
        return;
    };
    let homebound: Vec<ShipId> = state
        .contracts
        .ships_of(id)
        .into_iter()
        .filter(|s| {
            state
                .ship(s.as_str())
                .map(|sh| sh.cargo.is_empty() || sh.hull <= 0.0)
                .unwrap_or(true)
        })
        .collect();
    for s in homebound {
        state.contracts.unassign(&s);
    }
    if let Some(cc) = state.contracts.get_mut(id) {
        cc.carrier = None;
        cc.accepted_round = None;
        cc.expires_round = 0;
        cc.review_round = 0;
        cc.delivered = 0.0;
        cc.served_rounds = 0;
        // 回到挂单簿 ⇒ 从此刻重新起叫（抬价的节拍也从这一刻算），否则它一挂出来就
        // 立刻触发一次加价（`posted_round` 还停在很久以前）。
        cc.posted_round = state.round;
    }
    sim::ev(
        state,
        GameEvent::ContractEnded {
            contract: id,
            shipper: c.shipper,
            carrier,
            reason: reason.into(),
        },
    );
}

/// **每回合的巡检**：记考核分母 → 到点考核 → 固定期到期（续约或换人）→ 清理。
///
/// 交付不在这里——它发生在 [`crate::sim::haul_unload`] 那一刻（船真的到了泊位）。
/// 这里处理的是**时间的后果**，每回合开头跑一次（`sim::step_contracts`），顺序有意如此：
/// **先记分母再考核**，所以「这一回合货栈有货」也算进这一期。
pub(crate) fn settle_contracts(
    state: &mut State,
    config: &GameConfig,
    inputs: &mut RoundInputs,
) {
    // 0) **清理**：派工指向的舰没了（被击沉/报废/退役）⇒ 抹掉那条记录。
    //
    //    这里**没有事件、没有惩罚**（用户：「船沉没不管，只管统计运输量」）：船沉了这件事
    //    会由考核自己说话——这一期少搬的那些货就是代价。旧形态在这里发 `contract_lost`
    //    并罚一次信誉，等于把同一件事罚了两遍。
    let gone: Vec<ShipId> = state
        .contracts
        .assignments
        .iter()
        .filter(|(s, _)| {
            !state
                .ship(s.as_str())
                .map(|sh| sh.hull > 0.0)
                .unwrap_or(false)
        })
        .map(|(s, _)| s.clone())
        .collect();
    for s in gone {
        state.contracts.unassign(&s);
    }
    // 0b) **在途收尾**：关系已经结束（合同回到挂单簿）但船上还载着那趟货的船，卸完就该放掉
    //     ——它在替一张**已经不属于任何人**的单跑（见 [`end_contract`] 的「舱里有货的船不放」）。
    let leftover: Vec<ShipId> = state
        .contracts
        .assignments
        .iter()
        .filter(|(s, id)| {
            state
                .contracts
                .get(**id)
                .map(|c| c.is_open())
                .unwrap_or(false)
                && state
                    .ship(s.as_str())
                    .map(|sh| sh.cargo.is_empty())
                    .unwrap_or(true)
        })
        .map(|(s, _)| s.clone())
        .collect();
    for s in leftover {
        state.contracts.unassign(&s);
    }
    // 1) **记分母**：已受雇、且**这条腿有活**的合同 ⇒ 这一回合算一个「有货可运的回合」。
    //    放在 `step_production` 之后（`step_contracts` 的位置）+ 装卸之前，所以这一回合
    //    刚产出的货也算数。没有活的回合不进分母：那不是受雇方的错。
    //
    //    **两个方向共用同一把尺子**（[`freight::lane_has_work`]）：
    //    * 出口腿（产地 → 首都）：那处货栈还有**净剩余**（扣掉本地建设要用的）；
    //    * 进口腿（首都 → 站点）：那处**还缺**、且首都真的拿得出那些货。
    //    从前这里只看「起运货栈有没有货」——进口腿的起运端是**首都池**（不是货栈），
    //    按老口径它一回合都不会被算进分母，达标率会天生虚高。
    let stocked: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_hired())
        .filter(|c| freight::lane_has_work(state, config, &c.shipper, &c.from, &c.to))
        .map(|c| c.id)
        .collect();
    for id in stocked {
        if let Some(c) = state.contracts.get_mut(id) {
            c.served_rounds = c.served_rounds.saturating_add(1);
        }
    }
    // 2) **到点考核**：每个考核周期（这条线的一个往返）验一次货。
    let due: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_hired() && state.round >= c.review_round)
        .map(|c| c.id)
        .collect();
    for id in due {
        review_contract(state, config, id, inputs);
        // 排下一次考核。到期那一回合的排期随后被「续约/结束」覆盖，所以这里不必特判。
        if let Some(c) = state.contracts.get(id).cloned() {
            let interval = hire_terms(state, config, &c.from, &c.to).interval;
            if let Some(cc) = state.contracts.get_mut(id) {
                cc.review_round = state.round.saturating_add(interval);
            }
        }
    }
    // 3) **固定期到期**：雇主**按信誉决定**续约还是换人。
    //
    //    续约闸就是当初那条准入闸（[`eligibility`]）：你当初是怎么被选上的，现在就按同一条
    //    线续——不需要再发明一个「什么样的信誉才配续约」的阈值。整期一件货都没搬
    //    （`delivered == 0`）的不续约：那不是信誉问题，是这条线上没有产出（`no_output`）——
    //    若货栈**有货而它就是没搬**，这一期早就在考核里吃过差评了（账期一够就评）。
    let expired: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_expired(state.round))
        .map(|c| c.id)
        .collect();
    for id in expired {
        let Some(c) = state.contracts.get(id).cloned() else {
            continue;
        };
        let Some(carrier) = c.carrier.clone() else {
            continue;
        };
        let rep = state.faction(&carrier).map(|f| f.reputation).unwrap_or(0.0);
        let no_output = c.delivered <= 1e-9;
        // 整期一件货都没搬 ⇒ **不掷骰**（不是信誉问题）——所以这一档不记账是**对的**：
        // 输入面记的是「掷了什么」，没掷就是没有。
        let renew_key = format!("续约{id}");
        let renew_line = eligibility(config, rep, &c);
        let renew_roll = sim::derived_roll(&c.shipper, &renew_key, state.round, "renew");
        if !no_output {
            inputs.record_gate(
                "renew",
                &c.shipper,
                &renew_key,
                renew_roll,
                renew_line,
                if renew_roll < renew_line {
                    "renewed"
                } else {
                    "replaced"
                },
            );
        }
        let renew = !no_output && renew_roll < renew_line;
        if renew {
            // 续约**不发事件**：同一份关系继续，合同条款一个字都没变（只有计时器重置）。
            // 「这一刻发生了什么」已经由这一回合的 `contract_reviewed` 说了。
            let terms = hire_terms(state, config, &c.from, &c.to);
            if let Some(cc) = state.contracts.get_mut(id) {
                cc.accepted_round = Some(state.round);
                cc.expires_round = state.round.saturating_add(terms.term);
                cc.review_round = state.round.saturating_add(terms.interval);
                cc.delivered = 0.0;
                cc.served_rounds = 0;
            }
        } else {
            end_contract(state, id, if no_output { "no_output" } else { "term" });
        }
    }
}

#[cfg(test)]
#[path = "../tests/autocontrol/contract.rs"]
mod tests;
