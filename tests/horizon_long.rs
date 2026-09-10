//! Long-horizon health harness for the Planet X sandbox.
//!
//! The design goal is that the world stays *meaningful and unbroken* over
//! thousands of rounds: no NaN/Inf drift, no unbounded economic runawaay, no
//! silent collapse into a dead/stale board, and never a panic. This module is a
//! diagnostic + regression guard. Run the diagnostic with:
//!
//! ```text
//! cargo nextest run -P full --run-ignored all -E 'test(/diagnose_long_horizon/)' --no-capture
//! ```
//!
//! The individual `#[test]`s are shrink-wrapped invariants; the
//! `diagnose_long_horizon` test prints a human-readable health report so a
//! maintainer can eyeball the trajectory.
//!
//! ## 那几条**长局不变量**已经搬到 Python 侧（`play/tests/g3_long.py`）
//!
//! 2026-10（用户裁决：*「测试应该和游戏二进制解耦，直接测跑出来的数据」*）：下面这五条
//! 原来住在这里，现在住 `play/tests/g3_long.py` —— 它们断言的全是**读面上的数**，不需要
//! crate 内部 API，而在这里每条的**世界**都要自己跑一遍（七条用例跑的是**完全相同**的
//! `seed × 1000 回合`，同一批 2.8 万回合被重算了四遍，每次改一个字还要重编 8 个测试二进制）。
//!
//! | 原来住这里 | 现在住 |
//! | --- | --- |
//! | `no_nonfinite_over_long_run`（读面不许有非有限的数 + 不进吸收态 + 零活城复生） | `g3_long.py` 的前四条 |
//! | `world_value_is_bounded` | `g3_long.py`「世界总价值 < 2e6」 |
//! | `city_founding_requires_a_ship_there` | `g3_long.py`「建城必须有一艘在场的活舰」 |
//! | `coalition_mechanism_is_alive` | `g3_long.py`「合纵连横/制裁真的成立过」 |
//! | `test_power_statistic_matches_game_logic` | `g3_long.py`「霸权 = 占比最高者，且联盟/封锁自洽」 |
//!
//! 判据与阈值**一个没动**（`36` 回合复生上限、`2e6` 价值上限、`min_members`/`hegemon_power`
//! 从 `meta.json` 读），种子还更宽（Python 侧 7 个种子跑全部不变量）。跑法：
//!
//! ```text
//! uv run --project play/planet_xq python play/tests/run.py 3
//! ```
//!
//! 留在这里的是**探针**（`#[ignore]`，只打印不断言）与 `world_is_multipolar`——它是待调的
//! **平衡目标**，不是机制不变量。它们要用 crate 内部量（`sim::observe` 等），所以不搬。

use planet_x::config::load_config;
use planet_x::model::*;
use planet_x::prng::Prng;
use planet_x::sim;
use planet_x::world;
use std::collections::BTreeSet;

/// Total market value of one faction's stockpile.
fn faction_value(f: &Faction, config: &GameConfig) -> f64 {
    f.resources
        .iter()
        .map(|(k, v)| *v * config.resources.get(k).map(|r| r.value).unwrap_or(1.0))
        .sum()
}

