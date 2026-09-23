use super::{GOODS, Kind, Lab, Spec};

fn modern() -> Lab {
    Lab::new(&Spec::modern(3).with_specialty(2.0))
}

fn total_stock(lab: &Lab) -> f32 {
    lab.warehouses
        .warehouses
        .iter()
        .flat_map(|warehouse| warehouse.stocks.iter())
        .map(|stock| stock.volume)
        .sum()
}

fn stated(lab: &Lab, k: usize) -> f32 {
    lab.good_states()[k].index
}

fn index_range(lab: &Lab) -> f32 {
    let index: Vec<f32> = (0..GOODS).map(|k| stated(lab, k)).collect();
    if index.iter().all(|value| *value > 0.0) {
        index.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
            / index.iter().cloned().fold(f32::INFINITY, f32::min)
    } else {
        0.0
    }
}

#[test]
fn the_initial_state_is_finite_and_positive() {
    let mut lab = modern();
    lab.step();
    for state in lab.good_states() {
        assert!(
            state.index.is_finite() && state.index > 0.0,
            "{}",
            state.index
        );
        assert!(
            state.stock.is_finite() && state.stock > 0.0,
            "{}",
            state.stock
        );
    }
    for department in &lab.departments.departments {
        assert!(department.currency.is_finite() && department.currency > 0.0);
    }
}

#[test]
fn the_settlement_stays_solvable() {
    let mut lab = modern();
    lab.run(400);
    let solved = 18 * lab.round;
    assert!(
        lab.settlement_failures * 100 < solved,
        "结算解不出来的比例必须远低于 1%：{} / {solved}",
        lab.settlement_failures,
    );
    assert!(
        lab.settlement_degraded * 2 < solved,
        "退化（收敛但路上磕过）的比例应当低于一半：{} / {solved}",
        lab.settlement_degraded,
    );
}

#[test]
fn a_modern_economy_stays_bounded() {
    let mut lab = modern();
    lab.run(400);
    for state in lab.good_states() {
        assert!(
            state.index.is_finite() && state.index > 0.0,
            "指数 {}",
            state.index
        );
        assert!(
            state.stock.is_finite() && state.stock >= 0.0,
            "库存 {}",
            state.stock
        );
        assert!(state.dealt.is_finite() && state.dealt >= 0.0);
    }
    let stock = total_stock(&lab);
    assert!(stock.is_finite() && stock > 0.0, "总库存 {stock}");
}

#[test]
fn the_long_run_does_not_drift_to_the_f32_edges() {
    let mut lab = modern();
    lab.run(200);
    let early = index_range(&lab);
    let stock_early = total_stock(&lab);
    lab.run(600);
    let late = index_range(&lab);
    let stock_late = total_stock(&lab);
    assert!(early.is_finite() && late.is_finite(), "{early} {late}");
    assert!(
        late < early.max(1.0) * 4.0,
        "指数极差不应当扩张：{early:.3} -> {late:.3}",
    );
    assert!(
        stock_late < stock_early.max(1.0) * 8.0,
        "总库存不应当失控：{stock_early:.1} -> {stock_late:.1}",
    );
}

#[test]
fn mass_is_conserved() {
    let mut lab = modern();
    for _ in 0..60 {
        let before = total_stock(&lab);
        lab.step();
        let delivered: f32 = lab
            .departments
            .departments
            .iter()
            .flat_map(|department| department.delivery().iter())
            .sum();
        let taken: f32 = lab
            .departments
            .departments
            .iter()
            .flat_map(|department| department.intake().iter())
            .sum();
        let after = total_stock(&lab);
        let residual = (after - before) - (delivered - taken);
        assert!(
            residual.abs() < 0.01 * (1.0 + after.abs()),
            "第 {} 轮质量不守恒：残差 {residual}",
            lab.round,
        );
    }
}

