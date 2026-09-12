use fastrand::Rng;

use super::{SellerRule, Stock, Warehouse, Warehouses};
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

fn deterministic_rng() -> Rng {
    Rng::with_seed(0)
}

fn warehouse(quotes: &[&[(f32, f32)]]) -> Warehouses {
    Warehouses::new(
        quotes
            .iter()
            .map(|goods_quotes| {
                Warehouse::new(
                    goods_quotes
                        .iter()
                        .map(|&(volume, target)| {
                            Stock::new(volume, target).with_seller_rule(SellerRule::TargetVolume)
                        })
                        .collect(),
                )
            })
            .collect(),
    )
    .with_fluctuation(0.0)
}

fn warehouse_from(quotes: &[Vec<(f32, f32)>]) -> Warehouses {
    let quotes: Vec<&[(f32, f32)]> = quotes.iter().map(|quotes| quotes.as_slice()).collect();
    warehouse(&quotes)
}

fn warehouse1(items: &[(f32, f32)]) -> Warehouses {
    let quotes: Vec<&[(f32, f32)]> = items.iter().map(std::slice::from_ref).collect();
    warehouse(&quotes)
}

fn volumes(warehouse: &Warehouses) -> Vec<f32> {
    warehouse
        .warehouses
        .iter()
        .flat_map(|trader| trader.stocks.iter().map(|item| item.volume))
        .collect()
}

fn targets(warehouse: &Warehouses) -> Vec<f32> {
    warehouse
        .warehouses
        .iter()
        .flat_map(|trader| trader.stocks.iter().map(|item| item.target_volume))
        .collect()
}

fn prices(market: &Market) -> Vec<f32> {
    market
        .merchandises
        .iter()
        .map(|merchandise| merchandise.price)
        .collect()
}

fn total_stock(warehouse: &Warehouses) -> f32 {
    warehouse
        .warehouses
        .iter()
        .flat_map(|trader| trader.stocks.iter())
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

fn assert_finite_state(warehouse: &Warehouses, market: &Market) {
    for (i, trader) in warehouse.warehouses.iter().enumerate() {
        for (k, item) in trader.stocks.iter().enumerate() {
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
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(4.0, 4.0), (8.0, 8.0), (5.0, 5.0)]);
    let before = volumes(&warehouse);

    for _ in 0..50 {
        warehouse.step(&mut market, &mut rng);
    }

    assert_eq!(before, volumes(&warehouse), "无缺口时库存不应变化");
    assert_close(market.merchandises[0].price, 10.0, "无缺口时价格");
    for i in 0..3 {
        assert_close(dealt(&market, i, 0), 0.0, "无缺口时不应有成交");
    }
}

#[test]
fn every_reachable_target_is_cleared_in_one_step() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (2.0, 8.0), (5.0, 5.0)]);

    warehouse.step(&mut market, &mut rng);

    assert_eq!(
        targets(&warehouse),
        volumes(&warehouse),
        "一步之内应清到目标"
    );
    assert_close(market.merchandises[0].price, 10.0, "清算价");
}

#[test]
fn settled_state_is_a_fixed_point() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (2.0, 8.0), (5.0, 5.0)]);
    warehouse.step(&mut market, &mut rng);
    let settled = volumes(&warehouse);
    let settled_price = prices(&market);

    for step in 1..50 {
        warehouse.step(&mut market, &mut rng);
        assert_eq!(settled, volumes(&warehouse), "第 {step} 步后库存漂移");
        assert_eq!(settled_price, prices(&market), "第 {step} 步后价格漂移");
    }
}

#[test]
fn declaration_sign_follows_the_gap() {
    let mut rng = deterministic_rng();
    for (volume, target) in [(10.0, 4.0), (2.0, 8.0), (5.0, 5.0), (0.0, 3.0)] {
        let mut market = market(1, 10.0, 2);
        let mut warehouse = warehouse1(&[(volume, target), (3.0, 3.0)]);
        warehouse.step(&mut market, &mut rng);

        let declaration = warehouse.warehouses[0].stocks[0].marketing_volume;
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
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 1.0), (0.0, 2.0)]);
    let mut previous_gap = 6.0;

    for step in 0..60 {
        warehouse.step(&mut market, &mut rng);
        let gap = warehouse.warehouses[0].stocks[0].volume - 4.0;
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
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 1.0), (0.0, 2.0)]);
    warehouse.step(&mut market, &mut rng);

    assert_close(warehouse.warehouses[0].stocks[0].volume, 7.0, "卖方库存");
    assert_close(warehouse.warehouses[1].stocks[0].volume, 1.0, "买方 1 库存");
    assert_close(warehouse.warehouses[2].stocks[0].volume, 2.0, "买方 2 库存");
    assert_close(total_stock(&warehouse), 10.0, "总库存");
}

