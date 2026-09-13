use super::*;

fn market_from(goods: usize, price: f32, quotes: &[Vec<(f32, f32)>]) -> Market {
    let merchandises = (0..goods).map(|_| Merchandise { price }).collect();
    let traders = quotes
        .iter()
        .map(|goods_quotes| Trader {
            merchandises: goods_quotes
                .iter()
                .map(|&(price, volume)| TraderMerchandise::new(price, volume))
                .collect(),
        })
        .collect();
    Market::new(merchandises, traders)
}

fn market(goods: usize, quotes: &[&[(f32, f32)]]) -> Market {
    let quotes: Vec<Vec<(f32, f32)>> = quotes.iter().map(|quotes| quotes.to_vec()).collect();
    market_from(goods, 0.0, &quotes)
}

fn net(market: &Market, i: usize, k: usize) -> f32 {
    (0..market.traders.len())
        .map(|j| market.deals[i][j][k].volume)
        .sum()
}

fn dealt(market: &Market, i: usize, k: usize) -> f32 {
    (0..market.traders.len())
        .map(|j| market.deals[i][j][k].volume.abs())
        .sum()
}

fn deal_volumes(market: &Market) -> Vec<f32> {
    let mut volumes = Vec::new();
    for row in &market.deals {
        for column in row {
            for deal in column {
                volumes.push(deal.volume);
            }
        }
    }
    volumes
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-5,
        "期望 {expected}，实际 {actual}",
    );
}

fn assert_volume_bound(market: &Market) {
    for k in 0..market.merchandises.len() {
        for i in 0..market.traders.len() {
            let own = market.traders[i].merchandises[k].volume;
            assert!(
                dealt(market, i, k) <= own.abs() + 1e-5,
                "交易者 {i} 商品 {k}：成交 {} 超过申报 {}",
                dealt(market, i, k),
                own.abs(),
            );
            let net = net(market, i, k);
            assert!(
                net * own >= -1e-5,
                "交易者 {i} 商品 {k}：申报 {own} 却净成交 {net}（方向被翻）",
            );
        }
    }
}

fn assert_finite_table(market: &Market) {
    for row in &market.deals {
        for column in row {
            for deal in column {
                assert!(deal.volume.is_finite(), "成交量非有限：{}", deal.volume);
                assert!(deal.price.is_finite(), "成交价非有限：{}", deal.price);
            }
        }
    }
    for merchandise in &market.merchandises {
        assert!(
            merchandise.price.is_finite(),
            "市场价非有限：{}",
            merchandise.price
        );
    }
}

#[test]
fn new_preallocates_deal_table() {
    let market = market(2, &[&[(1.0, 1.0), (1.0, 0.0)], &[(1.0, -1.0), (1.0, 0.0)]]);
    assert_eq!(market.deals.len(), 2);
    for row in &market.deals {
        assert_eq!(row.len(), 2);
        for column in row {
            assert_eq!(column.len(), 2);
            assert!(
                column
                    .iter()
                    .all(|deal| deal.volume == 0.0 && deal.price == 0.0),
                "新建的成交表应当全是 0",
            );
        }
    }
}

#[test]
fn empty_market_keeps_its_price() {
    let mut market = market_from(2, 7.5, &[]);
    market.step();
    assert!(market.deals.is_empty());
    assert_close(market.merchandises[0].price, 7.5);
    assert_close(market.merchandises[1].price, 7.5);
}

#[test]
fn no_trade_keeps_the_previous_price() {
    let mut same_side = market_from(1, 7.5, &[vec![(10.0, 5.0)], vec![(12.0, 5.0)]]);
    same_side.step();
    assert_close(same_side.merchandises[0].price, 7.5);

    let mut idle = market_from(1, 7.5, &[vec![(10.0, 3.0)], vec![(10.0, 0.0)]]);
    idle.step();
    assert_close(idle.merchandises[0].price, 7.5);

    // 交叉不了的一对（卖 25 / 买 20）：**旧硬限价**下确实不成交，价格不动
    let mut not_crossing = market_from(1, 7.5, &[vec![(20.0, -7.0)], vec![(25.0, 2.0)]]);
    not_crossing.soft_eps = 0.0;
    not_crossing.step();
    assert_close(not_crossing.merchandises[0].price, 7.5);

    // 同样的报价在**软成交**下成交了——这正是"软成交替代限价"的落点：
    // 价格 = min(几何平均 √500 ≈ 22.36, 买价 20 × (1 − 0.15)) = 17
    let mut soft = market_from(1, 7.5, &[vec![(20.0, -7.0)], vec![(25.0, 2.0)]]);
    soft.step();
    assert_close(
        soft.merchandises[0].price,
        20.0 * (1.0 - Market::DEFAULT_SOFT_EPS),
    );
}

