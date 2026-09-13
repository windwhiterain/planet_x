use planet_x::local_price::{
    bloc_relations, Lab, LevelRule, Spec, GOODS, LADDER_CAPACITY, LADDER_FAST, LADDER_THRIFTY, NAMES,
};

#[derive(Clone)]
struct Args {
    scenario: String,
    rule: LevelRule,
    polities: usize,
    rounds: usize,
    every: usize,
    forgetting: f32,
    gain: f32,
    recenter: bool,
    anchor: bool,
    grant: f32,
    transform: bool,
    transform_rate: f32,
    transform_scale: f32,
    transform_polity: usize,
    transform_unit: usize,
    transform_in: usize,
    transform_out: usize,
    sanction_polity: usize,
    sanction_unit: usize,
    sanction_from: usize,
    sanction_to: usize,
    sanction_w: f32,
    food_supply: f32,
    relations: f32,
    block_from: usize,
    block_to: usize,
    block_polity: usize,
    seed: u64,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            scenario: String::from("scarce"),
            rule: LevelRule::Counterparty,
            polities: 3,
            rounds: 120,
            every: 10,
            forgetting: 0.1,
            gain: 0.02,
            recenter: false,
            anchor: true,
            grant: 10.0,
            transform: false,
            transform_rate: 1.0,
            transform_scale: 4.0,
            transform_polity: 0,
            transform_unit: 0,
            transform_in: 1,
            transform_out: 0,
            sanction_polity: 1,
            sanction_unit: 0,
            sanction_from: 40,
            sanction_to: 80,
            sanction_w: 0.0,
            food_supply: 0.5,
            relations: 1.0,
            block_from: usize::MAX,
            block_to: usize::MAX,
            block_polity: 0,
            seed: 11,
        }
    }
}

fn main() {
    let Some(args) = parse() else {
        usage();
        return;
    };
    match args.scenario.as_str() {
        "sweep" => sweep(&args),
        "blockade" => blockade(&args),
        "sanction" => sanction_run(&args),
        "sanction-sweep" => sanction_sweep(&args),
        "ladder" => ladder(&args),
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
            "--rule" => args.rule = LevelRule::parse(&value()?)?,
            "--polities" => args.polities = value()?.parse().ok()?,
            "--rounds" | "-n" => args.rounds = value()?.parse().ok()?,
            "--every" => args.every = value()?.parse().ok()?,
            "--forgetting" => args.forgetting = value()?.parse().ok()?,
            "--gain" => args.gain = value()?.parse().ok()?,
            "--recenter" => args.recenter = true,
            "--no-anchor" => args.anchor = false,
            "--grant" => args.grant = value()?.parse().ok()?,
            "--transform" => args.transform = true,
            "--transform-rate" => args.transform_rate = value()?.parse().ok()?,
            "--transform-scale" => args.transform_scale = value()?.parse().ok()?,
            "--transform-polity" => args.transform_polity = value()?.parse().ok()?,
            "--transform-unit" => args.transform_unit = value()?.parse().ok()?,
            "--transform-in" => args.transform_in = value()?.parse().ok()?,
            "--transform-out" => args.transform_out = value()?.parse().ok()?,
            "--sanction-polity" => args.sanction_polity = value()?.parse().ok()?,
            "--sanction-unit" => args.sanction_unit = value()?.parse().ok()?,
            "--sanction-from" => args.sanction_from = value()?.parse().ok()?,
            "--sanction-to" => args.sanction_to = value()?.parse().ok()?,
            "--sanction-w" => args.sanction_w = value()?.parse().ok()?,
            "--food-supply" => args.food_supply = value()?.parse().ok()?,
            "--w" => args.relations = value()?.parse().ok()?,
            "--block-from" => args.block_from = value()?.parse().ok()?,
            "--block-to" => args.block_to = value()?.parse().ok()?,
            "--block-polity" => args.block_polity = value()?.parse().ok()?,
            "--seed" | "-s" => args.seed = value()?.parse().ok()?,
            "--help" | "-h" => return None,
            _ => return None,
        }
    }
    Some(args)
}

