use planet_x::department::{DEFAULT_BARRIER, DEFAULT_CURVATURE, Rationing};
use planet_x::local_price::{
    GOODS, Kind, LADDER_CAPACITY, LADDER_FAST, LADDER_THRIFTY, Lab, NAMES, SECTOR_MOTIVE, Spec,
    bloc_relations,
};
use planet_x::warehouse::Warehouses;

const GOOD_LABELS: [&str; GOODS] = ["一产", "二产", "三产"];

#[derive(Clone)]
struct Args {
    scenario: String,
    polities: usize,
    rounds: usize,
    every: usize,
    grant: f32,
    transfer: f32,
    flow_scale: f32,
    price_curvature: f32,
    price_inertia: f32,
    target_rate: f32,
    rationing: String,
    barrier: f32,
    curvature: f32,
    relations: f32,
    block_from: usize,
    block_to: usize,
    block_polity: usize,
    block_weight: f32,
    sanction_polity: usize,
    sanction_unit: usize,
    sanction_from: usize,
    sanction_to: usize,
    sanction_w: f32,
    specialty: f32,
    capacity: Option<f32>,
    food_supply: f32,
    motive_ladder: bool,
    ladder_scale: f32,
    transform: bool,
    transform_rate: f32,
    transform_scale: f32,
    transform_polity: usize,
    transform_unit: usize,
    transform_in: usize,
    transform_out: usize,
    json: bool,
    goods_trace: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            scenario: String::from("modern"),
            polities: 3,
            rounds: 120,
            every: 10,
            grant: planet_x::local_price::GRANT,
            transfer: planet_x::department::Departments::DEFAULT_TRANSFER,
            flow_scale: planet_x::market::Market::DEFAULT_FLOW_SCALE,
            price_curvature: Warehouses::DEFAULT_PRICE_CURVATURE,
            price_inertia: Warehouses::DEFAULT_PRICE_INERTIA,
            target_rate: Warehouses::DEFAULT_TARGET_RATE,
            rationing: String::from("interior"),
            barrier: DEFAULT_BARRIER,
            curvature: DEFAULT_CURVATURE,
            relations: 1.0,
            block_from: usize::MAX,
            block_to: usize::MAX,
            block_polity: 0,
            block_weight: 0.0,
            sanction_polity: 1,
            sanction_unit: 0,
            sanction_from: 40,
            sanction_to: 80,
            sanction_w: 0.0,
            specialty: 1.0,
            capacity: None,
            food_supply: 0.5,
            motive_ladder: false,
            ladder_scale: 1.0,
            transform: false,
            transform_rate: 1.0,
            transform_scale: 4.0,
            transform_polity: 0,
            transform_unit: 0,
            transform_in: 1,
            transform_out: 0,
            json: false,
            goods_trace: false,
        }
    }
}

fn main() {
    let Some(args) = parse() else {
        usage();
        return;
    };
    match args.scenario.as_str() {
        "blockade" => blockade(&args),
        "sanction" => sanction_run(&args),
        "ladder" => ladder(&args),
        "sweep" => sweep(&args),
        "sectors" | "modern" => sectors(&args),
        _ => trace(&args),
    }
}

