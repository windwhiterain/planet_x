use fastrand::Rng;

use crate::estimator::Estimator;
use crate::estimator2d::Estimator2D;
use crate::market::Market;
use crate::market::Trader;
use crate::warehouse::{Book, Stock, Warehouse, Warehouses};

/// 报价尺度的数值边界：见 [`crate::utils::LOG_LIMIT`]。
/// 旧代码这里是 `PRICE_SCALE_FLOOR = 0.25` / `PRICE_SCALE_CEILING = 4.0` +
/// 49 档离散网格——那是三条策略假设：报价最多加价 4 倍、最多让价 4 倍、而且只能取 49 个值。
/// 现在尺度是**连续**的，边界只剩"f32 表示不到更远"这一条。
const SCALE_LOG_LIMIT: f32 = crate::utils::LOG_LIMIT;
/// 粗扫点数：只用来给细化定界，**不是模型常数**（间隔 2×44/128 ≈ 0.69）
const SCALE_COARSE_STEPS: usize = 129;
/// 细化的**相对**对数容差：数值方法的收敛判据，不是"报价精度"。
///
/// 必须是相对量。用绝对值会死循环：f32 在 `|log 尺度| ≈ 44` 处的 ULP 是 3.8e-6，
/// 比 1e-6 还大，区间永远缩不到容差以下。旧网格把尺度限在 [0.25, 4]（log ∈ ±1.39），
/// 所以这条悬崖以前碰不到；把范围放开之后它就露出来了。
const SCALE_TOLERANCE: f32 = 1e-6;
/// 细化迭代上限：即使相对容差也吃不到，也保证终止
const SCALE_REFINE_STEPS: usize = 200;
/// 二分的步数（区间缩到 2^-40）
/// 黄金分割比
const SCALE_GOLDEN: f32 = 0.618_034;
/// 本地参照价缺席时的计价物。**不是银河指数**——指数是读数，回头当输入就又是一圈自指（§20.8）。
const FALLBACK_REFERENCE: f32 = 1.0;

/// 粗扫的第 `index` 个 log 尺度
fn coarse_log_scale(index: usize) -> f32 {
    let fraction = index as f32 / (SCALE_COARSE_STEPS - 1) as f32;
    -SCALE_LOG_LIMIT + 2.0 * SCALE_LOG_LIMIT * fraction
}

fn coarse_log_step() -> f32 {
    2.0 * SCALE_LOG_LIMIT / (SCALE_COARSE_STEPS - 1) as f32
}

/// 一维最大化：粗扫定界 + 黄金分割细化。返回最优的 log 尺度。
///
/// 粗扫只负责把最优点夹进一个格子，真正的解由细化给出——所以"每档几 %"是
/// 收敛容差，不再是模型参数。
pub(super) fn maximize_log_scale(objective: impl Fn(f32) -> f32) -> f32 {
    let mut best_index = 0usize;
    let mut best_value = f32::NEG_INFINITY;
    for index in 0..SCALE_COARSE_STEPS {
        let value = objective(coarse_log_scale(index));
        if value > best_value {
            best_value = value;
            best_index = index;
        }
    }
    if !best_value.is_finite() {
        return 0.0;
    }
    let step = coarse_log_step();
    let centre = coarse_log_scale(best_index);
    let mut low = (centre - step).max(-SCALE_LOG_LIMIT);
    let mut high = (centre + step).min(SCALE_LOG_LIMIT);
    let mut left = high - SCALE_GOLDEN * (high - low);
    let mut right = low + SCALE_GOLDEN * (high - low);
    let mut left_value = objective(left);
    let mut right_value = objective(right);
    for _ in 0..SCALE_REFINE_STEPS {
        let width = high - low;
        let resolution = SCALE_TOLERANCE * (1.0 + low.abs().max(high.abs()));
        if !(width > resolution) {
            break;
        }
        if left_value > right_value {
            high = right;
            right = left;
            right_value = left_value;
            left = high - SCALE_GOLDEN * (high - low);
            left_value = objective(left);
        } else {
            low = left;
            left = right;
            left_value = right_value;
            right = low + SCALE_GOLDEN * (high - low);
            right_value = objective(right);
        }
    }
    let refined = 0.5 * (low + high);
    if objective(refined) >= best_value {
        refined
    } else {
        centre
    }
}

