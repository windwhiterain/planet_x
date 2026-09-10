//! 舰种 / 组件选装（AI 决定造什么）+ 海军随威胁重构。

use crate::model::*;
use crate::prng::Prng;
use crate::sim;
use std::collections::BTreeMap;

/// Pick a ship class for a new colony / new shipyard (with no class yet). Instead of
/// "random among fully-affordable" (which a cash-limited faction collapses to corvette),
/// the AI sizes its navy to what its **resource profile can fit** (soft affordability:
/// having most of the minerals counts, not all at once) and **diversifies** — it
/// prefers classes it currently has few of. So the fleet grows into a **mixed navy**
/// (screens + warships + carriers), not a one-class blob. Deterministic: the seeded
/// RNG drives a weighted pick over class scores (variety), reproducible per seed.
pub(crate) fn choose_next_class(state: &State, fid: &str, config: &GameConfig, rng: &mut Prng) -> String {
    let Some(f) = state.faction(fid) else { return "corvette".to_string() };
    let value_of = |r: &str| config.resources.get(r).map(|rr| rr.value).unwrap_or(1.0);
    let mut max_res = 0.0f64;
    for (r, v) in &f.resources {
        max_res = max_res.max(*v * value_of(r));
    }
    let ab = |r: &str| {
        if max_res > 1e-9 {
            f.resources.get(r).map(|v| *v * value_of(r) / max_res).unwrap_or(0.0)
        } else {
            0.0
        }
    };

    // Current fleet composition by class (to know what the navy already has).
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for s in &state.ships {
        if s.faction_id == fid && s.hull > 0.0 {
            *counts.entry(s.class.clone()).or_insert(0) += 1;
            total += 1;
        }
    }
    let total = total.max(1);
    let at_war = sim::faction_at_war(state, config, fid);

    // Score: a **normalized** resource fit — how well the faction's profile covers the
    // class's cost (0..1, so cheap and expensive hulls are on the same scale: scarce
    // minerals lower it, but cost magnitude does not inflate it) — plus a
    // diversification bonus for under-represented classes, minus an upkeep penalty,
    // plus a **war bonus** (at war the AI leans toward high-firepower hulls). This
    // yields a mixed navy that adapts to the threat without letting a rich faction's
    // expensive hulls run away with the score (and the war).
    let mut scored: Vec<(String, f64)> = Vec::new();
    for (cls, spec) in &config.ships {
        let cost_val: f64 = spec.build_cost.iter().map(|(r, c)| c * value_of(r)).sum();
        let covered: f64 = spec.build_cost.iter().map(|(r, c)| c * value_of(r) * ab(r)).sum();
        let fit = if cost_val > 1e-9 { covered / cost_val } else { 0.0 };
        let share = counts.get(cls).copied().unwrap_or(0) as f64 / total as f64;
        // Classes the faction has < 25% of get a pull toward a balanced mix.
        let mix_bonus = (0.25 - share).max(0.0) * 1.5;
        let upkeep_penalty = spec.upkeep * 0.04; // 贵舰难养，只有当资源/构成都支持才造
        // 威胁响应：战时给「主力旗舰」舰型加分（多造战列/航母）。现在舰级是平台修正器、
        // 没有独立 attack；用「主力舰强度 = 造价高(build_points≥40) 且维持费高(≥6.5)」
        // 这一离散标志区分旗舰（战列/航母）与巡洋/护卫，给一个明确的战争加成，避免和
        // 巡洋（造价相近）混在一起。
        let flagship = spec.build_points >= 40.0 && spec.upkeep >= 6.5;
        let war_bonus = if at_war && flagship { 0.6 } else { 0.0 };
        scored.push((cls.clone(), fit + mix_bonus - upkeep_penalty + war_bonus));
    }

    // Weighted random pick → variety; deterministic via the seeded RNG.
    let total_score: f64 = scored.iter().map(|(_, s)| s.max(0.0)).sum();
    if total_score <= 1e-9 {
        return "corvette".to_string();
    }
    let mut roll = rng.unit() * total_score;
    let mut last = "corvette".to_string();
    for (cls, s) in &scored {
        last = cls.clone();
        roll -= s.max(0.0);
        if roll <= 0.0 {
            return cls.clone();
        }
    }
    last
}