fn parse() -> Option<Args> {
    let mut args = Args::default();
    let mut items = std::env::args().skip(1);
    while let Some(flag) = items.next() {
        let mut value = || items.next();
        match flag.as_str() {
            "--scenario" => args.scenario = value()?,
            "--polities" => args.polities = value()?.parse().ok()?,
            "--rounds" | "-n" => args.rounds = value()?.parse().ok()?,
            "--every" => args.every = value()?.parse().ok()?,
            "--grant" => args.grant = value()?.parse().ok()?,
            "--transfer" => args.transfer = value()?.parse().ok()?,
            "--flow-scale" => args.flow_scale = value()?.parse().ok()?,
            "--price-curvature" => args.price_curvature = value()?.parse().ok()?,
            "--price-inertia" => args.price_inertia = value()?.parse().ok()?,
            "--target-rate" => args.target_rate = value()?.parse().ok()?,
            "--rationing" => args.rationing = value()?,
            "--barrier" => args.barrier = value()?.parse().ok()?,
            "--curvature" => args.curvature = value()?.parse().ok()?,
            "--w" => args.relations = value()?.parse().ok()?,
            "--block-from" => args.block_from = value()?.parse().ok()?,
            "--block-to" => args.block_to = value()?.parse().ok()?,
            "--block-polity" => args.block_polity = value()?.parse().ok()?,
            "--block-weight" => args.block_weight = value()?.parse().ok()?,
            "--sanction-polity" => args.sanction_polity = value()?.parse().ok()?,
            "--sanction-unit" => args.sanction_unit = value()?.parse().ok()?,
            "--sanction-from" => args.sanction_from = value()?.parse().ok()?,
            "--sanction-to" => args.sanction_to = value()?.parse().ok()?,
            "--sanction-w" => args.sanction_w = value()?.parse().ok()?,
            "--specialty" => args.specialty = value()?.parse().ok()?,
            "--capacity" => args.capacity = Some(value()?.parse().ok()?),
            "--food-supply" => args.food_supply = value()?.parse().ok()?,
            "--motive-ladder" => args.motive_ladder = true,
            "--ladder-scale" => args.ladder_scale = value()?.parse().ok()?,
            "--transform" => args.transform = true,
            "--transform-rate" => args.transform_rate = value()?.parse().ok()?,
            "--transform-scale" => args.transform_scale = value()?.parse().ok()?,
            "--transform-polity" => args.transform_polity = value()?.parse().ok()?,
            "--transform-unit" => args.transform_unit = value()?.parse().ok()?,
            "--transform-in" => args.transform_in = value()?.parse().ok()?,
            "--transform-out" => args.transform_out = value()?.parse().ok()?,
            "--json" => args.json = true,
            "--trace" | "--goods-trace" => args.goods_trace = true,
            _ => return None,
        }
    }
    Some(args)
}

fn usage() {
    println!(
        "用法：local_price [--scenario modern|sectors|scarce|symmetric|blockade|sanction|ladder|sweep]"
    );
    println!("  --polities N         政权数（默认 3）");
    println!("  --rounds, -n N       轮数（默认 120）");
    println!("  --every K            每 K 轮打印一行（默认 10）");
    println!(
        "  --grant G            每个部门开局的货币，同时是货币总量目标（默认 {}）",
        planet_x::local_price::GRANT
    );
    println!(
        "  --transfer R         每轮把余额拉向均值的比例，0 = 不转移（默认 {}）",
        planet_x::department::Departments::DEFAULT_TRANSFER
    );
    println!(
        "  --flow-scale S       势流的价差尺度，φ = tanh(Δln p / S)（默认 {}）",
        planet_x::market::Market::DEFAULT_FLOW_SCALE
    );
    println!(
        "  --price-curvature K  挂价对库存比值的陡度（默认 {}）",
        Warehouses::DEFAULT_PRICE_CURVATURE
    );
    println!(
        "  --price-inertia L    挂价的一阶低通系数，0 = 无惯性（默认 {}）",
        Warehouses::DEFAULT_PRICE_INERTIA
    );
    println!(
        "  --target-rate R      目标对未满足意愿的响应速率，0 = 恒为下限（默认 {}）",
        Warehouses::DEFAULT_TARGET_RATE
    );
    println!("  --rationing interior|hard   消费结算规则（默认 interior）");
    println!("  --barrier B --curvature T   内点法的障碍强度与边际效用曲率");
    println!("  --specialty F        各部门对自己那一层的产出乘数（比较优势）");
    println!("  --capacity N         部门产能预算");
    println!("  --w W                政权间的配对权重（默认 1.0）");
    println!(
        "  --block-from A --block-to B --block-polity P [--block-weight W]   在 [A,B) 轮封锁 P（W 默认 0 = 完全掐断）"
    );
    println!(
        "  --sanction-polity P --sanction-unit U --sanction-from A --sanction-to B   定向制裁一个部门"
    );
    println!("  --trace              逐轮逐商品打印指数/挂价/成交/库存/投入产出");
    println!("  --json               每个采样轮次打一行 JSONL（全精度）");
}

