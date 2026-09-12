use crate::utils::{conditional_swap, geometric_average, same_signature, similarity};

pub(super) fn step(market: &mut super::Market) {
    let merchandises_len = market.merchandises.len();
    let traders_len = market.traders.len();

    // potential
    for i in 0..traders_len {
        for j in 0..traders_len {
            for k in 0..merchandises_len {
                let (price_potential, volume_potential) = 'p: {
                    let trader_merchandise0 = &market.traders[i].merchandises[k];
                    let trader_merchandise1 = &market.traders[j].merchandises[k];
                    if same_signature(trader_merchandise0.volume, trader_merchandise1.volume) {
                        break 'p (0.0, 0.0);
                    }
                    let direction = trader_merchandise0.volume > trader_merchandise1.volume;
                    let (seller, buyer) =
                        conditional_swap(trader_merchandise0, trader_merchandise1, !direction);
                    let price_potential = (buyer.price - seller.price).max(0.0);
                    let volume_potential = similarity(seller.volume, -buyer.volume);
                    (price_potential, volume_potential)
                };
                let deal = &mut market.deals[i][j][k];
                deal.price_potential = price_potential;
                deal.volume_potential = volume_potential;
                deal.potential = deal.price_potential * deal.volume_potential;
            }
        }
    }

    // distribute volume, select lower from seller and buyer
    for i in 0..traders_len {
        for k in 0..merchandises_len {
            let mut total_potential = 0.0;
            for j in 0..traders_len {
                let deal = &market.deals[i][j][k];
                total_potential += deal.potential;
            }
            for j in 0..traders_len {
                let deal = &mut market.deals[i][j][k];
                let volume = if total_potential == 0.0 {
                    0.0
                } else {
                    let distribution = deal.potential / total_potential;
                    let volume = market.traders[i].merchandises[k].volume * distribution;
                    volume
                };
                deal.volume = volume;
            }
        }
    }
    for i in 0..traders_len {
        for j in 0..traders_len {
            for k in 0..merchandises_len {
                let deal0 = &market.deals[i][j][k];
                let deal1 = &market.deals[j][i][k];
                let agree = deal0.volume.abs().min(deal1.volume.abs());
                let deal0 = &mut market.deals[i][j][k];
                deal0.volume = deal0.volume.signum() * agree;
                let deal1 = &mut market.deals[j][i][k];
                deal1.volume = deal1.volume.signum() * agree;
            }
        }
    }
    // new price
    for k in 0..merchandises_len {
        let mut total_price_volum = 0.0;
        let mut total_volume = 0.0;
        for i in 0..traders_len {
            let mut trader_price_volum = 0.0;
            let mut trader_volume = 0.0;
            for j in 0..traders_len {
                let deal = &mut market.deals[i][j][k];
                deal.price = geometric_average(
                    market.traders[i].merchandises[k].price,
                    market.traders[j].merchandises[k].price,
                );
                trader_price_volum += deal.price * deal.volume.abs();
                trader_volume += deal.volume;
                total_price_volum += deal.price * deal.volume.abs();
                total_volume += deal.volume.abs();
            }
            let trader_merchandise = &mut market.traders[i].merchandises[k];
            trader_merchandise.deal_price = if trader_volume != 0.0 {
                trader_price_volum / trader_volume.abs()
            } else {
                0.0
            };
            trader_merchandise.deal_volume = trader_volume;
        }
        if total_volume > 0.0 {
            market.merchandises[k].price = total_price_volum / total_volume;
        }
    }
}