#[test]
fn inventory_is_conserved_over_a_long_run() {
    let mut rng = deterministic_rng();
    let mut market = market(2, 10.0, 3);
    let mut warehouse = warehouse(&[
        &[(10.0, 4.0), (8.0, 12.0)],
        &[(2.0, 8.0), (14.0, 6.0)],
        &[(0.0, 1.0), (5.0, 5.0)],
    ]);
    let initial = total_stock(&warehouse);

    for step in 0..200 {
        warehouse.step(&mut market, &mut rng);
        assert_close(
            total_stock(&warehouse),
            initial,
            &format!("第 {step} 步总库存"),
        );
    }
}

#[test]
fn long_horizon_state_is_finite_and_bounded() {
    let mut rng = deterministic_rng();
    let mut market = market(2, 10.0, 3);
    market.merchandises[1].price = 12.0;
    let mut warehouse = warehouse(&[
        &[(10.0, 4.0), (8.0, 12.0)],
        &[(2.0, 8.0), (14.0, 6.0)],
        &[(5.0, 5.0), (5.0, 5.0)],
    ]);

    for step in 0..500 {
        warehouse.step(&mut market, &mut rng);
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
    let mut rng_a = deterministic_rng();
    let mut rng_b = deterministic_rng();

    for _ in 0..40 {
        warehouse_a.step(&mut market_a, &mut rng_a);
        warehouse_b.step(&mut market_b, &mut rng_b);
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
    let mut rng_base = deterministic_rng();
    let mut rng_permuted = deterministic_rng();

    for _ in 0..20 {
        base_warehouse.step(&mut base_market, &mut rng_base);
        permuted_warehouse.step(&mut permuted_market, &mut rng_permuted);
    }

    assert_close(
        base_market.merchandises[0].price,
        permuted_market.merchandises[0].price,
        "清算价",
    );
    for i in 0..4 {
        let moved = order.iter().position(|&source| source == i).unwrap();
        assert_close(
            base_warehouse.warehouses[i].stocks[0].volume,
            permuted_warehouse.warehouses[moved].stocks[0].volume,
            &format!("交易者 {i} 的库存"),
        );
    }
}

#[test]
fn zero_price_market_is_frozen_and_finite() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 0.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (2.0, 8.0)]);
    let before = volumes(&warehouse);

    for _ in 0..30 {
        warehouse.step(&mut market, &mut rng);
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
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 6.0), (5.0, 5.0)]);
    warehouse.step(&mut market, &mut rng);
    assert_close(warehouse.warehouses[2].stocks[0].volume, 5.0, "闲置轮库存");

    warehouse.warehouses[0].stocks[0].target_volume = 10.0;
    warehouse.warehouses[2].stocks[0].target_volume = 0.0;

    for _ in 0..3 {
        warehouse.step(&mut market, &mut rng);
        assert_active_quotes_are_positive(&market);
    }
    assert_close(warehouse.warehouses[2].stocks[0].volume, 0.0, "补卖者");
    assert_close(warehouse.warehouses[0].stocks[0].volume, 9.0, "回补者");
}

#[test]
fn reversing_the_same_trade_does_not_revalue_the_good() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 6.0)]);
    warehouse.step(&mut market, &mut rng);
    assert_close(market.merchandises[0].price, 10.0, "首轮清算价");
    assert_close(warehouse.warehouses[0].stocks[0].volume, 4.0, "首轮卖方");
    assert_close(warehouse.warehouses[1].stocks[0].volume, 6.0, "首轮买方");

    warehouse.warehouses[0].stocks[0].target_volume = 10.0;
    warehouse.warehouses[1].stocks[0].target_volume = 0.0;
    for _ in 0..3 {
        warehouse.step(&mut market, &mut rng);
        assert_active_quotes_are_positive(&market);
    }

    assert_close(warehouse.warehouses[0].stocks[0].volume, 10.0, "回补者");
    assert_close(warehouse.warehouses[1].stocks[0].volume, 0.0, "回吐者");
    assert!(
        (5.0..=20.0).contains(&market.merchandises[0].price),
        "同一笔交易反向后价格水位应当还在 10 附近，实际 {}",
        market.merchandises[0].price,
    );
}