/// 报价维度 **argmax**：用卖出学习曲线，卖 `quantity` 件能拿到的**最好收入**。
///
/// 与 `sale_price` 同一个目标函数——这里只取最优值（给别人估值用），不取报价。
/// 价格不是一个存量数：**每次需要它都在这里现算一遍**。
pub(crate) fn best_sale_revenue(stock: &Stock, quantity: f32) -> f32 {
    if !(quantity > 0.0) {
        return 0.0;
    }
    let response = stock.sell_response();
    let curve = stock.sell_price_curve();
    let log_price = maximize_log_scale(|log_price| {
        let price = log_price.exp();
        let aggressiveness = Stock::sell_aggressiveness(price);
        let dealt = response.get(quantity, aggressiveness);
        let revenue = dealt * curve.get(price);
        if revenue.is_finite() {
            revenue
        } else {
            f32::NEG_INFINITY
        }
    });
    let price = log_price.exp();
    let dealt = response.get(quantity, Stock::sell_aggressiveness(price));
    let revenue = dealt * curve.get(price);
    if revenue.is_finite() && revenue > 0.0 {
        revenue
    } else {
        0.0
    }
}

/// 报价维度 **argmin**：用买入学习曲线，买到 `quantity` 件**至少要花多少钱**。
///
/// 曲线给不出可行报价时返回 `f32::INFINITY`（= 买不到）。
pub(crate) fn best_purchase_cost(stock: &Stock, quantity: f32) -> f32 {
    if !(quantity > 0.0) {
        return 0.0;
    }
    let response = stock.buy_response();
    let curve = stock.buy_price_curve();
    let mut best = f32::INFINITY;
    for index in 0..SCALE_COARSE_STEPS {
        let price = coarse_log_scale(index).exp();
        if !price.is_finite() || !(price > 0.0) {
            continue;
        }
        let unit = curve.get(price);
        if !unit.is_finite() || !(unit > 0.0) {
            continue;
        }
        let aggressiveness = Stock::buy_aggressiveness(price);
        let volume = volume_for_dealt(
            response.share(aggressiveness),
            response.depth(aggressiveness),
            quantity,
        );
        if volume.is_finite() && volume > 0.0 {
            let cost = volume * unit;
            if cost.is_finite() && cost < best {
                best = cost;
            }
        }
    }
    best
}

/// 报价维度 **argmax**：这笔现金按买入学习曲线**最多能换到几件**。
pub(crate) fn affordable_quantity(stock: &Stock, cash: f32) -> f32 {
    if !(cash > 0.0) {
        return 0.0;
    }
    let response = stock.buy_response();
    let curve = stock.buy_price_curve();
    let mut best = 0.0f32;
    for index in 0..SCALE_COARSE_STEPS {
        let price = coarse_log_scale(index).exp();
        if !price.is_finite() || !(price > 0.0) {
            continue;
        }
        let unit = curve.get(price);
        if !unit.is_finite() || !(unit > 0.0) {
            continue;
        }
        let affordable = affordable_volume(unit, cash);
        if !(affordable > 0.0) {
            continue;
        }
        let dealt = response.get(affordable, Stock::buy_aggressiveness(price));
        if dealt.is_finite() && dealt > best {
            best = dealt;
        }
    }
    best
}

fn fluctuation_factor(amplitude: f32, rng: &mut fastrand::Rng) -> f32 {
    let unit = rng.f32().clamp(1e-6, 1.0 - 1e-6);
    if !(amplitude > 0.0) {
        return 1.0;
    }
    (unit / (1.0 - unit)).powf(amplitude)
}

fn declared_volumes(stock: &mut Stock, fluctuation: f32, rng: &mut Rng) {
    // **仓库自适应的目标水位 = 三倍本轮实际取走的量。**
    //
    // 目标不再由部门按当轮计划写死（那会让计划为 0 的商品连目标也是 0，于是永远
    // 没人出价买它 —— §16.1bis 那个死结），也不再靠固定倍率乘除（那是个没有回复力
    // 的乘法游走：不被取空的商品一路下溢到 0，一直取空的商品一路涨到 1e29）。
    // 锚在**观测到的取货量**上就自然有界：取货量受消费能力约束。
    //
    // ⚠️ 锚的是 `stock.taken`（真正离开货架的量），**不是存量的净变化**。
    // 净变化 = 产量 − 取货量，只要产量赶上取货量就被 `.min(0.0)` 夹成 0、
    // 目标塌到下限：实测第 2000 轮九个部门三样商品全部停在下限上
    // （goods 级目标 12.000 = 3×0 + 6×2.0），而取货量是 4.064/部门/轮。
    stock.natural_volume_delta = (stock.volume - stock.previous_volume).min(0.0);
    // 下限是构造时的初始目标：`CAMPAIGN/2`（自有商品是 0，那是对的——生产者的
    // 自有商品本来就该全卖）。没有这个下限，目标会在"没人取货"时塌到 0 并自锁。
    stock.target_volume = (Stock::TARGET_COVER * stock.taken).max(stock.target_floor);
    let gap = stock.volume - stock.target_volume + stock.natural_volume_delta;
    // 旧代码这里还有一个 `clamp(0, |gap|)`：涨落只能**缩小**申报，不能放大。
    // 那也是一条策略假设，去掉。
    stock.declared_gap = gap;
    stock.marketing_volume = gap.abs() * fluctuation_factor(fluctuation, rng) * gap.signum();
}

