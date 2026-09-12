use crate::utils::{conditional_swap, geometric_average, same_signature};

/// 价差下界：买卖报价至少差这么多，保证一定交叉
const MIN_SPREAD: f32 = 0.02;
const MIN_PRICE: f32 = 1e-3;
const MAX_PRICE: f32 = 1e3;
const CLEARING_STEPS: usize = 48;

pub(super) fn step(market: &mut super::Market) {
    let merchandises_len = market.merchandises.len();
    let traders_len = market.traders.len();

    // 出清价：用各家的申报曲线解 Σ 量(p) = 0，再按解出的价重算量与报价
    for k in 0..merchandises_len {
        let price = clearing_price(market, k);
        market.merchandises[k].price = price;
        for trader in market.traders.iter_mut() {
            let trader_merchandise = &mut trader.merchandises[k];
            trader_merchandise.volume = trader_merchandise.volume_at(price);
        }
        center_quotes(market, k, price);
    }

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
                    let volume_potential = trader_merchandise1.volume.abs();
                    let direction = trader_merchandise0.volume > trader_merchandise1.volume;
                    let (seller, buyer) =
                        conditional_swap(trader_merchandise0, trader_merchandise1, !direction);
                    let price_potential = (buyer.price - seller.price).max(0.0);
                    (price_potential, volume_potential)
                };
                let deal = &mut market.deals[i][j][k];
                deal.price_potential = price_potential;
                deal.volume_potential = volume_potential;
                deal.distribution = deal.price_potential * deal.volume_potential;
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
    // 成交价：两侧报价几何居中，所以每一笔都落在市价上
    for k in 0..merchandises_len {
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
            }
            let trader_merchandise = &mut market.traders[i].merchandises[k];
            trader_merchandise.deal_price = if trader_volume != 0.0 {
                trader_price_volum / trader_volume.abs()
            } else {
                0.0
            };
            trader_merchandise.deal_volume = trader_volume;
        }
    }
}

/// 解出使超额需求为零的市价；解不出就用边界价
fn clearing_price(market: &super::Market, k: usize) -> f32 {
    let excess = |price: f32| -> f32 {
        market
            .traders
            .iter()
            .map(|trader| trader.merchandises[k].volume_at(price))
            .sum()
    };
    let (mut low, mut high) = (MIN_PRICE, MAX_PRICE);
    if excess(low) > 0.0 {
        return low;
    }
    if excess(high) < 0.0 {
        return high;
    }
    for _ in 0..CLEARING_STEPS {
        let middle = (low * high).sqrt();
        let value = excess(middle);
        if !value.is_finite() {
            break;
        }
        if value > 0.0 {
            high = middle;
        } else {
            low = middle;
        }
    }
    (low * high).sqrt()
}

/// 报价围绕市价几何居中：买卖各取一边，两侧报价相乘恰为市价²
fn center_quotes(market: &mut super::Market, k: usize, price: f32) {
    if price <= 0.0 {
        return;
    }
    let mut sell_log = 0.0;
    let mut sell_count = 0.0f32;
    let mut buy_log = 0.0;
    let mut buy_count = 0.0f32;
    for trader in market.traders.iter() {
        let trader_merchandise = &trader.merchandises[k];
        if trader_merchandise.volume > 0.0 && trader_merchandise.price > 0.0 {
            sell_log += (trader_merchandise.price / price).ln();
            sell_count += 1.0;
        } else if trader_merchandise.volume < 0.0 && trader_merchandise.price > 0.0 {
            buy_log += (trader_merchandise.price / price).ln();
            buy_count += 1.0;
        }
    }
    if sell_count == 0.0 || buy_count == 0.0 {
        return;
    }
    let spread = ((buy_log / buy_count - sell_log / sell_count).exp()).max(1.0 + MIN_SPREAD);
    let half = spread.sqrt();
    for trader in market.traders.iter_mut() {
        let trader_merchandise = &mut trader.merchandises[k];
        trader_merchandise.price = if trader_merchandise.volume > 0.0 {
            price / half
        } else if trader_merchandise.volume < 0.0 {
            price * half
        } else {
            price
        };
    }
}
