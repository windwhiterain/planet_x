use fastrand::Rng;

use super::{Department, Departments, Policy};
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};
use crate::warehouse::{Stock, Warehouse, Warehouses};

fn market(goods: usize, price: f32, traders: usize) -> Market {
    let merchandises = (0..goods).map(|_| Merchandise { price }).collect();
    let traders = (0..traders)
        .map(|_| Trader {
            merchandises: (0..goods)
                .map(|_| TraderMerchandise::new(0.0, 0.0))
                .collect(),
        })
        .collect();
    Market::new(merchandises, traders)
}

fn warehouses(stocks: &[&[(f32, f32)]]) -> Warehouses {
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
            })
            .collect(),
    )
    .with_fluctuation(0.0)
}

fn department(productions: &[f32], policies: &[(&[f32], f32)]) -> Department {
    Department::new(
        productions.to_vec(),
        policies
            .iter()
            .map(|(consumptions, motive)| Policy::new(consumptions.to_vec(), *motive))
            .collect(),
    )
}

fn central(productions: &[&[f32]], policies: &[&[(&[f32], f32)]]) -> Departments {
    Departments::new(
        productions
            .iter()
            .zip(policies)
            .map(|(productions, policies)| department(productions, policies))
            .collect(),
    )
}

fn setup(
    stocks: &[&[(f32, f32)]],
    productions: &[&[f32]],
    policies: &[&[(&[f32], f32)]],
) -> (Departments, Warehouses, Market) {
    let goods = stocks[0].len();
    (
        central(productions, policies),
        warehouses(stocks),
        market(goods, 1.0, stocks.len()),
    )
}

fn deterministic_rng() -> Rng {
    Rng::with_seed(0)
}

fn declaration(market: &Market, i: usize, k: usize) -> f32 {
    market.traders[i].merchandises[k].volume
}

fn holding(warehouses: &Warehouses, i: usize) -> Vec<f32> {
    warehouses.warehouses[i]
        .stocks
        .iter()
        .map(|stock| stock.volume)
        .collect()
}

fn total_stock(warehouses: &Warehouses) -> f32 {
    warehouses
        .warehouses
        .iter()
        .flat_map(|warehouse| warehouse.stocks.iter())
        .map(|stock| stock.volume)
        .sum()
}

fn total_currency(warehouses: &Warehouses) -> f32 {
    warehouses
        .warehouses
        .iter()
        .map(|warehouse| warehouse.currency)
        .sum()
}

fn assert_close(actual: f32, expected: f32, context: &str) {
    let tolerance = 1e-4 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}：期望 {expected}，实际 {actual}",
    );
}

fn assert_finite_state(departments: &Departments, warehouses: &Warehouses, market: &Market) {
    for (i, department) in departments.departments.iter().enumerate() {
        assert!(
            department.policy_execution().is_finite(),
            "部门 {i} 的执行量非有限：{}",
            department.policy_execution(),
        );
        assert!(
            warehouses.warehouses[i].currency.is_finite(),
            "部门 {i} 的货币非有限：{}",
            warehouses.warehouses[i].currency,
        );
        for (k, stock) in warehouses.warehouses[i].stocks.iter().enumerate() {
            assert!(
                stock.volume.is_finite() && stock.volume >= 0.0,
                "部门 {i} 商品 {k} 的库存非有限或为负：{}",
                stock.volume,
            );
        }
    }
    for (k, merchandise) in market.merchandises.iter().enumerate() {
        assert!(
            merchandise.price.is_finite(),
            "市场商品 {k} 的价格非有限：{}",
            merchandise.price,
        );
    }
}

#[test]
fn production_lands_in_the_indexed_warehouse_only() {
    let stocks: [&[(f32, f32)]; 3] = [&[(0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]; 3];
    let (mut departments, mut warehouses, market) = setup(
        &stocks,
        &[&[2.0, 0.0, 0.0], &[0.0, 3.0, 0.0], &[0.0, 0.0, 4.0]],
        &[&[], &[], &[]],
    );

    departments.plan(&mut warehouses, &market);

    assert_eq!(holding(&warehouses, 0), vec![2.0, 0.0, 0.0]);
    assert_eq!(holding(&warehouses, 1), vec![0.0, 3.0, 0.0]);
    assert_eq!(holding(&warehouses, 2), vec![0.0, 0.0, 4.0]);
}

#[test]
fn production_runs_without_any_policy() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0)], &[(0.0, 0.0)]],
        &[&[4.0], &[0.0]],
        &[&[], &[]],
    );

    departments.plan(&mut warehouses, &market);

    assert_close(holding(&warehouses, 0)[0], 14.0, "产出应当进仓");
    assert_close(departments.departments[0].policy_execution(), 0.0, "无政策");
}

