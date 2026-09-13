use fastrand::Rng;

use crate::estimator::Estimator;
use crate::estimator2d::{Estimator2D, Response};
use crate::market::Market;
use crate::market::Trader;
use crate::utils::normal_cdf;
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
const SCALE_BISECTION_STEPS: usize = 40;
/// 黄金分割比
const SCALE_GOLDEN: f32 = 0.618_034;
/// 超买目标：缺口 × e^(2σ)
const SATURATION_MARGIN: f32 = 2.0;
/// 「同样的达标概率下不肯多花钱」的容差
const PROBABILITY_TOLERANCE: f32 = 1e-4;
const LOCAL_PRICE_FORGETTING: f32 = 0.8;

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

fn fluctuation_factor(amplitude: f32, rng: &mut fastrand::Rng) -> f32 {
    let unit = rng.f32().clamp(1e-6, 1.0 - 1e-6);
    if !(amplitude > 0.0) {
        return 1.0;
    }
    (unit / (1.0 - unit)).powf(amplitude)
}

fn declared_volumes(stock: &mut Stock, fluctuation: f32, rng: &mut Rng) {
    stock.natural_volume_delta = (stock.volume - stock.previous_volume).min(0.0);
    let gap = stock.volume - stock.target_volume + stock.natural_volume_delta;
    // 旧代码这里还有一个 `clamp(0, |gap|)`：涨落只能**缩小**申报，不能放大。
    // 那也是一条策略假设，去掉。
    stock.declared_gap = gap;
    stock.marketing_volume = gap.abs() * fluctuation_factor(fluctuation, rng) * gap.signum();
}

fn sale_scale(stock: &Stock, available: f32, price: f32) -> f32 {
    if !(available > 0.0) || !(price > 0.0) {
        return 1.0;
    }
    let response = stock.sell_response();
    let curve = stock.sell_price_curve();
    let log_scale = maximize_log_scale(|log_scale| {
        let scale = log_scale.exp();
        let aggressiveness = Stock::sell_aggressiveness(scale);
        let dealt = response.get(available, aggressiveness);
        let revenue = dealt * price * curve.get(scale);
        if revenue.is_finite() {
            revenue
        } else {
            f32::NEG_INFINITY
        }
    });
    let scale = log_scale.exp();
    if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    }
}

