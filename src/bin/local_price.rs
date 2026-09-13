use planet_x::local_price::{
    bloc_relations, Lab, LevelRule, Spec, GOODS, LADDER_CAPACITY, LADDER_FAST, LADDER_THRIFTY,
    NAMES, SECTOR_MOTIVE,
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
    motive_ladder: bool,
    specialty: f32,
    capacity: Option<f32>,
    ladder_scale: f32,
    specialty_top: Option<f32>,
    json: bool,
    soft_eps: f32,
    goods_trace: bool,
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
            recenter: true,
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
            motive_ladder: false,
            specialty: 1.0,
            capacity: None,
            ladder_scale: 1.0,
            specialty_top: None,
            json: false,
            soft_eps: planet_x::market::Market::DEFAULT_SOFT_EPS,
            goods_trace: false,
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
            "--rule" => args.rule = LevelRule::parse(&value()?)?,
            "--polities" => args.polities = value()?.parse().ok()?,
            "--rounds" | "-n" => args.rounds = value()?.parse().ok()?,
            "--every" => args.every = value()?.parse().ok()?,
            "--forgetting" => args.forgetting = value()?.parse().ok()?,
            "--gain" => args.gain = value()?.parse().ok()?,
            "--no-recenter" => args.recenter = false,
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
            "--motive-ladder" => args.motive_ladder = true,
            "--specialty" => args.specialty = value()?.parse().ok()?,
            "--capacity" => args.capacity = Some(value()?.parse().ok()?),
            "--ladder-scale" => args.ladder_scale = value()?.parse().ok()?,
            "--specialty-top" => args.specialty_top = Some(value()?.parse().ok()?),
            "--json" => args.json = true,
            "--soft-eps" => args.soft_eps = value()?.parse().ok()?,
            "--trace" => args.goods_trace = true,
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
    println!("  --trace          sectors/modern 场景逐轮逐商品打印申报、报价、尺度、成交、库存、投入产出");
    println!("  --block-from A --block-to B --block-polity P   在 [A,B) 轮封锁 P");
    println!("  --seed, -s S     随机种子（默认 11）");
}

fn spec(args: &Args) -> Spec {
    let mut spec = raw_spec(args);
    if let Some(capacity) = args.capacity {
        spec.capacity = capacity;
    }
    if let Some(factor) = args.specialty_top {
        spec = spec.with_specialty_at(GOODS - 1, factor);
    }
    spec
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
        .with_soft_eps(args.soft_eps)
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
        "技术阶梯：同一个部门里两个工艺。省料但慢 = 每件粮吃 {:.2} 件工业品、产能占用 {:.2}/件；费料但快 = {:.2} 件工业品、产能占用 {:.2}/件；产能预算 {}。本例切换点在 工业品价 ÷ 粮食价 ≈ 1.10",
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

/// 商品的短名，按 good 的索引
const GOOD_LABELS: [&str; GOODS] = ["一产", "二产", "三产"];

fn dash(value: f32) -> String {
    if value > 0.0 {
        format!("{value:.3}")
    } else {
        String::from("—")
    }
}

/// 价格专用：**科学计数**。价格会横跨 1e-30 ~ 1e30，定点格式（`{:.3}`）会把
/// 任何小于 0.0005 的值一律打成 `0.000`，于是"塌到零"和"只是很小"分不出来——
/// 这正好掩盖了 §16 那条链的关键一步。
fn sci(value: f32) -> String {
    if value.is_finite() && value > 0.0 {
        format!("{value:.3e}")
    } else {
        String::from("—")
    }
}

/// 一种商品这一轮「最低保本价」：所有能产它的工艺里，投入成本 ÷ 产出量 的最小值
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
                .map(|(k, consumption)| consumption.max(0.0) * prices.get(k).copied().unwrap_or(0.0))
                .sum();
            best = best.min(cost / output);
        }
    }
    if best.is_finite() {
        best
    } else {
        0.0
    }
}

