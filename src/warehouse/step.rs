use crate::warehouse::Warehouse;
use crate::{estimator::Estimator, market::Market};

pub(super) fn step(warehouse: &mut Warehouse, market: &mut Market) {
    for (i, trader) in warehouse.traders.iter_mut().enumerate() {
        for (j, merchandise) in trader.merchandises.iter_mut().enumerate() {
            merchandise.natural_volume_delta = merchandise.volume - merchandise.previous_volume;
            merchandise.marketing_volume =
                merchandise.target_volume - merchandise.volume - merchandise.natural_volume_delta;
            if merchandise.marketing_volume > 0.0 {
                let sell_volume_scale_reciprocal = 1.0 / merchandise.marketing_volume.abs();
                merchandise.marketing_price_scale = merchandise
                    .sell_volume_scale_reciprocal2price_scale
                    .get(sell_volume_scale_reciprocal);
            } else {
                let buy_volume_scale = merchandise.marketing_volume.abs();
                merchandise.marketing_price_scale = merchandise
                    .buy_volume_scale2price_scale
                    .get(buy_volume_scale);
            }
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
        for j in 0..traders_len {
            for k in 0..merchandise_len {
                let deal = &market.deals[i][j][k];
                let merchandise = &mut warehouse.traders[i].merchandises[k];
                merchandise.volume -= deal.volume;
                if deal.volume > 0.0 {
                    merchandise
                        .sell_volume_scale_reciprocal2price_scale
                        .update(1.0 / deal.volume.abs(), merchandise.marketing_price_scale);
                } else if deal.volume < 0.0 {
                    merchandise
                        .buy_volume_scale2price_scale
                        .update(deal.volume.abs(), merchandise.marketing_price_scale);
                }
                merchandise.previous_volume = merchandise.volume;
            }
        }
    }
}