fn raw_spec(args: &Args) -> Spec {
    let base = match args.scenario.as_str() {
        "symmetric" => Spec::symmetric(args.polities),
        "ladder" => return Spec::ladder(args.polities, args.food_supply),
        "modern" => {
            let mut spec = Spec::modern(args.polities).with_specialty(args.specialty);
            if args.motive_ladder {
                spec.motive_ladder = ladder_weights(args);
            }
            return spec;
        }
        "sectors" => {
            let mut spec = Spec::sectors(args.polities).with_specialty(args.specialty);
            if args.motive_ladder {
                spec.motive_ladder = ladder_weights(args);
            }
            return spec;
        }
        _ => Spec::scarce(args.polities, 0, 0),
    };
    let base = base.with_specialty(args.specialty);
    if args.transform {
        let mut inputs = vec![0.0; GOODS];
        let mut outputs = vec![0.0; GOODS];
        inputs[args.transform_in] = args.transform_rate * args.transform_scale;
        outputs[args.transform_out] = args.transform_scale;
        base.with_transform(args.transform_polity, args.transform_unit, inputs, outputs)
    } else {
        base
    }
}

fn spec(args: &Args) -> Spec {
    let mut spec = raw_spec(args);
    if let Some(capacity) = args.capacity {
        spec.capacity = capacity;
    }
    spec
}

fn build_with(args: &Args, spec: Spec) -> Lab {
    Lab::new(&spec)
        .with_flow_scale(args.flow_scale)
        .with_rationing(match args.rationing.as_str() {
            "hard" => Rationing::Hard,
            _ => Rationing::Interior {
                barrier: args.barrier.max(0.0),
                curvature: args.curvature.max(0.0),
            },
        })
        .with_grant(args.grant)
        .with_transfer(args.transfer)
        .with_price_law(args.price_curvature, args.price_inertia, args.target_rate)
        .with_relations(&bloc_relations(args.polities, args.relations))
}

fn build(args: &Args) -> Lab {
    build_with(args, spec(args))
}

fn goods(values: &[f32]) -> String {
    values
        .iter()
        .map(|value| format!("{value:+.3}"))
        .collect::<Vec<String>>()
        .join(" ")
}

fn number(value: f32) -> String {
    format!("{value:>9.3}")
}

fn sci(value: f32) -> String {
    if value.is_finite() && value > 0.0 {
        format!("{value:.3e}")
    } else {
        String::from("—")
    }
}

fn money(lab: &Lab) -> (f32, f32, f32) {
    let mut total = 0.0f32;
    let mut low = f32::INFINITY;
    let mut high = f32::NEG_INFINITY;
    for department in &lab.departments.departments {
        total += department.currency;
        low = low.min(department.currency);
        high = high.max(department.currency);
    }
    if lab.departments.departments.is_empty() {
        low = 0.0;
        high = 0.0;
    }
    (total, low, high)
}

fn ladder_weights(args: &Args) -> Vec<f32> {
    SECTOR_MOTIVE
        .iter()
        .map(|motive| 1.0 + (motive - 1.0) * args.ladder_scale)
        .collect()
}

/// 一种商品这一轮的最低保本价：所有能产它的工艺里，投入成本 ÷ 产出量的最小值
fn break_even(lab: &Lab, good: usize, prices: &[f32]) -> f32 {
    let mut best = f32::INFINITY;
    for department in &lab.departments.departments {
        for policy in department
            .policies
            .iter()
            .filter(|policy| policy.is_production())
        {
            let output = policy.outputs.get(good).copied().unwrap_or(0.0);
            if !(output > 0.0) {
                continue;
            }
            let cost: f32 = policy
                .consumptions
                .iter()
                .enumerate()
                .map(|(k, consumption)| {
                    consumption.max(0.0) * prices.get(k).copied().unwrap_or(0.0)
                })
                .sum();
            best = best.min(cost / output);
        }
    }
    if best.is_finite() { best } else { 0.0 }
}

