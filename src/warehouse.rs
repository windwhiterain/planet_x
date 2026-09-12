mod step;

#[cfg(test)]
mod tests;

use crate::{estimator::Scale, market::Market};

pub struct Warehouse {
    pub traders: Vec<Trader>,
}

pub struct Trader {
    pub merchandises: Vec<Merchandise>,
}

pub struct Merchandise {
    previous_volume: f32,
    pub volume: f32,
    pub target_volume: f32,
    marketing_price_scale: f32,
    marketing_volume: f32,
    natural_volume_delta: f32,
    buy_volume_scale2price_scale: Scale,
    sell_volume_scale_reciprocal2price_scale: Scale,
}

impl Warehouse {
    pub fn step(&mut self, market: &mut Market) {
        step::step(self, market);
    }
}