#[test]
fn a_realized_trade_pulls_the_next_quote_toward_the_realized_price() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 6.0)]);
    warehouse.step(&mut market, &mut rng);
    let first_quote = market.traders[0].merchandises[0].price;
    let realized_price = market.merchandises[0].price;
    assert_close(first_quote, 10.0 / 6.0, "首轮报价");
    assert_close(realized_price, 10.0, "首轮成交价");

    warehouse.warehouses[0].stocks[0].target_volume = 0.0;
    warehouse.step(&mut market, &mut rng);
    let next_quote = market.traders[0].merchandises[0].price;

    assert!(
        next_quote > first_quote,
        "观测到按市场价成交后，同样规模的报价应当上移：{first_quote} -> {next_quote}",
    );
    assert!(
        next_quote <= realized_price + 1e-3,
        "报价不应当越过实际成交价水位：{next_quote} > {realized_price}",
    );
}

#[test]
fn zero_target_liquidates_all_stock() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(5.0, 0.0), (0.0, 5.0)]);

    warehouse.step(&mut market, &mut rng);

    assert_close(warehouse.warehouses[0].stocks[0].volume, 0.0, "清仓者");
    assert_close(warehouse.warehouses[1].stocks[0].volume, 5.0, "补库者");
    assert_eq!(targets(&warehouse), volumes(&warehouse));
}

#[test]
fn fluctuation_only_shrinks_the_declaration_and_keeps_its_direction() {
    let quotes = [&[(10.0, 4.0)][..], &[(0.0, 6.0)][..]];
    let mut deterministic = market(1, 10.0, 2);
    let mut fluctuated = market(1, 10.0, 2);
    let mut rng_plain = deterministic_rng();
    let mut rng_fluctuated = deterministic_rng();

    warehouse(&quotes).step(&mut deterministic, &mut rng_plain);
    warehouse(&quotes)
        .with_fluctuation(0.5)
        .step(&mut fluctuated, &mut rng_fluctuated);

    for i in 0..2 {
        let base = deterministic.traders[i].merchandises[0].volume;
        let value = fluctuated.traders[i].merchandises[0].volume;
        assert!(
            value.abs() <= base.abs() + 1e-5,
            "交易者 {i} 的申报被放大：基准 {base}，实际 {value}",
        );
        assert!(
            value * base >= 0.0,
            "交易者 {i} 的申报方向被翻转：基准 {base}，实际 {value}",
        );
    }
}

#[test]
fn fluctuation_is_a_deterministic_power_law_draw() {
    let quotes = vec![vec![(10.0, 4.0)], vec![(0.0, 6.0)], vec![(5.0, 5.0)]];
    let mut market_a = market(1, 10.0, 3);
    let mut market_b = market(1, 10.0, 3);
    let mut warehouse_a = warehouse_from(&quotes).with_fluctuation(0.5);
    let mut warehouse_b = warehouse_from(&quotes).with_fluctuation(0.5);
    let mut rng_a = deterministic_rng();
    let mut rng_b = deterministic_rng();
    for _ in 0..20 {
        warehouse_a.warehouses[0].stocks[0].volume += 1.0;
        warehouse_a.warehouses[1].stocks[0].volume -= 1.0;
        warehouse_b.warehouses[0].stocks[0].volume += 1.0;
        warehouse_b.warehouses[1].stocks[0].volume -= 1.0;
        warehouse_a.step(&mut market_a, &mut rng_a);
        warehouse_b.step(&mut market_b, &mut rng_b);
        assert_eq!(
            volumes(&warehouse_a),
            volumes(&warehouse_b),
            "同种子应当复现"
        );
        assert_eq!(prices(&market_a), prices(&market_b), "同种子价格应当复现");
    }

    let mut plain_market = market(1, 10.0, 3);
    let mut fluctuated_market = market(1, 10.0, 3);
    let mut plain = warehouse_from(&quotes);
    let mut fluctuated = warehouse_from(&quotes).with_fluctuation(0.5);
    let mut rng_plain = deterministic_rng();
    let mut rng_fluctuated = deterministic_rng();
    let mut differs = false;
    for _ in 0..20 {
        plain.warehouses[0].stocks[0].volume += 1.0;
        plain.warehouses[1].stocks[0].volume -= 1.0;
        fluctuated.warehouses[0].stocks[0].volume += 1.0;
        fluctuated.warehouses[1].stocks[0].volume -= 1.0;
        plain.step(&mut plain_market, &mut rng_plain);
        fluctuated.step(&mut fluctuated_market, &mut rng_fluctuated);
        if volumes(&plain) != volumes(&fluctuated) {
            differs = true;
        }
    }
    assert!(differs, "波动开启后轨迹应当与关闭时不同");
}

