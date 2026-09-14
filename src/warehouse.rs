pub(crate) mod step;

#[cfg(test)]
mod tests;

use crate::{
    estimator::PowerLaw,
    estimator2d::Response,
    market::Market,
};
use fastrand::Rng;

pub struct Warehouses {
    pub warehouses: Vec<Warehouse>,
    pub fluctuation: f32,
    /// 响应曲面（[`Response`]）的学习率（forgetting）。越小追得越快。
    pub response_forgetting: f32,
    /// 价格曲线（[`PowerLaw`]）的学习率（forgetting）。§19.5 扫出来的承重旋钮。
    pub price_forgetting: f32,
    /// 账本与本地比值的记忆。旧实现里是常量 `LOCAL_PRICE_FORGETTING`。
    pub book_forgetting: f32,
    /// index with 地方：同一地方共享一套「本地成交价 ÷ 指数」的比值（**已实现**的价）
    pub local_ratios: Vec<Vec<f32>>,
    /// index with 地方：同一地方共享一套账本（**挂出来**的价）
    ///
    /// 与 `local_ratios` 的区别是要害：比值只在有成交时才更新，账本只看挂单，
    /// 所以没有成交的地方也有价。决策用的是这一套。
    pub books: Vec<Vec<Book>>,
    /// 全局学习曲线（见 [`Globals`]）
    pub globals: Globals,
    /// 一轮里**没有**学习信号的曲线向全局曲线滑动多少（`0` = 关掉）
    pub global_gain: f32,
}

/// 一个地方一种商品的账本：两侧的**边际**挂价
///
/// - `bid` 是别人愿意付的最高价 ⇒ 你卖货能立刻拿到的价
/// - `ask` 是别人愿意收的最低价 ⇒ 你买货要立刻付出的价
///
/// 决策按「产出估 bid、投入估 ask」定价，也就是"要跨过价差才算赚"——
/// 这既堵住了"用一个中间价两头占便宜"，也让过剩的原料**合理地**让下游变便宜。
#[derive(Clone, Copy, Debug, Default)]
pub struct Book {
    pub bid: f32,
    pub ask: f32,
    /// **两侧是否都来自本轮的真实报价。**
    ///
    /// `update_books` 会给缺失的一侧填上 `carried`（= 上次成交比 × 指数，或上一轮的
    /// 中间价，或指数本身）。那是**合成值**，不是市场报出来的。而 `is_formed()` 只看
    /// 「两侧都 > 0」，于是合成值照样让它"成形"——账本因此可以完全是自己的回音，
    /// 再经 `aggregate_index` 写回指数。这个字段把"真实观测"与"合成"分开，
    /// 让指数**只由真正的双边市场导出**（见 [`Warehouses::aggregate_index`]）。
    pub observed: bool,
}

impl Book {
    /// 账本的中间价 = 两侧挂价的**几何平均**。
    ///
    /// 这里曾经是算术平均 `(bid + ask)/2`，那是错的量纲选择：价格在这个系统里是
    /// **乘法量**（`挂价 = 参照价 × 尺度`），而两侧的尺度由各自的学习器独立决定、
    /// 且**系统性不对称**（实测买尺度 3~47、卖尺度 0.1~0.3）。算术平均于是把
    /// `mid/参照价` 变成 `(买尺度 + 卖尺度)/2 ≈ 买尺度/2`，实测在成交停止、
    /// 买盘回退之后把 `mid/指数` 从 0.5 抬到 **3.675**，而**这个被抬高的 mid 又被
    /// 写回指数** ⇒ 指数按 3.675 倍/轮跑飞。
    ///
    /// 几何平均是对数量纲下正确的"中心"：两侧尺度互为倒数时 `mid = 参照价` **精确成立**，
    /// 也就是参照价成了这个聚合环的一个不动点——这正是它一直缺的性质。
    /// 引擎本来就用 [`crate::utils::geometric_average`] 定成交价，这里与它一致。
    pub fn mid(&self) -> f32 {
        crate::utils::geometric_average(self.bid, self.ask)
    }

