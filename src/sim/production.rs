//! 生产与维护：矿产出、货栈、劳动比、维护费、建造预算扣减。

use super::*;

/// The command-controlled 建设投资权重 of a building (its build priority).
/// Follows the control scope: an AI-controlled building uses the config default,
/// while a player-controlled building uses the commanded value.
pub fn invest_weight(
    state: &State,
    config: &GameConfig,
    fid: &str,
    cid: &str,
    b: &Building,
) -> f64 {
    let key = (cid.to_string(), b.id);
    if state.invest_control(fid.to_string(), &key).is_player() {
        state
            .control(fid.to_string())
            .and_then(|c| c.invest_weights.get(&key))
            .map(|c| c.value)
            .unwrap_or_else(|| config.building_spec(&b.kind).default_invest_weight)
    } else {
        config.building_spec(&b.kind).default_invest_weight
    }
}

/// The command-controlled 建造投资权重 of a 建造区 (shipyard) building.
pub fn build_weight(state: &State, config: &GameConfig, fid: &str, cid: &str, b: &Building) -> f64 {
    let key = (cid.to_string(), b.id);
    if state.build_control(fid.to_string(), &key).is_player() {
        state
            .control(fid.to_string())
            .and_then(|c| c.build_weights.get(&key))
            .map(|c| c.value)
            .unwrap_or_else(|| config.building_spec(&b.kind).default_build_weight)
    } else {
        config.building_spec(&b.kind).default_build_weight
    }
}

/// 某城的**福利权重**：势力级福利预算按这些权重分给城市。
///
/// `Player` 的 `loyalty_budget` 叶 = 玩家钉的权重；缺叶/`Auto` = 系统默认 1.0
/// （即所有城均分）。旧函数名 `city_loyalty_budget` 已改语义：它不再是「每城市场价值预算」，
/// 而是 `spec.md` 里「城市 福利权重」的落点。总预算在势力级 `welfare_budget`。
pub fn city_welfare_weight(state: &State, fid: FactionId, cid: CityId) -> f64 {
    if state
        .loyalty_budget_control(fid.clone(), cid.clone())
        .is_player()
    {
        state
            .control(fid.clone())
            .and_then(|c| c.loyalty_budget.get(&cid))
            .map(|c| c.value.max(0.0))
            .unwrap_or(1.0)
    } else {
        1.0
    }
}

// --- production -------------------------------------------------------------

pub fn deposit_area(deposits: &[(String, f64)], rt: &str) -> f64 {
    deposits
        .iter()
        .find(|(r, _)| r == rt)
        .map(|(_, a)| *a)
        .unwrap_or(0.0)
}

/// A city's labour ratio: population vs. total staff required by its buildings.
///
/// `pub(crate)`：与 [`max_affordable_inc`] 同理——自动控制估建造时间时必须用**同一把**尺子。
pub fn labor_ratio(state: &State, config: &GameConfig, cid: &str) -> f64 {
    let population = state.city(cid).map(|c| c.population as f64).unwrap_or(0.0);
    let mut staff_req = 0.0;
    if let Some(c) = state.city(cid) {
        for b in &c.buildings {
            staff_req += b.deployed * config.building_spec(&b.kind).staff_per_area;
        }
    }
    if staff_req <= 0.0 {
        1.0
    } else {
        (population / staff_req).clamp(config.economy.min_efficiency, 1.0)
    }
}

/// **首都天体上不留货栈**（迁都留下的旧中转点会被并回池子）——[`State::stock_at`] 的
/// 「首都 ⇒ 池子、其余 ⇒ 货栈」这条二分只有在**首都天体没有货栈**时才自洽。
///
/// 迁都把一处非首都天体变成首都：那处原本攒着「等船运走」的货栈，此刻既不属于「本地产出」
/// （首都产出直接进池），也不再是 [`State::stock_at`] 会去读的地方——不并回池子，
/// 那些货就**谁也拿不到**（既装不上船，也花不出去）。所以每回合开头归一一次。
///
/// 反过来（首都变成普通天体）不需要任何动作：它从此按普通站点收自己的产出，池子照旧是池子。
fn merge_capital_depots(state: &mut State) {
    let fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    for fid in fids {
        let cap = state.capital_body(&fid);
        let Some(map) = state.depots.remove(&(fid.clone(), cap)) else {
            continue;
        };
        if let Some(f) = state.faction_mut(&fid) {
            for (rt, amt) in map {
                *f.resources.entry(rt).or_insert(0.0) += amt;
            }
        }
    }
}

