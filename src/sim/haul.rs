//! 集货运输：配额 → 抽签派单 → 装卸分录（货权守恒）。

use super::*;

/// 一批货的**尽量等量**分配（用户裁决 Q6）：把 `capacity` 个单位按「还有货的种类数」平摊；
/// 某种货不够平摊，就把它的余量**交回去**、由其余种类再平摊（max-min 公平分配）。
///
/// ```text
/// 三种货、舱容 6，都够      ⇒ 每种 2
/// {铁 10, 铂 1, 碳 10}、舱容 6 ⇒ {铁 2.5, 铂 1, 碳 2.5}
/// 舱容 ≥ 总存量             ⇒ 全装走
/// ```
///
/// 为什么不是「按价值降序」或「按存量比例」：**尽量等量**不会让某种便宜货永远排在队尾
/// （矿是混装的散货，不是按单价挑的快递），也不会在货栈里留下一地分数残渣。
/// 纯函数、确定性（[`ResourceMap`] 是 `BTreeMap` ⇒ 名字序遍历，与插入顺序无关）。
pub fn haul_split(available: &ResourceMap, capacity: f64) -> ResourceMap {
    let mut out = ResourceMap::new();
    let mut cap = capacity;
    if cap <= 1e-9 {
        return out;
    }
    let mut pool: Vec<(&String, f64)> = available
        .iter()
        .filter(|(_, a)| **a > 1e-9)
        .map(|(k, a)| (k, *a))
        .collect();
    // 每一轮：把剩余舱容平摊给「还有货没分完」的种类；分光的种类退出、余量进下一轮。
    // 终止性：每轮要么分完舱容（cap → 0），要么至少有一种货被分光（pool 变小）。
    while !pool.is_empty() && cap > 1e-9 {
        let share = cap / pool.len() as f64;
        let mut next: Vec<(&String, f64)> = Vec::new();
        for (rt, left) in pool {
            let take = share.min(left);
            *out.entry(rt.clone()).or_insert(0.0) += take;
            cap -= take;
            if left - take > 1e-9 {
                next.push((rt, left - take));
            }
        }
        pool = next;
    }
    out.retain(|_, v| *v > 1e-9);
    out
}

/// 运输动作（变体与判据）**住在 `model`**：它同时是引擎内部类型与读面类型
/// （`view.haul_steps`），而 `model` 不许依赖 `sim`。这里只做转出，让
/// `sim::HaulStep` / `sim::haul::HaulStep` 照旧可用（全仓库的既有引用不动）。
pub use crate::model::HaulStep;

/// 这批货的**货主**（收货方）：执行承包单时是**托运方**，否则是船主自己。
///
/// 「货主」是本作里必须显式存在的概念（`.agents/notes/freight-collection.md` 的已定项）：
/// 承包让**船东与业主分离**，于是「装谁的货、卸进谁的池子、卸出来的货算谁的进度」
/// 三件事都不能再默认等于船主。判据是 [`ContractState::assignments`]（哪艘舰跑哪张单），
/// 不去猜「路线像不像」——同一处货栈可以有两张不同托运方的单。
pub fn cargo_owner(state: &State, fid: &str, ship_id: &str) -> FactionId {
    state
        .contracts
        .assignment_of(ship_id)
        .and_then(|id| state.contracts.get(id))
        .map(|c| c.shipper.clone())
        .unwrap_or_else(|| fid.to_string())
}

