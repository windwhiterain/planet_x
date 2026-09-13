use crate::department::{Department, Policy};
use crate::market::Market;
use crate::warehouse::Warehouse;

const FREE_COST: f32 = 1e-6;

fn policy_potential(policy: &Policy, prices: &[f32]) -> f32 {
    let mut cost = 0.0;
    let mut revenue = 0.0;
    let mut demanded = 0.0;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        let price = prices.get(k).copied().unwrap_or(0.0).max(0.0);
        let consumption = consumption.max(0.0);
        let output = policy.outputs.get(k).copied().unwrap_or(0.0).max(0.0);
        demanded += consumption;
        cost += consumption * price;
        revenue += output * price;
    }
    if policy.is_transform() {
        if cost <= 0.0 {
            return 0.0;
        }
        let margin = revenue - cost;
        if margin <= 0.0 {
            return 0.0;
        }
        return margin / cost;
    }
    let motive = policy.motive.max(0.0);
    if motive <= 0.0 || demanded <= 0.0 {
        return 0.0;
    }
    motive / cost.max(FREE_COST)
}

pub(super) fn plan(
    department: &mut Department,
    warehouse: &mut Warehouse,
    market: &Market,
    local: &[f32],
) {
    let goods = warehouse.stocks.len();
    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        stock.volume += department.productions[k].max(0.0);
    }

    let prices: Vec<f32> = (0..goods)
        .map(|k| {
            let index = market
                .merchandises
                .get(k)
                .map(|merchandise| merchandise.price.max(0.0))
                .unwrap_or(0.0);
            let ratio = local.get(k).copied().unwrap_or(1.0);
            let price = index * ratio;
            if price > 0.0 && price.is_finite() {
                price
            } else {
                index
            }
        })
        .collect();

    let available: Vec<f32> = warehouse
        .stocks
        .iter()
        .map(|stock| stock.volume.max(0.0))
        .collect();
    let mut intake = vec![0.0; goods];
    let mut output = vec![0.0; goods];

    let mut total = 0.0;
    let mut produce_total = 0.0;
    for policy in department.policies.iter_mut() {
        policy.price_potential = policy_potential(policy, &prices);
        policy.distribution = policy.price_potential;
        if policy.is_transform() {
            produce_total += policy.distribution;
        } else {
            total += policy.distribution;
        }
    }

    let mut choice = department.policy_choice;
    if total > 0.0 || produce_total > 0.0 {
        let mut best = f32::NEG_INFINITY;
        for (p, policy) in department.policies.iter_mut().enumerate() {
            let family = if policy.is_transform() {
                produce_total
            } else {
                total
            };
            policy.distribution = if family > 0.0 {
                policy.distribution / family
            } else {
                0.0
            };
            if policy.distribution > best {
                best = policy.distribution;
                choice = p;
            }
            for (k, consumption) in policy.consumptions.iter().enumerate() {
                let produced = policy.outputs.get(k).copied().unwrap_or(0.0);
                intake[k] += policy.distribution * consumption.max(0.0);
                output[k] += policy.distribution * produced.max(0.0);
            }
        }
    } else {
        for policy in department.policies.iter_mut() {
            policy.price_potential = 0.0;
            policy.distribution = 0.0;
        }
    }

    let mut execution: f32 = if total > 0.0 || produce_total > 0.0 {
        1.0
    } else {
        0.0
    };
    for (k, want) in intake.iter().enumerate() {
        if *want > 0.0 {
            execution = execution.min(available[k] / want);
        }
    }
    let execution = execution.clamp(0.0, 1.0);

    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        let net = (output[k] - intake[k]) * execution;
        stock.volume = (stock.volume + net).max(0.0);
        stock.target_volume = intake[k];
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
