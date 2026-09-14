mod step;

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
    /// index with 地方：同一地方共享一套「本地成交价 ÷ 指数」的比值（**已实现**的价）
    pub local_ratios: Vec<Vec<f32>>,
    /// index with 地方：同一地方共享一套账本（**挂出来**的价）
    ///
    /// 与 `local_ratios` 的区别是要害：比值只在有成交时才更新，账本只看挂单，
    /// 所以没有成交的地方也有价。决策用的是这一套。
    pub books: Vec<Vec<Book>>,
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
    pub reference: Vec<f32>,
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
    marketing_price_scale: f32,
    marketing_volume: f32,
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
}

impl Warehouses {
    pub const DEFAULT_FLUCTUATION: f32 = 0.2;

    pub fn new(warehouses: Vec<Warehouse>) -> Self {
        Self {
            warehouses,
            fluctuation: Self::DEFAULT_FLUCTUATION,
            local_ratios: Vec::new(),
            books: Vec::new(),
        }
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

    /// 银河指数 = **各地方账本中间价的申报量加权聚合**。
    ///
    /// 这是依赖倒置的另一半：账本先验、指数导出。旧口径的指数是"成交的加权平均"，
    /// 没有成交就一个字都不动——三产的价格因此冻成第 1 轮的化石。
    ///
    /// 代价要说清：指数一旦由账本导出，"指数 → 参照价 → 挂价 → 账本 → 指数"就是一个
    /// 没有回复力的乘法环（实测把挂价推到过 f32 边界）。把它钉住的是**水平锚**——
    /// 所以这套东西在 `--no-anchor` 下是不安全的，锚从"规范选择"变成了承重件。
    ///
    /// 某一轮一本账都没成形时返回 0，调用方应当保留旧指数。
    pub fn aggregate_index(&self, goods: usize) -> Vec<f32> {
        let mut index = vec![0.0f32; goods];
        for (k, slot) in index.iter_mut().enumerate() {
            let mut weight = 0.0f32;
            let mut weighted = 0.0f32;
            let mut flat = 0.0f32;
            let mut formed = 0.0f32;
            for (locality, row) in self.books.iter().enumerate() {
                let Some(book) = row.get(k) else {
                    continue;
                };
                // ⚠️ 这里原来判的是 `is_formed()`，但**合成出来的账本也会成形**：
                // 缺失的一侧被 `carried` 填上（正数！），于是账本成了自己的回音，
                // 经本函数写回指数。改成只认**真实双边报价**，指数才是市场观测。
                if !book.is_observed() {
                    continue;
                }
                let mid = book.mid();
                flat += mid;
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
                    weighted += volume * mid;
                    weight += volume;
                }
            }
            *slot = if weight > 0.0 {
                weighted / weight
            } else if formed > 0.0 {
                flat / formed
            } else {
                0.0
            };
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
    pub fn with_learning(mut self, forgetting: f32, fixed_slope: bool) -> Self {
        for warehouse in self.warehouses.iter_mut() {
            for stock in warehouse.stocks.iter_mut() {
                stock.reset_estimators(forgetting, fixed_slope);
            }
        }
        self
    }

    pub fn step(&mut self, market: &mut Market, rng: &mut Rng) {
        step::step(self, market, rng);
    }
}

impl Warehouse {
    pub fn new(stocks: Vec<Stock>) -> Self {
        Self {
            stocks,
            currency: 0.0,
            reference: Vec::new(),
            locality: 0,
        }
    }

    pub fn with_locality(mut self, locality: usize) -> Self {
        self.locality = locality;
        self
    }

    pub fn with_reference(mut self, reference: Vec<f32>) -> Self {
        self.reference = reference;
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
            marketing_price_scale: 1.0,
            marketing_volume: 0.0,
            natural_volume_delta: 0.0,
            taken: 0.0,
            declared_gap: 0.0,
            purchase_blocked: false,
            buy_response: Response::new(Response::DEFAULT_FORGETTING),
            sell_response: Response::new(Response::DEFAULT_FORGETTING),
            buy_price_curve: PowerLaw::new(0.5, 0.0, PowerLaw::DEFAULT_FORGETTING),
            sell_price_curve: PowerLaw::new(0.5, 0.0, PowerLaw::DEFAULT_FORGETTING),
        }
    }

    /// 重设两侧估计器：阶数可学或钉死，遗忘因子即学习率
    pub fn reset_estimators(&mut self, forgetting: f32, fixed_slope: bool) {
        self.buy_response = Response::new(forgetting);
        self.sell_response = Response::new(forgetting);
        let buy = PowerLaw::new(0.5, 0.0, forgetting);
        let sell = PowerLaw::new(0.5, 0.0, forgetting);
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
    }

    pub fn buy_aggressiveness(price_scale: f32) -> f32 {
        if price_scale.is_finite() && price_scale > 0.0 {
            price_scale
        } else {
            1.0
        }
    }

    pub fn sell_aggressiveness(price_scale: f32) -> f32 {
        if price_scale.is_finite() && price_scale > 0.0 {
            1.0 / price_scale
        } else {
            1.0
        }
    }

    pub fn marketing_volume(&self) -> f32 {
        self.marketing_volume
    }

    pub fn marketing_price_scale(&self) -> f32 {
        self.marketing_price_scale
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