fn report_by_good(lab: &Lab, ladder: bool) {
    let states = lab.good_states();
    let prices: Vec<f32> = states.iter().map(|state| state.price).collect();
    let bids: Vec<f32> = states
        .iter()
        .map(|state| if state.bid > 0.0 { state.bid } else { state.price })
        .collect();
    let asks: Vec<f32> = states
        .iter()
        .map(|state| if state.ask > 0.0 { state.ask } else { state.price })
        .collect();
    // 增值按**决策口径**估：产出用买价、投入用卖价。两边都用指数会和决策脱节。
    let added: Vec<f32> = states
        .iter()
        .enumerate()
        .map(|(k, state)| state.delivery * bids[k] - state.consumed * asks[k])
        .collect();
    let total: f32 = added.iter().sum();
    // 注意不能用 `total.max(1e-9)` 当除零护栏：总和为负时 max 会挑走 1e-9，
    // 占比立刻变成天文数字（实测 -5.95e11%）。要按绝对值判。
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
        println!("   {label:<8} {cells}");
    };
    println!(
        "{}投入产出阶梯：{} 轮后的价格（指数）与相对一产",
        if ladder { "有" } else { "无" },
        lab.round,
    );
    let first = prices.first().copied().unwrap_or(1.0).max(1e-9);
    let number = |value: f32| format!("{value:>9.3}");
    row("价格", &prices, number);
    row(
        "相对一产",
        &prices.iter().map(|price| price / first).collect::<Vec<f32>>(),
        number,
    );
    // 决策用的是账本（逐地方的边际买卖价），不是指数。两者必须并排看，
    // 否则会拿"指数口径的保本"去解释"账本口径的选择"。
    row("买价 bid", &bids, number);
    row("卖价 ask", &asks, number);
    row(
        "保本(按卖价)",
        &(0..GOODS).map(|k| break_even(lab, k, &asks)).collect::<Vec<f32>>(),
        number,
    );
    row(
        "保本(按指数)",
        &(0..GOODS).map(|k| break_even(lab, k, &prices)).collect::<Vec<f32>>(),
        number,
    );
    row(
        "产出入库",
        &states.iter().map(|state| state.delivery).collect::<Vec<f32>>(),
        number,
    );
    row(
        "投入消耗",
        &states.iter().map(|state| state.consumed).collect::<Vec<f32>>(),
        number,
    );
    row(
        "成交",
        &states.iter().map(|state| state.dealt).collect::<Vec<f32>>(),
        number,
    );
    row(
        "库存",
        &states.iter().map(|state| state.stock).collect::<Vec<f32>>(),
        |value| format!("{value:>9.1}"),
    );
    row(
        "增值",
        &added,
        |value| format!("{value:>9.2}"),
    );
    row(
        "增值占比",
        &added.iter().map(|value| share(*value)).collect::<Vec<f32>>(),
        |value| format!("{:>8.1}%", 100.0 * value),
    );
    let executions = lab.executions();
    println!(
        "   执行率   均值 {:.3}  最小 {:.3}   （共 {} 个部门）",
        executions.iter().sum::<f32>() / executions.len().max(1) as f32,
        executions.iter().cloned().fold(f32::INFINITY, f32::min),
        executions.len(),
    );
}

/// 阶梯陡度：`1 + (基准 − 1) × k`。k = 1 就是 `SECTOR_MOTIVE` 原样。
///
/// 注意**整体调高 `MOTIVE` 是没用的**——消费份额是 `motive/cost` 在消费族内归一化，
/// 全体同比放大不改变任何相对份额。能改变顶层相对地位的只有这个形状参数。
fn ladder_weights(args: &Args) -> Vec<f32> {
    SECTOR_MOTIVE
        .iter()
        .map(|motive| 1.0 + (motive - 1.0) * args.ladder_scale)
        .collect()
}


