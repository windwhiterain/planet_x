use crate::warehouse::Warehouse;
use crate::{estimator::Estimator, market::Market};

pub(super) fn step(warehouse: &mut Warehouse, market: &mut Market) {
    for (i, trader) in warehouse.traders.iter_mut().enumerate() {
        for (j, merchanidise) in trader.merchadises.iter_mut().enumerate() {
            merchanidise.natural_volume_delta = merchanidise.volume - merchanidise.previous_volume;
            merchanidise.marketing_volume = merchanidise.target_volume
                - merchanidise.volume
                - merchanidise.natural_volume_delta;
            if merchanidise.marketing_volume > 0.0 {
                let sell_volume_scale_reciprocal = 1.0 / merchanidise.marketing_volume.abs();
                merchanidise.marketing_price_scale = merchanidise
                    .sell_volume_scale_reciprocal2price_scale
                    .get(sell_volume_scale_reciprocal);
            } else {
                let buy_volume_scale = merchanidise.marketing_volume.abs();
                merchanidise.marketing_price_scale = merchanidise
                    .buy_volume_scale2price_scale
                    .get(buy_volume_scale);
            }
            let market_merchandise = &mut market.traders[i].merchandises[j];
            market_merchandise.price =
                market.merchandises[j].price * merchanidise.marketing_price_scale;
            market_merchandise.volume = merchanidise.marketing_volume;
        }
    }
    market.step();
    let traders_len = market.traders.len();
    let merchandise_len = market.merchandises.len();
    for i in 0..traders_len {
        for j in 0..traders_len {
            for k in 0..merchandise_len {
                let deal = &market.deals[i][j][k];
                let merchandise = &mut warehouse.traders[i].merchadises[k];
                merchandise.volume -= deal.volume;
                merchandise.marketing_volume = deal.volume;
                if deal.volume > 0.0 {
                    merchandise
                        .sell_volume_scale_reciprocal2price_scale
                        .update(1.0 / deal.volume, merchandise.marketing_price_scale);
                } else if deal.volume < 0.0 {
                    merchandise
                        .buy_volume_scale2price_scale
                        .update(deal.volume, merchandise.marketing_price_scale);
                }
            }
        }
    }
}
