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
        // ⚠️ 只在系数**非零**时读估值。`0.0 * f32::INFINITY == NaN`，而 `NaN` 会顺着
        // `margin <= 0` 的判断漏过去（`NaN <= 0.0` 是假），把**整条政策**——包括根本
        // 不用那样货的政策——的得分毒成 `NaN`；`policy_share` 再把 `NaN` 判成 0。实测：
        // 只要部门对**任何**一样商品估出"买不到"（曲线/响应退化时 `best_purchase_cost`
        // 返回 ∞），整个部门就停摆，连不消耗那样货的免费一产政策也一起死（§21）。
        if consumption > 0.0 {
            cost += consumption * unit_buy.get(k).copied().unwrap_or(0.0).max(0.0);
        }
        if output > 0.0 {
            revenue += output * unit_sell.get(k).copied().unwrap_or(0.0).max(0.0);
        }
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

/// 政策估值的**单件价格向量**：把本部门的两条学习曲线读在**上一轮实际经手的量**上。
///
/// ## 为什么不是"1 件"
///
/// §20.13 把部门估值改成"当场对自己的曲线做 argmax / argmin"，但读的量取的是 **1 件**。
/// 对一条**产线**（每轮要经手一整篮货）来说，1 件的读数是**单件最优**，不是价：
///
/// - 收入端 `best_sale_revenue(stock, 1)` 是"只卖一件能榨到的最高收入"。兑现率曲面在小量
///   上饱和，所以这条读数**高于流量的成交价**；
/// - 成本端 `best_purchase_cost(stock, 1)` 是"只买一件能压到的最低花费"。它是**最便宜的
///   一件**，**低于流量的采购价**；而且部门从不买那样货时，这条买入曲线整场不被更新，
///   读数就是一条**先验**。
///
/// 两个偏差方向相反，一起把"工业品价 ÷ 粮食价"这个**换挡信号**压掉（§21 实测：部门读到
/// 的价比市场价低约 2.5 倍，换挡点从 1.104 跑到 **2.7**）。本轮把**收入端**改成在
/// **本部门上一轮实际经手的量**上读，读数就是那条产线真正面对的（平均）成交价：换挡点
/// 回到 **1.7** 附近，而且**输入太贵时两条工艺一起关停**（旧口径在 工/粮 ≈ 2.7 时还在
/// 满负荷跑快工艺）。
///
/// **成本端这一步没修**：`best_purchase_cost(stock, q)` 在可行区间里对 `q` 是线性的
/// （响应曲面那一支返回的收盘量正好是 `q`），所以单位价与 `q` 无关，"最便宜的一件"这个
/// 偏差要靠部门**真的去买**、让曲面学出可行性边界来纠正。而"部门从不买 ⇒ 曲面是冷的 ⇒
/// 读数偏乐观"这条**学习**上的鸡生蛋问题（§21）还开着。
///
/// 还没经手过（第一轮、或单测里没有申报）就退回"1 件"读数——那是旧行为。
/// 已知近似：读的是**当前**流量，所以部门不会把自己这条政策**新增**的量算进去。
fn traded_prices(warehouse: &Warehouse) -> (Vec<f32>, Vec<f32>) {
    let goods = warehouse.stocks.len();
    let mut unit_sell = Vec::with_capacity(goods);
    let mut unit_buy = Vec::with_capacity(goods);
    for k in 0..goods {
        let stock = &warehouse.stocks[k];
        let declared = stock.marketing_volume();
        let sell_volume = if declared > 0.0 { declared.max(1.0) } else { 1.0 };
        let buy_volume = if declared < 0.0 { (-declared).max(1.0) } else { 1.0 };
        let revenue = best_sale_revenue(stock, sell_volume);
        unit_sell.push(if revenue.is_finite() && revenue > 0.0 {
            revenue / sell_volume
        } else {
            0.0
        });
        let cost = best_purchase_cost(stock, buy_volume);
        unit_buy.push(if cost.is_finite() && cost > 0.0 {
            cost / buy_volume
        } else {
            // 这个量买不到（曲面在大量上饱和、或曲线给不出可行报价）：退到一件的读数，
            // 不要让整条政策因为读数是 ∞ 而消失（见 `basket_value` 的零系数注释）。
            best_purchase_cost(stock, 1.0)
        });
    }
    (unit_sell, unit_buy)
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
    // 读的量是**本部门上一轮实际经手的量**（见 [`traded_prices`]），不是"1 件"。
    let (unit_sell, unit_buy) = traded_prices(warehouse);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::warehouse::Stock;

    /// `0.0 × ∞ = NaN` 是这里的头号陷阱：`NaN` 能骗过 `margin <= 0`（`NaN <= 0.0` 是假），
    /// 于是**每一条**政策的得分都变成 `NaN`，`policy_share` 再把它判成 0——一个部门只要
    /// 有**任何一样**货买不到，哪怕它有一条根本不用那样货的政策，也会整场停摆。
    #[test]
    fn a_zero_coefficient_never_reads_an_infinite_price() {
        let unroutable = f32::INFINITY;
        // 商品 0 买不到（`unit_buy = ∞`）、卖得掉（产出价有限）；商品 1 两边都正常。
        let unit_sell = [2.0f32, 3.0];
        let unit_buy = [unroutable, 1.0];

        // 零投入政策：成本必须是 0，不能因为"商品 0 买不到"被毒成 NaN。
        let free = Policy::production(vec![0.0, 0.0], vec![4.0, 0.0]);
        let (cost, revenue, _) = basket_value(&free, &unit_sell, &unit_buy);
        assert_eq!((cost, revenue), (0.0, 4.0 * 2.0), "零系数不该读到 ∞");
        assert!(production_score(&free, &unit_sell, &unit_buy, 1.0) > 0.0);

        // 真的要用那件买不到的货：成本是 ∞（政策不可行），得分 0——但**不是 NaN**。
        let needs_it = Policy::production(vec![1.0, 0.0], vec![4.0, 0.0]);
        let (cost, revenue, _) = basket_value(&needs_it, &unit_sell, &unit_buy);
        assert_eq!(cost, unroutable, "投入真的买不到时成本必须是 ∞");
        assert!(revenue.is_finite());
        assert_eq!(production_score(&needs_it, &unit_sell, &unit_buy, 1.0), 0.0);
    }

    /// 曲线要读在**本部门实际经手的量**上：卖 200 件的单位成交价必须低于只卖 1 件。
    /// 旧口径两种情况都读"1 件"，于是收入端拿的是榨取价——这个偏差把换挡信号压掉（§21）。
    /// （买入侧没有这个量效应，见下面那段注释。）
    #[test]
    fn the_curve_is_read_at_the_traded_quantity_not_at_one_unit() {
        let mut warehouse = Warehouse::new(vec![Stock::new(1000.0, 0.0)]);
        warehouse.stocks[0].set_marketing_volume(1.0);
        let (one, _) = traded_prices(&warehouse);
        warehouse.stocks[0].set_marketing_volume(200.0);
        let (many, _) = traded_prices(&warehouse);
        assert!(
            one[0] > many[0] && many[0] > 0.0,
            "单件读数是榨取价、流量读数是成交价：1 件 {} 对 200 件 {}",
            one[0],
            many[0],
        );

        // 买入侧**没有**这个量效应：`best_purchase_cost(stock, q)` 在可行区间里对 q 是
        // 线性的（曲面在那一支上返回的收盘量正好是 `q`），所以单位价与 q 无关。真正压低
        // 买入读数的是"**最便宜**的可行报价"这件事本身——它要靠部门真的去买、让曲面学出
        // 可行性边界来纠正（§21 里那个"买不到"的坑就是这么来的）。
        let mut warehouse = Warehouse::new(vec![Stock::new(0.0, 1000.0)]);
        warehouse.stocks[0].set_marketing_volume(-1.0);
        let (_, one) = traded_prices(&warehouse);
        warehouse.stocks[0].set_marketing_volume(-200.0);
        let (_, many) = traded_prices(&warehouse);
        assert!(one[0] > 0.0 && (many[0] - one[0]).abs() <= 1e-4 * one[0]);
    }
}
