use fastrand::Rng;

use super::{Stock, Warehouse, Warehouses};
use crate::estimator::Estimator;
use crate::estimator2d::Estimator2D;
use crate::market::{
    Market, Merchandise as MarketMerchandise, Trader as MarketTrader, TraderMerchandise,
};

const UNBOUNDED_CURRENCY: f32 = 1e12;

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
                        .map(|&(volume, target)| Stock::new(volume, target))
                        .collect(),
                )
                .with_currency(UNBOUNDED_CURRENCY)
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
    assert!(
        (5.0..=20.0).contains(&market.merchandises[0].price),
        "两方各自择价后清算价应当还在原来的水位附近：{}",
        market.merchandises[0].price,
    );
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
    assert_close(previous_gap, 0.0, "买方按概率覆盖缺口后卖方应当清空盈余");
}

#[test]
fn a_buyer_overshoots_its_target_rather_than_missing_it() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 3);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 1.0), (0.0, 2.0)]);
    warehouse.step(&mut market, &mut rng);

    assert!(
        warehouse.warehouses[1].stocks[0].volume >= 1.0 - 1e-4,
        "买方 1 不应当低于目标：{}",
        warehouse.warehouses[1].stocks[0].volume,
    );
    assert!(
        warehouse.warehouses[2].stocks[0].volume >= 2.0 - 1e-4,
        "买方 2 不应当低于目标：{}",
        warehouse.warehouses[2].stocks[0].volume,
    );
    assert!(
        warehouse.warehouses[0].stocks[0].volume <= 10.0 + 1e-4,
        "卖方不应当买回自己的货：{}",
        warehouse.warehouses[0].stocks[0].volume,
    );
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
    assert!(
        (5.0..=20.0).contains(&market.merchandises[0].price),
        "首轮清算价应当还在原来的水位附近：{}",
        market.merchandises[0].price,
    );
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
fn fluctuation_keeps_the_direction_of_the_declaration() {
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
        // 旧断言里还有一条 `value.abs() <= base.abs() + 1e-5`（"涨落只能缩小申报"），
        // 它随 `magnitude.clamp(0, |缺口|)` 一起删掉了——那是一条策略假设，不是守恒。
        // 幂律抽样现在可以放大申报，所以两者的大小关系不再有保证，只有方向还保证。
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

fn truth_dealt(volume: f32, aggressiveness: f32) -> f32 {
    let share = aggressiveness;
    let depth = 2.0 * aggressiveness;
    (volume * share / (1.0 + volume / depth)).min(volume)
}

fn train_seller(stock: &mut Stock, price_curve: impl Fn(f32) -> f32) {
    for round in 0..600 {
        let scale = 0.25 * 1.12f32.powf((round % 24) as f32);
        let volume = 1.0 + (round % 7) as f32;
        let aggressiveness = Stock::sell_aggressiveness(scale);
        stock
            .sell_response
            .update(volume, aggressiveness, truth_dealt(volume, aggressiveness));
        stock.sell_price_curve.update(scale, price_curve(scale));
    }
}

#[test]
fn the_scale_search_terminates_far_from_one() {
    // 这条测试是为一类真的死循环写的：细化若用**绝对**容差 1e-6，就小于 f32 在
    // |log 尺度| ≈ 44 处的 ULP（3.8e-6），区间永远缩不下去 ⇒ 永不返回。
    // 旧网格把尺度限在 [0.25, 4]（log ∈ ±1.39），所以这条悬崖碰不到；
    // 把范围放开到 f32 边界之后，最优点落在远处就必然踩上它。
    let far = |log_scale: f32| -(log_scale - 40.0).abs();
    let found = super::step::maximize_log_scale(far);
    assert!(
        (found - 40.0).abs() < 1e-2,
        "远端的最大值应当被找到：{found}",
    );

    let near = |log_scale: f32| -(log_scale + 43.0).abs();
    let found = super::step::maximize_log_scale(near);
    assert!(
        (found + 43.0).abs() < 1e-2,
        "贴着数值边界的最大值也应当被找到：{found}",
    );
}

#[test]
fn a_seller_picks_the_revenue_maximizing_scale() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 6.0)]);
    train_seller(&mut warehouse.warehouses[0].stocks[0], |scale| {
        0.15 * scale.powf(1.5)
    });

    warehouse.step(&mut market, &mut rng);

    let stock = &warehouse.warehouses[0].stocks[0];
    let chosen = stock.marketing_price_scale();
    let revenue = |scale: f32| {
        stock
            .sell_response()
            .get(6.0, Stock::sell_aggressiveness(scale))
            * 10.0
            * stock.sell_price_curve().get(scale)
    };
    let best = revenue(chosen);
    // 尺度现在是**连续**的，所以"没有更好的"要在一段稠密采样上验，而不是在 49 档网格上验。
    // 采样范围就是数值边界，不是报价范围——旧断言里那个"必须选在网格内部"已经失去意义：
    // 尺度的定义域没有内部与外部，只有 f32 表示得到与表示不到。
    let limit = crate::utils::LOG_LIMIT;
    for step in 0..2001 {
        let fraction = step as f32 / 2000.0;
        let candidate = (-limit + 2.0 * limit * fraction).exp();
        assert!(
            revenue(candidate) <= best + 1e-3,
            "报价尺度 {candidate} 的收入 {} 高于所选 {chosen} 的 {best}",
            revenue(candidate),
        );
    }
    assert!(
        chosen > 0.0 && chosen.is_finite(),
        "收入最大化的尺度必须是有限正数：{chosen}",
    );
}