    /// 两侧都有效才算一本成形的账
    pub fn is_formed(&self) -> bool {
        self.bid.is_finite() && self.ask.is_finite() && self.bid > 0.0 && self.ask > 0.0
    }

    /// **两侧都是真实报价**（而不是被 `carried` 合成出来的）——指数只能由这种账本导出。
    pub fn is_observed(&self) -> bool {
        self.observed && self.is_formed()
    }
}

/// index with [`crate::market::Trader`]
pub struct Warehouse {
    pub stocks: Vec<Stock>,
    pub currency: f32,
    pub locality: usize,
}

/// index with [`crate::market::Merchandise`]
pub struct Stock {
    previous_volume: f32,
    pub volume: f32,
    /// 期望持有的库存
    pub target_volume: f32,
    /// 目标水位的**下限**，等于构造时的初始目标。见 [`Stock::TARGET_COVER`]。
    pub target_floor: f32,
    /// 本轮挂出去的**绝对报价**（货币/件）。不是"参照价的倍数"——学习曲线与账本都
    /// 以绝对数为准（见 `sale_price` / `purchase_price` / `observe`）。
    marketing_price: f32,
    marketing_volume: f32,
    /// 上一轮自己的**成交价**（绝对价）：报价搜索带宽的中心（见 `quote_band`）。
    /// 没有成交过时留在计价物上。
    last_deal: f32,
    /// 报价搜索的**对数半宽**：搜索区间 = `上月成交价 × e^{±quote_band}`。
    /// 默认 [`Warehouses::DEFAULT_QUOTE_BAND`]（见 docs/market-system.md §7.3）。
    quote_band: f32,
    natural_volume_delta: f32,
    /// 本轮**被部门取走**的量（[`Stock::record_take`]），目标水位就锚在它上面。
    ///
    /// 锚在它上面，**不是**锚在存量的净变化上。净变化 = 产量 − 取货量，只要产量
    /// 赶上取货量就被抵消成 0，于是"产多于吃"的时候目标塌到下限、
    /// `TARGET_COVER × 取货量` 这一支完全失效：实测第 2000 轮九个部门三样商品
    /// **全部停在下限上**，goods 级目标 = 3×0 + 6×2.0 = 12.000，逐位对上，
    /// 而当时的取货量是 4.064/部门/轮（三倍应当是 12.19）、库存是 6636。
    ///
    /// 只算部门取货，**不算成交卖出**：卖出去的量是市场撮合的结果，把它算进来会
    /// 让"卖得多 ⇒ 目标高 ⇒ 想囤更多"变成一条新的正反馈，而这不是这里要问的问题
    /// （实测：算进来的话 `stock_moves_toward_the_target_without_overshoot` 里
    /// 目标会跟着销量往上爬，库存再也回不到目标）。
    taken: f32,
    /// 申报前的原始缺口（正 = 想卖，负 = 想买）。**只用于仪表**，不参与任何决策。
    pub declared_gap: f32,
    /// 想买但买不起：`purchase_scale` 返回 None 时申报被清零。**只用于仪表**。
    pub purchase_blocked: bool,
    buy_response: Response,
    sell_response: Response,
    buy_price_curve: PowerLaw,
    sell_price_curve: PowerLaw,
    /// 本轮这一侧**有没有吃到学习信号**（`observe` 里真的更新了它）。index with [`Side`]。
    ///
    /// 价格曲线与兑现率曲面分开记：同一次申报会让**响应**更新，但成交价缺席时
    /// **价格曲线**并没有拿到东西（`observe` 只在 `deal_price > 0` 时更新它）。
    learned_price: [bool; 2],
    learned_response: [bool; 2],
    /// 「这一侧这条曲线有多少自己的信息」的 EWMA（`0` = 从没学到过，`1` = 每轮都在学）。
    /// index with [`Side`]。滑动的**权重**就是 `1 - evidence`：一条已经学了很久的曲线
    /// 不该因为**一轮**没信号就被拉回全局（实测那会把所有曲线的**水平**耦合到一起，
    /// 而那正是 §20.7 还缺锚的那个自由度，见 `docs/local-price.md` §22.3）。
    price_evidence: [f32; 2],
    response_evidence: [f32; 2],
}

