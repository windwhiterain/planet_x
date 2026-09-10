//! 舰种 / 组件选装（AI 决定造什么）+ 海军随威胁重构。

use crate::model::*;
use crate::prng::Prng;
use crate::sim;
use std::collections::{BTreeMap, BTreeSet};

use super::contract::sigmoid;
use super::freight;

// --- 造舰动机（用户裁决：**让造货运船的动机和造战争船的动机解耦**）---------------
//
// 两条动机、**两条通道**，刻意不折进同一个分数：
//
// * **战斗舰**看**敌对国与自己的实力差距**（[`threat_motive`]）——取代旧的「是否处于战争」
//   这个布尔 + 常数加分。差距为负（敌人都比我弱）时动机压到 0 附近。
// * **货船**看**集货运力缺口**（[`freight::haul_gap`] → [`retool_haulers`]）——与威胁无关，
//   也不用舰队编成（编成是战时那条通道的事）。
//
// 两者**各占一个船坞**：战时重构先跑（行为逐字节不变），集货侧重构只认**它剩下的**船坞。
// 旧写法里两者共用 `choose_next_class` 的一个 score，于是「有威胁」永远压过「缺运力」
// ——那正是耦合。修 bug 时不要把两条动机再合成一个分数。

/// 造战斗舰的动机上限：旧写法里「处于战争 ⇒ 旗舰 +0.6」的那个 0.6，现在成了**上限**
/// （动机为 1 时才给满），于是这条改动只会让「战时堆旗舰」变得**更有分辨力**、不会更极端。
const WAR_BUILD_BONUS: f64 = 0.6;
/// 威胁动机的**中点**与**软化宽度**（相对差距的尺度，见 [`threat_motive`]）：
/// 相对差距 1.0（敌对国的净超出 = 我全部实力）⇒ 动机 0.5。
const THREAT_MID: f64 = 1.0;
const THREAT_WIDTH: f64 = 0.5;
/// **造舰的时间成本**：`TIME_PENALTY` 是顶格扣分，`TIME_REF` 是「不算慢」的回合数，
/// `TIME_WIDTH` 是软化宽度。三条是**同一件事**的三个位置（造多久算慢、慢多少扣多少）。
const TIME_PENALTY: f64 = 0.4;
const TIME_REF: f64 = 12.0;
const TIME_WIDTH: f64 = 6.0;
/// 集货动机的**中点**：搬不动的比例过半才是「真的运不过来」⇒ 动机 0.5。
const HAUL_MID: f64 = 0.5;
const HAUL_WIDTH: f64 = 0.25;