#[test]
fn policy_is_a_pure_resource_sink() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0), (10.0, 10.0)], &[(0.0, 0.0), (0.0, 0.0)]],
        &[&[0.0, 0.0], &[0.0, 0.0]],
        &[&[(&[3.0, 2.0], 40.0)], &[]],
    );

    departments.plan(&mut warehouses, &market);

    assert_close(holding(&warehouses, 0)[0], 7.0, "政策消耗第一种资源");
    assert_close(holding(&warehouses, 0)[1], 8.0, "政策消耗第二种资源");
    assert_close(departments.departments[0].policy_execution(), 1.0, "满额执行");
    assert_close(total_stock(&warehouses), 20.0 - 5.0, "只减不增");
}

#[test]
fn the_higher_motive_policy_takes_the_larger_share() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0)], &[(0.0, 0.0)]],
        &[&[0.0], &[0.0]],
        &[&[(&[2.0], 10.0), (&[2.0], 40.0)], &[]],
    );

    departments.plan(&mut warehouses, &market);

    assert_eq!(departments.departments[0].policy_choice(), 1, "应当以意愿更强的政策为主");
    let distribution: f32 = departments.departments[0]
        .policies
        .iter()
        .map(|policy| policy.distribution())
        .sum();
    assert_close(distribution, 1.0, "分布应当归一化");
    assert_close(holding(&warehouses, 0)[0], 8.0, "加权后的消耗量");
}

#[test]
fn a_continuous_distribution_draws_from_every_policy() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0), (10.0, 10.0)], &[(0.0, 0.0), (0.0, 0.0)]],
        &[&[0.0, 0.0], &[0.0, 0.0]],
        &[&[(&[3.0, 0.0], 30.0), (&[0.0, 3.0], 10.0)], &[]],
    );

    departments.plan(&mut warehouses, &market);

    let first = departments.departments[0].policies[0].distribution();
    let second = departments.departments[0].policies[1].distribution();
    assert!(
        first > 0.0 && second > 0.0,
        "两种政策都应当分到份额：{first} / {second}",
    );
    assert_close(
        holding(&warehouses, 0)[0],
        10.0 - 3.0 * first,
        "第一种资源按份额被提走",
    );
    assert_close(
        holding(&warehouses, 0)[1],
        10.0 - 3.0 * second,
        "第二种资源按份额被提走",
    );
    assert_close(
        departments.departments[0].policy_execution(),
        1.0,
        "份额之和即执行量",
    );
}

#[test]
fn a_policy_without_motive_takes_nothing() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0)], &[(0.0, 0.0)]],
        &[&[0.0], &[0.0]],
        &[&[(&[2.0], 0.0)], &[]],
    );

    departments.plan(&mut warehouses, &market);

    assert_close(departments.departments[0].policies[0].price_potential(), 0.0, "无意愿");
    assert_close(departments.departments[0].policy_execution(), 0.0, "不应执行");
    assert_close(holding(&warehouses, 0)[0], 10.0, "库存不应被消耗");
}

#[test]
fn a_cheaper_policy_takes_the_larger_share() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0), (10.0, 10.0)], &[(0.0, 0.0), (0.0, 0.0)]],
        &[&[0.0, 0.0], &[0.0, 0.0]],
        &[&[(&[2.0, 0.0], 40.0), (&[0.0, 2.0], 40.0)], &[]],
    );
    let market = {
        let mut market = market;
        market.merchandises[0].price = 1.0;
        market.merchandises[1].price = 4.0;
        market
    };

    departments.plan(&mut warehouses, &market);

    let first = departments.departments[0].policies[0].distribution();
    let second = departments.departments[0].policies[1].distribution();
    assert!(
        first > second,
        "同样意愿下应当更偏向便宜的资源：{first} / {second}",
    );
    assert!(
        second > 0.0,
        "分布是软化过的概率，贵的那个也不应当归零：{second}",
    );
    assert!(
        first > 0.9,
        "便宜一半的资源应当拿到大头：{first}",
    );
    assert_close(holding(&warehouses, 0)[0], 10.0 - 2.0 * first, "第一种资源按份额被提走");
}

