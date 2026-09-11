//! **站点自给**探针（只打印、不断言）：非首都站点的投资需求 vs 本地能供什么。
//!
//! 背景（用户 follow-up）：今天全势力的**消耗**都读 [`Faction::resources`]（首都集散地的池子），
//! 于是「地球上的矿 → 火星的首都池 → 冥王星的一栋楼」是**瞬移**。要按 `freight-collection.md`
//! §2 的同一条物理改掉：**非首都站点的消耗也要么用本地产出、要么靠船运过来**。
//!
//! 这一份探针回答**改之前**必须先知道的三件事（纯观察，不改行为）：
//!
//! 1. **投资需求落在哪**：各势力「未建成的建筑还差多少资源」里，多少在首都天体、多少在别处；
//! 2. **本地能不能供**：非首都站点的矿藏**结构上**有没有那种货（没有 ⇒ 只能靠运）；
//! 3. **本地存量/流量**：那处货栈现在有多少、每月产出多少（对比需求，看缺口多大）。
//!
//! 跑法：
//! ```text
//! cargo nextest run -P full --run-ignored all -E 'test(probe_site_supply)'
//! PROBE_ROUNDS=400 PROBE_SEEDS=7,42 cargo test --test site_supply_probe -- --ignored --nocapture
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
        .unwrap_or(240)
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

/// 一座城**还没建成的那部分**要花多少资源（按 `per_area_cost` 与引擎同一把尺子）。
fn city_invest_need(state: &State, config: &GameConfig, cid: &str) -> ResourceMap {
    let mut out: ResourceMap = ResourceMap::new();
    let Some(city) = state.city(cid) else {
        return out;
    };
    let res_mod = state
        .city_settlement(cid)
        .map(|s| s.construction_resource_mod)
        .unwrap_or(1.0);
    for b in &city.buildings {
        if !b.under_construction() {
            continue;
        }
        let spec = config.building_spec(&b.kind);
        let per_area = sim::per_area_cost(config, spec, res_mod, b);
        let left = b.area - b.deployed;
        for (rt, c) in per_area {
            *out.entry(rt).or_insert(0.0) += c * left;
        }
    }
    out
}

/// 一座城**每回合能挖多少**（按产出公式同源：面积 × 劳动 × 生产力 × production_rate，
/// 与 `sim::production::step_production` 同一把尺子）。`None` = 没有定居点。
fn city_mine_rate(state: &State, config: &GameConfig, cid: &str) -> ResourceMap {
    let mut out: ResourceMap = ResourceMap::new();
    let Some(city) = state.city(cid) else {
        return out;
    };
    let deposits: Vec<(String, f64)> = state
        .city_settlement(cid)
        .map(|s| {
            s.resources
                .iter()
                .map(|d| (d.resource.clone(), d.area))
                .collect()
        })
        .unwrap_or_default();
    let labor = sim::labor_ratio(state, config, cid);
    for b in &city.buildings {
        let spec = config.building_spec(&b.kind);
        if spec.role != "mining" {
            continue;
        }
        let Some(rt) = b.resource.clone() else {
            continue;
        };
        let area = b.deployed * sim::building_health(b, config);
        let effective = area.min(sim::deposit_area(&deposits, &rt));
        if effective <= 0.0 {
            continue;
        }
        let output = effective * labor * spec.productivity * config.economy.production_rate;
        *out.entry(rt).or_insert(0.0) += output;
    }
    out
}

fn add(dst: &mut ResourceMap, src: &ResourceMap) {
    for (k, v) in src {
        *dst.entry(k.clone()).or_insert(0.0) += v;
    }
}

fn units(m: &ResourceMap) -> f64 {
    m.values().sum()
}

fn value(config: &GameConfig, m: &ResourceMap) -> f64 {
    m.iter().map(|(k, v)| v * value_of(config, k)).sum()
}

