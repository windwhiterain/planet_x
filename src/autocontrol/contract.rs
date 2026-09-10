//! **承包市场的承运方**：谁接单、接哪一单、凭什么。
//!
//! 挂单侧（托运方）在 [`crate::autocontrol::freight`] 的 `post_contracts`；这一半是承运方。
//! 两半合起来就是 `.agents/notes/freight-collection.md` §4 的「设计 C」，用户裁决：
//! **Q1(b)** 承运人不赔货值、只掉信誉 / **Q2** 托运方挂单 / **Q4** 禁运同样挡承包 /
//! **Q10** 报酬是抽成 / **Q11** 超期不作废 / §C5 撮合**全部概率化**。
//!
//! # 两道闸，都是「抽签」而不是「划线」
//!
//! 1. **托运方的信誉门槛**（[`eligibility`]）：`σ((信誉 − 门槛) ÷ 宽度)`。Q1(b) 之后
//!    这是托运方**唯一**的自我保护（承运人不赔货值 ⇒ 押在陌生人手里的是它全部货值），
//!    所以门槛必须**真的挡人**；但仍然是概率——低信誉者**极少**被选中，不是数学上绝无可能。
//! 2. **承运方的自评闸**（[`accept_chance`]）：`σ(决策值 ÷ 温度)`——**略亏的单也会有人接**，
//!    而不是「净收益 ≥ 0 才接」的一刀切。
//!
//! 多个愿意接的承运人之间**按信誉加权抽签**（[`match_carriers`]）：信誉 = 竞争力，
//! 但高信誉不等于独占（那会变成赢家通吃的断崖）。
//!
//! # 决策值：`D = 报酬 + λ(信誉) × 预期信誉变化`（**注意符号**）
//!
//! §C1 原文把判据写成 `预期收益 ≥ λ(信誉) × 预期信誉变化`。实现时发现**那个符号是错的**：
//! 信誉变化为**负**（这单很可能超期）时，右边变成负数，不等式恒成立——于是「越危险的单
//! 越该接」，正好把设计想要的东西写反了。
//!
//! 有意义的形态是**把信誉按影子价格折进价值里**（λ 的单位 = 「一点信誉值多少市场价值」）：
//!
//! ```text
//! D = 报酬 + λ(信誉) × 预期信誉变化
//! ```
//!
//! * 稳单（`Δ信誉 > 0`）：低信誉者（λ 大）拿到的附加值**大** ⇒ 靠稳妥履约攒信誉；
//! * 险单（`Δ信誉 < 0`）：λ 大时被**扣**得很痛 ⇒ 低信誉者不赌；
//! * 高信誉者 λ 已很小 ⇒ 稳单险单对他几乎是同一个数 ⇒ **目标变成多赚钱**。
//!
//! 这正是 §C1 那张表（低信誉攒信誉 / 高信誉多赚钱）的机制落点，而且不需要再写一条特判。
//!
//! **λ 的量纲**：一次典型承包交付的报酬 ÷ 一次交付的信誉增量（见 `config.freight.
//! shadow_lambda0` 的注解）。把 λ 调成 1.0 那样的「小数字」会让信誉项在价值面前**完全
//! 不可见**（报酬 10 上下、信誉变化 0.08 上下）——那样这套阶梯就白写了。
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

/// **合格度**（托运方那一侧的闸）：`σ((信誉 − 门槛) ÷ 宽度)` ∈ `(0,1)`。
///
/// 它是**被看见的概率**：每回合对每个 `(单, 承运人)` 掷一次派生骰子（[`sim::derived_roll`]，
/// 不消费主 `Prng`），落在合格度之内这张单才对它可见，否则它这一回合"没听说这单"。
/// 于是低信誉者**极少**接到难单，但**不是**被永久排除——攒够信誉自然就看得见了。
pub fn eligibility(config: &GameConfig, reputation: f64, c: &Contract) -> f64 {
    sigmoid((reputation - c.min_reputation) / config.freight.gate_width.max(1e-9))
}