/// Deterministically pick a ship component loadout (舰船定制) for a faction building
/// a ship of `class` — the **resource → military** link, now also **category-balanced
/// and threat-aware**:
///   * components whose rare inputs the faction has in abundance score highest
///     (resource advantage); unaffordable ones are dropped;
///   * the loadout is balanced across weapon / defense / support so a ship can both
///     hit and survive (a real commander doesn't field a mono-stack of glass cannons —
///     it guarantees at least one weapon and, when the ship has ≥2 slots, one defense);
///   * when at war the AI is biased toward weapons (weapon score bonus), so it invests
///     in firepower; in peace it invests more in defense/support.
/// Deterministic (no RNG): score is a function of stockpile + config, ties break on
/// component id. Returns ≤ `ShipSpec::slots` component ids, cumulatively affordable.
/// `pub(crate)` so `world` can fit the starting / re-seeded / story-granted ships the
/// same way a shipyard does (a ship's firepower is entirely its fitted modules).
pub(crate) fn choose_loadout(state: &State, config: &GameConfig, fid: FactionId, class: &str) -> Vec<String> {
    let slots = config.ship_spec(class).slots as usize;
    if slots == 0 {
        return Vec::new();
    }
    let Some(f) = state.faction(&fid) else { return Vec::new() };
    let value_of = |r: &str| config.resources.get(r).map(|rr| rr.value).unwrap_or(1.0);

    // Normalized resource abundance by market value in the stockpile.
    let mut max_ab = 0.0f64;
    for (r, v) in &f.resources {
        max_ab = max_ab.max(*v * value_of(r));
    }
    if max_ab <= 1e-9 {
        return Vec::new();
    }
    let abund = |r: &str| f.resources.get(r).map(|v| *v * value_of(r) / max_ab).unwrap_or(0.0);

    // 战局感知：交战中的势力更看重武器（武器加分），和平时更偏向防御/支持。
    let at_war = sim::faction_at_war(state, config, &fid);
    // 舰级 = 平台修正器：组件对战斗的实际贡献被本舰级的修正系数缩放（战列=火力放大器、
    // 护卫=极速、航母=超远程）。这使「选什么模块」要和「装在哪级舰上」配套。
    let spec = config.ship_spec(class);

    // Score every candidate component by (a) resource fit — how much of its rare
    // inputs the faction can comfortably supply — plus (b) a small (class-scaled)
    // raw combat-gain tiebreak, minus (c) an upkeep drag. 战时给武器加分（更舍得堆火力）。
    let mut cands: Vec<(String, f64)> = Vec::new();
    for (id, cs) in &config.components {
        let fit: f64 = cs.cost.iter().map(|(r, c)| c * value_of(r) * abund(r)).sum();
        // 用舰级修正系数缩放每件模块的「战斗增益」，让选装与舰型匹配。
        let gain_weapon = cs.damage * spec.attack_mult;
        let gain_shield = cs.shield * spec.shield_mult;
        let gain_speed = cs.speed * spec.speed_mult;
        let gain_accel = cs.accel * spec.accel_mult;
        let gain_range = cs.range * spec.range_mult;
        let gain_regen = cs.shield_regen * spec.shield_regen_mult;
        let gain = gain_weapon * 4.0 + gain_shield * 0.8 + cs.hardness * spec.armor_mult * 3.0
            + gain_regen * 60.0 + gain_speed * 3.0 + gain_accel * 3.0
            + cs.intercept * spec.pd_mult * 2.0 + gain_range * 12.0;
        let mut score = fit + gain * 0.03 - cs.upkeep * 2.0;
        if at_war && cs.category == "weapon" {
            score += gain_weapon * 2.0; // 战时要火力。
        }
        if score > 0.0 {
            cands.push((id.clone(), score));
        }
    }
    cands.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    // 类别配比（拟人指挥官）：一艘军舰要么能打、要么能跑——**至少一件武器（硬保证）**、
    // **至少一件推进（硬保证：没有推进就没有速度/加速度，船动不了）**，其余按分数填满
    // （护盾/护甲/点防/辅助是可选的防御与支持，不硬性要求）。
    let mut chosen: Vec<String> = Vec::new();
    let mut remaining = f.resources.clone();
    let afford = |id: &str, rem: &ResourceMap| -> bool {
        config
            .component_spec(id)
            .cost
            .iter()
            .all(|(r, c)| rem.get(r).copied().unwrap_or(0.0) >= *c)
    };
    let count_cat = |chosen: &Vec<String>, cat: &str| -> usize {
        chosen.iter().filter(|id| config.component_spec(id).category == cat).count()
    };

    // 强制装配一件指定类别（买得起选最高分；买不起时按 `free_fallback` 决定怎么办）。
    //
    // `free_fallback` 区分**平台**与**军备**：
    // * **推进器是平台**（`true`）：没有推进器的舰根本下不了水（速度=0 的船坞废铁），
    //   所以船坞无论如何都会给它装上——这是「能不能出厂」的问题，不是「买不买得起」的问题。
    // * **武器是军备**（`false`）：**买不起就不装**（M7 硬门槛）。旧版在这里无视库存强塞最便宜
    //   的一件并把库存钳到 0，于是「全世界最稀缺的氦-3/金/铀」对军备毫无约束——制裁也就
    //   咬不到任何东西。现在缺稀有矿的势力**退回廉价配置**（动能炮 = 铁+碳），而不是白拿。
    let force_cat = |cat: &str, chosen: &mut Vec<String>, remaining: &mut ResourceMap, free_fallback: bool| {
        if count_cat(chosen, cat) >= 1 {
            return;
        }
        for (id, _) in &cands {
            if count_cat(chosen, cat) >= 1 {
                break;
            }
            if config.component_spec(id).category == cat && !chosen.contains(id) && afford(id, remaining) {
                let cs = config.component_spec(id);
                for (r, c) in &cs.cost {
                    *remaining.entry(r.clone()).or_insert(0.0) -= c;
                }
                chosen.push(id.clone());
            }
        }
        if count_cat(chosen, cat) == 0 {
            // 买得起的里面挑最便宜的（省钱兜底）。
            let mut cheapest_affordable: Option<(String, f64)> = None;
            for (id, cs) in &config.components {
                if cs.category != cat || !afford(id, remaining) {
                    continue;
                }
                let cost_val: f64 = cs.cost.iter().map(|(r, c)| c * value_of(r)).sum();
                if cheapest_affordable
                    .as_ref()
                    .map(|(_, c)| cost_val < *c)
                    .unwrap_or(true)
                {
                    cheapest_affordable = Some((id.clone(), cost_val));
                }
            }
            if let Some((id, _)) = cheapest_affordable {
                let cs = config.component_spec(&id);
                for (r, c) in &cs.cost {
                    *remaining.entry(r.clone()).or_insert(0.0) -= c;
                }
                chosen.push(id);
            } else if free_fallback {
                // 平台部件：付不起也装（下不了水的船没有意义）。
                let mut cheapest: Option<(String, f64)> = None;
                for (id, cs) in &config.components {
                    if cs.category != cat {
                        continue;
                    }
                    let cost_val: f64 = cs.cost.iter().map(|(r, c)| c * value_of(r)).sum();
                    if cheapest.as_ref().map(|(_, c)| cost_val < *c).unwrap_or(true) {
                        cheapest = Some((id.clone(), cost_val));
                    }
                }
                if let Some((id, _)) = cheapest {
                    let cs = config.component_spec(&id);
                    for (r, c) in &cs.cost {
                        let e = remaining.entry(r.clone()).or_insert(0.0);
                        *e = (*e - c).max(0.0); // 买不起也不至于负——平台兜底。
                    }
                    chosen.push(id);
                }
            }
        }
    };
    // 硬保证：至少一件武器（攻击力来源，**稀缺在此咬人**）+ 至少一件推进（速度来源，平台）。
    force_cat("weapon", &mut chosen, &mut remaining, false);
    force_cat("thrust", &mut chosen, &mut remaining, true);
    // 填满剩余槽位（按分数；护盾/护甲/点防/辅助/额外部件可选）。
    for (id, _) in &cands {
        if chosen.len() >= slots {
            break;
        }
        if chosen.contains(id) {
            continue;
        }
        if afford(id, &remaining) {
            let cs = config.component_spec(id);
            for (r, c) in &cs.cost {
                *remaining.entry(r.clone()).or_insert(0.0) -= c;
            }
            chosen.push(id.clone());
        }
    }
    chosen
}