/// 一行 JSON = 一个采样轮次的**全部**状态，全精度（科学计数）。
///
/// 存在的理由：终端表格既有定点格式吞掉小数的问题（`{:.3}` 把 1e-15 打成 `0.000`），
/// 又只能人眼读、不能程序化分析。JSONL 让每一轮都能被脚本直接比对。
///
/// 维度：`goods`（逐商品聚合）· `departments`（逐部门逐商品的库存/目标/计划/执行率）
/// · `polities`（逐政体逐商品的楔子）· 总执行率。
fn json_line(lab: &Lab) -> String {
    let states = lab.good_states();
    let executions = lab.executions();
    let wedges = lab.wedges();
    let mut goods = String::new();
    for (k, s) in states.iter().enumerate() {
        if k > 0 {
            goods.push(',');
        }
        goods.push_str(&format!(
            concat!(
                "{{\"good\":{},\"index\":{:e},\"bid\":{:e},\"ask\":{:e},\"deal_price\":{:e},",
                "\"declared_sell\":{:e},\"declared_buy\":{:e},\"quote_sell\":{:e},\"quote_buy\":{:e},",
                "\"scale_sell\":{:e},\"scale_buy\":{:e},\"dealt\":{:e},\"stock\":{:e},",
                "\"delivery\":{:e},\"consumed\":{:e},\"intake\":{:e},",
                "\"sell_ceiling\":{:e},\"buy_ceiling\":{:e},\"target\":{:e},",
                "\"wanted_buy\":{},\"blocked_buy\":{}}}"
            ),
            k, s.price, s.bid, s.ask, s.deal_price,
            s.declared_sell, s.declared_buy, s.quote_sell, s.quote_buy,
            s.scale_sell, s.scale_buy, s.dealt, s.stock,
            s.delivery, s.consumed, s.intake,
            s.sell_ceiling, s.buy_ceiling, s.target,
            s.wanted_buy, s.blocked_buy,
        ));
    }
    let mut departments = String::new();
    for (i, warehouse) in lab.warehouses.warehouses.iter().enumerate() {
        if i > 0 {
            departments.push(',');
        }
        let stock: Vec<String> = warehouse
            .stocks
            .iter()
            .map(|s| format!("{:e}", s.volume))
            .collect();
        let target: Vec<String> = warehouse
            .stocks
            .iter()
            .map(|s| format!("{:e}", s.target_volume))
            .collect();
        let gap: Vec<String> = warehouse
            .stocks
            .iter()
            .map(|s| format!("{:e}", s.declared_gap))
            .collect();
        departments.push_str(&format!(
            "{{\"department\":{i},\"stock\":[{}],\"target\":[{}],\"gap\":[{}],\"intake\":{:e},\"execution\":{:e},\"capacity_scale\":{:e}}}",
            stock.join(","),
            target.join(","),
            gap.join(","),
            lab.departments.departments[i].intake().iter().sum::<f32>(),
            lab.departments.departments[i].policy_execution(),
            lab.departments.departments[i].capacity_scale(),
        ));
    }
    let mut polities = String::new();
    for (p, row) in wedges.iter().enumerate() {
        if p > 0 {
            polities.push(',');
        }
        let cells: Vec<String> = row.iter().map(|w| format!("{w:e}")).collect();
        polities.push_str(&format!(
            "{{\"polity\":{p},\"wedge\":[{}]}}",
            cells.join(","),
        ));
    }
    // **逐地方账本**：部门决策价读的就是它（`plan` 读 `books[locality]`），
    // 而指数是它的聚合。两者是否脱钩，只有把账本本身打出来才能看见。
    let mut books = String::new();
    for (locality, row) in lab.warehouses.books.iter().enumerate() {
        if locality > 0 {
            books.push(',');
        }
        let cells: Vec<String> = row
            .iter()
            .map(|b| format!("[{:e},{:e},{}]", b.bid, b.ask, b.observed as u8))
            .collect();
        books.push_str(&format!(
            "{{\"locality\":{locality},\"book\":[{}]}}",
            cells.join(","),
        ));
    }
    // **产出来自哪种政策**：`自有商品免费生产`（consumptions 全 0）还是`阶梯工艺`
    // （consumptions 非 0）。`入库` 的总量分不出这两者——而三产入库非零**并不能**
    // 证明 t2 在跑。
    let mut split_free = vec![0.0f32; lab.market.merchandises.len()];
    let mut split_ladder = vec![0.0f32; lab.market.merchandises.len()];
    for (i, department) in lab.departments.departments.iter().enumerate() {
        let scale = lab
            .departments
            .departments
            .get(i)
            .map(|d| d.capacity_scale())
            .unwrap_or(0.0);
        for policy in department.policies.iter() {
            if !policy.is_production() {
                continue;
            }
            let free = policy.consumptions.iter().all(|c| *c <= 0.0);
            for (k, produced) in policy.outputs.iter().enumerate() {
                let amount = policy.distribution() * produced * scale;
                if free {
                    split_free[k] += amount;
                } else {
                    split_ladder[k] += amount;
                }
            }
        }
    }
    let delivery_split: Vec<String> = split_free
        .iter()
        .zip(split_ladder.iter())
        .map(|(f, l)| format!("[{f:e},{l:e}]"))
        .collect();
    format!(
        concat!(
            "{{\"round\":{},\"goods\":[{}],\"departments\":[{}],\"polities\":[{}],",
            "\"books\":[{}],\"delivery_free_or_ladder\":[{}],",
            "\"execution_mean\":{:e},\"execution_min\":{:e},\"uncleared\":{:e}}}"
        ),
        lab.round,
        goods,
        departments,
        polities,
        books,
        delivery_split.join(","),
        executions.iter().sum::<f32>() / executions.len().max(1) as f32,
        executions.iter().cloned().fold(f32::INFINITY, f32::min),
        lab.history.last().map(|h| h.uncleared).unwrap_or(0.0),
    )
}