/// Sum of a world's numeric fields that must never go NaN/Inf.
fn count_nonfinite(state: &State, _config: &GameConfig) -> (usize, Vec<String>) {
    let mut bad = 0usize;
    let mut samples = Vec::new();
    let mut check = |label: String, v: f64| {
        if !v.is_finite() {
            bad += 1;
            if samples.len() < 8 {
                samples.push(format!("{label}: {v}"));
            }
        }
    };

    for b in &state.bodies {
        check(format!("body[{}].position.0", b.name), b.position[0]);
        check(format!("body[{}].position.1", b.name), b.position[1]);
        check(
            format!("body[{}].orbit.peri", b.name),
            b.orbit.perihelion_distance as f64,
        );
        check(
            format!("body[{}].orbit.aphe", b.name),
            b.orbit.aphelion_distance as f64,
        );
        for s in &b.settlements {
            check(format!("settlement.{}area", s.name), s.total_area);
            for d in &s.resources {
                check(format!("deposit.{}area", d.resource), d.area);
            }
        }
    }

    for f in &state.factions {
        for (k, v) in &f.resources {
            check(format!("faction[{}].res.{k}", f.name), *v);
        }
        for (o, v) in &f.relations {
            check(format!("faction[{}].rel.{o}", f.name), *v);
        }
        check(format!("faction[{}].alignment", f.name), f.alignment);
        check(format!("faction[{}].aggression", f.name), f.aggression);
    }

    for c in &state.cities {
        for b in &c.buildings {
            check(format!("city[{}].bld[{}].area", c.name, b.id), b.area);
            check(
                format!("city[{}].bld[{}].deployed", c.name, b.id),
                b.deployed,
            );
            check(format!("city[{}].bld[{}].armor", c.name, b.id), b.armor);
        }
        for (cls, p) in &c.ship_progress {
            check(format!("city[{}].progress.{cls}", c.name), *p);
        }
    }

    for s in &state.ships {
        check(format!("ship[{}].pos.0", s.name), s.position[0]);
        check(format!("ship[{}].pos.1", s.name), s.position[1]);
        check(format!("ship[{}].hull", s.name), s.hull);
        check(format!("ship[{}].hull_max", s.name), s.hull_max);
        check(format!("ship[{}].shield", s.name), s.shield);
        check(format!("ship[{}].shield_max", s.name), s.shield_max);
    }

    for (fid, c) in &state.control {
        for (k, v) in &c.investment_budget {
            check(format!("ctrl[{}].inv.{k}", fid), v.value);
        }
        for (k, v) in &c.construction_budget {
            check(format!("ctrl[{}].con.{k}", fid), v.value);
        }
        for (_, v) in &c.invest_weights {
            check(format!("ctrl[{}].iw", fid), v.value);
        }
        for (_, v) in &c.build_weights {
            check(format!("ctrl[{}].bw", fid), v.value);
        }
    }

    (bad, samples)
}

/// A compact health snapshot of a world at a round.
fn health_report(state: &State, config: &GameConfig) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "round={:>5} ships={:>4} cities={:>2} dead={:>2} wars={:>2} worldval={:>12.1}",
        state.round,
        state.ships.len(),
        state.cities.iter().filter(|c| !c.razed).count(),
        zombie_count(state),
        count_wars(state, config),
        world_value(state, config),
    ));

    // Per-faction summary where it is interesting.
    for f in &state.factions {
        let ships = state
            .ships
            .iter()
            .filter(|s| s.faction_id == f.name)
            .count();
        let cities = state
            .cities
            .iter()
            .filter(|c| c.faction_id == f.name && !c.razed)
            .count();
        let val = faction_value(f, config);
        let at_war = state
            .factions
            .iter()
            .filter(|o| o.name != f.name && hostile_p(f, o, config))
            .count();
        lines.push(format!(
            "   {:>6} ships={:>3} cities={:>2} value={:>12.2} wars={:>2}",
            f.name, ships, cities, val, at_war
        ));
    }
    lines.join("\n")
}

fn hostile_p(a: &Faction, b: &Faction, config: &GameConfig) -> bool {
    if a.name == b.name {
        return false;
    }
    a.relations.get(&b.name).copied().unwrap_or(0.0) <= config.combat.war_threshold
}

/// A "zombie" faction: no ships and no living city — it can produce nothing,
/// build nothing, colonize nothing, and can only ever be a bystander.
fn is_zombie(state: &State, fid: FactionId) -> bool {
    let has_ship = state.ships.iter().any(|s| s.faction_id == fid);
    let has_city = state.cities.iter().any(|c| c.faction_id == fid && !c.razed);
    !has_ship && !has_city
}

fn zombie_count(state: &State) -> usize {
    state
        .factions
        .iter()
        .filter(|f| is_zombie(state, f.name.clone()))
        .count()
}

fn count_wars(state: &State, config: &GameConfig) -> usize {
    let mut n = 0;
    for i in 0..state.factions.len() {
        for j in (i + 1)..state.factions.len() {
            if hostile_p(&state.factions[i], &state.factions[j], config) {
                n += 1;
            }
        }
    }
    n
}

fn world_value(state: &State, config: &GameConfig) -> f64 {
    state
        .factions
        .iter()
        .map(|f| faction_value(f, config))
        .sum()
}