/// 一条学习曲线的方向：买 / 卖。用的是 `0/1`，可以直接索引逐侧的数组。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Buy = 0,
    Sell = 1,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Buy, Side::Sell];
}

/// **全局学习曲线**：全经济共享的一份「这样货在这个方向怎么定价 / 能成交多少」的模型，
/// 每个商品、每个方向各一条。
///
/// ## 它不是另一个估计器
///
/// 第一版把它做成"独立的一条曲线，把全经济的观测都喂给它"。**那是错的**：全经济的样本
/// 比任何个体都异质（不同地方、不同报价水平混在一个对数-对数回归里），遗忘最小二乘在
/// 那里会**发散**——实测一条 `buy@1` 在 20 轮内从 0.85 跳到 5.32、斜率反号，然后被
/// 当作滑动目标把所有人一起带跑（§22.2）。
///
/// 现在的定义是**有信息的人当前的共识**：每一轮取"拿到该侧学习信号的个体曲线"参数的
/// **中位数**（抗离群：实测个体曲线自己也会发散到 `buy@1 = 133`，均值会被它毒掉）。
/// 没有任何人拿到信号时，保留上一轮的共识。
///
/// 一轮里**没有**学习信号的个体曲线就向它滑动 [`Warehouses::global_gain`]：
/// **没有信息，退回大家的共识**，而不是退回一条从没更新过的先验。
///
/// 动机（`docs/local-price.md` §21）：一个部门对「自己从不经手的那样货」读到的正是
/// 那条没更新过的先验——不买 ⇒ 不知道价 ⇒ 更不该买，鸡生蛋。
pub struct Globals {
    /// index with `good`，再 index with [`Side`]：参数快照（`[斜率, 截距]`）
    price: Vec<[Option<[f32; 2]>; 2]>,
    /// index with `good`，再 index with [`Side`]：参数快照（6 个，见 [`Response::params`]）
    response: Vec<[Option<[f32; 6]>; 2]>,
}

/// 一组数的中位数（会改动输入的顺序）。空集返回 `None`。
fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[values.len() / 2])
}

impl Globals {
    pub fn new(goods: usize) -> Self {
        Self {
            price: vec![[None; 2]; goods],
            response: vec![[None; 2]; goods],
        }
    }

    pub fn len(&self) -> usize {
        self.price.len()
    }

    pub fn is_empty(&self) -> bool {
        self.price.is_empty()
    }

    /// 某一轮这一格的共识参数；没人拿到信号时是上一轮的共识（起始为 `None`）。
    pub fn price(&self, good: usize, side: Side) -> Option<[f32; 2]> {
        self.price.get(good)?.get(side as usize)?.to_owned()
    }

    pub fn response(&self, good: usize, side: Side) -> Option<[f32; 6]> {
        self.response.get(good)?.get(side as usize)?.to_owned()
    }

    /// 重算这一轮的共识：逐（商品、方向）取"拿到该侧信号的个体"参数的中位数。
    /// 没人拿到信号的那一格保持原值（没有新信息，不改共识）。
    pub fn refresh(&mut self, warehouses: &[Warehouse]) {
        let goods = self.price.len();
        for good in 0..goods {
            for side in Side::BOTH {
                let index = side as usize;
                let mut slopes = Vec::new();
                let mut intercepts = Vec::new();
                let mut params: [Vec<f32>; 6] = std::array::from_fn(|_| Vec::new());
                for warehouse in warehouses {
                    let Some(stock) = warehouse.stocks.get(good) else {
                        continue;
                    };
                    if stock.price_learned(side) {
                        let snapshot = stock.price_curve(side).params();
                        slopes.push(snapshot[0]);
                        intercepts.push(snapshot[1]);
                    }
                    if stock.response_learned(side) {
                        let snapshot = stock.response(side).params();
                        for (slot, value) in params.iter_mut().zip(snapshot) {
                            slot.push(value);
                        }
                    }
                }
                if let (Some(slope), Some(intercept)) =
                    (median(&mut slopes), median(&mut intercepts))
                {
                    self.price[good][index] = Some([slope, intercept]);
                }
                if params.iter().all(|values| !values.is_empty()) {
                    let mut snapshot = [0.0f32; 6];
                    for (slot, values) in snapshot.iter_mut().zip(params.iter_mut()) {
                        *slot = median(values).unwrap_or(0.0);
                    }
                    self.response[good][index] = Some(snapshot);
                }
            }
        }
    }
}

