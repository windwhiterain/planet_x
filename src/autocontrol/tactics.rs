//! 战术选目标（AI 决定打谁）+ 单舰 AI 回合。
//!
//! 这里装「一艘舰/一支舰队在战斗中具体怎么想」：每一发武器选哪个目标（克制/距离/
//! 行为风格）、要不要自保撤退、要不要护航/独狼、要不要轰炸/殖民——纯决策。所需的引擎
//! 原语（移动/开火/轰炸/殖民/距离/敌我判定）一律向 [`crate::sim`] 借。

use crate::model::*;
use crate::prng::Prng;
use crate::sim;
use std::collections::BTreeMap;

// 统一的基本权重：**距离 + 克制 + per-武器随机扰动**，所有自动逻辑共用。克制权重
// > 距离权重（把火力用在打得动的目标上，比贴着打更划算）；扰动是小量，让每件武器
// 各有一点点稳定的偏好（舰队火力不整齐划一）。
const W_DIST: f64 = 1.0;
const W_CTR: f64 = 1.5;
/// 理智<->热血层权重：把「威慑对比」折算进基本权重的强度。
const W_TEMPER: f64 = 0.35;
/// per-武器确定性噪声幅度 (±)。
const NOISE_AMP: f64 = 0.06;
/// 结盟集火(coalition focus)加成。
const FOCUS_BONUS: f64 = 0.5;

/// 武器克制评分（0..1）：**这一件**武器对目标的有效杀伤效率——导弹被目标点防御拦截而
/// 大打折扣（导弹 vs 点防），动能对高护盾目标较弱（动能 vs 护盾）。AI 据此挑「自己能有效
/// 杀伤」的目标，而不是把导弹浪费在全套点防御的堡垒上。确定性。
fn weapon_counter(weapon: &Weapon, config: &GameConfig, target: &Ship) -> f64 {
    let panel = ship_panel(config, target);
    let wd = weapon.damage.max(1e-9);
    let shield_share = panel.shield_max / (panel.shield_max + panel.hull_max).max(1e-9);
    let mut score = 1.0;
    match weapon.kind {
        WEAPON_MISSILE => {
            let intercept_frac = if panel.intercept > 0.0 {
                (panel.intercept / (panel.intercept + wd)).min(0.8)
            } else {
                0.0
            };
            score -= intercept_frac;
        }
        WEAPON_KINETIC => {
            score -= shield_share * 0.4;
        }
        _ => {}
    }
    score.clamp(0.0, 1.0)
}