fn affordable_volume(price: f32, scale: f32, cash: f32) -> f32 {
    let limit = price * scale;
    if limit.is_finite() && limit > 0.0 && cash.is_finite() && cash > 0.0 {
        cash / limit
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

fn purchase_scale(stock: &Stock, need: f32, price: f32, cash: f32) -> Option<(f32, f32)> {
    if !(need > 0.0) || !(price > 0.0) || !(cash > 0.0) {
        return None;
    }
    let response = stock.buy_response();
    let noise = response.noise().max(Response::MIN_NOISE);
    let target = need * (SATURATION_MARGIN * noise).exp();
    // 给定 log 尺度，返回（达标概率，代价，申报量）
    let evaluate = |log_scale: f32| -> (f32, f32, f32) {
        let scale = log_scale.exp();
        if !scale.is_finite() || !(scale > 0.0) {
            return (f32::NEG_INFINITY, f32::INFINITY, 0.0);
        }
        let affordable = affordable_volume(price, scale, cash);
        if !(affordable > 0.0) {
            return (f32::NEG_INFINITY, f32::INFINITY, 0.0);
        }
        let aggressiveness = Stock::buy_aggressiveness(scale);
        let share = response.share(aggressiveness);
        let depth = response.depth(aggressiveness);
        let goal = target.min(0.9 * share * depth);
        let useful = {
            let volume = volume_for_dealt(share, depth, goal);
            if volume.is_finite() {
                volume
            } else {
                9.0 * depth
            }
        };
        let volume = affordable.min(useful);
        if !(volume > 0.0) {
            return (f32::NEG_INFINITY, f32::INFINITY, 0.0);
        }
        let dealt = response.get(volume, aggressiveness);
        let margin = (dealt / need).max(1e-6).ln() / noise;
        let probability = normal_cdf(margin);
        if !probability.is_finite() {
            return (f32::NEG_INFINITY, f32::INFINITY, 0.0);
        }
        (probability, volume * price * scale, volume)
    };

    let mut probabilities = [f32::NEG_INFINITY; SCALE_COARSE_STEPS];
    let mut best_probability = f32::NEG_INFINITY;
    for (index, slot) in probabilities.iter_mut().enumerate() {
        let (probability, _, _) = evaluate(coarse_log_scale(index));
        *slot = probability;
        best_probability = best_probability.max(probability);
    }
    if !best_probability.is_finite() {
        return None;
    }
    // 「同样的达标概率下不肯多花钱」⇒ 找**最小**的可接受尺度。
    // 旧网格靠遍历顺序 + cost 比较实现同一件事；连续域上它是一次二分。
    let threshold = best_probability - PROBABILITY_TOLERANCE;
    let index = probabilities.iter().position(|value| *value >= threshold)?;
    let mut low = if index == 0 {
        coarse_log_scale(0)
    } else {
        coarse_log_scale(index - 1)
    };
    let mut high = coarse_log_scale(index);
    for _ in 0..SCALE_BISECTION_STEPS {
        let mid = 0.5 * (low + high);
        if evaluate(mid).0 >= threshold {
            high = mid;
        } else {
            low = mid;
        }
    }
    let log_scale = 0.5 * (low + high);
    let (_, _, volume) = evaluate(log_scale);
    let scale = log_scale.exp();
    if volume > 0.0 && scale.is_finite() && scale > 0.0 {
        Some((volume, scale))
    } else {
        None
    }
}

fn observe(stock: &mut Stock, merchandise: &crate::market::TraderMerchandise, reference: f32) {
    let declared = stock.marketing_volume;
    if !declared.is_finite() || declared == 0.0 {
        return;
    }
    let scale = stock.marketing_price_scale;
    let dealt = merchandise.deal_volume().abs();
    if declared > 0.0 {
        stock
            .sell_response
            .update(declared, Stock::sell_aggressiveness(scale), dealt);
    } else {
        stock
            .buy_response
            .update(declared.abs(), Stock::buy_aggressiveness(scale), dealt);
    }
    let deal_price = merchandise.deal_price();
    if reference.is_finite() && reference > 0.0 && deal_price.is_finite() && deal_price > 0.0 {
        let realized = deal_price / reference;
        if declared > 0.0 {
            stock.sell_price_curve.update(scale, realized);
        } else {
            stock.buy_price_curve.update(scale, realized);
        }
    }
}

/// 逐地方的账本：把同一地方所有交易者的挂单并起来取两侧边际价，再按增益混进上一轮。
///
/// **只看挂单，不看成交**——这正是它与 `local_ratios`（只在成交时更新）的分工。
/// 缺一侧时走回退链：这一轮挂出来的 → 该地方最近成交价 → 上一轮的账本 → 银河指数。
fn update_books(
    warehouses: &[Warehouse],
    market: &Market,
    local_ratios: &[Vec<f32>],
    books: &mut Vec<Vec<Book>>,
) {
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
                // 卖方：最低的要价才是边际
                if !(book.ask > 0.0) || price < book.ask {
                    book.ask = price;
                }
            } else if merchandise.volume < 0.0 {
                // 买方：最高的出价才是边际
                if price > book.bid {
                    book.bid = price;
                }
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
            let index = market.merchandises[k].price.max(0.0);
            let realized = local_ratios
                .get(locality)
                .and_then(|ratios| ratios.get(k))
                .copied()
                .unwrap_or(0.0)
                * index;
            let previous = books[locality][k];
            let carried = if realized.is_finite() && realized > 0.0 {
                realized
            } else if previous.is_formed() {
                previous.mid()
            } else {
                index
            };
            let observed_bid = if seen.bid > 0.0 { seen.bid } else { carried };
            let observed_ask = if seen.ask > 0.0 { seen.ask } else { carried };
            let blend = |old: f32, new: f32| {
                if old.is_finite() && old > 0.0 {
                    LOCAL_PRICE_FORGETTING * old + (1.0 - LOCAL_PRICE_FORGETTING) * new
                } else {
                    new
                }
            };
            books[locality][k] = Book {
                bid: blend(previous.bid, observed_bid),
                ask: blend(previous.ask, observed_ask),
            };
        }
    }
}

