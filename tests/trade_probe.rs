//! 贸易 / 稀缺 / 制裁 观测探针（全部 `#[ignore]`：只打印、不断言）。
//!
//! * `probe_mineral_dependence`：谁**结构上**缺什么矿（挖得到的 vs 船坞要的）。
//! * `probe_market_prices`：真实市场的**价格轨迹**（基价的几倍）、挂单/成交、每势力净进口、
//!   出厂舰——这是「缺某种矿 → 超高价 → 买不到」是否真的发生的观察面。
//! * `probe_landless`：「无城但有舰」的流亡态持续多久。
//!
//! 跑法：
//! ```text
//! cargo test --test trade_probe -- --ignored --nocapture
//! PROBE_ROUNDS=1000 PROBE_SEEDS=1,42 cargo test --test trade_probe -- --ignored --nocapture
//! ```

use planet_x::config::load_config;
use planet_x::model::*;
use planet_x::prng::Prng;
use planet_x::sim;
use planet_x::world;
use std::collections::{BTreeMap, BTreeSet};

fn rounds() -> u32 {
    std::env::var("PROBE_ROUNDS").ok().and_then(|s| s.parse().ok()).unwrap_or(400)
}

fn seeds() -> Vec<u64> {
    std::env::var("PROBE_SEEDS")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![7])
}

fn value_of(config: &GameConfig, rt: &str) -> f64 {
    config.resources.get(rt).map(|r| r.value).unwrap_or(1.0)
}

fn stock_value(state: &State, config: &GameConfig, fid: &str) -> f64 {
    state
        .faction(fid)
        .map(|f| f.resources.iter().map(|(k, v)| v * value_of(config, k)).sum())
        .unwrap_or(0.0)
}

/// 该势力**能挖到**的资源集合（其所有活城定居点的矿藏并集）。
fn minable(state: &State, fid: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for c in state.cities.iter().filter(|c| c.faction_id == fid && !c.razed) {
        if let Some(s) = state.city_settlement(&c.name) {
            for d in &s.resources {
                out.insert(d.resource.clone());
            }
        }
    }
    out
}

/// 该势力船坞**需要**的资源集合（只由已定 `ship_type` 的建造区决定）+ 未定舰级的建造区数。
fn yard_need(state: &State, config: &GameConfig, fid: &str) -> (BTreeSet<String>, usize) {
    let mut out = BTreeSet::new();
    let mut classless = 0usize;
    for c in state.cities.iter().filter(|c| c.faction_id == fid && !c.razed) {
        for b in &c.buildings {
            if !b.is_shipyard() {
                continue;
            }
            match &b.ship_type {
                Some(cls) => {
                    for (rt, _) in &config.ship_spec(cls).build_cost {
                        out.insert(rt.clone());
                    }
                }
                None => classless += 1,
            }
        }
    }
    (out, classless)
}

fn need_of_all_classes(config: &GameConfig) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for cls in config.ships.keys() {
        for (rt, _) in &config.ship_spec(cls).build_cost {
            out.insert(rt.clone());
        }
    }
    out
}

fn live_cities(state: &State, fid: &str) -> usize {
    state.cities.iter().filter(|c| c.faction_id == fid && !c.razed).count()
}

fn live_ships(state: &State, fid: &str) -> usize {
    state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0).count()
}

/// 1) 结构依赖：每个势力**挖得到**什么、全部舰级要什么、因此结构上必须进口什么。
#[test]
#[ignore]
fn probe_mineral_dependence() {
    let config = load_config();
    let state = world::default_state(&config, 7);
    let all = need_of_all_classes(&config);
    println!("== 结构依赖（t=0，seed 7）==");
    println!("全部舰级所需资源全集: {all:?}");
    for f in &state.factions {
        let have = minable(&state, &f.name);
        let (need, classless) = yard_need(&state, &config, &f.name);
        let must_trade: Vec<String> = all.difference(&have).cloned().collect();
        let exports: Vec<String> = have.difference(&all).cloned().collect();
        println!(
            "  {:<14} 城={} 舰={} 库存价值={:>7.1} 未定舰级建造区={}  本势力船坞需求={:?}\n                 挖得到={:?}\n                 全集下必须进口={:?}  可出口={:?}",
            f.name,
            live_cities(&state, &f.name),
            live_ships(&state, &f.name),
            stock_value(&state, &config, &f.name),
            classless,
            need,
            have,
            must_trade,
            exports
        );
    }
}