#[test]
fn a_seller_quotes_more_patiently_when_patience_pays() {
    let mut rng = deterministic_rng();
    let mut flat_market = market(1, 10.0, 2);
    let mut steep_market = market(1, 10.0, 2);
    let mut flat = warehouse1(&[(10.0, 4.0), (0.0, 6.0)]);
    let mut steep = warehouse1(&[(10.0, 4.0), (0.0, 6.0)]);
    train_seller(&mut flat.warehouses[0].stocks[0], |_| 1.0);
    train_seller(&mut steep.warehouses[0].stocks[0], |scale| {
        0.2 * scale.powf(1.5)
    });

    flat.step(&mut flat_market, &mut rng);
    steep.step(&mut steep_market, &mut rng);

    let flat_quote = flat_market.traders[0].merchandises[0].price;
    let steep_quote = steep_market.traders[0].merchandises[0].price;
    assert!(
        steep_quote > flat_quote,
        "耐心更值钱时应当报更高的价：平价 {flat_quote}，陡峭 {steep_quote}",
    );
}

#[test]
fn a_buyer_declares_at_least_the_gap_it_wants_to_cover() {
    let mut rng = deterministic_rng();
    let quotes = [&[(100.0, 50.0)][..], &[(0.0, 8.0)][..]];
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse(&quotes);

    warehouse.step(&mut market, &mut rng);

    let demand = market.traders[1].merchandises[0].volume;
    assert!(demand.abs() >= 8.0, "有钱的买方应当申报至少覆盖缺口：{demand}");
    assert!(
        market.traders[1].merchandises[0].price > 10.0,
        "买方应当报在市价之上：{}",
        market.traders[1].merchandises[0].price,
    );
}

#[test]
fn the_price_curve_learns_the_realized_level() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (0.0, 6.0)]);

    warehouse.step(&mut market, &mut rng);

    let stock = &warehouse.warehouses[0].stocks[0];
    let scale = stock.marketing_price_scale();
    let realized = market.traders[0].merchandises[0].deal_price() / 10.0;
    assert!(realized > 0.0, "首轮应当成交");
    let prior = scale.powf(0.5);
    let learned = stock.sell_price_curve().get(scale);
    if realized > prior {
        assert!(
            learned > prior && learned <= realized + 1e-4,
            "价格曲线未向实际成交水平上移：先验 {prior}，学得 {learned}，实际 {realized}",
        );
    } else {
        assert!(
            learned < prior && learned >= realized - 1e-4,
            "价格曲线未向实际成交水平下移：先验 {prior}，学得 {learned}，实际 {realized}",
        );
    }
}

#[test]
fn a_failed_quote_still_teaches_the_response() {
    let mut rng = deterministic_rng();
    let mut market = market(1, 10.0, 2);
    let mut warehouse = warehouse1(&[(10.0, 4.0), (5.0, 5.0)]);
    let before = warehouse.warehouses[0].stocks[0]
        .sell_response()
        .fill_ratio(6.0, 1.0);

    for _ in 0..30 {
        warehouse.step(&mut market, &mut rng);
    }

    let after = warehouse.warehouses[0].stocks[0]
        .sell_response()
        .fill_ratio(6.0, 1.0);
    assert!(
        after < before,
        "零成交应当压低响应曲线：{before} -> {after}",
    );
}
