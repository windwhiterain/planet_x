use super::settlement;
use crate::department::{Department, Policy, Rationing, SettlementReport};
use crate::market::Market;
use crate::warehouse::{Book, Warehouse};

const FREE_COST: f32 = 1e-6;
/// 分布的软化温度：越小越接近全押一个政策（角点、会抖），越大越平均
const POLICY_TEMPERATURE: f32 = 0.25;

/// 篮子估值：**产出按买价估、投入按卖价估**
///
/// 这是"要跨过价差才算赚"的保守口径。旧口径两边都用同一个中间价，等于假设
/// 自己既能按最高价卖、又能按最低价买——两头占便宜，也正是"过剩原料不会让下游变便宜"
/// 与"没人卖也照样能给投入定价"这两个毛病的来源。
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

/// 生产族：把天花板内的资源全给它能赚多少钱。天花板由调用方统一到同一把尺子上
fn production_score(policy: &Policy, bids: &[f32], asks: &[f32], ceiling: f32) -> f32 {
    let (cost, revenue, _) = basket_value(policy, bids, asks);
    let margin = revenue - cost;
    if margin <= 0.0 {
        return 0.0;
    }
    margin * ceiling.max(0.0)
}

/// 消费族：只看价格，不看资源约束
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

/// 原料允许这个政策跑多少篮子；没有投入品的政策返回无穷
/// （买入力按**卖价**折——那才是你真的要付的价）
fn material_ceiling(policy: &Policy, asks: &[f32], warehouse: &Warehouse) -> f32 {
    let mut ceiling = f32::INFINITY;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        if *consumption > 0.0 {
            let price = asks.get(k).copied().unwrap_or(0.0);
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
    book: &[Book],
    rationing: Rationing,
) {
    let goods = warehouse.stocks.len();

    // 决策价来自**这个部门所在地方的账本**（挂单推出来的、逐地方的），不再来自银河指数。
    // 某一侧没有挂单就回退到指数——那是"没有人愿意在这个方向上成交"的诚实表达。
    let index: Vec<f32> = (0..goods)
        .map(|k| {
            market
                .merchandises
                .get(k)
                .map(|merchandise| merchandise.price.max(0.0))
                .unwrap_or(0.0)
        })
        .collect();
    let side = |pick: fn(&Book) -> f32| -> Vec<f32> {
        (0..goods)
            .map(|k| {
                let quoted = book.get(k).map(pick).unwrap_or(0.0);
                if quoted > 0.0 && quoted.is_finite() {
                    quoted
                } else {
                    index.get(k).copied().unwrap_or(0.0)
                }
            })
            .collect()
    };
    let bids = side(|book| book.bid);
    let asks = side(|book| book.ask);

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
        let ceiling = material_ceiling(policy, &asks, warehouse)
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
            policy.price_potential = production_score(policy, &bids, &asks, ceiling);
            produce_best = produce_best.max(policy.price_potential);
        } else {
            policy.price_potential = consumption_potential(policy, &asks, department.capacity);
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
    let mut delivery = vec![0.0; goods];
    for (k, produced) in output.iter().enumerate() {
        delivery[k] = produced * capacity_scale;
    }
    let supply: Vec<f32> = (0..goods).map(|k| available[k] + delivery[k]).collect();

    // === consumption 结算 ===
    //
    // 一产/二产/三产是**三条彼此独立**的 consumption policy（有哪个就吃那个）。
    // 政策**内部**的配方是一个向量：整篮按同一个执行率缩放；政策**之间**必须彼此独立，
    // 不能因为一条政策缺货就把别的政策一起按下去。
    //
    // 默认走 [`settlement`] 的原始-对偶内点法：约束就是**仓库存量**，目标是
    // `Σ 意愿 × 执行率`，对数障碍负责软化——结构上不超取、解唯一、对数据连续。
    // `Rationing::Hard` 可以切回旧的硬配给做 A/B。
    let plans: Vec<Vec<f64>> = department
        .policies
        .iter()
        .map(|policy| {
            policy
                .consumptions
                .iter()
                .map(|consumption| (policy.distribution * consumption.max(0.0)) as f64)
                .collect()
        })
        .collect();
    let willingness: Vec<f64> = department
        .policies
        .iter()
        .map(|policy| policy.distribution as f64)
        .collect();
    let inventory: Vec<f64> = supply.iter().map(|volume| *volume as f64).collect();

    let (rates, eaten, settlement) = match rationing {
        Rationing::Interior { barrier } => {
            let outcome =
                settlement::solve(&willingness, &plans, &inventory, barrier.max(0.0) as f64);
            (outcome.x, outcome.eaten, outcome.report)
        }
        Rationing::Hard => {
            let rates = hard_rationing(&plans, &inventory);
            let eaten = take(&plans, &rates, goods);
            let utilization = (0..goods).fold(0.0f64, |worst, k| {
                if supply[k] > 0.0 {
                    worst.max(eaten[k] / inventory[k])
                } else {
                    worst
                }
            });
            (
                rates,
                eaten,
                SettlementReport {
                    converged: true,
                    utilization,
                    ..SettlementReport::default()
                },
            )
        }
    };

    // 部门的对外执行率：按"这条政策想吃的篮子有多大"加权。
    // 被判死（配方里有零存量商品）的政策也算进来，执行率是 0——那是诚实的读数。
    let mut weighted = 0.0f32;
    let mut share_weight = 0.0f32;
    for (p, policy) in department.policies.iter().enumerate() {
        if policy.distribution <= 0.0 {
            continue;
        }
        let wants: f32 = policy
            .consumptions
            .iter()
            .filter(|consumption| **consumption > 0.0)
            .sum();
        if wants <= 0.0 {
            continue;
        }
        let weight = policy.distribution * wants;
        share_weight += weight;
        weighted += weight * rates[p] as f32;
    }
    let execution = if share_weight > 0.0 {
        (weighted / share_weight).clamp(0.0, 1.0)
    } else if consume_weight > 0.0 || produce_weight > 0.0 {
        1.0
    } else {
        0.0
    };

    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        // 部门**只管取货**：只结算库存，不写目标。目标归仓库自己按"货架有没有被取空"
        // 自适应（见 `warehouse::step::declared_volumes`）。
        stock.volume = (supply[k] - eaten[k] as f32).max(0.0);
    }
    // 对外报告的就是**实际提货量**（执行率已经打进去了）。
    let intake: Vec<f32> = eaten.iter().map(|amount| *amount as f32).collect();

    department.policy_choice = choice;
    department.policy_execution = execution;
    department.intake = intake;
    department.delivery = delivery;
    department.capacity_scale = capacity_scale;
    department.settlement = settlement;
}

/// 按逐政策执行率把计划用量兑现成实际消耗
fn take(plans: &[Vec<f64>], rates: &[f64], goods: usize) -> Vec<f64> {
    let mut eaten = vec![0.0f64; goods];
    for (plan, rate) in plans.iter().zip(rates.iter()) {
        for (k, amount) in plan.iter().enumerate() {
            eaten[k] += amount * rate;
        }
    }
    eaten
}

/// 旧的硬配给：`x_p = min(1, min_k 存量_k / 该商品的总意愿)`。
///
/// 留在这里只为了 A/B。它对**本政策**取 min 是对的，但解在 LP 的退化面上不唯一，
/// 而且在"某样货恰好归零"处**不连续**。
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

pub(super) fn revenue(market: &Market, i: usize) -> f32 {
    let mut revenue = 0.0;
    for deals in &market.deals[i] {
        for deal in deals {
            revenue += deal.volume * deal.price;
        }
    }
    revenue
}