/// 承运人接下这单的**决策值** `D`（价值单位）。符号约定见模块文档。
///
/// * **报酬** = 抽成 × 还没送到的量 × 单位价值（Q10：承运人的报酬**就是它没交出去的那部分货**）；
/// * **按时概率** `p`：剩下的时间 ÷ 这单要跑几个回合（用**承运人自己的船**算，
///   含 MOND 导航的期望尝试次数——掌握 MOND 的人在同一条线上期望回合数低一个数量级）；
/// * **预期信誉变化** = `p × 涨 − (1−p) × 跌`（超期只扣一次，Q11）；
/// * **λ** 见 [`shadow_price`]。
pub fn decision_value(
    state: &State,
    config: &GameConfig,
    c: &Contract,
    fid: &str,
    ship_id: &str,
) -> f64 {
    let Some(ship) = state.ship(ship_id) else { return 0.0 };
    let panel = ship_panel(config, ship);
    let master = sim::is_mond_master(config, fid);
    // 这单要跑几个回合：**用承运人自己的船**算，且按**趟数**算（单量 47 件 vs 舱容 4 ⇒ 十几趟）。
    let rounds =
        job_rounds(state, config, c, panel.speed, cargo_capacity(config, ship), master);
    // 按时概率：时间不够就是 0（这单接了就注定掉信誉），时间宽裕就是 1。
    let p = if rounds.is_finite() && rounds > 0.0 {
        ((c.deadline as f64 - state.round as f64) / rounds).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let reward = c.share * value_of_amount(config, &c.resource, c.outstanding());
    let d_rep = config.freight.reputation_gain * p
        - config.freight.reputation_late_penalty * (1.0 - p);
    let rep = state.faction(fid).map(|f| f.reputation).unwrap_or(0.0);
    reward + shadow_price(config, rep) * d_rep
}

/// **接单概率** = `σ(D ÷ 温度)`：不是「划一条线过或不过」，而是**越划算越可能接**。
pub fn accept_chance(
    state: &State,
    config: &GameConfig,
    c: &Contract,
    fid: &str,
    ship_id: &str,
) -> f64 {
    let d = decision_value(state, config, c, fid, ship_id);
    sigmoid(d / config.freight.match_temperature.max(1e-6))
}

/// 本势力**最适合跑这条线**的空闲运输舰（`None` = 没有船能接）。
///
/// 排序键是**这条航线上的吞吐**（[`freight::trip_throughput`]：舱容 × 每回合能跑几趟）
/// ——不是舰级、不是舱容、也不是"谁离得近"：同一条线上，跑得快的船单位时间搬得多，
/// 而**距离已经折在里面**（柯伊伯带的往返一趟顶金星几十趟）。
///
/// 三处排除，各有理由：
/// * 舱里有货的舰——它得先把自己那票送完（否则货烂在舱里）；
/// * 已经接了别的单的舰——一艘舰一次只能跑一条线（承诺不能重叠）；
/// * 玩家开的舰（`ship_control != Auto`）——AI 不替玩家派活。
fn free_ship_for<'a>(
    state: &'a State,
    config: &GameConfig,
    fid: &str,
    from: &str,
    to: &str,
) -> Option<&'a Ship> {
    let mut cands: Vec<(&Ship, f64)> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .filter(|s| state.ship_control(s.name.clone()) == ControlMode::Auto)
        .filter(|s| state.contracts.assignment_of(&s.name).is_none())
        .filter(|s| s.cargo.is_empty())
        .map(|s| (s, freight::trip_throughput(state, config, s, from, to)))
        .filter(|(_, t)| *t > 0.0)
        .collect();
    cands.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.name.cmp(&b.0.name)));
    cands.first().map(|(s, _)| *s)
}

