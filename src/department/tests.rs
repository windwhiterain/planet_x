use super::{Department, Departments, Policy};
use crate::market::{Market, Merchandise, Trader, TraderMerchandise};
use crate::warehouse::{Stock, Warehouse, Warehouses};

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
                .with_reference(vec![1.0; goods.len()])
            })
            .collect(),
    )
}

fn setup(
    stocks: &[&[(f32, f32)]],
    policies: &[Vec<Policy>],
) -> (Departments, Warehouses, Market) {
    let goods = stocks[0].len();
    let departments = Departments::new(
        policies
            .iter()
            .map(|policies| {
                Department::new(
                    policies
                        .iter()
                        .map(|policy| {
                            if policy.is_production() {
                                Policy::production(
                                    policy.consumptions.clone(),
                                    policy.outputs.clone(),
                                )
                                .with_capacity_cost(policy.capacity_cost)
                            } else {
                                Policy::consumption(policy.consumptions.clone(), policy.motive)
                            }
                        })
                        .collect(),
                )
            })
            .collect(),
    );
    (
        departments,
        warehouses(stocks),
        market(goods, stocks.len()),
    )
}

fn consumption(good: usize, goods: usize, motive: f32) -> Policy {
    let mut consumptions = vec![0.0; goods];
    consumptions[good] = 1.0;
    Policy::consumption(consumptions, motive)
}

fn intake(departments: &Departments, i: usize) -> Vec<f32> {
    departments.departments[i].intake().to_vec()
}

fn total_stock(warehouses: &Warehouses) -> f32 {
    warehouses
        .warehouses
        .iter()
        .flat_map(|warehouse| warehouse.stocks.iter())
        .map(|stock| stock.volume)
        .sum()
}

#[test]
fn a_cheaper_policy_takes_the_larger_share() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 1.0), (10.0, 1.0)]],
        &[vec![consumption(0, 2, 1.0), consumption(1, 2, 1.0)]],
    );
    warehouses.warehouses[0].stocks[0].price = 1.0;
    warehouses.warehouses[0].stocks[1].price = 4.0;
    departments.plan(&mut warehouses, &market);
    let taken = intake(&departments, 0);
    assert!(
        taken[0] > taken[1],
        "同样的意愿下便宜的那种应当吃得更多：{taken:?}",
    );
}

#[test]
fn the_higher_motive_policy_takes_the_larger_share() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 1.0), (10.0, 1.0)]],
        &[vec![consumption(0, 2, 1.0), consumption(1, 2, 3.0)]],
    );
    warehouses.warehouses[0].stocks[0].price = 1.0;
    warehouses.warehouses[0].stocks[1].price = 1.0;
    departments.plan(&mut warehouses, &market);
    let taken = intake(&departments, 0);
    assert!(
        taken[1] > taken[0],
        "意愿更强的那种应当吃得更多：{taken:?}",
    );
}

#[test]
fn execution_is_bounded_by_the_stock_on_hand() {
    let (mut departments, mut warehouses, market) =
        setup(&[&[(3.0, 1.0)]], &[vec![consumption(0, 1, 1e6)]]);
    departments.plan(&mut warehouses, &market);
    let taken = intake(&departments, 0);
    assert!(
        taken[0] <= 3.0 + 1e-4,
        "提货不能超过货架上的量：{}",
        taken[0],
    );
}

#[test]
fn the_cash_row_limits_consumption() {
    let stocks: &[&[(f32, f32)]] = &[&[(100.0, 1.0)]];
    let (mut poor, mut poor_stocks, poor_market) =
        setup(stocks, &[vec![consumption(0, 1, 1e6)]]);
    let (mut rich, mut rich_stocks, rich_market) =
        setup(stocks, &[vec![consumption(0, 1, 1e6)]]);
    poor_stocks.warehouses[0].stocks[0].price = 2.0;
    rich_stocks.warehouses[0].stocks[0].price = 2.0;
    poor.departments[0].currency = 3.0;
    rich.departments[0].currency = 300.0;
    poor.plan(&mut poor_stocks, &poor_market);
    rich.plan(&mut rich_stocks, &rich_market);
    let poor_take = intake(&poor, 0)[0];
    let rich_take = intake(&rich, 0)[0];
    assert!(
        poor_take < rich_take,
        "钱少的应当买得少：{poor_take} vs {rich_take}",
    );
    assert!(
        poor_take <= 1.5 + 1e-3,
        "3 块钱按 2 元一件最多买 1.5 件：{poor_take}",
    );
}

