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
            let settled = &d.market_settled;
            let offered = &d.market_offered;
            let sv: f64 = settled
                .iter()
                .map(|(rt, amt)| amt * d.market_price.get(rt).copied().unwrap_or_else(|| value_of(&config, rt)))
                .sum();
            cum_settled_value += sv;
            for (fid, row) in &d.factions {
                *cum_net.entry(fid.clone()).or_insert(0.0) += row.net_import;
            }
            if checkpoints.contains(&r) {
                print!("  r{r:<5} 价格倍数:");
                for rt in &order {
                    let p = d.market_price.get(rt).copied().unwrap_or_else(|| value_of(&config, rt));
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

/// 3) 禁运是否真的发生、以及**为什么**（战争 / 冷到断供 / 反制联盟）。
#[test]
#[ignore]
fn probe_embargo() {
    let config = load_config();
    let n = rounds();
    let checkpoints: Vec<u32> = [1, 20, 60, 120, 240, 400, 600, 800, 1000].into_iter().filter(|c| *c <= n).collect();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut max_blocked: BTreeMap<FactionId, usize> = BTreeMap::new();
        let mut blocked_rounds: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut spawns: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut rounds_with_any = 0u32;
        let mut max_pairs = 0usize;
        println!("== 禁运 seed {seed}（{n} 回合）==");
        for r in 1..=n {
            let d = sim::advance(&mut state, &config, &mut rng);
            for e in &state.events {
                if let GameEvent::ShipSpawned { owner, .. } = e {
                    *spawns.entry(owner.clone()).or_insert(0) += 1;
                }
            }
            let mut any = false;
            let mut pairs = 0usize;
            for (fid, fm) in &d.factions {
                if fm.trade_blocked_by > 0 {
                    any = true;
                    *blocked_rounds.entry(fid.clone()).or_insert(0) += 1;
                    let e = max_blocked.entry(fid.clone()).or_insert(0);
                    *e = (*e).max(fm.trade_blocked_by);
                    pairs += fm.trade_blocked_by;
                }
            }
            if any {
                rounds_with_any += 1;
            }
            max_pairs = max_pairs.max(pairs / 2);

            if checkpoints.contains(&r) {
                let ids: Vec<String> = state.factions.iter().map(|f| f.name.clone()).collect();
                let mut by_cause: BTreeMap<&str, usize> = BTreeMap::new();
                let mut rels: Vec<f64> = Vec::new();
                for i in 0..ids.len() {
                    for j in (i + 1)..ids.len() {
                        let a = &ids[i];
                        let b = &ids[j];
                        let rel = state
                            .faction(a)
                            .and_then(|f| f.relations.get(b).copied())
                            .unwrap_or(0.0);
                        rels.push(rel);
                        if let Some(c) = sim::trade_block_cause(&state, &config, a, b) {
                            *by_cause.entry(c).or_insert(0) += 1;
                        }
                    }
                }
                rels.sort_by(|x, y| x.partial_cmp(y).unwrap());
                let med = rels.get(rels.len() / 2).copied().unwrap_or(0.0);
                let pairs_total = rels.len();
                let blocked = by_cause.values().sum::<usize>();
                println!(
                    "  r{r:<5} 封锁对={blocked}/{pairs_total} 原因={by_cause:?} 关系(最小/中位/最大)={:.1}/{:.1}/{:.1}",
                    rels.first().copied().unwrap_or(0.0),
                    med,
                    rels.last().copied().unwrap_or(0.0)
                );
            }
        }
        println!(
            "  有势力被禁运的回合={rounds_with_any}/{n}（{:.0}%）  同时被封锁的势力对峰值={max_pairs}",
            rounds_with_any as f64 / n as f64 * 100.0
        );
        for f in &state.factions {
            println!(
                "    {:<14} 被禁运回合={:<4} 最多被几国封锁={:<2} 出厂舰={}",
                f.name,
                blocked_rounds.get(&f.name).copied().unwrap_or(0),
                max_blocked.get(&f.name).copied().unwrap_or(0),
                spawns.get(&f.name).copied().unwrap_or(0)
            );
        }
    }
}

/// 4) **稀缺是否真的咬到战斗力**（M7）：新出厂舰挂的是什么**武器/防御**。
///
/// 只看 weapon/defense 类别——推进器是**平台**（船坞无论如何都要装，否则下不了水），
/// 把它算进来会把「稀有矿武装率」灌水。稀有矿武器 = 等离子炮(氦-3/金)、轨道炮(铀)；
/// 廉价武器 = 动能炮(铁/碳)、集束导弹(氢/碳)。
///
/// A/B：市场开启（`auto_trade_limit = 80`）vs 关闭（`0`，谁缺料谁自己扛）。
#[test]
#[ignore]
fn probe_armament_gate() {
    let base = load_config();
    let n = rounds();
    let _ = &base;
    for seed in seeds() {
        for arm in [0.0f64, 80.0] {
            let mut config = base.clone();
            config.market.auto_trade_limit = arm;
            let mut state = world::default_state(&config, seed);
            let mut rng = Prng::new(seed);
            let mut weapons: BTreeMap<String, u32> = BTreeMap::new();
            let mut defenses: BTreeMap<String, u32> = BTreeMap::new();
            let mut spawned: u32 = 0;
            for _ in 0..n {
                sim::advance(&mut state, &config, &mut rng);
                for e in &state.events {
                    if let GameEvent::ShipSpawned { ship, .. } = e {
                        spawned += 1;
                        if let Some(s) = state.ships.iter().find(|s| &s.name == ship) {
                            for c in &s.components {
                                match config.component_spec(c).category.as_str() {
                                    "weapon" => *weapons.entry(c.clone()).or_insert(0) += 1,
                                    "defense" => *defenses.entry(c.clone()).or_insert(0) += 1,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
            let d = sim::observe(&state, &config, &RoundSink::default());
            let blocked: usize = d.factions.values().map(|m| m.trade_blocked_by).sum();
            println!("== 武器质量 seed {seed} 市场额度={arm}（{n} 回合）== 出厂舰={spawned}");
            print!("   武器:");
            for (c, k) in &weapons {
                print!(" {c}={k}");
            }
            println!();
            print!("   防御:");
            for (c, k) in &defenses {
                print!(" {c}={k}");
            }
            println!();
            println!("   （末回合被禁运合计={blocked}）");
        }
    }
}

/// 5) **删掉 resurgence 之后世界会怎样**（D5 的代价与收益）。
///
/// * 「流亡」= 无活城但有舰（**还能自己复垦回来**：派船去空白定居点）。
/// * 「亡国」= 无活城且无舰（**再也回不来了**：无城不能造舰、无舰不能殖民）。
/// 关键问题：亡国会不会滚雪球（世界退化成少数永久旁观者），以及流亡能不能靠**航行**恢复。
#[test]
#[ignore]
fn probe_no_resurgence() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut landless_total: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut landless_run: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut landless_max: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut dead_total: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut dead_run: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut dead_max: BTreeMap<FactionId, u32> = BTreeMap::new();
        let mut dead_peak = 0usize;
        let mut dead_last = 0usize;
        let mut alive_trace: Vec<(u32, usize, usize)> = Vec::new();
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
            let mut dead_now = 0usize;
            let mut alive_now = 0usize;
            for f in &state.factions {
                let c = live_cities(&state, &f.name);
                let sh = live_ships(&state, &f.name);
                if c > 0 {
                    alive_now += 1;
                }
                let lr = landless_run.entry(f.name.clone()).or_insert(0);
                let dr = dead_run.entry(f.name.clone()).or_insert(0);
                if c == 0 {
                    *landless_total.entry(f.name.clone()).or_insert(0) += 1;
                    *lr += 1;
                    let m = landless_max.entry(f.name.clone()).or_insert(0);
                    *m = (*m).max(*lr);
                    if sh == 0 {
                        dead_now += 1;
                        *dr += 1;
                        *dead_total.entry(f.name.clone()).or_insert(0) += 1;
                        let dm = dead_max.entry(f.name.clone()).or_insert(0);
                        *dm = (*dm).max(*dr);
                    } else {
                        *dr = 0;
                    }
                } else {
                    *lr = 0;
                    *dr = 0;
                }
            }
            dead_peak = dead_peak.max(dead_now);
            dead_last = dead_now;
            alive_trace.push((state.round, alive_now, dead_now));
        }
        println!("== 删掉 resurgence 之后 seed {seed}（{n} 回合）==");
        println!("  亡国（无城无舰）峰值={dead_peak} 末态={dead_last}   末态仍有活城的势力数={}", alive_trace.last().map(|t| t.1).unwrap_or(0));
        // 每 1/4 段打印一次「仍有活城 / 亡国」的走向。
        let step = (n / 4).max(1);
        print!("  走向(回合:活城势力/亡国):");
        for (r, alive, dead) in &alive_trace {
            if r % step == 0 || *r == n {
                print!(" {r}:{alive}/{dead}");
            }
        }
        println!();
        for f in &state.factions {
            println!(
                "    {:<14} 无城回合={:<5}(最长{:<4}) 亡国回合={:<5}(最长{:<4}) 末态: 城={:<3} 舰={:<3}",
                f.name,
                landless_total.get(&f.name).copied().unwrap_or(0),
                landless_max.get(&f.name).copied().unwrap_or(0),
                dead_total.get(&f.name).copied().unwrap_or(0),
                dead_max.get(&f.name).copied().unwrap_or(0),
                live_cities(&state, &f.name),
                live_ships(&state, &f.name),
            );
        }
    }
}

/// 6) **M6：运费与 MOND 承运**——谁在付运费、谁靠穿越异常带抽税、丢了多少货。
#[test]
#[ignore]
fn probe_freight() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut freight: BTreeMap<FactionId, f64> = BTreeMap::new();
        let mut carrier: BTreeMap<FactionId, f64> = BTreeMap::new();
        let mut rounds_with_carrier = 0u32;
        let mut deep_routes = 0u32;
        for _ in 0..n {
            let d = sim::advance(&mut state, &config, &mut rng);
            for (fid, row) in &d.factions {
                *freight.entry(fid.clone()).or_insert(0.0) += row.freight_paid;
            }
            let mut any = false;
            for (fid, row) in &d.factions {
                *carrier.entry(fid.clone()).or_insert(0.0) += row.carrier_income;
                if row.carrier_income > 0.0 {
                    any = true;
                }
            }
            if any {
                rounds_with_carrier += 1;
            }
            // 有多少条「进出异常带」的航线存在（用首都半径粗看）。
            for f in &state.factions {
                let cap = state.capital_body(&f.name);
                if cap.is_empty() {
                    continue;
                }
                let p = state.body_position(&cap);
                if (p[0] * p[0] + p[1] * p[1]).sqrt() > config.mond.radius {
                    deep_routes += 1;
                    break;
                }
            }
        }
        println!("== 运费/MOND 承运 seed {seed}（{n} 回合）==");
        println!(
            "  异常带半径={} AU  masters={:?}  有承运收入的回合={rounds_with_carrier}/{n}  有势力首都位于带内的回合={deep_routes}/{n}",
            config.mond.radius, config.mond.masters
        );
        for f in &state.factions {
            println!(
                "    {:<14} 付运费={:>9.1}  承运收入={:>9.1}",
                f.name,
                freight.get(&f.name).copied().unwrap_or(0.0),
                carrier.get(&f.name).copied().unwrap_or(0.0)
            );
        }
    }
}

/// 8b) **集货的 A/B（因果读数）**：同一颗种子、同一段回合，**只切「集货开/关」一个开关**
/// （关 = 把各势力的**舰队默认角色**钉成「战舰」且归玩家 ⇒ 自动定编不许碰角色叶，
/// 见 `State::ship_freighter` 的取值链）。
///
/// 为什么非要 A/B：世界走向对战争极其敏感，隔一次改动比「积压占池值」那样的横向数字会被
/// 完全不同的战争结局搅浑（实测同一颗种子在不同提交上能差出几倍）。只切一个开关，
/// 「集货腿搬走了多少、首都池多了多少」才是**因果**读数。
#[test]
#[ignore]
fn probe_freight_ab() {
    let config = load_config();
    let n = rounds();
    let depot_units = |state: &State| -> f64 {
        state.depots.values().flat_map(|m| m.values()).sum()
    };
    let pool_value = |state: &State| -> f64 {
        state
            .factions
            .iter()
            .flat_map(|f| f.resources.iter())
            .map(|(rt, amt)| amt * value_of(&config, rt))
            .sum()
    };
    for seed in seeds() {
        let mut line = String::new();
        for hauling in [false, true] {
            let mut state = world::default_state(&config, seed);
            if !hauling {
                // 玩家的舰队默认角色 = 战舰（`Player` ⇒ 自动定编一个字都不写）。
                let fids: Vec<String> = state.factions.iter().map(|f| f.name.clone()).collect();
                for fid in fids {
                    if let Some(c) = state.control_mut(fid) {
                        c.default_freighter = Some(Control::player(false));
                    }
                }
            }
            let mut rng = Prng::new(seed);
            let mut delivered = 0.0;
            for _ in 0..n {
                sim::advance(&mut state, &config, &mut rng);
                for e in &state.events {
                    if let GameEvent::CargoDelivered { cargo, into_pool: true, .. } = e {
                        delivered += cargo.values().sum::<f64>();
                    }
                }
            }
            let tag = if hauling { "集货**开**" } else { "集货关" };
            line.push_str(&format!(
                "  {tag}：期末积压 {:.0} 单位 / 首都池值 {:.0}（积压/池值 {:.0}%）  进池货 {:.0} 件",
                depot_units(&state),
                pool_value(&state),
                if pool_value(&state) > 0.0 { 100.0 * depot_units(&state) / pool_value(&state) } else { 0.0 },
                delivered,
            ));
        }
        println!("== 集货 A/B seed {seed}（{n} 回合）==\n{line}");
    }
}

/// 8c) **雇佣市场的 A/B（因果读数）**：同一颗种子、同一段回合，**只切「雇佣市场开/关」一个开关**。
///
/// 关的做法：把**雇主的准入闸抬到没人够得着**（`freight.gate_base = 100`）——于是单子照旧
/// 挂出去（需求信号仍在，世界走向的确定性与开的那一侧逐字相同），但**没有一家能受雇**，
/// 整条雇佣腿的搬运量归零。这是**同一颗种子上只动一个数**的因果读数，不是跨提交对照
/// （同种子在不同提交上本来就能差出几倍，见 §5.2 的读数纪律）。
///
/// 与 8b（集货 A/B）合起来回答的是同一个问题：**「请人运」这条路到底搬走了多少货**
/// ——它是自有集货的补充，还是只是在内部倒手。
#[test]
#[ignore]
fn probe_hire_ab() {
    let config = load_config();
    let n = rounds();
    let depot_units =
        |state: &State| -> f64 { state.depots.values().flat_map(|m| m.values()).sum() };
    let pool_value = |state: &State| -> f64 {
        state
            .factions
            .iter()
            .flat_map(|f| f.resources.iter())
            .map(|(rt, amt)| amt * value_of(&config, rt))
            .sum()
    };
    for seed in seeds() {
        let mut line = String::new();
        for hiring in [false, true] {
            let mut cfg = config.clone();
            if !hiring {
                // 准入闸抬到天上 ⇒ 谁都不够格受雇（单子照挂，需求信号不受影响）。
                cfg.freight.gate_base = 100.0;
            }
            let mut state = world::default_state(&cfg, seed);
            let mut rng = Prng::new(seed);
            let (mut to_pool, mut by_hire) = (0.0, 0.0);
            let mut hires = 0usize;
            for _ in 0..n {
                sim::advance(&mut state, &cfg, &mut rng);
                for e in &state.events {
                    match e {
                        GameEvent::CargoDelivered { cargo, into_pool, owner, faction, .. } => {
                            let u: f64 = cargo.values().sum();
                            if *into_pool {
                                to_pool += u;
                            }
                            // 受雇跑的船：**货主不是船东**（`cargo_owner` 认派工记录）。
                            // 不看 `into_pool`：雇主可能在雇佣期内迁都，那时货卸进货栈而不是池子，
                            // 但它照样是**受雇搬走的货**。
                            if owner != faction {
                                by_hire += u;
                            }
                        }
                        GameEvent::ContractAccepted { .. } => hires += 1,
                        _ => {}
                    }
                }
            }
            let tag = if hiring { "雇佣**开**" } else { "雇佣关" };
            line.push_str(&format!(
                "  {tag}：期末积压 {:.0} 单位 / 首都池值 {:.0}（积压/池值 {:.0}%）  集货进池 {:.0} 件（其中受雇搬运 {:.0} 件）  成交 {} 单\n",
                depot_units(&state),
                pool_value(&state),
                if pool_value(&state) > 0.0 { 100.0 * depot_units(&state) / pool_value(&state) } else { 0.0 },
                to_pool,
                by_hire,
                hires,
            ));
        }
        println!("== 雇佣市场 A/B seed {seed}（{n} 回合）==\n{line}");
    }
}

/// 9) 流亡态：势力在「无城」状态下能撑多久、靠什么撑。
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

/// 8) **集货腿的量（M2）**：离岸产出积压了多少、值多少、折多少趟运输、有多少压在异常带里。
///
/// 这是 M3 派单 / M4 定价要面对的**需求侧**实测：没有运输时，货栈就是「等船来运」的存量。
/// * **开局需求**（第 1 回合末的积压）＝ 每月要搬多少货（单位/回合）——它决定需要几条船；
/// * **期末积压**＝ 一直没人运时攒下来的存量（单位 / 价值 / 折多少趟航母舱容）；
/// * **带内**＝ 积压所在天体在 MOND 异常带内（非 master）的处数——它决定**谁能去取**。
#[test]
#[ignore]
fn probe_collection_backlog() {
    let config = load_config();
    let n = rounds();
    let carrier_cap = config.ship_spec("carrier").cargo;
    let r_mond = config.mond.radius;
    // (单位, 价值, 货栈处数, 其中在异常带内的处数)
    let depot_of = |state: &State, fid: &str| -> (f64, f64, usize, usize) {
        let (mut units, mut value, mut bodies, mut deep) = (0.0, 0.0, 0usize, 0usize);
        for ((f, b), map) in &state.depots {
            if f != fid {
                continue;
            }
            bodies += 1;
            let p = state.body_position(b);
            if (p[0] * p[0] + p[1] * p[1]).sqrt() > r_mond {
                deep += 1;
            }
            for (rt, amt) in map {
                units += amt;
                value += amt * value_of(&config, rt);
            }
        }
        (units, value, bodies, deep)
    };
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        // 集货吞吐（扫每回合的事件流水；`advance` 开头会 clear，所以返回后就是本回合的）：
        // (装货件数, 卸货件数, 其中**卸进首都池**的件数)——最后一项才是「集货真正完成」。
        let (mut loaded, mut delivered, mut to_pool) = (0.0, 0.0, 0.0);
        let (mut load_trips, mut delivery_trips) = (0u32, 0u32);
        let tally = |state: &State,
                         loaded: &mut f64,
                         delivered: &mut f64,
                         to_pool: &mut f64,
                         load_trips: &mut u32,
                         delivery_trips: &mut u32| {
            for e in &state.events {
                match e {
                    GameEvent::CargoLoaded { cargo, .. } => {
                        *loaded += cargo.values().sum::<f64>();
                        *load_trips += 1;
                    }
                    GameEvent::CargoDelivered { cargo, into_pool, .. } => {
                        let u: f64 = cargo.values().sum();
                        *delivered += u;
                        *delivery_trips += 1;
                        if *into_pool {
                            *to_pool += u;
                        }
                    }
                    _ => {}
                }
            }
        };
        sim::advance(&mut state, &config, &mut rng); // 第 1 回合末
        tally(&state, &mut loaded, &mut delivered, &mut to_pool, &mut load_trips, &mut delivery_trips);
        let opening: BTreeMap<String, f64> = state
            .factions
            .iter()
            .map(|f| (f.name.clone(), depot_of(&state, &f.name).0))
            .collect();
        for _ in 1..n {
            sim::advance(&mut state, &config, &mut rng);
            tally(&state, &mut loaded, &mut delivered, &mut to_pool, &mut load_trips, &mut delivery_trips);
        }
        println!("== 集货积压 seed {seed}（{n} 回合，航母舱容 {carrier_cap}）==");
        let (mut tot_units, mut tot_value, mut tot_pool_v) = (0.0, 0.0, 0.0);
        for f in &state.factions {
            let name = f.name.as_str();
            let (units, value, bodies, deep) = depot_of(&state, &name);
            let pool = stock_value(&state, &config, name);
            let open = opening.get(name).copied().unwrap_or(0.0);
            tot_units += units;
            tot_value += value;
            tot_pool_v += pool;
            let haulers = state
                .ships
                .iter()
                .filter(|s| s.faction_id == name && s.hull > 0.0 && state.ship_freighter(s.name.clone()))
                .count();
            // 运输舰的**舰级构成**：运力 = 舱容 × 舰数，所以「派了谁」和「派了几条」一样重要。
            let mut classes: BTreeMap<String, usize> = BTreeMap::new();
            for s in state
                .ships
                .iter()
                .filter(|s| s.faction_id == name && s.hull > 0.0 && state.ship_freighter(s.name.clone()))
            {
                *classes.entry(s.class.clone()).or_insert(0) += 1;
            }
            let breakdown = classes
                .iter()
                .map(|(k, v)| format!("{k}×{v}"))
                .collect::<Vec<_>>()
                .join(" ");
            let ships = live_ships(&state, name);
            if units <= 0.0 && open <= 0.0 && haulers == 0 {
                continue;
            }
            let trips = units / carrier_cap;
            println!(
                "    {name:<14} 开局需求={open:>7.2}/回合  期末积压={units:>9.1} 单位 / 值 {value:>9.1}  \
                 货栈 {bodies} 处（带内 {deep}）  折 {trips:>6.1} 趟航母  舰 {ships:>2}（运输 {haulers}: {breakdown}）  池值 {pool:>9.1}"
            );
        }
        let ratio = if tot_pool_v > 0.0 { 100.0 * tot_value / tot_pool_v } else { 0.0 };
        println!(
            "    —— 合计：积压 {tot_units:.1} 单位 / 值 {tot_value:.1}；池值合计 {tot_pool_v:.1}（积压占池值 {ratio:.1}%）"
        );
        let per_load = if load_trips > 0 { loaded / load_trips as f64 } else { 0.0 };
        println!(
            "    —— 集货吞吐：装 {loaded:.0} 件（{load_trips} 趟，**每趟 {per_load:.1} 件**）/ 卸 {delivered:.0} 件（{delivery_trips} 趟），\
             其中**进首都池 {to_pool:.0} 件**（集货真正完成的那部分）"
        );
    }
}

/// 10) **雇佣运力市场**：挂单、受雇、派工、考核、续约/抽手——整条腿的真实流量。
///
/// 雇佣形态下要看的量与「一票货」形态**完全不同**：
///
/// * **需求侧**：谁在挂、要求多少运力（单位/回合）、抬价抬到哪儿；
/// * **成交与履约**：多少单被接下、**多少货真的被搬到了雇主首都**、受雇方拿到多少抽成；
/// * **考核**（信誉的唯一来源）：好评/差评各多少、**达标率**的分布；
/// * **关系的存续**：固定期到期换人（`term`）、无货可运收单（`no_cargo`）、受雇方缺船抽手
///   （`recalled`）各多少——**市场是否在换手**是这一版的健康指标；
/// * **派几条船**：用户说「对方派几艘船都无所谓」，所以要看到每张在期单子上实际有几条船。
#[test]
#[ignore]
fn probe_contract_market() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let (mut posted, mut posted_cap) = (0usize, 0.0);
        let mut accepted = 0usize;
        let (mut paid, mut cut, mut trips) = (0.0, 0.0, 0usize);
        let (mut good, mut bad) = (0usize, 0usize);
        let mut ratios: Vec<f64> = Vec::new();
        // 达标率要连着**要求运力**一起看：一个「要求 0.01 件/回合」的单，随便搬点货就是
        // 几百倍达标率——那不是干得好，是尺子坏了。
        let mut pairs: Vec<(f64, u32, f64, u32, f64)> = Vec::new();
        let mut ended: BTreeMap<String, usize> = BTreeMap::new();
        let mut income: BTreeMap<String, f64> = BTreeMap::new(); // 受雇方的抽成收入（件）
        let mut runs: BTreeMap<String, usize> = BTreeMap::new(); // 受雇方跑了几趟
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
            for e in &state.events {
                match e {
                    GameEvent::ContractPosted { capacity, .. } => {
                        posted += 1;
                        posted_cap += capacity;
                    }
                    GameEvent::ContractAccepted { .. } => accepted += 1,
                    GameEvent::ContractDelivered { amount, cut: c, carrier, .. } => {
                        paid += amount;
                        cut += c;
                        trips += 1;
                        *income.entry(carrier.clone()).or_insert(0.0) += c;
                        *runs.entry(carrier.clone()).or_insert(0) += 1;
                    }
                    GameEvent::ContractReviewed { ratio, good: g, contract, .. } => {
                        if *g {
                            good += 1;
                        } else {
                            bad += 1;
                        }
                        ratios.push(*ratio);
                        if let Some(k) = state.contracts.get(*contract) {
                            pairs.push((
                                k.capacity,
                                k.served_rounds,
                                k.delivered,
                                state.round.saturating_sub(k.accepted_round.unwrap_or(state.round)),
                                *ratio,
                            ));
                        }
                    }
                    GameEvent::ContractEnded { reason, .. } => {
                        *ended.entry(reason.clone()).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
        }
        let _ = trips;
        println!(
            "== 雇佣运力市场 seed {seed}（{n} 回合，开叫抽成 {:.0}%）==",
            config.freight.share * 100.0
        );
        for f in &state.factions {
            let name = f.name.as_str();
            let backlog: f64 = state
                .depots
                .iter()
                .filter(|((fid, _), _)| fid == name)
                .map(|(_, m)| m.values().sum::<f64>())
                .sum();
            let mine: Vec<&Contract> = state
                .contracts
                .contracts
                .iter()
                .filter(|c| c.shipper == name)
                .collect();
            let open = mine.iter().filter(|c| c.is_open()).count();
            let hired = mine.iter().filter(|c| c.is_hired()).count();
            let my_ships = state.ships.iter().filter(|s| s.faction_id == name && s.hull > 0.0).count();
            let serving: usize = state
                .contracts
                .assignments
                .iter()
                .filter(|(s, _)| {
                    state.ship(s.as_str()).map(|sh| sh.faction_id == name).unwrap_or(false)
                })
                .count();
            let inc = income.get(name).copied().unwrap_or(0.0);
            if backlog <= 0.0 && mine.is_empty() && inc <= 0.0 && my_ships == 0 {
                continue;
            }
            println!(
                "    {name:<14} 积压 {backlog:>8.1} 件  舰 {my_ships:>2}（替人跑 {serving}）  \
                 自己挂的单 {open} 张等人接 / {hired} 张在雇佣期  受雇收入 {inc:>7.1} 件（{} 趟）  信誉 {:.2}",
                runs.get(name).copied().unwrap_or(0),
                state.faction(name).unwrap().reputation
            );
        }
        let mean_ratio = if ratios.is_empty() {
            0.0
        } else {
            ratios.iter().sum::<f64>() / ratios.len() as f64
        };
        let under = ratios.iter().filter(|r| **r < 1.0).count();
        let paid_share = if paid + cut > 0.0 { cut / (paid + cut) } else { 0.0 };
        println!(
            "    —— 挂出 {posted} 张（要求运力合计 {posted_cap:.1} 件/回合）；成交 {accepted} 单；搬到位 {paid:.0} 件（{trips} 趟，受雇方自留 {cut:.0} 件 = 它的全部报酬，实付抽成 {:.1}%，开叫价 {:.0}%）",
            paid_share * 100.0,
            config.freight.share * 100.0
        );
        println!(
            "    —— 考核 {} 次：好评 {good} / 差评 {bad}；平均达标率 {mean_ratio:.2}（未达标 {under}/{} 次 ⇒ 雇主的耐心在往哪边走）",
            good + bad,
            ratios.len()
        );
        let reason = |k: &str| ended.get(k).copied().unwrap_or(0);
        println!(
            "    —— 结束 {} 份关系：到期换人 {} / 无货收单 {} / 受雇方抽手 {}",
            ended.values().sum::<usize>(),
            reason("term"),
            reason("no_cargo"),
            reason("recalled")
        );
        // **派几条船**：一张单可以跑多条船，所以要看到在期单子的实际编制。
        let staff: Vec<usize> = state
            .contracts
            .contracts
            .iter()
            .filter(|c| c.is_hired())
            .map(|c| state.contracts.ships_of(c.id).len())
            .collect();
        let open_shares: Vec<f64> = state
            .contracts
            .contracts
            .iter()
            .filter(|c| c.is_open())
            .map(|c| c.share)
            .collect();
        let (lo, hi) = (
            open_shares.iter().cloned().fold(f64::INFINITY, f64::min),
            open_shares.iter().cloned().fold(0.0f64, f64::max),
        );
        let mean_share = if open_shares.is_empty() {
            0.0
        } else {
            open_shares.iter().sum::<f64>() / open_shares.len() as f64
        };
        println!(
            "    —— 期末在簿 {} 张（等人接 {} 张，抽成已抬到 {:.1}%–{:.1}%，均 {:.1}%；上限 {:.0}%）；在期单子的编制 {:?}（每张几条船）",
            state.contracts.contracts.len(),
            state.contracts.contracts.iter().filter(|c| c.is_open()).count(),
            if open_shares.is_empty() { 0.0 } else { lo * 100.0 },
            hi * 100.0,
            mean_share * 100.0,
            config.freight.share_max * 100.0,
            staff
        );
        // 达标率 × 要求运力：低要求运力的单会把平均达标率**假性**拉高（尺子坏了，
        // 不是干得好），所以这一段必须逐条看。
        let mut caps: Vec<f64> = pairs.iter().map(|(c, _, _, _, _)| *c).collect();
        caps.sort_by(f64::total_cmp);
        let med_cap = caps.get(caps.len() / 2).copied().unwrap_or(0.0);
        let tiny = pairs.iter().filter(|(c, _, _, _, _)| *c < 0.2).count();
        println!(
            "    —— 考核时的要求运力：中位 {med_cap:.3} 件/回合、最小 {:.4}、最大 {:.2}；\
             其中 {tiny}/{} 次的要求运力 < 0.2（这些单子的达标率没有意义）",
            caps.first().copied().unwrap_or(0.0),
            caps.last().copied().unwrap_or(0.0),
            pairs.len()
        );
        // 尺子坏了要看清是哪一段坏的：把**要求运力 / 有货回合 / 雇佣期长度 / 交付量 / 达标率**
        // 摆在一起。达标率 = 交付 ÷ (要求运力 × 有货回合)；若「有货回合 ≪ 雇佣期长度」，
        // 这条公式就在奖励「搬积压」而不是奖励「提供运力」。
        for (cap, served, delivered, elapsed, ratio) in pairs.iter().take(8) {
            println!(
                "        · 要求 {cap:>7.3}/回合 × 有货 {served:>3} 回合 ÷ 雇佣 {elapsed:>3} 回合 \
                 ⇒ 应搬 {:>7.3}，实交 {delivered:>7.3} ⇒ 达标率 {ratio:>7.2}",
                cap * *served as f64
            );
        }
    }
}

/// 【探针·思潮→角色】长局里「思潮 → 派多少条腿去运货」是否真的分出了高低，以及差额是被
/// **自运**补上的还是被**雇佣**补上的（M5 的观察面：这一层此前完全不可见）。
///
/// 每行：思潮两轴 → 尚武度 → 倾向倍数 / 目标头数 / 实际运输舰数（全程均值），以及
/// 自有船搬回家的件数、雇人搬回（净入池）的件数、替别人跑赚到的抽成、压在货栈里的存货（均值）。
#[test]
#[ignore]
fn probe_ideology_freight() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut own: BTreeMap<String, f64> = BTreeMap::new();
        let mut hired_in: BTreeMap<String, f64> = BTreeMap::new();
        let mut earned: BTreeMap<String, f64> = BTreeMap::new();
        let mut hauler_rounds: BTreeMap<String, f64> = BTreeMap::new();
        let mut depot_sum: BTreeMap<String, f64> = BTreeMap::new();
        let mut ships_at_200: BTreeMap<String, usize> = BTreeMap::new();
        let mut first_hauler: BTreeMap<String, u32> = BTreeMap::new();
        for r in 1..=n {
            sim::advance(&mut state, &config, &mut rng);
            if r == 200 {
                for f in state.factions.clone() {
                    ships_at_200.insert(
                        f.name.clone(),
                        state.ships.iter().filter(|s| s.hull > 0.0 && s.faction_id == f.name).count(),
                    );
                }
            }
            for f in state.factions.clone() {
                let hauling = state
                    .ships
                    .iter()
                    .any(|s| s.hull > 0.0 && s.faction_id == f.name && state.ship_freighter(s.name.clone()));
                if hauling {
                    first_hauler.entry(f.name.clone()).or_insert(r);
                }
            }
            for e in &state.events {
                match e {
                    GameEvent::CargoDelivered { owner, into_pool, cargo, .. } => {
                        // 只算**进池**的那一卸（中转卸货不算到家）。
                        if *into_pool {
                            *own.entry(owner.clone()).or_insert(0.0) += cargo.values().sum::<f64>();
                        }
                    }
                    GameEvent::ContractDelivered { shipper, carrier, amount, cut, .. } => {
                        *hired_in.entry(shipper.clone()).or_insert(0.0) += amount - cut;
                        *earned.entry(carrier.clone()).or_insert(0.0) += cut;
                    }
                    _ => {}
                }
            }
            for f in state.factions.clone() {
                let c = state
                    .ships
                    .iter()
                    .filter(|s| s.hull > 0.0 && s.faction_id == f.name && state.ship_freighter(s.name.clone()))
                    .count() as f64;
                *hauler_rounds.entry(f.name.clone()).or_insert(0.0) += c;
                let d: f64 = state
                    .depots
                    .iter()
                    .filter(|((ff, _), _)| ff == &f.name)
                    .map(|(_, m)| m.values().sum::<f64>())
                    .sum();
                *depot_sum.entry(f.name.clone()).or_insert(0.0) += d;
            }
        }
        println!("--- seed {seed} / {n} 回合：思潮 → 集货（头数均值 / 件数累计）---");
        for f in &state.factions {
            let martial = f.ideology.peace_military - f.ideology.nature_colony;
            let lean = planet_x::autocontrol::freight::freight_lean(&state, &f.name);
            let quota = planet_x::autocontrol::freight::freighter_quota(&state, &f.name);
            let ships = state.ships.iter().filter(|s| s.hull > 0.0 && s.faction_id == f.name).count();
            // 诊断：**动得了**的舰（有推进模块 ⇒ 运力 > 0）与「角色轴归玩家」的舰数。
            // 若某个势力有舰却一条运输舰都派不出来，答案通常在这两列里。
            let movable = state
                .ships
                .iter()
                .filter(|s| {
                    s.hull > 0.0
                        && s.faction_id == f.name
                        && planet_x::autocontrol::freight::freight_tonnage(&config, s) > 0.0
                })
                .count();
            let pinned = state
                .ships
                .iter()
                .filter(|s| {
                    s.hull > 0.0
                        && s.faction_id == f.name
                        && state.ship_freighter_control(s.name.clone()).is_player()
                })
                .count();
            println!(
                "{:>10}: 军国{:+5.2} 殖民{:+5.2} ⇒ 尚武{martial:+5.2} 倾向{lean:.2} 配额{quota:>5.1} \
                 | 舰{ships:>2} 可动{movable:>2} 玩家钉{pinned:>2} 运输舰(均){:>5.2} 自运{:>8.0} 雇入{:>7.0} 赚运费{:>7.0} 货栈(均){:>7.0} r200舰{:>2} 首运r{:>3}",
                f.name,
                f.ideology.peace_military,
                f.ideology.nature_colony,
                hauler_rounds.get(&f.name).copied().unwrap_or(0.0) / n as f64,
                own.get(&f.name).copied().unwrap_or(0.0),
                hired_in.get(&f.name).copied().unwrap_or(0.0),
                earned.get(&f.name).copied().unwrap_or(0.0),
                depot_sum.get(&f.name).copied().unwrap_or(0.0) / n as f64,
                ships_at_200.get(&f.name).copied().unwrap_or(0),
                first_hauler.get(&f.name).copied().unwrap_or(0),
            );
        }
    }
}