/// **每回合的撮合**：把挂单簿上的单派给愿意接、也够格接的承运人。
///
/// 逐单处理、**按单号序**（先挂的先被接：一张单不会因为不断有新单出现而永远轮不到）。
/// 每张单三步，全部掷**派生骰子**（`(势力, 单号, 回合, 用途)`，不消费主 `Prng` 流）：
///
/// 1. **看得见吗**：托运方的门槛（[`eligibility`]）——没船可派的势力直接跳过（不掷骰）；
/// 2. **愿意接吗**：自评闸（[`accept_chance`]）；
/// 3. **谁接**：多个愿意接 ⇒ **按信誉加权抽签**（信誉 = 竞争力，但不独占）。
///
/// 中选者在这一回合就**押上一条船**：`carrier` 落定 + `assignments` 记下「哪艘舰执行哪张单」。
/// 路线不由这里写——`step_ships` 的运输舰分支会拿 `freight::route_for` 去写（那张单的路线
/// 优先级最高），角色叶也由 `assign_roles` 照常写（它认识 `assignments`，见
/// `freight::should_be_freighter` 的第 0 条）。**每个叶子只有一个写者**，这里不抢。
pub(crate) fn match_carriers(state: &mut State, config: &GameConfig) {
    let mut fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort(); // 确定性：候选顺序不依赖势力表的排列
    let open: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_open())
        .map(|c| c.id)
        .collect();
    for id in open {
        // 每单重新取一遍状态：前面成交的单会占掉船（`free_ship_for` 看得见）。
        let Some(c) = state.contracts.get(id).cloned() else { continue };
        if !c.is_open() {
            continue;
        }
        let round = state.round;
        let key = format!("契约{id}");
        // 1) + 2)：谁看得见、谁愿意接。
        let mut willing: Vec<(FactionId, f64, ShipId)> = Vec::new();
        for fid in &fids {
            if *fid == c.shipper {
                continue; // 自己给自己运不算承包（挂单的意义就是请人）
            }
            let Some(ship) = free_ship_for(state, config, fid, &c.from, &c.to) else { continue };
            let ship_id = ship.name.clone();
            let rep = state.faction(fid).map(|f| f.reputation).unwrap_or(0.0);
            if sim::derived_roll(fid, &key, round, "gate") >= eligibility(config, rep, &c) {
                continue; // 这一回合没"听说"这单（低信誉者极少看见）
            }
            if sim::derived_roll(fid, &key, round, "accept")
                >= accept_chance(state, config, &c, fid, &ship_id)
            {
                continue; // 看见了但不想接（越不划算越可能不接）
            }
            willing.push((fid.clone(), rep.max(1e-3), ship_id));
        }
        if willing.is_empty() {
            continue;
        }
        // 3) 多家愿意接 ⇒ 按信誉加权抽签：信誉高者更可能拿到，但不是必然。
        let total: f64 = willing.iter().map(|(_, w, _)| *w).sum();
        let mut x = sim::derived_roll("", &key, round, "pick") * total;
        let mut chosen = willing.last().cloned().expect("willing 非空");
        for (fid, w, ship) in &willing {
            if x < *w {
                chosen = (fid.clone(), *w, ship.clone());
                break;
            }
            x -= w;
        }
        let (carrier, _, ship_id) = chosen;
        if let Some(cc) = state.contracts.contracts.iter_mut().find(|c| c.id == id) {
            cc.carrier = Some(carrier.clone());
        }
        state.contracts.assign(ship_id.clone(), id);
        sim::ev(
            state,
            GameEvent::ContractAccepted {
                contract: id,
                shipper: c.shipper.clone(),
                carrier,
                ship: ship_id,
                resource: c.resource.clone(),
                amount: c.outstanding(),
                from: c.from.clone(),
                to: c.to.clone(),
            },
        );
    }
}

// --- 履约：交付 / 超期 / 丢单（M4c）--------------------------------------------
//
// 三件事都是**信誉**的账（Q1(b)：承运人不赔货值，所以掉信誉就是全部的代价）。
// 全部**确定性**：没有任何掷骰，信誉的涨跌是行为的直接函数。

/// 改一个势力的信誉（夹在 `[0, reputation_max]`）。
///
/// **下限是 0 而不是负数**：信誉没有「欠债」的语义——它是准入资产，不是钱包。
/// 归零者仍然可以接单（门槛是 σ 软化的），只是几乎接不到贵单/难单。
fn add_reputation(state: &mut State, config: &GameConfig, fid: &str, delta: f64) {
    if let Some(f) = state.factions.iter_mut().find(|f| f.name == fid) {
        f.reputation = (f.reputation + delta).clamp(0.0, config.freight.reputation_max.max(0.0));
    }
}