/// 卖方：挑一个**绝对报价**（不是"参照价的倍数"）最大化 `成交价 × 成交量`。
///
/// 学习曲线 `sell_price_curve` 现在是 `报价 → 成交价`（两个都是绝对值）。卖方的"力度"
/// 是 `1/报价`：报价越高越不激进、成交越少。
fn sale_price(stock: &Stock, available: f32) -> f32 {
    if !(available > 0.0) {
        return FALLBACK_REFERENCE;
    }
    let response = stock.sell_response();
    let curve = stock.sell_price_curve();
    let log_price = maximize_log_scale(|log_price| {
        let price = log_price.exp();
        let aggressiveness = Stock::sell_aggressiveness(price);
        let dealt = response.get(available, aggressiveness);
        let revenue = dealt * curve.get(price);
        if revenue.is_finite() {
            revenue
        } else {
            f32::NEG_INFINITY
        }
    });
    let price = log_price.exp();
    if price.is_finite() && price > 0.0 {
        price
    } else {
        FALLBACK_REFERENCE
    }
}

/// 手上这点钱，在这个**绝对单位成本**下最多能买几件。
fn affordable_volume(unit: f32, cash: f32) -> f32 {
    if unit.is_finite() && unit > 0.0 && cash.is_finite() && cash > 0.0 {
        cash / unit
    } else {
        0.0
    }
}

fn volume_for_dealt(share: f32, depth: f32, goal: f32) -> f32 {
    if !(goal > 0.0) || !(depth > 0.0) || !(share > 0.0) {
        return f32::INFINITY;
    }
    let floor = goal / depth;
    if share > floor {
        let volume = goal / (share - floor);
        if volume.is_finite() && volume > 0.0 && volume >= goal {
            return volume;
        }
    }
    if goal <= depth * (share - 1.0) {
        return goal;
    }
    f32::INFINITY
}

/// 买方：**在"期望达标"和"货币"两个约束下尽量少花钱**。决策变量是**绝对报价**。
///
/// ```text
/// min   volume × unit                         花费（unit = 曲线预测的**绝对**成交价）
/// s.t.  response.get(volume, price) >= need    期望达标——约束对着缺口
///       volume × unit <= cash                  货币约束（cash 是绝对货币）
/// ```
///
/// 绝对口径带来一件关键的事：**现金约束不再随价格水平自动缩放**。旧口径
/// `unit = 参考价 × realized` 里参考价每轮被指数重新缩放，于是价格水平在约束里被约掉，
/// 整个经济没有名义锚（§20）。现在 `cash` 与 `unit` 都是绝对值：报价高了就真的买不起，
/// 这就是水平的回复力。
fn purchase_price(stock: &Stock, need: f32, cash: f32) -> Option<(f32, f32)> {
    if !(need > 0.0) || !(cash > 0.0) {
        return None;
    }
    let response = stock.buy_response();
    let curve = stock.buy_price_curve();
    let mut cheapest: Option<(f32, f32, f32)> = None; // (cost, volume, price)
    let mut most: Option<(f32, f32, f32)> = None; // (dealt, volume, price) 够不着时的尽力
    for index in 0..SCALE_COARSE_STEPS {
        let price = coarse_log_scale(index).exp();
        if !price.is_finite() || !(price > 0.0) {
            continue;
        }
        let unit = curve.get(price);
        if !unit.is_finite() || !(unit > 0.0) {
            continue;
        }
        let aggressiveness = Stock::buy_aggressiveness(price);
        let volume = volume_for_dealt(
            response.share(aggressiveness),
            response.depth(aggressiveness),
            need,
        );
        if volume.is_finite() && volume > 0.0 {
            let cost = volume * unit;
            if cost.is_finite() && cost <= cash {
                let better = match cheapest {
                    Some((best, _, _)) => cost < best,
                    None => true,
                };
                if better {
                    cheapest = Some((cost, volume, price));
                }
                continue;
            }
        }
        // 这个价拿不到 need（或拿不起）：记下"预算内能拿到最多"的那一档。
        let affordable = affordable_volume(unit, cash);
        if !(affordable > 0.0) {
            continue;
        }
        let dealt = response.get(affordable, aggressiveness);
        if dealt.is_finite() && dealt > 0.0 {
            let better = match most {
                Some((best, _, _)) => dealt > best,
                None => true,
            };
            if better {
                most = Some((dealt, affordable, price));
            }
        }
    }
    cheapest
        .map(|(_, volume, price)| (volume, price))
        .or_else(|| most.map(|(_, volume, price)| (volume, price)))
}