pub(super) fn step(warehouses: &mut Warehouses, market: &mut Market, rng: &mut Rng) {
    let Warehouses {
        warehouses,
        fluctuation,
        local_ratios,
        books,
    } = warehouses;
    let fluctuation = *fluctuation;
    let reference_prices: Vec<f32> = market
        .merchandises
        .iter()
        .map(|merchandise| merchandise.price)
        .collect();
    let references: Vec<Vec<f32>> = warehouses
        .iter()
        .map(|warehouse| {
            if warehouse.reference.len() == reference_prices.len() {
                warehouse.reference.clone()
            } else {
                reference_prices.clone()
            }
        })
        .collect();
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
        for (k, stock) in stocks.iter_mut().enumerate() {
            let price = references[i][k].max(0.0);
            let scale = if stock.marketing_volume > 0.0 {
                sale_scale(stock, stock.marketing_volume, price)
            } else if stock.marketing_volume < 0.0 {
                match purchase_scale(stock, stock.marketing_volume.abs(), price, budget) {
                    Some((volume, scale)) => {
                        stock.marketing_volume = -volume;
                        scale
                    }
                    None => {
                        stock.marketing_volume = 0.0;
                        stock.purchase_blocked = true;
                        1.0
                    }
                }
            } else {
                1.0
            };
            stock.marketing_price_scale = scale;
        }
        for (k, stock) in stocks.iter_mut().enumerate() {
            let merchandise = &mut market.traders[i].merchandises[k];
            merchandise.price = price_of(references[i][k], stock.marketing_price_scale);
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
            observe(stock, merchandise, references[i][k]);
            stock.previous_volume = stock.volume;
        }
        let locality = warehouse.locality;
        if local_ratios.len() <= locality {
            local_ratios.resize(locality + 1, Vec::new());
        }
        observe_local_ratio(&market.traders[i], market, &mut local_ratios[locality]);
    }
    // 账本最后更新：这一轮的挂单已经定稿，成交与否都看得见
    update_books(warehouses, market, local_ratios, books);
}

fn observe_local_ratio(trader: &Trader, market: &Market, local: &mut Vec<f32>) {
    let goods = market.merchandises.len();
    if local.len() != goods {
        *local = vec![1.0; goods];
    }
    for k in 0..goods {
        let merchandise = &trader.merchandises[k];
        if merchandise.deal_volume() == 0.0 {
            continue;
        }
        let deal = merchandise.deal_price();
        let index = market.merchandises[k].price;
        if !(deal > 0.0) || !deal.is_finite() || !(index > 0.0) {
            continue;
        }
        // 旧代码这里还有一个 `clamp(0.05, 20.0)`——"本地价最多是指数的 20 倍"同样是策略假设。
        // 现在只要求比值有限且为正；真正越界时由上面的有限性检查挡掉。
        let ratio = deal / index;
        if !ratio.is_finite() || !(ratio > 0.0) {
            continue;
        }
        let slot = &mut local[k];
        *slot = if slot.is_finite() {
            LOCAL_PRICE_FORGETTING * *slot + (1.0 - LOCAL_PRICE_FORGETTING) * ratio
        } else {
            ratio
        };
    }
}

fn price_of(reference: f32, scale: f32) -> f32 {
    let price = reference.max(0.0) * scale;
    if price.is_finite() && price >= 0.0 {
        price
    } else {
        0.0
    }
}
