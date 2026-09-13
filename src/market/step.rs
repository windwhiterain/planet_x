use crate::utils::{conditional_swap, geometric_average, same_signature};

pub(super) fn step(market: &mut super::Market) {
    let merchandises_len = market.merchandises.len();
    let traders_len = market.traders.len();
    let soft_eps = market.soft_eps;

    // potential
    for i in 0..traders_len {
        for j in 0..traders_len {
            for k in 0..merchandises_len {
                let (price_potential, volume_potential, soft_price) = 'p: {
                    let trader_merchandise0 = &market.traders[i].merchandises[k];
                    let trader_merchandise1 = &market.traders[j].merchandises[k];
                    if same_signature(trader_merchandise0.volume, trader_merchandise1.volume) {
                        break 'p (0.0, 0.0, 0.0);
                    }
                    let volume_potential = trader_merchandise1.volume.abs();
                    let direction = trader_merchandise0.volume > trader_merchandise1.volume;
                    let (seller, buyer) =
                        conditional_swap(trader_merchandise0, trader_merchandise1, !direction);
                    let gap = seller.price - buyer.price;
                    if gap <= 0.0 {
                        // 交叉：照旧，价差即潜在量，成交价走几何平均
                        (-gap, volume_potential, 0.0)
                    } else if soft_eps > 0.0 && gap < soft_eps * buyer.price {
                        // 软成交：卖方略高于买价 ⇒ 以 买价×(1−eps) 卖出一小笔，
                        // 潜在量随价差线性缩水。gap = 0 处给的正量顺带修掉了
                        // "买价 == 卖价 ⇒ price_potential = 0 ⇒ 锁价账本不成交"这个边角。
                        let tolerance = soft_eps * buyer.price;
                        (
                            tolerance - gap,
                            volume_potential,
                            buyer.price * (1.0 - soft_eps),
                        )
                    } else {
                        (0.0, 0.0, 0.0)
                    }
                };
                let deal = &mut market.deals[i][j][k];
                deal.price_potential = price_potential;
                deal.volume_potential = volume_potential;
                deal.soft_price = soft_price;
                let relation = market.relations[i][j].clamp(0.0, 1.0);
                deal.distribution = deal.price_potential * deal.volume_potential * relation;
            }
        }
    }

    // distribute volume, select lower from seller and buyer
    for i in 0..traders_len {
        for k in 0..merchandises_len {
            let mut total_potential = 0.0;
            for j in 0..traders_len {
                let deal = &market.deals[i][j][k];
                total_potential += deal.distribution;
            }
            for j in 0..traders_len {
                let deal = &mut market.deals[i][j][k];
                let volume = if total_potential == 0.0 {
                    0.0
                } else {
                    deal.distribution /= total_potential;
                    let volume = market.traders[i].merchandises[k].volume * deal.distribution;
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
                deal.price = if deal.soft_price > 0.0 {
                    // 软成交：卖方让步到 买价×(1−eps)。**不能**用几何平均——那时
                    // 几何平均高于买价，等于让买方付得比自己的出价还多。
                    deal.soft_price
                } else {
                    geometric_average(
                        market.traders[i].merchandises[k].price,
                        market.traders[j].merchandises[k].price,
                    )
                };
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
            let price = total_price_volum / total_volume;
            market.merchandises[k].price = price;
            market.state.observe(k, price);
        }
    }
}