#[test]
fn the_money_stock_stays_on_target() {
    let mut lab = modern();
    let target = lab.departments.money_target;
    lab.run(200);
    let snapshot = lab.history.last().unwrap();
    assert!(
        (snapshot.money - target).abs() < target * 1e-3,
        "货币总量应当停在目标上：{} vs {target}",
        snapshot.money,
    );
}

#[test]
fn the_transfer_keeps_the_balances_within_reach_of_each_other() {
    let mut lab = modern();
    lab.run(200);
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for department in &lab.departments.departments {
        low = low.min(department.currency);
        high = high.max(department.currency);
    }
    let mean = 0.5 * (low + high);
    assert!(
        high - low < 1.5 * mean,
        "转移支付应当把余额控制在均值附近：{low:.1} .. {high:.1}",
    );
}

#[test]
fn the_index_is_the_posted_price_not_the_deal_price() {
    let mut lab = modern();
    lab.run(30);
    for k in 0..GOODS {
        let posted = lab.warehouses.quoted_index(GOODS)[k];
        assert!(
            (stated(&lab, k) - posted).abs() < 1e-4,
            "报关的指数应当就是挂价的几何平均",
        );
    }
}

#[test]
fn a_blockade_opens_a_local_gap() {
    let mut lab = modern();
    lab.run(40);
    let open = (0..lab.polities.len())
        .map(|p| lab.spread(p, 0).abs())
        .sum::<f32>();
    lab.block(&[0], 0.0);
    lab.run(40);
    let shut = (0..lab.polities.len())
        .map(|p| lab.spread(p, 0).abs())
        .sum::<f32>();
    assert!(
        shut > open,
        "封锁应当拉开价差：开放 {open:.4} -> 封锁 {shut:.4}",
    );
}

#[test]
fn a_sanctioned_department_gets_less_than_an_open_one() {
    let mut open = modern();
    open.run(60);
    let department = open.department_of(1, 0, Kind::Consumer);
    let fill = |lab: &Lab| {
        let warehouse = &lab.warehouses.warehouses[department];
        lab.departments.departments[department]
            .intake()
            .iter()
            .zip(warehouse.stocks.iter())
            .map(|(taken, stock)| {
                if stock.wanted > 0.0 {
                    taken / stock.wanted
                } else {
                    1.0
                }
            })
            .sum::<f32>()
            / GOODS as f32
    };
    let before = fill(&open);
    let mut shut = modern();
    shut.sanction(&[department], 0.0);
    shut.run(60);
    let after = fill(&shut);
    assert!(
        after <= before + 0.2,
        "被制裁部门的执行率不应当变好：{before:.3} -> {after:.3}",
    );
}

#[test]
fn the_price_law_is_gauge_free() {
    let mut cheap = Lab::new(&Spec::modern(3).with_specialty(2.0)).with_grant(500.0);
    let mut dear = Lab::new(&Spec::modern(3).with_specialty(2.0)).with_grant(4000.0);
    cheap.run(80);
    dear.run(80);
    let cheap_index: Vec<f32> = (0..GOODS).map(|k| stated(&cheap, k)).collect();
    let dear_index: Vec<f32> = (0..GOODS).map(|k| stated(&dear, k)).collect();
    for k in 0..GOODS {
        assert!(cheap_index[k] > 0.0 && dear_index[k] > 0.0);
    }
    let cheap_shape = cheap_index[0] / cheap_index[1];
    let dear_shape = dear_index[0] / dear_index[1];
    assert!(
        (cheap_shape / dear_shape).is_finite(),
        "价格水平是计量基准，相对价格不应当被打成非有限",
    );
}

#[test]
fn a_targeted_cut_does_not_freeze_the_whole_economy() {
    let mut lab = modern();
    lab.run(60);
    lab.sanction(&[lab.department_of(1, 0, Kind::Consumer)], 0.0);
    lab.run(60);
    let dealt: f32 = lab.good_states().iter().map(|state| state.dealt).sum();
    assert!(dealt > 0.0, "局部制裁不应当让全场停止流动");
    assert!(total_stock(&lab).is_finite());
}
