//! MOND 异常导航：那条只打印的探针。
//!
//! ## 2026-10（第 7 批）：两条纯函数测搬去了 g1
//!
//! `mond_drift` / `mond_arrival_chance` 都**只吃 `config`** ⇒ 新挂 `--call` 后逐值可复现：
//!
//! | 原用例 | 现在住 |
//! | --- | --- |
//! | `mond_drift_misses_in_anomaly_but_masters_are_exact` | g1「非掌握者会偏 / `roll=0` 必然蒙对 / 幅度随 roll 单调 / 掌握者精确」 |
//! | `mond_depth_only_costs_attempts_never_makes_it_impossible` | g1「任意有限深度都还有胜算」+「胜算随深度单调不增（100 个采样点）」+「一次到位的门槛 = `arrival_eps ÷ drift_per_au`」 |
//!
//! ⚠ 原件还有一段「把 `drift_per_au` 硬改成 50 ⇒ 胜算 < 0.01」的鲁棒性断言，那要**换一份 config**
//! ——`--call` 用的是出货配置，读面没有 config 覆盖入口，所以那半没搬（核心主张「永远 > 0」已在上面的判据里）。
//!
//! ## 2026-10（第 7 批）：`route_depth_measures_mond_immersion` 搬去了 g1
//!
//! 新挂了 `--call route_depth`（吃天体名或 `[x, y]`）⇒ 原件那四个断言**逐字可复现**
//! （半径从 `meta.mond.radius` 读，不用挑天体）：内侧↔内侧 = 0 / 一端 10 AU 深 = 10 /
//! 两端都深 = 按**较浅**那端算（2 AU）/ 双向对称。

use super::*;

/// 探针（`cargo test --lib probe_mond_attempts -- --ignored --nocapture`）：
/// 用**真实的** [`nav_roll`] 逐回合实测「深处目标要试几个回合」——闭式 `p` 是「单次尝试
/// 的命中率」，这里量的是它**在真实伪随机序列上**的表现（首次命中的回合数、1000 回合里的
/// 命中次数），并且验证**没有任何天体是 0 命中**（= 不存在进不去的目标）。
#[test]
#[ignore]
fn probe_mond_attempts() {
    let (config, state) = fresh_world(42);
    let m = &config.mond;
    let eps = config.combat.arrival_eps;
    println!("== MOND 导航尝试实测（ship=朝圣者, fid=中国；闭式 p = eps/(depth×drift)）==");
    println!(
        "  {:<8} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "天体", "深度AU", "闭式p", "闭式期望", "首次命中", "1000次命中", "命中率"
    );
    for b in &state.bodies {
        let r = (b.position[0] * b.position[0] + b.position[1] * b.position[1]).sqrt();
        if r <= m.radius {
            continue;
        }
        let depth = r - m.radius;
        let p = mond_arrival_chance(&config, depth, 0.0);
        let mut first: Option<u32> = None;
        let mut hits = 0u32;
        let n = 1000u32;
        for round in 0..n {
            let roll = nav_roll("中国", "朝圣者", round);
            if dist(mond_drift(&config, 0.0, b.position, roll), b.position) <= eps {
                hits += 1;
                first.get_or_insert(round + 1);
            }
        }
        println!(
            "  {:<8} {:>7.2} {:>7.3} {:>9.1} {:>9} {:>9} {:>9.3}",
            b.name,
            depth,
            p,
            if p > 0.0 { 1.0 / p } else { f64::INFINITY },
            first
                .map(|f| f.to_string())
                .unwrap_or_else(|| "从未".into()),
            hits,
            hits as f64 / n as f64
        );
        assert!(
            hits > 0,
            "{} 必须至少命中一次——不存在永远进不去的目标",
            b.name
        );
    }
}