#[test]
fn a_producer_earns_and_a_consumer_spends() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(0.0, 0.0)], &[(20.0, 1.0)]],
        &[
            vec![Policy::production(vec![0.0], vec![10.0])],
            vec![consumption(0, 1, 10.0)],
        ],
    );
    for i in 0..2 {
        departments.departments[i].currency = 100.0;
    }
    warehouses.warehouses[0].stocks[0].price = 2.0;
    warehouses.warehouses[1].stocks[0].price = 2.0;
    departments.plan(&mut warehouses, &market);
    let before: Vec<f32> = departments
        .departments
        .iter()
        .map(|department| department.currency)
        .collect();
    departments.settle(&warehouses);
    let after: Vec<f32> = departments
        .departments
        .iter()
        .map(|department| department.currency)
        .collect();
    assert!(after[0] > before[0], "生产者应当挣到钱：{before:?} -> {after:?}");
    assert!(after[1] < before[1], "消费者应当花钱：{before:?} -> {after:?}");
}

#[test]
fn the_transfer_equalizes_balances() {
    let (mut departments, _, _) =
        setup(&[&[(1.0, 1.0)], &[(1.0, 1.0)]], &[vec![], vec![]]);
    departments.departments[0].currency = 100.0;
    departments.departments[1].currency = 0.0;
    departments.transfer(1.0);
    let a = departments.departments[0].currency;
    let b = departments.departments[1].currency;
    assert!((a - 50.0).abs() < 1e-3 && (b - 50.0).abs() < 1e-3, "{a} {b}");
}

#[test]
fn the_transfer_conserves_the_total() {
    let (mut departments, _, _) =
        setup(&[&[(1.0, 1.0)], &[(1.0, 1.0)]], &[vec![], vec![]]);
    departments.departments[0].currency = 7.0;
    departments.departments[1].currency = 3.0;
    departments.transfer(0.5);
    let total: f32 = departments
        .departments
        .iter()
        .map(|department| department.currency)
        .sum();
    assert!((total - 10.0).abs() < 1e-4, "总额应当守恒：{total}");
}

#[test]
fn the_money_stock_is_held_at_its_target() {
    let (mut departments, _, _) =
        setup(&[&[(1.0, 1.0)], &[(1.0, 1.0)]], &[vec![], vec![]]);
    departments.departments[0].currency = 1000.0;
    departments.departments[1].currency = 1.0;
    departments.money_target = 200.0;
    departments.normalize();
    let total: f32 = departments
        .departments
        .iter()
        .map(|department| department.currency)
        .sum();
    assert!((total - 200.0).abs() < 1e-2, "总量应当被拉回目标：{total}");
    assert!(
        departments.departments[0].currency > departments.departments[1].currency,
        "按比例缩放必须保留相对份额",
    );
}

#[test]
fn a_long_run_conserves_mass() {
    let (mut departments, mut warehouses, mut market) = setup(
        &[&[(40.0, 20.0), (40.0, 20.0)], &[(40.0, 20.0), (40.0, 20.0)]],
        &[
            vec![Policy::production(vec![0.0, 0.0], vec![8.0, 0.0])],
            vec![consumption(0, 2, 10.0)],
        ],
    );
    for department in departments.departments.iter_mut() {
        department.currency = 1000.0;
    }
    departments.money_target = 2000.0;
    for _ in 0..200 {
        let before = total_stock(&warehouses);
        departments.step(&mut warehouses, &mut market);
        let produced: f32 = departments
            .departments
            .iter()
            .flat_map(|department| department.delivery().iter())
            .sum();
        let consumed: f32 = departments
            .departments
            .iter()
            .flat_map(|department| department.intake().iter())
            .sum();
        let after = total_stock(&warehouses);
        let residual = (after - before) - (produced - consumed);
        assert!(
            residual.abs() < 1e-2,
            "质量不守恒：Δ库存 {} 对 产 {produced} − 耗 {consumed}",
            after - before,
        );
        assert!(after.is_finite(), "库存非有限");
    }
}

#[test]
fn a_zero_cash_department_still_runs_without_a_cash_row() {
    let (mut departments, mut warehouses, market) =
        setup(&[&[(10.0, 1.0)]], &[vec![consumption(0, 1, 10.0)]]);
    departments.departments[0].currency = 0.0;
    warehouses.warehouses[0].stocks[0].price = 1.0;
    departments.plan(&mut warehouses, &market);
    let taken = intake(&departments, 0);
    assert!(
        taken[0] > 0.0,
        "现金上限为 0 时不加现金行，消费仍然由存量决定：{taken:?}",
    );
}
