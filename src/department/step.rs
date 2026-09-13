use crate::department::{Department, Policy};
use crate::market::Market;
use crate::warehouse::Warehouse;

const FREE_COST: f32 = 1e-6;
/// 分布的软化温度：越小越接近全押一个政策（角点、会抖），越大越平均
const POLICY_TEMPERATURE: f32 = 0.25;

fn basket_value(policy: &Policy, prices: &[f32]) -> (f32, f32, f32) {
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
    (cost, revenue, demanded)
}

/// 生产族：把天花板内的资源全给它能赚多少钱。天花板由调用方统一到同一把尺子上
fn production_score(policy: &Policy, prices: &[f32], ceiling: f32) -> f32 {
    let (cost, revenue, _) = basket_value(policy, prices);
    let margin = revenue - cost;
    if margin <= 0.0 {
        return 0.0;
    }
    margin * ceiling.max(0.0)
}

/// 消费族：只看价格，不看资源约束
fn consumption_potential(policy: &Policy, prices: &[f32], capacity: f32) -> f32 {
    let (cost, _, demanded) = basket_value(policy, prices);
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

/// 原料允许这个政策跑多少篮子；没有投入品的政策返回无穷
fn material_ceiling(policy: &Policy, prices: &[f32], warehouse: &Warehouse) -> f32 {
    let mut ceiling = f32::INFINITY;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        if *consumption > 0.0 {
            let price = prices.get(k).copied().unwrap_or(0.0);
            let buying_power = if price > 0.0 && warehouse.currency > 0.0 {
                warehouse.currency / price
            } else {
                0.0
            };
            let command = warehouse
                .stocks
                .get(k)
                .map(|stock| stock.volume.max(0.0))
                .unwrap_or(0.0)
                + buying_power;
            ceiling = ceiling.min(command / consumption);
        }
    }
    ceiling
}

/// 产能允许这个政策跑多少篮子；没有产能占用或产能不限时返回无穷
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

pub(super) fn plan(
    department: &mut Department,
    warehouse: &mut Warehouse,
    market: &Market,
    local: &[f32],
) {
    let goods = warehouse.stocks.len();

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

    let mut ceilings = Vec::with_capacity(department.policies.len());
    let mut reference = 0.0f32;
    for policy in department.policies.iter() {
        let ceiling = material_ceiling(policy, &prices, warehouse)
            .min(capacity_ceiling(policy, department.capacity));
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
            policy.price_potential = production_score(policy, &prices, ceiling);
            produce_best = produce_best.max(policy.price_potential);
        } else {
            policy.price_potential = consumption_potential(policy, &prices, department.capacity);
        }
    }

    let mut choice = department.policy_choice;
    let mut capacity_want = 0.0;
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
            capacity_want += policy.distribution * policy.capacity_use();
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

    let capacity_scale = if capacity_want > 0.0 {
        (department.capacity / capacity_want).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let mut execution: f32 = if consume_weight > 0.0 || produce_weight > 0.0 {
        1.0
    } else {
        0.0
    };
    let mut delivery = vec![0.0; goods];
    for (k, want) in intake.iter().enumerate() {
        delivery[k] = output[k] * capacity_scale;
        if *want > 0.0 {
            execution = execution.min((available[k] + delivery[k]) / want);
        }
    }
    let execution = execution.clamp(0.0, 1.0);

    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        stock.volume = (available[k] + delivery[k] - intake[k] * execution).max(0.0);
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
