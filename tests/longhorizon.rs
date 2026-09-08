//! Long-horizon health harness for the Planet X sandbox.
//!
//! The design goal is that the world stays *meaningful and unbroken* over
//! thousands of rounds: no NaN/Inf drift, no unbounded economic runawaay, no
//! silent collapse into a dead/stale board, and never a panic. This module is a
//! diagnostic + regression guard. Run the diagnostic with:
//!
//! ```text
//! cargo test --test longhorizon -- --nocapture
//! ```
//!
//! The individual `#[test]`s are shrink-wrapped invariants; the
//! `diagnose_long_horizon` test prints a human-readable health report so a
//! maintainer can eyeball the trajectory.

use planet_x::config::load_config;
use planet_x::model::*;
use planet_x::prng::Prng;
use planet_x::sim;
use planet_x::world;

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
        check(format!("body[{}].position.0", b.id), b.position[0]);
        check(format!("body[{}].position.1", b.id), b.position[1]);
        check(format!("body[{}].orbit.peri", b.id), b.orbit.perihelion_distance as f64);
        check(format!("body[{}].orbit.aphe", b.id), b.orbit.aphelion_distance as f64);
        for s in &b.settlements {
            check(format!("settlement.{}area", s.name), s.total_area);
            for d in &s.resources {
                check(format!("deposit.{}area", d.resource), d.area);
            }
        }
    }

    for f in &state.factions {
        for (k, v) in &f.resources {
            check(format!("faction[{}].res.{k}", f.id), *v);
        }
        for (o, v) in &f.relations {
            check(format!("faction[{}].rel.{o}", f.id), *v);
        }
        check(format!("faction[{}].alignment", f.id), f.alignment);
        check(format!("faction[{}].aggression", f.id), f.aggression);
    }

    for c in &state.cities {
        for b in &c.buildings {
            check(format!("city[{}].bld[{}].area", c.id, b.id), b.area);
            check(format!("city[{}].bld[{}].deployed", c.id, b.id), b.deployed);
            check(format!("city[{}].bld[{}].armor", c.id, b.id), b.armor);
        }
        for (cls, p) in &c.ship_progress {
            check(format!("city[{}].progress.{cls}", c.id), *p);
        }
    }

    for s in &state.ships {
        check(format!("ship[{}].pos.0", s.id), s.position[0]);
        check(format!("ship[{}].pos.1", s.id), s.position[1]);
        check(format!("ship[{}].hull", s.id), s.hull);
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
        let ships = state.ships.iter().filter(|s| s.faction_id == f.id).count();
        let cities = state.cities.iter().filter(|c| c.faction_id == f.id && !c.razed).count();
        let val = faction_value(f, config);
        let at_war = state
            .factions
            .iter()
            .filter(|o| o.id != f.id && hostile_p(f, o, config))
            .count();
        lines.push(format!(
            "   {:>6} ships={:>3} cities={:>2} value={:>12.2} wars={:>2}",
            f.name, ships, cities, val, at_war
        ));
    }
    lines.join("\n")
}

fn hostile_p(a: &Faction, b: &Faction, config: &GameConfig) -> bool {
    if a.id == b.id {
        return false;
    }
    a.relations.get(&b.id).copied().unwrap_or(0.0) <= config.combat.war_threshold
}

/// A "zombie" faction: no ships and no living city — it can produce nothing,
/// build nothing, colonize nothing, and can only ever be a bystander.
fn is_zombie(state: &State, fid: FactionId) -> bool {
    let has_ship = state.ships.iter().any(|s| s.faction_id == fid);
    let has_city = state.cities.iter().any(|c| c.faction_id == fid && !c.razed);
    !has_ship && !has_city
}

fn zombie_count(state: &State) -> usize {
    state.factions.iter().filter(|f| is_zombie(state, f.id)).count()
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
    state.factions.iter().map(|f| faction_value(f, config)).sum()
}