fn usage() {
    println!("用法：local_price [--scenario symmetric|scarce|blockade|sweep] [--rule fixed|vwap|counterparty|shortfall]");
    println!("  --polities N     政权数（默认 3）");
    println!("  --rounds, -n N   轮数（默认 120）");
    println!("  --every K        每 K 轮打印一行（默认 10）");
    println!("  --forgetting F   L 的学习率（默认 0.1）");
    println!("  --gain G         shortfall 规则的增益（默认 0.02）");
    println!("  --recenter       每轮把楔子的均值钉回 0");
    println!("  --w W            政权间的配对权重（默认 1.0）");
    println!("  --grant G        每个部门每轮的拨款（默认 10）");
    println!("  --no-anchor      关掉水平锚，观察原始漂移");
    println!("  --block-from A --block-to B --block-polity P   在 [A,B) 轮封锁 P");
    println!("  --seed, -s S     随机种子（默认 11）");
}

fn spec(args: &Args) -> Spec {
    let base = match args.scenario.as_str() {
        "symmetric" => Spec::symmetric(args.polities),
        "ladder" => return Spec::ladder(args.polities, args.food_supply),
        _ => Spec::scarce(args.polities, 0, 0),
    };
    if args.transform {
        let mut inputs = vec![0.0; GOODS];
        let mut outputs = vec![0.0; GOODS];
        inputs[args.transform_in] = args.transform_rate * args.transform_scale;
        outputs[args.transform_out] = args.transform_scale;
        base.with_transform(
            args.transform_polity,
            args.transform_unit,
            inputs,
            outputs,
        )
    } else {
        base
    }
}