pub fn step_production(state: &mut State, config: &GameConfig, flow: &mut RoundSink) {
    merge_capital_depots(state);
    let city_ids: Vec<CityId> = state.cities.iter().map(|c| c.name.clone()).collect();
    for cid in city_ids {
        let (body_id, faction_id, population, razed) = {
            let c = state.city(&cid).expect("city disappeared");
            (
                c.body_id.clone(),
                c.faction_id.clone(),
                c.population,
                c.razed,
            )
        };
        if razed {
            continue;
        }
        // **首都即集散地**（`.agents/notes/freight-collection.md`）：首都天体的产出免运输、
        // 直接进势力池；其余天体的产出**先落在产地货栈**，要等船来运回首都才可用。
        // 于是「非首都产出必须靠运输」不是一句设定，而是产出落库路径本身。
        let is_hub = state.capital_body(&faction_id) == body_id;
        let (ecocap, deposits) = {
            let s = state.city_settlement(&cid);
            match s {
                Some(s) => (
                    s.ecological_capacity,
                    s.resources
                        .iter()
                        .map(|d| (d.resource.clone(), d.area))
                        .collect::<Vec<_>>(),
                ),
                None => continue,
            }
        };

        let mut housing_area = 0.0;
        let mut staff_req = 0.0;
        let mut mines: Vec<(String, f64)> = Vec::new();
        for b in &state.city(&cid).expect("city disappeared").buildings {
            let spec = config.building_spec(&b.kind);
            staff_req += b.deployed * spec.staff_per_area;
            let health = building_health(b, config);
            match spec.role.as_str() {
                "housing" => housing_area += b.deployed * health,
                "mining" => {
                    if let Some(r) = &b.resource {
                        mines.push((r.clone(), b.deployed * health));
                    }
                }
                _ => {}
            }
        }

        let housing_capacity = housing_area * ecocap;

        // Population grows toward housing capacity.
        if housing_capacity > population as f64 {
            let delta =
                ((housing_capacity - population as f64) * config.economy.pop_growth).round() as i64;
            if delta > 0 {
                if let Some(c) = state.city_mut(&cid) {
                    c.population = ((c.population as i64 + delta)
                        .min(housing_capacity as i64)
                        .max(0)) as u32;
                }
            }
        }

        let labor = if staff_req <= 0.0 {
            1.0
        } else {
            (population as f64 / staff_req).clamp(config.economy.min_efficiency, 1.0)
        };

        // 记录本回合的**用工系数 / 住房容量 / 是否集散地**（B2 的中间量）：这三个数此前算完就扔，
        // 而它们正是「这座城产量为什么低 / 人口为什么不涨 / 挖出来的矿为什么用不了」的答案。
        // ⚠ 记的是**这一步用的**值：`labor` 取的是人口增长**之前**的人口（上面那段才涨），
        // 事后拿回合末的 state 重算会得到另一个数——建造那一步另算的那把，已经折进
        // `step_construction` 写的 `build.rate` 里，不在这里存第二份。
        {
            let cf = flow.city_flow.entry(cid.clone()).or_default();
            cf.labor = labor;
            cf.housing_capacity = housing_capacity;
            cf.is_hub = is_hub;
        }

        // Mining output.
        for (rt, area) in mines {
            let effective = area.min(deposit_area(&deposits, &rt));
            if effective <= 0.0 {
                continue;
            }
            let spec = config.building_spec("mining");
            let output = effective * labor * spec.productivity * config.economy.production_rate;
            // 记录本回合产出（step_production 的「中间量」），供 observe 做 agent 总结：
            // 每城 + 每势力各记一份；**记的是开采量**（不管它落在首都还是产地货栈）；
            // 随后按 `is_hub` 决定入库路径。
            *flow
                .city_production
                .entry(cid.clone())
                .or_default()
                .entry(rt.clone())
                .or_insert(0.0) += output;
            *flow
                .faction_production
                .entry(faction_id.clone())
                .or_default()
                .entry(rt.clone())
                .or_insert(0.0) += output;
            if is_hub {
                if let Some(f) = state.faction_mut(&faction_id) {
                    *f.resources.entry(rt).or_insert(0.0) += output;
                }
            } else {
                state.depot_add(&faction_id, &body_id, &rt, output);
            }
        }
    }
}