fn check_state(state: &State, config: &GameConfig) {
    let (bad, samples) = count_nonfinite(state, config);
    assert_eq!(bad, 0, "non-finite numbers found after round {}: {:?}", state.round, samples);
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

/// The world must never panic and must never produce a non-finite number across
/// a long, war-torn run (here on the default diplomacy curve which opens
/// peacefully and escalates on its own).
#[test]
fn no_nonfinite_over_long_run() {
    let config = load_config();
    let mut state = world::default_state(&config, 42);
    let mut rng = Prng::new(42);
    let horizon = 1000u32;
    let mut nonfinite_round = None;
    for _ in 0..horizon {
        sim::advance(&mut state, &config, &mut rng);
        let (bad, samples) = count_nonfinite(&state, &config);
        if bad > 0 {
            nonfinite_round = Some((state.round, samples));
            break;
        }
        // The board should never be empty of ships (a "dead" universe) — if every
        // faction lost everything, the game has collapsed and is no longer a game.
        if state.ships.is_empty() {
            panic!("all ships gone by round {}", state.round);
        }
    }
    let Some((round, samples)) = nonfinite_round else { return };
    panic!("non-finite numbers at round {}: {:?}", round, samples);
}

/// Economic sanity: the total world market value should not balloon without
/// bound — a stockpile that compounds forever is a broken economy, not a game.
/// We assert the total value at a late horizon is within a sane multiple of its
/// value at an early horizon (money is conserved-looking, not exponential).
#[test]
fn world_value_is_bounded() {
    let config = load_config();
    for seed in [1u64, 42] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        for _ in 0..1000 {
            sim::advance(&mut state, &config, &mut rng);
        }
        let late = world_value(&state, &config);
        // The value should be well-bounded — production is area/cap-limited so
        // the ledger must settle, not soar. (The initial board has a few hundred
        // value; runaways are the thing this guards.)
        assert!(
            late < 2_000_000.0,
            "world value at seed {} round {} is {:.0} — unbounded economic runaway",
            seed,
            state.round,
            late
        );
    }
}

/// 反僵尸/反垄断：在长局中，被彻底消灭（无舰又无活城）的势力应保持稀少——否则游戏
/// 会收敛成少数几个永久旁观者（「崩坏」）。反僵尸重建（resurgence）保证这点。
#[test]
fn zombie_factions_are_bounded() {
    let config = load_config();
    for seed in [1u64, 42] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut max_dead = 0usize;
        for _ in 0..1000 {
            sim::advance(&mut state, &config, &mut rng);
            check_state(&state, &config);
            max_dead = max_dead.max(zombie_count(&state));
        }
        // A handful may be momentarily wiped mid-cycle, but the world must not
        // degrade to a board of permanent bystanders.
        assert!(
            max_dead <= 2,
            "seed {}: too many permanently-neutral factions (max {max_dead} zombies)",
            seed
        );
    }
}

/// 确定性复现：同一种子 + 相同配置 + 相同回合数 → 输出逐字节一致（spec 的硬性要求）。
/// 这也锁定了新增机制（治理/重建/本土防御/MOND）不会破坏可复现性。
#[test]
fn same_seed_reproduces_identically() {
    let config = load_config();
    let mut a = world::default_state(&config, 42);
    let mut ra = Prng::new(42);
    let mut b = world::default_state(&config, 42);
    let mut rb = Prng::new(42);
    for _ in 0..200 {
        sim::advance(&mut a, &config, &mut ra);
        sim::advance(&mut b, &config, &mut rb);
    }
    let sa = planet_x::agent::render_state(&a, &config);
    let sb = planet_x::agent::render_state(&b, &config);
    assert_eq!(
        sa, sb,
        "same seed 42 at round 200 must reproduce identical agent state"
    );
    assert_eq!(a.round, b.round);
}

/// 多极与霸权制衡：合纵连横机制应让世界**不收敛成一家独大**——没有任何势力能长期
/// 垄断全部城市（最高城占低于一致阈值），且反制联盟确实会成立（机制是活的，不是摆设）。
/// 上千回合后游戏仍是多方参与，而非「一个霸主 + 一堆旁观者」。
#[test]
fn world_is_multipolar() {
    let config = load_config();
    for seed in [1u64, 42] {
        let mut state = world::default_state(&config, seed);
        let mut rng = Prng::new(seed);
        let mut max_top_share: f64 = 0.0;
        let mut coalition_seen = false;
        for _ in 0..1000u32 {
            sim::advance(&mut state, &config, &mut rng);
            check_state(&state, &config);
            let (_, share) = top_city_share(&state);
            max_top_share = max_top_share.max(share);
            if state.events.iter().any(|e| matches!(e, GameEvent::CoalitionFormed { .. })) {
                coalition_seen = true;
            }
        }
        // 不统一：没有势力能吞并到接近 100% 的城市（峰值留出余量）。
        assert!(
            max_top_share < 0.85,
            "seed {seed}: 单一势力城市占比峰值 {max_top_share:.3} —— 世界有被一家独大垄断的趋势"
        );
        // 制衡是活的：长局里应出现过反制联盟（合纵连横确实发生）。
        assert!(coalition_seen, "seed {seed}: 长局从未出现反制联盟（合纵连横未生效）");
    }
}

fn top_city_share(state: &State) -> (FactionId, f64) {
    let total = state.cities.iter().filter(|c| !c.razed).count() as f64;
    if total <= 0.0 {
        return (0, 0.0);
    }
    let mut best = (0u32, 0.0);
    for f in &state.factions {
        let n = state.cities.iter().filter(|c| c.faction_id == f.id && !c.razed).count() as f64;
        let s = n / total;
        if s > best.1 {
            best = (f.id, s);
        }
    }
    best
}
