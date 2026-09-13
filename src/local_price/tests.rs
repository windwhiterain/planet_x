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
    let mut paying = Lab::new(&transformation(0.5, 4.0), 11).with_rule(LevelRule::Fixed);
    paying.run(40);
    assert_eq!(
        paying.history.last().unwrap().transform_share,
        1.0,
        "半件工业品换一件粮食在溢价下应当开工",
    );

    let mut losing = Lab::new(&transformation(3.0, 4.0), 11).with_rule(LevelRule::Fixed);
    losing.run(40);
    assert_eq!(
        losing.history.last().unwrap().transform_share,
        0.0,
        "三件工业品换一件粮食亏本时不应当开工",
    );
}

#[test]
fn a_process_the_index_would_shut_runs_on_local_prices() {
    let mut inputs = vec![0.0; GOODS];
    let mut outputs = vec![0.0; GOODS];
    inputs[0] = 2.8;
    outputs[1] = 4.0;
    let spec = scarce().with_transform(1, 1, inputs, outputs);
    let mut lab = Lab::new(&spec, 11).with_rule(LevelRule::Fixed);
    let department = lab.department_of(1, 0);
    for round in 0..120 {
        if round >= 40 {
            lab.sanction(&[department], 0.0);
        } else {
            lab.unsanction();
        }
        lab.step();
    }

    let frame = &lab.history[lab.history.len() - 2];
    let snapshot = lab.history.last().unwrap();
    assert_eq!(
        snapshot.transform_share, 1.0,
        "本地粮价便宜到足以点着这个转换",
    );
    assert!(snapshot.transform_potential > 0.0);

    let food = frame.prices[0];
    let manufacture = frame.prices[1];
    let index_margin = (manufacture - 0.7 * food) / (0.7 * food);
    let local_food = food * frame.local_ratios[1][0];
    let local_manufacture = manufacture * frame.local_ratios[1][1];
    let local_margin = (local_manufacture - 0.7 * local_food) / (0.7 * local_food);

    assert!(
        local_manufacture > manufacture && local_food < food,
        "被制裁过的地方：本地工业品更贵、本地粮食更便宜：{local_manufacture} {local_food}",
    );
    assert!(
        index_margin < 0.0,
        "按全局指数它是亏的，指数口径会关停它：{index_margin}",
    );
    assert!(
        local_margin > 0.0,
        "按本地成交价它是赚的，所以它开着：{local_margin}",
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

fn permanently_sanctioned(rule: LevelRule, weight: f32, rounds: usize) -> Lab {
    let mut lab = Lab::new(&scarce(), 11).with_rule(rule);
    let department = lab.department_of(1, 0);
    for _ in 0..rounds {
        lab.sanction(&[department], weight);
        lab.step();
    }
    lab
}

#[test]
fn a_targeted_sanction_opens_a_monotone_local_gap() {
    let open = permanently_sanctioned(LevelRule::Fixed, 1.0, 120);
    let half = permanently_sanctioned(LevelRule::Fixed, 0.5, 120);
    let shut = permanently_sanctioned(LevelRule::Fixed, 0.0, 120);

    let no_barrier = open.spread(1, 0);
    let half_barrier = half.spread(1, 0);
    let full_barrier = shut.spread(1, 0);

    assert!(
        no_barrier.abs() < 0.02,
        "没有任何壁垒时不应当有本地价差：{no_barrier}",
    );
    assert!(
        half_barrier < no_barrier - 0.005,
        "壁垒越重，被制裁政权的本地价应当越低：{no_barrier} -> {half_barrier}",
    );
    assert!(
        full_barrier < half_barrier - 0.02,
        "完全断链应当把价差拉到最大：{half_barrier} -> {full_barrier}",
    );
    assert!(
        shut.polities[1].vwap[0] < open.polities[1].vwap[0],
        "被制裁政权的本地价应当低于它自己无壁垒时的水平",
    );
    assert!(
        shut.polities[0].vwap[0] > open.polities[0].vwap[0],
        "被制裁者退出后，其余政权的本地价应当抬高",
    );
}

#[test]
fn a_sanction_stays_local_to_the_named_department() {
    let lab = permanently_sanctioned(LevelRule::Fixed, 0.0, 80);
    let external = lab.department_external();
    let sanctioned = lab.department_of(1, 0);
    let neighbour = lab.department_of(1, 1);
    let foreign = lab.department_of(2, 0);

    assert_eq!(external[sanctioned], 0.0, "被制裁的部门必须与政权外断链");
    assert!(
        external[neighbour] > 0.0,
        "同一个政权里没被点名的部门照常对外做生意：{}",
        external[neighbour],
    );
    assert!(
        external[foreign] > 0.0,
        "别的政权照常做生意：{}",
        external[foreign],
    );
}

#[test]
fn a_sanctioned_department_drowns_in_its_own_output() {
    let mut lab = Lab::new(&scarce(), 11).with_rule(LevelRule::Fixed);
    let department = lab.department_of(1, 0);
    for _ in 0..60 {
        lab.sanction(&[department], 0.0);
        lab.step();
    }
    let early = lab.warehouses.warehouses[department].stocks[0].volume;
    for _ in 0..60 {
        lab.sanction(&[department], 0.0);
        lab.step();
    }
    let late = lab.warehouses.warehouses[department].stocks[0].volume;
    let fill = lab.department_fill()[department][0];

    assert_eq!(lab.department_external()[department], 0.0, "制裁期间不应当有对外成交");
    assert!(
        late > early,
        "卖不出去而产量照旧，库存只会越堆越高：{early} -> {late}",
    );
    assert!(
        fill < 0.2,
        "被制裁部门的兑现率应当塌掉：{fill}",
    );
}

#[test]
fn a_learned_wedge_costs_the_level_without_buying_a_gap() {
    let quiet = permanently_sanctioned(LevelRule::Fixed, 0.0, 120);
    let loud = permanently_sanctioned(LevelRule::Counterparty, 0.0, 120);

    let quiet_gap = quiet.spread(1, 0);
    let quiet_level = quiet.polities[1].vwap[0];
    let loud_level = loud.polities[1].vwap[0];

    assert!(
        quiet_gap.abs() < 0.15,
        "市场自己给的本地价差应当是小而可用的：{quiet_gap}",
    );
    assert!(quiet_level > 0.5, "对照组本地价应当贴着指数：{quiet_level}");
    assert!(
        loud_level < 0.3 * quiet_level,
        "把楔子接回报价会把本地价整体压到指数以下：{quiet_level} -> {loud_level}",
    );
    assert!(
        loud.spread(1, 0).abs() < 3.0 * quiet_gap.abs(),
        "它换不来更大的截面价差，只毁掉水平：安静 {quiet_gap} 接上 {}",
        loud.spread(1, 0),
    );
}

fn with_absorber(rounds: usize) -> Lab {
    let mut inputs = vec![0.0; GOODS];
    let mut outputs = vec![0.0; GOODS];
    inputs[0] = 2.0;
    outputs[1] = 4.0;
    let spec = scarce().with_transform(1, 1, inputs, outputs);
    let mut lab = Lab::new(&spec, 11).with_rule(LevelRule::Fixed);
    let department = lab.department_of(1, 0);
    for _ in 0..rounds {
        lab.sanction(&[department], 0.0);
        lab.step();
    }
    lab
}

#[test]
fn a_profitable_absorber_runs_but_cannot_clear_a_flood() {
    let bare = permanently_sanctioned(LevelRule::Fixed, 0.0, 80);
    let absorbing = with_absorber(80);
    let snapshot = absorbing.history.last().unwrap();

    assert_eq!(
        snapshot.transform_share, 1.0,
        "能赚钱的吸收者应当满负荷开工，而不是被埋在消费政策下面",
    );
    assert!(
        snapshot.transform_potential > 0.0,
        "吸收者开工的前提是利润率为正：{}",
        snapshot.transform_potential,
    );
    assert!(
        absorbing.spread(1, 0).abs() < bare.spread(1, 0).abs(),
        "吸收者应当压缩本地价差：无 {} 有 {}",
        bare.spread(1, 0),
        absorbing.spread(1, 0),
    );

    let sanctioned = absorbing.department_of(1, 0);
    let fill = absorbing.department_fill()[sanctioned][0];
    assert!(
        fill < 0.2,
        "吞吐由篮子规模决定、与价格无关，所以洪水清不掉：{fill}",
    );
    assert!(
        absorbing.warehouses.warehouses[sanctioned].stocks[0].volume > 100.0,
        "被制裁部门的库存仍然堆成山",
    );
}

#[test]
fn a_process_choice_follows_whichever_resource_is_tight() {
    let mut lab = Lab::new(&Spec::ladder(3, 0.5), 11).with_rule(LevelRule::Fixed);
    let department = lab.department_of(0, 0);
    lab.market.merchandises[0].price = 1.0;
    lab.market.merchandises[1].price = 1.0;
    let slow = 0;
    let fast = 1;

    lab.warehouses.warehouses[department].stocks[1].volume = 5.0;
    lab.departments.plan(&mut lab.warehouses, &lab.market);
    let scarce_material = lab.process_state(department);

    lab.warehouses.warehouses[department].stocks[1].volume = 20.0;
    lab.departments.plan(&mut lab.warehouses, &lab.market);
    let tight_capacity = lab.process_state(department);

    let mut roomy = Lab::new(&Spec::ladder(3, 0.5).with_capacity(200.0), 11)
        .with_rule(LevelRule::Fixed);
    roomy.market.merchandises[0].price = 1.0;
    roomy.market.merchandises[1].price = 1.0;
    roomy.warehouses.warehouses[department].stocks[1].volume = 20.0;
    roomy.departments.plan(&mut roomy.warehouses, &roomy.market);
    let roomy_capacity = roomy.process_state(department);

    assert_eq!(scarce_material.len(), 2, "阶梯上应当有两个工艺");
    assert!(
        scarce_material[slow].0 > scarce_material[fast].0,
        "原料不足时应当选省料但慢的（利润高、单位产能产出低）：{scarce_material:?}",
    );
    assert!(
        tight_capacity[fast].0 > tight_capacity[slow].0,
        "原料充足而产能紧张时应当选费料但快的（利润低、单位产能产出高）：{tight_capacity:?}",
    );
    assert!(
        roomy_capacity[slow].0 > roomy_capacity[fast].0,
        "产能也放开之后，紧的又变回原料，应当选回省料但慢的：{roomy_capacity:?}",
    );

    let rate = [LADDER_THRIFTY.0, LADDER_FAST.0];
    let capacity_cost = [LADDER_THRIFTY.2, LADDER_FAST.2];
    let margin_rate = |policy: usize| (1.0 - rate[policy]) / rate[policy];
    let output_per_capacity = |policy: usize| 1.0 / (capacity_cost[policy] * (1.0 + rate[policy]));
    assert!(
        margin_rate(fast) < margin_rate(slow),
        "费料但快的工艺利润率更低：{:.2} 对 {:.2}",
        margin_rate(fast),
        margin_rate(slow),
    );
    assert!(
        output_per_capacity(fast) > output_per_capacity(slow),
        "费料但快的工艺单位产能产出更高：{:.3} 对 {:.3}",
        output_per_capacity(fast),
        output_per_capacity(slow),
    );
}

#[test]
fn a_capacity_budget_throttles_the_department() {
    let mut tight = Lab::new(&Spec::ladder(3, 0.5), 11).with_rule(LevelRule::Fixed);
    tight.run(20);
    let mut loose = Lab::new(&Spec::ladder(3, 0.5).with_capacity(f32::INFINITY), 11)
        .with_rule(LevelRule::Fixed);
    loose.run(20);

    let tight_execution = tight.polities[0].execution;
    let loose_execution = loose.polities[0].execution;
    assert!(
        tight_execution < 1.0,
        "产能预算应当真的咬住：执行率 {tight_execution}",
    );
    assert!(
        tight_execution < loose_execution,
        "没有产能预算时执行率更高：紧张 {tight_execution} 宽松 {loose_execution}",
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