// --- interstellar market (resource sink + keystone supply) -------------------

/// Fleet upkeep: every ship costs its class's [`ShipSpec::upkeep`] (market value)
/// per round to keep in service. The faction's total fleet maintenance is paid
/// out of its stockpile (drained value-weighted across all minerals); if the
/// faction cannot cover it, the shortfall rusts its fleet (ships lose hull
/// proportionally, and ships driven to 0 are scrapped).
///
/// This is the continuous resource **sink** that bounds fleet size: big fleets
/// need a big economy to sustain, so the navy grows only as fast as the
/// economy feeds it rather than snowballing unboundedly.
///
/// **例外：无活城的势力（流亡舰队）不因维护费被拆解。** 维护费是**港口/后勤**的成本——
/// 没有港口就无从「欠费拆解」，舰队只能靠打捞、掠夺、拆东墙补西墙自持。这条例外是
/// `step_resurgence` 被删除（D5）之后**唯一的立足点保证**：复垦必须由**航行**完成
/// （派船去空白定居点，见 `colonize`），所以流亡舰队必须先**活到**能开过去。
/// 没有这条，实测 seed 1/7/42 跑到 1000 回合会**只剩 1-4 个势力有城、5-8 个永久亡国**
/// ——战争拆掉最后一座城 → 无产出 → 库存被维护费抽干 → 全舰队生锈拆解 → 永远回不来。
pub fn step_upkeep(state: &mut State, config: &GameConfig, flow: &mut RoundSink) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    for fid in faction_ids {
        let upkeep_total: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
            .map(|s| ship_panel(config, s).upkeep)
            .sum();
        // 记录本回合舰队维护费（step_upkeep 的「中间量」）。欠费与生锈在下面补进同一格
        // （「付不起会怎样」和「该付多少」是同一件事的两面，所以只占一个位置）。
        flow.upkeep.entry(fid.clone()).or_default().total = upkeep_total;
        if upkeep_total <= 1e-9 {
            continue;
        }
        // 流亡舰队（无活城）：不抽库存、不生锈——见上面的「例外」。
        let landless = !state.cities.iter().any(|c| c.faction_id == fid && !c.razed);
        if landless {
            continue;
        }
        let stock = state
            .faction(&fid)
            .map(|f| f.resources.clone())
            .unwrap_or_default();
        let total_value: f64 = stock.iter().map(|(k, v)| v * value_of(k)).sum();
        let pay = upkeep_total.min(total_value);
        if pay > 1e-9 {
            let ratio = (pay / total_value).min(1.0);
            if let Some(f) = state.faction_mut(&fid) {
                for (k, v) in stock.iter() {
                    let new = (*v - *v * ratio).max(0.0);
                    f.resources.insert(k.clone(), new);
                }
            }
        }
        // Unpaid upkeep rusts the fleet; hull reaching 0 scrapped.
        let short = (upkeep_total - total_value).max(0.0);
        // 记「欠了多少」：付不起的那部分（0 = 付清）。它就是生锈的分子。
        flow.upkeep.entry(fid.clone()).or_default().unpaid = short;
        if short > 1e-9 {
            let frac = (short / upkeep_total).min(1.0);
            let frac = frac.max(0.2); // at least a visible rust when short
            // 记**实际用的**那个比例（含 0.2 下限）：每艘舰掉的船体 = `hull_max × 这个数`。
            // 只有锈到 0 才留事件，所以掉血本身只有这一个读法。
            flow.upkeep.entry(fid.clone()).or_default().rust = frac;
            let mut scrap: Vec<ShipId> = Vec::new();
            for s in state.ships.iter_mut() {
                if s.faction_id != fid || s.hull <= 0.0 {
                    continue;
                }
                let rust = ship_panel(config, s).hull_max * frac;
                s.hull = (s.hull - rust).max(0.0);
                if s.hull <= 0.0 {
                    scrap.push(s.name.clone());
                }
            }
            for sid in scrap {
                // 经济性死亡：不是被打沉的，是养不起被拆解的（此前与战损共用同一个事件、
                // 无法区分）。走漏斗 → 保证有事件。
                kill_ship(state, &sid, DeathCause::UpkeepShortfall, None);
            }
        }
    }
}

