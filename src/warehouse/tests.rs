use super::{Stock, Warehouse, Warehouses};
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};

fn market(goods: usize, traders: usize) -> Market {
    Market::new(
        (0..goods).map(|_| Merchandise { price: 1.0 }).collect(),
        (0..traders)
            .map(|_| Trader {
                merchandises: (0..goods)
                    .map(|_| TraderMerchandise::new(0.0, 0.0))
                    .collect(),
            })
            .collect(),
    )
}

fn build(stocks: &[&[(f32, f32)]]) -> Warehouses {
    Warehouses::new(
        stocks
            .iter()
            .map(|goods| {
                Warehouse::new(
                    goods
                        .iter()
                        .map(|&(volume, target)| Stock::new(volume, target))
                        .collect(),
                )
                .with_reference(vec![1.0; goods.len()])
            })
            .collect(),
    )
}

fn prices(warehouse: &Warehouses, k: usize) -> Vec<f32> {
    warehouse
        .warehouses
        .iter()
        .map(|warehouse| warehouse.stocks[k].price)
        .collect()
}

fn volumes(warehouse: &Warehouses, k: usize) -> Vec<f32> {
    warehouse
        .warehouses
        .iter()
        .map(|warehouse| warehouse.stocks[k].volume)
        .collect()
}

fn assert_finite(warehouse: &Warehouses) {
    for warehouse in &warehouse.warehouses {
        for stock in &warehouse.stocks {
            assert!(
                stock.price.is_finite() && stock.price > 0.0,
                "挂价 {}",
                stock.price
            );
            assert!(stock.volume.is_finite(), "库存 {}", stock.volume);
            assert!(
                stock.target_volume.is_finite(),
                "目标 {}",
                stock.target_volume
            );
        }
    }
}

#[test]
fn a_glut_lowers_the_price_and_a_shortage_raises_it() {
    let mut warehouse = build(&[&[(0.5, 1.0)], &[(9.0, 1.0)]]);
    warehouse.set_price_law(1.0, 0.0, 0.0);
    let mut market = market(1, 2);
    warehouse.step(&mut market);
    let price = prices(&warehouse, 0);
    assert!(
        price[0] > 1.0,
        "缺货的一方应当挂高价（对目标而言库存不足）：{}",
        price[0],
    );
    assert!(price[1] < 1.0, "过剩的一方应当挂低价：{}", price[1]);
    assert!(price[0] > price[1], "缺货方必须比过剩方贵");
}

#[test]
fn the_map_is_bounded_by_the_curvature() {
    for curvature in [0.5f32, 1.0, 3.0] {
        let mut warehouse = build(&[&[(1e-6, 1.0)], &[(1e6, 1.0)]]);
        warehouse.set_price_law(curvature, 0.0, 0.0);
        let mut market = market(1, 2);
        warehouse.step(&mut market);
        let price = prices(&warehouse, 0);
        let band = curvature.exp();
        assert!(
            price[0] <= band + 1e-3 && price[0] >= 1.0,
            "缺货方的挂价 {:.4} 超出上界 e^{curvature} = {band:.4}",
            price[0],
        );
        assert!(
            price[1] >= 1.0 / band - 1e-3 && price[1] <= 1.0,
            "过剩方的挂价 {:.4} 超出下界",
            price[1],
        );
    }
}

#[test]
fn inertia_damps_the_first_step() {
    let stocks: &[&[(f32, f32)]] = &[&[(9.0, 1.0)]];
    let mut fast = build(stocks);
    fast.set_price_law(1.0, 0.0, 0.0);
    let mut slow = build(stocks);
    slow.set_price_law(1.0, 0.9, 0.0);
    let mut market = market(1, 1);
    fast.step(&mut market);
    slow.step(&mut market);
    let fast_drop = 1.0 - prices(&fast, 0)[0];
    let slow_drop = 1.0 - prices(&slow, 0)[0];
    assert!(fast_drop > 0.0, "惯性 0 时第一步就该降价");
    assert!(
        slow_drop < fast_drop * 0.5,
        "惯性 0.9 应当把第一步压到远小于无惯性的幅度：{slow_drop} vs {fast_drop}",
    );
}

#[test]
fn the_target_rises_when_the_department_wanted_more_than_it_took() {
    let mut warehouse = build(&[&[(5.0, 2.0)]]);
    warehouse.set_price_law(1.0, 0.0, 0.1);
    warehouse.warehouses[0].stocks[0].record_take(8.0, 2.0);
    let mut market = market(1, 1);
    warehouse.step(&mut market);
    assert!(
        warehouse.warehouses[0].stocks[0].target_volume > 2.0,
        "意愿大于实际 ⇒ 目标应当抬起来：{}",
        warehouse.warehouses[0].stocks[0].target_volume,
    );
}

#[test]
fn the_target_falls_when_the_department_was_served() {
    let mut warehouse = build(&[&[(5.0, 2.0)]]);
    warehouse.set_price_law(1.0, 0.0, 0.1);
    warehouse.warehouses[0].stocks[0].target_volume = 6.0;
    warehouse.warehouses[0].stocks[0].record_take(1.0, 3.0);
    let mut market = market(1, 1);
    warehouse.step(&mut market);
    assert!(
        warehouse.warehouses[0].stocks[0].target_volume < 6.0,
        "意愿小于实际 ⇒ 目标应当降下来：{}",
        warehouse.warehouses[0].stocks[0].target_volume,
    );
}