fn report_by_good(lab: &Lab) {
    let states = lab.good_states();
    let prices: Vec<f32> = states.iter().map(|state| state.index).collect();
    let bids: Vec<f32> = states
        .iter()
        .map(|state| {
            if state.bid > 0.0 {
                state.bid
            } else {
                state.index
            }
        })
        .collect();
    let asks: Vec<f32> = states
        .iter()
        .map(|state| {
            if state.ask > 0.0 {
                state.ask
            } else {
                state.index
            }
        })
        .collect();
    let added: Vec<f32> = states
        .iter()
        .enumerate()
        .map(|(k, state)| state.delivery * bids[k] - state.consumed * asks[k])
        .collect();
    let total: f32 = added.iter().sum();
    let share = |value: f32| {
        if total.abs() > 1e-9 {
            value / total
        } else {
            0.0
        }
    };
    let row = |label: &str, values: &[f32], format: fn(f32) -> String| {
        let cells = values
            .iter()
            .map(|value| format(*value))
            .collect::<Vec<String>>()
            .join(" ");
        println!("   {label:<10} {cells}");
    };
    println!("{} 轮后的逐商品状态：", lab.round);
    let first = prices.first().copied().unwrap_or(1.0).max(1e-9);
    row("指数", &prices, number);
    row(
        "相对一产",
        &prices
            .iter()
            .map(|price| price / first)
            .collect::<Vec<f32>>(),
        number,
    );
    row("买价 bid", &bids, number);
    row("卖价 ask", &asks, number);
    row(
        "保本(按卖价)",
        &(0..GOODS)
            .map(|k| break_even(lab, k, &asks))
            .collect::<Vec<f32>>(),
        number,
    );
    row(
        "产出入库",
        &states
            .iter()
            .map(|state| state.delivery)
            .collect::<Vec<f32>>(),
        number,
    );
    row(
        "投入消耗",
        &states
            .iter()
            .map(|state| state.consumed)
            .collect::<Vec<f32>>(),
        number,
    );
    row(
        "成交量",
        &states.iter().map(|state| state.dealt).collect::<Vec<f32>>(),
        number,
    );
    row(
        "库存",
        &states.iter().map(|state| state.stock).collect::<Vec<f32>>(),
        |value| format!("{value:>9.1}"),
    );
    row("增值", &added, |value| format!("{value:>9.2}"));
    row(
        "增值占比",
        &added
            .iter()
            .map(|value| share(*value))
            .collect::<Vec<f32>>(),
        |value| format!("{:>8.1}%", 100.0 * value),
    );
}

fn summary(lab: &Lab) {
    let snapshot = lab.history.last().unwrap();
    let (total, low, high) = money(lab);
    println!(
        "指数 {} 货币 合计 {total:.1} 最低 {low:.1} 最高 {high:.1} 转换占比 {:.1}% 转换利润率 {:+.3}",
        goods(&snapshot.prices),
        100.0 * snapshot.transform_share,
        snapshot.transform_potential,
    );
    for (p, polity) in lab.polities.iter().enumerate() {
        let row = snapshot.local_ratios.get(p).cloned().unwrap_or_default();
        let local = (0..GOODS)
            .map(|k| {
                format!(
                    "{:.3}",
                    snapshot.prices[k] * row.get(k).copied().unwrap_or(1.0)
                )
            })
            .collect::<Vec<String>>()
            .join(" ");
        println!(
            "  {}({}) 本地挂价 {local}   本地比值 {}",
            polity.name,
            p,
            (0..GOODS)
                .map(|k| format!("{:.3}", row.get(k).copied().unwrap_or(1.0)))
                .collect::<Vec<String>>()
                .join(" "),
        );
    }
}

