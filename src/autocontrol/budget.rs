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
pub(crate) fn read_budget(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    kind: BudgetKind,
) -> (ResourceMap, Vec<(String, ControlMode)>) {
    let stockpile: ResourceMap = state
        .faction(&fid)
        .map(|f| f.resources.clone())
        .unwrap_or_default();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    let stock_value: f64 = stockpile.iter().map(|(k, v)| v * value_of(k)).sum();
    // 造舰的「维护费保留」：自动指挥势力在投入造舰预算前，先从库存里预留 `upkeep ×
    // upkeep_reserve_mult` 的市场价值作为维护底线，只把超出部分用于造舰——「把海军养在
    // 经济能承受的规模」。这样基线 AI 不会无脑大建，避免维护费拖垮经济、军备崩盘。
    // 只对 Construction（造舰）生效；投资基础设施（Investment）不受影响。
    let reserve = if kind == BudgetKind::Construction {
        let upkeep: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
            .map(|s| ship_panel(config, s).upkeep)
            .sum();
        upkeep * config.economy.upkeep_reserve_mult
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
        let value = match mode {
            ControlMode::Ai => ai_value * con_scale,
            ControlMode::Player => state
                .control(fid.clone())
                .and_then(|c| match kind {
                    BudgetKind::Investment => c.investment_budget.get(rt),
                    BudgetKind::Construction => c.construction_budget.get(rt),
                })
                .map(|c| c.value)
                .unwrap_or(ai_value * con_scale),
        };
        budget.insert(rt.clone(), value);
        modes.push((rt.clone(), mode));
    }
    (budget, modes)
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
        for (rt, value) in budget {
            let mode = modes.iter().find(|(r, _)| r == rt).map(|(_, m)| *m).unwrap_or(ControlMode::Ai);
            let slot = match kind {
                BudgetKind::Investment => &mut c.investment_budget,
                BudgetKind::Construction => &mut c.construction_budget,
            };
            match mode {
                ControlMode::Ai => {
                    slot.insert(rt.clone(), Control::inherit(*value));
                }
                ControlMode::Player => {
                    slot.entry(rt.clone()).or_insert_with(|| Control::player(*value));
                }
            }
        }
    }
}
