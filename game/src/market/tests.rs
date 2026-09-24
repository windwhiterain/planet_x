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
    let market = market_from(2, 1.0, &[vec![(1.0, 1.0); 2], vec![(1.0, 1.0); 2]]);
    assert_eq!(market.deals.len(), 2);
    assert_eq!(market.deals[0].len(), 2);
    assert_eq!(market.deals[0][0].len(), 2);
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
fn flow_goes_from_the_low_price_to_the_high_price() {
    let mut market = market(1, &[&[(3.0, 10.0)], &[(9.0, 4.0)]]);
    market.step();
    assert!(
        market.deals[0][1][0].volume > 0.0,
        "低挂价的一方应当出货：{:?}",
        market.deals[0][1][0].volume,
    );
    assert!(market.deals[1][0][0].volume < 0.0);
    assert!(net(&market, 0, 0) > 0.0 && net(&market, 1, 0) < 0.0);
}

#[test]
fn equal_prices_move_nothing() {
    let mut market = market_from(1, 5.0, &[vec![(5.0, 10.0)], vec![(5.0, 10.0)]]);
    market.step();
    for row in &market.deals {
        for column in row {
            assert_close(column[0].volume, 0.0);
        }
    }
    assert_close(market.merchandises[0].price, 5.0);
}

#[test]
fn a_shipper_never_ships_more_than_its_stock() {
    for scale in [0.05f32, 0.5, 4.0] {
        let mut market = market(1, &[&[(3.0, 10.0)], &[(9.0, 4.0)], &[(20.0, 4.0)]]);
        market.flow_scale = scale;
        market.step();
        for i in 0..market.traders.len() {
            let own = market.traders[i].merchandises[0].volume;
            let shipped: f32 = (0..market.traders.len())
                .map(|j| market.deals[i][j][0].volume.max(0.0))
                .sum();
            assert!(
                shipped <= own + 1e-5,
                "scale {scale}：交易者 {i} 出货 {shipped} 超过库存 {own}",
            );
        }
    }
}

#[test]
fn a_receiver_is_never_forced_to_ship() {
    let mut market = market(1, &[&[(8.0, 0.5)], &[(2.0, 4.0)]]);
    market.step();
    assert!(
        market.traders[0].merchandises[0].deal_volume() < 0.0,
        "高挂价但库存极少的一方应当是收货方",
    );
}

#[test]
fn nan_quote_does_not_poison_the_market_price() {
    let mut market = market_from(1, 7.5, &[vec![(f32::NAN, 1.0)], vec![(10.0, 1.0)]]);
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
    let mut market = market_from(1, 7.5, &[vec![(f32::INFINITY, 1.0)], vec![(10.0, 1.0)]]);
    market.step();
    assert_close(market.merchandises[0].price, 7.5);
}

#[test]
fn self_deal_is_zero() {
    let mut market = market(1, &[&[(3.0, 10.0)], &[(9.0, 4.0)]]);
    market.step();
    for i in 0..market.traders.len() {
        assert_close(market.deals[i][i][0].volume, 0.0);
    }
}

#[test]
fn deal_table_is_antisymmetric() {
    let mut market = market(1, &[&[(3.0, 10.0)], &[(9.0, 4.0)], &[(6.0, 7.0)]]);
    market.step();
    for i in 0..market.traders.len() {
        for j in 0..market.traders.len() {
            assert_close(market.deals[i][j][0].volume, -market.deals[j][i][0].volume);
        }
    }
}

#[test]
fn every_commodity_clears_to_zero_sum() {
    let mut market = market(2, &[&[(3.0, 10.0), (4.0, 2.0)], &[(9.0, 4.0), (1.0, 5.0)]]);
    market.step();
    for k in 0..2 {
        let total: f32 = (0..market.traders.len()).map(|i| net(&market, i, k)).sum();
        assert_close(total, 0.0);
    }
}

#[test]
fn no_stock_is_created_out_of_nothing() {
    let mut market = market(1, &[&[(3.0, 10.0)], &[(9.0, 4.0)], &[(6.0, 7.0)]]);
    let before: f32 = (0..market.traders.len())
        .map(|i| market.traders[i].merchandises[0].volume)
        .sum();
    market.step();
    let after: f32 = (0..market.traders.len())
        .map(|i| market.traders[i].merchandises[0].volume - net(&market, i, 0))
        .sum();
    assert_close(after, before);
}

#[test]
fn traded_price_lies_between_the_two_quotes() {
    let mut market = market(1, &[&[(10.0, 5.0)], &[(12.0, 4.0)], &[(11.0, 2.0)]]);
    market.step();
    for i in 0..market.traders.len() {
        for j in 0..market.traders.len() {
            let deal = &market.deals[i][j][0];
            if deal.volume == 0.0 {
                continue;
            }
            let a = market.traders[i].merchandises[0].price;
            let b = market.traders[j].merchandises[0].price;
            let (low, high) = if a < b { (a, b) } else { (b, a) };
            assert!(
                deal.price >= low - 1e-4 && deal.price <= high + 1e-4,
                "成交价 {} 落在报价 [{low}, {high}] 之外",
                deal.price,
            );
        }
    }
    assert!((10.0..=12.0).contains(&market.merchandises[0].price));
}

#[test]
fn price_is_the_volume_weighted_geometric_average_of_the_quotes() {
    let mut market = market(1, &[&[(10.0, 1.0)], &[(12.0, 1.0)]]);
    market.step();
    assert_close(market.merchandises[0].price, f32::sqrt(120.0));
    assert_close(market.deals[0][1][0].price, f32::sqrt(120.0));
}

#[test]
fn solve_is_repeatable() {
    let quotes: &[&[(f32, f32)]] = &[&[(3.0, 10.0)], &[(9.0, 4.0)], &[(6.0, 7.0)]];
    let mut first = market(1, quotes);
    let mut second = market(1, quotes);
    first.step();
    second.step();
    assert_eq!(deal_volumes(&first), deal_volumes(&second));
    assert_eq!(first.merchandises[0].price, second.merchandises[0].price);
}

#[test]
fn a_bigger_price_gap_moves_more() {
    let mut narrow = market(1, &[&[(9.5, 20.0)], &[(10.5, 1.0)]]);
    let mut wide = market(1, &[&[(1.0, 20.0)], &[(20.0, 1.0)]]);
    narrow.step();
    wide.step();
    assert!(
        wide.deals[0][1][0].volume > narrow.deals[0][1][0].volume,
        "价差越大势越大：窄 {} 宽 {}",
        narrow.deals[0][1][0].volume,
        wide.deals[0][1][0].volume,
    );
}

#[test]
fn long_horizon_clearing_stays_finite_and_bounded() {
    let mut market = market(
        2,
        &[
            &[(3.0, 100.0), (7.0, 1.0)],
            &[(9.0, 40.0), (1.0, 80.0)],
            &[(6.0, 70.0), (4.0, 30.0)],
        ],
    );
    for _ in 0..500 {
        market.step();
        assert_finite_table(&market);
        for i in 0..market.traders.len() {
            for k in 0..2 {
                assert!(dealt(&market, i, k).is_finite());
            }
        }
    }
}

#[test]
#[should_panic]
fn mismatched_merchandise_table_panics() {
    let merchandises = vec![Merchandise { price: 1.0 }, Merchandise { price: 1.0 }];
    let traders = vec![Trader {
        merchandises: vec![TraderMerchandise::new(1.0, 1.0)],
    }];
    Market::new(merchandises, traders);
}
