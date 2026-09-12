use crate::department::{Department, Policy};
use crate::market::Market;
use crate::warehouse::Warehouse;

const FREE_COST: f32 = 1e-6;

fn policy_potential(policy: &Policy, market: &Market) -> f32 {
    let motive = policy.motive.max(0.0);
    if motive <= 0.0 {
        return 0.0;
    }
    let mut cost = 0.0;
    let mut demanded = 0.0;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        let consumption = consumption.max(0.0);
        demanded += consumption;
        cost += consumption * market.merchandises[k].price.max(0.0);
    }
    if demanded <= 0.0 {
        return 0.0;
    }
    motive / cost.max(FREE_COST)
}

pub(super) fn plan(department: &mut Department, warehouse: &mut Warehouse, market: &Market) {
    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        stock.volume += department.productions[k].max(0.0);
    }

    let available: Vec<f32> = warehouse
        .stocks
        .iter()
        .map(|stock| stock.volume.max(0.0))
        .collect();
    let mut withdrawal = vec![0.0; available.len()];

    let mut total = 0.0;
    for policy in department.policies.iter_mut() {
        policy.price_potential = policy_potential(policy, market);
        policy.distribution = policy.price_potential;
        total += policy.distribution;
    }

    let mut choice = department.policy_choice;
    if total > 0.0 {
        let mut best = f32::NEG_INFINITY;
        for (p, policy) in department.policies.iter_mut().enumerate() {
            policy.distribution /= total;
            if policy.distribution > best {
                best = policy.distribution;
                choice = p;
            }
            for (k, consumption) in policy.consumptions.iter().enumerate() {
                withdrawal[k] += policy.distribution * consumption.max(0.0);
            }
        }
    } else {
        for policy in department.policies.iter_mut() {
            policy.price_potential = 0.0;
            policy.distribution = 0.0;
        }
    }

    let mut execution: f32 = if total > 0.0 { 1.0 } else { 0.0 };
    for (k, want) in withdrawal.iter().enumerate() {
        if *want > 0.0 {
            execution = execution.min(available[k] / want);
        }
    }
    let execution = execution.clamp(0.0, 1.0);

    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        stock.volume = (stock.volume - withdrawal[k] * execution).max(0.0);
    }

    department.policy_choice = choice;
    department.policy_execution = execution;
}

pub(super) fn revenue(market: &Market, i: usize) -> f32 {
    let mut revenue = 0.0;
    for deals in &market.deals[i] {
        for deal in deals {
            revenue += deal.volume * deal.price;
        }
    }
    revenue
}