fn check_state(state: &State, config: &GameConfig) {
    let (bad, samples) = count_nonfinite(state, config);
    assert_eq!(
        bad, 0,
        "non-finite numbers found after round {}: {:?}",
        state.round, samples
    );
}

/// Diagnostic (opt-in, `cargo test -- --ignored --nocapture`): run several seeds
/// far into the future and print, for each, a health report at checkpoints. This
/// is a maintainer's observability tool, not a fast CI guard — so it is `#[ignore]`d
/// to keep the default `cargo test` fast.
#[test]
#[ignore]
fn diagnose_long_horizon() {
    let config = load_config();
    let checkpoints = [50u32, 200, 500, 1000, 2000, 3000];
    for seed in [1u64, 42, 12345] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut next = 0usize;
        println!("\n===== seed {seed} =====");
        for _round in 1..=3000u32 {
            sim::advance(&mut state, &config, &mut rng);
            check_state(&state, &config);
            if next < checkpoints.len() && state.round == checkpoints[next] {
                println!("--- checkpoint {} ---", state.round);
                println!("{}", health_report(&state, &config));
                next += 1;
            }
        }
        println!("--- final ({}) ---", state.round);
        println!("{}", health_report(&state, &config));
    }
}

/// 平衡观测（`--ignored`）：删掉 resurgence 之后，世界的**整合程度**——还剩几家有城、
/// 几家亡国、最强的城占多少。这是**待调的平衡目标**（不是机制不变量），所以只记录不断言。
///
/// 实测（`config/game.ron` 默认值，1000 回合）：
/// * seed 1：3 家有城 / 6 家亡国；seed 7：3 / 6；seed 42：4 / 4。
/// * 唯一「有立足点保证」的旧时代（`step_resurgence`）是 9 家全活、峰值僵尸 ≤2、城占峰值 <0.85。
/// 也就是说：**「允许亡国」这一条机制改动，把世界从 9 家整合到 3-4 家**。
#[test]
#[ignore]
fn probe_world_health() {
    let config = load_config();
    for seed in [1u64, 7, 42] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut max_dead = 0usize;
        let mut max_top_share: f64 = 0.0;
        for _ in 0..1000 {
            sim::advance(&mut state, &config, &mut rng);
            let dead = zombie_count(&state);
            max_dead = max_dead.max(dead);
            let (_, share) = top_power(&state, &config);
            max_top_share = max_top_share.max(share);
        }
        let alive = state
            .factions
            .iter()
            .filter(|f| {
                state
                    .cities
                    .iter()
                    .any(|c| c.faction_id == f.name && !c.razed)
            })
            .count();
        println!(
            "seed {seed}: 末态有城势力={alive}/{} 亡国峰值={max_dead} 城占峰值={max_top_share:.3}",
            state.factions.len()
        );
    }
}

/// 多极格局（**平衡目标，待调**）：没有任何势力能长期垄断接近全部城市（峰值城占 < 0.85），
/// 且后半程的「最强者」会轮换（不是同一个人锁死）。
///
/// 这条**曾经**是机制保证——`step_resurgence` 让「无舰无城的势力」总能重新立足，于是没有谁能
/// 把对手彻底抹掉。D5 删掉它之后，「亡国」成为合法终局，世界会自然整合：
/// 实测 1000 回合，seed 1/7 只剩 3 家有城、seed 42 剩 4 家，城占峰值 1.000，后半程最强者
/// 在 seed 1 锁死为俄罗斯。
///
/// 所以这两条现在是**待调的平衡目标**（用户明确要求「平衡性慢慢调，先确立机制的正确性」）：
/// 用 `#[ignore]` 明确挂起并把实测值记在这里，**不是**悄悄删掉或放宽阈值。
/// 要让它变绿，需要补的是**平衡手段**（例如让战争更倾向于「易主」而不是「拆平」、给劣势方
/// 更强的复垦激励、或调维护费/治理曲线），而不是恢复凭空造城。
#[test]
#[ignore = "平衡待调（D5 允许亡国后世界整合到 3-4 家）：见 .agents/notes/trade-and-sanctions.md"]
fn world_is_multipolar() {
    let config = load_config();
    for seed in [1u64, 42] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut max_top_share: f64 = 0.0;
        let mut leaders = BTreeSet::new();
        for _ in 0..1000u32 {
            sim::advance(&mut state, &config, &mut rng);
            check_state(&state, &config);
            let (top, share) = top_power(&state, &config);
            max_top_share = max_top_share.max(share);
            if state.round >= 500 {
                leaders.insert(top);
            }
        }
        // 不统一：没有势力能吞并到接近 100% 的城市（峰值留出余量）。
        assert!(
            max_top_share < 0.85,
            "seed {seed}: 单一势力城市占比峰值 {max_top_share:.3} —— 世界有被一家独大垄断的趋势"
        );
        assert!(
            leaders.len() >= 2,
            "seed {seed}: 后半程最强势力始终是同一个人 {leaders:?} —— 存在长期锁死的单极霸权"
        );
    }
}