/// 【探针·威胁动机】造战斗舰的动机改成「敌对国与自己的实力差距」之后，**世界层面**变了什么：
/// 逐 100 回合打印拆平数、开战数、各势力动机（均值/最大）、旗舰数与船坞改装去向。
///
/// 动机是**纯函数**，所以可以直接逐势力算，不必等它落到行为上。
#[test]
#[ignore]
fn probe_threat_motive() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut razings = 0usize;
        let mut wars = 0usize;
        let mut retools: BTreeMap<String, usize> = BTreeMap::new();
        println!("--- seed {seed} / {n} 回合：威胁动机 ---");
        for r in 1..=n {
            let d = sim::advance(&mut state, &config, &mut rng);
            for e in &state.events {
                match e {
                    GameEvent::CityRazed { .. } => razings += 1,
                    GameEvent::WarStarted { .. } => wars += 1,
                    _ => {}
                }
            }
            for t in &d.decisions.retools {
                *retools.entry(format!("{}→{}", t.from, t.to)).or_insert(0) += 1;
            }
            if r % 100 == 0 {
                let mut motives: Vec<(String, f64)> = state
                    .factions
                    .iter()
                    .map(|f| (f.name.clone(), planet_x::autocontrol::shipbuilding::threat_motive(&state, &config, &f.name)))
                    .collect();
                motives.sort_by(|a, b| b.1.total_cmp(&a.1));
                let mean = motives.iter().map(|(_, m)| m).sum::<f64>() / motives.len().max(1) as f64;
                let flagship = state
                    .ships
                    .iter()
                    .filter(|s| s.hull > 0.0 && (s.class == "carrier" || s.class == "battleship"))
                    .count();
                let top: Vec<String> = motives
                    .iter()
                    .take(3)
                    .map(|(n, m)| format!("{n} {m:.2}"))
                    .collect();
                println!(
                    "r{r:>3}: 拆平{razings:>3} 开战{wars:>3} 重舰{flagship:>3} 动机均{mean:.2} \
                     最高[{}]",
                    top.join(" / ")
                );
            }
        }
        let mut rs: Vec<(String, usize)> = retools.into_iter().collect();
        rs.sort_by(|a, b| b.1.cmp(&a.1));
        println!("改装去向（前 12）：{:?}", &rs[..rs.len().min(12)]);
    }
}