/// **交付结算**：承运人的船在目的天体卸下 `loaded` 单位（**卸出舱的总量**，含自留的抽成），
/// 其中 `cut` 是承运人留下的那一份。
///
/// 做三件事：
/// 1. 记进度（`delivered += loaded`——**是卸出舱的总量**，不是托运方实收的量。合同上写的
///    「运 W 单位」是**要搬多少货**，抽成是搬运费，不能从合同的量里扣：否则一份 20 件的单
///    按 15% 抽成永远差 15% 的尾款，`outstanding` 会像芝诺的乌龟一样追不上零）；
/// 2. **加信誉**——增量按**货值**折算（`gain × 货值 ÷ risk_value_ref`）：搬了不值钱的货
///    确实涨不了多少信誉，这与「信誉 = 你可靠地搬过多少值钱的东西」同义，不需要另设阈值；
/// 3. 发 `contract_delivered` 事件（带上承运人自留的抽成：那是它的**全部报酬**）。
pub(crate) fn on_delivery(
    state: &mut State,
    config: &GameConfig,
    ship_id: &str,
    loaded: f64,
    cut: f64,
) {
    let Some(id) = state.contracts.assignment_of(ship_id) else { return };
    let Some(c) = state.contracts.get(id).cloned() else { return };
    let Some(carrier) = c.carrier.clone() else { return };
    let shipper = c.shipper.clone();
    let gain = config.freight.reputation_gain
        * (value_of_amount(config, &c.resource, loaded) / config.freight.risk_value_ref.max(1e-9));
    if let Some(cc) = state.contracts.contracts.iter_mut().find(|c| c.id == id) {
        cc.delivered += loaded;
    }
    add_reputation(state, config, &carrier, gain);
    sim::ev(
        state,
        GameEvent::ContractDelivered {
            contract: id,
            shipper,
            carrier,
            ship: ship_id.to_string(),
            resource: c.resource,
            amount: (loaded - cut).max(0.0),
            cut,
            gain,
        },
    );
}

/// **每回合的履约巡检**：超期扣分（一次）+ 承运人丢船（回挂单簿）。
///
/// 交付不在这里——它发生在 [`crate::sim::haul_unload`] 那一刻（船真的到了泊位）。
/// 这里处理的是**时间的后果**与**船的存亡**，每回合开头跑一次（`sim::step_contracts`）。
pub(crate) fn settle_contracts(state: &mut State, config: &GameConfig) {
    // 1) **超期**：已接单、过了截止期、还没扣过 ⇒ 扣一次（Q11：只扣一次，不作废）。
    let overdue: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.carrier.is_some() && !c.late_penalized && state.round > c.deadline)
        .map(|c| c.id)
        .collect();
    for id in overdue {
        let Some(c) = state.contracts.get(id).cloned() else { continue };
        let Some(carrier) = c.carrier.clone() else { continue };
        let ship = ship_of(state, id).unwrap_or_default();
        add_reputation(state, config, &carrier, -config.freight.reputation_late_penalty);
        if let Some(cc) = state.contracts.contracts.iter_mut().find(|c| c.id == id) {
            cc.late_penalized = true;
        }
        sim::ev(
            state,
            GameEvent::ContractLate {
                contract: id,
                shipper: c.shipper.clone(),
                carrier,
                ship,
                rounds_late: state.round - c.deadline,
                penalty: config.freight.reputation_late_penalty,
            },
        );
    }
    // 2) **丢单**：押上的舰没了（被击沉/报废）⇒ 合同**回到挂单簿**（别人还能接），
    //    承运人掉一次信誉。**没有货值赔偿**（Q1(b)）——掉信誉就是全部的代价。
    //
    //    `!is_fulfilled()` 不是可选的：交付发生在 `step_ships`（回合后半），而挂单簿的收尾
    //    在下一回合的 `step_contracts` —— 一个**已经送完**的单子如果那艘舰在收尾之前沉了，
    //    会被这里当成"丢单"扣一次信誉。那是**罚一个履约完成的承运人**，与 Q1(b) 的意图相反。
    let stranded: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.carrier.is_some() && !c.is_fulfilled())
        .filter(|c| ship_of(state, c.id).is_none())
        .map(|c| c.id)
        .collect();
    for id in stranded {
        let Some(c) = state.contracts.get(id).cloned() else { continue };
        let Some(carrier) = c.carrier.clone() else { continue };
        let ship = state
            .contracts
            .assignments
            .iter()
            .find(|(_, cid)| **cid == id)
            .map(|(s, _)| s.clone())
            .unwrap_or_default();
        add_reputation(state, config, &carrier, -config.freight.reputation_lost_penalty);
        if let Some(cc) = state.contracts.contracts.iter_mut().find(|c| c.id == id) {
            cc.carrier = None; // 回到挂单簿：剩下的量还能被别人接走
            cc.late_penalized = false; // 新承运人有它自己的截止期判断（合同条件不变）
        }
        sim::ev(
            state,
            GameEvent::ContractLost {
                contract: id,
                shipper: c.shipper.clone(),
                carrier,
                ship,
                reason: "ship_gone".to_string(),
                penalty: config.freight.reputation_lost_penalty,
            },
        );
    }
    // 3) **托运方没货了 ⇒ 放人**（不掉信誉）。这条是必需的收尾，不是客气：
    //    接单的舰被钉在那张单上（`assignments`），而它装货的来源是**托运方的货栈**——
    //    那处积压可能已经被托运方自己的船搬空。不解除的话，这艘舰会**永远**停在别人的
    //    空货栈上等一批不会来的货（`HaulStep::Waiting`），船和单子一起烂在那里。
    let starved: Vec<u64> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.carrier.is_some() && !c.is_fulfilled())
        .filter(|c| {
            state
                .depot(&c.shipper, &c.from)
                .map(|m| m.get(&c.resource).copied().unwrap_or(0.0) <= 1e-9)
                .unwrap_or(true)
        })
        .map(|c| c.id)
        .collect();
    for id in starved {
        let Some(c) = state.contracts.get(id).cloned() else { continue };
        let Some(carrier) = c.carrier.clone() else { continue };
        let ship = state
            .contracts
            .assignments
            .iter()
            .find(|(_, cid)| **cid == id)
            .map(|(s, _)| s.clone())
            .unwrap_or_default();
        if let Some(cc) = state.contracts.contracts.iter_mut().find(|c| c.id == id) {
            cc.carrier = None;
        }
        sim::ev(
            state,
            GameEvent::ContractLost {
                contract: id,
                shipper: c.shipper.clone(),
                carrier,
                ship,
                reason: "no_cargo".to_string(),
                penalty: 0.0, // **不罚**：货不在那儿，不是承运人违约
            },
        );
    }
}

