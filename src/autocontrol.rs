//! 自动控制（Ai 决策）——「AI 大脑」。
//!
//! 这个 module 只装「谁来控制那些可控量、AI 到底怎么决策」的逻辑。它刻意允许复杂：
//! 战术选目标、舰种选择、组件选装、预算重算、经济预览、海军重构、舰队撤退… 这些是
//! 系统在 `mode = Ai` 时自己做的决定。**它不碰纯引擎机制**（开采/维护/市场/建造/战斗结算/
//! 治理/外交/剧情都是 [`crate::sim`] 的活）——sim 只把「引擎原语」借给这里，这里再
//! 把「决定 + 对可控状态的改写」回交给 sim 去执行。
//!
//! 边界划得很干净，整个项目其余部分保持精简：
//! * [`crate::sim`] = 模拟引擎（回合步进、物理/经济/军事结算、指标）。
//! * 本 module = 自动控制（谁在控制、AI 怎么想）。
//!
//! 所有函数要么 `pub(crate)`（被引擎调用）、要么 `pub`（被 agent CLI/`world` 调用），
//! 其余辅助一律私有，绝不外泄到项目其它角落。

use crate::model::*;
use crate::prng::Prng;
use crate::sim;
use std::collections::BTreeMap;

/// Round a float to 2 decimals (token-noise reduction); `+ 0.0` normalizes IEEE `-0.0`.
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0 + 0.0
}

/// Fixed seed for the [`control_plan`] dry-run ("PLAN"). Production / upkeep / governance are
/// RNG-independent, so this just keeps the preview deterministic across runs.
const PLAN_SEED: u64 = 0x50514f4e;

// --- 预算重算（Ai 每回合从库存重算，Player 只读命令） --------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BudgetKind {
    Investment,
    Construction,
}

/// Read a faction's per-resource budget for a kind: AI resources are recomputed
/// from the stockpile (`stockpile × invest_fraction`), player resources keep the
/// commanded value. Returns the budget map and the per-resource control modes.
pub(crate) fn read_budget(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    kind: BudgetKind,
) -> (ResourceMap, Vec<(String, ControlMode)>) {
    let stockpile: ResourceMap = state
        .faction(&fid)
        .map(|f| f.resources.clone())
        .unwrap_or_default();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    let stock_value: f64 = stockpile.iter().map(|(k, v)| v * value_of(k)).sum();
    // 造舰的「维护费保留」：自动指挥势力在投入造舰预算前，先从库存里预留 `upkeep ×
    // upkeep_reserve_mult` 的市场价值作为维护底线，只把超出部分用于造舰——「把海军养在
    // 经济能承受的规模」。这样基线 AI 不会无脑大建，避免维护费拖垮经济、军备崩盘。
    // 只对 Construction（造舰）生效；投资基础设施（Investment）不受影响。
    let reserve = if kind == BudgetKind::Construction {
        let upkeep: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
            .map(|s| ship_panel(config, s).upkeep)
            .sum();
        upkeep * config.economy.upkeep_reserve_mult
    } else {
        0.0
    };
    let build_value = stock_value * config.economy.invest_fraction;
    // 造舰预算允许的「上限」（市场价值）：不把库存打到维护底线之下。
    let con_cap = if kind == BudgetKind::Construction {
        (build_value).min((stock_value - reserve).max(0.0))
    } else {
        build_value
    };
    let con_scale = if kind == BudgetKind::Construction && build_value > 1e-9 {
        (con_cap / build_value).clamp(0.0, 1.0)
    } else {
        1.0
    };

    let mut budget: ResourceMap = ResourceMap::new();
    let mut modes = Vec::new();
    for (rt, v) in &stockpile {
        let ai_value = *v * config.economy.invest_fraction;
        let mode = match kind {
            BudgetKind::Investment => state.investment_budget_control(fid.clone(), rt),
            BudgetKind::Construction => state.construction_budget_control(fid.clone(), rt),
        };
        let value = match mode {
            ControlMode::Ai => ai_value * con_scale,
            ControlMode::Player => state
                .control(fid.clone())
                .and_then(|c| match kind {
                    BudgetKind::Investment => c.investment_budget.get(rt),
                    BudgetKind::Construction => c.construction_budget.get(rt),
                })
                .map(|c| c.value)
                .unwrap_or(ai_value * con_scale),
        };
        budget.insert(rt.clone(), value);
        modes.push((rt.clone(), mode));
    }
    (budget, modes)
}