/// **装货**：把 `from` 处**货主**手上的货装进 `ship` 的货舱。
///
/// 上限 = 有效舱容（[`cargo_capacity`]：舰级舱容 × 战损折算）− 已在舱；分配按 [`haul_split`]。
///
/// **货从哪来取决于 `from` 是哪**（[`State::stock_at`]，用户裁决「完全禁止瞬移」）：
/// * `from` = **货主的首都** ⇒ 装**首都池**——这是**补给腿**（首都 → 殖民地）的起点，
///   从前这里只认货栈，于是「首都发不出货」，非首都站点永远等不来材料；
/// * 其余天体 ⇒ 装该处**产地货栈**（集货腿，产出在原地等船）。
///
/// **补给腿只装对面真的缺的**（`to` 处的建设缺口，`freight::site_deficit`）：首都池里
/// 什么都有，照单全装会把用不上的货堆到殖民地门口——而那些货下一回合就会被集货签
/// 判定为「净剩余」**原路运回来**（两条腿自己和自己打架）。所以进口只装缺口里的货，
/// 与出口的「净剩余」口径互为镜像：同一件货不可能同时出现在两条腿上。
///
/// **受雇跑的线没有任何额外上限**：雇主挂的是**运力**（单位/回合），不是「要搬多少件」，
/// 所以受雇的船到了地方能装多少装多少——与雇主自己的运输舰完全一样（旧形态里这里还有一道
/// 「这张单还差多少」的闸，那是「一票货」形态的遗留）。返回**实际装走的总件数**
/// （0 = 那里没货，舰该原地等）。
pub fn haul_load(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    from: &str,
    to: &str,
) -> f64 {
    let Some(ship) = state.ship(ship_id) else {
        return 0.0;
    };
    let room = cargo_capacity(config, ship) - cargo_used(&ship.cargo);
    if room <= 1e-9 {
        return 0.0;
    }
    let owner = cargo_owner(state, fid, ship_id);
    let mut avail = state.stock_at(&owner, from).cloned().unwrap_or_default();
    // **出口腿不能装走本地保留量**（`site_reserve`）：起点不是货主首都时，
    // 只有 `exportable_at = max(0, 现货 − 保留量)` 的部分才允许离站。
    // 没有这一道，站点正在留着建楼/造舰的料会被船一起装回首都，下一回合
    // `site_deficit` 又把同一批货列为进口需求 ⇒ 往返乒乓。
    if from != state.capital_body(&owner) {
        let exportable = autocontrol::freight::exportable_at(state, config, &owner, from);
        avail.retain(|rt, amt| {
            *amt = amt.min(exportable.get(rt).copied().unwrap_or(0.0));
            *amt > 1e-9
        });
    }
    // 补给腿（起点是货主的首都）：只装终点缺的货。
    if from == state.capital_body(&owner) && to != from {
        let want = autocontrol::freight::site_deficit(state, config, &owner, to);
        avail.retain(|rt, amt| {
            *amt = amt.min(want.get(rt).copied().unwrap_or(0.0));
            *amt > 1e-9
        });
    }
    let plan = haul_split(&avail, room);
    let mut moved: ResourceMap = ResourceMap::new();
    for (rt, want) in &plan {
        let got = state.stock_take(&owner, from, rt, *want);
        if got > 0.0 {
            moved.insert(rt.clone(), got);
        }
    }
    let units: f64 = moved.values().sum();
    if units <= 0.0 {
        return 0.0;
    }
    if let Some(s) = state.ship_mut(ship_id) {
        for (rt, amt) in &moved {
            *s.cargo.entry(rt.clone()).or_insert(0.0) += amt;
        }
    }
    ev(
        state,
        GameEvent::CargoLoaded {
            ship: ship_id.to_string(),
            faction: fid.to_string(),
            owner,
            body: from.to_string(),
            cargo: moved,
        },
    );
    units
}

/// **卸货**：把 `ship` 的整个货舱卸进 `to`。`to` 是**货主**的首都 ⇒ 直接进货主的**势力池**
/// （[`Faction::resources`]：集货腿的终点，货从此可用）；否则进该天体的货栈（中转，还得再运一程）。
///
/// 执行承包单时多做一件事：**按抽成留下承运人的那一份**（Q10）——留下来的货直接进
/// 承运人自己的首都池（用户批准的简化：报酬**就是它没交出去的那部分货**，没有货币转移）。
/// 真正的「回程把自己那份拉回家」需要把 `Haul` 的无状态腿规则撑开（舱里不是空的就是满的
/// 那条判据不够用了），留作后续钩子。
/// 返回卸下的货（空 = 本来就空舱）。
pub fn haul_unload(
    state: &mut State,
    _config: &GameConfig,
    fid: &str,
    ship_id: &str,
    to: &str,
) -> ResourceMap {
    let cargo = state
        .ship_mut(ship_id)
        .map(|s| std::mem::take(&mut s.cargo))
        .unwrap_or_default();
    if cargo.is_empty() {
        return cargo;
    }
    let owner = cargo_owner(state, fid, ship_id);
    // 承包单：先按抽成切出承运人的那一份（Q10）。
    let contract = state
        .contracts
        .assignment_of(ship_id)
        .and_then(|id| state.contracts.get(id).cloned());
    let loaded: f64 = cargo.values().sum(); // 卸出舱的**总量**（合同进度按它记）
    let mut delivered = cargo.clone();
    let mut cut_units = 0.0;
    if let Some(c) = &contract {
        for (rt, amt) in delivered.iter_mut() {
            let cut = c.carrier_cut(*amt);
            *amt -= cut;
            cut_units += cut;
            if cut > 0.0 {
                // 承运人的报酬进它自己的首都池（Q10 的机制落点：没有货币转移）。
                if let Some(f) = state.factions.iter_mut().find(|f| f.name == fid) {
                    *f.resources.entry(rt.clone()).or_insert(0.0) += cut;
                }
            }
        }
        delivered.retain(|_, amt| *amt > 1e-9);
    }
    let into_pool = state.capital_body(&owner) == to;
    if into_pool {
        if let Some(f) = state.factions.iter_mut().find(|f| f.name == owner) {
            for (rt, amt) in &delivered {
                *f.resources.entry(rt.clone()).or_insert(0.0) += amt;
            }
        }
    } else {
        for (rt, amt) in &delivered {
            state.depot_add(&owner, to, rt, *amt);
        }
    }
    ev(
        state,
        GameEvent::CargoDelivered {
            ship: ship_id.to_string(),
            faction: fid.to_string(),
            owner: owner.clone(),
            body: to.to_string(),
            cargo: delivered.clone(),
            into_pool,
        },
    );
    // 雇佣的账在货物落地之后结：进度 + `contract_delivered` 事件（**信誉不在这里动**——
    // 雇佣形态下信誉只由雇主的周期考核产生）。
    if contract.is_some() && loaded > 1e-9 {
        autocontrol::contract::on_delivery(state, ship_id, loaded, cut_units);
    }
    cargo
}