#[test]
fn execution_is_bounded_by_the_stock_on_hand() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(4.0, 4.0)], &[(0.0, 0.0)]],
        &[&[0.0], &[0.0]],
        &[&[(&[10.0], 1e6)], &[]],
    );

    departments.plan(&mut warehouses, &market);

    assert_close(departments.departments[0].policy_execution(), 0.4, "受库存约束");
    assert_close(holding(&warehouses, 0)[0], 0.0, "有多少提多少");
}

#[test]
fn the_treasury_gains_exactly_the_grants_and_nothing_is_left_over() {
    let (mut departments, mut warehouses, mut market) = setup(
        &[&[(10.0, 10.0)], &[(0.0, 0.0)], &[(0.0, 4.0)]],
        &[&[4.0], &[0.0], &[0.0]],
        &[&[], &[], &[(&[4.0], 40.0)]],
    );
    departments.grants = vec![0.0, 8.0, 12.0];
    let mut rng = deterministic_rng();

    departments.step(&mut warehouses, &mut market, &mut rng);
    assert_close(departments.treasury, 20.0, "国库应当收到全部拨款");
    assert_close(total_currency(&warehouses), 0.0, "结余应当全部收回");

    departments.step(&mut warehouses, &mut market, &mut rng);
    assert_close(departments.treasury, 40.0, "每轮都收回");
}

#[test]
fn a_grant_is_what_lets_a_warehouse_buy() {
    let (mut departments, mut warehouses, mut market) = setup(
        &[&[(100.0, 0.0)], &[(0.0, 0.0)]],
        &[&[0.0], &[0.0]],
        &[&[], &[(&[4.0], 40.0)]],
    );
    market.merchandises[0].price = 10.0;

    departments.plan(&mut warehouses, &market);
    warehouses.step(&mut market, &mut deterministic_rng());
    assert_close(declaration(&market, 1, 0), 0.0, "没有钱就买不到");

    departments.grants = vec![0.0, 1000.0];
    let mut rng = deterministic_rng();
    departments.step(&mut warehouses, &mut market, &mut rng);
    assert!(
        holding(&warehouses, 1)[0] > 0.0,
        "拿到拨款后仓库才买得动：{}",
        holding(&warehouses, 1)[0],
    );
}

#[test]
fn settlement_pays_the_warehouse_that_sold() {
    let (mut departments, mut warehouses, mut market) = setup(
        &[&[(4.0, 8.0)], &[(0.0, 4.0)]],
        &[&[6.0], &[0.0]],
        &[&[], &[(&[4.0], 40.0)]],
    );
    warehouses.warehouses[1].currency = 100.0;
    let before = total_currency(&warehouses);

    departments.plan(&mut warehouses, &market);
    warehouses.step(&mut market, &mut deterministic_rng());
    departments.settle(&mut warehouses, &market);

    assert_close(
        total_currency(&warehouses),
        before,
        "国内结算不创造也不销毁货币",
    );
    assert!(
        warehouses.warehouses[0].currency > 0.0,
        "卖出者应当收到钱：{}",
        warehouses.warehouses[0].currency,
    );
    assert!(
        warehouses.warehouses[1].currency < 100.0,
        "买入者应当付钱：{}",
        warehouses.warehouses[1].currency,
    );
}

fn classic() -> (Departments, Warehouses, Market) {
    let a: &[f32] = &[4.0, 0.0, 0.0];
    let b: &[f32] = &[0.0, 4.0, 0.0];
    let c: &[f32] = &[0.0, 0.0, 4.0];
    let good0: &[f32] = &[4.0, 0.0, 0.0];
    let good1: &[f32] = &[0.0, 4.0, 0.0];
    let good2: &[f32] = &[0.0, 0.0, 4.0];

    let departments = vec![
        department(a, &[(good1, 20.0), (good2, 20.0)]),
        department(b, &[(good0, 20.0), (good2, 20.0)]),
        department(c, &[(good0, 20.0), (good1, 20.0)]),
    ];
    let quotes = [
        &[(0.0, 0.0), (4.0, 2.0), (4.0, 2.0)][..],
        &[(4.0, 2.0), (0.0, 0.0), (4.0, 2.0)][..],
        &[(4.0, 2.0), (4.0, 2.0), (0.0, 0.0)][..],
    ];

    (
        Departments::new(departments).with_grants(vec![10.0, 10.0, 10.0]),
        warehouses(&quotes),
        market(3, 1.0, 3),
    )
}

