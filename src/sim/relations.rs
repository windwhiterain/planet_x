//! 关系与外交：宣战/停战阈、好感调整、战痕地板、外交回合。

use super::*;

/// The set of unordered faction pairs currently at war (relation ≤ war_threshold).
pub fn war_pairs(state: &State, config: &GameConfig) -> BTreeSet<(FactionId, FactionId)> {
    let mut pairs = BTreeSet::new();
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let (a, b) = (ids[i].clone(), ids[j].clone());
            if hostile(state, config, &a, &b) {
                if a <= b { pairs.insert((a, b)); } else { pairs.insert((b, a)); }
            }
        }
    }
    pairs
}

// --- helpers ----------------------------------------------------------------

pub fn hostile(state: &State, config: &GameConfig, a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    relation(state, a, b) <= config.combat.war_threshold
}

/// 该势力当前是否处于交战状态：与任意其他势力的关系已达到交战阈值。
/// 用于「造舰按威胁响应」——战时倾向多造战争机器，和平时倾向多造殖民/经济舰。
pub fn faction_at_war(state: &State, config: &GameConfig, fid: &str) -> bool {
    state.factions.iter().any(|o| o.name != fid && hostile(state, config, fid, &o.name))
}

/// **「记恨」读者**——窗口层（[`State::notables`]）当前的唯一消费者。
///
/// 回头看 `war_scar_rounds` 回合内**最近一次** `WarStarted{a,b}`，返回它此刻还压着的关系
/// **地板**（`None` = 窗口里没有这道疤）。这就是 [`Salience::Notable`] 判据的范例落地：开战
/// 之后「相当一段时间两国互相记恨」，所以后面的外交计算需要回看**一定窗口**——窗口之外的那场
/// 战争不再影响任何计算，因此**不必**长存。
///
/// 地板从 `war_scar_relation`（负值）线性衰减到 0。新鲜时它低于 `war_threshold`，于是
/// **刚开战的对手不可能当回合就言和**（战争不会一闪即灭，正是此前 `war_started`/`war_ended`
/// 反复闪烁的成因之一）；随着疤变淡，地板抬过阈值，和平重新变得可能——「记恨，但会淡」。
///
/// `war_scar_rounds == 0` 或 `war_scar_relation >= 0` 时本机制关闭（返回 `None`）。
pub fn war_scar_floor(state: &State, config: &GameConfig, a: &str, b: &str) -> Option<f64> {
    let span = config.diplomacy.war_scar_rounds;
    let base = config.diplomacy.war_scar_relation;
    if span == 0 || base >= 0.0 {
        return None;
    }
    // 窗口本身由 `Notables::trim` 保证；这里再按 `span` 判一次，使 `war_scar_rounds` 可以短于
    // `history.notable_window`（否则读者会看见自己不该看的老疤）。
    let started = state
        .notables
        .entries
        .iter()
        .filter(|e| e.round + span > state.round)
        .filter_map(|e| match &e.event {
            GameEvent::WarStarted { a: x, b: y }
                if (x == a && y == b) || (x == b && y == a) =>
            {
                Some(e.round)
            }
            _ => None,
        })
        .max()?;
    let age = state.round.saturating_sub(started);
    Some(base * (1.0 - (age as f64) / (span as f64)))
}

pub fn relation(state: &State, a: &str, b: &str) -> f64 {
    state
        .faction(a)
        .and_then(|f| f.relations.get(b).copied())
        .unwrap_or(0.0)
}

/// 关系增减（开火/夺城的 delta、剧情的关系效果）。**写入前必须过战争疤痕地板**——
/// 见 [`set_relation_sym`] 的说明：关系有多个写入者，任何一个绕过地板，地板就不成立。
pub fn adjust_relation(state: &mut State, config: &GameConfig, a: &str, b: &str, delta: f64) {
    if a == b {
        return;
    }
    for (x, y) in [(a, b), (b, a)] {
        // 地板要在拿到 `&mut` 之前算好（借用的先后顺序）。
        let floor = war_scar_floor(state, config, x, y);
        if let Some(f) = state.faction_mut(x) {
            let mut v = f.relations.get(y).copied().unwrap_or(0.0) + delta;
            if let Some(floor) = floor {
                v = v.min(floor);
            }
            f.relations.insert(y.to_string(), v);
        }
    }
}