/// 当前综合实力最强的势力及其**占比**——按游戏的**单一权威**统计（`observe` 的
/// `power_share`，即 `sim::faction_power` 归一化），而非纯数城市。这样测试读到的是游戏
/// 合纵/遏制/制裁真正针对的那个「霸权」，不再有「测试以为的霸权 ≠ 游戏针对的霸权」分歧。
fn top_power(state: &State, config: &GameConfig) -> (FactionId, f64) {
    let m = sim::observe(state, config, &RoundSink::default());
    m.power_share
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| (k.clone(), *v))
        .unwrap_or((String::new(), 0.0))
}

/// 观测：合纵连横制裁严苛度调参（`--ignored`）。
#[test]
#[ignore]
fn probe_sanction() {
    let config = load_config();
    for seed in [1u64, 42, 12345] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut max_dead = 0usize;
        let mut max_top_share: f64 = 0.0;
        let mut top_sum: f64 = 0.0;
        let mut top_count = 0u32;
        let mut sm = std::collections::BTreeSet::new();
        for _ in 0..1200u32 {
            let flow = sim::advance(&mut state, &config, &mut rng);
            let dead = zombie_count(&state);
            let (top, share) = top_power(&state, &config);
            max_dead = max_dead.max(dead);
            max_top_share = max_top_share.max(share);
            if state.round >= 600 {
                top_sum += share;
                top_count += 1;
                sm.insert(top);
            }
            let _ = flow;
        }
        let avg = top_sum / top_count as f64;
        println!(
            "seed {seed}: max_dead={max_dead} max_top={max_top_share:.3} avg_top(half)={avg:.3} leaders={sm:?}"
        );
    }
}

/// 多极「科学测量」(`--ignored`)：不再只看「峰值 < 0.85 + 会轮换」，而是量化
/// `.agents/notes/balance-of-power.md` 的真正目标——
///   * `avg_top`       ：全局**长期**最强势力城占比（不是只看后半程，是整条轨迹逐回合平均）。
///   * `terminal_top`  ：最末回合最强势力城占比（末态是否又坍缩成一家独大 + 一堆旁观者）。
///   * `alive`         ：末回合仍有活城的势力数（>1 才叫「多方参与」，越多越好）。
///   * `zombies`       ：全局最大僵尸数（无舰无活城，永久旁观者）——越小越好。
///   * `gini`          ：末回合各势力「城占比」的基尼系数（0=完全均分，1=一家垄断）。
///   * `rotations`     ：最强势力每变化一次的次数（>1 才叫「轮换」，越多越不锁死）。
/// 把这些一次性打印成可比较的「多极健康报告」，供调参（平衡/治理/制裁）后横向对比。
#[test]
#[ignore]
fn probe_multipolar() {
    let config = load_config();
    for seed in [1u64, 42, 12345] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut top_sum = 0.0f64;
        let mut top_count = 0u32;
        let mut max_zombies = 0usize;
        let mut leader = String::new();
        let mut rotations = 0u32;
        for _ in 0..3000u32 {
            sim::advance(&mut state, &config, &mut rng);
            let (top, share) = top_power(&state, &config);
            top_sum += share;
            top_count += 1;
            max_zombies = max_zombies.max(zombie_count(&state));
            if top != leader {
                if !leader.is_empty() {
                    rotations += 1;
                }
                leader = top;
            }
        }
        let (_, terminal_top) = top_power(&state, &config);
        let alive = state
            .factions
            .iter()
            .filter(|f| {
                state
                    .cities
                    .iter()
                    .any(|c| c.faction_id == f.name && !c.razed)
            })
            .count();
        // 吉尼：排序末回合各势力城占比，算基尼系数。
        let mut shares: Vec<f64> = state
            .factions
            .iter()
            .map(|f| {
                let n = state
                    .cities
                    .iter()
                    .filter(|c| c.faction_id == f.name && !c.razed)
                    .count() as f64;
                n / state.cities.iter().filter(|c| !c.razed).count().max(1) as f64
            })
            .collect();
        shares.retain(|&s| s > 0.0);
        shares.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = shares.len().max(1) as f64;
        let gini = if shares.is_empty() {
            0.0
        } else {
            let mut cum = 0.0;
            let mut sum = 0.0;
            for (i, &s) in shares.iter().enumerate() {
                cum += s;
                sum += (i as f64 + 1.0) * s;
            }
            (2.0 * sum / cum / n - (n + 1.0) / n).max(0.0)
        };
        println!(
            "seed {seed}: avg_top={:.3} terminal_top={:.3} rotations={rotations} alive={alive} zombies={max_zombies} gini={gini:.3}",
            top_sum / top_count as f64,
            terminal_top
        );
    }
}

