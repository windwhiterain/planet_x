//! 预算重算（Ai 每回合从库存重算，Player 只读命令）。

use crate::model::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetKind {
    Investment,
    Construction,
}

/// Read a faction's per-resource budget for a kind: AI resources are recomputed
/// from the stockpile (`stockpile × invest_fraction`), player resources keep the
/// commanded value. Returns the budget map and the per-resource control modes.
/// 势力库存 = **首都池 + 各处产地货栈**（与预算基数同源）。
pub(crate) fn faction_stockpile(state: &State, fid: &str) -> ResourceMap {
    let mut stockpile: ResourceMap = state
        .faction(fid)
        .map(|f| f.resources.clone())
        .unwrap_or_default();
    for (_, m) in state.depots.iter().filter(|((f, _), _)| f == fid) {
        for (rt, amt) in m {
            *stockpile.entry(rt.clone()).or_insert(0.0) += amt;
        }
    }
    stockpile
}

/// 一张资源表按配置市值的总价值（预算上限/联合上限共用同一把尺子）。
pub(crate) fn resource_map_value(config: &GameConfig, map: &ResourceMap) -> f64 {
    map.iter()
        .map(|(rt, amt)| amt * config.resources.get(rt).map(|r| r.value).unwrap_or(1.0))
        .sum()
}

/// 势力活舰维护费（`upkeep_reserve` 的同一口径）。
pub(crate) fn faction_upkeep(state: &State, config: &GameConfig, fid: &str) -> f64 {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| ship_panel(config, s).upkeep)
        .sum()
}

pub(crate) fn read_budget(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    kind: BudgetKind,
) -> (ResourceMap, Vec<(String, ControlMode)>) {
    // **预算的基数 = 势力的全部实物**（首都池 + 各处产地货栈），不是只有池子那一份。
    //
    // 消耗侧改成「首都 ⇒ 池子、其余 ⇒ 本地货栈」之后（完全禁止瞬移），**只有池子**当基数
    // 会让「矿全在殖民地货栈里、池子空着」的势力把预算算成 0——它不是没钱，是钱在别的星球上。
    // 反过来，预算大也不等于能凭空花：每座城还各自被**本地库存**卡一道
    //（`sim::site_affordable`），所以这一条只决定「这个月愿意投多少」，决定不了「买不买得起」。
    let stockpile = faction_stockpile(state, &fid);
    let stock_value = resource_map_value(config, &stockpile);
    // 造舰的「维护费保留」：自动指挥势力在投入造舰预算前，先从库存里预留 `upkeep ×
    // upkeep_reserve_mult` 的市场价值作为维护底线，只把超出部分用于造舰——「把海军养在
    // 经济能承受的规模」。这样基线 AI 不会无脑大建，避免维护费拖垮经济、军备崩盘。
    // 只对 Construction（造舰）生效；投资基础设施（Investment）不受影响。
    let reserve = if kind == BudgetKind::Construction {
        faction_upkeep(state, config, &fid) * config.economy.upkeep_reserve_mult
    } else {
        0.0
    };
    let build_value = stock_value * config.economy.invest_fraction;
    // 造舰预算允许的「上限」（市场价值）：不把库存打到维护底线之下。
    let con_cap = if kind == BudgetKind::Construction {
        (build_value).min((stock_value - reserve).max(0.0))
    } else {
        build_value
    };
    let con_scale = if kind == BudgetKind::Construction && build_value > 1e-9 {
        (con_cap / build_value).clamp(0.0, 1.0)
    } else {
        1.0
    };

    let mut budget: ResourceMap = ResourceMap::new();
    let mut modes = Vec::new();
    for (rt, v) in &stockpile {
        let ai_value = *v * config.economy.invest_fraction;
        let mode = match kind {
            BudgetKind::Investment => state.investment_budget_control(fid.clone(), rt),
            BudgetKind::Construction => state.construction_budget_control(fid.clone(), rt),
        };
        let value = if mode.is_player() {
            state
                .control(fid.clone())
                .and_then(|c| match kind {
                    BudgetKind::Investment => c.investment_budget.get(rt),
                    BudgetKind::Construction => c.construction_budget.get(rt),
                })
                .map(|c| c.value * con_scale)
                .unwrap_or(ai_value * con_scale)
        } else {
            ai_value * con_scale
        };
        budget.insert(rt.clone(), value);
        modes.push((rt.clone(), mode));
    }
    (budget, modes)
}