#[test]
fn nan_quote_does_not_poison_the_market_price() {
    let mut market = market_from(1, 7.5, &[vec![(f32::NAN, 1.0)], vec![(10.0, -1.0)]]);
    market.step();
    assert_close(market.merchandises[0].price, 7.5);
    for row in &market.deals {
        for column in row {
            assert_close(column[0].volume, 0.0);
        }
    }
}

#[test]
fn infinite_quote_does_not_poison_the_market_price() {
    let mut market = market_from(1, 7.5, &[vec![(f32::INFINITY, 1.0)], vec![(10.0, -1.0)]]);
    market.step();
    assert_close(market.merchandises[0].price, 7.5);
}

#[test]
fn seller_sells_only_its_own_volume() {
    let mut market = market(1, &[&[(10.0, 1.0)], &[(12.0, -100.0)]]);
    market.step();

    assert_close(market.deals[0][1][0].volume, 1.0);
    assert_close(market.deals[1][0][0].volume, -1.0);
    assert_close(net(&market, 0, 0), 1.0);
    assert_close(net(&market, 1, 0), -1.0);
    assert_volume_bound(&market);
}

#[test]
fn deal_table_is_independent_of_trader_order() {
    let mut seller_first = market(1, &[&[(10.0, 1.0)], &[(12.0, -100.0)]]);
    seller_first.step();
    let mut buyer_first = market(1, &[&[(12.0, -100.0)], &[(10.0, 1.0)]]);
    buyer_first.step();

    assert_close(net(&seller_first, 0, 0), 1.0);
    assert_close(net(&seller_first, 1, 0), -1.0);
    assert_close(net(&buyer_first, 0, 0), -1.0);
    assert_close(net(&buyer_first, 1, 0), 1.0);
}

#[test]
fn permuting_traders_permutes_the_whole_deal_table() {
    let base = vec![
        vec![(10.0, 5.0), (20.0, -3.0)],
        vec![(12.0, -4.0), (22.0, 1.0)],
        vec![(11.0, -2.0), (21.0, -2.0)],
        vec![(13.0, 3.0), (19.0, 4.0)],
    ];
    let order = [2usize, 0, 3, 1];
    let permuted: Vec<Vec<(f32, f32)>> = order.iter().map(|&i| base[i].clone()).collect();

    let mut base_market = market_from(2, 9.0, &base);
    let mut permuted_market = market_from(2, 9.0, &permuted);
    base_market.step();
    permuted_market.step();

    for k in 0..2 {
        assert_close(
            base_market.merchandises[k].price,
            permuted_market.merchandises[k].price,
        );
        for i in 0..4 {
            let moved = order.iter().position(|&source| source == i).unwrap();
            for j in 0..4 {
                let moved_j = order.iter().position(|&source| source == j).unwrap();
                assert_close(
                    base_market.deals[i][j][k].volume,
                    permuted_market.deals[moved][moved_j][k].volume,
                );
            }
        }
    }
}

#[test]
fn idle_trader_does_not_perturb_the_others() {
    let mut without = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)]]);
    let mut with = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(11.0, 0.0)]]);
    without.step();
    with.step();

    assert_close(without.merchandises[0].price, with.merchandises[0].price);
    assert_close(net(&without, 0, 0), net(&with, 0, 0));
    assert_close(net(&without, 1, 0), net(&with, 1, 0));
    assert_close(net(&with, 2, 0), 0.0);
}