fn fmt(m: &ResourceMap) -> String {
    let mut v: Vec<(String, f64)> = m
        .iter()
        .filter(|(_, a)| **a > 1e-6)
        .map(|(k, a)| (k.clone(), *a))
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    if v.is_empty() {
        return "—".to_string();
    }
    v.iter()
        .take(4)
        .map(|(k, a)| format!("{k} {a:.1}"))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 一座城**已经建成的那部分**折算成资源当量（= 迄今投进去的投资量，逐件资源）。
///
/// 用 `成本/面积 × deployed` 反推——**同一把尺子**（[`sim::per_area_cost`]），
/// 于是「这一回合投了多少」= 两个回合的差。
fn city_built(state: &State, config: &GameConfig, cid: &str) -> ResourceMap {
    let mut out: ResourceMap = ResourceMap::new();
    let Some(city) = state.city(cid) else {
        return out;
    };
    let res_mod = state
        .city_settlement(cid)
        .map(|s| s.construction_resource_mod)
        .unwrap_or(1.0);
    for b in &city.buildings {
        let spec = config.building_spec(&b.kind);
        let per_area = sim::per_area_cost(config, spec, res_mod, b);
        for (rt, c) in per_area {
            *out.entry(rt).or_insert(0.0) += c * b.deployed;
        }
    }
    out
}

/// 全势力在「首都天体 / 非首都天体」上**已经建成**的资源当量（用来取差分量流速）。
fn built_split(
    state: &State,
    config: &GameConfig,
) -> (BTreeMap<String, ResourceMap>, BTreeMap<String, ResourceMap>) {
    let mut cap: BTreeMap<String, ResourceMap> = BTreeMap::new();
    let mut off: BTreeMap<String, ResourceMap> = BTreeMap::new();
    for f in &state.factions {
        let fid = f.name.as_str();
        let capital = state.capital_body(fid);
        for c in state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid && !c.razed)
        {
            let b = city_built(state, config, &c.name);
            let slot = if c.body_id == capital {
                cap.entry(fid.to_string()).or_default()
            } else {
                off.entry(fid.to_string()).or_default()
            };
            add(slot, &b);
        }
    }
    (cap, off)
}

fn diff(a: &ResourceMap, b: &ResourceMap) -> ResourceMap {
    let mut out: ResourceMap = ResourceMap::new();
    for (k, v) in b {
        let d = *v - a.get(k).copied().unwrap_or(0.0);
        if d > 1e-9 {
            out.insert(k.clone(), d);
        }
    }
    out
}

