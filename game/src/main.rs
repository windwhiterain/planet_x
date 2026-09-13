use std::path::PathBuf;

use game::{DEPARTMENT_NAMES, DomesticEconomy, GOOD_NAMES, GOODS};

fn main() {
    let mut rounds = 40usize;
    let mut seed = 11u64;
    let mut trace = false;
    let mut fluctuation = 0.0f32;
    let mut record: Option<PathBuf> = None;

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
            "--fluctuation" | "-f" => {
                fluctuation = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(fluctuation);
            }
            "--record" => {
                record = args.next().map(PathBuf::from);
            }
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

    if let Some(path) = record {
        write_stream(&path, rounds, seed, fluctuation);
        return;
    }

    let mut economy = DomesticEconomy::new(seed).with_fluctuation(fluctuation);
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

fn write_stream(path: &std::path::Path, rounds: usize, seed: u64, fluctuation: f32) {
    let mut economy = DomesticEconomy::new(seed).with_fluctuation(fluctuation);
    economy.run(rounds);

    let mut frames = vec![px_protocol::Frame::Protocol(px_protocol::ProtocolId::local())];
    frames.extend(
        economy
            .history
            .iter()
            .map(|snapshot| px_protocol::Frame::World(game::project::world_view(snapshot))),
    );

    let mut bytes = Vec::new();
    px_protocol::stream::write_stream(&mut bytes, &frames).expect("写流失败");
    std::fs::write(path, &bytes).expect("落盘失败");

    let id = px_protocol::ProtocolId::local();
    println!(
        "录制 {} 帧（{} 轮、种子 {seed}）→ {}（{} 字节）",
        frames.len(),
        rounds,
        path.display(),
        bytes.len(),
    );
    println!(
        "协议版本 {}、指纹 {}、git {}",
        id.schema_version,
        px_protocol::protocol_hash_hex(),
        id.git_rev,
    );
}

fn usage() {
    println!("用法：game [--rounds N] [--seed S] [--trace] [--fluctuation F] [--record PATH]");
    println!("  --rounds, -n       模拟轮数（默认 40）");
    println!("  --seed,   -s       随机种子（默认 11）");
    println!("  --trace            每轮打印各部门的收付与库存");
    println!("  --fluctuation, -f  仓库波动（默认 0；为 0 时世界与 seed 无关）");
    println!("  --record           把每轮的 WorldView 录成 .pxstream（供渲染器离线消费）");
}