/// 观测：在最高城占那一回合，最强势力各城到其首都的距离分布（看峰值是「紧凑区域帝国」
/// 还是「四处扩张」造成的——据此判断过度扩张制裁是否有效）。`--ignored`。
#[test]
#[ignore]
fn probe_peak() {
    let config = load_config();
    for seed in [1u64, 42, 12345] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let (mut peak_top, mut peak_share, mut peak_round) = (String::new(), 0.0f64, 0u32);
        for _ in 0..1200u32 {
            sim::advance(&mut state, &config, &mut rng);
            let (top, share) = top_power(&state, &config);
            if share > peak_share {
                peak_share = share;
                peak_top = top;
                peak_round = state.round;
            }
        }
        // Re-run to the peak round and dump the leader's cities.
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        for _ in 0..peak_round {
            sim::advance(&mut state, &config, &mut rng);
        }
        let mut ds: Vec<f64> = state
            .cities
            .iter()
            .filter(|c| c.faction_id == peak_top && !c.razed)
            .map(|c| planet_x::agent::governance_distance(&state, &peak_top, &c.body_id))
            .collect();
        ds.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let far = ds.iter().filter(|&&d| d > 18.0).count();
        println!(
            "seed {seed}: peak r{peak_round} top={peak_top} share={peak_share:.3} cities={} dist_range=[{:?} .. {:?}] far(>18)={far}",
            ds.len(),
            ds.first().copied().unwrap_or(0.0),
            ds.last().copied().unwrap_or(0.0)
        );
    }
}

/// 诊断：seed 1 中僵尸（无舰无活城）势力出现 3 个时的回合与各家状态。`--ignored`。
#[test]
#[ignore]
fn probe_zombies() {
    let config = load_config();
    let mut state = world::default_state(&config, 1);
    let mut rng = Prng::new(1);
    for _ in 0..1000u32 {
        sim::advance(&mut state, &config, &mut rng);
        if zombie_count(&state) >= 3 {
            println!(
                "--- round {} dead {} ---",
                state.round,
                zombie_count(&state)
            );
            for f in &state.factions {
                let ships = state
                    .ships
                    .iter()
                    .filter(|s| s.faction_id == f.name)
                    .count();
                let cities = state
                    .cities
                    .iter()
                    .filter(|c| c.faction_id == f.name && !c.razed)
                    .count();
                let razed = state
                    .cities
                    .iter()
                    .filter(|c| c.faction_id == f.name && c.razed)
                    .count();
                println!(
                    "  {} ships={ships} cities={cities} own_razed={razed}",
                    f.name
                );
            }
            let total_settlements: usize = state.bodies.iter().map(|b| b.settlements.len()).sum();
            let total_cities = state.cities.len();
            println!("  settlements={total_settlements} cities={total_cities}");
            return;
        }
    }
    println!("never reached 3 zombies");
}