/// **造一艘某舰级的舰大概要几个回合**（纯函数，给 AI 决策用；`None` = 结构上造不出来）。
///
/// ```text
/// 回合数 = build_points ÷ 每回合能推的进度
/// 每回合进度 = max_affordable_inc( 成本÷build_points , 本势力的**造舰预算** , 0 , 船坞速率 )
/// 船坞速率 = 该势力**最能造的那座城**的建造区面积 × 生产率 × 劳动比
/// ```
///
/// * **与真实建造同源**：分母用的就是 `sim` 里那一段（[`sim::max_affordable_inc`]），
///   预算读的就是自动控制自己写的那份造舰预算（[`super::budget::read_budget`]），
///   劳动比也是 [`sim::labor_ratio`]——**三处都不另编**，否则「AI 以为 5 回合、实际 50 回合」
///   这种错会悄悄发生（实测踩过：只看运力选舰级 ⇒ 全世界船坞都改成最贵的航母、一艘也下不了水）。
/// * **预算与「买不买得起」是同一件事**：造舰预算本来就是从库存里按维护费保留之后算出来的
///   （见 `budget::read_budget`），所以穷势力的每一级回合数都长，而**长的程度不一样**
///   ——那正是「单位时间的运力」要区分的东西。
/// * 取**最能造的那座城**（舰级进度是**按城**聚合的）：估的是「我这势力要多久能拿到这条船」，
///   而不是某座具体船坞的排期。
pub fn build_rounds(state: &State, config: &GameConfig, fid: &str, class: &str) -> Option<f64> {
    let spec = config.ship_spec(class);
    let bp = spec.build_points;
    if bp <= 1e-9 {
        return None;
    }
    let productivity = config.building_spec("construction").productivity;
    let mut best_rate = 0.0f64;
    for c in state.cities.iter().filter(|c| c.faction_id == fid && !c.razed) {
        let area: f64 = c
            .buildings
            .iter()
            .filter(|b| b.is_shipyard())
            .map(|b| b.deployed)
            .sum();
        if area <= 1e-9 {
            continue;
        }
        let labor = sim::labor_ratio(state, config, &c.name);
        best_rate = best_rate.max(area * productivity * labor);
    }
    if best_rate <= 1e-9 {
        return None; // 没有建造区 ⇒ 结构上造不出来。
    }
    let (budget, _mode) = super::budget::read_budget(
        state,
        config,
        fid.to_string(),
        super::budget::BudgetKind::Construction,
    );
    let per_progress: Vec<(String, f64)> = spec
        .build_cost
        .iter()
        .map(|(r, c)| (r.clone(), c / bp))
        .collect();
    let inc = sim::max_affordable_inc(&per_progress, &budget, &ResourceMap::new(), best_rate);
    if inc <= 1e-9 {
        return None; // 预算推不动任何进度 ⇒ 等同造不出来（不是「很久」）。
    }
    Some(bp / inc)
}

/// **威胁动机**（0..1 的连续量）：**敌对国比自己强多少**。
///
/// ```text
/// 敌对度_j = σ((开战阈值 − 对 j 的关系) ÷ 半个开战阈值)     // 关系正好在阈值上 = 0.5
/// 相对差距 = Σ_j 敌对度_j × (实力_j − 自己实力) ÷ 自己实力   // 只有比自己强的才算数
/// 动机     = σ((相对差距 − THREAT_MID) ÷ THREAT_WIDTH)
/// ```
///
/// * **实力**用均势外交那把尺子（[`sim::faction_power_share`]：城 + 舰体占全星系的比例），
///   不另编一个「军事实力」——否则「谁是霸权」与「谁该备战」会给出两个不同的答案。
/// * **相对差距**（除以自己的实力）而不是绝对差：`AGENTS.md`「永远用相对数值比例而非绝对数值」，
///   而且尺度无关（星系总实力随局内发展涨落，绝对差会漂）。
/// * **只有比自己强才算威胁**：差距为负时动机趋近 0——压得住场子的势力不会因为「在打仗」
///   就继续堆旗舰。于是**众弱结盟的备战动机 > 霸权的**，与 `step_balance_of_power` 的
///   合纵连横自然咬合（霸权反而会松懈，这是这段机制的剧情价值）。
pub fn threat_motive(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let shares = sim::faction_power_share(state, config);
    let mine = shares.get(fid).copied().unwrap_or(0.0).max(1e-6);
    let Some(f) = state.faction(fid) else { return 0.0 };
    let war = config.combat.war_threshold;
    let width = (-war).max(1.0) * 0.5;
    let mut deficit = 0.0;
    for (other, s) in &shares {
        if other == fid {
            continue;
        }
        let rel = f.relations.get(other).copied().unwrap_or(0.0);
        deficit += sigmoid((war - rel) / width) * (s - mine);
    }
    sigmoid((deficit / mine - THREAT_MID) / THREAT_WIDTH)
}