fn sectors(args: &Args) {
    for ladder in [false, true] {
        let mut local = args.clone();
        local.motive_ladder = ladder;
        let mut lab = build_with(&local, spec(&local));
        if args.json {
            for _ in 0..args.rounds {
                lab.step();
                if lab.round % args.every != 0 && lab.round != args.rounds {
                    continue;
                }
                println!("{}", json_line(&lab));
            }
            continue;
        }
        if args.goods_trace {
            println!(
                "=== 场景 {} 逐轮逐商品追踪（{} 轮，每 {} 轮一行）：{} ===",
                args.scenario,
                args.rounds,
                args.every,
                if ladder { "有意愿阶梯" } else { "无意愿阶梯" },
            );
            println!(
                "{:>4} {:>5} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9} {:>8} {:>8} {:>8} {:>8} {:>9} {:>9} {:>10} {:>10} {:>5} {:>5} {:>10}",
                "轮次",
                "商品",
                "指数",
                "买价",
                "卖价",
                "卖申报",
                "买申报",
                "成交",
                "成交价",
                "库存",
                "卖报价",
                "买报价",
                "买尺度",
                "入库",
                "投入",
                "卖天花板",
                "买天花板",
                "想买",
                "被拒",
                "目标",
            );
        }
        for _ in 0..args.rounds {
            lab.step();
            if !args.goods_trace || (lab.round % args.every != 0 && lab.round != args.rounds) {
                continue;
            }
            for (k, state) in lab.good_states().iter().enumerate() {
                println!(
                    "{:>4} {:>5} {:>11.3e} {:>11} {:>11} {:>9.2} {:>9.2} {:>9.2} {:>11} {:>9.1} {:>11} {:>11} {:>9.2} {:>9.3} {:>9.3} {:>10.1} {:>10.1} {:>5.0} {:>5.0} {:>12.2}",
                    lab.round,
                    GOOD_LABELS[k],
                    state.price,
                    sci(state.bid),
                    sci(state.ask),
                    state.declared_sell,
                    state.declared_buy,
                    state.dealt,
                    sci(state.deal_price),
                    state.stock,
                    sci(state.quote_sell),
                    sci(state.quote_buy),
                    dash(state.scale_buy),
                    state.delivery,
                    state.consumed,
                    state.sell_ceiling,
                    state.buy_ceiling,
                    state.wanted_buy,
                    state.blocked_buy,
                    state.target,
                );
            }
            let executions = lab.executions();
            println!(
                "     {:>5} 执行率 均值 {:.3} 最小 {:.3}   未成交 {:>5.1}%",
                "小结",
                executions.iter().sum::<f32>() / executions.len().max(1) as f32,
                executions.iter().cloned().fold(f32::INFINITY, f32::min),
                100.0 * lab.history.last().unwrap().uncleared,
            );
        }
        if args.goods_trace {
            println!();
        }
        report_by_good(&lab, ladder);
        println!();
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
        "{:>7} {:>10} {:>12} {:>12} {:>12} {:>12} {:>10}",
        "权重", "对外成交", "制裁政权价", "其余政权价", "价差(成交)", "价差(账本)", "执行率"
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
            "{weight:>7.2} {:>10.2} {:>12.3} {:>12.3} {:>+10.3} {:>+10.3} {:>10.2}",
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
            lab.book_spread(seat, 0),
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