/// **改造之后的健康读数**：世界还转得动吗？
///
/// 逐回合累计（只切世界自己的行为，不做 A/B）：
/// * **两条腿的吞吐**：进口（首都装货 → 非首都卸货）与出口（非首都装货 → 首都卸货）；
/// * **站点投资**：非首都城市实际花掉多少（按已建成面积反推，与 `probe_site_invest_flow` 同源）；
/// * **站点手上有没有货**：无城/有城两类的货栈存量中位数与「缺货的站点数」；
/// * **裸舰**（一件组件都没有 ⇒ 速度 0 ⇒ 永远不能当运输舰）——这是**死亡螺旋**的读数：
///   裸舰 ⇒ 没有运力 ⇒ 运不来模块 ⇒ 更多裸舰。
#[test]
#[ignore]
fn probe_site_supply_health() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        // (进口件数, 出口件数, 进口趟数, 出口趟数)
        let (mut imp, mut exp, mut imp_trips, mut exp_trips) = (0.0, 0.0, 0u32, 0u32);
        let mut rounds_log: Vec<String> = Vec::new();
        for r in 1..=n {
            sim::advance(&mut state, &config, &mut rng);
            for e in &state.events {
                match e {
                    GameEvent::CargoLoaded {
                        owner, body, cargo, ..
                    } => {
                        // 装货点是不是**货主的首都** ⇒ 这是进口腿（补给）的起点。
                        if *body == state.capital_body(owner) {
                            imp += cargo.values().sum::<f64>();
                            imp_trips += 1;
                        }
                    }
                    GameEvent::CargoDelivered {
                        owner,
                        body,
                        cargo,
                        into_pool,
                        ..
                    } => {
                        if !*into_pool && *body != state.capital_body(owner) {
                            exp += cargo.values().sum::<f64>();
                            exp_trips += 1;
                        }
                    }
                    _ => {}
                }
            }
            if r == 1 || r % 50 == 0 || r == n {
                let bare = state
                    .ships
                    .iter()
                    .filter(|s| s.hull > 0.0 && s.components.is_empty())
                    .count();
                let alive = state.ships.iter().filter(|s| s.hull > 0.0).count();
                let pool: f64 = state
                    .factions
                    .iter()
                    .map(|f| {
                        f.resources
                            .iter()
                            .map(|(k, v)| v * value_of(&config, k))
                            .sum::<f64>()
                    })
                    .sum();
                let depot: f64 = state
                    .depots
                    .iter()
                    .flat_map(|(_, m)| m.iter())
                    .map(|(k, v)| v * value_of(&config, k))
                    .sum();
                let cities: usize = state.cities.iter().filter(|c| !c.razed).count();
                let haulers = state
                    .ships
                    .iter()
                    .filter(|s| {
                        s.hull > 0.0 && state.ship_role(s.name.clone()) == ShipRole::Freight
                    })
                    .count();
                rounds_log.push(format!(
                    "    r{r:<4} 城 {cities:<3} 舰 {alive:<3}（运输 {haulers:<2}、**裸舰 {bare:<2}**） 池值 {pool:>8.1} 货栈值 {depot:>9.1} ｜ 累计进口 {imp:>8.1}（{imp_trips} 趟）/ 出口 {exp:>8.1}（{exp_trips} 趟）"
                ));
            }
        }
        println!("== 站点自给的健康读数 seed {seed}（{n} 回合）==");
        for l in &rounds_log {
            println!("{l}");
        }
    }
}

/// **每个非首都站点的库存 / 缺口 / 矿藏**（改完之后：谁在建设、谁在等货）。
#[test]
#[ignore]
fn probe_site_stock_and_deficit() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
        }
        println!("== 站点库存 seed {seed}（{n} 回合）==");
        for f in &state.factions {
            let fid = f.name.as_str();
            let cap = state.capital_body(fid);
            let mut lines: Vec<String> = Vec::new();
            for c in state
                .cities
                .iter()
                .filter(|c| c.faction_id == fid && !c.razed)
            {
                if c.body_id == cap {
                    continue;
                }
                let need = planet_x::autocontrol::freight::site_build_need(
                    &state, &config, fid, &c.body_id,
                );
                let deficit =
                    planet_x::autocontrol::freight::site_deficit(&state, &config, fid, &c.body_id);
                let stock = state.depot(fid, &c.body_id).cloned().unwrap_or_default();
                let built: f64 = c.buildings.iter().map(|b| b.deployed).sum();
                let plan: f64 = c.buildings.iter().map(|b| b.area).sum();
                lines.push(format!(
                    "      {:<8} 面积 {built:>6.1}/{plan:<6.1} 现货 {} ｜ 需求 {} ｜ 缺 {}",
                    c.body_id,
                    fmt(&stock),
                    fmt(&need),
                    fmt(&deficit),
                ));
            }
            if lines.is_empty() {
                continue;
            }
            println!(
                "  {fid:<12} 首都 {cap} 池值 {:.1}",
                f.resources
                    .iter()
                    .map(|(k, v)| v * value_of(&config, k))
                    .sum::<f64>()
            );
            for l in lines {
                println!("{l}");
            }
        }
    }
}