/// Write a computed budget back into the faction's controllable state so the
/// diff between rounds reflects what the simulation actually used.
pub(crate) fn write_budget(
    state: &mut State,
    fid: FactionId,
    kind: BudgetKind,
    budget: &ResourceMap,
    modes: &[(String, ControlMode)],
) {
    if let Some(c) = state.control_mut(fid.clone()) {
        for (rt, value) in budget {
            let mode = modes.iter().find(|(r, _)| r == rt).map(|(_, m)| *m).unwrap_or(ControlMode::Ai);
            let slot = match kind {
                BudgetKind::Investment => &mut c.investment_budget,
                BudgetKind::Construction => &mut c.construction_budget,
            };
            match mode {
                ControlMode::Ai => {
                    slot.insert(rt.clone(), Control::inherit(*value));
                }
                ControlMode::Player => {
                    slot.entry(rt.clone()).or_insert_with(|| Control::player(*value));
                }
            }
        }
    }
}

// --- 舰种 / 组件选装（AI 决定造什么） ----------------------------------------

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

    // 强制装配一件指定类别（买得起选最高分；买不起强制最便宜一件兜底）。
    let force_cat = |cat: &str, chosen: &mut Vec<String>, remaining: &mut ResourceMap| {
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
            let mut cheapest: Option<(String, f64)> = None;
            for (id, cs) in &config.components {
                if cs.category == cat {
                    let cost_val: f64 = cs.cost.iter().map(|(r, c)| c * value_of(r)).sum();
                    if cheapest.as_ref().map(|(_, c)| cost_val < *c).unwrap_or(true) {
                        cheapest = Some((id.clone(), cost_val));
                    }
                }
            }
            if let Some((id, _)) = cheapest {
                let cs = config.component_spec(&id);
                for (r, c) in &cs.cost {
                    let e = remaining.entry(r.clone()).or_insert(0.0);
                    *e = (*e - c).max(0.0); // 买不起也不至于负——最差兜底。
                }
                chosen.push(id);
            }
        }
    };
    // 硬保证：至少一件武器（攻击力来源）+ 至少一件推进（速度来源）。
    force_cat("weapon", &mut chosen, &mut remaining);
    force_cat("thrust", &mut chosen, &mut remaining);
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

// --- 海军随威胁重构（AI 重定向船坞） ----------------------------------------

/// 威胁响应（海军随威胁重构）：交战中，若某势力的舰队被单一舰型统治（占比 > `over_share`），
/// 就把它产出该舰型的最小 id 船坞重定向到 `choose_next_class` 选出的**战局感知新舰型**
/// （战争加分——多造重舰；去重加分——避免单调）。和平时不重定向（船坞保持生产既有舰型）。
/// 每次至多重定向一个船坞、且只在明显过度生产时触发，避免抖振。确定性（seeded RNG）。
pub(crate) fn retool_shipyards(state: &mut State, config: &GameConfig, fid: &str, rng: &mut Prng) {
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
    }
}