pub fn per_area_cost(
    config: &GameConfig,
    spec: &BuildingSpec,
    res_mod: f64,
    b: &Building,
) -> Vec<(String, f64)> {
    let mult = config.structure_spec(&b.structure).cost_mult * res_mod;
    spec.build_cost
        .iter()
        .map(|(rt, c)| (rt.clone(), c * mult))
        .collect()
}

pub fn budget_remaining(limit: &ResourceMap, spent: &ResourceMap, rt: &str) -> f64 {
    limit.get(rt).copied().unwrap_or(0.0) - spent.get(rt).copied().unwrap_or(0.0)
}

/// 在**预算**（`limit`，一件/回合）与**速率上限**（`cap`）下，这个回合最多能推多少进度。
///
/// `pub(crate)`：自动控制估计**造舰时间**用的就是这一把尺子（[`crate::autocontrol::shipbuilding::build_rounds`]）
/// ——各写一份必然漂移，于是「AI 以为要 5 回合、实际要 50 回合」这种错就会悄悄发生。
///
/// **同一个函数也用来卡「这个站点手上到底有没有这些货」**（把 `limit` 传成
/// [`State::stock_at`] 的副本、`spent` 传空表）：建造的可负担量 = `min(预算速率, 站点库存)`，
/// 两个上限共用一条算术，不再各写一份。
pub fn max_affordable_inc(
    cost_per_area: &[(String, f64)],
    limit: &ResourceMap,
    spent: &ResourceMap,
    cap: f64,
) -> f64 {
    let mut inc = cap;
    for (rt, c) in cost_per_area {
        if *c <= 1e-9 {
            continue;
        }
        let have = budget_remaining(limit, spent, rt);
        inc = inc.min(have / *c);
    }
    inc.max(0.0)
}

/// **这个站点此刻买得起多少**：把一笔单位成本与**该天体的库存**一起过一遍
/// [`max_affordable_inc`]（`limit` = [`State::stock_at`]、`spent` = 空、速率上限 = `cap`）。
///
/// 与 [`commit_spend`] 配对使用：先卡到「站点付得起」，再从站点扣。两处分开写必然漂移，
/// 而漂移的后果正是 `.agents/notes/freight-collection.md` 要堵的那个洞——**凭空造出**。
pub fn site_affordable(
    state: &State,
    fid: &str,
    body: &str,
    cost: &[(String, f64)],
    cap: f64,
) -> f64 {
    let stock = state.stock_at(fid, body).cloned().unwrap_or_default();
    max_affordable_inc(cost, &stock, &ResourceMap::new(), cap)
}

/// 在**某座城所在天体**上花掉一笔实物（建楼 / 造舰 / 装模块都走这里）。
///
/// * **首都天体** ⇒ 扣势力池（与旧行为逐字节相同）；
/// * **非首都天体** ⇒ 扣该处货栈，**够了才花**（调用方已经用 [`max_affordable_inc`] 把
///   `cost` 卡在站点库存之内）——非首都手上没有的货，只能靠船运过去。
///
/// `spent` 是**本回合这个势力在这类预算上花掉的量**（记账用，与 `build_city` 的
/// `inv_spent`/`con_spent` 同形）：它记的是**账单**，与从哪处库存扣无关。
pub fn commit_spend(
    state: &mut State,
    fid: &str,
    body: &str,
    spent: &mut ResourceMap,
    cost: &[(String, f64)],
) {
    for (rt, c) in cost {
        state.stock_take(fid, body, rt, *c);
        *spent.entry(rt.clone()).or_insert(0.0) += c;
    }
}
