use crate::warehouse::Warehouse;
use crate::{estimator::Estimator, market::Market};

pub(super) fn step(warehouse: &mut Warehouse, market: &mut Market) {
    for (i, trader) in warehouse.traders.iter_mut().enumerate() {
        for (j, merchandise) in trader.merchandises.iter_mut().enumerate() {
            merchandise.natural_volume_delta = merchandise.volume - merchandise.previous_volume;
            merchandise.marketing_volume =
                merchandise.volume - merchandise.target_volume + merchandise.natural_volume_delta;
            if merchandise.marketing_volume > 0.0 {
                merchandise.marketing_price_scale = merchandise
                    .sell_volume2price_scale
                    .get(merchandise.marketing_volume.abs());
            } else {
                merchandise.marketing_price_scale = merchandise
                    .buy_volume2price_scale
                    .get(merchandise.marketing_volume.abs());
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
        for k in 0..merchandise_len {
            let merchandise = &mut warehouse.traders[i].merchandises[k];
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