#[test]
fn the_target_never_leaves_its_floor_and_cap() {
    let mut warehouse = build(&[&[(5.0, 2.0)]]);
    warehouse.set_price_law(1.0, 0.0, 1.0);
    let mut market = market(1, 1);
    for _ in 0..50 {
        warehouse.warehouses[0].stocks[0].record_take(1e9, 0.0);
        warehouse.step(&mut market);
        let target = warehouse.warehouses[0].stocks[0].target_volume;
        assert!(target >= 2.0 - 1e-3, "目标跌破下限：{target}");
        assert!(target <= Warehouses::TARGET_CAP, "目标冲破上限：{target}");
    }
    for _ in 0..50 {
        warehouse.warehouses[0].stocks[0].record_take(0.0, 1e9);
        warehouse.step(&mut market);
        let target = warehouse.warehouses[0].stocks[0].target_volume;
        assert!(target >= 2.0 - 1e-3, "目标跌破下限：{target}");
    }
}

#[test]
fn a_zero_target_still_gives_a_finite_price() {
    let mut warehouse = build(&[&[(0.0, 0.0)]]);
    let mut market = market(1, 1);
    for _ in 0..20 {
        warehouse.step(&mut market);
        assert_finite(&warehouse);
    }
}

#[test]
fn the_index_is_the_geometric_mean_of_the_posted_prices() {
    let mut warehouse = build(&[&[(1.0, 1.0)], &[(4.0, 1.0)], &[(16.0, 1.0)]]);
    for (i, stock) in warehouse.warehouses.iter_mut().enumerate() {
        stock.stocks[0].price = [2.0, 8.0, 32.0][i];
    }
    let index = warehouse.quoted_index(1);
    let expected = (2.0f32 * 8.0 * 32.0f32).powf(1.0 / 3.0);
    assert!(
        (index[0] - expected).abs() < 1e-4,
        "几何平均应当是 {expected}，实际 {}",
        index[0],
    );
}

#[test]
fn the_index_ignores_a_warehouse_without_a_price() {
    let mut warehouse = build(&[&[(1.0, 1.0)], &[(1.0, 1.0)]]);
    warehouse.warehouses[0].stocks[0].price = 4.0;
    warehouse.warehouses[1].stocks[0].price = 0.0;
    let index = warehouse.quoted_index(1);
    assert!((index[0] - 4.0).abs() < 1e-5, "无挂价的仓库不该拉低指数");
}

#[test]
fn local_quotes_report_the_minimum_and_maximum_posted_price() {
    let mut warehouse = build(&[&[(1.0, 1.0)], &[(1.0, 1.0)]]);
    for (i, stock) in warehouse.warehouses.iter_mut().enumerate() {
        stock.locality = 0;
        stock.stocks[0].price = [3.0, 7.0][i];
    }
    let mut market = market(1, 2);
    warehouse.step(&mut market);
    assert!(
        (warehouse.ask[0][0] - 3.0).abs() < 1e-4,
        "{:?}",
        warehouse.ask
    );
    assert!(
        (warehouse.bid[0][0] - 7.0).abs() < 1e-4,
        "{:?}",
        warehouse.bid
    );
}

#[test]
fn the_market_receives_the_stock_and_not_a_declaration() {
    let mut warehouse = build(&[&[(5.0, 2.0)], &[(1.0, 2.0)]]);
    let mut market = market(1, 2);
    warehouse.step(&mut market);
    for i in 0..2 {
        assert!(
            market.traders[i].merchandises[0].volume >= 0.0,
            "仓库挂出去的是库存本身，不应当是带方向的申报：{}",
            market.traders[i].merchandises[0].volume,
        );
    }
}

#[test]
fn a_long_run_stays_finite() {
    let mut warehouse = build(&[&[(5.0, 2.0)], &[(1.0, 4.0)], &[(20.0, 1.0)]]);
    let mut market = market(1, 3);
    for _ in 0..300 {
        warehouse.step(&mut market);
        assert_finite(&warehouse);
    }
}

#[test]
fn the_stock_moves_by_the_net_deal_volume() {
    let mut warehouse = build(&[&[(50.0, 1.0)], &[(5.0, 1.0)]]);
    warehouse.set_price_law(1.0, 0.0, 0.0);
    let mut market = market(1, 2);
    let before = volumes(&warehouse, 0);
    warehouse.step(&mut market);
    let after = volumes(&warehouse, 0);
    let net: Vec<f32> = (0..2)
        .map(|i| market.traders[i].merchandises[0].deal_volume())
        .collect();
    for i in 0..2 {
        let expected = (before[i] - net[i]).max(0.0);
        assert!(
            (after[i] - expected).abs() < 1e-3,
            "仓库 {i} 的库存应当是 {expected}，实际 {}",
            after[i],
        );
    }
}
