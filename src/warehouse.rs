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
}

/// index with [`crate::market::Trader`]
pub struct Warehouse {
    pub stocks: Vec<Stock>,
    pub currency: f32,
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
        }
    }

    pub fn with_currency(mut self, currency: f32) -> Self {
        self.currency = currency.max(0.0);
        self
    }
}

impl Stock {
    pub fn new(volume: f32, target_volume: f32) -> Self {
        Self {
            previous_volume: volume,
            volume,
            target_volume,
            marketing_price_scale: 1.0,
            marketing_volume: 0.0,
            natural_volume_delta: 0.0,
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
