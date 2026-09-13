use fastrand::Rng;

use crate::estimator::Estimator;
use crate::estimator2d::{Estimator2D, Response};
use crate::market::Market;
use crate::utils::normal_cdf;
use crate::warehouse::{Stock, Warehouse, Warehouses};

const PRICE_SCALE_FLOOR: f32 = 0.25;
const PRICE_SCALE_CEILING: f32 = 4.0;
const PRICE_SCALE_STEPS: usize = 49;
const SATURATION_MARGIN: f32 = 2.0;
const PROBABILITY_TOLERANCE: f32 = 1e-4;

fn fluctuation_factor(amplitude: f32, rng: &mut fastrand::Rng) -> f32 {
    let unit = rng.f32().clamp(1e-6, 1.0 - 1e-6);
    if !(amplitude > 0.0) {
        return 1.0;
    }
    (unit / (1.0 - unit)).powf(amplitude)
}

pub(super) fn price_scales() -> [f32; PRICE_SCALE_STEPS] {
    let mut scales = [1.0; PRICE_SCALE_STEPS];
    let floor = PRICE_SCALE_FLOOR.ln();
    let span = PRICE_SCALE_CEILING.ln() - floor;
    for (step, scale) in scales.iter_mut().enumerate() {
        let fraction = step as f32 / (PRICE_SCALE_STEPS - 1) as f32;
        *scale = (floor + span * fraction).exp();
    }
    scales
}

fn declared_volumes(stock: &mut Stock, fluctuation: f32, rng: &mut Rng) {
    stock.natural_volume_delta = (stock.volume - stock.previous_volume).min(0.0);
    let gap = stock.volume - stock.target_volume + stock.natural_volume_delta;
    let magnitude = (gap.abs() * fluctuation_factor(fluctuation, rng)).clamp(0.0, gap.abs());
    stock.marketing_volume = magnitude * gap.signum();
}

fn sale_scale(stock: &Stock, available: f32, price: f32, scales: &[f32]) -> f32 {
    if !(available > 0.0) || !(price > 0.0) {
        return 1.0;
    }
    let response = stock.sell_response();
    let curve = stock.sell_price_curve();
    let mut best_scale = 1.0;
    let mut best_revenue = -1.0;
    for &scale in scales {
        let aggressiveness = Stock::sell_aggressiveness(scale);
        let dealt = response.get(available, aggressiveness);
        let revenue = dealt * price * curve.get(scale);
        if revenue.is_finite() && revenue > best_revenue {
            best_revenue = revenue;
            best_scale = scale;
        }
    }
    best_scale
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

fn purchase_scale(stock: &Stock, need: f32, price: f32, cash: f32, scales: &[f32]) -> Option<(f32, f32)> {
    if !(need > 0.0) || !(price > 0.0) || !(cash > 0.0) {
        return None;
    }
    let response = stock.buy_response();
    let noise = response.noise().max(Response::MIN_NOISE);
    let target = need * (SATURATION_MARGIN * noise).exp();
    let mut best: Option<(f32, f32)> = None;
    let mut best_probability = -1.0;
    let mut best_cost = f32::INFINITY;
    for &scale in scales {
        let affordable = affordable_volume(price, scale, cash);
        if !(affordable > 0.0) {
            continue;
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
            continue;
        }
        let dealt = response.get(volume, aggressiveness);
        let margin = (dealt / need).max(1e-6).ln() / noise;
        let probability = normal_cdf(margin);
        if !probability.is_finite() {
            continue;
        }
        let cost = volume * price * scale;
        let better = probability > best_probability + PROBABILITY_TOLERANCE
            || ((probability - best_probability).abs() <= PROBABILITY_TOLERANCE
                && cost < best_cost);
        if better {
            best_probability = probability;
            best_cost = cost;
            best = Some((volume, scale));
        }
    }
    best
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

pub(super) fn step(warehouses: &mut Warehouses, market: &mut Market, rng: &mut Rng) {
    let Warehouses {
        warehouses,
        fluctuation,
        ..
    } = warehouses;
    let fluctuation = *fluctuation;
    let reference_prices: Vec<f32> = market
        .merchandises
        .iter()
        .map(|merchandise| merchandise.price)
        .collect();
    let scales = price_scales();
    for (i, warehouse) in warehouses.iter_mut().enumerate() {
        let Warehouse { stocks, currency } = warehouse;
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
            let price = reference_prices[k].max(0.0);
            let scale = if stock.marketing_volume > 0.0 {
                sale_scale(stock, stock.marketing_volume, price, &scales)
            } else if stock.marketing_volume < 0.0 {
                match purchase_scale(stock, stock.marketing_volume.abs(), price, budget, &scales) {
                    Some((volume, scale)) => {
                        stock.marketing_volume = -volume;
                        scale
                    }
                    None => {
                        stock.marketing_volume = 0.0;
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
            merchandise.price = price_of(reference_prices[k], stock.marketing_price_scale);
            merchandise.volume = stock.marketing_volume;
        }
    }
    market.step();
    for (i, warehouse) in warehouses.iter_mut().enumerate() {
        for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
            let merchandise = &market.traders[i].merchandises[k];
            stock.volume -= merchandise.deal_volume();
            observe(stock, merchandise, reference_prices[k]);
            stock.previous_volume = stock.volume;
        }
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
