use game::{DEPARTMENTS, DomesticEconomy, GOODS};

fn assert_close(actual: f32, expected: f32, context: &str) {
    let tolerance = 1e-3 * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}：期望 {expected}，实际 {actual}",
    );
}

#[test]
fn every_department_sells_one_good_and_buys_the_other_two() {
    let mut economy = DomesticEconomy::new(11);
    economy.step();

    for i in 0..DEPARTMENTS {
        for k in 0..GOODS {
            let net = economy.market.traders[i].merchandises[k].deal_volume();
            let expected = if i == k { 4.0 } else { -2.0 };
            assert_close(net, expected, &format!("部门 {i} 商品 {k} 的净成交"));
        }
    }
}

#[test]
fn production_is_fully_consumed_by_the_other_departments() {
    let mut economy = DomesticEconomy::new(11);

    for round in 0..60 {
        economy.step();
        for k in 0..GOODS {
            let produced = 4.0;
            let consumed: f32 = (0..DEPARTMENTS)
                .filter(|department| *department != k)
                .map(|department| {
                    economy.departments.departments[department]
                        .policies
                        .iter()
                        .map(|policy| {
                            policy.distribution()
                                * economy.departments.departments[department].policy_execution()
                                * policy.consumptions[k]
                        })
                        .sum::<f32>()
                })
                .sum();
            assert_close(consumed, produced, &format!("第 {round} 轮商品 {k} 的消耗"));
        }
    }
}

#[test]
fn the_treasury_collects_exactly_the_grants() {
    let mut economy = DomesticEconomy::new(7);

    for round in 0..80 {
        economy.step();
        let grants: f32 = economy
            .departments
            .grants
            .iter()
            .sum::<f32>()
            * (round + 1) as f32;
        assert_close(economy.departments.treasury, grants, &format!("第 {round} 轮国库"));
        for warehouse in &economy.warehouses.warehouses {
            assert_close(warehouse.currency, 0.0, &format!("第 {round} 轮结余"));
        }
    }
}

#[test]
fn the_circulation_settles_into_a_fixed_point() {
    let mut economy = DomesticEconomy::new(11);
    economy.run(60);
    let settled = economy.history[60].clone();
    let settled_price = settled.prices;
    let settled_stock: f32 = settled.holdings.iter().flatten().sum::<f32>();
    let mut floor = settled.cpi;

    for round in 61..120 {
        economy.step();
        let snapshot = &economy.history[round];
        floor = floor.min(snapshot.cpi);
        assert_close(
            snapshot.holdings.iter().flatten().sum::<f32>(),
            settled_stock,
            &format!("第 {round} 轮社会总库存"),
        );
        for k in 0..GOODS {
            assert!(
                snapshot.prices[k] > 0.0 && snapshot.prices[k].is_finite(),
                "第 {round} 轮商品 {k} 的价格 {} 失控",
                snapshot.prices[k],
            );
            assert!(
                snapshot.prices[k] <= settled_price[k] * 1.5,
                "第 {round} 轮商品 {k} 的价格 {} 反弹过高",
                snapshot.prices[k],
            );
            assert_close(
                snapshot.prices[k],
                settled_price[k],
                &format!("第 {round} 轮商品 {k} 的价格"),
            );
        }
        assert_close(
            snapshot.cpi,
            settled.cpi,
            &format!("第 {round} 轮物价指数"),
        );
    }
    assert!(
        floor >= 0.5 * settled.cpi,
        "收敛前的低谷 {floor} 相对稳态 {} 过深",
        settled.cpi,
    );
}

#[test]
fn the_first_rounds_swing_once_before_the_fixed_point() {
    let mut economy = DomesticEconomy::new(11);
    economy.run(9);
    let trough = economy.history[9].cpi;
    economy.run(41);

    assert!(
        trough < economy.history[50].cpi,
        "价格应当先沉到一个低谷再回到稳态：低谷 {trough}，稳态 {}",
        economy.history[50].cpi,
    );
    assert!(
        trough >= 0.5 * economy.history[50].cpi,
        "低谷 {trough} 相对稳态 {} 过深",
        economy.history[50].cpi,
    );
}

#[test]
fn the_run_is_reproducible() {
    let mut first = DomesticEconomy::new(3);
    let mut second = DomesticEconomy::new(3);
    first.run(30);
    second.run(30);

    assert_eq!(first.history, second.history, "同一种子应当复现同一条路径");
}