/// **卡在哪一环**：到第 N 回合，逐势力拆开「造不出一艘舰」的那条链。
///
/// 链：建造区（在哪）→ 那儿有什么料（本地货栈 + 池子）→ 造一级舰要什么（船体 + 最低选装）
/// → 差多少（缺口）→ 有没有船能把这个缺口运过去（运输舰数 / 腿数）。
#[test]
#[ignore]
fn probe_site_deadlock() {
    let config = load_config();
    let n = rounds();
    let loadout = planet_x::autocontrol::minimum_loadout(&config);
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
        }
        println!("== 造舰链诊断 seed {seed}（{n} 回合）==");
        for f in &state.factions {
            let fid = f.name.as_str();
            let cap = state.capital_body(fid);
            let ships = state
                .ships
                .iter()
                .filter(|s| s.faction_id == fid && s.hull > 0.0)
                .count();
            let cities: Vec<&City> = state
                .cities
                .iter()
                .filter(|c| c.faction_id == fid && !c.razed)
                .collect();
            if cities.is_empty() && ships == 0 {
                continue;
            }
            let pool: f64 = f
                .resources
                .iter()
                .map(|(k, v)| v * value_of(&config, k))
                .sum();
            let depots: f64 = state
                .depots
                .iter()
                .filter(|((ff, _), _)| ff == fid)
                .flat_map(|(_, m)| m.iter())
                .map(|(k, v)| v * value_of(&config, k))
                .sum();
            let haulers = state
                .ships
                .iter()
                .filter(|s| {
                    s.faction_id == fid
                        && s.hull > 0.0
                        && state.ship_role(s.name.clone()) == ShipRole::Freight
                })
                .count();
            let lns = planet_x::autocontrol::freight::lanes(&state, &config, fid);
            println!(
                "  {fid:<12} 首都 {cap:<6} 城 {}（首都天体上 {}）舰 {ships}（运输 {haulers}）池值 {pool:.1} 货栈值 {depots:.1} 腿 {}",
                cities.len(),
                cities.iter().filter(|c| c.body_id == cap).count(),
                lns.len()
            );
            // 每一处有建造区的地方：它想造什么、差什么。
            for c in &cities {
                let yards: Vec<&Building> =
                    c.buildings.iter().filter(|b| b.is_shipyard()).collect();
                if yards.is_empty() {
                    continue;
                }
                let stock = state.stock_at(fid, &c.body_id).cloned().unwrap_or_default();
                let deficit =
                    planet_x::autocontrol::freight::site_deficit(&state, &config, fid, &c.body_id);
                for y in yards {
                    let cls = y.ship_type.clone().unwrap_or_default();
                    let hull: ResourceMap = config
                        .ships
                        .get(&cls)
                        .map(|s| s.build_cost.clone())
                        .unwrap_or_default();
                    let mut need = hull.clone();
                    add(&mut need, &loadout);
                    let lack = positive(&need, &stock);
                    // 悬空图指针 ⇒ 这个建造区**停产**（`build_city` 里那条 `continue`）。
                    let bp = match &y.blueprint {
                        None => "无图".to_string(),
                        Some(id) => {
                            let known = state
                                .control(fid.to_string())
                                .map(|c| c.blueprints.contains_key(id))
                                .unwrap_or(false);
                            format!("图 {id}{}", if known { "" } else { "**悬空**" })
                        }
                    };
                    println!(
                        "      {:<8}{:<6} 建造区 {cls:<11} 面积 {:.1} 进度 {:.1} {bp} ｜ 要 {} ｜ 现货 {} ｜ 缺 {}",
                        c.body_id,
                        if c.body_id == cap { "(首都)" } else { "" },
                        y.deployed,
                        c.ship_progress.get(&cls).copied().unwrap_or(0.0),
                        fmt(&need),
                        fmt(&stock),
                        fmt(&lack),
                    );
                    let _ = &deficit;
                }
            }
        }
    }
}

fn positive(a: &ResourceMap, b: &ResourceMap) -> ResourceMap {
    let mut out = ResourceMap::new();
    for (k, v) in a {
        let d = *v - b.get(k).copied().unwrap_or(0.0);
        if d > 1e-9 {
            out.insert(k.clone(), d);
        }
    }
    out
}

