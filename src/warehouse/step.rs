use fastrand::Rng;

use crate::estimator::{Estimator, PowerLaw};
use crate::market::Market;
use crate::warehouse::{SellerRule, Warehouse};

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

pub(super) fn step(warehouse: &mut Warehouse, market: &mut Market, rng: &mut Rng) {
    let Warehouse {
        traders,
        fluctuation,
        ..
    } = warehouse;
    let fluctuation = *fluctuation;
    for (i, trader) in traders.iter_mut().enumerate() {
        for (j, merchandise) in trader.merchandises.iter_mut().enumerate() {
            merchandise.natural_volume_delta = merchandise.volume - merchandise.previous_volume;
            merchandise.marketing_volume =
                merchandise.volume - merchandise.target_volume + merchandise.natural_volume_delta;
            let sign = merchandise.marketing_volume.signum();
            let base = if merchandise.marketing_volume > 0.0 {
                let surplus = merchandise.marketing_volume.abs();
                match merchandise.seller_rule {
                    SellerRule::TargetVolume => surplus,
                    SellerRule::RevenueMax => {
                        revenue_max_volumes(&merchandise.sell_volume2price_scale, surplus)
                    }
                }
            } else {
                merchandise.marketing_volume.abs()
            };
            let magnitude = (base * fluctuation_factor(fluctuation, rng)).clamp(0.0, base);
            merchandise.marketing_volume = magnitude * sign;
            merchandise.marketing_price_scale = if merchandise.marketing_volume > 0.0 {
                merchandise.sell_volume2price_scale.get(magnitude)
            } else {
                merchandise.buy_volume2price_scale.get(magnitude)
            };
            let market_merchandise = &mut market.traders[i].merchandises[j];
            market_merchandise.price =
                market.merchandises[j].price * merchandise.marketing_price_scale;
            market_merchandise.volume = merchandise.marketing_volume;
        }
    }
    market.step();
    let traders_len = market.traders.len();
    let merchandise_len = market.merchandises.len();
    for i in 0..traders_len {
        for k in 0..merchandise_len {
            let merchandise = &mut traders[i].merchandises[k];
            let market_merchandise = &market.traders[i].merchandises[k];
            let deal_volume = market_merchandise.deal_volume();
            merchandise.volume -= deal_volume;
            let market_price = market.merchandises[k].price;
            if deal_volume != 0.0 && market_price > 0.0 {
                let realized_scale = market_merchandise.deal_price() / market_price;
                if deal_volume > 0.0 {
                    merchandise
                        .sell_volume2price_scale
                        .update(deal_volume.abs(), realized_scale);
                } else {
                    merchandise
                        .buy_volume2price_scale
                        .update(deal_volume.abs(), realized_scale);
                }
            }
            merchandise.previous_volume = merchandise.volume;
        }
    }
}