/// 威胁响应（海军随威胁重构）：交战中，若某势力的舰队被单一舰型统治（占比 > `over_share`），
/// 就把它产出该舰型的最小 id 船坞重定向到 `choose_next_class` 选出的**战局感知新舰型**
/// （战争加分——多造重舰；去重加分——避免单调）。和平时不重定向（船坞保持生产既有舰型）。
/// 每次至多重定向一个船坞、且只在明显过度生产时触发，避免抖振。确定性（seeded RNG）。
///
/// 真的改了就往 `retools` **追加一行**（纯记录）：这是少数几个**不留事件的 AI 决策**之一，
/// 事后只能从 `ship_type` 的变化反推、且看不出是什么时候改的。
pub(crate) fn retool_shipyards(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    rng: &mut Prng,
    retools: &mut Vec<RetoolDecision>,
) {
    if !sim::faction_at_war(state, config, fid) {
        return;
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for s in &state.ships {
        if s.faction_id == fid && s.hull > 0.0 {
            *counts.entry(s.class.clone()).or_insert(0) += 1;
            total += 1;
        }
    }
    if total == 0 {
        return;
    }
    let (over_class, over_count) = counts.iter().max_by_key(|(_, n)| **n).map(|(k, n)| (k.clone(), *n)).unwrap();
    // 舰队不是被单一舰型**严重**统治就不重定向（保守：只在极度单一时触发，避免扰动
    // 权力平衡与「霸权→联盟」的合纵连横节奏）。
    if (over_count as f64) / (total as f64) < 0.60 {
        return;
    }
    let new_class = choose_next_class(state, fid, config, rng);
    if new_class == over_class {
        return;
    }
    // 找到产出 over_class 的最小 id 船坞（按城市 id、再按建筑 id）。
    let mut target: Option<(CityId, BuildingId)> = None;
    for c in &state.cities {
        if c.faction_id != fid {
            continue;
        }
        for b in &c.buildings {
            if b.is_shipyard() && b.ship_type.as_deref() == Some(over_class.as_str()) {
                target = Some((c.name.clone(), b.id));
                break;
            }
        }
        if target.is_some() {
            break;
        }
    }
    if let Some((cid, bid)) = target {
        if let Some(c) = state.city_mut(&cid) {
            for b in &mut c.buildings {
                if b.id == bid {
                    b.ship_type = Some(new_class.clone());
                    break;
                }
            }
        }
        retools.push(RetoolDecision {
            faction: fid.to_string(),
            city: cid,
            building: bid,
            from: over_class,
            to: new_class,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::prng::Prng;
    use crate::world::default_state;
    use std::collections::BTreeSet;

    /// Build the config + a fresh deterministic world (round 0).
    fn fresh_world(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let state = default_state(&config, seed);
        (config, state)
    }

    /// 造舰选装必须**确定**并且**总量可负担**（资源→组件 的确定性链接）。
    #[test]
    fn choose_loadout_is_deterministic_and_affordable() {
        let (config, mut state) = fresh_world(42);
        // Give China (3) a fat rare-mineral stack so it can afford a real loadout.
        if let Some(f) = state.faction_mut("中国") {
            for (r, amt) in [
                ("铀", 200.0), ("金", 200.0), ("氦-3", 200.0),
                ("铂", 200.0), ("氢", 200.0), ("钍", 200.0),
                ("铁", 200.0), ("碳", 200.0), ("硅", 200.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        let slots = config.ship_spec("battleship").slots as usize;
        let a = choose_loadout(&state, &config, "中国".to_string(), "battleship");
        let b = choose_loadout(&state, &config, "中国".to_string(), "battleship");
        assert_eq!(a, b, "loadout must be deterministic");
        assert!(a.len() <= slots, "must not exceed slot cap ({slots})");
        // The whole chosen set must be cumulatively affordable out of the stockpile.
        let mut pool = state.faction("中国").unwrap().resources.clone();
        for c in &a {
            for (r, amt) in &config.component_spec(c).cost {
                assert!(
                    pool.get(r).copied().unwrap_or(0.0) >= *amt,
                    "loadout {c} must be affordable for resource {r}"
                );
                *pool.entry(r.clone()).or_insert(0.0) -= *amt;
            }
        }
        // A resource-rich faction should fill more than a token slot.
        assert!(a.len() >= 2, "rich faction should field a real loadout, got {a:?}");
    }

    /// 拟人指挥官：海军**混编**——一支富有的、近乎全护卫的势力，`choose_next_class` 会被
    /// 「去重加分」拉去建其它舰型（不只堆护卫），形成更像真实海军的混编。
    #[test]
    fn choose_next_class_diversifies_toward_a_mix() {
        let (config, mut state) = fresh_world(42);
        if let Some(f) = state.faction_mut("中国") {
            for (r, amt) in [
                ("铀", 300.0), ("金", 300.0), ("氦-3", 300.0), ("铂", 300.0),
                ("氢", 300.0), ("钍", 300.0), ("铁", 300.0), ("碳", 300.0),
                ("硅", 300.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        // 强制这支势力的现役舰队全部是护卫舰——其余舰型因此「欠份额」，得到去重加分。
        for s in state.ships.iter_mut() {
            if s.faction_id == "中国" {
                s.class = "corvette".to_string();
            }
        }
        let mut rng = Prng::new(7);
        let mut got = BTreeSet::new();
        for _ in 0..60 {
            got.insert(choose_next_class(&state, "中国", &config, &mut rng));
        }
        assert!(
            got.len() >= 3,
            "an all-corvette navy should be pulled into a mix, got {got:?}"
        );
    }

    /// 威胁响应造舰（拟人「战时多造重舰」）：交战中，AI 会比和平时更倾向造重型战斗舰
    /// （攻击力高的舰型加分），而不是只堆轻护卫。
    #[test]
    fn choose_next_class_builds_heavier_navy_at_war() {
        let (config, mut state) = fresh_world(42);
        if let Some(f) = state.faction_mut("中国") {
            for (r, amt) in [
                ("铀", 300.0), ("金", 300.0), ("氦-3", 300.0), ("铂", 300.0),
                ("氢", 300.0), ("钍", 300.0), ("铁", 300.0), ("碳", 300.0),
                ("硅", 300.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        // 舰队全部护卫舰，让去重加分对各舰型一视同仁。
        for s in state.ships.iter_mut() {
            if s.faction_id == "中国" {
                s.class = "corvette".to_string();
            }
        }
        let sample_heavy = |st: &State| -> usize {
            let mut rng = Prng::new(99);
            let mut heavy = 0;
            for _ in 0..240 {
                let c = choose_next_class(st, "中国", &config, &mut rng);
                if c == "battleship" || c == "carrier" {
                    heavy += 1;
                }
            }
            heavy
        };
        let peace = sample_heavy(&state);
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let war = sample_heavy(&state);
        assert!(
            war > peace,
            "at war the AI should build more heavy hulls (war {war} > peace {peace})"
        );
    }

    /// 威胁响应（海军随威胁重构）：交战中，被单一舰型过度统治的势力会把一个船坞重定向到
    /// 战局感知的新舰型（多造重舰），让威胁响应作用于整支舰队而不仅是新建舰厂。
    #[test]
    fn war_retools_over_abundant_shipyard_toward_a_war_class() {
        let (config, mut state) = fresh_world(42);
        let shipyard_types = |st: &State, f: FactionId| -> BTreeSet<(CityId, String)> {
            st.cities
                .iter()
                .filter(|c| c.faction_id == f)
                .flat_map(|c| {
                    c.buildings
                        .iter()
                        .filter(|b| b.is_shipyard() && b.ship_type.is_some())
                        .map(|b| (c.name.clone(), b.ship_type.clone().unwrap()))
                })
                .collect()
        };
        // China (3) 舰队全护卫（过度单一），并让其与 US (1) 交战。
        for s in state.ships.iter_mut() {
            if s.faction_id == "中国" {
                s.class = "corvette".to_string();
            }
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let before = shipyard_types(&state, "中国".to_string());
        let mut rng = Prng::new(7);
        let mut retools = Vec::new();
        retool_shipyards(&mut state, &config, "中国", &mut rng, &mut retools);
        let after = shipyard_types(&state, "中国".to_string());
        assert!(
            after.iter().any(|(_, t)| t != "corvette"),
            "a corvette-dominated wartime fleet should retool a shipyard into a war class; before={before:?} after={after:?}"
        );
        // 改装决策必须**被记下来**（它不发事件，只有这里能留下"什么时候改成什么的"）。
        let rec = retools.iter().find(|r| r.faction == "中国").expect("改装要留一条判定");
        assert_eq!(rec.from, "corvette", "改装前后舰级要对得上：{rec:?}");
        assert_eq!(
            after.iter().find(|(c, _)| *c == rec.city).map(|(_, t)| t.clone()),
            Some(rec.to.clone()),
            "判定里记的新舰级必须就是状态里改成的那个：{rec:?}"
        );
    }

    /// 拟人指挥官：军舰选装要「又能打、又能扛」（…）；战局感知也在此测试。
    #[test]
    fn choose_loadout_is_balanced_and_threat_aware() {
        let (config, mut state) = fresh_world(42);
        if let Some(f) = state.faction_mut("中国") {
            for (r, amt) in [
                ("铀", 300.0), ("金", 300.0), ("氦-3", 300.0), ("铂", 300.0),
                ("氢", 300.0), ("钍", 300.0), ("铁", 300.0), ("碳", 300.0),
                ("硅", 300.0),
            ] {
                *f.resources.entry(r.to_string()).or_insert(0.0) += amt;
            }
        }
        // 和平：一艘巡洋舰（slot≥2）应至少各有一件武器与防御。
        let peace = choose_loadout(&state, &config, "中国".to_string(), "cruiser");
        assert!(
            peace.iter().any(|c| config.component_spec(c).category == "weapon"),
            "a ship should field a weapon (got {peace:?})"
        );
        assert!(
            peace.iter().any(|c| config.component_spec(c).category == "defense"),
            "a ship should field a defense (got {peace:?})"
        );
        // 开战：武器数不应比和平少（战时要火力的偏置）。
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let war = choose_loadout(&state, &config, "中国".to_string(), "cruiser");
        let peace_w = peace.iter().filter(|c| config.component_spec(c).category == "weapon").count();
        let war_w = war.iter().filter(|c| config.component_spec(c).category == "weapon").count();
        assert!(
            war_w >= peace_w,
            "at war the AI should field at least as many weapons (war {war_w} >= peace {peace_w}); war={war:?} peace={peace:?}"
        );
    }
}