/// **投资的实际流速**（每回合投在首都城市 / 非首都城市上的资源当量）——这一条才是
/// 「改成本地库存之后，有多少活被卡住」的分母。
#[test]
#[ignore]
fn probe_site_invest_flow() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let (mut prev_cap, mut prev_off) = built_split(&state, &config);
        let (mut tot_cap, mut tot_off) = (0.0, 0.0);
        // 非首都城市：本地产出累计 vs 投资消耗累计（看自给率）。
        let (mut off_rate_sum, mut rounds_counted) = (0.0, 0u32);
        for r in 1..=n {
            sim::advance(&mut state, &config, &mut rng);
            let (cap, off) = built_split(&state, &config);
            let mut round_cap = 0.0;
            let mut round_off: ResourceMap = ResourceMap::new();
            for f in &state.factions {
                let fid = f.name.as_str();
                let c = diff(
                    prev_cap.get(fid).unwrap_or(&ResourceMap::new()),
                    cap.get(fid).unwrap_or(&ResourceMap::new()),
                );
                let o = diff(
                    prev_off.get(fid).unwrap_or(&ResourceMap::new()),
                    off.get(fid).unwrap_or(&ResourceMap::new()),
                );
                round_cap += units(&c);
                add(&mut round_off, &o);
            }
            // 非首都本地产出（每回合都算：产出落在产地货栈，与投资花的是同一批货）。
            let mut rate: ResourceMap = ResourceMap::new();
            for c in state.cities.iter().filter(|c| !c.razed) {
                if c.body_id == state.capital_body(&c.faction_id) {
                    continue;
                }
                add(&mut rate, &city_mine_rate(&state, &config, &c.name));
            }
            off_rate_sum += units(&rate);
            rounds_counted += 1;
            tot_cap += round_cap;
            tot_off += units(&round_off);
            prev_cap = cap;
            prev_off = off;
            if r == 12 || r == 60 || r == n {
                println!(
                    "  seed {seed} r{r:<4} 本回合投资 首都 {round_cap:>7.2} / 非首都 {:>7.2} 单位 ｜ 非首都本地产出 {:.1}/回合 ｜ 非首都货栈存量 {:.1}",
                    units(&round_off),
                    units(&rate),
                    state
                        .depots
                        .iter()
                        .filter(|((f, b), _)| state.capital_body(f) != **b)
                        .flat_map(|(_, m)| m.iter())
                        .map(|(_, v)| *v)
                        .sum::<f64>()
                );
            }
        }
        let avg_rate = if rounds_counted > 0 {
            off_rate_sum / rounds_counted as f64
        } else {
            0.0
        };
        println!(
            "== 投资流速 seed {seed}（{n} 回合）== 累计投资：首都 {tot_cap:.1} 单位 / **非首都 {tot_off:.1} 单位**（{:.0}% 在非首都）｜ 非首都本地产出均 {avg_rate:.1}/回合（合计 {:.1}）",
            if tot_cap + tot_off > 0.0 {
                100.0 * tot_off / (tot_cap + tot_off)
            } else {
                0.0
            },
            avg_rate * rounds_counted as f64,
        );
    }
}

/// **结构缺口**：非首都站点的矿藏里，**没有**建楼要用的那几种货的地方有多少。
///
/// 这是「本地产出能不能替代运货」的硬边界：矿藏是固定的，一个只出水冰的站点**永远**
/// 挖不出建楼要的铁/碳/硅 ⇒ 那部分只能靠船运（或干脆不建）。纯静态（r0 的世界）。
#[test]
#[ignore]
fn probe_site_deposit_gaps() {
    let config = load_config();
    let mut wanted: Vec<String> = Vec::new();
    for (_, spec) in config.buildings.iter() {
        for rt in spec.build_cost.keys() {
            if !wanted.contains(rt) {
                wanted.push(rt.clone());
            }
        }
    }
    wanted.sort();
    println!(
        "== 建楼要的资源：{wanted:?}（{n} 个天体）==",
        n = world::default_state(&config, 7).bodies.len()
    );
    let state = world::default_state(&config, 7);
    // 按天体列出定居点矿藏——这是「本地产出」的天花板。
    let mut missing: BTreeMap<String, usize> = BTreeMap::new();
    for b in &state.bodies {
        for s in &b.settlements {
            if s.resources.is_empty() {
                continue;
            }
            let have: Vec<String> = s.resources.iter().map(|d| d.resource.clone()).collect();
            let lack: Vec<String> = wanted
                .iter()
                .filter(|w| !have.contains(w))
                .cloned()
                .collect();
            for l in &lack {
                *missing.entry(l.clone()).or_insert(0) += 1;
            }
            println!(
                "  {:<10} {:<14} 矿藏 {:?}  ⇒ 缺 {:?}",
                b.name, s.name, have, lack
            );
        }
    }
    println!("  —— 18 个定居点里，挖不出某货的处数：{missing:?}");
}

