use crate::utils::geometric_average;

pub(super) fn step(market: &mut super::Market) {
    let goods = market.merchandises.len();
    let traders = market.traders.len();
    let scale = market.flow_scale;

    for row in market.deals.iter_mut() {
        for deals in row.iter_mut() {
            for deal in deals.iter_mut() {
                deal.volume = 0.0;
                deal.price = 0.0;
            }
        }
    }

    for k in 0..goods {
        for i in 0..traders {
            let price_i = market.traders[i].merchandises[k].price;
            if !price_i.is_finite() || !(price_i > 0.0) {
                continue;
            }
            let stock_i = market.traders[i].merchandises[k].volume;
            if !stock_i.is_finite() || !(stock_i > 0.0) {
                continue;
            }
            let mut weight = vec![0.0f32; traders];
            let mut total = 0.0f32;
            for j in 0..traders {
                if i == j {
                    continue;
                }
                let price_j = market.traders[j].merchandises[k].price;
                if !price_j.is_finite() || !(price_j > price_i) {
                    continue;
                }
                let gap = (price_j / price_i).ln();
                let phi = (gap / scale).tanh();
                if !phi.is_finite() || !(phi > 0.0) {
                    continue;
                }
                let relation = market.relations[i][j].clamp(0.0, 1.0);
                weight[j] = phi * relation;
                total += weight[j];
            }
            if !total.is_finite() || !(total > 0.0) {
                continue;
            }
            let shipped = total.min(1.0);
            for j in 0..traders {
                let share = weight[j];
                if !(share > 0.0) {
                    continue;
                }
                let flow = stock_i * shipped * (share / total);
                if !flow.is_finite() || !(flow > 0.0) {
                    continue;
                }
                market.deals[i][j][k].volume = flow;
                market.deals[j][i][k].volume = -flow;
            }
        }

        for i in 0..traders {
            let mut net = 0.0f32;
            let mut weight = 0.0f32;
            let mut weighted = 0.0f32;
            for j in 0..traders {
                let deal = &mut market.deals[i][j][k];
                let natural = geometric_average(
                    market.traders[i].merchandises[k].price,
                    market.traders[j].merchandises[k].price,
                );
                deal.price = if natural.is_finite() { natural } else { 0.0 };
                if deal.volume != 0.0 {
                    weighted += deal.price * deal.volume.abs();
                    weight += deal.volume.abs();
                }
                net += deal.volume;
            }
            let merchandise = &mut market.traders[i].merchandises[k];
            merchandise.deal_volume = net;
            merchandise.deal_price = if weight > 0.0 { weighted / weight } else { 0.0 };
        }

        let mut total_price_volume = 0.0;
        let mut total_volume = 0.0;
        for i in 0..traders {
            for j in 0..traders {
                let deal = &market.deals[i][j][k];
                if deal.volume > 0.0 {
                    total_price_volume += deal.price * deal.volume;
                    total_volume += deal.volume;
                }
            }
        }
        if total_volume > 0.0 {
            let price = total_price_volume / total_volume;
            market.merchandises[k].price = price;
            market.state.observe(k, price);
        }
    }
}