fn trace(args: &Args) {
    let mut lab = build(args);
    println!(
        "场景 {} 政权 {} 轮 {} 货币 {} 势尺度 {} 挂价陡度 {} 惯性 {} 目标速率 {}",
        args.scenario,
        args.polities,
        args.rounds,
        args.grant,
        args.flow_scale,
        args.price_curvature,
        args.price_inertia,
        args.target_rate,
    );
    if args.goods_trace {
        println!(
            "{:>5} {:>6} {:>11} {:>11} {:>11} {:>9} {:>9} {:>9} {:>11} {:>9.1} {:>9.3} {:>9.3} {:>9.3} {:>12}",
            "轮次",
            "商品",
            "指数",
            "买价",
            "卖价",
            "卖出",
            "买入",
            "成交",
            "成交价",
            "库存",
            "入库",
            "投入",
            "意愿",
            "目标",
        );
    }
    for _ in 0..args.rounds {
        lab.step();
        if lab.round % args.every != 0 && lab.round != args.rounds {
            continue;
        }
        let snapshot = lab.history.last().unwrap();
        let index: Vec<f32> = snapshot.prices.clone();
        let range = if index.iter().all(|value| *value > 0.0) {
            index.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
                / index.iter().cloned().fold(f32::INFINITY, f32::min)
        } else {
            0.0
        };
        println!(
            "{:>5} 指数 {} 指数极差 {range:.3} 转换 {:>5.1}%",
            snapshot.round,
            goods(&index),
            100.0 * snapshot.transform_share,
        );
        if args.goods_trace {
            for (k, state) in lab.good_states().iter().enumerate() {
                println!(
                    "{:>5} {:>6} {:>11} {:>11} {:>11} {:>9.2} {:>9.2} {:>9.2} {:>11} {:>9.1} {:>9.3} {:>9.3} {:>9.3} {:>12.2}",
                    snapshot.round,
                    GOOD_LABELS[k],
                    sci(state.index),
                    sci(state.bid),
                    sci(state.ask),
                    state.sold,
                    state.bought,
                    state.dealt,
                    sci(state.deal_price),
                    state.stock,
                    state.delivery,
                    state.consumed,
                    state.wanted,
                    state.target,
                );
            }
        }
    }
    println!();
    report_by_good(&lab);
    summary(&lab);
}

fn json_line(lab: &Lab) -> String {
    let states = lab.good_states();
    let mut goods = String::new();
    for (k, s) in states.iter().enumerate() {
        if k > 0 {
            goods.push(',');
        }
        goods.push_str(&format!(
            concat!(
                "{{\"good\":{},\"index\":{:e},\"bid\":{:e},\"ask\":{:e},\"deal_price\":{:e},",
                "\"quote_sell\":{:e},\"quote_buy\":{:e},",
                "\"sold\":{:e},\"bought\":{:e},\"dealt\":{:e},\"stock\":{:e},",
                "\"delivery\":{:e},\"consumed\":{:e},\"wanted\":{:e},\"target\":{:e}}}"
            ),
            k,
            s.index,
            s.bid,
            s.ask,
            s.deal_price,
            s.quote_sell,
            s.quote_buy,
            s.sold,
            s.bought,
            s.dealt,
            s.stock,
            s.delivery,
            s.consumed,
            s.wanted,
            s.target,
        ));
    }
    let mut departments = String::new();
    for (i, department) in lab.departments.departments.iter().enumerate() {
        if i > 0 {
            departments.push(',');
        }
        let warehouse = &lab.warehouses.warehouses[i];
        let stock: Vec<String> = warehouse
            .stocks
            .iter()
            .map(|stock| format!("{:e}", stock.volume))
            .collect();
        let target: Vec<String> = warehouse
            .stocks
            .iter()
            .map(|stock| format!("{:e}", stock.target_volume))
            .collect();
        let price: Vec<String> = warehouse
            .stocks
            .iter()
            .map(|stock| format!("{:e}", stock.price))
            .collect();
        let intake: Vec<String> = department
            .intake()
            .iter()
            .map(|value| format!("{value:e}"))
            .collect();
        let delivery: Vec<String> = department
            .delivery()
            .iter()
            .map(|value| format!("{value:e}"))
            .collect();
        departments.push_str(&format!(
            concat!(
                "{{\"department\":{},\"stock\":[{}],\"target\":[{}],\"price\":[{}],",
                "\"intake\":[{}],\"delivery\":[{}],",
                "\"currency\":{:e},\"capacity_scale\":{:e},",
                "\"converged\":{},\"degraded\":{},\"iterations\":{},\"residual\":{:e},\"utilization\":{:e}}}"
            ),
            i,
            stock.join(","),
            target.join(","),
            price.join(","),
            intake.join(","),
            delivery.join(","),
            department.currency,
            department.capacity_scale(),
            department.settlement().converged,
            department.settlement().degraded,
            department.settlement().iterations,
            department.settlement().residual,
            department.settlement().utilization,
        ));
    }
    let quotes: Vec<String> = lab
        .warehouses
        .ask
        .iter()
        .enumerate()
        .map(|(locality, row)| {
            let cells: Vec<String> = row
                .iter()
                .enumerate()
                .map(|(k, ask)| {
                    let bid = lab
                        .warehouses
                        .bid
                        .get(locality)
                        .and_then(|row| row.get(k))
                        .copied()
                        .unwrap_or(0.0);
                    format!("[{bid:e},{ask:e}]")
                })
                .collect();
            format!(
                "{{\"locality\":{locality},\"quote\":[{}]}}",
                cells.join(",")
            )
        })
        .collect();
    let (total, low, high) = money(lab);
    let delivery_split: Vec<String> = (0..GOODS)
        .map(|k| {
            let free: f32 = lab
                .departments
                .departments
                .iter()
                .flat_map(|department| department.policies.iter())
                .filter(|policy| {
                    policy.is_production() && policy.consumptions.iter().all(|value| *value <= 0.0)
                })
                .map(|policy| policy.distribution() * policy.outputs.get(k).copied().unwrap_or(0.0))
                .sum();
            let ladder: f32 = lab
                .departments
                .departments
                .iter()
                .flat_map(|department| department.policies.iter())
                .filter(|policy| {
                    policy.is_production() && policy.consumptions.iter().any(|value| *value > 0.0)
                })
                .map(|policy| policy.distribution() * policy.outputs.get(k).copied().unwrap_or(0.0))
                .sum();
            format!("[{free:e},{ladder:e}]")
        })
        .collect();
    format!(
        concat!(
            "{{\"round\":{},\"goods\":[{}],\"departments\":[{}],\"quotes\":[{}],",
            "\"delivery_free_or_ladder\":[{}],",
            "\"money\":{:e},\"money_min\":{:e},\"money_max\":{:e},",
            "\"settlement_failures\":{},\"settlement_degraded\":{}}}"
        ),
        lab.round,
        goods,
        departments,
        quotes.join(","),
        delivery_split.join(","),
        total,
        low,
        high,
        lab.settlement_failures,
        lab.settlement_degraded,
    )
}

