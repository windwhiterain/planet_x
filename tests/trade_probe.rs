//! 贸易 / 稀缺 / 制裁 观测探针（全部 `#[ignore]`：只打印、不断言）。
//!
//! 目的：在设计「真实市场（稀缺 → 高价 → 不卖给你）」之前，先量出**现有**
//! `sim::step_market` 的真实行为：
//!
//! 1. `probe_mineral_dependence`：谁**结构上**缺什么矿（挖得到的 vs 船坞要的）。
//! 2. `probe_market_flow`：逐回合——真的缺料吗？市场补得上吗（fill scale）？
//!    限额（`auto_trade_limit`）与「卖无可卖」哪个先卡住？最终有没有船出厂？
//! 3. `probe_landless`：「无城但有舰」的流亡态持续多久（= 用贸易养活无立足点势力的前提）。
//!
//! 口径与 `step_market` **逐字同源**：`need` 只由**已定舰级**的建造区决定；
//! `surplus` 只算「不在 need 里、且高于 working_buffer」的部分；`limit` 用
//! `auto_trade_limit`（探针不复制制裁倍率，被制裁的霸权需另行扣除）。
//!
//! 跑法：
//! ```text
//! cargo test --test trade_probe -- --ignored --nocapture
//! PROBE_ROUNDS=1000 PROBE_SEEDS=1,42,12345 cargo test --test trade_probe -- --ignored --nocapture
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

/// 该势力船坞**需要**的资源集合——**与 `step_market` 同口径**：只由已定 `ship_type`
/// 的建造区决定。返回 (need, 未定舰级的建造区数)。
///
/// 注意：「未定舰级的建造区」是真实存在的状态（`build_city` 规划新建造区面积时
/// `ship_type = None`）——它既不参与 `step_market` 的 `need`，也不参与 `build_city`
/// 的造舰（无舰级 = 不造舰），所以是一个**只存在不产出**的建造区。
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

/// 全部舰级所需资源的并集（「谁必须靠贸易」的结构表用得上）。
fn need_of_all_classes(config: &GameConfig) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for cls in config.ships.keys() {
        for (rt, _) in &config.ship_spec(cls).build_cost {
            out.insert(rt.clone());
        }
    }
    out
}

/// 富余价值——与 `step_market` 第 3 步同口径（不在 `need` 里、且高于 `working_buffer`）。
fn sellable_value(state: &State, config: &GameConfig, fid: &str, need: &BTreeSet<String>) -> f64 {
    let floor = config.market.working_buffer;
    let Some(f) = state.faction(fid) else { return 0.0 };
    f.resources
        .iter()
        .filter(|(rt, _)| !need.contains(*rt))
        .map(|(rt, amt)| (amt - floor).max(0.0) * value_of(config, rt))
        .sum()
}

/// 补齐 `need` 到 `working_buffer` 所需价值——与 `step_market` 第 2 步同口径。
fn deficit_value(state: &State, config: &GameConfig, fid: &str, need: &BTreeSet<String>) -> f64 {
    let buf = config.market.working_buffer;
    let Some(f) = state.faction(fid) else { return 0.0 };
    need.iter()
        .map(|rt| (buf - f.resources.get(rt).copied().unwrap_or(0.0)).max(0.0) * value_of(config, rt))
        .sum()
}

/// 复刻 `step_market` 的第 4 步：本回合这笔交易实际能成交的比例 `scale`。
fn market_fill_scale(state: &State, config: &GameConfig, fid: &str, need: &BTreeSet<String>) -> f64 {
    let m = &config.market;
    let buy = deficit_value(state, config, fid, need);
    if buy <= 1e-6 {
        return 1.0;
    }
    let sell = sellable_value(state, config, fid, need);
    if sell <= 1e-6 {
        return 0.0;
    }
    (buy.min(m.auto_trade_limit).min(sell / (1.0 + m.spread)) / buy).clamp(0.0, 1.0)
}

fn live_cities(state: &State, fid: &str) -> usize {
    state.cities.iter().filter(|c| c.faction_id == fid && !c.razed).count()
}

fn live_ships(state: &State, fid: &str) -> usize {
    state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0).count()
}

/// 1) 结构依赖：每个势力**挖得到**什么、**全部舰级**要什么、因此结构上必须进口什么。
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

