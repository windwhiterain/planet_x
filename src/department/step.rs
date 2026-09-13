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

/// 转换族：把全部资源都给它的话能赚多少钱 —— 每个原料与产能各给一条天花板
fn transform_score(policy: &Policy, prices: &[f32], warehouse: &Warehouse, capacity: f32) -> f32 {
    let (cost, revenue, _) = basket_value(policy, prices);
    let margin = revenue - cost;
    if margin <= 0.0 {
        return 0.0;
    }
    margin * activity_ceiling(policy, prices, warehouse, capacity)
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

/// 这个政策独占资源时最多能跑多少：原料与产能各给一条上界。
/// 原料按「支配力」算 —— 手里有的，加上现金还买得起的；否则买方（手里本来就没货）会被判死刑
fn activity_ceiling(policy: &Policy, prices: &[f32], warehouse: &Warehouse, capacity: f32) -> f32 {
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
    let capacity_use = policy.capacity_use();
    if capacity.is_finite() && capacity_use > 0.0 {
        ceiling = ceiling.min(capacity / capacity_use);
    }
    if ceiling.is_finite() {
        ceiling.max(0.0)
    } else {
        1.0
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

    let mut produce_best = 0.0f32;
    for policy in department.policies.iter_mut() {
        if policy.is_transform() {
            policy.price_potential =
                transform_score(policy, &prices, warehouse, department.capacity);
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
        policy.distribution = if policy.is_transform() {
            policy_share(policy.price_potential, produce_best)
        } else {
            policy.price_potential
        };
        if policy.is_transform() {
            produce_weight += policy.distribution;
        } else {
            consume_weight += policy.distribution;
        }
    }

    if consume_weight > 0.0 || produce_weight > 0.0 {
        let mut best_share = f32::NEG_INFINITY;
        for (p, policy) in department.policies.iter_mut().enumerate() {
            let family = if policy.is_transform() {
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

    let mut execution: f32 = if consume_weight > 0.0 || produce_weight > 0.0 {
        1.0
    } else {
        0.0
    };
    for (k, want) in intake.iter().enumerate() {
        if *want > 0.0 {
            execution = execution.min(available[k] / want);
        }
    }
    if capacity_want > 0.0 {
        execution = execution.min(department.capacity / capacity_want);
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