fn sectors(args: &Args) {
    let mut lab = build(args);
    for _ in 0..args.rounds {
        lab.step();
        if args.json {
            if lab.round % args.every == 0 || lab.round == args.rounds {
                println!("{}", json_line(&lab));
            }
            continue;
        }
        if lab.round % args.every != 0 && lab.round != args.rounds {
            continue;
        }
        let snapshot = lab.history.last().unwrap();
        let consumed: f32 = lab.good_states().iter().map(|state| state.consumed).sum();
        let dealt: f32 = lab.good_states().iter().map(|state| state.dealt).sum();
        let stock: f32 = lab.good_states().iter().map(|state| state.stock).sum();
        println!(
            "{:>5} 指数 {} 消费 {consumed:>10.2} 成交 {dealt:>10.2} 库存 {stock:>10.2}",
            snapshot.round,
            goods(&snapshot.prices),
        );
    }
    if args.json {
        return;
    }
    println!();
    report_by_good(&lab);
    summary(&lab);
}

fn local_gap(lab: &Lab, good: usize) -> String {
    (0..lab.polities.len())
        .map(|p| format!("{}({:+.4})", lab.polities[p].name, lab.spread(p, good)))
        .collect::<Vec<String>>()
        .join(" ")
}

fn blockade(args: &Args) {
    let mut lab = build(args);
    for round in 0..args.rounds {
        let blocked = round >= args.block_from && round < args.block_to;
        if blocked {
            lab.block(&[args.block_polity], args.block_weight);
        } else {
            lab.block(&[args.block_polity], 1.0);
        }
        lab.step();
        if lab.round % args.every != 0 && lab.round != args.rounds {
            continue;
        }
        let snapshot = lab.history.last().unwrap();
        println!(
            "第 {:>4} 轮 {} 指数 {} 一产价差 {}",
            snapshot.round,
            if blocked { "封锁" } else { "通行" },
            goods(&snapshot.prices),
            local_gap(&lab, 0),
        );
    }
    summary(&lab);
}

