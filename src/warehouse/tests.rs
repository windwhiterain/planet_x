use super::{Merchandise, Trader, Warehouse};
use crate::market::{
    Market, Merchandise as MarketMerchandise, Trader as MarketTrader, TraderMerchandise,
};

fn market(goods: usize, price: f32, traders: usize) -> Market {
    let merchandises = (0..goods).map(|_| MarketMerchandise { price }).collect();
    let traders = (0..traders)
        .map(|_| MarketTrader {
            merchandises: (0..goods)
                .map(|_| TraderMerchandise::new(0.0, 0.0))
                .collect(),
        })
        .collect();
    Market::new(merchandises, traders)
}

fn warehouse(quotes: &[&[(f32, f32)]]) -> Warehouse {
    Warehouse::new(
        quotes
            .iter()
            .map(|goods_quotes| {
                Trader::new(
                    goods_quotes
                        .iter()
                        .map(|&(volume, target)| Merchandise::new(volume, target))
                        .collect(),
                )
            })
            .collect(),
    )
}

fn warehouse_from(quotes: &[Vec<(f32, f32)>]) -> Warehouse {
    let quotes: Vec<&[(f32, f32)]> = quotes.iter().map(|quotes| quotes.as_slice()).collect();
    warehouse(&quotes)
}

fn warehouse1(items: &[(f32, f32)]) -> Warehouse {
    let quotes: Vec<&[(f32, f32)]> = items.iter().map(std::slice::from_ref).collect();
    warehouse(&quotes)
}

fn volumes(warehouse: &Warehouse) -> Vec<f32> {
    warehouse
        .traders
        .iter()
        .flat_map(|trader| trader.merchandises.iter().map(|item| item.volume))
        .collect()
}

fn targets(warehouse: &Warehouse) -> Vec<f32> {
    warehouse
        .traders
        .iter()
        .flat_map(|trader| trader.merchandises.iter().map(|item| item.target_volume))
        .collect()
}

fn prices(market: &Market) -> Vec<f32> {
    market
        .merchandises
        .iter()
        .map(|merchandise| merchandise.price)
        .collect()
}

fn total_stock(warehouse: &Warehouse) -> f32 {
    warehouse
        .traders
        .iter()
        .flat_map(|trader| trader.merchandises.iter())
        .map(|item| item.volume)
        .sum()
}

fn dealt(market: &Market, i: usize, k: usize) -> f32 {
    (0..market.traders.len())
        .map(|j| market.deals[i][j][k].volume.abs())
        .sum()
}

fn assert_close(actual: f32, expected: f32, context: &str) {
    let tolerance = 1e-4 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}：期望 {expected}，实际 {actual}",
    );
}

fn assert_finite_state(warehouse: &Warehouse, market: &Market) {
    for (i, trader) in warehouse.traders.iter().enumerate() {
        for (k, item) in trader.merchandises.iter().enumerate() {
            assert!(
                item.volume.is_finite(),
                "交易者 {i} 商品 {k} 的库存非有限：{}",
                item.volume
            );
            assert!(
                item.marketing_volume.is_finite() && item.marketing_price_scale.is_finite(),
                "交易者 {i} 商品 {k} 的报价非有限：{} / {}",
                item.marketing_volume,
                item.marketing_price_scale,
            );
            assert!(
                item.natural_volume_delta.is_finite(),
                "交易者 {i} 商品 {k} 的自然增减非有限：{}",
                item.natural_volume_delta,
            );
        }
    }
    for (k, merchandise) in market.merchandises.iter().enumerate() {
        assert!(
            merchandise.price.is_finite(),
            "市场商品 {k} 的价格非有限：{}",
            merchandise.price
        );
    }
    for row in &market.deals {
        for column in row {
            for deal in column {
                assert!(deal.volume.is_finite(), "成交量非有限：{}", deal.volume);
                assert!(deal.price.is_finite(), "成交价非有限：{}", deal.price);
            }
        }
    }
}

#[test]
fn trader_at_target_stays_silent() {
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(4.0, 4.0), (8.0, 8.0), (5.0, 5.0)]);
    let before = volumes(&warehouse);

    for _ in 0..50 {
        warehouse.step(&mut market);
    }

    assert_eq!(before, volumes(&warehouse), "无缺口时库存不应变化");
    assert_close(market.merchandises[0].price, 10.0, "无缺口时价格");
    for i in 0..3 {
        assert_close(dealt(&market, i, 0), 0.0, "无缺口时不应有成交");
    }
}

#[test]
fn every_reachable_target_is_cleared_in_one_step() {
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (2.0, 8.0), (5.0, 5.0)]);

    warehouse.step(&mut market);

    assert_eq!(
        targets(&warehouse),
        volumes(&warehouse),
        "一步之内应清到目标"
    );
    assert_close(market.merchandises[0].price, 10.0, "清算价");
}