/// 一条运输路线本回合到达泊位后的**动作**（装或卸），返回这一步的记录。
///
/// `leg` 是本回合要办的那一端、`other` 是另一端——**装货要知道对面是谁**：补给腿
/// （`leg` 是货主的首都）只装对面缺的货（见 [`haul_load`]）。卸货用不到 `other`。
pub fn haul_act(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    leg: &str,
    other: &str,
    holding: bool,
) -> HaulStep {
    if holding {
        let cargo = haul_unload(state, config, fid, ship_id, leg);
        HaulStep::Delivered {
            body: leg.to_string(),
            units: cargo.values().sum(),
            // 「进池」说的是**货主**的池子（承包时货主是托运方，不是船东）。
            into_pool: state.capital_body(&cargo_owner(state, fid, ship_id)) == leg,
        }
    } else {
        let units = haul_load(state, config, fid, ship_id, leg, other);
        if units > 0.0 {
            HaulStep::Loaded {
                body: leg.to_string(),
                units,
            }
        } else {
            HaulStep::Waiting {
                body: leg.to_string(),
            }
        }
    }
}

/// 一条运输路线（[`ShipBehavior::Haul`]）本回合的**完整执行**：腿别判定 + 移动 + 装卸都在这里，
/// 所以调用方**不要再自己移动这艘舰**。
///
/// **腿别不存状态**：舱里有货 ⇒ 去 `to`；空舱 ⇒ 去 `from`（见 [`ShipBehavior::Haul`] 的说明）。
/// 到达判定用 `arrival_eps`（与殖民/停泊同一把尺子），且目标点照样会被 MOND 偏移——
/// 所以深处取货/送货不是「做不到」，而是**要多试几个回合**（`move_toward` 每回合重新算一次）。
pub fn haul_step(
    state: &mut State,
    config: &GameConfig,
    ship_id: &str,
    class: &str,
    from: &str,
    to: &str,
    inputs: &mut crate::model::RoundInputs,
) -> HaulStep {
    let Some(ship) = state.ship(ship_id) else {
        return HaulStep::EnRoute {
            body: from.to_string(),
        };
    };
    let fid = ship.faction_id.clone();
    let pos = ship.position;
    let holding = !ship.cargo.is_empty();
    let leg = if holding { to } else { from };
    let other = if holding { from } else { to };
    let target = state.body_position(leg);
    let eps = config.combat.arrival_eps;
    // 已经停在泊位内：本回合直接办事（与 Colonize 一样是「到达即行动」）。
    if dist(pos, target) <= eps {
        return haul_act(state, config, &fid, ship_id, leg, other, holding);
    }
    // **kiting 姿态照常生效**（用户裁决：角色不影响 kiting）：附近有敌舰时，航路上的软目标
    // 会被拉开/压近。它只改**移动**、不改「到没到」——到达判定看真实位置，且上面那一步
    // 「已在泊位内就直接办事」先于移动，所以**靠了泊位的运输舰不会被敌人推得卸不了货**。
    let dest = autocontrol::kiting_dest(state, config, ship_id).unwrap_or(target);
    move_toward(state, config, ship_id, class, dest, Some(inputs));
    let np = state.ship(ship_id).map(|s| s.position).unwrap_or(pos);
    if dist(np, target) <= eps {
        return haul_act(state, config, &fid, ship_id, leg, other, holding);
    }
    HaulStep::EnRoute {
        body: leg.to_string(),
    }
}
