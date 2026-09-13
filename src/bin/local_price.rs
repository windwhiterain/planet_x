use planet_x::local_price::{bloc_relations, Lab, LevelRule, Spec, GOODS, NAMES};

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
    match args.scenario.as_str() {
        "symmetric" => Spec::symmetric(args.polities),
        _ => Spec::scarce(args.polities, 0, 0),
    }
}

fn build(args: &Args) -> Lab {
    let lab = Lab::new(&spec(args), args.seed)
        .with_rule(args.rule)
        .with_forgetting(args.forgetting)
        .with_gain(args.gain)
        .with_recenter(args.recenter)
        .with_anchor(args.anchor)
        .with_grant(args.grant)
        .with_relations(&bloc_relations(args.polities, args.relations));
    lab
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
        "{:>4} {:>22} {:>26} {:>7} {:>8} {:>18}",
        "轮次", "银河指数", "各政权楔子(good0/1/2)", "执行率", "跨境占比", "每轮原始水平漂移"
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
            "{:>4} {:>22} {:>26} {:>7.3} {:>7.1}% {:>17}",
            snapshot.round,
            goods(&snapshot.prices),
            goods(&wedge),
            snapshot.executions[0],
            100.0 * share,
            format!(
                "{:+.2}%",
                100.0 * snapshot.gauge.iter().sum::<f32>() / GOODS as f32
            ),
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