#[test]
fn settled_state_is_a_fixed_point() {
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (2.0, 8.0), (5.0, 5.0)]);
    warehouse.step(&mut market);
    let settled = volumes(&warehouse);
    let settled_price = prices(&market);

    for step in 1..50 {
        warehouse.step(&mut market);
        assert_eq!(settled, volumes(&warehouse), "第 {step} 步后库存漂移");
        assert_eq!(settled_price, prices(&market), "第 {step} 步后价格漂移");
    }
}

#[test]
fn declaration_sign_follows_the_gap() {
    for (volume, target) in [(10.0, 4.0), (2.0, 8.0), (5.0, 5.0), (0.0, 3.0)] {
        let mut market = market(1, 10.0, 2);
        let mut warehouse = warehouse1(&[(volume, target), (3.0, 3.0)]);
        warehouse.step(&mut market);

        let declaration = warehouse.traders[0].merchandises[0].marketing_volume;
        let gap = volume - target;
        assert!(
            declaration * gap >= 0.0,
            "库存 {volume} 目标 {target}：申报 {declaration} 与缺口 {gap} 方向相反",
        );
        if gap != 0.0 {
            assert!(declaration != 0.0, "有缺口却申报 0");
        }
    }
}

#[test]
fn stock_moves_toward_the_target_without_overshoot() {
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 1.0), (0.0, 2.0)]);
    let mut previous_gap = 6.0;

    for step in 0..60 {
        warehouse.step(&mut market);
        let gap = warehouse.traders[0].merchandises[0].volume - 4.0;
        assert!(gap >= -1e-4, "第 {step} 步越过了目标：库存缺口 {gap}");
        assert!(
            gap <= previous_gap + 1e-4,
            "第 {step} 步缺口从 {previous_gap} 涨到 {gap}",
        );
        previous_gap = gap;
    }
    assert_close(previous_gap, 3.0, "需求受限时应收敛到剩余缺口");
}

#[test]
fn stock_is_bounded_by_the_counterparties_demand() {
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 1.0), (0.0, 2.0)]);
    warehouse.step(&mut market);

    assert_close(warehouse.traders[0].merchandises[0].volume, 7.0, "卖方库存");
    assert_close(
        warehouse.traders[1].merchandises[0].volume,
        1.0,
        "买方 1 库存",
    );
    assert_close(
        warehouse.traders[2].merchandises[0].volume,
        2.0,
        "买方 2 库存",
    );
    assert_close(total_stock(&warehouse), 10.0, "总库存");
}

#[test]
fn inventory_is_conserved_over_a_long_run() {
    let mut market = market(2, 10.0, 3);
    let mut warehouse = warehouse(&[
        &[(10.0, 4.0), (8.0, 12.0)],
        &[(2.0, 8.0), (14.0, 6.0)],
        &[(0.0, 1.0), (5.0, 5.0)],
    ]);
    let initial = total_stock(&warehouse);

    for step in 0..200 {
        warehouse.step(&mut market);
        assert_close(
            total_stock(&warehouse),
            initial,
            &format!("第 {step} 步总库存"),
        );
    }
}

#[test]
fn long_horizon_state_is_finite_and_bounded() {
    let mut market = market(2, 10.0, 3);
    market.merchandises[1].price = 12.0;
    let mut warehouse = warehouse(&[
        &[(10.0, 4.0), (8.0, 12.0)],
        &[(2.0, 8.0), (14.0, 6.0)],
        &[(5.0, 5.0), (5.0, 5.0)],
    ]);

    for step in 0..500 {
        warehouse.step(&mut market);
        assert_finite_state(&warehouse, &market);
        for (k, price) in prices(&market).iter().enumerate() {
            assert!(
                (1e-3..=1e3).contains(price),
                "第 {step} 步商品 {k} 的价格 {price} 失控",
            );
        }
        for (i, volume) in volumes(&warehouse).iter().enumerate() {
            assert!(
                volume.abs() <= 1e3,
                "第 {step} 步第 {i} 项库存 {volume} 失控",
            );
        }
    }
}

#[test]
fn warehouse_run_is_deterministic() {
    let mut market_a = market(2, 10.0, 3);
    let mut market_b = market(2, 10.0, 3);
    let quotes = vec![
        vec![(10.0, 4.0), (8.0, 12.0)],
        vec![(2.0, 8.0), (14.0, 6.0)],
        vec![(5.0, 5.0), (5.0, 5.0)],
    ];
    let mut warehouse_a = warehouse_from(&quotes);
    let mut warehouse_b = warehouse_from(&quotes);

    for _ in 0..40 {
        warehouse_a.step(&mut market_a);
        warehouse_b.step(&mut market_b);
        assert_eq!(volumes(&warehouse_a), volumes(&warehouse_b));
        assert_eq!(prices(&market_a), prices(&market_b));
    }
}

