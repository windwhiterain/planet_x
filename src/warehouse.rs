mod step;

#[cfg(test)]
mod tests;

use crate::{estimator::PowerLaw, market::Market};
use fastrand::Rng;

pub struct Warehouses {
    pub warehouses: Vec<Warehouse>,
    pub fluctuation: f32,
}

/// index with [`crate::market::Trader`]
pub struct Warehouse {
    pub stocks: Vec<Stock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SellerRule {
    TargetVolume,
    #[default]
    RevenueMax,
}

/// index with [`crate::market::Merchandise`]
pub struct Stock {
    previous_volume: f32,
    pub volume: f32,
    pub target_volume: f32,
    marketing_price_scale: f32,
    marketing_volume: f32,
    natural_volume_delta: f32,
    buy_volume2price_scale: PowerLaw,
    sell_volume2price_scale: PowerLaw,
    seller_rule: SellerRule,
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

    pub fn step(&mut self, market: &mut Market, rng: &mut Rng) {
        step::step(self, market, rng);
    }
}

impl Warehouse {
    pub fn new(stocks: Vec<Stock>) -> Self {
        Self { stocks }
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
            buy_volume2price_scale: PowerLaw::new(1.0, 0.0, PowerLaw::DEFAULT_FORGETTING),
            sell_volume2price_scale: PowerLaw::new(-1.0, 0.0, PowerLaw::DEFAULT_FORGETTING),
            seller_rule: SellerRule::default(),
        }
    }

    pub fn with_seller_rule(mut self, seller_rule: SellerRule) -> Self {
        self.seller_rule = seller_rule;
        self
    }
}