/// 把这一轮的成交回灌给两个学习器。**两个都学绝对值**：
/// 价格曲线学 `报价 → 成交价`，响应曲线学 `(申报量, 力度) → 成交量`。
///
/// 旧口径下曲线学的是 `deal_price / 参照价`，那让学习曲线对"价格水平"完全免疫——
/// 水平因此没有任何来自学习器的锚（§20）。
fn observe(stock: &mut Stock, merchandise: &crate::market::TraderMerchandise) {
    let declared = stock.marketing_volume;
    if !declared.is_finite() || declared == 0.0 {
        return;
    }
    let price = stock.marketing_price;
    let dealt = merchandise.deal_volume().abs();
    if declared > 0.0 {
        stock
            .sell_response
            .update(declared, Stock::sell_aggressiveness(price), dealt);
    } else {
        stock
            .buy_response
            .update(declared.abs(), Stock::buy_aggressiveness(price), dealt);
    }
    let deal_price = merchandise.deal_price();
    if price.is_finite() && price > 0.0 && deal_price.is_finite() && deal_price > 0.0 {
        if declared > 0.0 {
            stock.sell_price_curve.update(price, deal_price);
        } else {
            stock.buy_price_curve.update(price, deal_price);
        }
    }
}

/// 逐地方的**决策账本**：同一地方所有挂单并起来取两侧边际价，再按增益混进上一轮。
///
/// 回退链只用**本地量**：上一轮自己的中间价，再不行用计价物。**没有银河指数**——
/// 指数是读数，不能回头当输入（§20.10）。
fn update_books(warehouses: &[Warehouse], market: &Market, books: &mut Vec<Vec<Book>>, book_forgetting: f32) {
    let goods = market.merchandises.len();
    let mut observed: Vec<Vec<Book>> = Vec::new();
    for (i, warehouse) in warehouses.iter().enumerate() {
        let locality = warehouse.locality;
        if observed.len() <= locality {
            observed.resize(locality + 1, vec![Book::default(); goods]);
        }
        let Some(trader) = market.traders.get(i) else {
            continue;
        };
        for (k, merchandise) in trader.merchandises.iter().enumerate() {
            let price = merchandise.price;
            if !price.is_finite() || !(price > 0.0) {
                continue;
            }
            let book = &mut observed[locality][k];
            if merchandise.volume > 0.0 {
                if !(book.ask > 0.0) || price < book.ask {
                    book.ask = price;
                }
            } else if merchandise.volume < 0.0 && price > book.bid {
                book.bid = price;
            }
        }
    }
    for (locality, row) in observed.iter().enumerate() {
        if books.len() <= locality {
            books.resize(locality + 1, vec![Book::default(); goods]);
        }
        if books[locality].len() != goods {
            books[locality] = vec![Book::default(); goods];
        }
        for (k, seen) in row.iter().enumerate() {
            let previous = books[locality][k];
            let carried = if previous.is_formed() { previous.mid() } else { FALLBACK_REFERENCE };
            let observed_bid = if seen.bid > 0.0 { seen.bid } else { carried };
            let observed_ask = if seen.ask > 0.0 { seen.ask } else { carried };
            let blend = |old: f32, new: f32| {
                if old.is_finite() && old > 0.0 {
                    book_forgetting * old + (1.0 - book_forgetting) * new
                } else {
                    new
                }
            };
            books[locality][k] = Book {
                bid: blend(previous.bid, observed_bid),
                ask: blend(previous.ask, observed_ask),
                observed: seen.bid > 0.0 && seen.ask > 0.0,
            };
        }
    }
}

