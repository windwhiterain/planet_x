use crate::utils::{conditional_swap, geometric_average, same_signature};

pub(super) fn step(market: &mut super::Market) {
    let merchandises_len = market.merchandises.len();
    let traders_len = market.traders.len();
    let soft_eps = market.soft_eps;

    // potential
    for i in 0..traders_len {
        for j in 0..traders_len {
            for k in 0..merchandises_len {
                let (price_potential, volume_potential, price_cap) = 'p: {
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
                    if soft_eps <= 0.0 {
                        // 旧的硬限价，只用于对照：报价差一点就**精确地**不成交
                        if gap <= 0.0 {
                            (-gap, volume_potential, 0.0)
                        } else {
                            (0.0, 0.0, 0.0)
                        }
                    } else if buyer.price > 0.0 && seller.price.is_finite() {
                        // 软成交（**替代限价**，不是把限价往外挪）：**无论价差多大都成交**。
                        //
                        // 「无论价差多大」不包括非有限的报价：NaN / inf 不是一个报价，
                        // 拿它成交只会把价格打成 0 或 NaN。
                        //
                        //   价格上界 = 买价 × (1 − eps)        ← 卖方拿不到买价本身
                        //   卖方量   = 申报量 ÷ max(1, 1 + (卖价 − 买价(1−eps))/(买价·eps))
                        //
                        // 价差不再决定"成不成交"，只决定"成交多少"——**价格竞争换成数量竞争**。
                        // 卖方越贪，让步越大，拿到的那一份越小，但永远拿得到一份。
                        //
                        // `max(1, ·)` 是必需的：原式在 `卖价 = 买价(1−2eps)` 处分母过零、
                        // 再往左变负（量先趋于无穷再变负）。夹在 1 上的语义正好是
                        // "比限价还便宜的卖方拿满量，但不许超过自己申报的量"。
                        let cap = buyer.price * (1.0 - soft_eps);
                        let concession = (seller.price - cap).max(0.0);
                        let divisor = (1.0 + concession / (buyer.price * soft_eps)).max(1.0);
                        (1.0 / divisor, volume_potential, cap)
                    } else {
                        (0.0, 0.0, 0.0)
                    }
                };
                let deal = &mut market.deals[i][j][k];
                deal.price_potential = price_potential;
                deal.volume_potential = volume_potential;
                deal.price_cap = price_cap;
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
                let natural = geometric_average(
                    market.traders[i].merchandises[k].price,
                    market.traders[j].merchandises[k].price,
                );
                deal.price = if !natural.is_finite() {
                    // 报价里有非有限值（NaN / inf）时几何平均也是非有限的。**必须在这里
                    // 截断**：下面按 `price × |volume|` 加权，而没成交的配对 volume 是 0，
                    // `NaN × 0 = NaN` 会把整条成交量加权和毒成 NaN（实测整场价格变 NaN）。
                    0.0
                } else if deal.price_cap > 0.0 {
                    // 软成交：价格**上界**是 买价×(1−eps)。取 min 而不是直接替换——
                    // 卖方报得比限价还便宜时，几何平均本来就更低，该让它继续起作用。
                    natural.min(deal.price_cap)
                } else {
                    natural
                };
                // 只有真的成交了的配对才进加权和：没成交的配对价格只是副产品，
                // 让它以 0 权重参与毫无意义，还会把非有限值带进来。
                if deal.volume != 0.0 {
                    trader_price_volum += deal.price * deal.volume.abs();
                    total_price_volum += deal.price * deal.volume.abs();
                }
                trader_volume += deal.volume;
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