/// 距离分值（0..1）：越近越高——目标进入本武器射程时是正向奖励，贴脸趋近 1。
fn dist_score(d: f64, range: f64) -> f64 {
    if range > 1e-9 {
        (1.0 - (d / range).clamp(0.0, 1.0)).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// per-武器确定性随机扰动：由（武器 seed、目标名）哈希出 ±`NOISE_AMP` 的小偏置，稳定
/// 可复现。每件武器对每个目标各有一点点不同的偏好（独立索敌单位的体现）。
fn weapon_noise(seed: u64, target: &str) -> f64 {
    let mut h = seed;
    for b in target.as_bytes() {
        h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
    }
    let u01 = (h >> 11) as f64 / (1u64 << 53) as f64;
    (u01 - 0.5) * 2.0 * NOISE_AMP
}

/// 基本权重（所有自动逻辑通用）：距离 + 克制 + per-武器随机扰动。
fn basic_weight(d: f64, weapon: &Weapon, target: &Ship, config: &GameConfig) -> f64 {
    W_DIST * dist_score(d, weapon.range) + W_CTR * weapon_counter(weapon, config, target) + weapon_noise(weapon.seed, &target.name)
}

/// 行为风格层（在基本权重之上）：理智<->热血按「威慑对比」偏置、火力分配按「攻击历史
/// 新鲜度 × 武器 fire_spread」把最近打过的目标权重修正。`hist` 是本舰的攻击历史（可能带
/// 本回合已打的本地更新）。
fn doctrine_weight(
    state: &State,
    config: &GameConfig,
    attacker: &Ship,
    weapon: &Weapon,
    target: &Ship,
    hist: &BTreeMap<ShipId, f64>,
    d: f64,
) -> f64 {
    let mut s = basic_weight(d, weapon, target, config);
    // 理智<->热血：`temper<0` 欺软怕硬(打威慑低于自己的)，`>0` 飞蛾扑火(打威慑高于自己的)。
    // 用「威慑比」的对数来量化敌我差距：即使本舰威慑远大于目标，弱目标之间仍能分清高下
    // （避免 `(my-tg)/(my+tg)` 在 my≫tg 时把所有弱目标压成 ~1、失去区分度）。
    //
    // 读的是**有效风格**（`State::ship_doctrine`：叶 → 舰队默认 → 舰上记录值）——AI 只读它，
    // 从不写它，所以玩家钉住的风格不会被 AI 覆盖。
    let temper = state.ship_doctrine(attacker.name.clone()).temper;
    if temper.abs() > 1e-9 {
        let my_det = sim::deterrence(state, config, &attacker.name);
        let tg_det = sim::deterrence(state, config, &target.name);
        let rel = ((my_det + 1.0) / (tg_det + 1.0)).ln().clamp(-4.0, 4.0);
        s += W_TEMPER * -temper * rel;
    }
    // 火力分配（在基本权重之上）：`fire_spread>0` 越近打过的权重越低(雨露均沾)，`<0` 越高
    // (死磕补刀)。
    let recency = hist.get(&target.name).copied().unwrap_or(0.0);
    if weapon.fire_spread.abs() > 1e-9 && recency > 1e-9 {
        s *= 1.0 - weapon.fire_spread * recency;
    }
    s
}

/// 一件武器在**射程内**挑得分最高的活敌舰（按基本权重 + 行为风格层）。
fn best_target_in_range(state: &State, config: &GameConfig, attacker: &Ship, weapon: &Weapon, hist: &BTreeMap<ShipId, f64>) -> Option<ShipId> {
    let mut best: Option<(f64, ShipId)> = None;
    for s in &state.ships {
        if s.hull <= 0.0 || !sim::hostile(state, config, &attacker.faction_id, &s.faction_id) {
            continue;
        }
        let d = sim::dist(attacker.position, s.position);
        if d > weapon.range {
            continue;
        }
        let score = doctrine_weight(state, config, attacker, weapon, s, hist, d);
        if best.as_ref().map_or(true, |&(bs, _)| score > bs) {
            best = Some((score, s.name.clone()));
        }
    }
    best.map(|(_, n)| n)
}

/// 一艘舰对目标 `s` 的**综合得分**（追击/选主目标用）：取各武器行为风格得分的最大，叠加
/// 结盟集火加成。用于判断「该追谁」/撤退判定。
fn target_ship_score(state: &State, config: &GameConfig, attacker: &Ship, s: &Ship, d: f64) -> f64 {
    let weapons = ship_weapons(config, attacker);
    let hist = attacker.attack_hist.clone();
    let mut score = 0.0f64;
    for w in &weapons {
        score = score.max(doctrine_weight(state, config, attacker, w, s, &hist, d));
    }
    if weapons.is_empty() {
        score = 10.0 / (d + 1.0);
    }
    score
}

pub(crate) fn nearest_enemy_ship(state: &State, config: &GameConfig, owner: &str, pos: [f64; 2], range: f64, focus: Option<FactionId>, attacker_id: &str) -> Option<ShipId> {
    let Some(attacker) = state.ship(attacker_id) else { return None };
    let mut best: Option<(f64, ShipId)> = None; // (score, name)
    for s in &state.ships {
        if s.hull <= 0.0 || !sim::hostile(state, config, owner, &s.faction_id) {
            continue;
        }
        let d = sim::dist(pos, s.position);
        if d > range {
            continue;
        }
        let mut score = target_ship_score(state, config, attacker, s, d);
        if focus.as_ref() == Some(&s.faction_id) {
            score += FOCUS_BONUS;
        }
        if best.as_ref().map_or(true, |&(bs, _)| score > bs) {
            best = Some((score, s.name.clone()));
        }
    }
    best.map(|(_, n)| n)
}

/// 本势力的旗舰（高价值舰种）：第一艘航母（按名字最小），否则 None。用于护航——AI 派
/// 闲着的舰护卫它，防止高价值舰被轻易打掉。
fn fleet_flag(state: &State, fid: &str) -> Option<ShipId> {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0 && s.class == "carrier")
        .min_by_key(|s| s.name.clone())
        .map(|s| s.name.clone())
}

/// 本舰本回合的开火计划：每件武器的每一发都**独立索敌**——按行为风格层挑一个射程内的活
/// 敌舰。攻击历史用本地副本随时更新（打过的刷新到 1），使「雨露均沾」武器在**同回合内**
/// 就能把多发摊到不同目标。确定性。
pub(crate) fn build_fire_plan(state: &State, config: &GameConfig, ship_id: &str) -> Vec<(usize, ShipId)> {
    let Some(ship) = state.ship(ship_id) else { return Vec::new() };
    let weapons = ship_weapons(config, ship);
    if weapons.is_empty() {
        return Vec::new();
    }
    let mut hist = ship.attack_hist.clone();
    let mut plan = Vec::new();
    for (i, w) in weapons.iter().enumerate() {
        let shots = (w.fire_rate.round()).max(1.0) as usize;
        for _ in 0..shots {
            if let Some(t) = best_target_in_range(state, config, ship, w, &hist) {
                plan.push((i, t.clone()));
                hist.insert(t, 1.0); // 本回合内后续发能看到这次的「新鲜攻击」。
            }
        }
    }
    plan
}

/// 就近的敌对城（在围城射程内）：进攻自动化（轰炸不需要行为）的目标候选。
fn nearest_hostile_city_in_siege_range(state: &State, config: &GameConfig, owner: &str, pos: [f64; 2]) -> Option<CityId> {
    let mut best: Option<(f64, CityId)> = None;
    for c in &state.cities {
        if c.razed || !sim::hostile(state, config, owner, &c.faction_id) {
            continue;
        }
        let p = sim::city_position(state, &c.name);
        let d = sim::dist(pos, p);
        if d > config.combat.siege_range {
            continue;
        }
        let score = -d; // 就近。
        if best.as_ref().map_or(true, |&(bs, _)| score > bs) {
            best = Some((score, c.name.clone()));
        }
    }
    best.map(|(_, c)| c)
}

/// 自动轰炸：若 `owner` 的敌对城进入 `pos` 的围城射程，就轰炸最近的那座；返回是否轰炸。
/// 无行为需求——凡在围城射程内的敌对城即触发。
fn auto_bombard(state: &mut State, config: &GameConfig, ship_id: &str, owner: &str, pos: [f64; 2]) -> bool {
    if let Some(city) = nearest_hostile_city_in_siege_range(state, config, owner, pos) {
        sim::bombard_city(state, config, ship_id, &city);
        true
    } else {
        false
    }
}

/// 自动战斗（攻击/轰炸都不需要行为）：射程内有敌对舰就按统一基本权重逐发索敌开火；
/// 否则若有敌对城进入围城射程则就地轰炸。对玩家与 AI 共用。不修改该舰的指令（行为保留）。
pub(crate) fn auto_combat(state: &mut State, config: &GameConfig, ship_id: &str, owner: &str) {
    let pos = state.ship(ship_id).map(|s| s.position).unwrap_or([0.0, 0.0]);
    let plan = build_fire_plan(state, config, ship_id);
    if !plan.is_empty() {
        sim::fire(state, config, ship_id, &plan);
        return;
    }
    auto_bombard(state, config, ship_id, owner, pos);
}

/// 本舰的**软移动目的地**（风筝<->贴脸姿态）：在附近有敌舰时按 `kiting` 重新定距离——
/// `kiting<0`(风筝)把舰钉在**最远武器射程**、敌近则拉开；`kiting>0`(贴脸)把舰**压近**到最小
/// 交战距离。`kiting=0`(基线)无调整。返回 `None` 表示不调整（调用方照用其软目标 `base`）。
/// Move/Follow/Dock/Idle 都是**软目标**——即使玩家也不能硬控制它：附近有敌舰时此姿态自动
/// 生效，对玩家与 AI 一视同仁。引擎结算不读它。
pub(crate) fn kiting_dest(state: &State, config: &GameConfig, ship_id: &str) -> Option<[f64; 2]> {
    let Some(ship) = state.ship(ship_id) else { return None };
    // 有效姿态（叶 → 舰队默认 → 舰上记录值）：AI 只读，玩家写的叶优先。
    let kiting = state.ship_kiting(ship_id.to_string());
    if kiting.abs() < 1e-9 {
        return None; // 基线：无软调整。
    }
    let owner = ship.faction_id.clone();
    let pos = ship.position;
    let range = ship_panel(config, ship).attack_range;
    // 感知半径：只对**附近**敌舰生效（武器射程 + 一点缓冲），不越全图。
    let awareness = range + 0.5;
    let Some(enemy) = nearest_enemy_ship(state, config, &owner, pos, awareness, None, ship_id) else {
        return None;
    };
    let epos = state.ship(&enemy).map(|s| s.position).unwrap_or(pos);
    let d = sim::dist(pos, epos);
    if d < 1e-9 {
        return None;
    }
    // unit: 从敌舰指向本舰的单位向量——把目的地放在「本舰当前这一侧、距敌 desired_r」处。
    let unit = [(pos[0] - epos[0]) / d, (pos[1] - epos[1]) / d];
    let desired_r = if kiting < 0.0 {
        range // 风筝：保持在最远武器射程。
    } else {
        config.combat.min_engage_range.max(0.0) // 贴脸：压近到最小交战距离。
    };
    Some([epos[0] + unit[0] * desired_r, epos[1] + unit[1] * desired_r])
}

fn pick_target(state: &State, config: &GameConfig, owner: &str, pos: [f64; 2], _rng: &mut Prng, focus: Option<FactionId>, attacker_id: &str) -> Option<ShipBehavior> {
    let Some(attacker) = state.ship(attacker_id) else { return None };
    // 对舰：在追击半径内选综合得分最高的敌舰（基本权重 + 行为风格层 + 集火加成）。
    let mut best_ship: Option<(f64, ShipId)> = None;
    for s in &state.ships {
        if s.hull <= 0.0 || !sim::hostile(state, config, owner, &s.faction_id) {
            continue;
        }
        let d = sim::dist(pos, s.position);
        if config.combat.pursuit_range > 0.0 && d > config.combat.pursuit_range {
            continue;
        }
        let mut score = target_ship_score(state, config, attacker, s, d);
        if focus.as_ref() == Some(&s.faction_id) {
            score += FOCUS_BONUS;
        }
        if best_ship.as_ref().map_or(true, |&(bs, _)| score > bs) {
            best_ship = Some((score, s.name.clone()));
        }
    }
    if let Some((_, s)) = best_ship {
        return Some(ShipBehavior::Follow { ship: s });
    }
    // 城市：无追得上的敌舰时，驶向最近的敌对城（到围城射程内便自动轰炸）。
    let mut best_city: Option<(f64, CityId)> = None;
    for c in &state.cities {
        if c.razed || !sim::hostile(state, config, owner, &c.faction_id) {
            continue;
        }
        let p = sim::city_position(state, &c.name);
        let d = sim::dist(pos, p);
        let score = -d;
        if best_city.as_ref().map_or(true, |&(bs, _)| score > bs) {
            best_city = Some((score, c.name.clone()));
        }
    }
    if let Some((_, c)) = best_city {
        return Some(ShipBehavior::DockCity { city: c });
    }
    // 无仗可打：就近（重建）殖民一处被夷平的定居点。
    for c in &state.cities {
        if c.razed {
            return Some(ShipBehavior::Colonize { body: c.body_id.clone() });
        }
    }
    None
}

fn resolve_target(state: &mut State, config: &GameConfig, ship_id: &str, owner: &str, pos: [f64; 2], rng: &mut Prng, focus: Option<FactionId>) -> Option<ShipBehavior> {
    let cur = state.ship_behavior(ship_id.to_string());
    // 保持一个仍有效的跟随/围城行为，避免指挥官每回合在目标间抖动。
    if let Some(b) = cur {
        if matches!(b, ShipBehavior::Follow { .. } | ShipBehavior::DockCity { .. })
            && sim::behavior_is_valid(state, config, b.clone(), owner)
        {
            return Some(b);
        }
    }
    let picked = pick_target(state, config, &owner, pos, rng, focus, ship_id);
    let mut behavior = picked.unwrap_or(ShipBehavior::Idle);
    // 护航/独狼：交战时闲着、且**不是独狼**（`lone_wolf < 0`）的舰，就近跟随本势力旗舰
    // （航母）。独狼（`lone_wolf` 高）空闲时保持自由接战（`pick_target` 已挑最近的敌舰）。
    if matches!(behavior, ShipBehavior::Idle)
        && config.combat.escort_range > 0.0
        && sim::faction_at_war(state, config, owner)
    {
        let lone_wolf = state.ship_doctrine(ship_id.to_string()).lone_wolf;
        if lone_wolf < -0.01 {
            if let Some(flag_id) = fleet_flag(state, &owner) {
                if flag_id != ship_id {
                    let fpos = state.ship(&flag_id).map(|s| s.position).unwrap_or(pos);
                    if sim::dist(pos, fpos) <= config.combat.escort_range {
                        behavior = ShipBehavior::Follow { ship: flag_id };
                    }
                }
            }
        }
    }
    if let Some(c) = state.control_mut(owner.to_string()) {
        c.ship_orders.insert(ship_id.to_string(), Control::inherit(behavior.clone()));
    }
    if matches!(behavior, ShipBehavior::Idle) {
        None
    } else {
        Some(behavior)
    }
}

/// 风筝<->贴脸影响自保撤退阈值：风筝(negative)更早撤(阈值更高)，贴脸(positive)打得更久
/// 再撤(阈值更低)。同一姿态也驱动 `kiting_dest` 的软移动（敌近则拉开/压近）。
fn effective_retreat_hull(config: &GameConfig, kiting: f64) -> f64 {
    (config.combat.retreat_hull + 0.14 * -kiting).clamp(0.02, 0.9)
}

/// 一艘 AI 舰在本回合的行为：接战（逐发独立索敌、火力分配）、自保撤退（激进更晚撤）、
/// 护航/独狼、殖民、轰炸——并把它实际执行的指令写回可控状态（`Control::inherit`），使
/// 逐回合 diff 能反映系统真正做了什么。执行所需的引擎原语（移动/开火/轰炸/殖民）借自
/// [`crate::sim`]。
pub(crate) fn ai_ship_turn(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    ship_id: &str,
    focus_of: &BTreeMap<FactionId, Option<FactionId>>,
    next_building_id: &mut BuildingId,
) {
    let Some(ship) = state.ship(ship_id) else { return };
    if ship.hull <= 0.0 {
        return;
    }
    let owner = ship.faction_id.clone();
    let class = ship.class.clone();
    let pos = ship.position;
    let range = ship_panel(config, ship).attack_range;
    let my_hull = ship.hull;
    let my_hull_max = ship.hull_max;
    let focus = focus_of.get(&owner).cloned().flatten();
    // 有效姿态（叶 → 舰队默认 → 记录值）：撤退阈值也跟着它走。
    let kiting = state.ship_kiting(ship_id.to_string());

    let tgt = nearest_enemy_ship(state, config, &owner, pos, range, focus.clone(), ship_id);

    // 自保撤退（拟人的「别送死」，激进更晚撤）：舰已受重创、敌在本舰射程内、且离首都有
    // 一定距离时，后撤回首都/本土修整充能。让战争有「打残→撤→养好→再来」的损耗循环。
    if let Some(target) = tgt {
        let retreat_hull = effective_retreat_hull(config, kiting);
        if my_hull / my_hull_max.max(1e-9) < retreat_hull {
            let cap_body = state.capital_body(&owner);
            let cap_pos = state.body_position(&cap_body);
            if sim::dist(pos, cap_pos) > config.combat.retreat_min_dist {
                if let Some(c) = state.control_mut(owner.clone()) {
                    c.ship_orders.insert(ship_id.to_string(), Control::inherit(ShipBehavior::Move { position: cap_pos }));
                }
                sim::ev(state, GameEvent::Withdraw { ship: ship_id.to_string(), to_body: cap_body });
                sim::move_toward(state, config, ship_id, &class, cap_pos);
                return;
            }
        }
        // 接战：每件武器逐发独立索敌（火力分配 / 克制 / 理智热血都作用于目标选择）。
        let plan = build_fire_plan(state, config, ship_id);
        if !plan.is_empty() {
            if let Some(c) = state.control_mut(owner.clone()) {
                c.ship_orders.insert(ship_id.to_string(), Control::inherit(ShipBehavior::Follow { ship: target.clone() }));
            }
            sim::fire(state, config, ship_id, &plan);
        }
        return;
    }

    let Some(behavior) = resolve_target(state, config, ship_id, &owner, pos, rng, focus.clone()) else {
        return;
    };

    // 殖民：到达定居点天体即刻建城（优先于自动接战/轰炸）。
    if let ShipBehavior::Colonize { body } = &behavior {
        let bpos = state.body_position(body);
        if sim::dist(pos, bpos) <= config.combat.arrival_eps {
            sim::colonize(state, config, rng, ship_id, body, next_building_id);
            return;
        }
    }
    // 自动轰炸：原地附近若有敌对城在围城射程内，先轰炸（轰炸不需要行为）。
    if auto_bombard(state, config, ship_id, &owner, pos) {
        return;
    }

    let base = sim::behavior_dest(state, &behavior);
    sim::move_toward(state, config, ship_id, &class, kiting_dest(state, config, ship_id).unwrap_or(base));

    // 移动后：自动接战（攻击不要行为）→ 自动轰炸 → 殖民落地。
    if let Some(ship) = state.ship(ship_id) {
        let np = ship.position;
        if let Some(target) = nearest_enemy_ship(state, config, &owner, np, range, focus.clone(), ship_id) {
            let plan = build_fire_plan(state, config, ship_id);
            if !plan.is_empty() {
                if let Some(c) = state.control_mut(owner.clone()) {
                    c.ship_orders.insert(ship_id.to_string(), Control::inherit(ShipBehavior::Follow { ship: target.clone() }));
                }
                sim::fire(state, config, ship_id, &plan);
                return;
            }
        }
        if auto_bombard(state, config, ship_id, &owner, np) {
            return;
        }
        if let ShipBehavior::Colonize { body } = &behavior {
            let bpos = state.body_position(body);
            if sim::dist(np, bpos) <= config.combat.arrival_eps {
                sim::colonize(state, config, rng, ship_id, body, next_building_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::model::GameEvent;
    use crate::prng::Prng;
    use crate::sim::{advance, dist};
    use crate::world::default_state;

    /// Build the config + a fresh deterministic world (round 0).
    fn fresh_world(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let state = default_state(&config, seed);
        (config, state)
    }

    /// 护航目标：旗舰 = 本势力最小的航母（高价值舰种），用于让闲着的舰护卫它。
    #[test]
    fn fleet_flag_is_the_factions_first_carrier() {
        let (_config, mut state) = fresh_world(42);
        // China (3) ship 2 设为航母 → 成为旗舰。
        let ship2 = state.ships[2].name.clone();
        if let Some(s) = state.ship_mut(&ship2) {
            s.class = "carrier".to_string();
        }
        assert_eq!(fleet_flag(&state, "中国"), Some(ship2));
        // 没有航母 → 无旗舰（无护航）。
    }

    /// 拟人的「不追远敌」（驻守而非过度延伸）：超出追击半径的敌对舰不应被选为追击目标。
    #[test]
    fn ships_do_not_chase_enemies_beyond_pursuit_range() {
        let (config, mut state) = fresh_world(42);
        // 中国 ship 0 在 [40,40]；把 US 的 ship 5 放到远处（远超 pursuit_range 12）。
        let ship0 = state.ships[0].name.clone();
        let ship5 = state.ships[5].name.clone();
        if let Some(s) = state.ship_mut(&ship5) {
            s.position = [140.0, 40.0];
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let picked = pick_target(&state, &config, "中国", [40.0, 40.0], &mut Prng::new(1), None, &ship0);
        // 远处那艘敌舰不应被选中（超出追击半径）；可能选中更近的目标或城市/空。
        let chased = matches!(picked, Some(ShipBehavior::Follow { ship: ref s }) if *s == ship5);
        assert!(
            !chased,
            "a hostile beyond pursuit_range should not be chased; got {picked:?}"
        );
    }

    /// 风筝<->贴脸（kiting）软移动：附近有敌舰时，`kiting<0` 的舰被推到「最远武器射程」处
    /// （敌近则拉开），`kiting>0` 的舰压近到目标；`kiting=0`（基线）不调整（None）。
    #[test]
    fn kiting_repositions_relative_to_nearby_enemy() {
        let (config, mut state) = fresh_world(42);
        let ship0 = state.ships[0].name.clone();
        let ship3 = state.ships[3].name.clone();
        let ship4 = state.ships[4].name.clone();
        let ship5 = state.ships[5].name.clone();
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = [0.0, 0.0];
            s.kiting = -1.0; // 风筝
        }
        if let Some(e) = state.ship_mut(&ship3) {
            e.position = [0.3, 0.0];
        }
        // 其余美国舰挪远，确保最近敌舰就是 ship3。
        if let Some(s) = state.ship_mut(&ship4) {
            s.position = [50.0, 50.0];
        }
        if let Some(s) = state.ship_mut(&ship5) {
            s.position = [50.0, 50.0];
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let range = crate::model::ship_panel(&config, state.ship(&ship0).unwrap()).attack_range;
        let en = state.ship(&ship3).unwrap().position;
        // 风筝：目的地的敌我距离应拉到「最远武器射程」处（敌更近则被推开）。
        let kite = kiting_dest(&state, &config, &ship0).expect("kite ship has a dest");
        assert!(
            dist(kite, en) >= range - 1e-9,
            "kite ship should hold at weapon range; dest={kite:?} enemy={en:?} range={range}"
        );
        // 贴脸：压近到目标。
        if let Some(s) = state.ship_mut(&ship0) {
            s.kiting = 1.0;
        }
        let close = kiting_dest(&state, &config, &ship0).expect("face-hug ship has a dest");
        let d_now = dist([0.0, 0.0], en);
        let d_close = dist(close, en);
        assert!(
            d_close <= d_now,
            "face-hug ship should close in; dest={close:?} d_now={d_now} d_close={d_close}"
        );
        // 基线：kiting=0 不调整。
        if let Some(s) = state.ship_mut(&ship0) {
            s.kiting = 0.0;
        }
        assert!(kiting_dest(&state, &config, &ship0).is_none(), "baseline kiting must be None");
    }

    /// 火力分配（雨露均沾）：一件 `fire_spread>0`、`fire_rate>1` 的武器，会把本回合的多发
    /// 摊给**多个**目标——按攻击历史新鲜度，刚打过的目标在下一次选择时权重被降低。
    #[test]
    fn spread_weapon_distributes_fire_across_targets() {
        let (mut config, mut state) = fresh_world(42);
        // 让导弹变成「雨露均沾 + 两连发」，攻击舰装它、站在 [40,40]。
        if let Some(c) = config.components.get_mut("missile") {
            c.fire_rate = 2.0;
            c.fire_spread = 1.0;
        }
        let ship0 = state.ships[0].name.clone();
        let ship3 = state.ships[3].name.clone();
        let ship5 = state.ships[5].name.clone();
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = [40.0, 40.0];
            s.components = vec!["missile".to_string()];
        }
        if let Some(s) = state.ship_mut(&ship3) {
            s.position = [40.1, 40.0];
        }
        if let Some(s) = state.ship_mut(&ship5) {
            s.position = [40.2, 40.0];
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let plan = build_fire_plan(&state, &config, &ship0);
        assert_eq!(plan.len(), 2, "a fire_rate=2 weapon should fire 2 shots, got {plan:?}");
        let first = plan[0].1.clone();
        let second = plan[1].1.clone();
        assert!(
            first != second,
            "a 雨露均沾 (fire_spread>0) weapon should spread its 2 shots across 2 targets, got {plan:?}"
        );
    }

    /// 理智<->热血（威慑对比）：一个热血(temper>0)的舰应倾向攻击**威慑高于自己**的目标
    /// （飞蛾扑火），而理智(temper<0)应倾向攻击威慑低于自己的目标（欺软怕硬）。
    #[test]
    fn temper_biases_toward_weaker_or_stronger_deterrence() {
        let (mut config, mut state) = fresh_world(42);
        // 让导弹射程极大（能同时看到两处**分离**的敌群），并把两群目标放到不同簇（相隔
        // 远超 deterrence_radius），使它们的「舰队威慑」真正不同。
        if let Some(c) = config.components.get_mut("missile") {
            c.range = 200.0;
        }
        let ship0 = state.ships[0].name.clone();
        let ship1 = state.ships[1].name.clone();
        let ship3 = state.ships[3].name.clone();
        let ship4 = state.ships[4].name.clone();
        let ship5 = state.ships[5].name.clone();
        // 攻击舰 ship0 装导弹、站在 [40,40]；把中国队其它舰移远以免污染攻击方威慑。
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = [40.0, 40.0];
            s.components = vec!["missile".to_string()];
        }
        if let Some(s) = state.ship_mut(&ship1) {
            s.position = [600.0, 600.0];
        }
        // 弱目标 ship3：孤立、空载（威慑低）。强目标 ship5：重装（威慑高）。
        // 两簇相隔 ~120 AU（远超 deterrence_radius 8），舰队的威慑互不叠加。
        if let Some(s) = state.ship_mut(&ship3) {
            s.position = [40.0, 40.0];
            s.components = Vec::new();
        }
        if let Some(s) = state.ship_mut(&ship4) {
            s.position = [600.0, 600.0];
        }
        if let Some(s) = state.ship_mut(&ship5) {
            s.position = [60.0, 40.0];
            s.components = vec!["railgun".to_string()];
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        // 理智(tempter<0)：欺软怕硬 → 挑威慑低的 ship3。
        if let Some(s) = state.ship_mut(&ship0) {
            s.doctrine.temper = -1.0;
        }
        let rational = nearest_enemy_ship(&state, &config, "中国", [40.0, 40.0], 200.0, None, &ship0);
        assert_eq!(rational, Some(ship3), "理智 should pick the weaker 威慑 target, got {rational:?}");
        // 热血(temper>0)：飞蛾扑火 → 挑威慑高的 ship5。
        if let Some(s) = state.ship_mut(&ship0) {
            s.doctrine.temper = 1.0;
        }
        let hot = nearest_enemy_ship(&state, &config, "中国", [40.0, 40.0], 200.0, None, &ship0);
        assert_eq!(hot, Some(ship5), "热血 should pick the stronger 威慑 target, got {hot:?}");
    }

    /// 武器克制选目标（拟人「别浪费导弹打点防重镇」）：一舰有导弹时，应优先攻击**没有**
    /// 点防御、导弹不会被拦截的目标，而不是把导弹打在被点防全面阻挡的目标上。
    #[test]
    fn target_selection_respects_weapon_advantage() {
        let (config, mut state) = fresh_world(42);
        // China (3) fields a missile-armed attacker (ship 0). Two hostile US (1)
        // targets sit in range: ship 3 has point-defense (intercepts missiles),
        // ship 5 has none — the missile attacker should prefer ship 5.
        let ship0 = state.ships[0].name.clone();
        let ship3 = state.ships[3].name.clone();
        let ship5 = state.ships[5].name.clone();
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = [40.0, 40.0];
            s.components = vec!["missile".to_string()];
        }
        if let Some(s) = state.ship_mut(&ship3) {
            s.position = [40.2, 40.0];
            s.components = vec!["point_defense".to_string()];
        }
        if let Some(s) = state.ship_mut(&ship5) {
            s.position = [40.3, 40.0];
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let target = nearest_enemy_ship(&state, &config, "中国", [40.0, 40.0], 0.4, None, &ship0);
        assert_eq!(
            target,
            Some(ship5),
            "a missile attacker should shun the point-defense ship (5), got {target:?}"
        );
    }

    /// 自保撤退（拟人「别送死」）：一舰离首都较远、已被打残、且敌在射程内时，应后撤
    /// 修整充能（发出 Withdraw、朝首都移动、不送死），而不是死战到被击毁。
    #[test]
    fn damaged_far_ai_ship_withdraws_to_heal() {
        let (config, mut state) = fresh_world(42);
        let home = state.body_position("地球"); // 地球（中国首都）。
        let ship2 = state.ships[2].name.clone();
        let ship3 = state.ships[3].name.clone();
        // China (3) destroyer id 2: badly wounded (hull 5/24) and far from its capital.
        if let Some(s) = state.ship_mut(&ship2) {
            s.position = [40.0, 40.0];
            s.hull = 5.0;
            s.hull_max = 24.0;
        }
        // A hostile US (1) ship within the destroyer's attack range.
        if let Some(s) = state.ship_mut(&ship3) {
            s.position = [40.3, 40.0];
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);

        let d_before = dist([40.0, 40.0], home);
        let mut rng = Prng::new(42);
        advance(&mut state, &config, &mut rng);

        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::Withdraw { ship: s, .. } if *s == ship2)),
            "a damaged far-from-home ship must withdraw, events={:?}",
            state.events
        );
        let s = state.ship(&ship2).expect("withdrawing ship must survive");
        let d_after = dist(s.position, home);
        assert!(
            d_after < d_before,
            "withdrawing ship should head for home ({d_before:.2} -> {d_after:.2})"
        );
    }
}