impl Warehouses {
    pub const DEFAULT_FLUCTUATION: f32 = 0.2;
    /// 账本/本地比值混合的默认记忆（旧 `warehouse::step::LOCAL_PRICE_FORGETTING`）
    pub const DEFAULT_BOOK_FORGETTING: f32 = 0.8;
    /// 报价搜索的默认对数半宽（中心 = 上一轮自己的成交价）。
    ///
    /// §7.3 实测定档（modern/capacity 12）：半宽 1.0 在 20000 轮 × 5 个种子上把 index
    /// 收到 10^-1.3…10^2.5、成交 74–96 件/轮（±44 时是 89），本地价的 log 跨度
    /// （seed 11、5000 轮均值）从 **48** 收到 **1.45**；更紧的 0.5 会把市场冻死
    /// （成交 → 0.66 件/轮）；3.0 则有一个种子长时程漂到 10^10.7。
    pub const DEFAULT_QUOTE_BAND: f32 = 1.0;

    /// 没有学习信号的曲线每轮向全局曲线滑动的默认增益。
    ///
    /// 实测（`docs/local-price.md` §22.3）：字面的"没信号就滑到底"会把全经济的**水平**
    /// 耦合起来、把 index 推到 `10^11`；加了 `1 - evidence` 的权重之后要**小**才不再明显
    /// 放大漂移。`0.02` 是量出来的折中：阶梯的换挡点落回理论值 1.104 附近（0 是 2.7、
    /// 0.05 是 0.8），4 个种子 × 20000 轮的极差与关掉时同量级（1 个种子仍更差）。
    /// **这不是水平锚**：§20.7 那条锚还欠着，调大这个增益会直接把它暴露出来。
    pub const DEFAULT_GLOBAL_GAIN: f32 = 0.02;

    pub fn new(warehouses: Vec<Warehouse>) -> Self {
        let goods = warehouses
            .first()
            .map(|warehouse| warehouse.stocks.len())
            .unwrap_or(0);
        Self {
            warehouses,
            fluctuation: Self::DEFAULT_FLUCTUATION,
            response_forgetting: Response::DEFAULT_FORGETTING,
            price_forgetting: PowerLaw::DEFAULT_FORGETTING,
            book_forgetting: Self::DEFAULT_BOOK_FORGETTING,
            local_ratios: Vec::new(),
            books: Vec::new(),
            globals: Globals::new(goods),
            global_gain: Self::DEFAULT_GLOBAL_GAIN,
        }
    }

    /// 没有学习信号的曲线每轮向全局曲线滑动多少（`0` = 关掉）
    pub fn with_global_gain(mut self, gain: f32) -> Self {
        self.global_gain = if gain.is_finite() { gain.max(0.0) } else { 0.0 };
        self
    }

    /// 一个地方的账本；这一轮还没成形就返回 None（调用方回退到指数）
    pub fn book(&self, locality: usize, good: usize) -> Option<Book> {
        let book = self.books.get(locality)?.get(good).copied()?;
        if book.is_formed() {
            Some(book)
        } else {
            None
        }
    }

