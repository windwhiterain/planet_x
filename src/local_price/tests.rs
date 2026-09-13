use super::*;

fn assert_close(actual: f32, expected: f32, tolerance: f32, context: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}：期望 {expected}±{tolerance}，实际 {actual}",
    );
}

fn symmetric() -> Spec {
    Spec::symmetric(3)
}

fn scarce() -> Spec {
    Spec::scarce(3, 0, 0)
}

#[test]
fn fixed_levels_keep_every_wedge_at_zero() {
    let mut lab = Lab::new(&symmetric(), 11).with_rule(LevelRule::Fixed);
    lab.run(60);

    for polity in &lab.polities {
        for k in 0..GOODS {
            assert_eq!(polity.wedge[k], 0.0, "固定水平下不应当有楔子");
        }
    }
    for good in 0..GOODS {
        assert!(lab.market.merchandises[good].price > 0.0);
        assert!(lab.market.merchandises[good].price.is_finite());
    }
}

#[test]
fn the_price_level_has_no_anchor_of_its_own() {
    let mut free = Lab::new(&symmetric(), 11)
        .with_rule(LevelRule::Fixed)
        .with_anchor(false);
    let mut excursion = 0.0f32;
    for _ in 0..80 {
        free.step();
        let snapshot = free.history.last().unwrap();
        for k in 0..GOODS {
            excursion = excursion.max((snapshot.prices[k] / BASE_PRICE).ln().abs());
        }
    }
    assert!(
        excursion > 0.2,
        "没有任何锚时价格水平应当四处游走，实际最大偏离 {excursion}",
    );

    let mut anchored = Lab::new(&symmetric(), 11)
        .with_rule(LevelRule::Fixed)
        .with_anchor(true);
    for _ in 0..80 {
        anchored.step();
        let snapshot = anchored.history.last().unwrap();
        let basket = snapshot
            .prices
            .iter()
            .map(|price| price.max(1e-9).ln())
            .sum::<f32>()
            / GOODS as f32;
        assert_close(basket.exp(), BASE_PRICE, 1e-3, "锚应当把篮子钉在 1");
    }
}

#[test]
fn the_anchor_leaves_the_relative_premium_of_a_scarce_good() {
    let mut lab = Lab::new(&scarce(), 11).with_rule(LevelRule::Fixed);
    lab.run(60);

    let prices = lab
        .history
        .last()
        .unwrap()
        .prices
        .clone();
    assert!(
        prices[0] > prices[1] * 1.05,
        "稀缺商品的相对价应当明显更高：{prices:?}",
    );
    assert!(
        prices[0] > BASE_PRICE && prices[1] < BASE_PRICE,
        "稀缺商品应当贵于篮子、其余便宜于篮子：{prices:?}",
    );
}

#[test]
fn a_learned_wedge_runs_away_without_a_real_anchor() {
    for rule in [
        LevelRule::OwnVwap,
        LevelRule::Counterparty,
        LevelRule::Shortfall,
    ] {
        let mut lab = Lab::new(&scarce(), 11).with_rule(rule);
        lab.run(60);
        let worst = lab
            .wedges()
            .iter()
            .flatten()
            .fold(0.0f32, |worst, wedge| worst.max(wedge.abs()));
        assert!(
            worst > 0.5,
            "{} 规则在没有任何真实锚时会一路跑到钳位上，实际最大楔子 {worst}",
            rule.name(),
        );
    }

    let mut control = Lab::new(&scarce(), 11).with_rule(LevelRule::Fixed);
    control.run(60);
    let worst = control
        .wedges()
        .iter()
        .flatten()
        .fold(0.0f32, |worst, wedge| worst.max(wedge.abs()));
    assert_eq!(worst, 0.0, "对照组不应当动");
}