/// **货船舰级**：本作里「一趟装多少 ÷ 单位时间 ÷ 养它多少钱」最好的那一级——**数据驱动，无特判**。
///
/// 用的是与定编同一把尺子（舰级版 `舱容 × 速度 × 维护费`，见 [`freight::freight_tonnage`]）
/// ⇒ 配置里改数值它就跟着变，不写死舰名。今天选出的是**航母**（20×1.0÷7.5 = 2.67，
/// 次高的驱逐只有 2.08），恰好也是唯一一个 `default_role = true` 的舰级
/// ——派生结论与作者意图对上了，这是那把尺子没错的旁证。
pub fn hauler_class(state: &State, config: &GameConfig, fid: &str) -> Option<String> {
    config
        .ships
        .keys()
        .map(|cls| {
            // **单位时间能造出来的运力** = 舰级运力 ÷ 建造回合数（造不出来的记 0）。
            //
            // 为什么必须除建造时间：只看运力时永远选**航母**（20×1.0÷7.5 = 2.67，比次高的
            // 驱逐 2.08 高 28%，却贵一倍还多）。实测（seed 7 / 600 回合）：每个势力的船坞都被
            // 改成航母却**一艘也下不了水**，全世界 600 回合只拆平 4 次——战争没了、运输也没了。
            // 除过时间之后，穷势力会选**造得动**的那一级，富势力仍然选航母（它也是唯一一个
            // `default_role = true` 的舰级，派生结论与作者意图对上了）。
            let spec = config.ship_spec(cls);
            let tonnage = spec.cargo * spec.speed_mult / spec.upkeep.max(1e-6);
            let score = match build_rounds(state, config, fid, cls) {
                Some(rounds) if rounds > 0.0 => tonnage / rounds,
                _ => 0.0,
            };
            (cls.clone(), score)
        })
        .filter(|(_, s)| *s > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(c, _)| c)
}

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
    // **造战斗舰的动机** = 敌对国与自己的实力差距（连续量），不再是「是否处于战争」这个布尔。
    let motive = threat_motive(state, config, fid);

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
        let war_bonus = if flagship { WAR_BUILD_BONUS * motive } else { 0.0 };
        // **时间成本**（用户裁决：「得让 AI 能估计建造时间」）：造得越久，这一级越不该现在排产。
        // 用 `σ((回合数 − TIME_REF) ÷ TIME_WIDTH)` 而不是硬性的「超过 N 回合不造」——
        // **结构上造不出来**（`None`）才顶格扣分，而「要造很久」只是扣分（穷势力仍然造得出重舰，
        // 只是不划算），遵 `AGENTS.md`：不设进不去的目标。
        let time_penalty = match build_rounds(state, config, fid, cls) {
            Some(rounds) => TIME_PENALTY * sigmoid((rounds - TIME_REF) / TIME_WIDTH),
            None => TIME_PENALTY,
        };
        scored.push((cls.clone(), fit + mix_bonus - upkeep_penalty + war_bonus - time_penalty));
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

/// 选装评分的**偏好旋钮**（`choose_loadout` 里唯一允许被"设计主题"改写的那几个数）。
///
/// [`Default`] = **历史行为的逐字复刻**（所有分类权重 1.0、无造价惩罚、战时武器加分 2.0）
/// ⇒ `choose_loadout` 与设计图落地之前逐字节一致，而 AI 的设计主题只是在它上面加了一层
/// 「这一型舰是干什么的」（`autocontrol::blueprints` 用 [`Self::from_theme`]）。
///
/// ⚠ 它**不改**「买不起就不装」那条硬规则，也不改「至少一件武器 + 至少一件推进」那两条硬保证
/// ——主题只调**偏好**，不能凭空造出势力供不起的舰。
#[derive(Clone, Copy, Debug)]
pub(crate) struct LoadoutPrefs {
    /// 战时武器加分（历史值 2.0）。
    pub war_weapon_bonus: f64,
    /// 四个模块分类对评分里"战斗增益"项的权重。
    pub cat_weapon: f64,
    pub cat_defense: f64,
    pub cat_thrust: f64,
    pub cat_utility: f64,
    /// 造价（市场价值）惩罚：>0 时更看重便宜模块。
    pub cost_penalty: f64,
    /// **库存折价**（1.0 = 用真实库存；<1 = 只当自己有那么多）。
    ///
    /// 它只给**设计图生成器**用（`choose_loadout_themed` 按 `autocontrol.blueprint_stock_margin`
    /// 折算）：图是在**回合步进里**画的，而船是在**下一回合出厂那一刻**才付钱的，两者之间库存
    /// 会变。留一道余量 ⇒ 出厂时那笔钱通常还在（否则 `commit_spend` 的钳零会白送模块，
    /// 而白送的模块**照样要付维护费**——实测那会把舰队推过"养不起→锈蚀→拆解"的悬崖）。
    pub stock_scale: f64,
}

