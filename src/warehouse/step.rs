use fastrand::Rng;

use crate::estimator::{Estimator, PowerLaw};
use crate::market::Market;
use crate::warehouse::{SellerRule, Warehouses};

fn revenue_max_volumes(price_scale: &PowerLaw, surplus: f32) -> f32 {
    let slope = price_scale.slope();
    if slope > -1.0 {
        return surplus;
    }
    let corner = {
        let log_volume = (PowerLaw::MAX_LOG_SCALE - price_scale.intercept()) / slope;
        if log_volume.is_finite() && log_volume < 0.0 {
            log_volume.exp()
        } else {
            surplus
        }
    };
    if slope < -1.0 {
        if corner < surplus && corner * price_scale.get(corner) > surplus * price_scale.get(surplus)
        {
            return corner;
        }
        return surplus;
    }
    corner.min(surplus)
}

fn fluctuation_factor(amplitude: f32, rng: &mut fastrand::Rng) -> f32 {
    let unit = rng.f32().clamp(1e-6, 1.0 - 1e-6);
    if !(amplitude > 0.0) {
        return 1.0;
    }
    (unit / (1.0 - unit)).powf(amplitude)
}

pub(super) fn step(warehouses: &mut Warehouses, market: &mut Market, rng: &mut Rng) {
    let Warehouses {
        warehouses,
        fluctuation,
        ..
    } = warehouses;
    let fluctuation = *fluctuation;
    for (i, warehouse) in warehouses.iter_mut().enumerate() {
        for (j, stock) in warehouse.stocks.iter_mut().enumerate() {
            stock.natural_volume_delta = stock.volume - stock.previous_volume;
            stock.marketing_volume =
                stock.volume - stock.target_volume + stock.natural_volume_delta;
            let sign = stock.marketing_volume.signum();
            let base = if stock.marketing_volume > 0.0 {
                let surplus = stock.marketing_volume.abs();
                match stock.seller_rule {
                    SellerRule::TargetVolume => surplus,
                    SellerRule::RevenueMax => {
                        revenue_max_volumes(&stock.sell_volume2price_scale, surplus)
                    }
                }
            } else {
                stock.marketing_volume.abs()
            };
            let magnitude = (base * fluctuation_factor(fluctuation, rng)).clamp(0.0, base);
            stock.marketing_volume = magnitude * sign;
            stock.marketing_price_scale = if stock.marketing_volume > 0.0 {
                stock.sell_volume2price_scale.get(magnitude)
            } else {
                stock.buy_volume2price_scale.get(magnitude)
            };
            let merchandise = &mut market.traders[i].merchandises[j];
            merchandise.price = market.merchandises[j].price * stock.marketing_price_scale;
            merchandise.volume = stock.marketing_volume;
        }
    }
    market.step();
    for (i, warehouse) in warehouses.iter_mut().enumerate() {
        for (k, stock) in warehouse.stocks.iter_mut().enumerate() {
            let merchandise = &market.traders[i].merchandises[k];
            let deal_volume = merchandise.deal_volume();
            stock.volume -= deal_volume;
            let market_price = market.merchandises[k].price;
            if deal_volume != 0.0 && market_price > 0.0 {
                let realized_scale = merchandise.deal_price() / market_price;
                if deal_volume > 0.0 {
                    stock
                        .sell_volume2price_scale
                        .update(deal_volume.abs(), realized_scale);
                } else {
                    stock
                        .buy_volume2price_scale
                        .update(deal_volume.abs(), realized_scale);
                }
            }
            stock.previous_volume = stock.volume;
        }
    }
}