    /// 银河价 = **各地方本地价的成交量加权几何平均**（纯读数，display only）。
    ///
    /// 本地价 = 该地方账本的中间价（`Book::mid`，买卖双方**绝对报价**的几何平均）。
    /// 报价现在是绝对价（`Stock::marketing_price`），不再有"参照价 × 尺度"那一层，
    /// 所以账本中间价就是"大家在这地方实际挂出来的价格水平"。
    ///
    /// 某一轮一个地方都没成形时返回 0，调用方应当保留旧指数。
    pub fn aggregate_index(&self, goods: usize) -> Vec<f32> {
        let mut index = vec![0.0f32; goods];
        for (k, slot) in index.iter_mut().enumerate() {
            let mut weight = 0.0f32;
            let mut weighted_log = 0.0f32;
            let mut flat_log = 0.0f32;
            let mut formed = 0.0f32;
            for (locality, row) in self.books.iter().enumerate() {
                let Some(book) = row.get(k) else {
                    continue;
                };
                if !book.is_formed() {
                    continue;
                }
                let mid = book.mid();
                if !(mid > 0.0) || !mid.is_finite() {
                    continue;
                }
                let log_mid = mid.ln();
                flat_log += log_mid;
                formed += 1.0;
                let volume: f32 = self
                    .warehouses
                    .iter()
                    .filter(|warehouse| warehouse.locality == locality)
                    .map(|warehouse| {
                        warehouse
                            .stocks
                            .get(k)
                            .map(|stock| stock.marketing_volume().abs())
                            .unwrap_or(0.0)
                    })
                    .sum();
                if volume > 0.0 && volume.is_finite() {
                    weighted_log += volume * log_mid;
                    weight += volume;
                }
            }
            *slot = if weight > 0.0 {
                (weighted_log / weight).exp()
            } else if formed > 0.0 {
                (flat_log / formed).exp()
            } else {
                0.0
            };
            if !slot.is_finite() {
                *slot = 0.0;
            }
        }
        index
    }

    /// 某个地方某种商品的成交价相对指数的比值，没有成交过就返回 None
    pub fn local_ratio(&self, locality: usize, good: usize) -> Option<f32> {
        let ratio = self
            .local_ratios
            .get(locality)
            .and_then(|ratios| ratios.get(good))
            .copied()?;
        if ratio > 0.0 && ratio.is_finite() {
            Some(ratio)
        } else {
            None
        }
    }

    pub fn with_fluctuation(mut self, fluctuation: f32) -> Self {
        self.fluctuation = fluctuation.max(0.0);
        self
    }

    /// 学习率与阶数：forgetting 越小追得越快，fixed_slope 为真则只学水平
    pub fn with_learning(self, forgetting: f32, fixed_slope: bool) -> Self {
        self.with_learning_rates(forgetting, forgetting, fixed_slope)
    }

    /// 响应曲面与价格曲线**分开**设。
    ///
    /// §19.5 把"仓库内部那三个写死的学习率"列为唯一没扫过的旋钮。实测（多种子、
    /// 5000 轮）扫出来的是**价格曲线**：冻结它整场就稳，只冻结 `Response` 照样炸。
    pub fn with_learning_rates(mut self, response: f32, price: f32, fixed_slope: bool) -> Self {
        self.response_forgetting = response.clamp(f32::MIN_POSITIVE, 1.0);
        self.price_forgetting = price.clamp(f32::MIN_POSITIVE, 1.0);
        for warehouse in self.warehouses.iter_mut() {
            for stock in warehouse.stocks.iter_mut() {
                stock.reset_estimators(self.response_forgetting, self.price_forgetting, fixed_slope);
            }
        }
        // 全局共识跟着重开：它是从个体曲线里算出来的，个体重开它就得重开。
        self.globals = Globals::new(self.globals.len());
        self
    }

    /// 账本与本地比值的混合记忆：`0.8` = 旧默认，`1.0` = 只认本轮真实报价（不混）
    pub fn with_book_forgetting(mut self, forgetting: f32) -> Self {
        self.book_forgetting = forgetting.clamp(0.0, 1.0);
        self
    }