#[test]
fn seed_selects_the_fluctuation_path() {
    let quotes = vec![vec![(10.0, 4.0)], vec![(0.0, 6.0)], vec![(5.0, 5.0)]];
    let run = |seed: u64| {
        let mut market = market(1, 10.0, 3);
        let mut warehouse = warehouse_from(&quotes).with_fluctuation(0.5);
        let mut rng = Rng::with_seed(seed);
        for _ in 0..20 {
            warehouse.warehouses[0].stocks[0].volume += 1.0;
            warehouse.warehouses[1].stocks[0].volume -= 1.0;
            warehouse.step(&mut market, &mut rng);
        }
        market.merchandises[0].price
    };

    assert_eq!(run(7), run(7), "同一种子应当复现同一条路径");
    let seeds = [0u64, 1, 2, 3, 7, 11];
    let prices: Vec<f32> = seeds.iter().map(|seed| run(*seed)).collect();
    let first = prices[0];
    assert!(
        prices.iter().any(|price| *price != first),
        "不同种子应当走出不同路径：{prices:?}",
    );
}

#[test]
fn seller_rule_switches_the_offered_volume() {
    let mut rng = deterministic_rng();
    let quotes = [&[(10.0, 4.0)][..], &[(0.0, 6.0)][..]];
    let mut target_market = market(1, 10.0, 2);
    let mut withhold_market = market(1, 10.0, 2);

    let target_rule = Warehouses::new(
        quotes
            .iter()
            .map(|quotes| {
                Warehouse::new(
                    quotes
                        .iter()
                        .map(|&(volume, target)| {
                            Stock::new(volume, target).with_seller_rule(SellerRule::TargetVolume)
                        })
                        .collect(),
                )
            })
            .collect(),
    )
    .with_fluctuation(0.0);
    let withhold_rule = Warehouses::new(
        quotes
            .iter()
            .map(|quotes| {
                Warehouse::new(
                    quotes
                        .iter()
                        .map(|&(volume, target)| {
                            Stock::new(volume, target).with_seller_rule(SellerRule::RevenueMax)
                        })
                        .collect(),
                )
            })
            .collect(),
    )
    .with_fluctuation(0.0);

    let mut target_rule = target_rule;
    let mut withhold_rule = withhold_rule;
    target_rule.step(&mut target_market, &mut rng);
    withhold_rule.step(&mut withhold_market, &mut rng);

    let target_offer = target_market.traders[0].merchandises[0].volume;
    let withhold_offer = withhold_market.traders[0].merchandises[0].volume;
    assert!(target_offer > 0.0, "基准卖方应当卖出");
    assert!(
        withhold_offer < target_offer,
        "收益最大应当挂出更少的申报：基准 {target_offer}，实际 {withhold_offer}",
    );
    assert!(
        withhold_market.traders[0].merchandises[0].price
            > target_market.traders[0].merchandises[0].price,
        "少卖应当报出更高的价格",
    );
}

#[test]
fn revenue_max_withholds_when_the_revenue_is_flat() {
    let mut rng = deterministic_rng();
    let quotes = [&[(10.0, 4.0)][..], &[(0.0, 6.0)][..]];
    let mut market = market(1, 10.0, 2);
    let mut warehouse = Warehouses::new(
        quotes
            .iter()
            .map(|quotes| {
                Warehouse::new(
                    quotes
                        .iter()
                        .map(|&(volume, target)| Stock::new(volume, target))
                        .collect(),
                )
            })
            .collect(),
    )
    .with_fluctuation(0.0);

    warehouse.step(&mut market, &mut rng);

    let offer = market.traders[0].merchandises[0].volume;
    let ask = market.traders[0].merchandises[0].price;
    let dealt = market.traders[1].merchandises[0].deal_volume();
    assert!(
        offer > 0.0 && offer < 0.01,
        "单位弹性下利润对该量完全平坦，应当挂出最少量的解：{offer}",
    );
    assert!(ask > 1000.0, "少卖应当顶到价格尺度上限：{ask}");
    assert_eq!(dealt, 0.0, "报价过高应当无法成交：{dealt}");
}
