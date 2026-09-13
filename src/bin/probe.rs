use planet_x::department::probe::{self, Treasury};

fn main() {
    let trace = std::env::args().any(|argument| argument == "--trace");
    let learning = std::env::args().any(|argument| argument == "--learning");
    if learning {
        for (regime, treasury) in [
            ("国内", Treasury::Redistributing),
            ("国际", Treasury::Free),
        ] {
            for (mode, fixed_slope) in [("阶数可学", false), ("阶数钉死", true)] {
                for forgetting in probe::LEARNING_RATES {
                    for (label, delta) in [("同相", 0.0f32), ("递相", 2.0 * std::f32::consts::PI / 3.0)]
                    {
                        let report =
                            probe::learning_sweep(treasury, forgetting, fixed_slope, 0.25, delta);
                        println!(
                            "{regime} {mode} 遗忘 {:.2} {label}：成交率 {:>5.1}% 价格 {:.3}~{:.3} 漂移 {:+.1}% 学漂移 {:+.1}% 库存 {:.1}~{:.1} 冻结 {} [{}]",
                            forgetting,
                            100.0 - report.uncleared,
                            report.min_price,
                            report.max_price,
                            100.0 * report.drift,
                            100.0 * report.drift_online,
                            report.min_stock,
                            report.max_stock,
                            report.frozen,
                            report.verdict(),
                        );
                    }
                }
            }
        }
        return;
    }
    let amplitude = 0.25;
    for (regime, treasury) in [
        ("国内经济循环：回收货币，统一重新发放", Treasury::Redistributing),
        ("国际经济循环：不收回，不发放", Treasury::Free),
    ] {
        println!("{regime}（振幅 {amplitude}）");
        for period in probe::SWEEP_PERIODS {
            for (sign, offset) in [("正", 0.0f32), ("负", std::f32::consts::PI)] {
                for delta in probe::SWEEP_PHASES {
                    let report = probe::sweep_offset(treasury, period, delta, offset, amplitude);
                    println!(
                        "  {sign}正弦 周期 {:>4} 相位差 {:>5.0}° 成交率 {:>5.1}% | 价格 {:.3}~{:.3} 漂移 {:+.1}% 学漂移 {:+.1}% 库存 {:.1}~{:.1} 冻结 {} [{}]",
                        period,
                        delta.to_degrees(),
                        100.0 - report.uncleared,
                        report.min_price,
                        report.max_price,
                        100.0 * report.drift,
                        100.0 * report.drift_online,
                        report.min_stock,
                        report.max_stock,
                        report.frozen,
                        report.verdict(),
                    );
                }
            }
        }
        println!();
    }

    if trace {
        for (regime, treasury) in [
            ("国内", Treasury::Redistributing),
            ("国际", Treasury::Free),
        ] {
            println!("{regime} 周期 6 同相 逐轮明细");
            for round in probe::trace(treasury, 6.0, 0.25, probe::TRACE_PHASES, 48) {
                println!(
                    "  第 {:>2} 轮 价格 [{}] 库存 {} 目标 {} 净成交 {}",
                    round.round,
                    join(&round.prices),
                    (0..3)
                        .map(|i| format!("[{}]", join(&round.stocks[i])))
                        .collect::<Vec<String>>()
                        .join(" "),
                    (0..3)
                        .map(|i| format!("[{}]", join(&round.targets[i])))
                        .collect::<Vec<String>>()
                        .join(" "),
                    (0..3)
                        .map(|i| format!("[{}]", join(&round.net[i])))
                        .collect::<Vec<String>>()
                        .join(" "),
                );
                for i in 0..3 {
                    println!("        {}", round.curve(i).line());
                }
                println!("        商品0 {}", round.quotes(0));
            }
            println!();
        }
    }
}

fn join(values: &[f32]) -> String {
    values
        .iter()
        .map(|value| format!("{value:+.2}"))
        .collect::<Vec<String>>()
        .join(" ")
}