fn sanction_run(args: &Args) {
    let mut lab = build(args);
    let sanctioned = lab.department_of(args.sanction_polity, args.sanction_unit, Kind::Consumer);
    for round in 0..args.rounds {
        let active = round >= args.sanction_from && round < args.sanction_to;
        if active {
            lab.sanction(&[sanctioned], args.sanction_w);
        } else if round == args.sanction_from {
            lab.unsanction();
        }
        lab.step();
        if lab.round % args.every != 0 && lab.round != args.rounds {
            continue;
        }
        let snapshot = lab.history.last().unwrap();
        let department = &lab.departments.departments[sanctioned];
        let fill: f32 = department
            .intake()
            .iter()
            .zip(
                lab.warehouses.warehouses[sanctioned]
                    .stocks
                    .iter()
                    .map(|stock| stock.wanted),
            )
            .map(|(taken, wanted)| if wanted > 0.0 { taken / wanted } else { 1.0 })
            .sum::<f32>()
            / GOODS as f32;
        println!(
            "第 {:>4} 轮 {} 指数 {} 被制裁部门执行率 {:.3} 一产价差 {}",
            snapshot.round,
            if active { "制裁" } else { "通行" },
            goods(&snapshot.prices),
            fill,
            local_gap(&lab, 0),
        );
    }
    summary(&lab);
}

fn ladder(args: &Args) {
    println!(
        "技术阶梯：同一个部门里两个工艺。省料但慢 = 每件粮吃 {:.2} 件工业品、产能占用 {:.2}/件；费料但快 = {:.2} 件工业品、产能占用 {:.2}/件；产能预算 {}",
        LADDER_THRIFTY.0, LADDER_THRIFTY.2, LADDER_FAST.0, LADDER_FAST.2, LADDER_CAPACITY,
    );
    println!(
        "{:>9} {:>10} {:>26} {:>26} {:>12}",
        "粮食供给",
        "工/粮价",
        "省料但慢 份额/单位产能利润",
        "费料但快 份额/单位产能利润",
        "粮食指数"
    );
    for supply in [0.4f32, 0.6, 0.8, 1.0, 1.5, 2.5, 4.0] {
        let mut local = args.clone();
        local.food_supply = supply;
        let mut lab = build_with(&local, Spec::ladder(local.polities, supply));
        lab.run(args.rounds);
        let department = lab.department_of(0, 0, Kind::Producer);
        let processes = lab.process_state(department);
        let report = |index: usize| match processes.get(index) {
            Some((share, potential, _)) => {
                format!("{:>10.1}% / {:>+10.3}", 100.0 * share, potential)
            }
            None => String::from("—"),
        };
        let food = lab.good_states()[0].index;
        let manufacture = lab.good_states()[1].index;
        println!(
            "{supply:>9.2} {:>10.2} {:>26} {:>26} {:>12.3}",
            manufacture / food.max(1e-9),
            report(0),
            report(1),
            food,
        );
    }
}

fn sweep(args: &Args) {
    println!(
        "{:>10} {:>12} {:>12} {:>12} {:>12}",
        "产能", "消费", "成交", "库存", "指数极差"
    );
    for capacity in [4.0f32, 6.0, 8.0, 12.0, 16.0, 24.0] {
        let mut local = args.clone();
        local.capacity = Some(capacity);
        let mut lab = build(&local);
        lab.run(args.rounds);
        let states = lab.good_states();
        let consumed: f32 = states.iter().map(|state| state.consumed).sum();
        let dealt: f32 = states.iter().map(|state| state.dealt).sum();
        let stock: f32 = states.iter().map(|state| state.stock).sum();
        let index: Vec<f32> = states.iter().map(|state| state.index).collect();
        let range = if index.iter().all(|value| *value > 0.0) {
            index.iter().cloned().fold(f32::NEG_INFINITY, f32::max)
                / index.iter().cloned().fold(f32::INFINITY, f32::min)
        } else {
            0.0
        };
        println!("{capacity:>10.1} {consumed:>12.2} {dealt:>12.2} {stock:>12.2} {range:>12.3}");
    }
}

#[allow(dead_code)]
fn name_of(polity: usize) -> &'static str {
    NAMES[polity % NAMES.len()]
}