/// **非首都站点的自给缺口**：需求落在哪、本地供得起多少、结构上缺什么。
#[test]
#[ignore]
fn probe_site_supply() {
    let config = load_config();
    let n = rounds();
    for seed in seeds() {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        for _ in 0..n {
            sim::advance(&mut state, &config, &mut rng);
        }
        println!("== 站点自给 seed {seed}（{n} 回合）==");
        let (mut w_cap_need, mut w_off_need, mut w_off_stock, mut w_off_rate) =
            (0.0, 0.0, 0.0, 0.0);
        let mut w_off_unbuildable = 0.0;
        for f in &state.factions {
            let fid = f.name.as_str();
            let cap = state.capital_body(fid);
            let mut cap_need: ResourceMap = ResourceMap::new();
            let mut off_need: ResourceMap = ResourceMap::new();
            let mut off_rate: ResourceMap = ResourceMap::new();
            let mut off_stock: ResourceMap = ResourceMap::new();
            // 本地**根本没有矿藏**的那种货：需求里这部分只能靠运（结构缺口）。
            let mut unbuildable: ResourceMap = ResourceMap::new();
            let mut off_cities = 0usize;
            let mut cap_cities = 0usize;
            for c in state
                .cities
                .iter()
                .filter(|c| c.faction_id == fid && !c.razed)
            {
                let need = city_invest_need(&state, &config, &c.name);
                if c.body_id == cap {
                    cap_cities += 1;
                    add(&mut cap_need, &need);
                    continue;
                }
                off_cities += 1;
                let rate = city_mine_rate(&state, &config, &c.name);
                let stock = state.depot(fid, &c.body_id).cloned().unwrap_or_default();
                add(&mut off_need, &need);
                add(&mut off_rate, &rate);
                add(&mut off_stock, &stock);
                for (rt, amt) in &need {
                    if rate.get(rt).copied().unwrap_or(0.0) <= 1e-9 {
                        *unbuildable.entry(rt.clone()).or_insert(0.0) += amt;
                    }
                }
            }
            let (cn, on) = (units(&cap_need), units(&off_need));
            let (os, orr) = (units(&off_stock), units(&off_rate));
            if cn <= 0.0 && on <= 0.0 && os <= 0.0 && orr <= 0.0 {
                continue;
            }
            w_cap_need += cn;
            w_off_need += on;
            w_off_stock += os;
            w_off_rate += orr;
            w_off_unbuildable += units(&unbuildable);
            println!(
                "  {fid:<12} 首都={cap:<6} 城 {cap_cities}(首都)/{off_cities}(外地)  投资需求 首都 {cn:>8.1} / 外地 {on:>8.1} 单位",
                cap = cap
            );
            println!(
                "      外地需求 {}  ｜ 本地月产 {}  ｜ 本地货栈 {}  ｜ **只能靠运的** {:.1} 单位 {}",
                fmt(&off_need),
                fmt(&off_rate),
                fmt(&off_stock),
                units(&unbuildable),
                fmt(&unbuildable),
            );
            let _ = value(&config, &off_need);
        }
        println!(
            "  —— 世界合计：投资需求 首都 {w_cap_need:.1} / 外地 {w_off_need:.1} 单位；外地本地产出 {w_off_rate:.1}/回合、外地货栈存量 {w_off_stock:.1}；**结构上只能靠运 {w_off_unbuildable:.1} 单位**"
        );
    }
}