impl Default for LoadoutPrefs {
    fn default() -> Self {
        Self {
            war_weapon_bonus: 2.0,
            cat_weapon: 1.0,
            cat_defense: 1.0,
            cat_thrust: 1.0,
            cat_utility: 1.0,
            cost_penalty: 0.0,
            stock_scale: 1.0,
        }
    }
}

impl LoadoutPrefs {
    /// 设计主题 → 偏好旋钮（`config/game.ron` 的 `autocontrol.blueprint_themes`）。
    ///
    /// `stock_scale` 取 `1/(1 + blueprint_stock_margin)`：`margin = 1` ⇒ 只把一半库存当可用
    /// （"画得出的图，钱要留一倍"）。`margin = 0` ⇒ 与出厂现算同一条线（历史行为）。
    pub(crate) fn from_theme(theme: &crate::model::DesignTheme, margin: f64) -> Self {
        Self {
            cat_weapon: theme.cat_weapon,
            cat_defense: theme.cat_defense,
            cat_thrust: theme.cat_thrust,
            cat_utility: theme.cat_utility,
            cost_penalty: theme.cost_penalty,
            stock_scale: 1.0 / (1.0 + margin.max(0.0)),
            ..Self::default()
        }
    }

    /// 某个分类的权重（未知分类归 `utility`：配置里只会有这四类，这是兜底而不是分支）。
    fn cat_mult(&self, category: &str) -> f64 {
        match category {
            "weapon" => self.cat_weapon,
            "defense" => self.cat_defense,
            "thrust" => self.cat_thrust,
            _ => self.cat_utility,
        }
    }
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
    choose_loadout_prefs(state, config, fid, class, &LoadoutPrefs::default())
}

/// 同 [`choose_loadout`]，但按一份**设计主题**的偏好选装（AI 的设计图生成器走这条）。
///
/// 与出厂现算有**两处刻意的不一样**（都是"图是在回合步进里画的"这件事带来的）：
/// 1. 主题只改评分（分类权重 + 造价惩罚）；
/// 2. 库存按 `autocontrol.blueprint_stock_margin` 折价 —— 留一道余量，让出厂那一刻真的付得起
///    （见 [`LoadoutPrefs::stock_scale`]）。
///
/// 可得性检查、硬保证、排序兜底全部照旧 ⇒ 主题不同的两张图**都**买得起，只是"钱花在哪一类
/// 模块上"不同。
pub(crate) fn choose_loadout_themed(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    class: &str,
    theme: &crate::model::DesignTheme,
) -> Vec<String> {
    let margin = config.autocontrol.blueprint_stock_margin;
    choose_loadout_prefs(state, config, fid, class, &LoadoutPrefs::from_theme(theme, margin))
}