// --- 战术选目标（AI 决定打谁） ------------------------------------------------

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
    let temper = attacker.doctrine.temper;
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
    if ship.kiting.abs() < 1e-9 {
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
    let desired_r = if ship.kiting < 0.0 {
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
        let lone_wolf = state.ship(ship_id).map(|s| s.doctrine.lone_wolf).unwrap_or(0.0);
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

// --- 单舰 AI 回合（从 sim::step_military 的 is_ai 分支抽出） ------------------

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
    let kiting = ship.kiting;

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

// --- 经济→成本预览（--control-plan） ------------------------------------------

/// A **cost → benefit preview** for one faction's current control surface: the per-round
/// economy balance the simulation will produce next round (production vs fleet upkeep vs
/// governance), the commanded construction/investment budgets, and a verdict on whether the
/// faction is over-extending its economy.
///
/// The numbers are the **simulation's own**: it dry-runs one real [`sim::advance`] on a clone
/// with a fixed RNG seed, then reads the captured [`RoundFlow`] through [`sim::round_metrics`] —
/// so there is zero drift between the preview and what [`sim::advance`] would actually do.
/// (Production, upkeep and governance are RNG-independent, so the fixed seed is just for
/// determinism.) Purely analytical: it never mutates the caller's state and never consumes the
/// caller's RNG.
///
/// Exposed via the agent CLI `--control-plan <faction>`; callers use it to see the cost of a
/// budget before committing it, instead of discovering a collapse by trial and error.
pub fn control_plan(state: &State, config: &GameConfig, fid: &str) -> Option<serde_json::Value> {
    if !state.factions.iter().any(|f| f.name == fid) {
        return None;
    }
    let metrics = dry_metrics(state, config);
    plan_core(state, config, &metrics, fid)
}

/// The same cost→benefit preview for **every** faction, from a single dry-run (one clone +
/// one [`sim::advance`]); the returned map is keyed by faction id. Exposed via
/// `--control-plan` (no faction argument).
pub fn control_plan_all(state: &State, config: &GameConfig) -> BTreeMap<String, serde_json::Value> {
    let metrics = dry_metrics(state, config);
    state
        .factions
        .iter()
        .filter_map(|f| plan_core(state, config, &metrics, &f.name).map(|v| (f.name.clone(), v)))
        .collect()
}

/// Dry-run one real [`sim::advance`] on a clone (fixed seed) and return the captured
/// [`sim::round_metrics`] — the simulation's own per-round numbers, never re-derived. The
/// caller's state and RNG are untouched.
fn dry_metrics(state: &State, config: &GameConfig) -> RoundMetrics {
    let mut s = state.clone();
    let mut r = Prng::new(PLAN_SEED);
    let flow = sim::advance(&mut s, config, &mut r);
    sim::round_metrics(&s, config, &flow)
}

/// Build one faction's profile from the real `state` (commands / stockpile) and the
/// dry-run `metrics` (production / upkeep / governance for the coming round).
fn plan_core(state: &State, config: &GameConfig, metrics: &RoundMetrics, fid: &str) -> Option<serde_json::Value> {
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    // 库存市场价值（当前、未推进）。
    let stock_value: f64 = state
        .faction(fid)
        .map(|f| f.resources.iter().map(|(k, v)| v * value_of(k)).sum())
        .unwrap_or(0.0);
    // 当前舰队维护费（step_upkeep / read_budget 用的同一口径）。
    let upkeep_now: f64 = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| ship_panel(config, s).upkeep)
        .sum();

    // 本回合订单（按当前 mode：Ai 重算 / Player 用命令），已含造舰维护保留上限。
    let (con_budget, _) = read_budget(state, config, fid.to_string(), BudgetKind::Construction);
    let (inv_budget, _) = read_budget(state, config, fid.to_string(), BudgetKind::Investment);
    let con_value: f64 = con_budget.iter().map(|(k, v)| v * value_of(k)).sum();
    let inv_value: f64 = inv_budget.iter().map(|(k, v)| v * value_of(k)).sum();
    // AI 自己的保守造舰上限（维护保留后）：用于对比「命令的预算」是否更激进。
    let ai_cap = (stock_value * config.economy.invest_fraction)
        .min((stock_value - upkeep_now * config.economy.upkeep_reserve_mult).max(0.0));

    let Some(fm) = metrics.factions.get(fid) else { return None };

    let production = fm.production_value;
    let upkeep = fm.upkeep;
    let governance = fm.governance_cost;
    let net = production - upkeep - governance;
    let feed_cap = (production - governance).max(0.0); // 扣掉治理后能养得起的舰队维护。
    let fleet_overextended = upkeep > feed_cap + 1e-9;
    let over_committed = con_value > ai_cap + 1e-9;
    let net_negative = net < -1e-9;
    let rounds = if net_negative {
        Some((stock_value / -net).max(0.0))
    } else {
        None
    };
    let verdict = if net_negative {
        "bleeding"
    } else if over_committed {
        "over-committed"
    } else {
        "healthy"
    };

    Some(serde_json::json!({
        "faction": fid,
        "round": state.round,
        "production_value": r2(production),
        "upkeep": r2(upkeep),
        "governance_cost": r2(governance),
        "governance_coverage": r2(fm.governance_coverage),
        "net_flow": r2(net),
        "stock_market_value": r2(stock_value),
        "construction_budget_value": r2(con_value),
        "investment_budget_value": r2(inv_value),
        "ai_construction_cap": r2(ai_cap),
        "over_committed_construction": over_committed,
        "fleet_upkeep_cap": r2(feed_cap),
        "fleet_overextended": fleet_overextended,
        "fleet_value": r2(fm.fleet_value),
        "ship_count": fm.ship_count,
        "city_count": fm.city_count,
        "population": fm.population,
        "rounds_before_insolvent": rounds.map(r2),
        "verdict": verdict,
    }))
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

    /// Guard: the cost→benefit preview reports the current economy balance
    /// (`net = production − upkeep − governance`) and, when the agent commands an
    /// over-committed construction budget, flags it *before* it collapses — the
    /// reported "建造预算拉满 → 维护 > 产出 → 城清零" footgun.
    #[test]
    fn control_plan_balances_and_flags_over_committed_construction() {
        let (config, mut state) = fresh_world(42);

        // Baseline: 中国 self-sustaining at the start.
        let plan = control_plan(&state, &config, "中国").expect("faction exists");
        let p = plan["production_value"].as_f64().unwrap();
        let u = plan["upkeep"].as_f64().unwrap();
        let g = plan["governance_cost"].as_f64().unwrap();
        let net = plan["net_flow"].as_f64().unwrap();
        assert!((net - (p - u - g)).abs() < 0.05, "net must ≈ production − upkeep − governance");
        assert_eq!(plan["verdict"].as_str().unwrap(), "healthy");
        assert!(!plan["over_committed_construction"].as_bool().unwrap());
        assert!(plan["rounds_before_insolvent"].is_null());

        // Command a huge construction budget on a held resource with mode=Player.
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "construction_budget": [
                {"resource": "铁", "value": 10000.0, "mode": "Player"},
                {"resource": "碳", "value": 10000.0, "mode": "Player"}
            ]}]
        });
        crate::web::apply_patch(&mut state, &config, &diff).expect("apply construction over-commit");

        let plan2 = control_plan(&state, &config, "中国").expect("faction exists");
        assert!(
            plan2["construction_budget_value"].as_f64().unwrap() > 0.0,
            "commanded construction budget must be non-zero"
        );
        assert!(
            plan2["over_committed_construction"].as_bool().unwrap(),
            "over-committed construction must be flagged"
        );
        assert_ne!(plan2["verdict"].as_str().unwrap(), "healthy");
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
        let mut got = std::collections::BTreeSet::new();
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
        let shipyard_types = |st: &State, f: FactionId| -> std::collections::BTreeSet<(CityId, String)> {
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
        retool_shipyards(&mut state, &config, "中国", &mut rng);
        let after = shipyard_types(&state, "中国".to_string());
        assert!(
            after.iter().any(|(_, t)| t != "corvette"),
            "a corvette-dominated wartime fleet should retool a shipyard into a war class; before={before:?} after={after:?}"
        );
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
