mod step;

#[cfg(test)]
mod tests;

use crate::{estimator::PowerLaw, market::Market};
use fastrand::Rng;

pub struct Warehouse {
    pub traders: Vec<Trader>,
    pub fluctuation: f32,
}

pub struct Trader {
    pub merchandises: Vec<Merchandise>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SellerRule {
    TargetVolume,
    #[default]
    RevenueMax,
}

pub struct Merchandise {
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

impl Warehouse {
    pub const DEFAULT_FLUCTUATION: f32 = 0.2;

    pub fn new(traders: Vec<Trader>) -> Self {
        Self {
            traders,
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

impl Trader {
    pub fn new(merchandises: Vec<Merchandise>) -> Self {
        Self { merchandises }
    }
}

impl Merchandise {
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