#[test]
fn the_classic_three_departments_pay_each_other_in_a_ring() {
    let (mut departments, mut warehouses, mut market) = classic();

    departments.step(&mut warehouses, &mut market, &mut deterministic_rng());

    let net = |i: usize| -> Vec<f32> {
        (0..3)
            .map(|k| declaration(&market, i, k))
            .collect::<Vec<f32>>()
    };
    for i in 0..3 {
        for k in 0..3 {
            let declared = declaration(&market, i, k);
            if k == i {
                assert_close(declared, 4.0, &format!("部门 {i} 应当卖出自己的商品"));
            } else {
                assert!(
                    declared < 0.0,
                    "部门 {i} 应当买入商品 {k}，实际申报 {declared}",
                );
            }
        }
        assert_close(
            net(i)[(i + 1) % 3],
            net(i)[(i + 2) % 3],
            &format!("部门 {i} 的两笔买入应当对称"),
        );
    }

    for k in 0..3 {
        let dealt: f32 = (0..3).map(|i| declaration(&market, i, k).max(0.0)).sum();
        assert_close(dealt, 4.0, &format!("商品 {k} 的出清量"));
    }
}

#[test]
fn the_classic_ring_keeps_the_books_balanced_over_a_long_run() {
    let (mut departments, mut warehouses, mut market) = classic();
    let mut rng = deterministic_rng();
    let opening = total_stock(&warehouses);

    for step in 0..120 {
        departments.step(&mut warehouses, &mut market, &mut rng);
        assert_finite_state(&departments, &warehouses, &market);
        assert_close(
            departments.treasury,
            30.0 * (step + 1) as f32,
            &format!("第 {step} 步国库"),
        );
        for (i, warehouse) in warehouses.warehouses.iter().enumerate() {
            for (k, stock) in warehouse.stocks.iter().enumerate() {
                assert!(
                    stock.volume >= 0.0 && stock.volume <= 1e4,
                    "第 {step} 步部门 {i} 商品 {k} 的库存 {} 失控",
                    stock.volume,
                );
            }
        }
        let circulation = total_stock(&warehouses);
        assert!(
            (0.0..=opening * 1e3).contains(&circulation),
            "第 {step} 步社会总库存 {circulation} 失控",
        );
        for (k, merchandise) in market.merchandises.iter().enumerate() {
            assert!(
                (1e-3..=1e3).contains(&merchandise.price),
                "第 {step} 步商品 {k} 的价格 {} 失控",
                merchandise.price,
            );
        }
    }
}

#[test]
fn a_run_is_deterministic() {
    let stocks = [&[(4.0, 8.0), (1.0, 0.0)][..], &[(1.0, 0.0), (4.0, 8.0)][..]];
    let productions: [&[f32]; 2] = [&[6.0, 0.0], &[0.0, 6.0]];
    let policies_0: &[(&[f32], f32)] = &[(&[2.0, 1.0], 20.0)];
    let policies_1: &[(&[f32], f32)] = &[(&[1.0, 2.0], 18.0)];
    let policies: [&[(&[f32], f32)]; 2] = [policies_0, policies_1];

    let run = |seed: u64| {
        let (mut departments, mut warehouses, mut market) = setup(&stocks, &productions, &policies);
        departments.grants = vec![20.0, 20.0];
        let mut rng = Rng::with_seed(seed);
        for _ in 0..40 {
            departments.step(&mut warehouses, &mut market, &mut rng);
        }
        (
            holding(&warehouses, 0),
            holding(&warehouses, 1),
            market.merchandises[0].price,
            market.merchandises[1].price,
            departments.treasury,
        )
    };

    assert_eq!(run(7), run(7), "同一种子应当复现同一条路径");
}