    /// 报价搜索的对数半宽（中心 = 上一轮自己的成交价）。默认 = [`Warehouses::DEFAULT_QUOTE_BAND`]。
    pub fn with_quote_band(mut self, band: f32) -> Self {
        let band = if band.is_finite() {
            band.clamp(0.0, crate::utils::LOG_LIMIT)
        } else {
            crate::utils::LOG_LIMIT
        };
        for warehouse in self.warehouses.iter_mut() {
            for stock in warehouse.stocks.iter_mut() {
                stock.quote_band = band;
            }
        }
        self
    }

    pub fn step(&mut self, market: &mut Market, rng: &mut Rng) {
        for warehouse in self.warehouses.iter_mut() {
            for stock in warehouse.stocks.iter_mut() {
                stock.clear_learning_signals();
            }
        }
        step::step(self, market, rng);
        // 结算与观测都做完之后：这一轮**没吃到信号**的曲线向全局曲线滑动
        self.slide_silent_curves_toward_global();
    }

    /// 先把这一轮的**全局共识**重算出来（[`Globals::refresh`]），再让一轮里
    /// **没有**学习信号的曲线向它滑动 [`Self::global_gain`]。价格曲线与兑现率曲面
    /// 分别判各自的信号。
    ///
    /// **滑动权重是 `global_gain × (1 - evidence)`**：`evidence` 是"这条曲线有多少自己的
    /// 信息"的 EWMA（见 [`Stock::advance_evidence`]）。字面地"只要这一轮没信号就滑到底"
    /// 会把全经济的曲线**水平**耦合在一起，而水平正是 §20.7 还缺锚的那个自由度——
    /// 实测那样跑 20000 轮会把 index 推到 `10^11`（不耦合时是 `10^3`，见 §22.3）。
    /// 加权的版本只拖动**没有自己信息**（或已经陈旧）的曲线，正是"不知道就问大家"。
    ///
    /// `global_gain = 0` 时共识照样刷新（只作读数），但谁都不动。
    pub fn slide_silent_curves_toward_global(&mut self) {
        let Self {
            warehouses,
            globals,
            global_gain,
            response_forgetting,
            price_forgetting,
            ..
        } = self;
        globals.refresh(warehouses);
        let gain = *global_gain;
        if !gain.is_finite() || gain <= 0.0 {
            return;
        }
        let response_forgetting = *response_forgetting;
        let price_forgetting = *price_forgetting;
        for warehouse in warehouses.iter_mut() {
            for (good, stock) in warehouse.stocks.iter_mut().enumerate() {
                for side in Side::BOTH {
                    let price_signalled = stock.price_learned(side);
                    let evidence =
                        stock.advance_evidence(side, true, price_forgetting, price_signalled);
                    if let Some(target) = globals.price(good, side) {
                        stock
                            .price_curve_mut(side)
                            .slide_toward(target, gain * (1.0 - evidence));
                    }
                    let response_signalled = stock.response_learned(side);
                    let evidence =
                        stock.advance_evidence(side, false, response_forgetting, response_signalled);
                    if let Some(target) = globals.response(good, side) {
                        stock
                            .response_mut(side)
                            .slide_toward(target, gain * (1.0 - evidence));
                    }
                }
            }
        }
    }
}

impl Warehouse {
    pub fn new(stocks: Vec<Stock>) -> Self {
        Self {
            stocks,
            currency: 0.0,
            locality: 0,
        }
    }

    pub fn with_locality(mut self, locality: usize) -> Self {
        self.locality = locality;
        self
    }

    pub fn with_currency(mut self, currency: f32) -> Self {
        self.currency = currency.max(0.0);
        self
    }
}

impl Stock {
    /// 目标水位 = **`TARGET_COVER` × 本轮实际被取走的量**（见 [`Stock::taken`]）。
    ///
    /// 这是把目标**锚在观测到的流量上**，而不是锚在一个固定的乘法倍率上：
    /// 取货量受部门的消费能力约束、本身有界，所以目标不会像纯乘法那样
    /// 一路衰减到下溢或一路爆炸（实测固定 ×1.1/÷1.1 会让一产目标掉到 0.00、
    /// 三产目标涨到 1.13e29）。语义就是常说的"备三倍的货"。
    pub const TARGET_COVER: f32 = 3.0;

