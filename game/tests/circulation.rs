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

    // ⚠️ 报价带宽收窄到 1.0（planet_x §7.3）之后，前 5 轮是一次**启动过冲**：部门先吃掉
    // 一部分初始库存（第 0 轮 intake 7.31），第 6 轮起流量就精确回到产量 4 并一直保持
    // （实测到 400 轮）。所以这条"生产被完全消耗"按**稳态**断言：先空跑 10 轮预热，再查。
    economy.run(10);
    for round in 10..60 {
        economy.step();
        for k in 0..GOODS {
            let produced = 4.0;
            // 直接用**实际提货量**：`distribution × 执行率 × 配方` 那个口径随
            // `distribution` 归一化与 `execution` 一起删掉了，而实际流量本来就该这么测。
            let consumed: f32 = (0..DEPARTMENTS)
                .filter(|department| *department != k)
                .map(|department| economy.departments.departments[department].intake()[k])
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
    // ⚠️ 报价带宽 1.0 之后定点出现得更晚：第 60 轮价格还在 1.8922 上行，第 ~75 轮才落到
    // 1.913579 并**逐位停住**（实测到 400 轮不变；旧带宽下定点是 1.900542、第 60 轮已到）。
    // 所以热身 60 -> 120、窗口 61..120 -> 121..180。
    economy.run(120);
    let settled = economy.history[120].clone();
    let settled_price = settled.prices;
    let settled_stock: f32 = settled.holdings.iter().flatten().sum::<f32>();
    let mut floor = settled.cpi;

    for round in 121..180 {
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