fn build_with(args: &Args, spec: Spec) -> Lab {
    Lab::new(&spec, args.seed)
        .with_rule(args.rule)
        .with_forgetting(args.forgetting)
        .with_gain(args.gain)
        .with_recenter(args.recenter)
        .with_anchor(args.anchor)
        .with_grant(args.grant)
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

fn trace(args: &Args) {
    let mut lab = build(args);
    println!(
        "场景 {} 规则 {} 政权 {} 轮 {} 学习率 {} 权重 {} 重定心 {}",
        args.scenario,
        args.rule.name(),
        args.polities,
        args.rounds,
        args.forgetting,
        args.relations,
        args.recenter,
    );
    println!(
        "{:>4} {:>22} {:>26} {:>7} {:>8} {:>18} {:>9} {:>9}",
        "轮次", "银河指数", "各政权楔子(good0/1/2)", "执行率", "跨境占比", "每轮原始水平漂移", "转换占比", "转换利润率"
    );
    for _ in 0..args.rounds {
        lab.step();
        if lab.round % args.every != 0 && lab.round != args.rounds {
            continue;
        }
        let snapshot = lab.history.last().unwrap();
        let external = snapshot.internal + snapshot.external;
        let share = if external > 0.0 {
            snapshot.external / external
        } else {
            0.0
        };
        let wedge = snapshot.wedges[0]
            .iter()
            .map(|value| 100.0 * value)
            .collect::<Vec<f32>>();
        println!(
            "{:>4} {:>22} {:>26} {:>7.3} {:>7.1}% {:>17} {:>9} {:>9}",
            snapshot.round,
            goods(&snapshot.prices),
            goods(&wedge),
            snapshot.executions[0],
            100.0 * share,
            format!(
                "{:+.2}%",
                100.0 * snapshot.gauge.iter().sum::<f32>() / GOODS as f32
            ),
            format!("{:.1}%", 100.0 * snapshot.transform_share),
            format!("{:+.3}", snapshot.transform_potential),
        );
    }
    summary(&lab);
}

fn summary(lab: &Lab) {
    println!();
    println!("政权      最终楔子(百分比)              本地价                          期望成交价");
    for (p, polity) in lab.polities.iter().enumerate() {
        let wedge = polity
            .wedge
            .iter()
            .map(|value| format!("{:+.1}%", 100.0 * value))
            .collect::<Vec<String>>()
            .join(" ");
        let level = polity
            .level
            .iter()
            .map(|value| format!("{value:.3}"))
            .collect::<Vec<String>>()
            .join(" ");
        let vwap = polity
            .vwap
            .iter()
            .map(|value| format!("{value:.3}"))
            .collect::<Vec<String>>()
            .join(" ");
        println!(
            "{}({}) {:>26} {:>30} {:>30} 执行 {:.3}",
            polity.name,
            p,
            wedge,
            level,
            vwap,
            polity.execution,
        );
    }
    let snapshot = lab.history.last().unwrap();
    let first = &lab.history[0];
    let drift = (0..GOODS)
        .map(|k| (snapshot.prices[k] / first.prices[k]).ln())
        .sum::<f32>()
        / GOODS as f32;
    let volume = snapshot.internal + snapshot.external;
    let index: Vec<f32> = lab
        .market
        .merchandises
        .iter()
        .map(|merchandise| merchandise.price)
        .collect();
    for (p, polity) in lab.polities.iter().enumerate() {
        let local = (0..GOODS)
            .map(|k| {
                format!(
                    "{:.3}",
                    index[k] * lab.warehouses.local_ratio(p, k).unwrap_or(1.0)
                )
            })
            .collect::<Vec<String>>()
            .join(" ");
        println!(
            "  {}({}) 本地成交价 {local}   本地比值 {}",
            polity.name,
            p,
            (0..GOODS)
                .map(|k| format!("{:.3}", lab.warehouses.local_ratio(p, k).unwrap_or(1.0)))
                .collect::<Vec<String>>()
                .join(" "),
        );
    }
    println!(
        "银河指数 {} 全程漂移 {:+.1}% 未成交 {:.1}% 跨境占比 {:.1}% 国库 {:.1}",
        goods(&snapshot.prices),
        100.0 * drift,
        100.0 * snapshot.uncleared,
        if volume > 0.0 {
            100.0 * snapshot.external / volume
        } else {
            0.0
        },
        snapshot.treasury,
    );
}

fn blockade(args: &Args) {
    let mut lab = build(args);
    let mut max_wedge = 0.0f32;
    let mut min_execution = 1.0f32;
    for round in 0..args.rounds {
        let blocked = round >= args.block_from && round < args.block_to;
        if blocked {
            lab.block(&[args.block_polity], args.relations);
        } else {
            lab.block(&[args.block_polity], 1.0);
        }
        lab.step();
        let polity = &lab.polities[args.block_polity];
        max_wedge = max_wedge.max(polity.wedge[0]);
        min_execution = min_execution.min(polity.execution);
        if lab.round % args.every == 0 || lab.round == args.rounds {
            let snapshot = lab.history.last().unwrap();
            let volume = snapshot.internal + snapshot.external;
            println!(
                "第 {:>3} 轮 {} 银河 {} 楔子 {} 本地价 {} 执行 {:.3} 跨境 {:>5.1}% 未成交 {:>5.1}%",
                snapshot.round,
                if blocked { "封锁" } else { "通行" },
                goods(&snapshot.prices),
                goods(&polity.wedge),
                goods(&polity.level),
                polity.execution,
                if volume > 0.0 {
                    100.0 * snapshot.external / volume
                } else {
                    0.0
                },
                100.0 * snapshot.uncleared,
            );
        }
    }
    println!(
        "封锁期间 {} 的 good0 楔子峰值 {:+.1}%，执行率最低 {:.3}",
        NAMES[args.block_polity % NAMES.len()],
        100.0 * max_wedge,
        min_execution,
    );
    summary(&lab);
}

fn ladder(args: &Args) {
    println!(
        "技术阶梯：同一个部门里两个工艺。省料但慢 = 每件粮吃 {:.2} 件工业品、产能占用 {:.2}/件；费料但快 = {:.2} 件工业品、产能占用 {:.2}/件；产能预算 {}。本例切换点在 工业品价 ÷ 粮食价 ≈ 0.86",
        LADDER_THRIFTY.0,
        LADDER_THRIFTY.2,
        LADDER_FAST.0,
        LADDER_FAST.2,
        LADDER_CAPACITY,
    );
    println!(
        "{:>9} {:>10} {:>26} {:>26} {:>10} {:>10}",
        "粮食供给", "工/粮价", "省料但慢 份额/单位产能利润", "费料但快 份额/单位产能利润", "部门执行率", "粮食指数"
    );
    for supply in [0.4f32, 0.6, 0.8, 1.0, 1.5, 2.5, 4.0] {
        let mut local = args.clone();
        local.food_supply = supply;
        let mut lab = build_with(&local, Spec::ladder(local.polities, supply));
        lab.run(args.rounds);
        let department = lab.department_of(0, 0);
        let processes = lab.process_state(department);
        let report = |index: usize| match processes.get(index) {
            Some((share, potential, _)) => format!("{:>10.1}% / {:>+10.3}", 100.0 * share, potential),
            None => String::from("—"),
        };
        let food = lab.market.merchandises[0].price;
        let manufacture = lab.market.merchandises[1].price;
        println!(
            "{supply:>9.2} {:>10.2} {:>26} {:>26} {:>10.3} {:>10.3}",
            manufacture / food.max(1e-9),
            report(0),
            report(1),
            lab.polities[0].execution,
            food,
        );
    }
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

fn local_prices(lab: &Lab, good: usize) -> String {
    lab.polities
        .iter()
        .map(|polity| format!("{:.3}", polity.vwap[good]))
        .collect::<Vec<String>>()
        .join(" ")
}

fn spread(lab: &Lab, seat: usize, good: usize) -> f32 {
    lab.spread(seat, good)
}

fn sanction_run(args: &Args) {
    let mut lab = build(args);
    let department = lab.department_of(args.sanction_polity, args.sanction_unit);
    let name = lab.polities[args.sanction_polity].name;
    println!(
        "局部制裁：{name}(政权 {}) 第 {} 个部门 = 仓库 {department}，在 [{} , {}) 轮与政权外断链，权重 {}，规则 {}",
        args.sanction_polity,
        args.sanction_unit,
        args.sanction_from,
        args.sanction_to,
        args.sanction_w,
        args.rule.name(),
    );
    println!(
        "{:>4} {:>5} {:>9} {:>10} {:>10} {:>9} {:>9} {:>22} {:>9} {:>8} {:>9}",
        "轮次", "状态", "粮食指数", "部门成交价", "部门兑现", "部门库存", "对外成交", "各政权本地价", "价差", "转换占比", "利润率"
    );
    let mut during = Vec::new();
    let mut after = Vec::new();
    for round in 0..args.rounds {
        let on = round >= args.sanction_from && round < args.sanction_to;
        if on {
            lab.sanction(&[department], args.sanction_w);
        } else {
            lab.unsanction();
        }
        lab.step();
        let prices = lab.department_prices();
        let fills = lab.department_fill();
        let external = lab.department_external();
        let gap = spread(&lab, args.sanction_polity, 0);
        if on && round + 1 >= args.sanction_from + 8 {
            during.push(gap);
        }
        if !on && round >= args.sanction_to + 8 {
            after.push(gap);
        }
        let edge = round + 1 == args.sanction_from || round + 1 == args.sanction_to;
        if lab.round % args.every == 0 || edge || lab.round == args.rounds {
            let snapshot = lab.history.last().unwrap();
            println!(
                "{:>4} {:>5} {:>9.3} {:>10} {:>10} {:>9.1} {:>9.2} {:>22} {:>+9.3} {:>8.1}% {:>+9.3}",
                lab.round,
                if on { "制裁" } else { "通行" },
                lab.market.merchandises[0].price,
                format!("{:.3}", prices[department][0]),
                format!("{:.2}", fills[department][0]),
                lab.warehouses.warehouses[department].stocks[0].volume,
                external[department],
                local_prices(&lab, 0),
                gap,
                100.0 * snapshot.transform_share,
                snapshot.transform_potential,
            );
        }
    }
    let index = lab
        .market
        .merchandises
        .iter()
        .map(|merchandise| merchandise.price)
        .collect::<Vec<f32>>();
    for p in 0..lab.polities.len() {
        let local = (0..GOODS)
            .map(|k| {
                format!(
                    "{:.3}",
                    index[k] * lab.warehouses.local_ratio(p, k).unwrap_or(1.0)
                )
            })
            .collect::<Vec<String>>()
            .join(" ");
        println!("  地方 {p} 本地成交价 {local}   指数 {}", goods(&index));
    }
    println!(
        "制裁期间价差均值 {:+.4}（{} 轮），解除后 {:+.4}（{} 轮）",
        mean(&during),
        during.len(),
        mean(&after),
        after.len(),
    );
}

fn sanction_sweep(args: &Args) {
    println!(
        "局部制裁严重度扫描：政权 {} 第 {} 个部门，{} 轮，规则 {}",
        args.sanction_polity,
        args.sanction_unit,
        args.rounds,
        args.rule.name(),
    );
    println!(
        "{:>7} {:>10} {:>12} {:>12} {:>10} {:>10}",
        "权重", "对外成交", "制裁政权价", "其余政权价", "价差", "执行率"
    );
    for weight in [1.0f32, 0.75, 0.5, 0.25, 0.0] {
        let mut local = args.clone();
        local.sanction_w = weight;
        let mut lab = build(&local);
        let department = lab.department_of(local.sanction_polity, local.sanction_unit);
        for _ in 0..args.rounds {
            lab.sanction(&[department], weight);
            lab.step();
        }
        let external = lab.department_external();
        let seat = local.sanction_polity;
        println!(
            "{weight:>7.2} {:>10.2} {:>12.3} {:>12.3} {:>+10.3} {:>10.2}",
            external[department],
            lab.polities[seat].vwap[0],
            mean(
                &lab.polities
                    .iter()
                    .enumerate()
                    .filter(|(p, _)| *p != seat)
                    .map(|(_, polity)| polity.vwap[0])
                    .collect::<Vec<f32>>()
            ),
            spread(&lab, seat, 0),
            lab.polities[seat].execution,
        );
    }
}

fn sweep(args: &Args) {
    println!(
        "场景 {} 规则 {} 政权 {} 轮 {} 学习率 {} 重定心 {}",
        args.scenario,
        args.rule.name(),
        args.polities,
        args.rounds,
        args.forgetting,
        args.recenter,
    );
    println!("{:>6} {:>10} {:>24} {:>26}", "权重", "跨境占比", "最终楔子(good0)", "本地价(good0)");
    for weight in [0.0f32, 0.1, 0.25, 0.5, 0.75, 1.0] {
        let mut local = Args {
            relations: weight,
            ..Default::default()
        };
        local.scenario = args.scenario.clone();
        local.rule = args.rule;
        local.polities = args.polities;
        local.rounds = args.rounds;
        local.forgetting = args.forgetting;
        local.gain = args.gain;
        local.recenter = args.recenter;
        local.seed = args.seed;
        let mut lab = build(&local);
        lab.run(args.rounds);
        let snapshot = lab.history.last().unwrap();
        let volume = snapshot.internal + snapshot.external;
        let wedges = lab
            .polities
            .iter()
            .map(|polity| format!("{:+.1}%", 100.0 * polity.wedge[0]))
            .collect::<Vec<String>>()
            .join(" ");
        let levels = lab
            .polities
            .iter()
            .map(|polity| format!("{:.3}", polity.level[0]))
            .collect::<Vec<String>>()
            .join(" ");
        println!(
            "{weight:>6.2} {:>9.1}% {:>24} {:>26} 执行 {}",
            if volume > 0.0 {
                100.0 * snapshot.external / volume
            } else {
                0.0
            },
            wedges,
            levels,
            lab.polities
                .iter()
                .map(|polity| format!("{:.2}", polity.execution))
                .collect::<Vec<String>>()
                .join("/"),
        );
    }
}