    /// 记下本轮从货架上**离开**的量——部门结算后的实际提货量。
    ///
    /// 每次调用**覆盖**（不是累加）：部门每轮对每样商品调一次，重置就自然发生了；
    /// 成交卖出由仓库自己累加在上面（申报发生在本轮成交之前，所以卖出的那部分
    /// 要到下一轮才读得到）。
    pub fn record_take(&mut self, volume: f32) {
        self.taken = if volume.is_finite() {
            volume.max(0.0)
        } else {
            0.0
        };
    }

    pub fn new(volume: f32, target_volume: f32) -> Self {
        Self {
            previous_volume: volume,
            volume,
            target_floor: target_volume,
            target_volume,
            marketing_price: 1.0,
            marketing_volume: 0.0,
            last_deal: 1.0,
            quote_band: Warehouses::DEFAULT_QUOTE_BAND,
            natural_volume_delta: 0.0,
            taken: 0.0,
            declared_gap: 0.0,
            purchase_blocked: false,
            buy_response: Response::new(Response::DEFAULT_FORGETTING),
            sell_response: Response::new(Response::DEFAULT_FORGETTING),
            buy_price_curve: PowerLaw::new(0.5, 0.0, PowerLaw::DEFAULT_FORGETTING),
            sell_price_curve: PowerLaw::new(0.5, 0.0, PowerLaw::DEFAULT_FORGETTING),
            learned_price: [false; 2],
            learned_response: [false; 2],
            price_evidence: [0.0; 2],
            response_evidence: [0.0; 2],
        }
    }

    /// 重设两侧估计器：阶数可学或钉死，遗忘因子即学习率。
    ///
    /// 响应曲面与价格曲线分开传：扫出来只有价格曲线是承重的（见
    /// [`Warehouses::with_learning_rates`]）。
    pub fn reset_estimators(
        &mut self,
        response_forgetting: f32,
        price_forgetting: f32,
        fixed_slope: bool,
    ) {
        self.buy_response = Response::new(response_forgetting);
        self.sell_response = Response::new(response_forgetting);
        let buy = PowerLaw::new(0.5, 0.0, price_forgetting);
        let sell = PowerLaw::new(0.5, 0.0, price_forgetting);
        self.buy_price_curve = if fixed_slope {
            buy.with_fixed_slope()
        } else {
            buy
        };
        self.sell_price_curve = if fixed_slope {
            sell.with_fixed_slope()
        } else {
            sell
        };
        self.clear_learning_signals();
        self.price_evidence = [0.0; 2];
        self.response_evidence = [0.0; 2];
    }

