use super::settlement;
use crate::department::{Department, Policy, Rationing, SettlementReport};
use crate::warehouse::Warehouse;

const FREE_COST: f32 = 1e-6;

const EMPTY_SHELF: f32 = 1e-3;
const POLICY_TEMPERATURE: f32 = 0.25;

fn basket_value(policy: &Policy, bids: &[f32], asks: &[f32]) -> (f32, f32, f32) {
    let mut cost = 0.0;
    let mut revenue = 0.0;
    let mut demanded = 0.0;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        let ask = asks.get(k).copied().unwrap_or(0.0).max(0.0);
        let bid = bids.get(k).copied().unwrap_or(0.0).max(0.0);
        let consumption = consumption.max(0.0);
        let output = policy.outputs.get(k).copied().unwrap_or(0.0).max(0.0);
        demanded += consumption;
        cost += consumption * ask;
        revenue += output * bid;
    }
    (cost, revenue, demanded)
}

fn production_score(policy: &Policy, bids: &[f32], asks: &[f32], ceiling: f32) -> f32 {
    let (cost, revenue, _) = basket_value(policy, bids, asks);
    let margin = revenue - cost;
    if margin <= 0.0 {
        return 0.0;
    }
    margin * ceiling.max(0.0)
}

fn consumption_potential(policy: &Policy, asks: &[f32], capacity: f32) -> f32 {
    let (cost, _, demanded) = basket_value(policy, &[], asks);
    let motive = policy.motive.max(0.0);
    if motive <= 0.0 || demanded <= 0.0 {
        return 0.0;
    }
    let capacity_use = policy.capacity_use();
    if capacity.is_finite() && capacity_use > 0.0 {
        return motive / capacity_use;
    }
    motive / cost.max(FREE_COST)
}

fn material_ceiling(policy: &Policy, warehouse: &Warehouse) -> f32 {
    let mut ceiling = f32::INFINITY;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        if *consumption > 0.0 {
            let command = warehouse
                .stocks
                .get(k)
                .map(|stock| stock.volume.max(0.0))
                .unwrap_or(0.0);
            ceiling = ceiling.min(command / consumption);
        }
    }
    ceiling
}

fn capacity_ceiling(policy: &Policy, capacity: f32) -> f32 {
    let capacity_use = policy.capacity_use();
    if capacity.is_finite() && capacity_use > 0.0 {
        capacity / capacity_use
    } else {
        f32::INFINITY
    }
}

fn policy_share(score: f32, best: f32) -> f32 {
    if !(score > 0.0) || !(best > 0.0) {
        return 0.0;
    }
    ((score / best) / POLICY_TEMPERATURE).exp()
}

