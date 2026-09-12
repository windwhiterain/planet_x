use game::{DEPARTMENT_NAMES, DomesticEconomy, GOOD_NAMES, GOODS};

fn main() {
    let mut rounds = 40usize;
    let mut seed = 11u64;
    let mut trace = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rounds" | "-n" => {
                rounds = args.next().and_then(|value| value.parse().ok()).unwrap_or(rounds);
            }
            "--seed" | "-s" => {
                seed = args.next().and_then(|value| value.parse().ok()).unwrap_or(seed);
            }
            "--trace" => trace = true,
            "--help" | "-h" => {
                usage();
                return;
            }
            other => {
                eprintln!("未知参数：{other}");
                usage();
                return;
            }
        }
    }

    let mut economy = DomesticEconomy::new(seed);
    println!("国内经济循环：{} 个部门、{} 种商品、{rounds} 轮、种子 {seed}", DEPARTMENT_NAMES.len(), GOODS);
    println!(
        "中央每轮拨款 {}，花不完的收回国库；部门生产自己的商品，政策只消耗资源",
        economy.departments.grants.iter().sum::<f32>(),
    );
    println!();

    println!(
        "{:>4} {:>9} {:>28} {:>9} {:>10} {:>9} {:>9} {:>9}",
        "轮次", "物价指数", "价格(粮食/工业品/服务)", "产出", "政策消耗", "拨款", "国库", "执行率"
    );
    for _ in 0..rounds {
        economy.step();
        let snapshot = economy.history.last().unwrap();
        println!(
            "{:>4} {:>9.2} {:>28} {:>9.2} {:>10.2} {:>9.2} {:>9.2} {:>9}",
            snapshot.round,
            snapshot.cpi,
            GOOD_NAMES
                .iter()
                .enumerate()
                .map(|(k, _)| format!("{:.4}", snapshot.prices[k]))
                .collect::<Vec<String>>()
                .join("/"),
            snapshot.output,
            snapshot.consumption,
            snapshot.granted,
            snapshot.treasury,
            snapshot
                .executions
                .iter()
                .map(|execution| format!("{execution:.2}"))
                .collect::<Vec<String>>()
                .join("/"),
        );
        if trace {
            for (i, name) in DEPARTMENT_NAMES.iter().enumerate() {
                println!(
                    "       {name} 收 {:>7.2} 付 {:>7.2} 库存 {}",
                    snapshot.revenues[i],
                    snapshot.payments[i],
                    (0..GOODS)
                        .map(|k| format!("{:.2}", snapshot.holdings[i][k]))
                        .collect::<Vec<String>>()
                        .join("/"),
                );
            }
        }
    }

    let last = economy.history.last().unwrap();
    let settled = &economy.history[economy.history.len().min(2).max(1)];
    println!();
    println!("结算：");
    println!(
        "  国库累计 {:.2} = 拨款 {:.2} × {} 轮，未花完的全部收回",
        last.treasury,
        last.granted,
        last.round,
    );
    println!(
        "  政策消耗（国内最终使用）每轮 {:.2}，产出每轮 {:.2}，成交额每轮 {:.2}",
        settled.consumption, settled.output, settled.turnover,
    );
    println!(
        "  首轮出清后物价指数 {:.2}，末轮 {:.2}；库存合计 {} -> {}",
        settled.cpi,
        last.cpi,
        settled
            .holdings
            .iter()
            .map(|holding| holding.iter().sum::<f32>())
            .sum::<f32>(),
        last.holdings
            .iter()
            .map(|holding| holding.iter().sum::<f32>())
            .sum::<f32>(),
    );
}

fn usage() {
    println!("用法：game [--rounds N] [--seed S] [--trace]");
    println!("  --rounds, -n  模拟轮数（默认 40）");
    println!("  --seed,   -s  随机种子（默认 11）");
    println!("  --trace       每轮打印各部门的收付与库存");
}