    /// 推进"这一侧这条曲线有多少自己的信息"的 EWMA，返回新值。
    ///
    /// `forgetting` 用该估计器自己的 forgetting：每轮都拿到信号 ⇒ 收敛到 1，
    /// 停下来的曲线会慢慢衰减回 0（于是**陈旧**的曲线也会重新向全局滑动）。
    pub(crate) fn advance_evidence(
        &mut self,
        side: Side,
        price: bool,
        forgetting: f32,
        signalled: bool,
    ) -> f32 {
        let slot = if price {
            &mut self.price_evidence[side as usize]
        } else {
            &mut self.response_evidence[side as usize]
        };
        let forgetting = if forgetting.is_finite() {
            forgetting.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let signal = if signalled { 1.0 } else { 0.0 };
        *slot = forgetting * *slot + (1.0 - forgetting) * signal;
        *slot
    }

    /// 清掉本轮的"吃到信号"标记。**每轮开始前**调一次（见 [`Warehouses::step`]）。
    pub(crate) fn clear_learning_signals(&mut self) {
        self.learned_price = [false; 2];
        self.learned_response = [false; 2];
    }

    pub(crate) fn mark_price_learned(&mut self, side: Side) {
        self.learned_price[side as usize] = true;
    }

    pub(crate) fn mark_response_learned(&mut self, side: Side) {
        self.learned_response[side as usize] = true;
    }

    pub(crate) fn price_learned(&self, side: Side) -> bool {
        self.learned_price[side as usize]
    }

    pub(crate) fn response_learned(&self, side: Side) -> bool {
        self.learned_response[side as usize]
    }

    /// 这一侧的两条曲线（`Side` 索引）
    pub fn price_curve(&self, side: Side) -> &PowerLaw {
        match side {
            Side::Buy => &self.buy_price_curve,
            Side::Sell => &self.sell_price_curve,
        }
    }

    pub(crate) fn price_curve_mut(&mut self, side: Side) -> &mut PowerLaw {
        match side {
            Side::Buy => &mut self.buy_price_curve,
            Side::Sell => &mut self.sell_price_curve,
        }
    }

    /// 这一侧的兑现率曲面（`Side` 索引）
    pub fn response(&self, side: Side) -> &Response {
        match side {
            Side::Buy => &self.buy_response,
            Side::Sell => &self.sell_response,
        }
    }

    pub(crate) fn response_mut(&mut self, side: Side) -> &mut Response {
        match side {
            Side::Buy => &mut self.buy_response,
            Side::Sell => &mut self.sell_response,
        }
    }
    /// **测试用**：直接给出买入价格曲线（`报价 -> 绝对成交价`）。
    ///
    /// 部门估值走的就是这条曲线（§20.13），但生产代码里它只由 `observe` 学习。
    /// 要单独测"估值怎么影响政策份额"，就得能把它钉成一个已知映射。
    #[cfg(test)]
    pub(crate) fn set_buy_price_curve(&mut self, curve: PowerLaw) {
        self.buy_price_curve = curve;
    }

    /// 买方"力度"：报价越高越激进（越容易成交）。
    pub fn buy_aggressiveness(price: f32) -> f32 {
        if price.is_finite() && price > 0.0 {
            price
        } else {
            1.0
        }
    }

    /// 卖方"力度"：报价越高越不激进（越难成交）。
    pub fn sell_aggressiveness(price: f32) -> f32 {
        if price.is_finite() && price > 0.0 {
            1.0 / price
        } else {
            1.0
        }
    }

    /// **测试用**：直接给出上一轮申报名义量（决策就是在这个量上读曲线的）。
    #[cfg(test)]
    pub(crate) fn set_marketing_volume(&mut self, volume: f32) {
        self.marketing_volume = volume;
    }

    pub fn marketing_volume(&self) -> f32 {
        self.marketing_volume
    }

    pub fn marketing_price(&self) -> f32 {
        self.marketing_price
    }

    /// 报价搜索的对数区间 `[center − band, center + band]`，中心是上一轮自己的成交价。
    ///
    /// §7.3：把"单个交易者能在十几个数量级里挑报价"收成"上一个成交价附近的一个区间"。
    /// `quote_band = LOG_LIMIT` 时区间就是全 f32 范围（历史行为，逐位不变）；
    /// 默认见 [`Warehouses::DEFAULT_QUOTE_BAND`]。
    pub(crate) fn quote_log_bounds(&self) -> (f32, f32) {
        let limit = crate::utils::LOG_LIMIT;
        let center = if self.last_deal.is_finite() && self.last_deal > 0.0 {
            self.last_deal.ln().clamp(-limit, limit)
        } else {
            0.0
        };
        let band = if self.quote_band.is_finite() {
            self.quote_band.clamp(0.0, limit)
        } else {
            limit
        };
        // 半宽取满 = "没有区间"：直接给全 f32 范围，**不受锚平移的影响**。
        // 这条短路保证默认档与"不带宽带"的历史行为逐位相同。
        if band >= limit {
            return (-limit, limit);
        }
        ((center - band).max(-limit), (center + band).min(limit))
    }

    pub fn buy_response(&self) -> &Response {
        &self.buy_response
    }

    pub fn sell_response(&self) -> &Response {
        &self.sell_response
    }

    pub fn buy_price_curve(&self) -> &PowerLaw {
        &self.buy_price_curve
    }

    pub fn sell_price_curve(&self) -> &PowerLaw {
        &self.sell_price_curve
    }
}
