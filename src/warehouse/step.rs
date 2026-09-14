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
/// 学到的"局部价 ÷ 参考价"预测的下限。
///
/// `realized` 是拟合量，实测会塌到 1e-27；而 `purchase_scale` 里
/// `affordable = cash / (price × realized)` 于是无界，申报量能算出 1e26
/// （旧代码的注释自己记了这条"遗留"）。这是**数值护栏**，不是"本地价不该低于
/// 参考价的 x%"那种策略边界：只挡住学习器塌陷那一段。
const REALIZED_FLOOR: f32 = 1e-3;

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

/// 买方：**在"期望达标"和"货币"两个约束下尽量少花钱**。
///
/// ```text
/// min   volume × unit                        花费（unit = 参考价 × 学到的局部比率）
/// s.t.  response.get(volume, a) >= need      期望达标——约束是**绝对**的，对着缺口
///       volume × unit <= cash                货币约束
/// ```
///
/// 与旧版的差别全在"约束对着谁"：
/// - 旧版 `threshold = best_probability − 1e-4` 是相对**自己**的最大值定义的，全体没戏时
///   恒成立，于是判据退化成"用最便宜的方式够不着"；这里约束对着 `need`，够不着就是够不着。
/// - 旧版 `goal = target.min(0.9 × share × depth)` 让申报量度量**模型的信念**；
///   这里申报量由"要拿到 `need` 需要挂多少"反解出来（`volume_for_dealt`）。
///
/// **低尺度区域是不可行的**，这正是不再需要"取最小尺度"那种人为下界的原因：
/// 那里 `share(a)` 太小、`volume_for_dealt` 反解不出有限的量，约束直接把它排除。
/// 于是最便宜的解自然落在"刚好能拿下的那个价"上，而不是网格下界。
///
/// 估值上限 `realized <= 1` 是**外层**护栏：局部隐含价高于参考价时不出这个价。
/// 没有它，够不着缺口时"最大化期望成交"那一支会把报价推到网格顶端（`e^44`）。
fn purchase_scale(stock: &Stock, need: f32, price: f32, cash: f32) -> Option<(f32, f32)> {
    if !(need > 0.0) || !(price > 0.0) || !(cash > 0.0) {
        return None;
    }
    let response = stock.buy_response();
    let curve = stock.buy_price_curve();
    let mut cheapest: Option<(f32, f32, f32)> = None; // (cost, volume, scale)
    let mut most: Option<(f32, f32, f32)> = None; // (dealt, volume, scale) 够不着时的尽力
    for index in 0..SCALE_COARSE_STEPS {
        let scale = coarse_log_scale(index).exp();
        if !scale.is_finite() || !(scale > 0.0) {
            continue;
        }
        let realized = curve.get(scale);
        if !realized.is_finite() || !(realized > 0.0) || realized > 1.0 {
            continue;
        }
        let aggressiveness = Stock::buy_aggressiveness(scale);
        // 学习器把 realized 预测到 0 附近时，按 0 计价会让 `cheapest` 与
        // `affordable` 两条路一起跑飞；这里按数值下限计价（见 [`REALIZED_FLOOR`]）。
        let unit = price * realized.max(REALIZED_FLOOR);
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
                    cheapest = Some((cost, volume, scale));
                }
                continue;
            }
        }
        // 这个价拿不到 need（或拿不起）：记下'预算内能拿到最多'的那一档。
        // **实测必须不夹在 need 上**：夹了之后 `motive_ladder=true` 那档从完美稳态
        // （uncleared = 0.000、价格逐位不变 1200 轮）退化成 `uncleared = 1.000` 的发散。
        // 也就是说这个分支实际承担的是"把现金按局部价换成货"的职能，
        // 而约束（期望达标）在学到的天花板偏小时本来就常常不可行。
        // 遗留：`dir` 那档的学到的局部价会塌到 1e-27，于是 `affordable = cash/unit`
        // 算出 3.9e26 的申报量——本轮给 unit 兜了个数值底（见 [`REALIZED_FLOOR`]），但根治仍是 `buy_price_curve` 的塌陷（见 §20）。
        let affordable = affordable_volume(price, realized.max(REALIZED_FLOOR), cash);
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
                most = Some((dealt, affordable, scale));
            }
        }
    }
    cheapest
        .map(|(_, volume, scale)| (volume, scale))
        .or_else(|| most.map(|(_, volume, scale)| (volume, scale)))
}

/// 逐商品把 log 报价尺度按申报量加权去均值，返回归一因子（几何平均尺度）。
///
/// 报价 = `参照价 × 尺度`，参照价 = `指数 × e^楔子`，而指数又由账本反推。于是
/// "所有尺度 ×c、指数 ÷c"不改变任何一笔真实成交——这是个纯规范方向，学习器的目标
/// 对它简并，没有任何东西把它拉回 1。回路增益因此就是"平均 log 尺度"：它每轮只要
/// 不是 1，相对价就乘一次，一路漂到 f32 边界（§19.2）。
///
/// 把每个商品的这个自由度每轮钉到 0，尺度只留相对信息；水平交给 `anchor_prices`。
/// 没有申报量的商品返回 1（不动）。
fn scale_normalization(warehouses: &[Warehouse], goods: usize) -> Vec<f32> {
    let mut log_sum = vec![0.0f32; goods];
    let mut weight = vec![0.0f32; goods];
    for warehouse in warehouses {
        for (k, stock) in warehouse.stocks.iter().enumerate() {
            let declared = stock.marketing_volume.abs();
            let scale = stock.marketing_price_scale;
            if k < goods && declared > 0.0 && scale.is_finite() && scale > 0.0 {
                log_sum[k] += declared * scale.ln();
                weight[k] += declared;
            }
        }
    }
    (0..goods)
        .map(|k| {
            if weight[k] > 0.0 && log_sum[k].is_finite() {
                (log_sum[k] / weight[k]).exp()
            } else {
                1.0
            }
        })
        .collect()
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
    book_forgetting: f32,
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
                    book_forgetting * old + (1.0 - book_forgetting) * new
                } else {
                    new
                }
            };
            books[locality][k] = Book {
                bid: blend(previous.bid, observed_bid),
                ask: blend(previous.ask, observed_ask),
                // 只有**本轮两侧都有真实报价**才算观测。`seen` 只在本轮有人以非零
                // 申报量报价时才会被填，所以这一行同时排除了"零申报量的报价定账本"。
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
    }
    // 规范自由度：把每个商品的 log 尺度去均值（见 [`scale_normalization`]）。
    // 必须在**全部**仓库的尺度都定下来之后、写报价之前做，而且是全局的一趟。
    let normalization = scale_normalization(warehouses, market.merchandises.len());
    for (i, warehouse) in warehouses.iter_mut().enumerate() {
        for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
            let factor = normalization[k];
            if factor.is_finite() && factor > 0.0 {
                stock.marketing_price_scale /= factor;
            }
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
        observe_local_ratio(
            &market.traders[i],
            market,
            &mut local_ratios[locality],
            *book_forgetting,
        );
    }
    // 账本最后更新：这一轮的挂单已经定稿，成交与否都看得见
    update_books(warehouses, market, local_ratios, books, *book_forgetting);
}

fn observe_local_ratio(
    trader: &Trader,
    market: &Market,
    local: &mut Vec<f32>,
    book_forgetting: f32,
) {
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
            book_forgetting * *slot + (1.0 - book_forgetting) * ratio
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