/// Dynamic international-relations step.
///
/// Each unordered faction pair independently:
///   * drifts toward its **resting affinity** (bloc formation), derived from the
///     two factions' `alignment`. Aggressive factions close in on a hostile
///     affinity faster, so ideologically-distant powers escalate to war on their
///     own (a build-up phase) and allies cohere.
///   * if already at war and the pair did **not** fight this round, winds down
///     toward `ceasefire_relation` (war fatigue) — so wars end once the fighting
///     stops, and can later re-escalate.
///   * gets a little `noise`, so relations fluctuate and cross the threshold
///     irregularly rather than settling.
///
/// Hostile acts (`attack_delta` / `capture_delta` applied in [`adjust_relation`])
/// still push relations down during combat, which is what keeps an active war hot.
pub fn step_diplomacy(state: &mut State, config: &GameConfig, rng: &mut Prng) {
    let d = &config.diplomacy;
    let band = 2.0;

    // Which (unordered) faction pairs engaged in hostilities this round, so war
    // fatigue does not cancel out the combat-driven relation drops while a war
    // is actually being fought.
    let mut fought: BTreeSet<(FactionId, FactionId)> = BTreeSet::new();
    let mut note_pair = |a: Option<FactionId>, b: Option<FactionId>| {
        if let (Some(a), Some(b)) = (a, b) {
            if a != b {
                if a <= b { fought.insert((a, b)); } else { fought.insert((b, a)); }
            }
        }
    };
    for e in &state.events {
        match e {
            GameEvent::Attack { attacker, target, .. } => {
                note_pair(
                    state.ship(attacker).map(|s| s.faction_id.clone()),
                    state.ship(target).map(|s| s.faction_id.clone()),
                );
            }
            GameEvent::Siege { attacker, city, .. } => {
                note_pair(
                    state.ship(attacker).map(|s| s.faction_id.clone()),
                    state.city(city).map(|c| c.faction_id.clone()),
                );
            }
            _ => {}
        }
    }

    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let (a, b) = (ids[i].clone(), ids[j].clone());
            let (align_a, align_b, aggr, ideo_a, ideo_b) = {
                let fa = state.factions.iter().find(|f| f.name == a).expect("faction a gone");
                let fb = state.factions.iter().find(|f| f.name == b).expect("faction b gone");
                (fa.alignment, fb.alignment, fa.aggression.max(fb.aggression), fa.ideology, fb.ideology)
            };
            let mut rel = relation(state, &a, &b);
            let mut aff = d.affinity_floor + d.affinity_span * (1.0 - (align_a - align_b).abs().min(band) / band);
            // 思潮相似度（可变化当代思潮）：相似 → 亲和上移，对立 → 亲和下移（对称修正）。
            // 与 alignment（历史静态阵营亲缘）叠加，构成「历史静态 + 思潮可变」双因子。
            if d.ideology_affinity_span != 0.0 {
                let sim = ideology_similarity(&ideo_a, &ideo_b);
                aff += d.ideology_affinity_span * (2.0 * sim - 1.0);
            }
            let at_war = rel <= config.combat.war_threshold;
            let pair = if a <= b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) };
            let clashing = fought.contains(&pair);

            if at_war && !clashing {
                // War fatigue: cool the conflict toward ceasefire once the guns
                // fall silent, so wars end rather than grind forever.
                rel += d.war_fatigue * (d.ceasefire_relation - rel);
            } else {
                // Bloc drift toward resting affinity; aggressive powers close a
                // hostile gap faster (they escalate, they do not befriend rivals).
                let rate = if aff < 0.0 { 1.0 + aggr } else { 1.0 };
                rel += d.drift_rate * rate * (aff - rel);
            }

            // Little random fluctuation so relations wobble and cross thresholds.
            rel += rng.range_f64(-d.noise, d.noise);

            // 写入走**唯一漏斗**：钳位 + 战争疤痕地板（记恨）都在里面，所以随机扰动压不过地板。
            // 关系有多个写入者（这里的外交漂移、攻击/夺城 delta、合纵的相互靠拢、剧情效果），
            // 地板必须对**每一个**成立——否则「刚开战的对手不可能当回合言和」会被别人推翻。
            set_relation_sym(state, a.clone(), b.clone(), rel, config);
        }
    }
}

// --- balance of power (合纵连横 / 弱者联盟对抗霸权) ------------------------------

/// **关系写入的唯一漏斗**：钳位 + 战争疤痕地板（记恨）。写入双方，保持对称。
///
/// 为什么必须漏斗化：疤痕是一条**地板**（`rel.min(floor)`），而关系有多个写入者——外交漂移、
/// 倒戈/夺城的 `capture_delta`、**合纵的弱者相互靠拢**、剧情的 relations 效果……只要有一个
/// 写入者绕过地板，它就**不再是地板**。实测证据：`step_balance_of_power` 的「合纵」在
/// `step_diplomacy` **之后**跑，把两个正彼此交战的弱者拉近，于是刚开战的一对可以在 **6 回合**
/// 内言和，而地板承诺的是至少 9 回合。经过本漏斗后，「忘记套用地板」在结构上不可能——
/// 与 [`ev`]/[`kill_ship`] 那套 single-writer 纪律同源。
pub fn set_relation_sym(state: &mut State, a: FactionId, b: FactionId, v: f64, config: &GameConfig) {
    let floor = war_scar_floor(state, config, &a, &b);
    let mut v = v.clamp(config.diplomacy.hostility_floor, config.diplomacy.friendship_ceiling);
    if let Some(floor) = floor {
        v = v.min(floor);
    }
    for f in state.factions.iter_mut().filter(|f| f.name == a || f.name == b) {
        let other = if f.name == a { b.clone() } else { a.clone() };
        f.relations.insert(other, v);
    }
}
