use super::settlement;
use crate::department::{Department, Policy, Rationing, SettlementReport};
use crate::market::Market;
use crate::warehouse::Warehouse;
use crate::warehouse::step::{affordable_quantity, best_purchase_cost, best_sale_revenue};

const FREE_COST: f32 = 1e-6;
/// 分布的软化温度：越小越接近全押一个政策（角点、会抖），越大越平均
const POLICY_TEMPERATURE: f32 = 0.25;

/// 篮子的**曲线估值**：产出按本部门**卖出学习曲线**的 argmax 估、投入按**买入学习曲线**
/// 的 argmin 估。
///
/// 这里**不读任何挂牌价 / 账本 / 指数**——价格不是一个存量数：每次需要它，就在对应曲线上
/// 现算一遍 argmax/argmin（§20.13）。`unit_sell` / `unit_buy` 是"一件"的估值，由调用方用
/// [`best_sale_revenue`] / [`best_purchase_cost`] 现算。
fn basket_value(policy: &Policy, unit_sell: &[f32], unit_buy: &[f32]) -> (f32, f32, f32) {
    let mut cost = 0.0;
    let mut revenue = 0.0;
    let mut demanded = 0.0;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        let consumption = consumption.max(0.0);
        let output = policy.outputs.get(k).copied().unwrap_or(0.0).max(0.0);
        demanded += consumption;
        cost += consumption * unit_buy.get(k).copied().unwrap_or(0.0).max(0.0);
        revenue += output * unit_sell.get(k).copied().unwrap_or(0.0).max(0.0);
    }
    (cost, revenue, demanded)
}

/// 生产族：把天花板内的资源全给它能赚多少钱（收入与成本都由曲线现算）。
fn production_score(policy: &Policy, unit_sell: &[f32], unit_buy: &[f32], ceiling: f32) -> f32 {
    let (cost, revenue, _) = basket_value(policy, unit_sell, unit_buy);
    let margin = revenue - cost;
    if margin <= 0.0 {
        return 0.0;
    }
    margin * ceiling.max(0.0)
}

/// 消费族：只看曲线给的买入成本，不看资源约束。
fn consumption_potential(policy: &Policy, unit_buy: &[f32], capacity: f32) -> f32 {
    let (cost, _, demanded) = basket_value(policy, &[], unit_buy);
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

/// 原料允许这个政策跑多少篮子；没有投入品的政策返回无穷。
///
/// 买入力由**买入学习曲线**现算（[`affordable_quantity`]），不读挂牌价。
fn material_ceiling(policy: &Policy, buying_power: &[f32], warehouse: &Warehouse) -> f32 {
    let mut ceiling = f32::INFINITY;
    for (k, consumption) in policy.consumptions.iter().enumerate() {
        if *consumption > 0.0 {
            let command = warehouse
                .stocks
                .get(k)
                .map(|stock| stock.volume.max(0.0))
                .unwrap_or(0.0)
                + buying_power.get(k).copied().unwrap_or(0.0).max(0.0);
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
    rationing: Rationing,
) {
    let goods = warehouse.stocks.len();

    // 一切"价格"都在**这里**由本部门自己的学习曲线现算（报价维度 argmax / argmin）：
    // 不读账本、不读本地价、不读指数，也没有任何"价格"标量被存下来（§20.13）。
    let unit_sell: Vec<f32> = (0..goods)
        .map(|k| best_sale_revenue(&warehouse.stocks[k], 1.0))
        .collect();
    let unit_buy: Vec<f32> = (0..goods)
        .map(|k| best_purchase_cost(&warehouse.stocks[k], 1.0))
        .collect();
    // 手里的现金按买入曲线最多能换到几件（逐商品各自按整份现金算，与旧口径一致）。
    let buying_power: Vec<f32> = (0..goods)
        .map(|k| affordable_quantity(&warehouse.stocks[k], warehouse.currency))
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
        let ceiling = material_ceiling(policy, &buying_power, warehouse)
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
            policy.price_potential = production_score(policy, &unit_sell, &unit_buy, ceiling);
            produce_best = produce_best.max(policy.price_potential);
        } else {
            policy.price_potential = consumption_potential(policy, &unit_buy, department.capacity);
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
    // 政策**内部**的配方是一个向量：整篮按同一个篮子数缩放；政策**之间**必须彼此独立，
    // 不能因为一条政策缺货就把别的政策一起按下去。
    //
    // 约束是**裸配方**（不再乘 `distribution`），目标是**绝对 motive**（不再归一化）。
    // 旧版把归一化后的 `distribution` 同时当目标系数和物质量，于是 motive 的绝对大小
    // 完全不起作用（全乘 1000 输出逐位不变），需求在量上对价格也毫无弹性。
    // `distribution` 现在只是"这条政策在族里占多大份额"的读数。
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
    // 参与与否仍由**经济可行性**把关（`price_potential > 0`：亏损的生产政策、
    // 空篮子、零 motive 都不参与），但参与之后的目标权重是它自己的 motive。
    let willingness: Vec<f64> = department
        .policies
        .iter()
        .map(|policy| {
            if policy.price_potential > 0.0 {
                policy.motive.max(0.0) as f64
            } else {
                0.0
            }
        })
        .collect();
    let inventory: Vec<f64> = supply.iter().map(|volume| *volume as f64).collect();

    let (_rates, eaten, settlement) = match rationing {
        Rationing::Interior {
            barrier,
            curvature,
        } => {
            let outcome = settlement::solve(
                &willingness,
                &plans,
                &inventory,
                barrier.max(0.0) as f64,
                curvature.max(0.0) as f64,
            );
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

    for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
        // 部门**只管取货**：只结算库存，不写目标。目标归仓库自己按"货架有没有被取空"
        // 自适应（见 `warehouse::step::declared_volumes`）。
        stock.volume = (supply[k] - eaten[k] as f32).max(0.0);
        // 把**本轮实际取走的量**交给仓库：目标水位 = 三倍这个量，锚在取货量上
        // 而不是锚在存量的净变化上（见 `Stock::taken`）。
        stock.record_take(eaten[k] as f32);
    }
    // 对外报告的就是**实际提货量**（执行率已经打进去了）。
    let intake: Vec<f32> = eaten.iter().map(|amount| *amount as f32).collect();

    department.policy_choice = choice;
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