/// 这张单**此刻押着的那艘舰**（还活着才算数）。`None` = 没人跑（丢了或还没接）。
fn ship_of(state: &State, contract: u64) -> Option<ShipId> {
    state
        .contracts
        .assignments
        .iter()
        .find(|(s, id)| **id == contract && state.ship(s.as_str()).map(|sh| sh.hull > 0.0).unwrap_or(false))
        .map(|(s, _)| s.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::world::default_state;

    fn fresh(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let state = default_state(&config, seed);
        (config, state)
    }

    /// 手搓一张单（避开挂单侧的估算，专测承运方这一半）。
    fn contract(
        state: &mut State,
        config: &GameConfig,
        resource: &str,
        amount: f64,
        from: &str,
        to: &str,
        mins: f64,
    ) -> u64 {
        state.contracts.post(
            "美国".into(),
            resource.into(),
            amount,
            from.into(),
            to.into(),
            config.freight.share,
            state.round,
            state.round + 6,
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
            assert!(l <= prev + 1e-12, "λ 必须随信誉单调不增：rep={rep:.1} 时 {l:.3} > {prev:.3}");
            prev = l;
        }
        assert!(shadow_price(&config, 0.0) > shadow_price(&config, 4.0) * 5.0, "两端要拉开差距");
        assert!(shadow_price(&config, 0.0) <= config.freight.shadow_lambda0 + 1e-9, "上界是 lambda0");
    }

    /// **低信誉接不到难单**：同一张深空贵单，低信誉者的合格度必须显著更低。
    #[test]
    fn a_low_reputation_carrier_is_rarely_shown_a_hard_contract() {
        let (config, mut state) = fresh(42);
        let id = contract(&mut state, &config, "氦-3", 40.0, "冥王星", "地球", 2.2);
        let c = state.contracts.get(id).unwrap().clone();
        let low = eligibility(&config, 0.8, &c);
        let high = eligibility(&config, 3.0, &c);
        assert!(high > low, "高信誉的合格度必须更高：{high:.3} vs {low:.3}");
        assert!(low < 0.05, "信誉远低于门槛 ⇒ 极少看见（实为 {low:.4}）");
        assert!(high > 0.9, "信誉远高于门槛 ⇒ 基本总看得见（实为 {high:.4}）");
    }

    /// **λ 的两端就是 §C1 那张表**：险单（注定超期）低信誉者不赌、高信誉者敢赌；稳单反之。
    ///
    /// 这是本模块符号约定（`D = 报酬 + λ × Δ信誉`）的守卫——若把它写成 §C1 原文的
    /// `报酬 − λ × Δ信誉`，这个用例会**反过来**失败（险单对低信誉者反而变香）。
    #[test]
    fn the_reputation_ladder_makes_newcomers_cautious_and_veterans_greedy() {
        let (config, mut state) = fresh(42);
        // 一张**注定超期**的单（截止期已经过了 ⇒ 按时概率 0）。
        let doomed = state.contracts.post(
            "美国".into(), "氦-3".into(), 60.0, "冥王星".into(), "地球".into(), 0.15, 0, 0, 0.0,
        );
        // 一张**稳单**（截止期很远）。
        let safe = state.contracts.post(
            "美国".into(), "氦-3".into(), 60.0, "金星".into(), "地球".into(), 0.15, 0, 99, 0.0,
        );
        let (doomed, safe) = (
            state.contracts.get(doomed).unwrap().clone(),
            state.contracts.get(safe).unwrap().clone(),
        );
        // 找一艘中国的舰（谁的不重要——两张单都用同一条船比较）。
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.class == "destroyer")
            .unwrap()
            .name
            .clone();
        let value = |state: &State, c: &Contract, rep: f64| {
            let mut s = state.clone();
            s.faction_mut("中国").unwrap().reputation = rep;
            decision_value(&s, &config, c, "中国", &ship)
        };
        let (d_low, d_high) = (value(&state, &doomed, 0.5), value(&state, &doomed, 3.5));
        assert!(
            d_high > d_low,
            "注定超期的单：高信誉者（λ 小）才敢接。低 {d_low:.2} vs 高 {d_high:.2}"
        );
        let (s_low, s_high) = (value(&state, &safe, 0.5), value(&state, &safe, 3.5));
        assert!(
            s_low > s_high,
            "稳单：低信誉者（λ 大）更想要——他靠履约攒信誉。低 {s_low:.2} vs 高 {s_high:.2}"
        );
        // 而且**险单的差距必须比稳单大**：信誉才是险单的定价者。
        assert!(
            (d_high - d_low) > (s_high - s_low),
            "险单对信誉的敏感度必须高于稳单：（险 {:.2} vs 稳 {:.2}）",
            d_high - d_low,
            s_high - s_low
        );
    }

    /// **端到端（M4c）**：承运人真的把货搬到了托运方首都，**报酬就是它自留的那部分货**，
    /// 托运方收到的是扣掉抽成的量，双方的信誉/池子都按 Q10/Q1(b) 记账。
    ///
    /// 这一条把整条腿走完：挂单（手搓）→ 押船 → 装（**从托运方的货栈装**）→ 飞 → 卸进
    /// **托运方的池子** → 抽成进承运人自己的池子 → 信誉上涨 → 单子送出挂单簿。
    #[test]
    fn a_contract_delivery_pays_the_carrier_in_cargo_and_credits_reputation() {
        let (config, mut state) = fresh(42);
        let share = config.freight.share;
        // --- 布景：中国在金星积压 12 件碳、自己没有船；美国派一艘驱逐舰去运 ---
        state.depots.clear();
        state.contracts.contracts.clear();
        state.ships.retain(|s| s.faction_id != "中国");
        state.depot_add("中国", "金星", "碳", 12.0);
        // 把所有人关系拉正：这一条测的是**运输腿**，不是战争。实测过不这么做会怎样——
        // 美国的驱逐舰在第 2 回合被打沉，合同正确地回到挂单簿（那是 M4c 的丢单路径，
        // 不是这个用例要测的东西），于是"交付"这件事根本没发生。
        for f in state.factions.iter_mut() {
            for v in f.relations.values_mut() {
                *v = 50.0;
            }
            // 也把家底垫厚：否则第 2 回合就会因为**维护费付不出**而把船报废
            // （实测 `ShipDestroyed cause=UpkeepShortfall`）——那同样是本用例之外的事。
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
        // 把承运的舰**直接摆到金星的泊位上**（否则要先飞几个回合，用例说不清是谁的功劳）。
        let at_venus = state.body_position("金星");
        {
            let s = state.ship_mut(&carrier_ship).unwrap();
            s.position = at_venus;
            s.velocity = 0.0;
        }
        let id = state.contracts.post(
            "中国".into(), "碳".into(), 12.0, "金星".into(), "地球".into(), share, 0, 99, 0.0,
        );
        state.contracts.contracts.iter_mut().find(|c| c.id == id).unwrap().carrier =
            Some("美国".into());
        state.contracts.assign(carrier_ship.clone(), id);
        let rep_us0 = state.faction("美国").unwrap().reputation;
        let mut rng = crate::prng::Prng::new(42);
        let mut delivered_events = 0usize;
        // 事件流只保留**本回合**（`advance` 开头清空），所以跨回合的量要在这里累加。
        let (mut paid, mut cut) = (0.0, 0.0);
        // 8 回合：3 趟（驱逐舰舱容 4 × 12 件）+ 往返在途；**收尾在第 7 回合**——交付发生在
        // `step_ships`（一回合的后半段），而挂单簿的收尾在下一回合的 `step_contracts` 里。
        for _ in 0..8 {
            sim::advance(&mut state, &config, &mut rng);
            for e in &state.events {
                if let GameEvent::ContractDelivered { contract, amount, cut: c, .. } = e {
                    if *contract == id {
                        delivered_events += 1;
                        paid += amount;
                        cut += c;
                    }
                }
            }
        }
        assert!(delivered_events > 0, "八回合内该跑完一趟（金星→地球很近）");
        // 账要**按事件流**核（池子同时在被维护费/建造花掉，直接比池子的差值是脆的——
        // 这一课是 M2b 学到的）：交付总量必须**精确**等于 12 件减去承运人抽成。
        assert!(
            (paid - 12.0 * (1.0 - share)).abs() < 1e-6,
            "托运方该实收 {} 件（12 × (1 − 抽成)），实为 {paid:.3}",
            12.0 * (1.0 - share)
        );
        assert!(
            (cut - 12.0 * share).abs() < 1e-6,
            "承运人该自留 {} 件（12 × 抽成），实为 {cut:.3}",
            12.0 * share
        );
        // 货真的从**托运方的货栈**搬走了：金星那批 12 件不该还在（每回合还在产新的碳）。
        let left: f64 = state
            .depots
            .get(&("中国".to_string(), "金星".to_string()))
            .map(|m| m.values().sum())
            .unwrap_or(0.0);
        assert!(left < 12.0, "承运人该把货装走：金星货栈还剩 {left:.2} 件");
        assert!(
            state.faction("美国").unwrap().reputation > rep_us0,
            "按时交付该涨信誉（{rep_us0:.3} → {:.3}）",
            state.faction("美国").unwrap().reputation
        );
        // 单子送完 ⇒ 送出挂单簿，执行关系也一并收干净（不留幽灵引用）。
        assert!(state.contracts.get(id).is_none(), "送完的单子该移出挂单簿");
        assert_eq!(state.contracts.assignment_of(&carrier_ship), None, "执行关系要收干净");
    }

    /// **Q11：超期只扣一次信誉，合同不作废**——扣两次或直接作废都是错的。
    #[test]
    fn a_late_contract_costs_reputation_once_and_survives() {
        let (config, mut state) = fresh(42);
        let id = state.contracts.post(
            "中国".into(), "碳".into(), 5.0, "金星".into(), "地球".into(), 0.15, 0, 0, 0.6,
        );
        state.contracts.contracts.iter_mut().find(|c| c.id == id).unwrap().carrier =
            Some("美国".into());
        // 押一艘**真实存在**的舰：否则「船没了」那条路径会同时触发（那是另一个用例的事）。
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "美国")
            .expect("美国开局有舰")
            .name
            .clone();
        state.contracts.assign(ship, id);
        let rep0 = state.faction("美国").unwrap().reputation;
        state.round = 1; // 越过 deadline = 0
        settle_contracts(&mut state, &config);
        let rep1 = state.faction("美国").unwrap().reputation;
        assert!(
            (rep0 - rep1 - config.freight.reputation_late_penalty).abs() < 1e-9,
            "超期该扣一次：{rep0:.3} → {rep1:.3}"
        );
        assert!(state.contracts.get(id).is_some(), "超期**不作废**（Q11）");
        // 再跑几个回合：**不能再扣**（`late_penalized` 钉住这条）。
        for _ in 2..6 {
            state.round += 1;
            settle_contracts(&mut state, &config);
        }
        assert!(
            (state.faction("美国").unwrap().reputation - rep1).abs() < 1e-9,
            "超期只扣一次（实为 {:.3}）",
            state.faction("美国").unwrap().reputation
        );
    }

    /// **承运人把船丢了**：合同**回到挂单簿**（别人还能接），承运人掉一次信誉，**没有赔偿**。
    #[test]
    fn losing_the_ship_puts_the_contract_back_on_the_book_with_a_reputation_hit() {
        let (config, mut state) = fresh(42);
        let id = state.contracts.post(
            "中国".into(), "碳".into(), 5.0, "金星".into(), "地球".into(), 0.15, 0, 99, 0.6,
        );
        state.contracts.contracts.iter_mut().find(|c| c.id == id).unwrap().carrier =
            Some("美国".into());
        // 押的舰**不存在**（等价于被击沉后从状态里消失）。
        state.contracts.assign("已经沉了的舰".into(), id);
        let rep0 = state.faction("美国").unwrap().reputation;        settle_contracts(&mut state, &config);
        let c = state.contracts.get(id).expect("合同要留在挂单簿上（回到没人接的状态）");
        assert!(c.is_open(), "丢单 ⇒ 回到挂单簿，别人还能接");
        assert!(
            (rep0 - state.faction("美国").unwrap().reputation - config.freight.reputation_lost_penalty)
                .abs() < 1e-9,
            "丢单该扣一次信誉"
        );
        // 已经送到的那部分**不退回**（Q1(b)：不赔货值），所以剩余量照旧只差剩下的。
        assert_eq!(c.delivered, 0.0);
    }

    /// **端到端**：挂出去的单会被接走，接单者押上一艘**属于它自己**的船，且那艘船跑的是
    /// **托运方**的路线（不是承运人自己的首都）。
    #[test]
    fn an_open_contract_gets_a_carrier_and_a_real_ship() {
        let (config, mut state) = fresh(42);
        // 清场：只留中国有积压（挂单方），其余势力保持开局舰队（承运方候选）。
        state.depots.clear();
        state.contracts.contracts.clear();
        state.factions.iter_mut().for_each(|f| f.reputation = 1.0);
        state.depot_add("中国", "金星", "碳", 400.0);
        state.ships.retain(|s| s.faction_id != "中国"); // 中国没有船 ⇒ 只能请人
        let mut rng = crate::prng::Prng::new(42);
        sim::advance(&mut state, &config, &mut rng);
        let taken: Vec<&Contract> = state
            .contracts
            .contracts
            .iter()
            .filter(|c| c.carrier.is_some())
            .collect();
        assert!(!taken.is_empty(), "400 件积压挂出去，该有人接：{:?}", state.contracts.contracts);
        for c in taken {
            let carrier = c.carrier.clone().unwrap();
            assert_ne!(carrier, c.shipper, "不能自己接自己的单");
            let ships: Vec<&ShipId> = state
                .contracts
                .assignments
                .iter()
                .filter(|(_, id)| **id == c.id)
                .map(|(s, _)| s)
                .collect();
            assert_eq!(ships.len(), 1, "一张单押一艘船");
            let ship = state.ship(ships[0]).expect("押的船必须存在");
            assert_eq!(ship.faction_id, carrier, "押的必须是**承运人自己**的船");
            // 路线由 `route_for` 给出：必须是**托运方**的货栈 → 托运方首都。
            let route = freight::route_for(&state, &carrier, &ships[0]).expect("接单的舰要有路线");
            assert_eq!(route, (c.from.clone(), c.to.clone()), "跑的是托运方的路线");
            assert_eq!(route.1, state.capital_body(&c.shipper), "目的 = 托运方首都");
        }
    }
}