/// 【探针·debuff 行为】单种子快速观测：逐回合打印「最强势力 + 各势力思潮 debuff 惩罚」，
/// 用于调 `IdeologyDebuffConfig` 系数（看哪个势力被打击、多狠、是否真能扳倒单极化）。
/// `--ignored`。
#[test]
#[ignore]
fn probe_debuff_behavior() {
    let config = load_config();
    for seed in [1u64, 42] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut leader = String::new();
        let mut streak = 0u32;
        for _ in 0..900u32 {
            sim::advance(&mut state, &config, &mut rng);
            let (top, share) = top_power(&state, &config);
            if top != leader {
                if state.round >= 100 && state.round % 40 <= 1 {
                    println!("seed {seed} r{} leader-> {top}", state.round);
                }
                leader = top.clone();
                streak = 1;
            } else {
                streak += 1;
            }
            if state.round % 150 == 0 {
                let p = total_ideology_penalty(&state, &config);
                let mut s: Vec<String> = p.iter().map(|(k, v)| format!("{k}={v:.2}")).collect();
                s.sort();
                println!(
                    "  seed {seed} r{}  top={top}({share:.2})  debts: {s:?}",
                    state.round
                );
            }
        }
        println!("== seed {seed} final leader={leader} streak={streak} ==");
    }
}

// 临时：各势力思潮 debuff 惩罚（用 sim 公开的可观测接口）。
fn total_ideology_penalty(
    state: &State,
    config: &GameConfig,
) -> std::collections::BTreeMap<String, f64> {
    sim::faction_ideology_debuffs(state, config)
}

/// 【探针·技术 vs 科学 谁更优势】短局量化：量测 `science_tech` 轴两端（技术正端 / 科学负端）
/// 与「实力占比」的关系——正端（开采 MOND 区资源的势力）是否普遍更强、更常坐上最强位。
/// `--ignored`。
#[test]
#[ignore]
fn probe_tech_vs_science() {
    let config = load_config();
    let seeds = [1u64, 2, 3, 4, 5, 7, 11];
    const ROUNDS: u32 = 200;

    let mut all: Vec<(f64, f64)> = Vec::new(); // (science_tech, power_share)
    let mut tech_top = 0usize;
    let mut sci_top = 0usize;
    let mut tech_power_sum = 0.0;
    let mut sci_power_sum = 0.0;
    let mut tech_n = 0usize;
    let mut sci_n = 0usize;

    for seed in seeds {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        for _ in 0..ROUNDS {
            sim::advance(&mut state, &config, &mut rng);
        }
        let m = sim::observe(&state, &config, &planet_x::model::RoundSink::default());
        for f in &state.factions {
            let st = f.ideology.science_tech;
            if let Some(&ps) = m.power_share.get(&f.name) {
                all.push((st, ps));
                if st > 0.0 {
                    tech_power_sum += ps;
                    tech_n += 1;
                } else if st < 0.0 {
                    sci_power_sum += ps;
                    sci_n += 1;
                }
            }
        }
        // 谁是最强位，落在哪端。
        let top = m
            .power_share
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(k, _)| k.clone())
            .unwrap();
        let st = state
            .faction(&top)
            .map(|f| f.ideology.science_tech)
            .unwrap_or(0.0);
        let top_share = m.power_share.get(&top).copied().unwrap_or(0.0);
        if st > 0.0 {
            tech_top += 1;
        } else {
            sci_top += 1;
        }
        println!("seed {seed}: top={top} share={top_share:.2} science_tech_of_top={st:+.2}");
    }

    // 皮尔逊相关：science_tech 与 power_share。
    let (n, sx, sy, sxx, syy, sxy) = all.iter().fold(
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
        |(n, sx, sy, sxx, syy, sxy), (x, y)| {
            (
                n + 1.0,
                sx + x,
                sy + y,
                sxx + x * x,
                syy + y * y,
                sxy + x * y,
            )
        },
    );
    let corr = if n > 1.0 {
        let denom = ((n * sxx - sx * sx) * (n * syy - sy * sy)).sqrt();
        if denom > 1e-9 {
            (n * sxy - sx * sy) / denom
        } else {
            0.0
        }
    } else {
        0.0
    };
    println!(
        "--- 汇总: tech端 n={tech_n}(均实力{:.3})  science端 n={sci_n}(均实力{:.3})  corr(science_tech,power)={corr:.3}  最强=n_tech:{tech_top} n_science:{sci_top}",
        tech_power_sum / tech_n.max(1) as f64,
        sci_power_sum / sci_n.max(1) as f64,
    );
}