/// 把同一势力的投资 + 建造预算联合压到 `库存市值 − 维护费 reserve` 之内（P1-5）。
///
/// 单个预算由 [`read_budget`] 各自限制；但 Investment 与 Construction 同时花，两者之和仍
/// 可能突破 reserve。这里只做一笔**比例缩放**：两笔预算保持相对优先级，总和不超过底线。
/// 实际花费仍由 `site_affordable` 按本地库存守门。
pub(crate) fn cap_joint_budgets(
    state: &State,
    config: &GameConfig,
    fid: &str,
    investment: &mut ResourceMap,
    construction: &mut ResourceMap,
) {
    let stock_value = resource_map_value(config, &faction_stockpile(state, fid));
    let reserve = faction_upkeep(state, config, fid) * config.economy.upkeep_reserve_mult;
    let cap = (stock_value - reserve).max(0.0);
    let requested = resource_map_value(config, investment)
        + resource_map_value(config, construction);
    if requested <= cap + 1e-9 || requested <= 1e-9 {
        return;
    }
    let scale = (cap / requested).clamp(0.0, 1.0);
    for v in investment.values_mut() {
        *v *= scale;
    }
    for v in construction.values_mut() {
        *v *= scale;
    }
}

/// Write a computed budget back into the faction's controllable state so the
/// diff between rounds reflects what the simulation actually used.
pub(crate) fn write_budget(
    state: &mut State,
    fid: FactionId,
    kind: BudgetKind,
    budget: &ResourceMap,
    modes: &[(String, ControlMode)],
) {
    if let Some(c) = state.control_mut(fid.clone()) {
        // P2-9：清掉本回合不再出现的旧叶子，避免控制读面/后续回合看见“幽灵预算”。
        // ⚠ Player 叶是玩家的持久命令，即使当前库存里没有这个 key 也保留。
        {
            let slot = match kind {
                BudgetKind::Investment => &mut c.investment_budget,
                BudgetKind::Construction => &mut c.construction_budget,
            };
            slot.retain(|rt, leaf| budget.contains_key(rt) || leaf.mode.is_player());
        }
        for (rt, value) in budget {
            let mode = modes
                .iter()
                .find(|(r, _)| r == rt)
                .map(|(_, m)| *m)
                .unwrap_or(ControlMode::Auto);
            let slot = match kind {
                BudgetKind::Investment => &mut c.investment_budget,
                BudgetKind::Construction => &mut c.construction_budget,
            };
            if mode.is_player() {
                // 玩家指令：值由玩家给，系统只是把「玩家会用的那个值」抄进读面——
                // 叶子已存在时绝不覆盖（`or_insert_with`）。
                // ⚠ 注意 `read_budget` 读的时候已经乘过 `con_scale`（维护费 reserve 保护），
                // 所以这里写回的是**保护后的可执行额度**，玩家叶本身不动。
                slot.entry(rt.clone())
                    .or_insert_with(|| Control::player(*value));
            } else {
                // 系统自动决定：写成**继承**叶（`mode = Inherit`）——它的值只是「本回合
                // 实际用了多少」的流水记录，不构成指令。
                slot.insert(rt.clone(), Control::inherit(*value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::world::default_state;

    /// **P2-9：`write_budget` 清掉本回合不存在的旧 AI 叶，但保留 Player 命令**。
    #[test]
    fn write_budget_prunes_stale_auto_keys_but_keeps_player_commands() {
        let config = load_config();
        let mut state = default_state(&config, 42);
        let fid = state.factions[0].name.clone();
        {
            let c = state.control.entry(fid.clone()).or_default();
            c.construction_budget
                .insert("旧自动".to_string(), Control::inherit(1.0));
            c.construction_budget
                .insert("旧玩家".to_string(), Control::player(2.0));
        }
        write_budget(
            &mut state,
            fid.clone(),
            BudgetKind::Construction,
            &ResourceMap::new(),
            &[],
        );
        let c = state.control(fid).expect("control 还在");
        assert!(
            !c.construction_budget.contains_key("旧自动"),
            "旧 AI 叶应被 prune，避免幽灵预算"
        );
        assert!(
            c.construction_budget.contains_key("旧玩家"),
            "Player 叶是持久命令，即使当前库存没有该 key 也要保留"
        );
    }

    /// **P1-5：Player 的 construction 预算也受维护费 reserve 约束**。
    ///
    /// 低库存、高出勤舰队时，玩家批再大的 construction 额度，可执行值也必须被
    /// `con_scale` 压到 0；库存垫厚后同一片叶才恢复原值（守卫不空转）。
    #[test]
    fn player_construction_budget_respects_the_upkeep_reserve() {
        let config = load_config();
        let mut state = default_state(&config, 42);
        let fid = state
            .factions
            .iter()
            .find(|f| {
                state
                    .ships
                    .iter()
                    .any(|s| s.faction_id == f.name && s.hull > 0.0)
            })
            .map(|f| f.name.clone())
            .expect("世界至少有一个带舰势力");
        let rt = config
            .resources
            .keys()
            .next()
            .cloned()
            .expect("配置至少一种资源");
        let value = config.resources.get(&rt).map(|r| r.value).unwrap_or(1.0);

        // 玩家把 construction 叶写大；库存却只有一点点 ⇒ reserve 把可执行额度压到 0。
        if let Some(f) = state.faction_mut(&fid) {
            f.resources.clear();
            f.resources.insert(rt.clone(), 1e-6 / value.max(1e-9));
        }
        state
            .control_mut(fid.clone())
            .expect("势力有 control")
            .construction_budget
            .insert(rt.clone(), Control::player(1e9));
        let (budget, _) = read_budget(&state, &config, fid.clone(), BudgetKind::Construction);
        assert_eq!(
            budget.get(&rt).copied().unwrap_or(0.0),
            0.0,
            "低库存时玩家 construction 预算必须被 reserve 压到 0"
        );

        // 把库存垫到远高于 reserve，同一片叶应该恢复成玩家值（proves the guard isn't vacuous）。
        if let Some(f) = state.faction_mut(&fid) {
            f.resources.insert(rt.clone(), 1e9);
        }
        state
            .control_mut(fid.clone())
            .unwrap()
            .construction_budget
            .insert(rt.clone(), Control::player(123.0));
        let (budget, _) = read_budget(&state, &config, fid, BudgetKind::Construction);
        assert!(
            (budget.get(&rt).copied().unwrap_or(0.0) - 123.0).abs() < 1e-9,
            "库存垫厚后玩家叶应恢复为可执行值 123，实为 {:?}",
            budget.get(&rt)
        );
    }
    /// **P1-5：投资与建造预算有联合上限**，两者同时花不能击穿维护费 reserve。
    #[test]
    fn joint_investment_and_construction_budgets_stay_above_the_reserve() {
        let config = load_config();
        let mut state = default_state(&config, 42);
        let fid = state
            .factions
            .iter()
            .find(|f| {
                state
                    .ships
                    .iter()
                    .any(|s| s.faction_id == f.name && s.hull > 0.0)
            })
            .map(|f| f.name.clone())
            .expect("世界至少有一个带舰势力");
        for f in state.factions.iter_mut() {
            f.resources.clear();
        }
        state.depots.clear();
        let reserve = faction_upkeep(&state, &config, &fid)
            * config.economy.upkeep_reserve_mult;
        assert!(reserve > 0.0, "用例前提：该势力有维护费 reserve");
        // 库存恰好比 reserve 多 100（用市值为 1 的资源摆出来）。
        let rt = config
            .resources
            .iter()
            .find(|(_, spec)| (spec.value - 1.0).abs() < 1e-9)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| config.resources.keys().next().unwrap().clone());
        let value = config.resources.get(&rt).map(|r| r.value).unwrap_or(1.0);
        if let Some(f) = state.faction_mut(&fid) {
            f.resources.insert(rt.clone(), (reserve + 100.0) / value);
        }
        let mut investment = ResourceMap::new();
        investment.insert(rt.clone(), 1000.0);
        let mut construction = ResourceMap::new();
        construction.insert(rt.clone(), 1000.0);
        cap_joint_budgets(&state, &config, &fid, &mut investment, &mut construction);
        let total = resource_map_value(&config, &investment)
            + resource_map_value(&config, &construction);
        assert!(
            total <= 100.0 + 1e-6,
            "两笔预算之和必须被联合闸压到库存 − reserve = 100 以内，实为 {total}"
        );
        assert!(
            total > 0.0,
            "reserve 之上还有 100 的可用价值，不该把预算全部归零"
        );
    }

}