pub(super) fn step(warehouses: &mut Warehouses, market: &mut Market, rng: &mut Rng) {
    let Warehouses {
        warehouses,
        fluctuation,
        book_forgetting,
        local_ratios,
        books,
        ..
    } = warehouses;
    let fluctuation = *fluctuation;
    for (i, warehouse) in warehouses.iter_mut().enumerate() {
        let Warehouse { stocks, currency, .. } = warehouse;
        for stock in stocks.iter_mut() {
            declared_volumes(stock, fluctuation, rng);
        }
        let deficits = stocks
            .iter()
            .filter(|stock| stock.marketing_volume < 0.0)
            .count();
        let budget = if deficits > 0 {
            currency.max(0.0) / deficits as f32
        } else {
            0.0
        };
        for stock in stocks.iter_mut() {
            let price = if stock.marketing_volume > 0.0 {
                sale_price(stock, stock.marketing_volume)
            } else if stock.marketing_volume < 0.0 {
                match purchase_price(stock, stock.marketing_volume.abs(), budget) {
                    Some((volume, price)) => {
                        stock.marketing_volume = -volume;
                        price
                    }
                    None => {
                        stock.marketing_volume = 0.0;
                        stock.purchase_blocked = true;
                        FALLBACK_REFERENCE
                    }
                }
            } else {
                FALLBACK_REFERENCE
            };
            stock.marketing_price = price;
        }
        for (k, stock) in stocks.iter_mut().enumerate() {
            let merchandise = &mut market.traders[i].merchandises[k];
            merchandise.price = stock.marketing_price;
            merchandise.volume = stock.marketing_volume;
        }
    }
    // 卖方报价封顶：**不得超过市场上最高的买价**。
    //
    // ⚠️ 这条是**暂时保留待验**的。它当初是为了堵"卖方收入无界 ⇒ argmax 跑到 f32 边界"
    // 而加的补丁（实测一产 ask 5.55e12）。而那个无界性的真正来源是兑现率曲面里
    // 缺失的水平渐近线——现在 `Response` 的两个响应都饱和了，收入对报价不再是
    // 单调的，理论上封顶就不再必要。`--free-sellers` 用来验证这一点。
    market.step();
    for (i, warehouse) in warehouses.iter_mut().enumerate() {
        for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
            let merchandise = &market.traders[i].merchandises[k];
            stock.volume -= merchandise.deal_volume();
            observe(stock, merchandise);
            stock.previous_volume = stock.volume;
        }
        let locality = warehouse.locality;
        if local_ratios.len() <= locality {
            local_ratios.resize(locality + 1, Vec::new());
        }
        // 只用于诊断：本地成交价 ÷ 上一轮的本地中间价（不是银河指数）。
        let local_reference: Vec<f32> = (0..market.merchandises.len())
            .map(|k| {
                books.get(locality)
                    .and_then(|row| row.get(k))
                    .filter(|book| book.is_formed())
                    .map(|book| book.mid())
                    .unwrap_or(FALLBACK_REFERENCE)
            })
            .collect();
        observe_local_ratio(
            &market.traders[i],
            &local_reference,
            &mut local_ratios[locality],
            *book_forgetting,
        );
    }
    // 账本最后更新：这一轮的挂单已经定稿，成交与否都看得见
    update_books(warehouses, market, books, *book_forgetting);
}

fn observe_local_ratio(
    trader: &Trader,
    reference: &[f32],
    local: &mut Vec<f32>,
    book_forgetting: f32,
) {
    let goods = trader.merchandises.len();
    if local.len() != goods {
        *local = vec![1.0; goods];
    }
    for k in 0..goods {
        let merchandise = &trader.merchandises[k];
        if merchandise.deal_volume() == 0.0 {
            continue;
        }
        let deal = merchandise.deal_price();
        // 比值对着**本地参照价**（本地价），不是银河指数。
        let reference = reference.get(k).copied().unwrap_or(FALLBACK_REFERENCE);
        if !(deal > 0.0) || !deal.is_finite() || !(reference > 0.0) || !reference.is_finite() {
            continue;
        }
        let ratio = deal / reference;
        if !ratio.is_finite() || !(ratio > 0.0) {
            continue;
        }
        let slot = &mut local[k];
        *slot = if slot.is_finite() {
            book_forgetting * *slot + (1.0 - book_forgetting) * ratio
        } else {
            ratio
        };
    }
}
