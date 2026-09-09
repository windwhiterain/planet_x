//! 经济→成本预览（`--control-plan`）。
//!
//! 把「如果按现在的控制面推进一个回合，经济会变成什么样」提前算出来——读 [`budget`] 的
//! 预算、向 [`crate::sim`] 借一次干跑引擎结算、再汇总成一组粗粒度指标。纯分析，不改状态、
//! 不耗调用方 RNG。

use super::budget::{read_budget, BudgetKind};
use super::{r2, PLAN_SEED};
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
/// with a fixed RNG seed, then reads the captured [`RoundFlow`] through [`sim::round_metrics`] —
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
    let metrics = dry_metrics(state, config);
    plan_core(state, config, &metrics, fid)
}

/// The same cost→benefit preview for **every** faction, from a single dry-run (one clone +
/// one [`sim::advance`]); the returned map is keyed by faction id. Exposed via
/// `--control-plan` (no faction argument).
pub fn control_plan_all(state: &State, config: &GameConfig) -> BTreeMap<String, serde_json::Value> {
    let metrics = dry_metrics(state, config);
    state
        .factions
        .iter()
        .filter_map(|f| plan_core(state, config, &metrics, &f.name).map(|v| (f.name.clone(), v)))
        .collect()
}

/// Dry-run one real [`sim::advance`] on a clone (fixed seed) and return the captured
/// [`sim::round_metrics`] — the simulation's own per-round numbers, never re-derived. The
/// caller's state and RNG are untouched.
fn dry_metrics(state: &State, config: &GameConfig) -> RoundMetrics {
    let mut s = state.clone();
    let mut r = Prng::new(PLAN_SEED);
    let flow = sim::advance(&mut s, config, &mut r);
    sim::round_metrics(&s, config, &flow)
}

/// Build one faction's profile from the real `state` (commands / stockpile) and the
/// dry-run `metrics` (production / upkeep / governance for the coming round).
fn plan_core(state: &State, config: &GameConfig, metrics: &RoundMetrics, fid: &str) -> Option<serde_json::Value> {
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

    // 本回合订单（按当前 mode：Ai 重算 / Player 用命令），已含造舰维护保留上限。
    let (con_budget, _) = read_budget(state, config, fid.to_string(), BudgetKind::Construction);
    let (inv_budget, _) = read_budget(state, config, fid.to_string(), BudgetKind::Investment);
    let con_value: f64 = con_budget.iter().map(|(k, v)| v * value_of(k)).sum();
    let inv_value: f64 = inv_budget.iter().map(|(k, v)| v * value_of(k)).sum();
    // AI 自己的保守造舰上限（维护保留后）：用于对比「命令的预算」是否更激进。
    let ai_cap = (stock_value * config.economy.invest_fraction)
        .min((stock_value - upkeep_now * config.economy.upkeep_reserve_mult).max(0.0));

    let Some(fm) = metrics.factions.get(fid) else { return None };

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::world::default_state;

    /// Build the config + a fresh deterministic world (round 0).
    fn fresh_world(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let state = default_state(&config, seed);
        (config, state)
    }

    /// Guard: the cost→benefit preview reports the current economy balance
    /// (`net = production − upkeep − governance`) and, when the agent commands an
    /// over-committed construction budget, flags it *before* it collapses — the
    /// reported "建造预算拉满 → 维护 > 产出 → 城清零" footgun.
    #[test]
    fn control_plan_balances_and_flags_over_committed_construction() {
        let (config, mut state) = fresh_world(42);

        // Baseline: 中国 self-sustaining at the start.
        let plan = control_plan(&state, &config, "中国").expect("faction exists");
        let p = plan["production_value"].as_f64().unwrap();
        let u = plan["upkeep"].as_f64().unwrap();
        let g = plan["governance_cost"].as_f64().unwrap();
        let net = plan["net_flow"].as_f64().unwrap();
        assert!((net - (p - u - g)).abs() < 0.05, "net must ≈ production − upkeep − governance");
        assert_eq!(plan["verdict"].as_str().unwrap(), "healthy");
        assert!(!plan["over_committed_construction"].as_bool().unwrap());
        assert!(plan["rounds_before_insolvent"].is_null());

        // Command a huge construction budget on a held resource with mode=Player.
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "construction_budget": [
                {"resource": "铁", "value": 10000.0, "mode": "Player"},
                {"resource": "碳", "value": 10000.0, "mode": "Player"}
            ]}]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("apply construction over-commit");

        let plan2 = control_plan(&state, &config, "中国").expect("faction exists");
        assert!(
            plan2["construction_budget_value"].as_f64().unwrap() > 0.0,
            "commanded construction budget must be non-zero"
        );
        assert!(
            plan2["over_committed_construction"].as_bool().unwrap(),
            "over-committed construction must be flagged"
        );
        assert_ne!(plan2["verdict"].as_str().unwrap(), "healthy");
    }
}