#[test]
fn a_wedge_that_feeds_its_own_quote_drives_the_raw_level_down() {
    let mut free = Lab::new(&symmetric(), 11)
        .with_rule(LevelRule::Fixed)
        .with_anchor(false)
        .with_grant(10.0);
    free.run(40);
    let quiet = free
        .history
        .iter()
        .map(|snapshot| snapshot.gauge.iter().sum::<f32>() / GOODS as f32)
        .sum::<f32>()
        / free.history.len() as f32;

    let mut loud = Lab::new(&symmetric(), 11)
        .with_rule(LevelRule::Counterparty)
        .with_anchor(true)
        .with_grant(10.0);
    loud.run(40);
    let stirred = loud
        .history
        .iter()
        .skip(1)
        .map(|snapshot| snapshot.gauge.iter().sum::<f32>() / GOODS as f32)
        .fold(0.0f32, |worst, drift| worst.max(drift.abs()));

    assert!(
        stirred > quiet.abs(),
        "把楔子接进报价后，原始水平漂移应当被放大：平静 {quiet}，接上 {stirred}",
    );
}

#[test]
fn relations_cut_the_cross_border_volume() {
    let mut open = Lab::new(&scarce(), 11)
        .with_rule(LevelRule::Fixed)
        .with_relations(&bloc_relations(3, 1.0));
    open.run(30);
    let snapshot = open.history.last().unwrap();
    assert!(snapshot.external > 0.0, "权重为 1 时应当有跨境成交");
    assert!(snapshot.internal > 0.0, "政权内部也应当有成交");

    let mut shut = Lab::new(&scarce(), 11)
        .with_rule(LevelRule::Fixed)
        .with_relations(&bloc_relations(3, 0.0));
    shut.run(30);
    let snapshot = shut.history.last().unwrap();
    assert_eq!(snapshot.external, 0.0, "权重为 0 时跨境成交应当归零");
    assert!(snapshot.internal > 0.0, "封锁不应当停掉政权内部的成交");
}

fn transformation(rate: f32, scale: f32) -> Spec {
    let mut inputs = vec![0.0; GOODS];
    let mut outputs = vec![0.0; GOODS];
    inputs[1] = rate * scale;
    outputs[0] = scale;
    scarce().with_transform(0, 0, inputs, outputs)
}

fn premium(lab: &Lab) -> f32 {
    let prices = &lab.history.last().unwrap().prices;
    prices[0] / prices[1]
}

#[test]
fn a_transformation_runs_only_while_it_pays() {
    let mut paying = Lab::new(&transformation(1.0, 4.0), 11).with_rule(LevelRule::Fixed);
    paying.run(40);
    assert_eq!(
        paying.history.last().unwrap().transform_share,
        1.0,
        "1 单位工业品换 1 单位粮食在溢价下应当开工",
    );

    let mut losing = Lab::new(&transformation(3.0, 4.0), 11).with_rule(LevelRule::Fixed);
    losing.run(40);
    assert_eq!(
        losing.history.last().unwrap().transform_share,
        0.0,
        "3 换 1 亏本时不应当开工",
    );
}

#[test]
fn a_running_transformation_shrinks_the_scarcity_premium() {
    let mut idle = Lab::new(&scarce(), 11).with_rule(LevelRule::Fixed);
    idle.run(60);
    let mut working = Lab::new(&transformation(1.0, 4.0), 11).with_rule(LevelRule::Fixed);
    working.run(60);

    assert!(
        premium(&idle) > 1.05,
        "没有转换时稀缺品应当有溢价：{}",
        premium(&idle),
    );
    assert!(
        premium(&working) < 0.5 * premium(&idle) + 0.5,
        "转换开工后稀缺溢价应当显著收敛：闲置 {} 开工 {}",
        premium(&idle),
        premium(&working),
    );
}

#[test]
fn a_transformation_competes_with_consumption_for_its_input() {
    let mut idle = Lab::new(&scarce(), 11).with_rule(LevelRule::Fixed);
    idle.run(40);
    let mut working = Lab::new(&transformation(1.0, 4.0), 11).with_rule(LevelRule::Fixed);
    working.run(40);

    let idle_input = idle.history.last().unwrap().prices[1];
    let working_input = working.history.last().unwrap().prices[1];
    assert!(
        working_input > idle_input,
        "转换吃掉投入品后，投入品的相对价应当上升：闲置 {idle_input} 开工 {working_input}",
    );
}

#[test]
fn the_lab_is_reproducible() {
    let run = |seed: u64| {
        let mut lab = Lab::new(&scarce(), seed)
            .with_rule(LevelRule::Counterparty)
            .with_recenter(true);
        lab.run(40);
        lab.wedges()
    };
    assert_eq!(run(7), run(7), "同一种子应当复现");
}