pub(super) fn plan(department: &mut Department, warehouse: &mut Warehouse, rationing: Rationing) {
    let goods = warehouse.stocks.len();

    let prices: Vec<f32> = warehouse
        .stocks
        .iter()
        .map(|stock| {
            if stock.price.is_finite() && stock.price > 0.0 {
                stock.price
            } else {
                1.0
            }
        })
        .collect();
    let bids = prices.clone();
    let asks = prices;

    let available: Vec<f32> = warehouse
        .stocks
        .iter()
        .map(|stock| stock.volume.max(0.0))
        .collect();

    let mut ceilings = Vec::with_capacity(department.policies.len());
    let mut reference = 0.0f32;
    for policy in department.policies.iter() {
        let ceiling =
            material_ceiling(policy, warehouse).min(capacity_ceiling(policy, department.capacity));
        if ceiling.is_finite() {
            reference = reference.max(ceiling);
        }
        ceilings.push(ceiling);
    }
    if !(reference > 0.0) {
        reference = 1.0;
    }

    let mut produce_best = 0.0f32;
    for (p, policy) in department.policies.iter_mut().enumerate() {
        if policy.is_production() {
            let ceiling = if ceilings[p].is_finite() {
                ceilings[p]
            } else {
                reference
            };
            policy.price_potential = production_score(policy, &bids, &asks, ceiling);
            produce_best = produce_best.max(policy.price_potential);
        } else {
            policy.price_potential = consumption_potential(policy, &asks, department.capacity);
        }
    }

    let mut choice = department.policy_choice;
    let mut consume_weight = 0.0;
    let mut produce_weight = 0.0;
    for policy in department.policies.iter_mut() {
        policy.distribution = if policy.is_production() {
            policy_share(policy.price_potential, produce_best)
        } else {
            policy.price_potential
        };
        if policy.is_production() {
            produce_weight += policy.distribution;
        } else {
            consume_weight += policy.distribution;
        }
    }

    if consume_weight > 0.0 || produce_weight > 0.0 {
        let mut best_share = f32::NEG_INFINITY;
        for (p, policy) in department.policies.iter_mut().enumerate() {
            let family = if policy.is_production() {
                produce_weight
            } else {
                consume_weight
            };
            policy.distribution = if family > 0.0 {
                policy.distribution / family
            } else {
                0.0
            };
            if policy.distribution > best_share {
                best_share = policy.distribution;
                choice = p;
            }
        }
    } else {
        for policy in department.policies.iter_mut() {
            policy.price_potential = 0.0;
            policy.distribution = 0.0;
        }
    }

    let plans: Vec<Vec<f64>> = department
        .policies
        .iter()
        .map(|policy| {
            policy
                .consumptions
                .iter()
                .map(|consumption| consumption.max(0.0) as f64)
                .collect()
        })
        .collect();
    let outputs: Vec<Vec<f64>> = department
        .policies
        .iter()
        .map(|policy| {
            (0..goods)
                .map(|k| policy.outputs.get(k).copied().unwrap_or(0.0).max(0.0) as f64)
                .collect()
        })
        .collect();
    let unbounded: Vec<bool> = department
        .policies
        .iter()
        .map(|policy| {
            policy
                .consumptions
                .iter()
                .all(|consumption| *consumption <= 0.0)
        })
        .collect();
    let capacity_use: Vec<f64> = department
        .policies
        .iter()
        .zip(unbounded.iter())
        .map(|(policy, unbounded)| {
            let use_per = policy.capacity_use().max(0.0) as f64;
            if use_per > 0.0 {
                use_per
            } else if *unbounded {
                1.0
            } else {
                0.0
            }
        })
        .collect();
    let capacity_budget = if department.capacity.is_finite() && department.capacity > 0.0 {
        department.capacity as f64
    } else {
        capacity_use.iter().sum()
    };
    let willingness: Vec<f64> = department
        .policies
        .iter()
        .map(|policy| {
            if !(policy.price_potential > 0.0) {
                return 0.0;
            }
            if policy.is_production() {
                let (cost, revenue, _) = basket_value(policy, &bids, &asks);
                (revenue - cost).max(0.0) as f64
            } else {
                let cost = basket_value(policy, &[], &asks).0;
                (policy.motive.max(0.0) / cost.max(FREE_COST)) as f64
            }
        })
        .collect();
    let inventory: Vec<f64> = available
        .iter()
        .map(|volume| {
            if volume.is_finite() && *volume > EMPTY_SHELF {
                *volume as f64
            } else {
                0.0
            }
        })
        .collect();

    let desire_use: Vec<f64> = capacity_use
        .iter()
        .map(|use_per| if *use_per > 0.0 { *use_per } else { 1.0 })
        .collect();
    let money_cost: Vec<f64> = department
        .policies
        .iter()
        .map(|policy| basket_value(policy, &[], &asks).0.max(0.0) as f64)
        .collect();
    let cash = department.currency.max(0.0) as f64;
    let desire = |barrier: f32, curvature: f32| -> Vec<f64> {
        settlement::solve(
            &willingness,
            &plans,
            &outputs,
            &[],
            &[
                settlement::Budget {
                    coefficient: &desire_use,
                    limit: capacity_budget,
                },
                settlement::Budget {
                    coefficient: &money_cost,
                    limit: cash,
                },
            ],
            barrier.max(0.0) as f64,
            curvature.max(0.0) as f64,
        )
        .x
    };
    let (_rates, eaten, delivered, wanted_volume, settlement) = match rationing {
        Rationing::Interior { barrier, curvature } => {
            let outcome = settlement::solve(
                &willingness,
                &plans,
                &outputs,
                &inventory,
                &[
                    settlement::Budget {
                        coefficient: &capacity_use,
                        limit: capacity_budget,
                    },
                    settlement::Budget {
                        coefficient: &money_cost,
                        limit: cash,
                    },
                ],
                barrier.max(0.0) as f64,
                curvature.max(0.0) as f64,
            );
            let wanted = desire(barrier, curvature);
            let wanted_volume: Vec<f64> = (0..goods)
                .map(|k| (0..plans.len()).map(|p| plans[p][k] * wanted[p]).sum())
                .collect();
            (
                outcome.x,
                outcome.eaten,
                outcome.delivered,
                wanted_volume,
                outcome.report,
            )
        }
        Rationing::Hard => {
            let rates = hard_rationing(&plans, &inventory);
            let eaten = take(&plans, &rates, goods);
            let delivered = take(&outputs, &rates, goods);
            let utilization = (0..goods).fold(0.0f64, |worst, k| {
                if available[k] > 0.0 {
                    worst.max(eaten[k] / inventory[k])
                } else {
                    worst
                }
            });
            (
                rates,
                eaten.clone(),
                delivered,
                eaten,
                SettlementReport {
                    converged: true,
                    utilization,
                    ..SettlementReport::default()
                },
            )
        }
    };
    let capacity_scale = if capacity_budget > 0.0 {
        let used: f64 = capacity_use
            .iter()
            .zip(_rates.iter())
            .map(|(use_per, rate)| use_per * rate)
            .sum();
        (used / capacity_budget).clamp(0.0, 1.0) as f32
    } else {
        1.0
    };
    let delivery: Vec<f32> = delivered.iter().map(|amount| *amount as f32).collect();

    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        stock.volume = (available[k] + delivery[k] - eaten[k] as f32).max(0.0);
        stock.record_take(wanted_volume[k] as f32, eaten[k] as f32);
    }
    let intake: Vec<f32> = eaten.iter().map(|amount| *amount as f32).collect();

    department.policy_choice = choice;
    department.intake = intake;
    department.delivery = delivery;
    department.capacity_scale = capacity_scale;
    department.settlement = settlement;
}

fn take(plans: &[Vec<f64>], rates: &[f64], goods: usize) -> Vec<f64> {
    let mut eaten = vec![0.0f64; goods];
    for (plan, rate) in plans.iter().zip(rates.iter()) {
        for (k, amount) in plan.iter().enumerate() {
            eaten[k] += amount * rate;
        }
    }
    eaten
}

fn hard_rationing(plans: &[Vec<f64>], inventory: &[f64]) -> Vec<f64> {
    let columns: Vec<f64> = (0..inventory.len())
        .map(|k| plans.iter().map(|plan| plan[k]).sum())
        .collect();
    plans
        .iter()
        .map(|plan| {
            let mut rate = 1.0f64;
            for (k, amount) in plan.iter().enumerate() {
                if *amount > 0.0 && columns[k] > 0.0 {
                    rate = rate.min((inventory[k] / columns[k]).min(1.0));
                }
            }
            rate.max(0.0)
        })
        .collect()
}