/// 【探针·单极化思潮画像】长局扫描：找出「单极化」结局（同一势力长时间霸占最强位），
/// 记录该霸权在锁定期末的**思潮向量**，用于识别「优势思潮」（该给 debuff 的画像）。
///
/// 单极化判定：同一势力连续 ≥ `MIN_LOCK` 回合稳居 `top_power` 最强位。锁定时记录
/// 霸权名、实力占比、4 条思潮轴均分（锁定期最近 [`AVG_WINDOW`] 回合的平均）。
///
/// 输出为行分隔 JSON：`{"span":..,"seed":..,"locked":true,"hegemon":..,"share":..,
/// "pm":..,"st":..,"pe":..,"nc":..,"locklen":..}`；`locked=false` 表示该局未单极化。
/// `--ignored`（不含默认测试，也不消耗常规 CI）。
#[test]
#[ignore]
fn probe_polar_ideology() {
    const MIN_LOCK: u32 = 120; // 连续同霸主的回合数阈值
    const ROUNDS: u32 = 1000;
    const AVG_WINDOW: usize = 40;
    let spans = [0.0f64, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 16.0];

    for span in spans {
        for seed in 1..=10u64 {
            let mut config = load_config();
            config.diplomacy.ideology_affinity_span = span;
            let mut state = world::default_state(&config, seed);
            let mut rng = Prng::new(seed);

            // 逐回合记录最强位：谁是当前 #1、它已连续坐庄多少回合（截至当前回合）。
            // 单极化判定：**末段**最强位由同一势力连续 ≥ MIN_LOCK 回合（截至末回合）——
            // 这才是「单极化结局」，而非中途闪现的短暂锁。
            let mut leader = String::new();
            let mut streak = 0u32; // 当前 #1 已连续坐庄回合数
            let mut max_streak = 0u32;
            let mut buf: Vec<(String, f64, f64, f64, f64)> = Vec::new(); // 末段 #1 的最近思潮

            for _ in 0..ROUNDS {
                sim::advance(&mut state, &config, &mut rng);
                let (top, _share) = top_power(&state, &config);
                if top == leader {
                    streak += 1;
                } else {
                    leader = top.clone();
                    streak = 1;
                    buf.clear();
                }
                max_streak = max_streak.max(streak);
                // 缓冲当前 #1 的思潮（仅当它已是本段 #1 且有望成为末段霸权的候选）。
                if let Some(f) = state.faction(&leader) {
                    buf.push((
                        leader.clone(),
                        f.ideology.peace_military,
                        f.ideology.science_tech,
                        f.ideology.people_elite,
                        f.ideology.nature_colony,
                    ));
                }
                if buf.len() > AVG_WINDOW {
                    buf.remove(0);
                }
            }

            // 末回合的 #1 及其连续坐庄长度 —— 达阈值即单极化结局。
            let (terminal_top, terminal_share) = top_power(&state, &config);
            let locked = terminal_top == leader && streak >= MIN_LOCK;
            if locked {
                let n = buf.len() as f64;
                let (pm, st, pe, nc) = {
                    let mut a = 0.0;
                    let mut b = 0.0;
                    let mut c = 0.0;
                    let mut d = 0.0;
                    for r in &buf {
                        a += r.1;
                        b += r.2;
                        c += r.3;
                        d += r.4;
                    }
                    (a / n, b / n, c / n, d / n)
                };
                println!(
                    "{{\"span\":{span},\"seed\":{seed},\"locked\":true,\"hegemon\":\"{terminal_top}\",\"share\":{terminal_share:.3},\"pm\":{pm:.3},\"st\":{st:.3},\"pe\":{pe:.3},\"nc\":{nc:.3},\"locklen\":{streak}}}"
                );
            } else {
                println!(
                    "{{\"span\":{span},\"seed\":{seed},\"locked\":false,\"terminal\":\"{terminal_top}\",\"share\":{terminal_share:.3},\"max_streak\":{max_streak}}}"
                );
            }
        }
    }
}
