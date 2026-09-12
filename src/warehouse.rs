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
    pub fn new(traders: Vec<Trader>) -> Self {
        Self { traders }
    }

    pub fn step(&mut self, market: &mut Market) {
        step::step(self, market);
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
            buy_volume_scale2price_scale: Scale::new(1.0),
            sell_volume_scale_reciprocal2price_scale: Scale::new(1.0),
        }
    }
}