#[test]
fn buyer_never_ends_up_selling() {
    let mut market = market(1, &[&[(12.0, -5.0)], &[(10.0, 3.0)], &[(10.0, 9.0)]]);
    market.step();

    assert_close(net(&market, 0, 0), -5.0);
    assert!(net(&market, 1, 0) > 0.0, "卖方 1 应当是净卖出");
    assert!(net(&market, 2, 0) > 0.0, "卖方 2 应当是净卖出");
    assert_close(
        net(&market, 0, 0) + net(&market, 1, 0) + net(&market, 2, 0),
        0.0,
    );
    assert_volume_bound(&market);
}

#[test]
fn volume_is_rationed_proportionally_across_counterparties() {
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(12.0, -4.0)]]);
    market.step();

    assert_close(net(&market, 0, 0), 5.0);
    assert_close(net(&market, 1, 0), -2.5);
    assert_close(net(&market, 2, 0), -2.5);
    assert_volume_bound(&market);
}

#[test]
fn no_trader_deals_more_than_it_declared() {
    let mut cases = vec![
        market(1, &[&[(10.0, 1.0)], &[(12.0, -100.0)]]),
        market(1, &[&[(12.0, -100.0)], &[(10.0, 1.0)]]),
        market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(12.0, -4.0)]]),
        market(1, &[&[(12.0, -5.0)], &[(10.0, 3.0)], &[(10.0, 9.0)]]),
        market(
            2,
            &[&[(10.0, 1.0), (20.0, -7.0)], &[(12.0, -100.0), (25.0, 2.0)]],
        ),
    ];
    for case in &mut cases {
        case.step();
        assert_volume_bound(case);
    }
}

#[test]
fn deal_table_is_antisymmetric() {
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(11.0, -2.0)]]);
    market.step();

    for i in 0..market.traders.len() {
        for j in 0..market.traders.len() {
            assert_close(market.deals[i][j][0].volume, -market.deals[j][i][0].volume);
        }
    }
}

#[test]
fn self_deal_is_zero() {
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(11.0, -2.0)]]);
    market.step();

    for i in 0..market.traders.len() {
        assert_close(market.deals[i][i][0].volume, 0.0);
    }
}

#[test]
fn every_commodity_clears_to_zero_sum() {
    let mut market = market(
        2,
        &[
            &[(10.0, 5.0), (20.0, -3.0)],
            &[(12.0, -4.0), (22.0, 1.0)],
            &[(11.0, -2.0), (21.0, -2.0)],
        ],
    );
    market.step();

    for k in 0..market.merchandises.len() {
        let total: f32 = (0..market.traders.len()).map(|i| net(&market, i, k)).sum();
        assert_close(total, 0.0);
    }
}

#[test]
fn trader_deal_volume_is_the_net_of_its_row() {
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(11.0, 0.0)]]);
    market.step();

    for i in 0..market.traders.len() {
        assert_close(
            market.traders[i].merchandises[0].deal_volume(),
            net(&market, i, 0),
        );
    }
}

#[test]
fn trader_deal_price_is_a_positive_finite_price() {
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(11.0, 0.0)]]);
    market.step();

    for (i, trader) in market.traders.iter().enumerate() {
        let deal_price = trader.merchandises[0].deal_price();
        let deal_volume = trader.merchandises[0].deal_volume();
        assert!(
            deal_price.is_finite(),
            "交易者 {i} 的成交价非有限：{deal_price}",
        );
        if deal_volume != 0.0 {
            assert!(
                deal_price > 0.0,
                "交易者 {i} 成交 {deal_volume} 却得到非正成交价 {deal_price}",
            );
        }
    }
}

