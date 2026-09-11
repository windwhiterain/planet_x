//! 经济→成本预览（`--control-plan`）。
//!
//! 把「如果按现在的控制面推进一个回合，经济会变成什么样」提前算出来——读 [`budget`] 的
//! 预算、向 [`crate::sim`] 借一次干跑引擎结算、再汇总成一组粗粒度指标。纯分析，不改状态、
//! 不耗调用方 RNG。

use super::budget::{BudgetKind, read_budget};
use super::{PLAN_SEED, r2};
use crate::model::*;
use crate::prng::Prng;
use crate::sim;
use std::collections::BTreeMap;

/// A **cost → benefit preview** for one faction's current control surface: the per-round
/// economy balance the simulation will produce next round (production vs fleet upkeep vs
/// governance), the commanded construction/investment budgets, and a verdict on whether the
/// faction is over-extending its economy.
///
/// The numbers are the **simulation's own**: it dry-runs one real [`sim::advance`] on a clone
/// with a fixed RNG seed, then reads the captured [`RoundSink`] through [`sim::observe`] —
/// so there is zero drift between the preview and what [`sim::advance`] would actually do.
/// (Production, upkeep and governance are RNG-independent, so the fixed seed is just for
/// determinism.) Purely analytical: it never mutates the caller's state and never consumes the
/// caller's RNG.
///
/// Exposed via the agent CLI `--control-plan <faction>`; callers use it to see the cost of a
/// budget before committing it, instead of discovering a collapse by trial and error.
pub fn control_plan(state: &State, config: &GameConfig, fid: &str) -> Option<serde_json::Value> {
    if !state.factions.iter().any(|f| f.name == fid) {
        return None;
    }
    let view = dry_view(state, config);
    plan_core(state, config, &view, fid)
}

/// The same cost→benefit preview for **every** faction, from a single dry-run (one clone +
/// one [`sim::advance`]); the returned map is keyed by faction id. Exposed via
/// `--control-plan` (no faction argument).
pub fn control_plan_all(state: &State, config: &GameConfig) -> BTreeMap<String, serde_json::Value> {
    let view = dry_view(state, config);
    state
        .factions
        .iter()
        .filter_map(|f| plan_core(state, config, &view, &f.name).map(|v| (f.name.clone(), v)))
        .collect()
}

/// Dry-run one real [`sim::advance`] on a clone (fixed seed) and return the round's
/// [`RoundView`] — the simulation's own per-round numbers, never re-derived. The
/// caller's state and RNG are untouched.
fn dry_view(state: &State, config: &GameConfig) -> RoundView {
    let mut s = state.clone();
    let mut r = Prng::new(PLAN_SEED);
    sim::advance(&mut s, config, &mut r)
}

/// Build one faction's profile from the real `state` (commands / stockpile) and the
/// dry-run `view` (production / upkeep / governance for the coming round).
fn plan_core(
    state: &State,
    config: &GameConfig,
    view: &RoundView,
    fid: &str,
) -> Option<serde_json::Value> {
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    // 库存市场价值（当前、未推进）。
    let stock_value: f64 = state
        .faction(fid)
        .map(|f| f.resources.iter().map(|(k, v)| v * value_of(k)).sum())
        .unwrap_or(0.0);
    // 当前舰队维护费（step_upkeep / read_budget 用的同一口径）。
    let upkeep_now: f64 = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| ship_panel(config, s).upkeep)
        .sum();

    // 本回合订单（按当前 mode：Ai 重算 / Player 用命令）。
    //
    // ⚠ `read_budget` 返回的 Player 值已经乘过维护 reserve 的 `con_scale`（真正可执行
    // 额度）；但预览要回答的是「玩家/Runner 命令了多少」，所以 Player 叶单独还原成命令值。
    let (con_budget, con_modes) = read_budget(state, config, fid.to_string(), BudgetKind::Construction);
    let (inv_budget, inv_modes) = read_budget(state, config, fid.to_string(), BudgetKind::Investment);
    let con_value = budget_display_value(
        state,
        fid,
        &con_budget,
        &con_modes,
        &value_of,
        BudgetKind::Construction,
    );
    let inv_value = budget_display_value(
        state,
        fid,
        &inv_budget,
        &inv_modes,
        &value_of,
        BudgetKind::Investment,
    );
    // AI 自己的保守造舰上限（维护保留后）：用于对比「命令的预算」是否更激进。
    let ai_cap = (stock_value * config.economy.invest_fraction)
        .min((stock_value - upkeep_now * config.economy.upkeep_reserve_mult).max(0.0));

    let Some(fm) = view.factions.get(fid) else {
        return None;
    };

    let production = fm.production_value;
    let upkeep = fm.upkeep;
    let governance = fm.governance_cost;
    let net = production - upkeep - governance;
    let feed_cap = (production - governance).max(0.0); // 扣掉治理后能养得起的舰队维护。
    let fleet_overextended = upkeep > feed_cap + 1e-9;
    let over_committed = con_value > ai_cap + 1e-9;
    let net_negative = net < -1e-9;
    let rounds = if net_negative {
        Some((stock_value / -net).max(0.0))
    } else {
        None
    };
    let verdict = if net_negative {
        "bleeding"
    } else if over_committed {
        "over-committed"
    } else {
        "healthy"
    };

    Some(serde_json::json!({
        "faction": fid,
        "round": state.round,
        "production_value": r2(production),
        "upkeep": r2(upkeep),
        "governance_cost": r2(governance),
        "governance_coverage": r2(fm.governance_coverage),
        "net_flow": r2(net),
        "stock_market_value": r2(stock_value),
        "construction_budget_value": r2(con_value),
        "investment_budget_value": r2(inv_value),
        "ai_construction_cap": r2(ai_cap),
        "over_committed_construction": over_committed,
        "fleet_upkeep_cap": r2(feed_cap),
        "fleet_overextended": fleet_overextended,
        "fleet_value": r2(fm.fleet_value),
        "ship_count": fm.ship_count,
        "city_count": fm.city_count,
        "population": fm.population,
        "rounds_before_insolvent": rounds.map(r2),
        "verdict": verdict,
    }))
}

/// Preview 里的预算值：Player 叶显示**命令值**（不是 reserve 缩放后的可执行值），
/// AI 路径显示引擎本回合算出的继承值。
fn budget_display_value(
    state: &State,
    fid: &str,
    budget: &ResourceMap,
    modes: &[(String, ControlMode)],
    value_of: &impl Fn(&str) -> f64,
    kind: BudgetKind,
) -> f64 {
    budget
        .iter()
        .map(|(rt, v)| {
            let mode = modes
                .iter()
                .find(|(r, _)| r == rt)
                .map(|(_, m)| *m)
                .unwrap_or(ControlMode::Auto);
            let raw = if mode.is_player() {
                state
                    .control(fid.to_string())
                    .and_then(|c| match kind {
                        BudgetKind::Investment => c.investment_budget.get(rt),
                        BudgetKind::Construction => c.construction_budget.get(rt),
                    })
                    .map(|c| c.value)
                    .unwrap_or(*v)
            } else {
                *v
            };
            raw * value_of(rt)
        })
        .sum()
}

#[cfg(test)]
#[path = "../tests/autocontrol/economy.rs"]
mod tests;
