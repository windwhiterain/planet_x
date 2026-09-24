use super::settlement;
use crate::department::{Department, Policy, Rationing, SettlementReport};
use crate::warehouse::Warehouse;

const FREE_COST: f32 = 1e-6;

/// 低于这个存量就当作空货架（见 [`plan`] 里对库存行的处理）
const EMPTY_SHELF: f32 = 1e-3;
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

/// 原料允许这个政策跑多少篮子；没有投入品的政策返回无穷。
/// 钱不在这里：现金约束是结算里的一条预算行。
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

pub(super) fn plan(department: &mut Department, warehouse: &mut Warehouse, rationing: Rationing) {
    let goods = warehouse.stocks.len();

    // 决策价 = **这个仓库自己挂出来的价**（它按"库存对目标 + 未满足意愿"每轮自适应）。
    // 不再读逐地方的账本、也不再回退到指数：仓库自己就是那个地方的边际价值。
    // 一个地方每种货的当地信息由**仓库之间的价格差**经市场势流传递，而不是靠一个
    // 由成交或挂单反解出来的中间价。
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

    // === 结算：**唯一的数量当局** ===
    //
    // 生产与消费同时进结算器，争同一批货与同一份产能。产出也是解给出的，所以约束用
    // **净配方**（投入 − 产出）：`available` 是期初存量，`s_k > 0` 就是期末库存为正。
    // 这一条同时修掉了"生产投入从未被扣除"的质量不守恒。
    //
    // 目标权重 `w_p` 是**每篮效用**，两族的量纲刻意不同、必须各自说清：
    // - 生产：每篮净货币价值（`产出估值 − 投入估价`），所以产能与原料的稀缺直接进价格；
    // - 消费：`motive ÷ 篮子要价`——**这就是需求的价格弹性**。旧版这里传的是裸
    //   `motive`（常数），于是需求在量上对价格毫无反应，"价格涨 → 欲望降 → 价格落"
    //   这条负反馈在输入端就不存在。
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
    // 一条**没有任何投入**的工艺（零投入主生产）在仓库存量上不构成任何一行约束，
    // 于是它唯一的稀缺要素只能是产能。若它的 `capacity_cost` 又是 0（旧夹具、以及
    // 任何没标产能占用的工艺），LP 对它就**没有上界**，产量与库存一起发散——旧代码
    // 是用 `capacity_scale ≤ 1` 隐式挡住这件事的，那等于"整套政策各跑一遍"。
    // 这里把它显式化：没有产能占用的零投入工艺按**一次运行**计价。
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
    // 货架上的"灰"不算库存：存量为正但极小（浮点残渣）时，约束行系数
    // `(投入 − 产出) / 存量` 会放大到 1e5 量级，把内点法的 KKT 系统打成病态
    // （实测残差 1.0、迭代顶到上限）。当成 0 就等于把这一行整条拿掉。
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

    // 现金行的系数 = 一篮货物的要价（按**本仓库自己的挂价**算），上限 = 部门手里的余额。
    // 它同时管两件事：消费的预算约束（买不起就少吃），以及规则 (b) 的 `wanted`
    // ——把库存行拿掉之后，价格高 ⇒ 买得起的篮子数掉下来 ⇒ `wanted` 能落到 `taken`
    // 以下 ⇒ 规则 (b) 的第二半（满足了就降目标）才可达。两向都有，才叫反馈。
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