#[test]
fn traded_price_lies_between_the_two_quotes() {
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, -4.0)], &[(11.0, -2.0)]]);
    market.step();

    // ⚠️ **不变量换了。** 旧契约是"成交价落在两个报价之间"。软成交按定义做不到：
    // 价格上界是 `买价 × (1−eps)`，卖方要价一旦高于该上界，成交价就**同时低于两个报价**
    // （实测卖 10 / 买 11 那一对成交在 9.35 = 11 × 0.85）。
    // 换成的两条契约是：
    //   ① 不高于买方的买价——买方永远不会付得比自己的出价多；
    //   ② 不低于 `min(卖价, 买价×(1−eps))`——不会低于更便宜的那一方。
    let eps = Market::DEFAULT_SOFT_EPS;
    for i in 0..market.traders.len() {
        for j in 0..market.traders.len() {
            let deal = &market.deals[i][j][0];
            if deal.volume == 0.0 {
                continue;
            }
            let ask = market.traders[i].merchandises[0].price;
            let bid = market.traders[j].merchandises[0].price;
            let (ask, bid) = if ask > bid { (bid, ask) } else { (ask, bid) };
            assert!(
                deal.price <= bid,
                "成交价 {} 高于买价 {bid}",
                deal.price,
            );
            assert!(
                deal.price >= ask.min(bid * (1.0 - eps)) - 1e-4,
                "成交价 {} 低于 min(卖价 {ask}, 买价×(1−eps))",
                deal.price,
            );
        }
    }
    assert!((9.35..=12.0).contains(&market.merchandises[0].price));
}

#[test]
fn price_is_the_volume_weighted_geometric_average_of_the_quotes() {
    // 硬限价（eps = 0）下价格就是几何平均本身
    let mut hard = market(1, &[&[(10.0, 1.0)], &[(12.0, -1.0)]]);
    hard.soft_eps = 0.0;
    hard.step();
    assert_close(hard.merchandises[0].price, f32::sqrt(120.0));
    assert_close(hard.deals[0][1][0].price, f32::sqrt(120.0));

    // 软成交下再取 `买价 × (1−eps)` 的上界。注意这一对**是交叉的**（卖 10 < 买 12），
    // 但几何平均 10.954 仍然高于上界 10.2，所以照样被压到 10.2——
    // `eps` 是买方对自己出价留的那一份，与卖价高低无关。
    let mut soft = market(1, &[&[(10.0, 1.0)], &[(12.0, -1.0)]]);
    soft.step();
    assert_close(
        soft.merchandises[0].price,
        12.0 * (1.0 - Market::DEFAULT_SOFT_EPS),
    );
}

#[test]
fn repeated_clearing_of_frozen_quotes_is_a_fixed_point() {
    let mut market = market(
        2,
        &[
            &[(10.0, 5.0), (20.0, -3.0)],
            &[(12.0, -4.0), (22.0, 1.0)],
            &[(11.0, -2.0), (21.0, -2.0)],
        ],
    );
    market.step();
    let first = deal_volumes(&market);
    let first_price = market.merchandises[0].price;

    market.step();
    assert_eq!(first, deal_volumes(&market), "冻结报价下成交表应当不动");
    assert_eq!(first_price, market.merchandises[0].price);
}

#[test]
fn long_horizon_clearing_stays_finite_and_bounded() {
    let mut market = market(
        2,
        &[
            &[(10.0, 5.0), (20.0, -3.0)],
            &[(12.0, -4.0), (22.0, 1.0)],
            &[(11.0, -2.0), (21.0, -2.0)],
            &[(13.0, 3.0), (19.0, 4.0)],
        ],
    );
    for step in 0..1000 {
        market.step();
        assert_finite_table(&market);
        for k in 0..market.merchandises.len() {
            let price = market.merchandises[k].price;
            // 下界 10 → 10×(1−eps)：软成交的价格上界本来就在买价之下，
            // 所以最低可以压到 `最低买价 × (1−eps) = 8.5`（实测 9.943）。
            assert!(
                (10.0 * (1.0 - Market::DEFAULT_SOFT_EPS)..=22.0).contains(&price),
                "第 {step} 步商品 {k} 的价格 {price} 越出报价区间",
            );
        }
    }
}

#[test]
fn solve_is_repeatable() {
    let mut market = market(1, &[&[(12.0, -5.0)], &[(10.0, 3.0)], &[(10.0, 9.0)]]);
    market.step();
    let first = deal_volumes(&market);

    market.step();
    let second = deal_volumes(&market);

    assert_eq!(first, second, "同一份申报连算两次应当得到同一张成交表");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "等长且同序")]
fn mismatched_merchandise_table_panics() {
    let _ = market(2, &[&[(1.0, 1.0), (1.0, 0.0)], &[(1.0, -1.0)]]);
}
