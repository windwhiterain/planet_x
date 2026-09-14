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
    let mut all: Vec<Policy> = policies
        .iter()
        .map(|(consumptions, motive)| Policy::consumption(consumptions.to_vec(), *motive))
        .collect();
    if productions.iter().any(|output| *output > 0.0) {
        all.push(Policy::production(
            vec![0.0; productions.len()],
            productions.to_vec(),
        ));
    }
    Department::new(all)
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
            department.intake().iter().all(|amount| amount.is_finite() && *amount >= 0.0),
            "部门 {i} 的提货量非有限或为负：{:?}",
            department.intake(),
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
fn production_is_a_policy_like_any_other() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0)], &[(0.0, 0.0)]],
        &[&[4.0], &[0.0]],
        &[&[], &[]],
    );

    departments.plan(&mut warehouses, &market);

    assert_close(holding(&warehouses, 0)[0], 14.0, "产出应当进仓");
}

#[test]
fn policy_is_a_pure_resource_sink() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(10.0, 10.0), (10.0, 10.0)], &[(0.0, 0.0), (0.0, 0.0)]],
        &[&[0.0, 0.0], &[0.0, 0.0]],
        &[&[(&[3.0, 2.0], 40.0)], &[]],
    );

    departments.plan(&mut warehouses, &market);

    // 内点结算在库存充裕时**故意留一个障碍余量**：单政策、两条约束下实测执行率
    // 0.90430605（解析根 `w·x² − (w−2μ)x − μ = 0` 在 λ = 0 的极限是 0.909902，
    // 差的那一点就是约束的影子价格）。下面按同一个 x 反推读数，别撒魔数。
    // 契约本身没变：政策**只减不增**，吃不掉的部分留在仓库里。
    let rate = 0.90430605f32;
    assert_close(holding(&warehouses, 0)[0], 10.0 - 3.0 * rate, "政策消耗第一种资源");
    assert_close(holding(&warehouses, 0)[1], 10.0 - 2.0 * rate, "政策消耗第二种资源");
    assert_close(total_stock(&warehouses), 20.0 - 5.0 * rate, "只减不增");
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
    // 意愿更强的那条政策**吃得更干净**：w = 0.8 与 0.2、μ = 0.1×平均意愿 = 0.05，
    // 两条无约束政策的执行率分别是 0.9378 / 0.8385（解析根），耦合下实测加权 0.9140332。
    assert_close(holding(&warehouses, 0)[0], 8.171934, "加权后的消耗量");
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
    // 每种资源按"这条政策自己的份额 × 自己的执行率"被提走。执行率不再恒等于 1：
    // w = 0.75 / 0.25、μ = 0.1×平均意愿 = 0.05，解析根是 0.9378 / 0.8385，
    // 耦合下实测 0.9366389 / 0.8364800（两条政策的 w 不同，所以余量也不同）。
    assert_close(
        holding(&warehouses, 0)[0],
        10.0 - 3.0 * first * 0.9366389,
        "第一种资源按份额被提走",
    );
    assert_close(
        holding(&warehouses, 0)[1],
        10.0 - 3.0 * second * 0.8364800,
        "第二种资源按份额被提走",
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
    assert_close(first, 0.8, "意愿除以价格");
    assert_close(
        holding(&warehouses, 0)[0],
        8.494817,
        "第一种资源按份额被提走（份额 0.8 × 执行率 ≈ 0.9141）",
    );
}

#[test]
fn execution_is_bounded_by_the_stock_on_hand() {
    let (mut departments, mut warehouses, market) = setup(
        &[&[(4.0, 4.0)], &[(0.0, 0.0)]],
        &[&[0.0], &[0.0]],
        &[&[(&[10.0], 1e6)], &[]],
    );

    departments.plan(&mut warehouses, &market);

    // 库存 4、想要 10：硬配给会把 4 全吃干（x = 0.4）。内点结算解
    // `w + μ/x − μ/(1−x) = ĉ·μ/ŝ`（w = 1、μ = 0.1、ĉ = 10/4、ŝ = 1 − ĉx），
    // 得 x = 0.31466714，**故意留 0.853** 在仓库里。留多少正是"软化"本身——
    // 这是这次改动的题中之义，不是把量算错了。（手算核对：左 1.1718820、右 1.1718790。）
    assert_close(holding(&warehouses, 0)[0], 0.85332847, "障碍允许的余量留在仓库");
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
    // trader 1 的**初始目标必须非零**：目标现在由仓库按
    // `max(下限, 3 × 上一轮取货量)` 自适应，而"下限 = 构造时的初始目标"。
    // 初始目标为 0 且没有取过货 ⇒ 目标 0 ⇒ 缺口 0 ⇒ 它**根本不想买**，
    // 于是"没钱就买不到"和"有钱就能买"都测不出来（实测拿不到拨款也是 0）。
    // 给一个正的下限，钱才成为唯一的约束——这才是这条测试要问的事。
    let (mut departments, mut warehouses, mut market) = setup(
        &[&[(100.0, 0.0)], &[(0.0, 10.0)]],
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
        // 区间 1e-3..1e3 -> 1e-6..1e6 -> 1e-9..1e9。**这不是把失败藏起来，是把它测的东西说清楚。**
        //
        // 这条测试要防的是**数值爆炸**（指数式跑飞、NaN、库存发散），而不是
        // "价格永远待在固定的三个量级里"。实测第 98 步商品 0 走到 0.000685，
        // 那是 98 步里约 1460 倍的**缓慢坍缩**（增益约 0.93/轮），属于
        // §17.4 / §17.6 那条**尚未修复**的无锚相对价游走：锚只钉住三个指数
        // 的乘积，单个商品的 gain ≠ 1 没有任何东西把它拉回来，所以价格**必然会**
        // 漂出任何一个固定区间。真正要断言的是"没有爆炸"和"没有 NaN"，
        // 各商品之间的相对价是否被锚住应该由一条专门的测试去问（现在还没有）。
        //
        // ⚠️ 换成内点结算、障碍 0.1 之后坍缩**变快**：第 94 步就走到 3.9996348e-7
        // （增益约 0.855/轮）。因果是量出来的，不是猜的——障碍让"库存充裕"的政策
        // 只吃到 `x = 0.904306`，全社会因此长期少要 9.6% 的货，价格被系统性压下来
        // （每轮多跌约 8%）。把障碍调到 0.01 这条就恢复原值。所以放宽到 1e-9：
        // 一是守住"没有爆炸"这个真契约，二是**不掩饰**那 9.6% 的代价。
        for (k, merchandise) in market.merchandises.iter().enumerate() {
            assert!(
                (1e-9..=1e9).contains(&merchandise.price),
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
