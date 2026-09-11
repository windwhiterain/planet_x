//! 科技干线（MOND 掌握度）观测探针（全部 `#[ignore]`：只打印、不断言）。
//!
//! * `probe_mond_control`：掌握度随「飞船在异常区」的涨落——谁在学、学到多少、
//!   有没有人锈回去、前沿挪到了哪里。
//! * `probe_mond_carrier`：连续化之后**承运垄断还在不在**（抽成按掌握度折算）。
//!
//! 跑法：
//! ```text
//! cargo test --test tech_probe -- --ignored --nocapture
//! PROBE_ROUNDS=800 PROBE_SEEDS=7,42 cargo test --test tech_probe -- --ignored --nocapture
//! ```

use planet_x::config::load_config;
use planet_x::model::*;
use planet_x::prng::Prng;
use planet_x::sim;
use planet_x::world;
use std::collections::BTreeMap;

fn rounds() -> u32 {
    std::env::var("PROBE_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(400)
}

fn seeds() -> Vec<u64> {
    std::env::var("PROBE_SEEDS")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![7])
}

/// 1) 掌握度轨迹：涨 / 平 / 锈，以及它在世界里的**分化程度**。
#[test]
#[ignore]
fn probe_mond_control() {
    let config = load_config();
    let n = rounds();
    let checkpoints: Vec<u32> = [40, 120, 240, 400, 600, 800, 1000]
        .into_iter()
        .filter(|c| *c <= n)
        .collect();
    println!(
        "== MOND 掌握度（前沿 0 → {:.1} AU；在场强度=Σ(1+深度×{})）==",
        sim::mond_frontier(&config, 0.0),
        config.mond.knowledge.depth_weight
    );
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let names: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
        let mut in_band_rounds: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut presence_sum: BTreeMap<FactionId, f64> = BTreeMap::new();
        let mut peak: BTreeMap<FactionId, f64> = BTreeMap::new();
        let mut trail: BTreeMap<FactionId, Vec<(u32, f64)>> = BTreeMap::new();
        let mut crossed: BTreeMap<FactionId, bool> = BTreeMap::new();

        for r in 1..=n {
            sim::advance(&mut state, &config, &mut rng);
            for name in &names {
                let presence = sim::mond_presence(&state, &config, name);
                if presence > 0.0 {
                    *in_band_rounds.entry(name.clone()).or_insert(0) += 1;
                }
                *presence_sum.entry(name.clone()).or_insert(0.0) += presence;
                let c = sim::mond_control(&state, name);
                let p = peak.entry(name.clone()).or_insert(0.0);
                *p = p.max(c);
                if c > 0.35 {
                    crossed.insert(name.clone(), true);
                }
                if checkpoints.contains(&r) {
                    trail.entry(name.clone()).or_default().push((r, c));
                }
            }
        }

        println!("-- seed {seed}（{n} 回合）--");
        println!(
            "   {:<14} {:>6} {:>7} {:>7} {:>8}  {}",
            "势力", "峰值", "终值", "在场率", "平均在场", "轨迹(回合:掌握度 / 前沿 AU)"
        );
        for name in &names {
            let c = sim::mond_control(&state, name);
            let presence_rate = *in_band_rounds.get(name).unwrap_or(&0) as f64 / n.max(1) as f64;
            let presence_mean = presence_sum.get(name).copied().unwrap_or(0.0) / n.max(1) as f64;
            let trail_str: Vec<String> = trail
                .get(name)
                .map(|v| {
                    v.iter()
                        .map(|(r, c)| format!("{r}:{c:.2}/{:.0}", sim::mond_frontier(&config, *c)))
                        .collect()
                })
                .unwrap_or_default();
            println!(
                "   {:<14} {:>6.2} {:>7.2} {:>6.0}% {:>8.2}  {}",
                name,
                peak.get(name).copied().unwrap_or(0.0),
                c,
                presence_rate * 100.0,
                presence_mean,
                trail_str.join(" ")
            );
        }
        println!("   越过 0.35 的势力：{} / {}", crossed.len(), names.len());
    }
}

/// 3) 诊断用：长局末「定制化舰」（装了组件的舰）还剩几条——`horizon_mid` 的非空守卫
/// 押的就是这个数（它在 M2 之后翻成 0，这里量清楚是「舰死光了」还是「造不出带组件的舰」）。
#[test]
#[ignore]
fn probe_customized_ships() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut spawned = 0u32;
        let mut spawned_with_bp = 0u32;
        let mut peak_ships = 0usize;
        let mut peak_fitted = 0usize;
        let mut last_fitted_round = 0u32;
        for r in 1..=n {
            sim::advance(&mut state, &config, &mut rng);
            for e in &state.events {
                if let GameEvent::ShipSpawned { blueprint, .. } = e {
                    spawned += 1;
                    if blueprint.is_some() {
                        spawned_with_bp += 1;
                    }
                }
            }
            let live = state.ships.iter().filter(|s| s.hull > 0.0).count();
            let fitted = state
                .ships
                .iter()
                .filter(|s| s.hull > 0.0 && !s.components.is_empty())
                .count();
            peak_ships = peak_ships.max(live);
            if fitted > 0 {
                last_fitted_round = r;
            }
            peak_fitted = peak_fitted.max(fitted);
        }
        println!(
            "== seed {seed}（{n} 回合）：出厂 {spawned}（带设计图 {spawned_with_bp}）｜\
             活舰峰值 {peak_ships}｜带组件的舰峰值 {peak_fitted}｜\
             末回合活舰 {} 带组件 {}｜最后一次见到带组件的舰 r{last_fitted_round}",
            state.ships.iter().filter(|s| s.hull > 0.0).count(),
            state
                .ships
                .iter()
                .filter(|s| s.hull > 0.0 && !s.components.is_empty())
                .count(),
        );
    }
}

/// 2) 承运垄断：抽成按掌握度折算之后，深空线上的钱还是不是落在掌握者手里。
#[test]
#[ignore]
fn probe_mond_carrier() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut income: BTreeMap<FactionId, f64> = BTreeMap::new();
        let mut control_sum: BTreeMap<FactionId, f64> = BTreeMap::new();
        let mut rounds_with_carrier = 0u32;
        for _ in 1..=n {
            let d = sim::advance(&mut state, &config, &mut rng);
            let mut any = false;
            for (fid, row) in &d.factions {
                *income.entry(fid.clone()).or_insert(0.0) += row.carrier_income;
                if row.carrier_income > 0.0 {
                    any = true;
                }
            }
            if any {
                rounds_with_carrier += 1;
            }
            for f in &state.factions {
                *control_sum.entry(f.name.clone()).or_insert(0.0) += f.mond_control;
            }
        }
        println!(
            "== 承运抽成 seed {seed}（{n} 回合，有承运收入的回合 {rounds_with_carrier}/{n}）=="
        );
        for f in &state.factions {
            println!(
                "   {:<14} 累计抽成 {:>10.2}   平均掌握度 {:.3}   终值 {:.3}   在带内舰 {:>2}",
                f.name,
                income.get(&f.name).copied().unwrap_or(0.0),
                control_sum.get(&f.name).copied().unwrap_or(0.0) / n.max(1) as f64,
                f.mond_control,
                state
                    .ships
                    .iter()
                    .filter(|s| {
                        s.hull > 0.0
                            && s.faction_id == f.name
                            && sim::dist(s.position, [0.0, 0.0]) > config.mond.radius
                    })
                    .count()
            );
        }
    }
}