/// 2) 真实市场：价格（基价的几倍）/ 挂单 / 成交 / 每势力净进口 / 出厂舰。
#[test]
#[ignore]
fn probe_market_prices() {
    let config = load_config();
    let n = rounds();
    let checkpoints: Vec<u32> = [1, 5, 20, 60, 120, 240, 400, 600, 800, 1000, 1500, 2000]
        .into_iter()
        .filter(|c| *c <= n)
        .collect();
    let order: Vec<String> = config.resources.keys().cloned().collect();

    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        // 累计：每势力的净进口、成交额。
        let mut cum_net: BTreeMap<FactionId, f64> = BTreeMap::new();
        let mut cum_settled_value = 0.0f64;
        let mut cum_spawns: BTreeMap<FactionId, u32> = BTreeMap::new();
        println!("== 真实市场 seed {seed}（{n} 回合）==");
        print!("  资源基价:");
        for rt in &order {
            print!(" {rt}={:.1}", value_of(&config, rt));
        }
        println!();
        for r in 1..=n {
            let d = sim::advance(&mut state, &config, &mut rng);
            for e in &state.events {
                if let GameEvent::ShipSpawned { owner, .. } = e {
                    *cum_spawns.entry(owner.clone()).or_insert(0) += 1;
                }
            }
            let settled = &d.metrics.market_settled;
            let offered = &d.metrics.market_offered;
            let sv: f64 = settled
                .iter()
                .map(|(rt, amt)| amt * d.metrics.market_price.get(rt).copied().unwrap_or_else(|| value_of(&config, rt)))
                .sum();
            cum_settled_value += sv;
            for (fid, v) in &d.metrics.market_net_import {
                *cum_net.entry(fid.clone()).or_insert(0.0) += *v;
            }
            if checkpoints.contains(&r) {
                print!("  r{r:<5} 价格倍数:");
                for rt in &order {
                    let p = d.metrics.market_price.get(rt).copied().unwrap_or_else(|| value_of(&config, rt));
                    let base = value_of(&config, rt);
                    print!(" {rt}={:.2}", if base > 0.0 { p / base } else { 1.0 });
                }
                println!();
                print!("         挂单:");
                for rt in &order {
                    print!(" {rt}={:.0}", offered.get(rt).copied().unwrap_or(0.0));
                }
                println!();
                print!("         成交:");
                for rt in &order {
                    print!(" {rt}={:.1}", settled.get(rt).copied().unwrap_or(0.0));
                }
                println!("   成交额={sv:.1} 世界库存价值={:.0}", state.factions.iter().map(|f| stock_value(&state, &config, &f.name)).sum::<f64>());
            }
        }
        println!("  累计成交额={cum_settled_value:.0}");
        println!("  累计出厂舰: {cum_spawns:?}");
        println!("  末态:");
        for f in &state.factions {
            println!(
                "    {:<14} 城={:<3} 舰={:<3} 库存价值={:>8.1} 累计净进口={:>8.1}",
                f.name,
                live_cities(&state, &f.name),
                live_ships(&state, &f.name),
                stock_value(&state, &config, &f.name),
                cum_net.get(&f.name).copied().unwrap_or(0.0)
            );
        }
    }
}

/// 3) 流亡态：势力在「无城」状态下能撑多久、靠什么撑。
#[test]
#[ignore]
fn probe_landless() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut longest: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut cur: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut total: BTreeMap<FactionId, u32> = BTreeMap::new();
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
            for f in &state.factions {
                let lc = live_cities(&state, &f.name);
                let e = cur.entry(f.name.clone()).or_insert(0);
                if lc == 0 {
                    *e += 1;
                    *total.entry(f.name.clone()).or_insert(0) += 1;
                    let l = longest.entry(f.name.clone()).or_insert(0);
                    *l = (*l).max(*e);
                } else {
                    *e = 0;
                }
            }
        }
        println!("== 流亡态 seed {seed}（{n} 回合）==");
        for f in &state.factions {
            println!(
                "  {:<14} 无城总回合={:<4} 最长连续无城={:<4} 末态: 城={} 舰={} 库存价值={:.1}",
                f.name,
                total.get(&f.name).copied().unwrap_or(0),
                longest.get(&f.name).copied().unwrap_or(0),
                live_cities(&state, &f.name),
                live_ships(&state, &f.name),
                stock_value(&state, &config, &f.name)
            );
        }
    }
}
