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
}

impl Book {
    pub fn mid(&self) -> f32 {
        0.5 * (self.bid + self.ask)
    }

    /// 两侧都有效才算一本成形的账
    pub fn is_formed(&self) -> bool {
        self.bid.is_finite() && self.ask.is_finite() && self.bid > 0.0 && self.ask > 0.0
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
    marketing_price_scale: f32,
    marketing_volume: f32,
    natural_volume_delta: f32,
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
                if !book.is_formed() {
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
    /// 目标的**自适应倍率**：上一轮货架被取空 ⇒ 目标 ×k（得多买）；
    /// 还有货没被取走 ⇒ 目标 ÷k（压多了）。
    ///
    /// k 要小：这是个乘法控制器，k 越大振荡越硬。取 1.1 = 每轮 ±10%。
    pub const TARGET_GROWTH: f32 = 1.1;

    /// 目标水位的初始值。**所有仓库都用同一个数**——目标从此由仓库自己
    /// 按"货架有没有被取空"调，而不是由部门按当轮计划写死（那会让计划为 0
    /// 的商品连目标也变成 0，于是永远没人买它）。
    pub const INITIAL_TARGET: f32 = 100.0;

    pub fn new(volume: f32, target_volume: f32) -> Self {
        Self {
            previous_volume: volume,
            volume,
            target_volume,
            marketing_price_scale: 1.0,
            marketing_volume: 0.0,
            natural_volume_delta: 0.0,
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
