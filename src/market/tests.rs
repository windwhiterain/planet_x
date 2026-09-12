//! 市场撮合的行为测试：成交量、成交方向、以及成交表的维度与守恒。

use super::*;

/// 按「每个交易者一组 (价, 量)」建市场，商品数由 `goods` 指定。
fn market(goods: usize, quotes: &[&[(f32, f32)]]) -> Market {
    let merchandises = (0..goods).map(|_| Merchandise { price: 0.0 }).collect();
    let traders = quotes
        .iter()
        .map(|goods_quotes| Trader {
            merchandises: goods_quotes
                .iter()
                .map(|&(price, volume)| TraderMerchandise { price, volume })
                .collect(),
        })
        .collect();
    Market::new(merchandises, traders)
}

/// 交易者 `i` 在商品 `k` 上的净成交（正 = 净卖出，负 = 净买入）。
fn net(market: &Market, i: usize, k: usize) -> f32 {
    (0..market.traders.len())
        .map(|j| market.deals[i][j][k].volume)
        .sum()
}

/// 交易者 `i` 在商品 `k` 上成交的总量（不计方向）。
fn dealt(market: &Market, i: usize, k: usize) -> f32 {
    (0..market.traders.len())
        .map(|j| market.deals[i][j][k].volume.abs())
        .sum()
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 1e-5,
        "期望 {expected}，实际 {actual}",
    );
}

/// 每个交易者的成交量不超过自己申报的量，且净成交方向与自己申报的方向一致。
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
fn solve_on_empty_market_does_not_panic() {
    let mut market = market(2, &[]);
    market.step();
    assert!(market.deals.is_empty());
    // 注：无成交时 merchandises[k].price 目前被写成 f32::INFINITY（哨兵口径未定，这里不断言）。
}

#[test]
fn seller_sells_only_its_own_volume() {
    // t0 只肯卖 1，t1 想买 100：成交只能是 1。修复前 t0 会被成交 100。
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
    // 同一笔经济事实，只把两个交易者在 Vec 里的顺序对调。
    // 修复前：卖出方在前 -> 超卖（卖 1 成交 100）；买入方在前 -> 方向整体反转。
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
fn buyer_never_ends_up_selling() {
    // t0 买 5，排在两个卖方前面：修复前整张表方向反转（买 5 变成卖 5）。
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
    // 供给 5 < 需求 8：卖方卖满 5，两个买家按潜在量比例各拿一半。
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
    // deals[i][j] 必须是 deals[j][i] 的反号：同一笔成交在两个视角下方向相反。
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
fn no_deal_when_prices_do_not_cross() {
    // 买方出 20，卖方要 25：价格不交叉，成交量必须为 0。
    let mut market = market(1, &[&[(20.0, -7.0)], &[(25.0, 2.0)]]);
    market.step();

    for row in &market.deals {
        for column in row {
            assert_close(column[0].volume, 0.0);
        }
    }
}

#[test]
fn no_deal_when_everyone_is_on_the_same_side() {
    // 两个都想卖：没人接，成交量必须为 0。
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, 5.0)]]);
    market.step();

    for row in &market.deals {
        for column in row {
            assert_close(column[0].volume, 0.0);
        }
    }
}

#[test]
fn solve_is_repeatable() {
    let mut market = market(1, &[&[(12.0, -5.0)], &[(10.0, 3.0)], &[(10.0, 9.0)]]);
    market.step();
    let first: Vec<f32> = market
        .deals
        .iter()
        .flat_map(|row| row.iter().map(|column| column[0].volume))
        .collect();

    market.step();
    let second: Vec<f32> = market
        .deals
        .iter()
        .flat_map(|row| row.iter().map(|column| column[0].volume))
        .collect();

    assert_eq!(first, second, "同一份申报连算两次应当得到同一张成交表");
}

#[test]
#[should_panic(expected = "等长且同序")]
fn mismatched_merchandise_table_panics() {
    // 同下标必须同商品：长度不一致时（debug 构建）立刻炸，而不是撮合到一半才越界。
    let _ = market(2, &[&[(1.0, 1.0), (1.0, 0.0)], &[(1.0, -1.0)]]);
}