fn choose_loadout_prefs(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    class: &str,
    prefs: &LoadoutPrefs,
) -> Vec<String> {
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
    // 「战斗增益」那一项乘**主题**的分类权重（`prefs`；默认全 1.0 = 历史行为逐字不变）。
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
        let gain = (gain_weapon * 4.0 + gain_shield * 0.8 + cs.hardness * spec.armor_mult * 3.0
            + gain_regen * 60.0 + gain_speed * 3.0 + gain_accel * 3.0
            + cs.intercept * spec.pd_mult * 2.0 + gain_range * 12.0)
            * prefs.cat_mult(&cs.category);
        let cost_val: f64 = cs.cost.iter().map(|(r, c)| c * value_of(r)).sum();
        let mut score = fit + gain * 0.03 - cs.upkeep * 2.0 - cost_val * prefs.cost_penalty;
        if at_war && cs.category == "weapon" {
            score += gain_weapon * prefs.war_weapon_bonus; // 战时要火力。
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
    // 可用库存 = 真实库存 × `prefs.stock_scale`（1.0 = 历史行为）。折价只影响"装不装得起"，
    // 不影响下面的评分（评分里那份 `abund` 用的是**真实**库存的归一化分布）。
    let mut remaining: ResourceMap = f
        .resources
        .iter()
        .map(|(k, v)| (k.clone(), v * prefs.stock_scale))
        .collect();
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

/// **出厂选装的唯一入口**：有设计图就按图装配，没有（或图没写选装）就走生成器。
///
/// 两条路（`.agents/notes/ship-blueprint-spec.md` §2.4 + 本轮 `autocontrol::blueprints`）：
/// * 图存在且 `components` 非空 ⇒ **用图上的选装**（原样，顺序 = 槽位顺序；合法性由 `--apply`
///   与建图路径在写入时守卫）；
/// * 没有图（旧档 / 开局预置舰队 / 剧情赠舰）或图的选装为空 ⇒ 现场调 [`choose_loadout`]。
///
/// ⚠ **归属（`Player`/`Auto`）不在这里分支**——这是本轮改掉的旧语义。旧版只在图归 `Player`
/// 时才用图上的选装，于是 `Auto` 图的 `components` 被**静默忽略**（"我画了图，出厂却按生成器
/// 装"），而 `Auto` 图那一层本来就承诺"系统可重估它"。正确的分工是：
/// **图 = 出厂规格（装什么），`mode` = 谁可以改这张图（写到哪一层归谁）**。
/// 于是 AI 设计的图（`Control::inherit` + `components`）真的会让新舰按设计下水
/// ——否则 `Auto` 图那一层还是半个空头支票。
///
/// ⚠ **`components` 为空 = 「交给生成器」**：一张只钉舰级/意图的图不必把选装也抄一遍。
///
/// ⚠ 本函数是**纯读**的：它不改状态、不消费 RNG。选装算在哪一刻是设计的一部分——
/// AI 的设计图在**回合步进里**（`autocontrol::blueprints`）按当时的库存算好并落进图里，
/// 于是"这张图造一艘要什么模块"是可读的；没有图时则仍是**出厂那一刻**现算（旧行为）。
pub(crate) fn resolve_loadout(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    class: &str,
    blueprint: Option<&BlueprintId>,
) -> Vec<String> {
    if let Some(id) = blueprint {
        if let Some(leaf) = state.control(fid.clone()).and_then(|c| c.blueprints.get(id)) {
            if !leaf.value.components.is_empty() {
                return leaf.value.components.clone();
            }
        }
    }
    choose_loadout(state, config, fid, class)
}

/// 威胁响应（海军随威胁重构）：交战中，若某势力的舰队被单一舰型统治（占比 > `over_share`），
/// 就把它产出该舰型的最小 id 船坞重定向到 `choose_next_class` 选出的**战局感知新舰型**
/// （战争加分——多造重舰；去重加分——避免单调）。和平时不重定向（船坞保持生产既有舰型）。
/// 每次至多重定向一个船坞、且只在明显过度生产时触发，避免抖振。确定性（seeded RNG）。
///
/// 真的改了就往 `retools` **追加一行**（纯记录）：这是少数几个**不留事件的 AI 决策**之一，
/// 事后只能从 `ship_type` 的变化反推、且看不出是什么时候改的。
///
/// **设计图的归属 gate**（spec §4.7：不堵这条路，「玩家钉住的图 AI 不许重估」就会因为
/// 另一条路径（改 `ship_type` 造成 `blueprint_class_mismatch`）而失效）：
/// * 目标舰坞挂了**归属解析为 `Player`** 的图 ⇒ **跳过它**，另选一个；一个都没有就什么都不做；
/// * 挂了 `Auto` 图 ⇒ 改的是**图**（`class`，并保持 `ship_type` 与它一致），而不是只改
///   `ship_type`（那样会让图与区对不上，等于把图作废）；
/// * 挂了**悬空指针**（图不存在）⇒ 那个区本来就停产（Q10(a)），跳过。
///
/// ⚠ **RNG 消耗次序不变**：`choose_next_class` 的 `rng.unit()` 仍在同一位置、同样次数被调用
/// （归属只影响它**之后**的目标选择），否则同 seed 的 `--digest` 会变。
pub(crate) fn retool_shipyards(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    rng: &mut Prng,
    retools: &mut Vec<RetoolDecision>,
) {
    // **威胁动机取代「是否处于战争」这个布尔**（连续、且只有**比自己强的**敌人才算威胁）。
    // 用概率闸而不是 `motive > 常数`：动机 0.9 ⇒ 九成回合照旧重构；动机 0.1 ⇒ 偶尔提前备战
    // （冷战期也会造舰——那正是「动机连续」的意义）。骰子走 `derived_roll`，不消费主 `Prng`。
    let motive = threat_motive(state, config, fid);
    if sim::derived_roll(fid, "war-retool", state.round, "retool") >= motive {
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
    // 找到产出 over_class 的**可改装**船坞（按城名、再按建筑 id 的第一个）。
    let target = retoolable_yards(state, fid)
        .into_iter()
        .find(|(_, _, cls, _)| cls == &over_class);
    if let Some((cid, bid, from, bp)) = target {
        apply_yard_class(state, fid, &cid, bid, bp.as_ref(), &new_class);
        retools.push(RetoolDecision {
            faction: fid.to_string(),
            city: cid,
            building: bid,
            from,
            to: new_class,
        });
    }
}

/// 本势力**可改装的船坞**：`(城, 建筑, 现在造的舰级, 挂的设计图)`，按 (城名, 建筑 id) 序。
///
/// 两道归属闸（与设计图那套口径一致，见 spec §4.7）：
/// * **悬空图指针** ⇒ 那个区本来就停产（Q10(a)），改装它没有意义 ⇒ 跳过；
/// * **归属解析为 `Player` 的图** ⇒ 跳过（玩家钉住的图 AI 不许重估）。
///
/// 只有 `ship_type` 已定的区算数（没定舰级的区本来就不出舰）。
fn retoolable_yards(
    state: &State,
    fid: &str,
) -> Vec<(CityId, BuildingId, String, Option<BlueprintId>)> {
    let mut out = Vec::new();
    let fid_owned = fid.to_string();
    for c in &state.cities {
        if c.faction_id != fid {
            continue;
        }
        for b in &c.buildings {
            if !b.is_shipyard() {
                continue;
            }
            let Some(cls) = b.ship_type.clone() else { continue };
            if let Some(bp) = b.blueprint.as_ref() {
                if !blueprint_known(state, &fid_owned, bp) {
                    continue;
                }
                if state.blueprint_control(&fid_owned, bp).is_player() {
                    continue;
                }
            }
            out.push((c.name.clone(), b.id, cls, b.blueprint.clone()));
        }
    }
    out
}

/// 把一个船坞的舰级改成 `new_class`（调用方已保证它可改装：没挂玩家图、指针不悬空）。
///
/// 挂了 `Auto` 图 ⇒ **连图的 `class` 一起改**：口径 A 要求两者相等，只改一边会让这张图对不上
/// 它自己的建造区（`blueprint_class_mismatch` 的形态）。
/// 选装**不预生成**（`components` 保持原样 = 空 ⇒ 出厂时由生成器现算，见 `resolve_loadout`）：
/// 在这里算一次会把「出厂那一刻按当时库存算」变成「改装那一刻算」，那是行为改变
/// （spec §2.4 的硬约束）。
fn apply_yard_class(
    state: &mut State,
    fid: &str,
    cid: &CityId,
    bid: BuildingId,
    bp: Option<&BlueprintId>,
    new_class: &str,
) {
    if let Some(c) = state.city_mut(cid) {
        for b in &mut c.buildings {
            if b.id == bid {
                b.ship_type = Some(new_class.to_string());
                break;
            }
        }
    }
    if let Some(bp) = bp {
        if let Some(leaf) = state
            .control_mut(fid.to_string())
            .and_then(|c| c.blueprints.get_mut(bp))
        {
            leaf.value.class = new_class.to_string();
        }
    }
}

/// **集货侧的独立通道**：搬不动的比例一大，就腾**一个**船坞改产货船。
///
/// ```text
/// 动机 = σ((搬不动的比例 − HAUL_MID) ÷ HAUL_WIDTH) × 思潮倾向
/// ```
///
/// * **搬不动的比例**（[`freight::haul_gap`]）已经把「雇到的人」扣掉了：雇得到人就不必自己
///   造船——雇佣市场本来就该顶掉缺口（分工，而不是重复建设）。
/// * **乘思潮倾向**（[`freight::freight_lean`]）：军国宁可缺货、宁可雇人，也不把船坞从战争
///   生产上挪开——与角色轴用的是**同一个** `lean`（同一条裁决，两处落地）。
/// * 掷一次 `(势力, "hauler-retool", 回合)` 的派生骰子：**不消费主 `Prng`**。
/// * 命中就改**一个**船坞（至多一个/回合，免得一口气把造船能力全搬走）：挑**本势力舰数最少
///   的那一级**的船坞（改产冗余最小的一级），同分按 (城名, 建筑 id) 序（`sort_by` 是稳定的）。
/// * **至多腾一个**（本势力已经有造货船的船坞就什么都不做）
/// * **不碰本回合已被战时重构拿走的船坞**（`claimed`）：两条动机各占一个，谁也不淹没谁
///   ——这就是「造货船的动机与造战争船的动机**解耦**」的落点。
pub(crate) fn retool_haulers(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    claimed: &BTreeSet<(CityId, BuildingId)>,
    retools: &mut Vec<RetoolDecision>,
) {
    let Some(hauler) = hauler_class(state, config, fid) else { return };
    // **至多一个货船船坞**：配额是按「处积压数」算的几条腿，一个船坞的产出绰绰有余。
    // 没有这条闸，缺口大的势力会**每回合**腾一个船坞，把全势力的造船能力都改成货船
    //（实测：那样做会把整个世界的战争产能搬空）。
    if retoolable_yards(state, fid).iter().any(|(_, _, cls, _)| *cls == hauler) {
        return;
    }
    let p = (sigmoid((freight::haul_gap(state, config, fid) - HAUL_MID) / HAUL_WIDTH)
        * freight::freight_lean(state, fid))
    .clamp(0.0, 1.0);
    if p <= 0.0 || sim::derived_roll(fid, "hauler-retool", state.round, "retool") >= p {
        return;
    }
    // 每一级现在有几艘（挑**冗余最小**的那一级改产）。
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for s in state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0) {
        *counts.entry(s.class.clone()).or_insert(0) += 1;
    }
    let mut cands: Vec<(CityId, BuildingId, String, Option<BlueprintId>)> =
        retoolable_yards(state, fid)
            .into_iter()
            .filter(|(cid, bid, cls, _)| {
                cls != &hauler && !claimed.contains(&(cid.clone(), bid.clone()))
            })
            .collect();
    cands.sort_by(|a, b| {
        counts
            .get(&a.2)
            .copied()
            .unwrap_or(0)
            .cmp(&counts.get(&b.2).copied().unwrap_or(0))
    });
    let Some((cid, bid, from, bp)) = cands.into_iter().next() else { return };
    apply_yard_class(state, fid, &cid, bid, bp.as_ref(), &hauler);
    retools.push(RetoolDecision {
        faction: fid.to_string(),
        city: cid,
        building: bid,
        from,
        to: hauler,
    });
}

/// 这个势力库里**有没有**这张图（`Building.blueprint` 是悬空指针吗）。
fn blueprint_known(state: &State, fid: &str, bp: &str) -> bool {
    state
        .control(fid.to_string())
        .map(|c| c.blueprints.contains_key(bp))
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "../tests/autocontrol/shipbuilding.rs"]
mod tests;