#[test]
fn trader_order_permutes_the_trajectory() {
    let base = vec![
        vec![(10.0, 4.0)],
        vec![(0.0, 1.0)],
        vec![(0.0, 2.0)],
        vec![(0.0, 3.0)],
    ];
    let order = [2usize, 0, 3, 1];
    let permuted: Vec<Vec<(f32, f32)>> = order.iter().map(|&i| base[i].clone()).collect();

    let mut base_market = market(1, 10.0, 4);
    let mut permuted_market = market(1, 10.0, 4);
    let mut base_warehouse = warehouse_from(&base);
    let mut permuted_warehouse = warehouse_from(&permuted);

    for _ in 0..20 {
        base_warehouse.step(&mut base_market);
        permuted_warehouse.step(&mut permuted_market);
    }

    assert_close(
        base_market.merchandises[0].price,
        permuted_market.merchandises[0].price,
        "清算价",
    );
    for i in 0..4 {
        let moved = order.iter().position(|&source| source == i).unwrap();
        assert_close(
            base_warehouse.traders[i].merchandises[0].volume,
            permuted_warehouse.traders[moved].merchandises[0].volume,
            &format!("交易者 {i} 的库存"),
        );
    }
}

#[test]
fn zero_price_market_is_frozen_and_finite() {
    let mut market = market(1, 0.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (2.0, 8.0)]);
    let before = volumes(&warehouse);

    for _ in 0..30 {
        warehouse.step(&mut market);
        assert_finite_state(&warehouse, &market);
    }

    assert_eq!(before, volumes(&warehouse), "零价格下不应有库存变动");
    assert_close(market.merchandises[0].price, 0.0, "零价格");
}

fn assert_active_quotes_are_positive(market: &Market) {
    for (i, trader) in market.traders.iter().enumerate() {
        for (k, item) in trader.merchandises.iter().enumerate() {
            assert!(
                item.price.is_finite(),
                "交易者 {i} 商品 {k} 的报价非有限：{}",
                item.price,
            );
            if item.volume != 0.0 {
                assert!(
                    item.price > 0.0,
                    "交易者 {i} 商品 {k} 申报 {} 却挂出非正价格 {}",
                    item.volume,
                    item.price,
                );
            }
        }
    }
}

#[test]
fn an_idle_round_does_not_lock_a_trader_out_of_the_market() {
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 6.0), (5.0, 5.0)]);
    warehouse.step(&mut market);
    assert_close(
        warehouse.traders[2].merchandises[0].volume,
        5.0,
        "闲置轮库存",
    );

    warehouse.traders[0].merchandises[0].target_volume = 10.0;
    warehouse.traders[2].merchandises[0].target_volume = 0.0;

    for _ in 0..3 {
        warehouse.step(&mut market);
        assert_active_quotes_are_positive(&market);
    }
    assert_close(warehouse.traders[2].merchandises[0].volume, 0.0, "补卖者");
    assert_close(warehouse.traders[0].merchandises[0].volume, 9.0, "回补者");
}

#[test]
fn reversing_the_same_trade_does_not_revalue_the_good() {
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 6.0)]);
    warehouse.step(&mut market);
    assert_close(market.merchandises[0].price, 10.0, "首轮清算价");
    assert_close(warehouse.traders[0].merchandises[0].volume, 4.0, "首轮卖方");
    assert_close(warehouse.traders[1].merchandises[0].volume, 6.0, "首轮买方");

    warehouse.traders[0].merchandises[0].target_volume = 10.0;
    warehouse.traders[1].merchandises[0].target_volume = 0.0;
    for _ in 0..3 {
        warehouse.step(&mut market);
        assert_active_quotes_are_positive(&market);
    }

    assert_close(warehouse.traders[0].merchandises[0].volume, 10.0, "回补者");
    assert_close(warehouse.traders[1].merchandises[0].volume, 0.0, "回吐者");
    assert!(
        (5.0..=20.0).contains(&market.merchandises[0].price),
        "同一笔交易反向后价格水位应当还在 10 附近，实际 {}",
        market.merchandises[0].price,
    );
}

#[test]
fn zero_target_liquidates_all_stock() {
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(5.0, 0.0), (0.0, 5.0)]);

    warehouse.step(&mut market);

    assert_close(warehouse.traders[0].merchandises[0].volume, 0.0, "清仓者");
    assert_close(warehouse.traders[1].merchandises[0].volume, 5.0, "补库者");
    assert_eq!(targets(&warehouse), volumes(&warehouse));
}