/// 2) 市场流：缺料频率、市场的成交比例（fill scale）、限额/卖无可卖谁先卡住、出厂舰数。
#[test]
#[ignore]
fn probe_market_flow() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);

        #[derive(Default, Clone)]
        struct Acc {
            yard_rounds: u32,
            /// 某个 need 资源库存 == 0 的回合数（回合末，建设/造舰已经花过钱）。
            starve_rounds: u32,
            /// 某个 need 资源低于 working_buffer 的回合数。
            below_buf_rounds: u32,
            /// fill scale 的分布：<0.25 / <0.75 / ==1（三分段）。
            scale_near0: u32,
            scale_partial: u32,
            scale_full: u32,
            /// 真缺料时：卖无可卖 / 超限额 / 富余<缺口。
            broke_rounds: u32,
            capped_rounds: u32,
            no_surplus_rounds: u32,
            starve_what: BTreeMap<String, u32>,
            classless_max: usize,
            spawned: u32,
            min_value: f64,
            max_value: f64,
            landless_rounds: u32,
            zombie_rounds: u32,
        }

        let mut acc: BTreeMap<FactionId, Acc> = state
            .factions
            .iter()
            .map(|f| (f.name.clone(), Acc { min_value: f64::MAX, ..Default::default() }))
            .collect();

        let mut world_prod: BTreeMap<String, f64> = BTreeMap::new();
        let mut zero_prod_rounds: BTreeMap<String, u32> = BTreeMap::new();
        let mut world_value_start = 0.0;
        let mut world_value_end = 0.0;

        for r in 0..n {
            let flow = sim::advance(&mut state, &config, &mut rng);

            // 出厂舰数（事件层，权威）。
            for e in &state.events {
                if let GameEvent::ShipSpawned { owner, .. } = e {
                    if let Some(a) = acc.get_mut(owner) {
                        a.spawned += 1;
                    }
                }
            }

            let mut prod_this_round: BTreeMap<String, f64> = BTreeMap::new();
            for (_fid, m) in &flow.flow.faction_production {
                for (rt, v) in m {
                    *prod_this_round.entry(rt.clone()).or_insert(0.0) += *v;
                    *world_prod.entry(rt.clone()).or_insert(0.0) += *v;
                }
            }
            for rt in config.resources.keys() {
                if prod_this_round.get(rt).copied().unwrap_or(0.0) <= 1e-9 {
                    *zero_prod_rounds.entry(rt.clone()).or_insert(0) += 1;
                }
            }

            let mut wv = 0.0;
            for f in &state.factions {
                let sv = stock_value(&state, &config, &f.name);
                wv += sv;
                if r == 0 {
                    world_value_start += sv;
                }
                world_value_end = wv;

                let a = acc.get_mut(&f.name).expect("acc");
                a.min_value = a.min_value.min(sv);
                a.max_value = a.max_value.max(sv);
                if live_cities(&state, &f.name) == 0 {
                    a.landless_rounds += 1;
                    if live_ships(&state, &f.name) == 0 {
                        a.zombie_rounds += 1;
                    }
                }

                let (need, classless) = yard_need(&state, &config, &f.name);
                a.classless_max = a.classless_max.max(classless);
                if need.is_empty() {
                    continue;
                }
                a.yard_rounds += 1;

                let stock = &f.resources;
                let mut starved = Vec::new();
                let mut below = false;
                for rt in &need {
                    let have = stock.get(rt).copied().unwrap_or(0.0);
                    if have <= 1e-9 {
                        starved.push(rt.clone());
                    }
                    if have < config.market.working_buffer {
                        below = true;
                    }
                }
                if below {
                    a.below_buf_rounds += 1;
                }

                let scale = market_fill_scale(&state, &config, &f.name, &need);
                if scale < 0.25 {
                    a.scale_near0 += 1;
                } else if scale < 0.999 {
                    a.scale_partial += 1;
                } else {
                    a.scale_full += 1;
                }

                if !starved.is_empty() {
                    a.starve_rounds += 1;
                    for rt in &starved {
                        *a.starve_what.entry(rt.clone()).or_insert(0) += 1;
                    }
                    let sell = sellable_value(&state, &config, &f.name, &need);
                    let def = deficit_value(&state, &config, &f.name, &need);
                    if sell <= 1e-6 {
                        a.broke_rounds += 1;
                    }
                    if def > config.market.auto_trade_limit {
                        a.capped_rounds += 1;
                    }
                    if sell < def {
                        a.no_surplus_rounds += 1;
                    }
                }
            }
        }

        println!("== 市场流 seed {seed}（{n} 回合）==");
        println!("世界总库存价值: 起 {world_value_start:.1} → 末 {world_value_end:.1}");
        print!("  世界各资源累计产出:");
        for (rt, v) in &world_prod {
            print!(" {rt}={v:.0}");
        }
        println!();
        print!("  全世界无人生产的回合数（供给=0）:");
        for (rt, v) in &zero_prod_rounds {
            print!(" {rt}={v}");
        }
        println!();
        for (fid, a) in &acc {
            let yr = a.yard_rounds.max(1) as f64;
            println!(
                "  {fid:<14} 船坞回合={:<4} 缺料(库存0)={:<4}({:.0}%) 低于缓冲={:<4}({:.0}%) | 成交比例 近0={:<4} 部分={:<4} 满={:<4} | 卖无可卖={:<4} 超限额={:<3} 富余<缺口={:<4} | 未定舰级建造区峰值={:<2} 出厂舰={:<3} | 无城={:<3} 无城无舰={:<3} | 价值 {:.1}..{:.1}",
                a.yard_rounds,
                a.starve_rounds,
                a.starve_rounds as f64 / yr * 100.0,
                a.below_buf_rounds,
                a.below_buf_rounds as f64 / yr * 100.0,
                a.scale_near0,
                a.scale_partial,
                a.scale_full,
                a.broke_rounds,
                a.capped_rounds,
                a.no_surplus_rounds,
                a.classless_max,
                a.spawned,
                a.landless_rounds,
                a.zombie_rounds,
                if a.min_value == f64::MAX { 0.0 } else { a.min_value },
                a.max_value
            );
            if !a.starve_what.is_empty() {
                println!("      库存为 0 的料: {:?}", a.starve_what);
            }
        }
    }
}

/// 3) 流亡态：势力在「无城」状态下能撑多久、靠什么撑（这是「用贸易取代重建」的前提）。
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
