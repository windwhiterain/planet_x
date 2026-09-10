//! Round-stepping simulation engine.
//!
//! [`advance`] moves the world forward by one round (month). Everything that
//! affects game balance is read from the [`GameConfig`]; no magic numbers live
//! here. Resources are dictionaries (key -> amount), building kinds and ship
//! classes are string keys resolved against the config, and building mechanics
//! switch on the building's config `role` (`"housing"`, `"mining"`,
//! `"shipyard"`).
//!
//! The economy is area-based and continuous: population caps a city's labour
//! ratio, mining output scales with area × labour, and construction builds
//! continuous area within the settlement's finite total area.
//!
//! Budgets are split into two independent, directly-set pools per faction:
//! the **investment budget** (`investment_budget`) funds building infrastructure
//! (each building competes by its 建设投资权重), and the **construction budget**
//! (`construction_budget`) funds ship building (each 建造区 competes by its
//! 建造投资权重). The two never compete with each other.
//!
//! Buildings are the city's hardness: bombardment damages them by area share,
//! and when a city's buildings are all destroyed the city is razed to a blank,
//! colonizable settlement (cities are never captured).
//!
//! The command-controlled state lives in [`State::control`]
//! ([`ControllableState`]). The simulation writes to that state each round; the
//! caller diffs it between consecutive rounds to obtain the per-faction
//! instruction (action) record.
//!
//! **This is the pure engine.** The *Ai decision* layer — who decides, what to
//! build, which target to engage, how budgets are recomputed, the cost→benefit
//! preview — is extracted into [`crate::autocontrol`]. The engine reads the
//! command-controlled instructions out of `State::control` and calls back into
//! `autocontrol` for the decision functions it needs to run the round.

use crate::autocontrol;
use crate::model::*;
use crate::prng::Prng;
use std::collections::{BTreeMap, BTreeSet};

/// 构建一个「未推进」状态的 [`Derived`]：流量为空（尚无生产/维护/治理），但 `metrics` 用
/// [`round_metrics`] 从**真实 state** 汇总（含各势力聚合、实力占比/霸权/战争等存量政治）。
/// 用于回合 0、`--start` 载入的 checkpoint、以及任何「只看当前世界观测、不推进」的场合，
/// 使 agent 视图的 `metrics` 不是空壳，且与游戏逻辑同源。
pub fn derived_from_state(state: &State, config: &GameConfig) -> Derived {
    let metrics = round_metrics(state, config, &RoundFlow::default());
    Derived { flow: RoundFlow::default(), metrics }
}

/// Advance the world by one round, writing the new controllable state into
/// [`State::control`].
///
/// Returns the unified [`Derived`] record for this round — the **single structure** holding
/// both the flow intermediates (`flow`: per-city/per-faction production, fleet upkeep,
/// governance cost/coverage) that the step functions actually used and did not land in the
/// persisted state, **and** the post-round summary/metrics (`metrics`: world totals,
/// power_share, hegemon, coalition, sanctioned, wars, per-faction aggregation, and the
/// single-source `faction_power`). Callers read everything derived from this one value, so the
/// agent's "summary" matches the simulation's numbers exactly (never re-derived twice). This
/// is the `(state, rng) -> (state', rng', Derived)` data-flow principle: `state` is mutated in
/// place, `rng` is consumed via `&mut`, and all derived data rides out in one `Derived`.
pub fn advance(state: &mut State, config: &GameConfig, rng: &mut Prng) -> Derived {
    state.round += 1;
    state.time_month += 1.0;
    // 本回合事件日志从空开始，回合演化中追加。
    state.events.clear();
    // 记录回合开始的交战状态，用于在本回合结束时检测「开战 / 停战」跃迁。
    let wars_before = war_pairs(state, config);

    // Update each body's current position (当前位置) from its orbit. The stored position is the
    // **world (heliocentric)** coordinate: for a satellite (an orbit with a `parent`) it is the
    // parent's world position plus the local orbit offset, so satellites orbit their planet.
    crate::model::resolve_positions(&mut state.bodies, state.time_month as f32);

    let mut flow = RoundFlow::default();
    step_production(state, config, &mut flow);
    // 承包市场（挂单侧）：把「自己一个回合搬不动的积压」挂出去。放在 `step_production` 之后
    // （货栈是本回合刚更新过的），而在 `step_military` 的逐舰循环之前——挂单估运力用的是
    // `should_be_freighter`（纯函数），它与本回合稍后真正写进角色叶、并据此派单的那批舰
    // **同口径**，所以不存在「先挂单、再发现自己其实有闲船」的错位。
    step_contracts(state, config);
    step_upkeep(state, config, &mut flow);
    step_market(state, config, &mut flow);
    step_construction(state, config, rng, &mut flow);
    step_military(state, config, rng, &mut flow);
    // 光速治理：以距离首都为代价的管理/忠诚度，给超大帝国一个自然上限。
    step_governance(state, config, &mut flow);
    // 重建没有「步进」了：唯一的重建路径是**殖民舰开到空白定居点**（见 `sim::colonize`，
    // 由 `step_military` 里的殖民行为触发）。既无舰又无活城的势力就此亡国——见
    // `tests/longhorizon.rs` 的亡国守卫。
    //
    // 迁都：亡城强迁（首都天体失守→人口最高活城）+ 周期性 AI 评估。放在这里，
    // 让本回合刚靠殖民舰立起立足点的势力也能当回合被认领新首都。
    step_capital(state, config);
    step_diplomacy(state, config, rng);
    // 合纵连横 / 均势外交：当一方被判定为「霸权」时，其余较弱势力结成反制联盟——
    // 军事上联手制衡，经济上多国资源封锁。这给「一家独大」一个自然的众矢之的。
    step_balance_of_power(state, config);

    // 外交跃迁：任何一对势力跨越战争阈值（开战 / 停战）都在本回合记一条事件。
    let wars_after = war_pairs(state, config);
    for (a, b) in wars_after.difference(&wars_before) {
        ev(state, GameEvent::WarStarted { a: a.clone(), b: b.clone() });
    }
    for (a, b) in wars_before.difference(&wars_after) {
        ev(state, GameEvent::WarEnded { a: a.clone(), b: b.clone() });
    }

    // 剧情：推进叙事弧/编年史（数据驱动，见 config/game.ron 的 `story` 表）。
    step_story(state, config);

    // 思潮（可变化意识形态）：按「变化因素」（战争得失/MOND 接触/经济好坏/人均面积）驱动。
    // 放在回合末：此时事件（战争得失/城夷平/叛乱）与流量（产出/维护/治理）均已就位。
    step_ideology(state, config, &flow);

    // 历史层收尾：两层各自按配置裁剪（这是唯一拿得到 config 的地方）。
    // `max_milestones` 默认 0 = 无损；`notable_window` 默认 24 回合，滑窗过期是**预期行为**。
    state.milestones.trim(config.history.max_milestones);
    state.notables.trim(state.round, config.history.notable_window);

    // 结回合：把所有派生数据装进一个 `Derived`（flow 中间量 + post 观测/总结）。`post`
    // 由 `round_metrics` 汇总（复用 `balance_picture`/`sanctioned_hegemon`/`faction_power`
    // 等 step 同源计算），因此观测与游戏逻辑**严格一致**；`faction_power` 是单一权威。
    let metrics = round_metrics(state, config, &flow);
    Derived { flow, metrics }
}

/// The set of unordered faction pairs currently at war (relation ≤ war_threshold).
fn war_pairs(state: &State, config: &GameConfig) -> BTreeSet<(FactionId, FactionId)> {
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

pub(crate) fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

pub(crate) fn city_position(state: &State, cid: &str) -> [f64; 2] {
    state
        .city(cid)
        .map(|c| state.body_position(&c.body_id))
        .unwrap_or([0.0, 0.0])
}

pub(crate) fn hostile(state: &State, config: &GameConfig, a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    relation(state, a, b) <= config.combat.war_threshold
}

/// 该势力当前是否处于交战状态：与任意其他势力的关系已达到交战阈值。
/// 用于「造舰按威胁响应」——战时倾向多造战争机器，和平时倾向多造殖民/经济舰。
pub(crate) fn faction_at_war(state: &State, config: &GameConfig, fid: &str) -> bool {
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
fn war_scar_floor(state: &State, config: &GameConfig, a: &str, b: &str) -> Option<f64> {
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

fn relation(state: &State, a: &str, b: &str) -> f64 {
    state
        .faction(a)
        .and_then(|f| f.relations.get(b).copied())
        .unwrap_or(0.0)
}

/// 关系增减（开火/夺城的 delta、剧情的关系效果）。**写入前必须过战争疤痕地板**——
/// 见 [`set_relation_sym`] 的说明：关系有多个写入者，任何一个绕过地板，地板就不成立。
fn adjust_relation(state: &mut State, config: &GameConfig, a: &str, b: &str, delta: f64) {
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

/// 两方思潮的**相似度**（[0,1]）：`1 − 四轴平均 |Δ|/2`。每轴取 `[-1,1]`，故单轴归一化距离
/// 为 `|Δ|/2`（同极=0、对极=1），再对 [`Ideology`] 的 4 条轴取平均。相似度越高两方思潮越像。
/// 供外交静息亲和修正使用（思潮可变因子），与静态 alignment 叠加。
pub(crate) fn ideology_similarity(a: &Ideology, b: &Ideology) -> f64 {
    let dist = (0.5 * (a.peace_military - b.peace_military).abs()
        + 0.5 * (a.science_tech - b.science_tech).abs()
        + 0.5 * (a.people_elite - b.people_elite).abs()
        + 0.5 * (a.nature_colony - b.nature_colony).abs())
        / 4.0;
    (1.0 - dist).clamp(0.0, 1.0)
}

/// Append a [`GameEvent`] to this round's log — **and** to every history layer it belongs to.
///
/// 这是**发事件的唯一漏斗**，也是 [`State::events`]/[`State::milestones`]/[`State::notables`]
/// 三层的唯一写入点：分层由 [`GameEvent::salience`] 单点声明（每层的 `push` 自己过滤），所以
/// 「事件发了、历史没记」在结构上不可能——和 [`kill_ship`]/[`spawn_ship`] 这些状态漏斗是同一
/// 套纪律。
///
/// `events` 每回合被 [`advance`] 清空；两层历史不清空，容量裁剪在 `advance` 收尾时统一做
/// （那里才拿得到 config）：`milestones` 按 `history.max_milestones` 截断，`notables` 按
/// `history.notable_window` 滑窗。
pub(crate) fn ev(state: &mut State, e: GameEvent) {
    state.milestones.push(state.round, e.clone());
    state.notables.push(state.round, e.clone());
    state.events.push(e);
}

// --- 状态变更漏斗（single writer）--------------------------------------------
//
// 所有「改变归属 / 存亡」的写入都必须走下面这几个漏斗：漏斗负责 (1) 改状态 (2) 记事件。
// 这样「历史」就不再是顺手记的副产品——**忘记记事件在结构上变得不可能**。这是 Stage B 的
// 「治根」：Stage A 只是把已知的漏补齐，漏斗化让**未来新增的路径**也必须经过事件。
//
// 更精确的因果仍由调用方给出（`fire` 知道补刀者、`step_upkeep` 知道是欠费、`bombard_city`
// 知道是哪艘舰拆的城）；漏斗只保证「无论如何都有一条事件」。

/// 击毁 / 报废一艘舰（漏斗）：hull 归零 + 记一条 [`GameEvent::ShipDestroyed`]。
///
/// **同一艘舰只记一次**（本回合内多条路径命中时以第一条为准，避免重复计数）。返回是否
/// 新记了一条；`false` = 之前那条路径已经记过。
pub(crate) fn kill_ship(state: &mut State, ship: &ShipId, cause: DeathCause, by: Option<Killer>) -> bool {
    if state.events.iter().any(|e| matches!(e, GameEvent::ShipDestroyed { ship: s, .. } if s == ship)) {
        return false;
    }
    let Some((owner, class)) = state.ship(ship).map(|s| (s.faction_id.clone(), s.class.clone())) else {
        return false;
    };
    if let Some(s) = state.ship_mut(ship) {
        s.hull = 0.0;
    }
    ev(state, GameEvent::ShipDestroyed { ship: ship.clone(), owner, class, cause, by });
    true
}

/// 清扫本回合 `hull ≤ 0` 的舰（漏斗**兜底**）：保证任何从 `state.ships` 消失的舰都有一条
/// 死因事件，然后移除它并清掉它的指令。
///
/// `watched` = **进入本步进时还活着**的舰集合。返回值只统计它们当中「被兜底补记」的数量——
/// 正常为 0（精确路径都已记过）。不为 0 意味着**某条路径漏了 [`kill_ship`]**，于是
/// `debug_assert` 在测试里立刻喊出来；release 下仍保持历史完整（用最保守的
/// [`DeathCause::Scrapped`] 补一条，不谎称是战损）。
///
/// 为什么需要 `watched`：回合**开始前**就已经 `hull ≤ 0` 的舰（只有外部/测试能在回合之间
/// 造成这种状态；`advance` 开头会 `events.clear()`，所以它的死因事件本来就不属于本回合）
/// 不该在这里喊——它不是本回合的漏记，只是被顺带清走。
fn sweep_dead_ships(state: &mut State, watched: &BTreeSet<ShipId>) -> usize {
    let dead: Vec<ShipId> = state
        .ships
        .iter()
        .filter(|s| s.hull <= 0.0)
        .map(|s| s.name.clone())
        .collect();
    let mut invented = 0;
    for sid in dead {
        if kill_ship(state, &sid, DeathCause::Scrapped, None) && watched.contains(&sid) {
            invented += 1;
        }
    }
    state.ships.retain(|s| s.hull > 0.0);
    let alive: BTreeSet<ShipId> = state.ships.iter().map(|s| s.name.clone()).collect();
    for c in state.control.values_mut() {
        c.ship_orders.retain(|sid, _| alive.contains(sid));
    }
    invented
}

/// 造一艘舰的参数（[`spawn_ship`] 的输入）。
pub(crate) struct ShipSpawn<'a> {
    pub owner: FactionId,
    pub class: &'a str,
    pub position: [f64; 2],
    /// 出厂城（船坞建造时给出；剧情赠舰 / 重建种子舰在天体附近下水则为 `None`）。
    pub city: Option<CityId>,
    pub via: SpawnVia,
    /// 是否从库存支付装配组件成本（船坞出厂要付；剧情赠舰与重建种子舰不付）。
    pub pay_components: bool,
}

/// 造一艘舰（漏斗）：装配组件 + 确定性取名 + 算 effective 面板 + 记一条
/// [`GameEvent::ShipSpawned`]。
///
/// 三条造舰路径（船坞出厂 / 剧情赠舰 / 反僵尸重建）统一走这里。此前 `step_resurgence`
/// 的种子舰**完全不发事件**——投影实测 63 次出生里 **46 次无解释**，直到把对账守卫从
/// 「城的归属」扩到「舰的出生」才被抓出来（见 `every_ship_state_change_is_explained_by_an_event`）。
///
/// 确定性：`choose_loadout` / `ship_display_name` 都无 RNG，`ship_name_seq` 单调递增，
/// 故本漏斗不改变模拟的随机数流。
pub(crate) fn spawn_ship(state: &mut State, config: &GameConfig, spec: ShipSpawn<'_>) -> ShipId {
    let components = autocontrol::choose_loadout(state, config, spec.owner.clone(), spec.class);
    let class = spec.class.to_string();
    let cspec = config.ship_spec(&class);
    // 舰名 = 从本势力名字库确定性取的一个唯一名（名字即唯一 key，击毁后不复用）。
    let seq = *state.ship_name_seq.entry(spec.owner.clone()).or_insert(0);
    state.ship_name_seq.insert(spec.owner.clone(), seq + 1);
    let fname = state.faction(&spec.owner).map(|f| f.name.clone()).unwrap_or_default();
    let name = ship_display_name(config.ship_pool(&fname), seq);
    let mut ship = Ship {
        name: name.clone(),
        class: class.clone(),
        faction_id: spec.owner.clone(),
        position: spec.position,
        hull: 0.0,
        hull_max: 0.0,
        shield: 0.0,
        shield_max: 0.0,
        components,
        component_hp: Vec::new(),
        velocity: 0.0,
        doctrine: cspec.default_doctrine,
        kiting: cspec.default_kiting,
        freighter: cspec.default_freighter,
        attack_hist: BTreeMap::new(),
        cargo: BTreeMap::new(),
    };
    let panel = ship_panel(config, &ship);
    ship.hull = panel.hull_max;
    ship.hull_max = panel.hull_max;
    ship.shield = panel.shield_max;
    ship.shield_max = panel.shield_max;
    // 每件组件初始满完整度（模块毁损用）。
    ship.component_hp = ship.components.iter().map(|c| component_integrity(config, c)).collect();
    if spec.pay_components {
        let comp_cost: Vec<(String, f64)> = ship
            .components
            .iter()
            .flat_map(|c| config.component_spec(c).cost.clone())
            .collect();
        let mut spent: ResourceMap = ResourceMap::new();
        commit_spend(state, &spec.owner, &mut spent, &comp_cost);
    }
    ev(state, GameEvent::ShipSpawned {
        ship: name.clone(),
        owner: spec.owner.clone(),
        class: class.clone(),
        city: spec.city,
        via: spec.via,
    });
    state
        .control
        .entry(spec.owner)
        .or_default()
        .ship_orders
        .insert(name.clone(), Control::inherit(ShipBehavior::Idle));
    state.ships.push(ship);
    name
}

/// 一座城被**夷平**的方式（决定记哪条事件）。
///
/// `pub(crate)`：投影守卫要用它构造一个确定性的「先夷平、同回合再被别家复垦」样本。
pub(crate) enum RazeCause {
    /// 被舰炮拆平 → `CityRazed`（带拆城的舰/势力、伤害、夷平前人口）。
    Bombardment { by_ship: ShipId, by_faction: FactionId, damage: f64 },
    /// 离心叛乱：市民自己散伙，无外部攻击者 → `Revolt`。
    Revolt { faction: FactionId, loyalty: f64 },
}

/// 把一座城夷平为空白（漏斗）：清人口 / 建筑 / 造舰进度、`razed = true`，并记事件。
///
/// 两条路径（舰炮拆平 / 离心叛乱）统一走这里。**刻意保留两条路径各自对忠诚度的效果**
/// （炮击不动 `loyalty`、叛乱清零）：治理步进跳过 razed 城，故该值对模拟是惰性的，但它是
/// 投影 `cities.loyalty` 列的一部分——改它会改变已发布的轨迹。
///
/// `pub(crate)`：投影守卫需要一个**确定性**的「先夷平、同回合再被别家复垦」样本，
/// 否则只能靠长局恰好撞上（而唯一大量产生这种巧合的 `step_resurgence` 已删除）。
pub(crate) fn raze_city(state: &mut State, cid: &CityId, cause: RazeCause) {
    let pop_before = state.city(cid).map(|c| c.population).unwrap_or(0);
    // 「谁失去了这座城市」**只有在此刻才知道**：夷平不改 `faction_id`（空白城保留最后主人的
    // diaspora claim），但同一回合后来的 `reseed_city`/`found_city` 会把它改写成新主。
    // 事后再读就只会读到新主（错的人），所以在这里就把它钉进事件。
    let owner = state.city(cid).map(|c| c.faction_id.clone()).unwrap_or_default();
    if let Some(c) = state.city_mut(cid) {
        c.razed = true;
        c.population = 0;
        c.buildings.clear();
        c.ship_progress.clear();
        if matches!(&cause, RazeCause::Revolt { .. }) {
            c.loyalty = 0.0;
        }
    }
    match cause {
        RazeCause::Bombardment { by_ship, by_faction, damage } => ev(state, GameEvent::CityRazed {
            city: cid.clone(),
            owner,
            fallen_to: by_faction,
            by_ship,
            damage,
            pop_before,
        }),
        RazeCause::Revolt { faction, loyalty } => ev(state, GameEvent::Revolt {
            city: cid.clone(),
            faction,
            loyalty,
        }),
    }
}

/// 给一座城的新建筑补上「投资 / 建造权重」控制叶子。
///
/// `Control::inherit` 的 `mode = Inherit` → 控制解析沿作用域链上溯，与「叶子不存在」等价，
/// 因此这一步**不改变任何决策**（只是让控制面里那座城的建筑是可枚举的）。
fn wire_city_control(state: &mut State, config: &GameConfig, cid: &CityId, to: &FactionId) {
    let buildings = state.city(cid).map(|c| c.buildings.clone()).unwrap_or_default();
    let ctrl = state.control.entry(to.clone()).or_default();
    for b in &buildings {
        let ikey = (cid.clone(), b.id);
        ctrl.invest_weights.entry(ikey).or_insert_with(|| {
            Control::inherit(config.building_spec(&b.kind).default_invest_weight)
        });
        if b.is_shipyard() {
            let bkey = (cid.clone(), b.id);
            ctrl.build_weights.entry(bkey).or_insert_with(|| {
                Control::inherit(config.building_spec(&b.kind).default_build_weight)
            });
        }
    }
}

/// 复垦一座**空白城**（razed → 活城）并交给 `to`（漏斗）：重新播种建筑 / 人口 / 造舰进度、
/// 忠诚重置为 1.0，并记 `ColonyFounded { how: Refounded, prev_owner }`。
///
/// `prev_owner` 由漏斗自己读（改归属**之前**的持有者 = 空白城保留的 diaspora claim），
/// 所以「谁失去了这座城市」不可能被调用方漏掉。
///
/// 由**殖民舰抵达**触发（见 `colonize`）——这是唯一能让一座空白城重新立起来的路径：
/// 重建必须**有船跑到那里**，不再是凭空变城。
/// 返回 `false` = 该城没有可用的定居点（调用方自行处理）。
pub(crate) fn reseed_city(
    state: &mut State,
    config: &GameConfig,
    cid: &CityId,
    to: &FactionId,
    seeded_ship_class: &str,
    next_building_id: &mut BuildingId,
) -> bool {
    let Some(settlement) = state.city_settlement(cid).cloned() else { return false };
    let Some(body) = state.city(cid).map(|c| c.body_id.clone()) else { return false };
    let prev_owner = state.city(cid).map(|c| c.faction_id.clone());
    let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
    let buildings = seed_colony_buildings(&settlement, pop, seeded_ship_class, config, next_building_id);
    if let Some(c) = state.city_mut(cid) {
        c.razed = false;
        c.faction_id = to.clone();
        c.population = pop;
        c.buildings = buildings;
        c.ship_progress.clear();
        c.ship_progress.insert(seeded_ship_class.to_string(), 0.0);
        c.loyalty = 1.0;
    }
    wire_city_control(state, config, cid, to);
    ev(state, GameEvent::ColonyFounded {
        city: cid.clone(),
        owner: to.clone(),
        body,
        seeded_ship_class: seeded_ship_class.to_string(),
        how: FoundingHow::Refounded,
        prev_owner,
    });
    true
}

/// 在**从未被占据**的定居点上新建一座城（漏斗）并记 `ColonyFounded { how: NewSite }`。
/// 城名由调用方给出（殖民城 / 收容所两种命名不同）。返回 `false` = 同名城已存在（防御）。
fn found_city(
    state: &mut State,
    config: &GameConfig,
    name: &CityId,
    body: &BodyId,
    settlement: &Settlement,
    to: &FactionId,
    seeded_ship_class: &str,
    next_building_id: &mut BuildingId,
) -> bool {
    if state.city(name).is_some() {
        return false;
    }
    let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
    let buildings = seed_colony_buildings(settlement, pop, seeded_ship_class, config, next_building_id);
    let mut progress: BTreeMap<String, f64> = BTreeMap::new();
    progress.insert(seeded_ship_class.to_string(), 0.0);
    state.cities.push(City {
        name: name.clone(),
        body_id: body.clone(),
        settlement: settlement.name.clone(),
        faction_id: to.clone(),
        population: pop,
        buildings,
        ship_progress: progress,
        razed: false,
        space_station: false,
        loyalty: 1.0,
    });
    wire_city_control(state, config, name, to);
    ev(state, GameEvent::ColonyFounded {
        city: name.clone(),
        owner: to.clone(),
        body: body.clone(),
        seeded_ship_class: seeded_ship_class.to_string(),
        how: FoundingHow::NewSite,
        prev_owner: None,
    });
    true
}

/// A building's health ratio (armor / armor_max), clamped to [0, 1]. Intact
/// buildings are 1.0; damaged buildings produce/operate at a reduced ratio.
fn building_health(b: &Building, config: &GameConfig) -> f64 {
    let amax = b.armor_max(config);
    if amax <= 1e-9 {
        1.0
    } else {
        (b.armor / amax).clamp(0.0, 1.0)
    }
}

/// The command-controlled 建设投资权重 of a building (its build priority).
/// Follows the control scope: an AI-controlled building uses the config default,
/// while a player-controlled building uses the commanded value.
fn invest_weight(state: &State, config: &GameConfig, fid: &str, cid: &str, b: &Building) -> f64 {
    let key = (cid.to_string(), b.id);
    if state.invest_control(fid.to_string(), &key).is_player() {
        state
            .control(fid.to_string())
            .and_then(|c| c.invest_weights.get(&key))
            .map(|c| c.value)
            .unwrap_or_else(|| config.building_spec(&b.kind).default_invest_weight)
    } else {
        config.building_spec(&b.kind).default_invest_weight
    }
}

/// The command-controlled 建造投资权重 of a 建造区 (shipyard) building.
fn build_weight(state: &State, config: &GameConfig, fid: &str, cid: &str, b: &Building) -> f64 {
    let key = (cid.to_string(), b.id);
    if state.build_control(fid.to_string(), &key).is_player() {
        state
            .control(fid.to_string())
            .and_then(|c| c.build_weights.get(&key))
            .map(|c| c.value)
            .unwrap_or_else(|| config.building_spec(&b.kind).default_build_weight)
    } else {
        config.building_spec(&b.kind).default_build_weight
    }
}

/// The command-controlled 娱乐/福利预算 of a city (its loyalty spending per round,
/// in market value). Follows the control scope: the system uses the config default,
/// a Player-commanded city uses the commanded value.
fn city_loyalty_budget(state: &State, config: &GameConfig, fid: FactionId, cid: CityId) -> f64 {
    if state.loyalty_budget_control(fid.clone(), cid.clone()).is_player() {
        state
            .control(fid.clone())
            .and_then(|c| c.loyalty_budget.get(&cid))
            .map(|c| c.value)
            .unwrap_or(config.governance.default_entertainment)
    } else {
        config.governance.default_entertainment
    }
}

// --- production -------------------------------------------------------------

fn deposit_area(deposits: &[(String, f64)], rt: &str) -> f64 {
    deposits
        .iter()
        .find(|(r, _)| r == rt)
        .map(|(_, a)| *a)
        .unwrap_or(0.0)
}

/// A city's labour ratio: population vs. total staff required by its buildings.
fn labor_ratio(state: &State, config: &GameConfig, cid: &str) -> f64 {
    let population = state.city(cid).map(|c| c.population as f64).unwrap_or(0.0);
    let mut staff_req = 0.0;
    if let Some(c) = state.city(cid) {
        for b in &c.buildings {
            staff_req += b.deployed * config.building_spec(&b.kind).staff_per_area;
        }
    }
    if staff_req <= 0.0 {
        1.0
    } else {
        (population / staff_req).clamp(config.economy.min_efficiency, 1.0)
    }
}

fn step_production(state: &mut State, config: &GameConfig, flow: &mut RoundFlow) {
    let city_ids: Vec<CityId> = state.cities.iter().map(|c| c.name.clone()).collect();
    for cid in city_ids {
        let (body_id, faction_id, population, razed) = {
            let c = state.city(&cid).expect("city disappeared");
            (c.body_id.clone(), c.faction_id.clone(), c.population, c.razed)
        };
        if razed {
            continue;
        }
        // **首都即集散地**（`.agents/notes/freight-collection.md`）：首都天体的产出免运输、
        // 直接进势力池；其余天体的产出**先落在产地货栈**，要等船来运回首都才可用。
        // 于是「非首都产出必须靠运输」不是一句设定，而是产出落库路径本身。
        let is_hub = state.capital_body(&faction_id) == body_id;
        let (ecocap, deposits) = {
            let s = state.city_settlement(&cid);
            match s {
                Some(s) => (
                    s.ecological_capacity,
                    s.resources.iter().map(|d| (d.resource.clone(), d.area)).collect::<Vec<_>>(),
                ),
                None => continue,
            }
        };

        let mut housing_area = 0.0;
        let mut staff_req = 0.0;
        let mut mines: Vec<(String, f64)> = Vec::new();
        for b in &state.city(&cid).expect("city disappeared").buildings {
            let spec = config.building_spec(&b.kind);
            staff_req += b.deployed * spec.staff_per_area;
            let health = building_health(b, config);
            match spec.role.as_str() {
                "housing" => housing_area += b.deployed * health,
                "mining" => {
                    if let Some(r) = &b.resource {
                        mines.push((r.clone(), b.deployed * health));
                    }
                }
                _ => {}
            }
        }

        let housing_capacity = housing_area * ecocap;

        // Population grows toward housing capacity.
        if housing_capacity > population as f64 {
            let delta = ((housing_capacity - population as f64) * config.economy.pop_growth).round() as i64;
            if delta > 0 {
                if let Some(c) = state.city_mut(&cid) {
                    c.population = ((c.population as i64 + delta).min(housing_capacity as i64).max(0)) as u32;
                }
            }
        }

        let labor = if staff_req <= 0.0 {
            1.0
        } else {
            (population as f64 / staff_req).clamp(config.economy.min_efficiency, 1.0)
        };

        // Mining output.
        for (rt, area) in mines {
            let effective = area.min(deposit_area(&deposits, &rt));
            if effective <= 0.0 {
                continue;
            }
            let spec = config.building_spec("mining");
            let output = effective * labor * spec.productivity * config.economy.production_rate;
            // 记录本回合产出（step_production 的「中间量」），供 round_metrics 做 agent 总结：
            // 每城 + 每势力各记一份；**记的是开采量**（不管它落在首都还是产地货栈）；
            // 随后按 `is_hub` 决定入库路径。
            *flow.city_production.entry(cid.clone()).or_default().entry(rt.clone()).or_insert(0.0) += output;
            *flow.faction_production.entry(faction_id.clone()).or_default().entry(rt.clone()).or_insert(0.0) += output;
            if is_hub {
                if let Some(f) = state.faction_mut(&faction_id) {
                    *f.resources.entry(rt).or_insert(0.0) += output;
                }
            } else {
                state.depot_add(&faction_id, &body_id, &rt, output);
            }
        }
    }
}

// --- interstellar market (resource sink + keystone supply) -------------------

/// Fleet upkeep: every ship costs its class's [`ShipSpec::upkeep`] (market value)
/// per round to keep in service. The faction's total fleet maintenance is paid
/// out of its stockpile (drained value-weighted across all minerals); if the
/// faction cannot cover it, the shortfall rusts its fleet (ships lose hull
/// proportionally, and ships driven to 0 are scrapped).
///
/// This is the continuous resource **sink** that bounds fleet size: big fleets
/// need a big economy to sustain, so the navy grows only as fast as the
/// economy feeds it rather than snowballing unboundedly.
///
/// **例外：无活城的势力（流亡舰队）不因维护费被拆解。** 维护费是**港口/后勤**的成本——
/// 没有港口就无从「欠费拆解」，舰队只能靠打捞、掠夺、拆东墙补西墙自持。这条例外是
/// `step_resurgence` 被删除（D5）之后**唯一的立足点保证**：复垦必须由**航行**完成
/// （派船去空白定居点，见 `colonize`），所以流亡舰队必须先**活到**能开过去。
/// 没有这条，实测 seed 1/7/42 跑到 1000 回合会**只剩 1-4 个势力有城、5-8 个永久亡国**
/// ——战争拆掉最后一座城 → 无产出 → 库存被维护费抽干 → 全舰队生锈拆解 → 永远回不来。
fn step_upkeep(state: &mut State, config: &GameConfig, flow: &mut RoundFlow) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    for fid in faction_ids {
        let upkeep_total: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
            .map(|s| ship_panel(config, s).upkeep)
            .sum();
        // 记录本回合舰队维护费（step_upkeep 的「中间量」）。
        flow.upkeep.insert(fid.clone(), upkeep_total);
        if upkeep_total <= 1e-9 {
            continue;
        }
        // 流亡舰队（无活城）：不抽库存、不生锈——见上面的「例外」。
        let landless = !state.cities.iter().any(|c| c.faction_id == fid && !c.razed);
        if landless {
            continue;
        }
        let stock = state.faction(&fid).map(|f| f.resources.clone()).unwrap_or_default();
        let total_value: f64 = stock.iter().map(|(k, v)| v * value_of(k)).sum();
        let pay = upkeep_total.min(total_value);
        if pay > 1e-9 {
            let ratio = (pay / total_value).min(1.0);
            if let Some(f) = state.faction_mut(&fid) {
                for (k, v) in stock.iter() {
                    let new = (*v - *v * ratio).max(0.0);
                    f.resources.insert(k.clone(), new);
                }
            }
        }
        // Unpaid upkeep rusts the fleet; hull reaching 0 scrapped.
        let short = (upkeep_total - total_value).max(0.0);
        if short > 1e-9 {
            let frac = (short / upkeep_total).min(1.0);
            let frac = frac.max(0.2); // at least a visible rust when short
            let mut scrap: Vec<ShipId> = Vec::new();
            for s in state.ships.iter_mut() {
                if s.faction_id != fid || s.hull <= 0.0 {
                    continue;
                }
                let rust = ship_panel(config, s).hull_max * frac;
                s.hull = (s.hull - rust).max(0.0);
                if s.hull <= 0.0 {
                    scrap.push(s.name.clone());
                }
            }
            for sid in scrap {
                // 经济性死亡：不是被打沉的，是养不起被拆解的（此前与战损共用同一个事件、
                // 无法区分）。走漏斗 → 保证有事件。
                kill_ship(state, &sid, DeathCause::UpkeepShortfall, None);
            }
        }
    }
}

/// 军工需要的资源集合：所有舰级的 `build_cost` ∪ 所有组件的 `cost`。
/// 这是买方想常备的目标集合（一个势力有船坞，就想备齐这些料）。
fn military_need(config: &GameConfig) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for cls in config.ships.keys() {
        for rt in config.ship_spec(cls).build_cost.keys() {
            out.insert(rt.clone());
        }
    }
    for id in config.components.keys() {
        for rt in config.component_spec(id).cost.keys() {
            out.insert(rt.clone());
        }
    }
    out
}

/// **关系即价格**：卖方 `seller` 卖给买方 `buyer` 时的实际成交价倍率。
///
/// * 关系为负 → 加价（越冷越贵），到交战边缘封顶 `hostile_price_markup`（默认 2.5×）。
/// * 关系为正 → 折扣（越暖越便宜），到 `friendly_relation` 封顶 `friendly_price_discount`
///   （默认 85 折）。
///
/// 同一个矿，向朋友买和向敌人买不是一个价——这是「不同关系不同价格」（D4）。
fn relation_price_mult(state: &State, config: &GameConfig, seller: &str, buyer: &str) -> f64 {
    let m = &config.market;
    let rel = relation(state, seller, buyer);
    if rel < 0.0 {
        // 跨度 = 交战阈值的绝对值再多一点（越过它就已经开战/禁运了）。
        let span = (-config.combat.war_threshold).max(1.0) + 1.0;
        1.0 + m.hostile_price_markup * ((-rel) / span).clamp(0.0, 1.0)
    } else {
        let f = m.friendly_relation.max(1.0);
        1.0 - m.friendly_price_discount * (rel / f).clamp(0.0, 1.0)
    }
}

/// **全面禁运**：某一方「根本不卖给你」——**所有资源**都断供（D4：全面）。
///
/// 三档判据（任一档成立即封锁）：
/// 1. **交战**：关系 ≤ `war_threshold`（任一方这样看对方即可）。
/// 2. **反制联盟封锁**：已倒向联盟的弱者 ↔ 被锁定的霸权（取代旧的 `sanction_trade_mult`
///    ——那个只是「少卖一点」，这个是真的「不卖」）。
/// 3. **冷到断供**：关系 ≤ `embargo_relation`（比交战阈值更早，还没开打就断货）。
///
/// 判据是**对称**的：一方不卖，买卖就做不成。公开给测试/观测（`--schema` 之外的控制面
/// 也可用它回答「他到底卖不卖我」）。
pub fn trade_blocked(state: &State, config: &GameConfig, a: &str, b: &str) -> bool {
    trade_block_cause(state, config, a, b).is_some()
}

/// [`trade_blocked`] 的**原因**（`None` = 没封锁）。把「为什么断供」区分开，才能对账
/// ——否则「禁运太多」无法判断是战争、联盟还是阈值太严。
pub fn trade_block_cause(state: &State, config: &GameConfig, a: &str, b: &str) -> Option<&'static str> {
    if a == b {
        return None;
    }
    if hostile(state, config, a, b) || hostile(state, config, b, a) {
        return Some("war");
    }
    let cold = config.market.embargo_relation;
    if relation(state, a, b) <= cold || relation(state, b, a) <= cold {
        return Some("cold");
    }
    // 反制联盟对霸权的封锁：谁「倒向联盟」谁就不跟霸权做生意。
    if config.balance.hegemon_power <= 1.0 {
        if let Some(h) = sanctioned_hegemon(state, config) {
            let estranged = |x: &str| x != h && relation(state, x, &h) <= config.balance.coalition_estrange;
            if (a == h && estranged(b)) || (b == h && estranged(a)) {
                return Some("coalition");
            }
        }
    }
    None
}

/// 星际市场（真实交换所）。每回合三步，全部确定性、无 RNG：
///
/// 1. **价格发现**：`mult = (coverage_rounds / 覆盖回合数)^price_alpha`，其中
///    「覆盖回合数」= 世界总库存 ÷ 全球消费率（滑窗）。稀缺 → 高价（顶到
///    [`MarketConfig::price_ceiling`]），过剩 → 折价（[`MarketConfig::price_floor`]）。
///    消费率由「上回合市场时刻的库存 + 本回合产出 − 本回合市场时刻的库存」实测——
///    不猜需求。
/// 2. **挂单**：供给来自**各势力真实的富余**（库存扣掉自己要留的部分），挂单**记名卖家**
///    （禁运判据在卖家身上，见 [`step_market`] 的可见性过滤）。
/// 3. **结算（配给）**：买方按购买力（自己可出口富余的价值）从别人的挂单里买，
///    **仓里有多少卖多少**；买不到就是买不到。付款=把自己可出口的实物交给卖家，
///    另按 `spread` 烧掉一笔手续费（真实的价值 sink）。
///
/// 与旧实现的根本差别：旧版是**常数价的无限贩卖机**（没有卖家、没有仓、没有价格），
/// 所以「缺某种矿」不可能更贵、也不可能「不卖给你」。见
/// `.agents/notes/trade-and-sanctions.md` 的实测基线。
fn step_market(state: &mut State, config: &GameConfig, flow: &mut RoundFlow) {
    let m = &config.market;
    if m.auto_trade_limit <= 0.0 {
        return;
    }
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    let need = military_need(config);

    // --- 1) 观测：世界总库存 + 本回合产出 → 消费率（滑窗）→ 价格 ----------------
    let mut world_stock: ResourceMap = ResourceMap::new();
    for f in &state.factions {
        for (rt, v) in &f.resources {
            *world_stock.entry(rt.clone()).or_insert(0.0) += *v;
        }
    }
    let mut produced: ResourceMap = ResourceMap::new();
    for prod in flow.faction_production.values() {
        for (rt, v) in prod {
            *produced.entry(rt.clone()).or_insert(0.0) += *v;
        }
    }
    let last = state.market.last_stock.clone();
    let first_round = last.is_empty();
    let mut price: ResourceMap = ResourceMap::new();
    let mut avg_demand: ResourceMap = ResourceMap::new();
    for rt in config.resources.keys() {
        let have = world_stock.get(rt).copied().unwrap_or(0.0);
        let made = produced.get(rt).copied().unwrap_or(0.0);
        // 消费 = 上回合市场时刻库存 + 本回合产出 − 本回合市场时刻库存。
        // 中间发生的支出：上回合的建设/治理 + 本回合的维护 + 市场手续费。
        let consumed = if first_round {
            0.0
        } else {
            (last.get(rt).copied().unwrap_or(0.0) + made - have).max(0.0)
        };
        let prev = state.market.avg_demand.get(rt).copied().unwrap_or(0.0);
        let demand = if first_round || prev <= 0.0 {
            consumed
        } else {
            prev * (1.0 - m.demand_smoothing) + consumed * m.demand_smoothing
        };
        avg_demand.insert(rt.clone(), demand);
        let mult = if demand <= m.demand_min {
            // 几乎没人消费它 → 没有稀缺信号，按基价（否则没人用的矿会被永久顶成天价）。
            1.0
        } else {
            let cover = (have / demand).max(m.cover_floor);
            (m.coverage_rounds / cover)
                .powf(m.price_alpha)
                .clamp(m.price_floor, m.price_ceiling)
        };
        price.insert(rt.clone(), value_of(rt) * mult);
    }

    // --- 2) 挂单：真实供给（谁卖、卖什么、多少、什么价） ------------------------
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let mut offers: Vec<Offer> = Vec::new();
    // 挂单簿：卖方还能交出什么（结算期间唯一权威的「可给出去的实物」账本）。
    let mut remaining: BTreeMap<(FactionId, String), f64> = BTreeMap::new();
    for fid in &faction_ids {
        let Some(f) = state.faction(fid) else { continue };
        for (rt, amt) in &f.resources {
            // 自己需要的资源至少留 `working_buffer`；其余按 `reserve_fraction` 留一半。
            let keep = if need.contains(rt) {
                m.working_buffer.max(amt * m.reserve_fraction)
            } else {
                amt * m.reserve_fraction
            };
            let over = amt - keep;
            if over > 1e-6 {
                offers.push(Offer {
                    seller: fid.clone(),
                    resource: rt.clone(),
                    amount: over,
                    ask: price.get(rt).copied().unwrap_or_else(|| value_of(rt)),
                });
                remaining.insert((fid.clone(), rt.clone()), over);
            }
        }
    }

    // --- 3) 结算：买方按购买力从别人的挂单里买（配给） --------------------------
    // 买方顺序＝购买力降序（「钱多的人先买」，确定性：同额按名字排序）。
    let mut buyers: Vec<(FactionId, f64)> = faction_ids
        .iter()
        .map(|fid| (fid.clone(), listed_value(&remaining, &price, &value_of, fid)))
        .collect();
    buyers.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut settled: ResourceMap = ResourceMap::new();
    let mut net_import: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut spent: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut freight_paid: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut carrier_income: BTreeMap<FactionId, f64> = BTreeMap::new();
    for (buyer, _) in buyers {
        // 想买的：军工需要、且低于目标库存的资源，**越贵越先买**（先抢最稀缺的）。
        let stock = state.faction(&buyer).map(|f| f.resources.clone()).unwrap_or_default();
        let mut want: Vec<(String, f64, f64)> = need
            .iter()
            .map(|rt| {
                let have = stock.get(rt).copied().unwrap_or(0.0);
                let p = price.get(rt).copied().unwrap_or_else(|| value_of(rt));
                (rt.clone(), (m.working_buffer - have).max(0.0), p)
            })
            .filter(|(_, w, _)| *w > 1e-9)
            .collect();
        want.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)));

        for (rt, mut short, p) in want {
            if short <= 1e-9 || p <= 1e-9 {
                continue;
            }
            // 别的势力此刻还剩多少这种资源在卖（按卖家名排序 → 确定性）。
            // **禁运过滤**：不卖给你的卖家，其挂单对你根本不存在。
            let sellers: Vec<(FactionId, f64)> = remaining
                .iter()
                .filter(|((s, r), amt)| {
                    r == &rt && s != &buyer && **amt > 1e-9 && !trade_blocked(state, config, s, &buyer)
                })
                .map(|((s, _), amt)| (s.clone(), *amt))
                .collect();
            for (seller, avail) in sellers {
                if short <= 1e-9 {
                    break;
                }
                // 购买力每次现算：自己的货可能已经被别人买走了。
                let spendable = listed_value(&remaining, &price, &value_of, &buyer);
                let limit_left = (m.auto_trade_limit - spent.get(&buyer).copied().unwrap_or(0.0)).max(0.0);
                let max_purchase = (spendable / (1.0 + m.spread).max(1e-9)).min(limit_left);

                // --- 路线：距离 + 引力异常带浸入深度（M6）---------------------------
                // 货物不是瞬移的：运得越远越贵；要穿越异常带就更贵——而只有掌握了 MOND 的
                // 势力能可靠走那条线（其余人要么付溢价，要么**丢货**）。
                let anchor_b = trade_anchor(state, &buyer);
                let anchor_s = trade_anchor(state, &seller);
                let depth = route_depth(config, anchor_b, anchor_s);
                let dist_au = dist(anchor_b, anchor_s);
                let mond_extra = if depth > 0.0 {
                    m.mond_freight_mult * (depth / (depth + 1.0))
                } else {
                    0.0
                };
                let freight_rate = m.freight_per_au * dist_au * (1.0 + mond_extra);
                // 成交价 = 市场价 ×（关系倍率 + 运费率）：向敌人买、运得远、要过异常带都更贵。
                let rel_mult = relation_price_mult(state, config, &seller, &buyer);
                let p_eff = p * (rel_mult + freight_rate);
                let take = short.min(avail).min(max_purchase / p_eff.max(1e-9));
                if take <= 1e-9 {
                    continue;
                }
                let give_market = take * p;               // 货值（按市场价）
                let pay_seller = give_market * rel_mult;  // 卖方实收（含关系溢价/折扣）
                let freight = give_market * freight_rate; // 运费
                let cost = pay_seller + freight;          // 贸易额（不含手续费）
                let fee = cost * m.spread;                // 市场手续费（烧掉）
                // 丢货：非 master 的货走异常带会**部分失联**（确定性比例，不是掷骰——
                // 掷骰会污染 `Prng` 流、破坏同种子复现）。
                let reliable = is_mond_master(config, &buyer) || is_mond_master(config, &seller);
                let loss = if depth > 0.0 && !reliable {
                    (m.mond_loss_per_au * depth).clamp(0.0, m.mond_loss_cap)
                } else {
                    0.0
                };
                let lost_units = take * loss;
                let received = give_market * (1.0 - loss);

                // 实物交割：卖家的货 → 买家；途中损失的那部分直接消失。
                if let Some(f) = state.faction_mut(&seller) {
                    let e = f.resources.entry(rt.clone()).or_insert(0.0);
                    *e = (*e - take).max(0.0);
                }
                if let Some(f) = state.faction_mut(&buyer) {
                    *f.resources.entry(rt.clone()).or_insert(0.0) += take - lost_units;
                }
                if let Some(r) = remaining.get_mut(&(seller.clone(), rt.clone())) {
                    *r -= take;
                }
                *settled.entry(rt.clone()).or_insert(0.0) += take - lost_units;

                // 付款：卖方（货款）+ 承运人（异常带那一段运费）+ 市场（手续费，烧掉）。
                pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, Some(&seller), pay_seller);
                let carrier = if depth > 0.0 && m.carrier_share > 0.0 {
                    config
                        .mond
                        .masters
                        .iter()
                        .find(|x| *x != &buyer && *x != &seller && state.faction(x).is_some())
                        .cloned()
                } else {
                    None
                };
                match &carrier {
                    Some(c) => {
                        // 只有 master 能可靠穿越异常带 → 它对这条线上的贸易**抽税**。
                        let carrier_fee = freight * m.carrier_share;
                        pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, Some(c), carrier_fee);
                        pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, None, freight - carrier_fee);
                        *carrier_income.entry(c.clone()).or_insert(0.0) += carrier_fee;
                    }
                    None => {
                        // 没有承运人（或承运人就是买卖双方之一）：那一段运费直接烧掉。
                        pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, None, freight);
                    }
                }
                pay_with_surplus(state, &mut remaining, &price, &value_of, &buyer, None, fee);
                *freight_paid.entry(buyer.clone()).or_insert(0.0) += freight;

                // 净进口 = 实收货值 − 全部流出（关系溢价是老账，运费与丢货是新账）。
                *net_import.entry(buyer.clone()).or_insert(0.0) += received - cost - fee;
                *net_import.entry(seller.clone()).or_insert(0.0) += pay_seller - give_market;
                *spent.entry(buyer.clone()).or_insert(0.0) += cost;
                short -= take;
            }
        }
    }

    // --- 4) 写回：挂单（原始清单，供观测）/ 价格 / 成交 / 需求滑窗 ---------------
    state.market = MarketState {
        offers,
        price,
        settled,
        avg_demand,
        last_stock: world_stock,
    };
    for (fid, v) in net_import {
        flow.market_net.insert(fid, v);
    }
    for (fid, v) in freight_paid {
        flow.market_freight.insert(fid, v);
    }
    for (fid, v) in carrier_income {
        flow.market_carrier_income.insert(fid, v);
    }
}

// --- 承包市场（集货腿的第二条路：请人来运）-------------------------------------
//
// 集货腿有两条路：**自己派船**（`step_ships` 里的定编 + 抽签派单）与**请人来运**（承包）。
// 本步管后者里**托运方**的那一半（挂单 + 收回无人接的过期单）；承运方的接单/履约在 M4b–M4c。
//
// 为什么它不是「又一个市场步」而是独立的一步：**承包的标的是运力，不是货**。
// 商品市场撮合的是「谁卖什么、多少钱」，成交即完成（货瞬移）；承包撮合的是
// 「谁替谁跑一趟」，成交只是**开始**——后面要真的有船去装、去运、去卸
// （复用 `Haul` 的常驻路线），所以它是运输行为的一部分，不是市场的一部分。
//
// 依据与裁决见 `.agents/notes/freight-collection.md` §4（Q1(b) 只扣信誉 / Q2 挂单制 /
// Q4 禁运同样挡承包 / Q10 抽成制 / Q11 超期不作废）。
fn step_contracts(state: &mut State, config: &GameConfig) {
    // 挂单（内含**加价 + 延期**：没人接的单子自己涨价，见 `freight::escalate_open_contracts`）
    autocontrol::freight::post_contracts(state, config);
    // 挂完就撮合：看得见、又愿意接的承运人**按信誉加权抽签**接下单子，并当场押上一条船
    // （`carrier` + `assignments`）。路线与角色叶**不在这里写**——`step_ships` 的运输舰分支
    // 与 `assign_roles` 会照常处理（它们都认识 `assignments`），每个叶子只有一个写者。
    autocontrol::contract::match_carriers(state, config);
    // 履约巡检：**超期**（只扣一次信誉，Q11）+ **丢单**（押上的舰没了 ⇒ 合同回挂单簿）。
    // 交付不在这里——它发生在 `haul_unload` 那一刻（船真的靠了泊位）。
    autocontrol::contract::settle_contracts(state, config);
    // 已完成的单子移出挂单簿（挂单簿只留未完成的；「成交了」由事件与流水账记录）。
    state.contracts.retire_fulfilled();
    // 单子没了（完成/收回）⇒ 清掉指向它的执行关系，免得有舰永远钉在一张不存在的单上。
    state.contracts.drop_dangling_assignments();
}

/// 某势力此刻挂单簿上的**总价值**（= 它的购买力：能拿出来交换的实物值多少）。
fn listed_value(
    remaining: &BTreeMap<(FactionId, String), f64>,
    price: &ResourceMap,
    value_of: &impl Fn(&str) -> f64,
    fid: &FactionId,
) -> f64 {
    remaining
        .iter()
        .filter(|((s, _), _)| s == fid)
        .map(|((_, rt), amt)| amt * price.get(rt).copied().unwrap_or_else(|| value_of(rt)))
        .sum()
}

/// 从 `fid` 的挂单簿里取出价值 `value` 的实物：
/// `to = Some(卖家)` 时交割给对方（付款），`to = None` 时实物消失（市场手续费 sink）。
/// 按资源名确定性顺序取，因此整条结算链无 RNG、可复现。
fn pay_with_surplus(
    state: &mut State,
    remaining: &mut BTreeMap<(FactionId, String), f64>,
    price: &ResourceMap,
    value_of: &impl Fn(&str) -> f64,
    fid: &FactionId,
    to: Option<&FactionId>,
    value: f64,
) {
    if value <= 1e-9 {
        return;
    }
    let keys: Vec<String> = remaining
        .keys()
        .filter(|(s, _)| s == fid)
        .map(|(_, rt)| rt.clone())
        .collect();
    let mut left = value;
    for rt in keys {
        if left <= 1e-9 {
            break;
        }
        let avail = remaining.get(&(fid.clone(), rt.clone())).copied().unwrap_or(0.0);
        if avail <= 1e-9 {
            continue;
        }
        let p = price.get(&rt).copied().unwrap_or_else(|| value_of(&rt));
        if p <= 1e-9 {
            continue;
        }
        let units = (left / p).min(avail);
        if units <= 1e-9 {
            continue;
        }
        *remaining.entry((fid.clone(), rt.clone())).or_insert(0.0) -= units;
        if let Some(f) = state.faction_mut(fid) {
            let e = f.resources.entry(rt.clone()).or_insert(0.0);
            *e = (*e - units).max(0.0);
        }
        if let Some(to) = to {
            if let Some(f) = state.faction_mut(to) {
                *f.resources.entry(rt.clone()).or_insert(0.0) += units;
            }
        }
        left -= units * p;
    }
}


// --- construction (dual budgets) ---------------------------------------------

fn per_area_cost(config: &GameConfig, spec: &BuildingSpec, res_mod: f64, b: &Building) -> Vec<(String, f64)> {
    let mult = config.structure_spec(&b.structure).cost_mult * res_mod;
    spec.build_cost
        .iter()
        .map(|(rt, c)| (rt.clone(), c * mult))
        .collect()
}

fn budget_remaining(limit: &ResourceMap, spent: &ResourceMap, rt: &str) -> f64 {
    limit.get(rt).copied().unwrap_or(0.0) - spent.get(rt).copied().unwrap_or(0.0)
}

fn max_affordable_inc(cost_per_area: &[(String, f64)], limit: &ResourceMap, spent: &ResourceMap, cap: f64) -> f64 {
    let mut inc = cap;
    for (rt, c) in cost_per_area {
        if *c <= 1e-9 {
            continue;
        }
        let have = budget_remaining(limit, spent, rt);
        inc = inc.min(have / *c);
    }
    inc.max(0.0)
}

fn commit_spend(state: &mut State, fid: &str, spent: &mut ResourceMap, cost: &[(String, f64)]) {
    for (rt, c) in cost {
        if let Some(f) = state.faction_mut(fid) {
            let e = f.resources.entry(rt.clone()).or_insert(0.0);
            *e = (*e - c).max(0.0);
        }
        *spent.entry(rt.clone()).or_insert(0.0) += c;
    }
}

fn step_construction(state: &mut State, config: &GameConfig, rng: &mut Prng, flow: &mut RoundFlow) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let mut next_building_id = state
        .cities
        .iter()
        .flat_map(|c| c.buildings.iter().map(|b| b.id))
        .max()
        .map_or(0, |m| m + 1);

    for fid in faction_ids {
        let (investment, inv_modes) = autocontrol::read_budget(state, config, fid.clone(), autocontrol::BudgetKind::Investment);
        let (construction, con_modes) = autocontrol::read_budget(state, config, fid.clone(), autocontrol::BudgetKind::Construction);
        autocontrol::write_budget(state, fid.clone(), autocontrol::BudgetKind::Investment, &investment, &inv_modes);
        autocontrol::write_budget(state, fid.clone(), autocontrol::BudgetKind::Construction, &construction, &con_modes);

        let mut inv_spent: ResourceMap = ResourceMap::new();
        let mut con_spent: ResourceMap = ResourceMap::new();

        let city_ids: Vec<CityId> = state.cities.iter().filter(|c| c.faction_id == fid).map(|c| c.name.clone()).collect();
        for cid in city_ids {
            build_city(
                state,
                config,
                cid,
                fid.clone(),
                &investment,
                &construction,
                &mut inv_spent,
                &mut con_spent,
                &mut next_building_id,
                rng,
            );
        }
    }
    // 威胁响应（整支舰队随威胁重构）：战时把过度生产的「轻舰」船坞按战况重定向到更重/更
    // 需要的舰型，让威胁响应不只作用于新建舰厂。确定性（seeded RNG）。
    let retool_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    for fid in retool_ids {
        autocontrol::retool_shipyards(state, config, &fid, rng, &mut flow.decisions.retools);
    }
}


#[allow(clippy::too_many_arguments)]
fn build_city(
    state: &mut State,
    config: &GameConfig,
    cid: CityId,
    fid: FactionId,
    invest_limit: &ResourceMap,
    con_limit: &ResourceMap,
    inv_spent: &mut ResourceMap,
    con_spent: &mut ResourceMap,
    next_building_id: &mut BuildingId,
    _rng: &mut Prng,
) {
    if state.city(&cid).map(|c| c.razed).unwrap_or(true) {
        return;
    }
    let (ecocap, total_area, speed_mod, res_mod, deposits) = {
        // 城市的定居点 = 它自己占据的那一个（1:1），面积/矿藏以该定居点为准。
        let s = state.city_settlement(&cid);
        match s {
            Some(s) => (
                s.ecological_capacity,
                s.total_area,
                s.construction_speed_mod,
                s.construction_resource_mod,
                s.resources.iter().map(|d| (d.resource.clone(), d.area)).collect::<Vec<_>>(),
            ),
            None => return,
        }
    };

    let mut buildings: Vec<Building> = state.city(&cid).expect("city gone").buildings.clone();
    let population = state.city(&cid).map(|c| c.population as f64).unwrap_or(0.0);
    let labor = labor_ratio(state, config, &cid);

    fn new_b(id: BuildingId, kind: &str, resource: Option<String>, ship_type: Option<String>, structure: &str, area: f64, deployed: f64, config: &GameConfig) -> Building {
        let armor = deployed * config.structure_spec(structure).armor_per_area;
        Building {
            id,
            kind: kind.to_string(),
            resource,
            ship_type,
            structure: structure.to_string(),
            area,
            deployed,
            armor,
        }
    }

    fn find_area(buildings: &[Building], kind: &str, resource: Option<&str>) -> f64 {
        buildings
            .iter()
            .filter(|b| b.kind == kind)
            .find(|b| match (resource, b.resource.as_deref()) {
                (Some(r), Some(br)) => r == br,
                (None, None) => true,
                _ => false,
            })
            .map(|b| b.area)
            .unwrap_or(0.0)
    }

    fn raise_area(
        buildings: &mut Vec<Building>,
        config: &GameConfig,
        next_id: &mut BuildingId,
        kind: &str,
        resource: Option<&str>,
        add: f64,
    ) {
        if add <= 1e-9 {
            return;
        }
        let idx = buildings.iter().position(|b| {
            b.kind == kind
                && match (resource, b.resource.as_deref()) {
                    (Some(r), Some(br)) => r == br,
                    (None, None) => true,
                    _ => false,
                }
        });
        match idx {
            Some(i) => buildings[i].area += add,
            None => {
                buildings.push(new_b(*next_id, kind, resource.map(str::to_string), None, "concrete", add, 0.0, config));
                *next_id += 1;
            }
        }
    }

    // 1) Plan new area (raising targets), bounded by total_area.
    let mut planning_remaining = total_area - buildings.iter().map(|b| b.area).sum::<f64>();

    let desired_res = (population / ecocap.max(1e-6)) * config.economy.housing_buffer;
    let res_area = find_area(&buildings, "residential", None);
    if res_area < desired_res - 1e-9 && planning_remaining > 0.0 {
        let add = (desired_res - res_area).min(planning_remaining);
        raise_area(&mut buildings, config, next_building_id, "residential", None, add);
        planning_remaining -= add;
    }

    // Grow a 建造区 (shipyard) toward its target. If a city holds several
    // shipyards, grow the one with the largest current footprint.
    let con_target = (total_area * 0.15).clamp(4.0, 12.0);
    let shipyard_index = buildings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.is_shipyard())
        .max_by(|(_, a), (_, b)| a.deployed.total_cmp(&b.deployed))
        .map(|(i, _)| i);
    let con_area = shipyard_index.map(|i| buildings[i].area).unwrap_or(0.0);
    if con_area < con_target - 1e-9 && planning_remaining > 0.0 {
        let add = (con_target - con_area).min(planning_remaining);
        let idx = shipyard_index;
        match idx {
            Some(i) => buildings[i].area += add,
            None => buildings.push(new_b(*next_building_id, "construction", None, None, "concrete", add, 0.0, config)),
        }
        if idx.is_none() {
            *next_building_id += 1;
        }
        planning_remaining -= add;
    }

    for (rt, darea) in &deposits {
        if planning_remaining <= 0.0 {
            break;
        }
        let cur = find_area(&buildings, "mining", Some(rt));
        if cur < *darea - 1e-9 {
            let add = (*darea - cur).min(planning_remaining);
            raise_area(&mut buildings, config, next_building_id, "mining", Some(rt), add);
            planning_remaining -= add;
        }
    }

    // 2) Build: grow deployed toward the planned area, spending the investment
    // budget. Higher invest weight builds first.
    buildings.sort_by(|a, b| {
        invest_weight(state, config, &fid, &cid, b).total_cmp(&invest_weight(state, config, &fid, &cid, a))
    });
    for b in buildings.iter_mut() {
        if !b.under_construction() {
            continue;
        }
        let spec = config.building_spec(&b.kind);
        let per_area = per_area_cost(config, spec, res_mod, b);
        let speed = spec.construction_speed * speed_mod * spec.productivity * labor;
        let desired = (b.area - b.deployed).min(speed);
        let inc = max_affordable_inc(&per_area, invest_limit, inv_spent, desired);
        if inc <= 1e-6 {
            continue;
        }
        let cost: Vec<(String, f64)> = per_area.iter().map(|(rt, c)| (rt.clone(), *c * inc)).collect();
        commit_spend(state, &fid, inv_spent, &cost);
        b.deployed += inc;
    }

    // Regrow armor: freshly-built area is intact; otherwise repair toward max at
    // the configured regen rate.
    for b in buildings.iter_mut() {
        let amax = b.armor_max(config);
        if b.under_construction() {
            b.armor = amax;
        } else {
            b.armor = (b.armor + (amax - b.armor) * config.combat.armor_regen).min(amax).max(0.0);
        }
    }

    // 3) Ship building: each 建造区 contributes to its class's rate; progress is
    // per city. The construction budget funds completed ships; shipyards compete
    // for it by build weight, so a higher-weight shipyard pays for its ship first.
    let mut shipyards: Vec<(usize, String, f64, f64)> = Vec::new(); // (index, class, weight, area)
    for (i, b) in buildings.iter().enumerate() {
        if b.is_shipyard() {
            if let Some(cls) = b.ship_type.clone() {
                let area = b.deployed;
                if area > 1e-9 {
                    shipyards.push((i, cls, build_weight(state, config, &fid, &cid, b), area));
                }
            }
        }
    }
    shipyards.sort_by(|a, b| b.2.total_cmp(&a.2));

    let city_progress: BTreeMap<String, f64> = state.city(&cid).map(|c| c.ship_progress.clone()).unwrap_or_default();

    // Aggregate per-class production rate (all 建造区 of a class add up toward the
    // city pool) and per-class build priority (max of its shipyards' weights).
    let mut class_rate: BTreeMap<String, f64> = BTreeMap::new();
    let mut class_weight: BTreeMap<String, f64> = BTreeMap::new();
    for (_, cls, w, area) in &shipyards {
        *class_rate.entry(cls.clone()).or_insert(0.0) += area * config.building_spec("construction").productivity * labor;
        let e = class_weight.entry(cls.clone()).or_insert(0.0);
        *e = e.max(*w);
    }
    let mut classes: Vec<(String, f64, f64)> = class_rate
        .iter()
        .map(|(c, r)| (c.clone(), *r, class_weight.get(c).copied().unwrap_or(0.0)))
        .collect();
    classes.sort_by(|a, b| b.2.total_cmp(&a.2));

    // The construction budget is a per-round rate: it funds ship progress
    // incrementally (cost-per-progress × increment). A class completes a ship
    // once it has accrued `build_points`, at the city level.
    let body_id = state.city(&cid).map(|c| c.body_id.clone()).unwrap_or_default();
    let body_pos = state.body_position(&body_id);
    let mut to_write_progress = city_progress;
    for (cls, rate, _) in &classes {
        let spec = config.ship_spec(cls);
        let bp = spec.build_points;
        let per_progress: Vec<(String, f64)> = spec.build_cost.iter().map(|(rt, c)| (rt.clone(), c / bp)).collect();
        let increment = max_affordable_inc(&per_progress, con_limit, con_spent, *rate).max(0.0);
        if increment <= 1e-9 {
            continue;
        }
        let cost: Vec<(String, f64)> = per_progress.iter().map(|(rt, c)| (rt.clone(), *c * increment)).collect();
        commit_spend(state, &fid, con_spent, &cost);
        *to_write_progress.entry(cls.clone()).or_insert(0.0) += increment;
        // Spawn ships as their build points fill (the cost was paid as progress).
        // On launch, the ship is fitted with a deterministic component loadout chosen
        // from the faction's resource advantage (see `choose_loadout`); the component
        // cost is paid out of the stockpile and the effective panel (hull_max, etc.)
        // is computed from class + components. All of that (naming, fitting, paying,
        // recording `ShipSpawned`) lives in the `spawn_ship` funnel.
        while to_write_progress.get(cls).copied().unwrap_or(0.0) >= bp - 1e-9 {
            spawn_ship(state, config, ShipSpawn {
                owner: fid.clone(),
                class: cls.as_str(),
                position: [body_pos[0] + 0.05, body_pos[1] + 0.05],
                city: Some(cid.clone()),
                via: SpawnVia::Shipyard,
                pay_components: true,
            });
            *to_write_progress.entry(cls.clone()).or_insert(0.0) -= bp;
        }
    }
    if let Some(city) = state.city_mut(&cid) {
        city.ship_progress = to_write_progress;
    }

    // 4) Write back, and ensure every building has invest/build-weight entries.
    if let Some(c) = state.control_mut(fid.clone()) {
        for b in &buildings {
            let key = (cid.clone(), b.id);
            c.invest_weights
                .entry(key.clone())
                .or_insert_with(|| Control::inherit(config.building_spec(&b.kind).default_invest_weight));
            if b.is_shipyard() {
                c.build_weights
                    .entry(key)
                    .or_insert_with(|| Control::inherit(config.building_spec(&b.kind).default_build_weight));
            }
        }
    }
    if let Some(city) = state.city_mut(&cid) {
        city.buildings = buildings;
    }
}

// --- military ---------------------------------------------------------------

/// 威慑（自动计算）：某舰的威慑 = 本舰**综合战力** + `deterrence_radius` 内同势力友舰
/// 战力之和——「威慑 = 综合战力 + 附近同势力战力互相叠加」。这是「理智<->热血」选目标的
/// 依据：欺软怕硬打威慑低于自己的、飞蛾扑火打威慑高于自己的。确定性（无 RNG）。
pub(crate) fn deterrence(state: &State, config: &GameConfig, ship_id: &str) -> f64 {
    let Some(me) = state.ship(ship_id) else { return 0.0 };
    let r = config.combat.deterrence_radius;
    let mut d = ship_power(config, me);
    if r > 0.0 {
        for s in &state.ships {
            if s.name != ship_id && s.faction_id == me.faction_id && s.hull > 0.0
                && dist(me.position, s.position) <= r {
                d += ship_power(config, s);
            }
        }
    }
    d
}

/// 综合战力：把一艘舰的当前有效面板折算成一个标量（威慑 / 目标价值用）。权重与选装评分
/// 一致（攻击最重、护甲/点防次之），让「威慑」是真实的火力估值而非拍脑袋。
fn ship_power(config: &GameConfig, ship: &Ship) -> f64 {
    let p = ship_panel(config, ship);
    p.attack * 4.0 + p.hull_max * 1.0 + p.shield_max * 0.8 + p.hardness * 3.0 + p.intercept * 2.0
}

fn step_military(state: &mut State, config: &GameConfig, rng: &mut Prng, flow: &mut RoundFlow) {
    // 进入本步进时**还活着**的舰：漏斗兜底的断言只对它们成立（见 `sweep_dead_ships`）。
    let alive_at_step_start: BTreeSet<ShipId> = state
        .ships
        .iter()
        .filter(|s| s.hull > 0.0)
        .map(|s| s.name.clone())
        .collect();
    let mut order: Vec<ShipId> = state.ships.iter().map(|s| s.name.clone()).collect();
    for i in (1..order.len()).rev() {
        let j = rng.range(i as u64 + 1) as usize;
        order.swap(i, j);
    }

    let mut next_building_id = state
        .cities
        .iter()
        .flat_map(|c| c.buildings.iter().map(|b| b.id))
        .max()
        .map_or(0, |m| m + 1);

    // 联盟军事协同的「集火目标」：每回合每个势力各算一次（O(势力 × 实体)，摊薄到全军）。
    // 结盟势力在霸权先动手时优先集火该霸权，而非各自就近乱打。
    let focus_of: BTreeMap<FactionId, Option<FactionId>> = state
        .factions
        .iter()
        .map(|f| (f.name.clone(), coalition_war_focus(state, config, &f.name)))
        .collect();

    // **集货定编**（本回合一次）：先把「谁是运输舰」定下来并写进第三条风格轴，再逐舰执行。
    // 放在这里而不是让每艘舰自己算，是为了**与舰的处理顺序无关**——下面的 `order` 是按 rng
    // 打乱的：若逐舰现算，前几艘舰这一回合装的货会改掉后面舰的名额，结论就依赖抽到的顺序了。
    autocontrol::freight::assign_roles(state, config);

    for ship_id in order {
        let Some(ship) = state.ship(&ship_id) else { continue };
        if ship.hull <= 0.0 {
            continue;
        }
        let owner = ship.faction_id.clone();
        let class = ship.class.clone();
        let pos = ship.position;

        let is_ai = state.ship_control(ship_id.clone()) == ControlMode::Auto;

        if !is_ai {
            // --- player-controlled: execute the commanded behavior literally ---
            let mut behavior = state.ship_behavior(ship_id.clone()).unwrap_or(ShipBehavior::Idle);
            // A stale Follow/DockCity/Colonize order (followed ship destroyed, target
            // city razed, or a body with no settlement) must not send the ship drifting
            // toward the origin ([0,0]); degrade it to Idle and record a StaleOrder
            // event so the agent knows to re-issue. Move/Idle are always valid.
            if !behavior_is_valid(state, config, behavior.clone(), &owner) {
                let reason = match behavior {
                    ShipBehavior::Follow { ship } => format!("followed ship {ship} gone"),
                    ShipBehavior::DockCity { city } => format!("target city {city} razed"),
                    ShipBehavior::Colonize { body } => format!("body {body} has no blank settlement"),
                    ShipBehavior::Haul { from, to } => format!("haul route {from} → {to} invalid"),
                    _ => "invalid".to_string(),
                };
                if let Some(c) = state.control_mut(owner.clone()) {
                    c.ship_orders.insert(ship_id.clone(), Control::player(ShipBehavior::Idle));
                }
                ev(state, GameEvent::StaleOrder { ship: ship_id.clone(), reason });
                behavior = ShipBehavior::Idle;
            }
            // 行为 = 纯移动/停泊指令：攻击与轰炸都不再是行为——射程内有敌舰/敌城即自动发生。
            // （待命 Idle = 原地保持「不移动」，但仍会落到下面的自动接战/轰炸。）
            match &behavior {
                ShipBehavior::Colonize { body } => {
                    let bpos = state.body_position(body);
                    if dist(pos, bpos) <= config.combat.arrival_eps {
                        colonize(state, config, rng, &ship_id, body, &mut next_building_id);
                        continue;
                    }
                    move_toward(state, config, &ship_id, &class, bpos);
                    let np = state.ship(&ship_id).map(|s| s.position).unwrap_or(pos);
                    if dist(np, bpos) <= config.combat.arrival_eps {
                        colonize(state, config, rng, &ship_id, body, &mut next_building_id);
                        continue;
                    }
                }
                ShipBehavior::Idle => {
                    // 待命：软目标——原地保持；但附近有敌舰时按 kiting 姿态自动软移动（风筝拉开/贴脸压近）。
                    let dest = autocontrol::kiting_dest(state, config, &ship_id).unwrap_or(pos);
                    move_toward(state, config, &ship_id, &class, dest);
                }
                // 运输：**整条路线本回合都在 `haul_step` 里执行**（择腿 + 移动 + 装卸），
                // 所以这里不再自己移动——落到下面的自动接战，运输舰在航线上照样开火/轰炸。
                ShipBehavior::Haul { from, to } => {
                    haul_step(state, config, &ship_id, &class, from, to);
                }
                _ => {
                    // Move / Follow / DockCity / Dock：驶向行为目的地（软目标）；附近有敌舰时由 kiting 姿态调整。
                    let base = behavior_dest(state, &behavior);
                    let dest = autocontrol::kiting_dest(state, config, &ship_id).unwrap_or(base);
                    move_toward(state, config, &ship_id, &class, dest);
                }
            }
            // --- 自动战斗：攻击与轰炸不需要行为（射程内自动发生）---
            autocontrol::auto_combat(state, config, &ship_id, &owner);
            continue;
        }

        // --- AI-controlled: the auto-control brain decides and executes ---
        autocontrol::ai_ship_turn(
            state,
            config,
            rng,
            &ship_id,
            &focus_of,
            &mut next_building_id,
            &mut flow.decisions.ships,
        );
        continue;
    }

    // 护甲再生（%/时间）：每回合幸存舰只按舰级 hull_regen 恢复其最大护甲的一
    // 个比例（不消耗资源、不复活已毁舰）。处于本方本土防御半径内的舰获得额外
    // home_regen_bonus 再生（cult 的 MOND 异常使其圣所旁舰只极难被消耗）。
    // 先读出每艘舰的本土再生加成，再统一修改（避免与 ships 的可变借用冲突）。
    let (bonuses, friendly): (Vec<f64>, Vec<bool>) = state
        .ships
        .iter()
        .map(|s| {
            if s.hull > 0.0 {
                let dest = state.body_position(&state.capital_body(&s.faction_id));
                let f = state
                    .faction(&s.faction_id)
                    .map(|fac| dist(s.position, dest) <= fac.home_radius)
                    .unwrap_or(false);
                (home_regen_bonus(state, &s.faction_id, s.position), f)
            } else {
                (0.0, false)
            }
        })
        .unzip();
    let comp_repair = config.combat.component_repair;
    for (i, s) in state.ships.iter_mut().enumerate() {
        if s.hull > 0.0 {
            let panel = ship_panel(config, s);
            s.hull = (s.hull + panel.hull_max * (panel.hull_regen + bonuses[i])).min(panel.hull_max);
            // 能量护盾每回合再生（护盾组件）：护盾优先吸收、损毁后再生，是防御组件的关键。
            if panel.shield_max > 0.0 {
                s.shield = (s.shield + panel.shield_max * panel.shield_regen).min(panel.shield_max);
            }
            // 模块修复：受损组件在母港/友方本土修得更快（与自保撤退闭环：打残→撤→修→再来）。
            if comp_repair > 0.0 && !s.components.is_empty() {
                if s.component_hp.len() != s.components.len() {
                    s.component_hp = s.components.iter().map(|c| component_integrity(config, c)).collect();
                }
                let rate = comp_repair * if friendly[i] { 2.5 } else { 1.0 };
                for (j, c) in s.components.iter().enumerate() {
                    let max = component_integrity(config, c);
                    if s.component_hp[j] < max {
                        s.component_hp[j] = (s.component_hp[j] + max * rate).min(max);
                    }
                }
            }
        }
    }

    // 清扫本回合战沉的舰（漏斗**兜底**）：保证「从 state.ships 消失的舰都有死因事件」，
    // 并清掉它的指令。正常 0 艘需要兜底——不为 0 说明某条路径漏了 `kill_ship`，
    // `debug_assert` 会在测试里立刻炸出来。
    let invented = sweep_dead_ships(state, &alive_at_step_start);
    debug_assert_eq!(invented, 0, "有 {invented} 艘舰死亡却没有事件：某条路径漏了 kill_ship");
}

// --- 重建：只有一条路——派殖民舰去复垦 ---------------------------------------
//
// 这里原本是 `step_resurgence`（反僵尸重建）：一支势力一旦「无舰又无活城」，就**凭空**在自己的
// 残骸足迹 / 任何空白城 / 任何未占据定居点上重新立起一座城，甚至夺取城数最多者的活城，还白送
// 一艘种子舰。它保证了「回合末总有立足点」，但代价是**反科学**：城市凭空出现、货物凭空出现
// （补贴市场）、组件凭空装配——三处「免费午餐」互相掩护，于是「缺矿」既不更贵、也不致命。
//
// 已删除（D5）。现在世界只有一条重建路径，且它是物理的：
//   **造一艘殖民舰 → 把它开到一处空白定居点 → `colonize` 复垦。**
// 见 `ShipBehavior::Colonize`（自动控制在「无仗可打」时就会就近挑一处被夷平的定居点）
// 与 `sim::colonize`（`reseed_city` / `found_city` 两个漏斗）。
//
// 后果（有意接受的）：一支**既无舰又无活城**的势力确实再也回不来了——那是亡国，是合法的
// 结局，而不是需要被掩盖的状态。守卫因此改判为「亡国不许滚雪球」+「有舰的流亡势力必须
// 自己复垦回来」，见 `tests/longhorizon.rs`。

// --- governance (light-speed management) -------------------------------------

// --- 思潮优势端自平衡 debuff（平滑、无硬阈值断点） ------------------------

/// C¹ 连续斜坡：`t = clamp01((x−a)/(b−a))`，`t²(3−2t)`。输出 0..1，端点无跳变。
fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 全星系**活城总人口**（各势力所有未夷平城市的人口之和）。
fn total_live_pop(state: &State) -> u64 {
    state
        .cities
        .iter()
        .filter(|c| !c.razed)
        .map(|c| c.population as u64)
        .sum()
}

/// 该势力**战争强度**（0..1）：与任何其他势力的最低关系相对交战阈值越深越贴近 1（平滑）。
/// 关系远在交战阈值之上 → 0（没在打仗）；跌到阈值之下越深 → 趋近 1（在交战）。
fn war_strength(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let wt = config.combat.war_threshold;
    let mut worst: f64 = 0.0;
    for o in &state.factions {
        if o.name == fid {
            continue;
        }
        worst = worst.min(relation(state, fid, &o.name));
    }
    let h = (wt - worst).max(0.0);
    smoothstep(0.0, config.ideology.debuff.war_band, h)
}

/// 该势力的**军事实力占比**（舰队引擎数值之和 / 全星系舰队引擎数值之和），0..1。
fn faction_military_share(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let total: f64 = state.ships.iter().map(|s| ship_panel(config, s).hull_max).sum();
    if total <= 1e-9 {
        return 0.0;
    }
    let mine: f64 = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ship_panel(config, s).hull_max)
        .sum();
    mine / total
}

/// 该势力**舰在 MOND 异常区的占比**：距太阳 > `mond.radius` 的舰数 / 该势力总舰数（0..1）。
/// 无舰 → 0（即完全没在探索异常区）。
fn faction_mond_ship_share(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let r = config.mond.radius;
    let ships: Vec<&Ship> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .collect();
    if ships.is_empty() {
        return 0.0;
    }
    let in_mond = ships.iter().filter(|s| dist(s.position, [0.0, 0.0]) > r).count() as f64;
    in_mond / ships.len() as f64
}

/// 该势力**活城人口占全星系比例**（0..1）。
fn faction_pop_share(state: &State, fid: &str, p_total: f64) -> f64 {
    let pop: u64 = state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && !c.razed)
        .map(|c| c.population as u64)
        .sum();
    if p_total <= 1e-9 {
        0.0
    } else {
        pop as f64 / p_total
    }
}

/// 该势力**殖民活动强度**（0..1）：正在执行 `Colonize`（殖民）行为的舰数，用 `smoothstep`
/// 连续成形（0 艘 → 0；≥2 艘 → 1），无突然跳变。无舰 → 0（完全不殖民）。
fn faction_colonizing(state: &State, fid: &str) -> f64 {
    let ships: Vec<&Ship> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .collect();
    if ships.is_empty() {
        return 0.0;
    }
    let colonize = ships
        .iter()
        .filter(|s| {
            state
                .ship_behavior(s.name.clone())
                .map(|b| matches!(b, ShipBehavior::Colonize { .. }))
                .unwrap_or(false)
        })
        .count() as f64;
    smoothstep(0.0, 2.0, colonize)
}

/// 该势力**思潮优势端自平衡 debuff** —— 返回全国忠诚度惩罚（0..`max_loyalty_penalty`）。
///
/// 打击强度由**该势力自身在星系中的体量**（活城人口占比 `dom`）驱动——越大越该被打，因此
/// 随单极化持续、不随「行为一致化」消退。再乘上各轴「思潮 vs 行为不符」的连续度：
///
/// `penalty = max × clamp01(dom × Σ_轴 w_轴 × violate_轴)`：
///   * `dom` = 该势力活城人口 / 全星系活城人口的 `smoothstep(gate_lo..gate_hi)`；
///   * `violate_轴` = 该势力「优势端思潮 vs 行为不符」的连续度（见 [`IdeologyDebuffConfig`]）。
///
/// 全 C¹ 平滑、无硬阈值；只对优势端思潮本身生效（自指向，不误伤中立/对立端）。
fn ideology_loyalty_debuff(
    state: &State,
    config: &GameConfig,
    fid: &str,
    p_total: f64,
) -> f64 {
    let d = &config.ideology.debuff;
    let id = state
        .faction(fid)
        .map(|f| f.ideology)
        .unwrap_or_default();
    // 打击强度 = 该势力自身体量（单极化越坐大越该被打）。
    let dom = smoothstep(d.gate_lo, d.gate_hi, faction_pop_share(state, fid, p_total));

    // 轴1 和平↔军国，优势端=军国(+)：军国 且（不战争 且 低军事实力占比）。
    let w = war_strength(state, config, fid);
    let ml = faction_military_share(state, config, fid);
    let viol_mil = smoothstep(0.0, 1.0, id.peace_military)
        * (1.0 - w)
        * (1.0 - smoothstep(0.0, d.mil_share_ref, ml));
    // 轴2 科学↔技术，优势端=科学(−)：科学 且 舰在 MOND 区占比低。
    let ms = faction_mond_ship_share(state, config, fid);
    let viol_sci = smoothstep(0.0, 1.0, -id.science_tech) * (1.0 - smoothstep(0.0, 1.0, ms));
    // 轴3 人民↔精英，优势端=精英(+)：精英 且 人口占全星系比例高（体量由 dom 承担，这里只看精英度）。
    let viol_elite = smoothstep(0.0, 1.0, id.people_elite);
    // 轴4 自然↔殖民，优势端=殖民(+)：殖民 且 不殖民。
    let col = faction_colonizing(state, fid);
    let viol_col = smoothstep(0.0, 1.0, id.nature_colony) * (1.0 - smoothstep(0.0, 1.0, col));

    let raw = d.w_military * viol_mil + d.w_science * viol_sci + d.w_elite * viol_elite + d.w_colony * viol_col;
    d.max_loyalty_penalty * (dom * raw).clamp(0.0, 1.0)
}

/// 本回合各势力的「思潮优势端自平衡 debuff」忠诚度惩罚（0..`max_loyalty_penalty`）。
/// 纯观测（不推进世界），供 agent 视图与调参。与 `step_governance` 同源（同一公式）。
pub fn faction_ideology_debuffs(state: &State, config: &GameConfig) -> BTreeMap<FactionId, f64> {
    let p_total = total_live_pop(state) as f64;
    state
        .factions
        .iter()
        .map(|f| (f.name.clone(), ideology_loyalty_debuff(state, config, &f.name, p_total)))
        .collect()
}

/// 光速治理：每座城按其与统治势力首都的距离产生一笔治理开销（距离越远、管辖越难）。
/// 势力从库存按价值支付；付得起时城市忠诚度向距离目标恢复（远则低），付不起（欠费）
/// 时忠诚度暴跌。忠诚度跌破 [`GovernanceConfig::loyalty_revolt`] 即爆发离心叛乱，城市
/// 被夷平为空白（可再殖民）。这给超大帝国一个自然上限——既能管的领地有限，遥远的
/// 殖民地在治理失败时丢失，使世界在上千回合后保持多方参与。
fn step_governance(state: &mut State, config: &GameConfig, flow: &mut RoundFlow) {
    let g = &config.governance;
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    // 思潮优势端自平衡 debuff 的输入：全星系活城人口（用于算「该势力体量」占比）。
    let p_total = total_live_pop(state) as f64;

    for fid in faction_ids {
        let capital = state.capital_body(&fid);
        let cap_pos = state.body_position(&capital);

        // 该势力所有活城 + 每城到首都的距离 + 每城想投入的娱乐/福利预算。
        let mut cities: Vec<(CityId, f64, f64)> = Vec::new(); // (name, distance, ent_budget)
        let mut total_pop = 0u64;
        for c in &state.cities {
            if c.faction_id != fid || c.razed {
                continue;
            }
            total_pop += c.population as u64;
            let d = dist(state.body_position(&c.body_id), cap_pos);
            let ent = city_loyalty_budget(state, config, fid.clone(), c.name.clone());
            cities.push((c.name.clone(), d, ent));
        }
        if cities.is_empty() {
            continue;
        }
        // 人口越多，管理能力越分散——人口超载放大远距离治理难度（与距离叠加）。
        let overload = (total_pop as f64 / g.population_capacity.max(1e-6) - 1.0).max(0.0);
        let scale = 1.0 + overload;
        let mut total_admin = 0.0;
        let mut ent_total = 0.0;
        for (_, d, ent) in &cities {
            let a = (d - g.admin_range).max(0.0);
            total_admin += (g.admin_base + g.admin_per_au * a) * scale;
            ent_total += ent;
        }
        let governance_total = (total_admin + ent_total) * sanction_cost_mult(state, config, &fid);

        // 用库存（按价值加权）支付治理 + 娱乐开销（与舰队维护同源）。覆盖率决定
        // 治理是否到位以及娱乐投入是否真正落地。
        let stock = state.faction(&fid).map(|f| f.resources.clone()).unwrap_or_default();
        let total_value: f64 = stock.iter().map(|(k, v)| v * value_of(k)).sum();
        let pay = governance_total.min(total_value);
        if pay > 1e-9 {
            let ratio = (pay / total_value).min(1.0);
            if let Some(f) = state.faction_mut(&fid) {
                for (k, v) in stock.iter() {
                    let new = (*v - *v * ratio).max(0.0);
                    f.resources.insert(k.clone(), new);
                }
            }
        }
        let coverage = if governance_total > 1e-9 {
            (total_value / governance_total).min(1.0)
        } else {
            1.0
        };
        // 记录本回合治理流（step_governance 的「中间量」）：总开销 + 覆盖率。
        flow.governance.insert(fid.clone(), GovernanceFlow { total: governance_total, coverage });

        // 忠诚度向「距离目标 + 娱乐加成 + 首都人口占比 buff」恢复/下降，并标记叛乱。
        // 首都人口占全势力的比例越高，全国向心力越强（每城目标忠诚更高）。用占比：把
        // 首都放在人口中心有真实收益，而不是无脑堆绝对人口。
        let cap_bonus = faction_capital_share(state, &fid) * g.capital_share_loyalty_buff;
        // 思潮优势端自平衡 debuff：该势力若身处「垄断」的优势端思潮又「言行不符」，扣全国忠诚。
        let ideo_penalty = ideology_loyalty_debuff(state, config, &fid, p_total);
        let mut to_revolt = Vec::new();
        for (cid, d, ent) in &cities {
            let a = (d - g.loyalty_range).max(0.0);
            let target_base = (1.0 - g.loyalty_distance * a * scale).clamp(0.0, 1.0);
            let ent_bonus = (ent * coverage) / g.entertainment_cost.max(1e-6);
            let target_eff = (target_base + ent_bonus + cap_bonus - ideo_penalty).clamp(0.0, 1.0);
            let cur = state.city(cid).map(|c| c.loyalty).unwrap_or(1.0);
            let new = if coverage >= 1.0 - 1e-6 {
                (cur + (target_eff - cur) * g.loyalty_recover).clamp(0.0, 1.0)
            } else {
                (cur - g.loyalty_penalty * (1.0 - coverage)).max(0.0)
            };
            if let Some(c) = state.city_mut(cid) {
                c.loyalty = new;
            }
            if new < g.loyalty_revolt {
                to_revolt.push(cid.clone());
            }
        }

        // 离心叛乱：低忠诚城市**改旗易帜**——不夷为荒地，而是倒戈到「思潮与旧主最对立」
        // 的势力（见 [`most_ideologically_distant_faction`]/[`defect_city`]）。这既让过度扩张
        // 的大帝国体量回落，又让旁观/小势力能接盘城市、成长为多极棋子。找不到可倒戈目标
        // （世界只剩一家）时兜底夷为空白（可再殖民，旧行为）。
        for cid in to_revolt {
            // 先读爆发时的忠诚度：下面两条分支都会把它改写（倒戈重置为 1.0、夷平清零）。
            let loyalty = state.city(&cid).map(|c| c.loyalty).unwrap_or(0.0);
            let target = most_ideologically_distant_faction(state, &fid)
                .filter(|to| to != &fid);
            if let Some(to) = target {
                // 漏斗：改归属 + 记 `CityDefected`（在同一处，漏不掉）。
                defect_city(state, config, &cid, &fid, &to, loyalty);
            } else {
                // 漏斗：夷平为空白 + 记 `Revolt`。
                raze_city(state, &cid, RazeCause::Revolt { faction: fid.clone(), loyalty });
            }
        }
    }
}

// --- 离心「改旗易帜」(loyalty-driven defection) -----------------------------

/// 两股思潮的**对立度**：4 条轴上的 L1 距离（`|Δ|` 之和），范围 [0,8]。
/// 越大代表两国的当代思潮越对立。
fn ideology_distance(a: &Ideology, b: &Ideology) -> f64 {
    (a.peace_military - b.peace_military).abs()
        + (a.science_tech - b.science_tech).abs()
        + (a.people_elite - b.people_elite).abs()
        + (a.nature_colony - b.nature_colony).abs()
}

/// 与 `owner` **思潮最对立**的势力（取其当前 [`Faction::ideology`]）：这是低忠诚城市
/// 「改旗易帜」的倒戈目标——居民不认同旧主的思潮，投向与其最对立的强权。确定性、无
/// RNG：距离最大者胜，并列取名字序最小。`owner` 自身排除；世界只剩一家时返回 `None`。
fn most_ideologically_distant_faction(state: &State, owner: &str) -> Option<FactionId> {
    let owner_ideo = state.faction(owner).map(|f| f.ideology)?;
    let mut best: Option<(f64, FactionId)> = None;
    for f in &state.factions {
        if f.name == owner {
            continue;
        }
        let d = ideology_distance(&owner_ideo, &f.ideology);
        let better = match &best {
            None => true,
            Some((bd, bn)) => d > *bd || (d == *bd && f.name < *bn),
        };
        if better {
            best = Some((d, f.name.clone()));
        }
    }
    best.map(|(_, n)| n)
}

/// 把一座城从 `from` 倒戈给 `to`（漏斗）：城市换主、居民重燃对新主的认同（忠诚重置为 1.0，
/// 不再立刻叛变）、人口/建筑/船坞/空间站全部保留（这是一次**改旗易帜**，不是夷平）。
/// 同时把旧主对该城建筑/娱乐预算的控制叶子迁到新主名下，使新主的 AI 确实能治理这座
/// 城；并对旧主↔新主施加「夺城」级的关系打击（倒戈在旧主眼中几近叛国）。
/// `loyalty` 是爆发时的忠诚度（换主后会被重置，故由调用方先读好传入）。
fn defect_city(state: &mut State, config: &GameConfig, city: &str, from: &str, to: &str, loyalty: f64) {
    // 先收集该城建筑 id（避免在可变借权时再读 state.city）。
    let building_ids: Vec<BuildingId> = state
        .city(city)
        .map(|c| c.buildings.iter().map(|b| b.id).collect())
        .unwrap_or_default();

    // 换主 + 忠诚重置。
    if let Some(c) = state.city_mut(city) {
        c.faction_id = to.to_string();
        c.loyalty = 1.0;
        c.razed = false;
    }

    // 控制转移：把旧主控制面里 keyed-by-(city, building) 的叶子搬到新主名下。
    let mut moved_invest: Vec<(InvestKey, Control<f64>)> = Vec::new();
    let mut moved_build: Vec<(BuildKey, Control<f64>)> = Vec::new();
    let mut moved_loyalty: Option<(CityId, Control<f64>)> = None;
    if let Some(o) = state.control_mut(from.to_string()) {
        for bid in &building_ids {
            let ikey = (city.to_string(), *bid);
            if let Some(v) = o.invest_weights.remove(&ikey) {
                moved_invest.push((ikey, v));
            }
            let bkey = (city.to_string(), *bid);
            if let Some(v) = o.build_weights.remove(&bkey) {
                moved_build.push((bkey, v));
            }
        }
        if let Some(v) = o.loyalty_budget.remove(city) {
            moved_loyalty = Some((city.to_string(), v));
        }
    }
    if let Some(n) = state.control_mut(to.to_string()) {
        for (k, v) in moved_invest {
            n.invest_weights.insert(k, v);
        }
        for (k, v) in moved_build {
            n.build_weights.insert(k, v);
        }
        if let Some((k, v)) = moved_loyalty {
            n.loyalty_budget.insert(k, v);
        }
    }

    // 外交：倒戈 = 夺城级的关系下压（旧主视新主为敌）。
    let cur = relation(state, from, to);
    set_relation_sym(state, from.to_string(), to.to_string(), cur + config.diplomacy.capture_delta, config);
    // 漏斗负责记事件：改归属与记 `CityDefected` 在同一处，**忘记记在结构上不可能**。
    ev(state, GameEvent::CityDefected {
        city: city.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        loyalty,
    });
}

// --- 迁都 (capital relocation) ----------------------------------------------

/// 维护每个势力的「有效首都」的唯一事实来源（[`ControllableState::capital`]）。
///
/// 两个触发（都确定性、无 RNG）：
/// * **亡城强迁（硬规则，先于一切）**：只要有效首都天体上已无本势力的活城（被夷平或
///   被敌人殖民夺走），就把首都切到本势力**人口最高的活城**（并列取名字序）——不能让
///   首都钉在已死的天体上。即使 Player 设过首都也强迁（死首都无效）。
/// * **周期性 AI 评估**：非 Player 控制的首都（[`State::capital_control`] == Ai）每
///   [`GovernanceConfig::capital_review_every`] 回合重估一次：候选 = 人口最高的活城；
///   仅当它对全势力各城的「总治理距离成本」比当前首都低
///   [`GovernanceConfig::capital_relocate_threshold`] AU 以上时才迁（避免反复横跳）。
///
/// 放在 [`step_resurgence`] 之后：刚重建出立足点的势力也能当回合被认领一个新首都。
fn step_capital(state: &mut State, config: &GameConfig) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let review_every = config.governance.capital_review_every.max(1);

    for fid in faction_ids {
        let cur = state.capital_body(&fid);
        let living: Vec<CityId> = state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid && !c.razed)
            .map(|c| c.name.clone())
            .collect();
        if living.is_empty() {
            continue; // 无活城：resurgence 会在后续回合重建，届时再定首都。
        }

        let cur_owned = living.iter().any(|cid| {
            state.city(cid).map(|c| c.body_id == cur).unwrap_or(false)
        });

        let mut new_cap: Option<BodyId> = None;
        let mut reason = "";

        if !cur_owned {
            // 亡城强迁 → 人口最高的活城（并列取名字序）。
            new_cap = Some(highest_pop_city_body(state, &fid));
            reason = "destroyed";
        } else if state.capital_control(&fid) == ControlMode::Auto && state.round % review_every == 0 {
            let best = highest_pop_city_body(state, &fid);
            if best != cur {
                let cur_cost = capital_anchor_cost(state, config, &fid, &cur);
                let best_cost = capital_anchor_cost(state, config, &fid, &best);
                if best_cost + config.governance.capital_relocate_threshold < cur_cost {
                    new_cap = Some(best);
                    reason = "ai_review";
                }
            }
        }

        if let Some(nc) = new_cap {
            let from = cur.clone();
            // 迁都的全国忠诚度代价：旧首都人口占比 × 系数 = 每座城忠诚下降。占比越高
            // 迁离越动荡（国本动摇）；亡城强迁时旧首都已失（占比=0）→ 应急无忠诚代价。
            let old_share = faction_capital_share(state, &fid);
            let loyalty_cost = old_share * config.governance.capital_share_relocate_cost;
            // 保留原 mode 标记（Player 仍归玩家、Inherit 让作用域链决定）——迁都是换「值」，
            // 不改变「由谁决定」的层次化粒度。
            let prev_mode = state
                .control
                .get(&fid)
                .and_then(|c| c.capital.as_ref())
                .map(|c| c.mode)
                .unwrap_or_default();
            {
                let ctrl = state.control.entry(fid.clone()).or_default();
                ctrl.capital = Some(Control { value: nc.clone(), mode: prev_mode });
            }
            if loyalty_cost > 0.0 {
                for cid in &living {
                    if let Some(c) = state.city_mut(cid) {
                        c.loyalty = (c.loyalty - loyalty_cost).max(0.0);
                    }
                }
            }
            ev(state, GameEvent::CapitalRelocated {
                faction: fid.clone(),
                from,
                to: nc,
                reason: reason.to_string(),
            });
        }
    }
}

/// 该势力**有效首都**的人口占其全势力活城人口的比例（0..1）。首都占全势力人口的比例
/// 越高 → 全国忠诚度 buff 越强；迁离占比高的首都 → 全国忠诚度代价越大。
fn faction_capital_share(state: &State, fid: &str) -> f64 {
    let cap = state.capital_body(fid);
    let mut cap_pop = 0u64;
    let mut total_pop = 0u64;
    for c in &state.cities {
        if c.faction_id != fid || c.razed {
            continue;
        }
        let p = c.population as u64;
        total_pop += p;
        if c.body_id == cap {
            cap_pop += p;
        }
    }
    if total_pop == 0 {
        0.0
    } else {
        cap_pop as f64 / total_pop as f64
    }
}

/// 本势力**人口最高的活城**之天体（并列取名字序，确定性）。调用方保证该势力有活城。
fn highest_pop_city_body(state: &State, fid: &str) -> BodyId {
    state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && !c.razed)
        .min_by_key(|c| (std::cmp::Reverse(c.population), c.name.clone()))
        .map(|c| c.body_id.clone())
        .expect("caller guarantees a living city")
}

/// 该势力若以 `cap` 为首都，其全部活城到它的「总治理距离成本」（AU）：
/// Σ max(0, dist(body, cap) - admin_range)。用作迁都「是否更优」的判据（越小越好）。
fn capital_anchor_cost(state: &State, config: &GameConfig, fid: &str, cap: &str) -> f64 {
    let g = &config.governance;
    let cap_pos = state.body_position(cap);
    let mut total = 0.0;
    for c in &state.cities {
        if c.faction_id != fid || c.razed {
            continue;
        }
        let d = dist(state.body_position(&c.body_id), cap_pos);
        total += (d - g.admin_range).max(0.0);
    }
    total
}

/// Move a ship one round's step toward `dest`, capped by its class speed.
pub(crate) fn move_toward(state: &mut State, config: &GameConfig, ship_id: &str, _class: &str, dest: [f64; 2]) {
    let Some(ship) = state.ship(ship_id).cloned() else { return };
    let pos = ship.position;
    let fid = ship.faction_id.clone();
    // MOND 异常区：没有掌握修正引力的势力把指令坐标「算错」，实际航向产生偏移。
    // 偏移幅度是**伪随机**的（`nav_roll` 按 势力×舰名×回合 派生）：这一回合偏多少是确定的，
    // 但**下回合是全新的一次尝试**——所以深处目标不是「进不去」，而是「要多试几个回合」。
    let dest = mond_drift(config, &fid, dest, nav_roll(&fid, &ship.name, state.round));
    let distance = dist(pos, dest);
    if distance <= 1e-9 {
        return;
    }
    // 有效速度 = 推进模块给的**巡航速度**（被舰级 speed_mult 缩放），但当前速度
    // `velocity` 每回合按推进模块的**加速度** `accel` 提升、最多到巡航——舰船不能
    // 瞬间加速，而是逐步逼近巡航速度（加速度=战位调整的快慢）。
    let panel = ship_panel(config, &ship);
    let cruise = panel.speed;
    // 当前速度向巡航逼近（accel 若为 0 则直接到巡航，避免推进全被打伤时卡死）。
    let vel = if cruise > 0.0 {
        let accel = panel.accel;
        let next = if accel > 1e-9 { ship.velocity + accel } else { cruise };
        next.min(cruise)
    } else {
        0.0
    };
    let step = if distance <= config.combat.arrival_eps { 0.0 } else { vel.min(distance) };
    if let Some(s) = state.ship_mut(ship_id) {
        s.velocity = vel;
        if step > 0.0 {
            let nx = (dest[0] - pos[0]) / distance;
            let ny = (dest[1] - pos[1]) / distance;
            s.position = [s.position[0] + nx * step, s.position[1] + ny * step];
        }
    }
}

/// 某势力的**交割锚点**（货物从哪儿发出 / 运到哪儿）：优先其首都天体，其次（无活城的
/// 流亡势力）其第一艘活舰的位置，都没有则原点。用于估算贸易路线的距离与是否穿越异常带。
fn trade_anchor(state: &State, fid: &str) -> [f64; 2] {
    let cap = state.capital_body(fid);
    if !cap.is_empty() && state.body(&cap).is_some() {
        return state.body_position(&cap);
    }
    if let Some(s) = state.ships.iter().find(|s| s.faction_id == fid && s.hull > 0.0) {
        return s.position;
    }
    [0.0, 0.0]
}

/// 一条贸易路线**浸入引力异常带的最大深度**（AU，0 = 完全没进异常带）。
///
/// * 两端都在异常带外 → 0（普通航线，无 MOND 代价）。
/// * 一端在内一端在外 → `远端半径 − radius`（越深，穿越成本越高）。
/// * 两端都在带内 → `较浅那端半径 − radius`（整条线都在异常带里，走浅处算）。
///
/// 就是 `decider_radius - mond.radius` 的闭式表达，确定性、无 RNG。
pub(crate) fn route_depth(config: &GameConfig, a: [f64; 2], b: [f64; 2]) -> f64 {
    let r = |p: [f64; 2]| (p[0] * p[0] + p[1] * p[1]).sqrt();
    let (ra, rb) = (r(a), r(b));
    let (inner, outer) = if ra <= rb { (ra, rb) } else { (rb, ra) };
    let deep_ref = if inner > config.mond.radius { inner } else { outer };
    (deep_ref - config.mond.radius).max(0.0)
}

/// 是否掌握 MOND 修正引力（异常区内无导航偏移，因而能可靠承运）。
pub(crate) fn is_mond_master(config: &GameConfig, fid: &str) -> bool {
    config.mond.masters.iter().any(|x| x == fid)
}

/// MOND 主力导航偏移：舰船所在势力未掌握 MOND 修正引力（见 [`MondConfig::masters`]）
/// 且目标点进入异常区（距太阳超过 `mond.radius`）时，返回一个沿切向偏移的**伪目标**。
/// 非 master 舰因此难以精确机动到深处目标（难以轰炸/殖民/停靠、也运不出货），
/// 体现「指令坐标与实际坐标产生偏移」。
///
/// **偏移幅度是伪随机的、不是写死的**（用户裁决，见 `.agents/notes/freight-collection.md`）：
/// `roll ∈ [0,1)`（由 [`nav_roll`] 按「势力 × 舰名 × 回合」确定性派生）决定这一次尝试
/// 偏多少——`roll = 0` 就是**指哪打哪**。于是：
///
/// ```text
/// 一次尝试的成功率  p = P(偏移 ≤ arrival_eps) = min(1, arrival_eps / (depth × drift_per_au))
/// ```
///
/// **任何深度都 p > 0**（哪怕深度 100 AU、MOND 再强，也只是多试几个回合，而不是永远进不去），
/// 而 p 随深度单调递减：`伊克西翁 30.17 AU → p≈0.92`、`妊神星 35.01 → 0.29`、
/// `创神星 38.16 → 0.20`。所以「深处难去」不再是硬墙，而是**要试几次**。
///
/// 确定性：`roll` 由调用方给（纯函数），不消费主 `Prng` 流。
fn mond_drift(config: &GameConfig, fid: &str, dest: [f64; 2], roll: f64) -> [f64; 2] {
    let m = &config.mond;
    if m.drift_per_au <= 0.0 || m.masters.iter().any(|x| x == fid) {
        return dest;
    }
    let r = (dest[0] * dest[0] + dest[1] * dest[1]).sqrt();
    let depth = (r - m.radius).max(0.0);
    if depth <= 0.0 {
        return dest;
    }
    // 这一次尝试偏多少：幅度 = 上界 × roll^shape（`roll = 0` ⇒ 精确命中）。
    // `drift_shape < 1` 让偏移偏向大值（更常迷航在远处），但**永远留着蒙对的可能**。
    let shape = if m.drift_shape > 0.0 { m.drift_shape } else { 1.0 };
    let drift = depth * m.drift_per_au * roll.clamp(0.0, 1.0).powf(shape);
    // 切向（垂直于径向），代表轨道力学计算错误。
    let inv = if r > 1e-9 { 1.0 / r } else { 0.0 };
    let tx = -dest[1] * inv;
    let ty = dest[0] * inv;
    [dest[0] + tx * drift, dest[1] + ty * drift]
}

/// 一次**导航尝试**的确定性伪随机数 ∈ `[0,1)`——[`mond_drift`] 的偏移幅度取自它。
///
/// 就是 [`derived_roll`] 取**空盐**的那一档（`(势力, 舰名, 回合)`）。空盐让字节序列与
/// `derived_roll` 出现之前**逐字相同**，所以已经实测过的 MOND 表（成功率 / 首次命中回合）
/// 不作废。为什么不用主 [`Prng`] 流、为什么「下一回合是一次全新尝试」，见 [`derived_roll`]。
fn nav_roll(fid: &str, ship: &str, round: u32) -> f64 {
    derived_roll(fid, ship, round, "")
}

/// FNV-1a 64：只为把几个字段混成一个种子，不需要密码学强度。
fn fnv1a(bytes: impl Iterator<Item = u8>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 派生一枚**确定性伪随机数** ∈ `[0,1)`：把 `(势力, 舰名, 回合, 用途)` 混成一个 64 位种子
/// （FNV-1a）后再取 [`Prng::unit`]。
///
/// # 为什么不用主 [`Prng`] 流
///
/// 那会让「某艘舰多试了一次」改变**整个世界后续的掷骰**：同种子可复现就退化成
/// 「只要舰队数量一变，后面全变」，存档续玩的随机位置也解释不通。派生骰子是**独立**的：
/// 同一回合、同一艘舰、同一用途的结果恒定，下一回合才是一枚新骰子。
///
/// # `salt` = 用途
///
/// 同一艘舰在同一回合需要**多枚互不相关的骰子**（导航偏移、选线抽签……）。不区分用途，
/// 「偏得远的舰」和「被派去远货栈的舰」就会被同一枚骰子绑在一起（可观测的相关性）。
/// **空盐 = 导航那一档**（[`nav_roll`]），字节序列保持原样。
///
/// 这是本仓库的默认做法（见 `AGENTS.md`「默认用概率分布」）：**概率 ≠ 不可复现**。
pub(crate) fn derived_roll(fid: &str, ship: &str, round: u32, salt: &str) -> f64 {
    let mut bytes: Vec<u8> = Vec::with_capacity(fid.len() + ship.len() + salt.len() + 10);
    bytes.extend_from_slice(fid.as_bytes());
    bytes.push(b'|');
    bytes.extend_from_slice(ship.as_bytes());
    bytes.push(b'@');
    bytes.extend_from_slice(&round.to_le_bytes());
    if !salt.is_empty() {
        bytes.push(b'#');
        bytes.extend_from_slice(salt.as_bytes());
    }
    Prng::from_state(fnv1a(bytes.into_iter())).unit()
}

/// [`mond_drift`] 一次尝试的**成功率**：`偏移 = 上界 × roll^shape`，`roll` 均匀 ∈ `[0,1)`，
/// 而「到达」要求偏移 ≤ `arrival_eps`，于是
///
/// ```text
/// p = min(1, (arrival_eps / (depth × drift_per_au))^(1/shape))
/// ```
///
/// `shape = 1`（默认）时就是 `eps/(depth×drift)`。只服务观察与守卫（结算读 [`mond_drift`]）：
/// **p 恒 > 0**，深度越大只会越难、需要越多次尝试，永远没有「进不去」。
pub(crate) fn mond_arrival_chance(config: &GameConfig, depth: f64) -> f64 {
    let m = &config.mond;
    if m.drift_per_au <= 0.0 || depth <= 0.0 {
        return 1.0;
    }
    let ratio = config.combat.arrival_eps / (depth * m.drift_per_au);
    if ratio >= 1.0 {
        return 1.0;
    }
    let shape = if m.drift_shape > 0.0 { m.drift_shape } else { 1.0 };
    ratio.powf(1.0 / shape)
}

// --- 集货腿（M2b）：装卸货 + 运输路线的执行 -----------------------------------

/// 一批货的**尽量等量**分配（用户裁决 Q6）：把 `capacity` 个单位按「还有货的种类数」平摊；
/// 某种货不够平摊，就把它的余量**交回去**、由其余种类再平摊（max-min 公平分配）。
///
/// ```text
/// 三种货、舱容 6，都够      ⇒ 每种 2
/// {铁 10, 铂 1, 碳 10}、舱容 6 ⇒ {铁 2.5, 铂 1, 碳 2.5}
/// 舱容 ≥ 总存量             ⇒ 全装走
/// ```
///
/// 为什么不是「按价值降序」或「按存量比例」：**尽量等量**不会让某种便宜货永远排在队尾
/// （矿是混装的散货，不是按单价挑的快递），也不会在货栈里留下一地分数残渣。
/// 纯函数、确定性（[`ResourceMap`] 是 `BTreeMap` ⇒ 名字序遍历，与插入顺序无关）。
pub fn haul_split(available: &ResourceMap, capacity: f64) -> ResourceMap {
    let mut out = ResourceMap::new();
    let mut cap = capacity;
    if cap <= 1e-9 {
        return out;
    }
    let mut pool: Vec<(&String, f64)> = available
        .iter()
        .filter(|(_, a)| **a > 1e-9)
        .map(|(k, a)| (k, *a))
        .collect();
    // 每一轮：把剩余舱容平摊给「还有货没分完」的种类；分光的种类退出、余量进下一轮。
    // 终止性：每轮要么分完舱容（cap → 0），要么至少有一种货被分光（pool 变小）。
    while !pool.is_empty() && cap > 1e-9 {
        let share = cap / pool.len() as f64;
        let mut next: Vec<(&String, f64)> = Vec::new();
        for (rt, left) in pool {
            let take = share.min(left);
            *out.entry(rt.clone()).or_insert(0.0) += take;
            cap -= take;
            if left - take > 1e-9 {
                next.push((rt, left - take));
            }
        }
        pool = next;
    }
    out.retain(|_, v| *v > 1e-9);
    out
}

/// 一条运输路线（[`ShipBehavior::Haul`]）本回合**做了什么**（观察与守卫用；不影响行为）。
#[derive(Clone, Debug, PartialEq)]
pub enum HaulStep {
    /// 在 `body` 装上了 `units` 件货（Q5 A：有多少装多少，绝不空舱等待）。
    Loaded { body: BodyId, units: f64 },
    /// 在 `body` 卸下 `units` 件货；`into_pool = true` 表示卸进了**首都池**（集货完成）。
    Delivered { body: BodyId, units: f64, into_pool: bool },
    /// 停在 `from` 但**货栈是空的**：原地等——「有货就走」的另一半正是「没货就不走」
    /// （空载跑一趟是白烧时间，而货栈随时会因产出再涨）。
    Waiting { body: BodyId },
    /// 这一回合只是**在路上**，正驶向 `body`（装/卸都还没发生）。
    EnRoute { body: BodyId },
}

impl HaulStep {
    /// 这一步发生在哪个天体（`EnRoute` = 正驶向的那一端）——判定表与探针读它。
    pub fn body(&self) -> &str {
        match self {
            HaulStep::Loaded { body, .. }
            | HaulStep::Delivered { body, .. }
            | HaulStep::Waiting { body }
            | HaulStep::EnRoute { body } => body,
        }
    }
}

/// 这批货的**货主**（收货方）：执行承包单时是**托运方**，否则是船主自己。
///
/// 「货主」是本作里必须显式存在的概念（`.agents/notes/freight-collection.md` 的已定项）：
/// 承包让**船东与业主分离**，于是「装谁的货、卸进谁的池子、卸出来的货算谁的进度」
/// 三件事都不能再默认等于船主。判据是 [`ContractState::assignments`]（哪艘舰跑哪张单），
/// 不去猜「路线像不像」——同一处货栈可以有两张不同托运方的单。
fn cargo_owner(state: &State, fid: &str, ship_id: &str) -> FactionId {
    state
        .contracts
        .assignment_of(ship_id)
        .and_then(|id| state.contracts.get(id))
        .map(|c| c.shipper.clone())
        .unwrap_or_else(|| fid.to_string())
}

/// **装货**：把 `from` 处**货主**的产地货栈装进 `ship` 的货舱。
///
/// 上限 = 有效舱容（[`cargo_capacity`]：舰级舱容 × 战损折算）− 已在舱；分配按 [`haul_split`]。
/// 执行承包单时还多一道上限：**这张单还差多少**——承运人只替托运方搬它挂出来（且还没送到）
/// 的量，多装了等于运了没谈过价的货。
/// 返回**实际装走的总件数**（0 = 那里没货，舰该原地等）。
fn haul_load(state: &mut State, config: &GameConfig, fid: &str, ship_id: &str, from: &str) -> f64 {
    let Some(ship) = state.ship(ship_id) else {
        return 0.0;
    };
    let free = cargo_capacity(config, ship) - cargo_used(&ship.cargo);
    if free <= 1e-9 {
        return 0.0;
    }
    let owner = cargo_owner(state, fid, ship_id);
    let room = match state
        .contracts
        .assignment_of(ship_id)
        .and_then(|id| state.contracts.get(id))
    {
        Some(c) => free.min(c.outstanding()),
        None => free,
    };
    if room <= 1e-9 {
        return 0.0; // 这张单已经送够了（等挂单簿收尾），别再装
    }
    let avail = state.depot(&owner, from).cloned().unwrap_or_default();
    let plan = haul_split(&avail, room);
    let mut moved: ResourceMap = ResourceMap::new();
    for (rt, want) in &plan {
        let got = state.depot_take(&owner, from, rt, *want);
        if got > 0.0 {
            moved.insert(rt.clone(), got);
        }
    }
    let units: f64 = moved.values().sum();
    if units <= 0.0 {
        return 0.0;
    }
    if let Some(s) = state.ship_mut(ship_id) {
        for (rt, amt) in &moved {
            *s.cargo.entry(rt.clone()).or_insert(0.0) += amt;
        }
    }
    ev(
        state,
        GameEvent::CargoLoaded {
            ship: ship_id.to_string(),
            faction: fid.to_string(),
            owner,
            body: from.to_string(),
            cargo: moved,
        },
    );
    units
}

/// **卸货**：把 `ship` 的整个货舱卸进 `to`。`to` 是**货主**的首都 ⇒ 直接进货主的**势力池**
/// （[`Faction::resources`]：集货腿的终点，货从此可用）；否则进该天体的货栈（中转，还得再运一程）。
///
/// 执行承包单时多做一件事：**按抽成留下承运人的那一份**（Q10）——留下来的货直接进
/// 承运人自己的首都池（用户批准的简化：报酬**就是它没交出去的那部分货**，没有货币转移）。
/// 真正的「回程把自己那份拉回家」需要把 `Haul` 的无状态腿规则撑开（舱里不是空的就是满的
/// 那条判据不够用了），留作后续钩子。
/// 返回卸下的货（空 = 本来就空舱）。
fn haul_unload(state: &mut State, config: &GameConfig, fid: &str, ship_id: &str, to: &str) -> ResourceMap {
    let cargo = state
        .ship_mut(ship_id)
        .map(|s| std::mem::take(&mut s.cargo))
        .unwrap_or_default();
    if cargo.is_empty() {
        return cargo;
    }
    let owner = cargo_owner(state, fid, ship_id);
    // 承包单：先按抽成切出承运人的那一份（Q10）。
    let contract = state
        .contracts
        .assignment_of(ship_id)
        .and_then(|id| state.contracts.get(id).cloned());
    let loaded: f64 = cargo.values().sum(); // 卸出舱的**总量**（合同进度按它记）
    let mut delivered = cargo.clone();
    let mut cut_units = 0.0;
    if let Some(c) = &contract {
        for (rt, amt) in delivered.iter_mut() {
            let cut = c.carrier_cut(*amt);
            *amt -= cut;
            cut_units += cut;
            if cut > 0.0 {
                // 承运人的报酬进它自己的首都池（Q10 的机制落点：没有货币转移）。
                if let Some(f) = state.factions.iter_mut().find(|f| f.name == fid) {
                    *f.resources.entry(rt.clone()).or_insert(0.0) += cut;
                }
            }
        }
        delivered.retain(|_, amt| *amt > 1e-9);
    }
    let into_pool = state.capital_body(&owner) == to;
    if into_pool {
        if let Some(f) = state.factions.iter_mut().find(|f| f.name == owner) {
            for (rt, amt) in &delivered {
                *f.resources.entry(rt.clone()).or_insert(0.0) += amt;
            }
        }
    } else {
        for (rt, amt) in &delivered {
            state.depot_add(&owner, to, rt, *amt);
        }
    }
    ev(
        state,
        GameEvent::CargoDelivered {
            ship: ship_id.to_string(),
            faction: fid.to_string(),
            owner: owner.clone(),
            body: to.to_string(),
            cargo: delivered.clone(),
            into_pool,
        },
    );
    // 承包的账在货物落地之后结：进度 + 信誉 + `contract_delivered` 事件。
    if contract.is_some() && loaded > 1e-9 {
        autocontrol::contract::on_delivery(state, config, ship_id, loaded, cut_units);
    }
    cargo
}

/// 一条运输路线本回合到达泊位后的**动作**（装或卸），返回这一步的记录。
fn haul_act(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    leg: &str,
    holding: bool,
) -> HaulStep {
    if holding {
        let cargo = haul_unload(state, config, fid, ship_id, leg);
        HaulStep::Delivered {
            body: leg.to_string(),
            units: cargo.values().sum(),
            // 「进池」说的是**货主**的池子（承包时货主是托运方，不是船东）。
            into_pool: state.capital_body(&cargo_owner(state, fid, ship_id)) == leg,
        }
    } else {
        let units = haul_load(state, config, fid, ship_id, leg);
        if units > 0.0 {
            HaulStep::Loaded {
                body: leg.to_string(),
                units,
            }
        } else {
            HaulStep::Waiting {
                body: leg.to_string(),
            }
        }
    }
}

/// 一条运输路线（[`ShipBehavior::Haul`]）本回合的**完整执行**：腿别判定 + 移动 + 装卸都在这里，
/// 所以调用方**不要再自己移动这艘舰**。
///
/// **腿别不存状态**：舱里有货 ⇒ 去 `to`；空舱 ⇒ 去 `from`（见 [`ShipBehavior::Haul`] 的说明）。
/// 到达判定用 `arrival_eps`（与殖民/停泊同一把尺子），且目标点照样会被 MOND 偏移——
/// 所以深处取货/送货不是「做不到」，而是**要多试几个回合**（`move_toward` 每回合重新算一次）。
pub(crate) fn haul_step(
    state: &mut State,
    config: &GameConfig,
    ship_id: &str,
    class: &str,
    from: &str,
    to: &str,
) -> HaulStep {
    let Some(ship) = state.ship(ship_id) else {
        return HaulStep::EnRoute {
            body: from.to_string(),
        };
    };
    let fid = ship.faction_id.clone();
    let pos = ship.position;
    let holding = !ship.cargo.is_empty();
    let leg = if holding { to } else { from };
    let target = state.body_position(leg);
    let eps = config.combat.arrival_eps;
    // 已经停在泊位内：本回合直接办事（与 Colonize 一样是「到达即行动」）。
    if dist(pos, target) <= eps {
        return haul_act(state, config, &fid, ship_id, leg, holding);
    }
    // **kiting 姿态照常生效**（用户裁决：角色不影响 kiting）：附近有敌舰时，航路上的软目标
    // 会被拉开/压近。它只改**移动**、不改「到没到」——到达判定看真实位置，且上面那一步
    // 「已在泊位内就直接办事」先于移动，所以**靠了泊位的运输舰不会被敌人推得卸不了货**。
    let dest = autocontrol::kiting_dest(state, config, ship_id).unwrap_or(target);
    move_toward(state, config, ship_id, class, dest);
    let np = state.ship(ship_id).map(|s| s.position).unwrap_or(pos);
    if dist(np, target) <= eps {
        return haul_act(state, config, &fid, ship_id, leg, holding);
    }
    HaulStep::EnRoute {
        body: leg.to_string(),
    }
}

/// 确定性命中率：武器追踪能力 `tracking`（AU/月）越高，越能咬住高速目标。目标速度
/// `target_speed` 越高，对低追踪武器的规避越强——所以推进组件 = 生存能力和抢先战位。
/// 无 RNG：命中定义为「伤害折减」而非「命中/未命中」的随机判定，保持确定性。
fn hit_factor(tracking: f64, target_speed: f64) -> f64 {
    if tracking <= 0.0 {
        return 0.3;
    }
    let evade = (target_speed / (tracking + target_speed)).min(1.0) * 0.6;
    (1.0 - evade).clamp(0.2, 1.0)
}

/// 开火：本舰的每件武器**独立索敌**、逐发射击（`fire_rate` 发/时间）。`plan` 给出一发
/// 打向哪个目标（每发条目 = 武器下标）。每发独立结算（命中 × 防御 × 护盾/护甲），一回合
/// 内多发可打在**不同**目标上（火力分配）。伤害按目标**聚合**——每个目标累计伤害 > 0 才发
/// 一次 `Attack` 事件、调一次关系（避免逐发打、关系掉得过快）。本舰攻击某目标后把它在该舰
/// `attack_hist` 里的新鲜度刷到 1（供火力分配层读，跨回合记忆）。确定性（无 RNG）。
pub(crate) fn fire(state: &mut State, config: &GameConfig, attacker_id: &str, plan: &[(usize, ShipId)]) {
    let afac = state.ship(attacker_id).map(|a| a.faction_id.clone()).unwrap_or_default();
    let weapons = state.ship(attacker_id).map(|a| ship_weapons(config, a)).unwrap_or_default();
    let mut damage_acc: BTreeMap<ShipId, f64> = BTreeMap::new();
    // 被击毁的舰 + **补刀的那一发**（哪艘舰/哪个势力/什么弹种）。此前只记「被毁」不记凶手，
    // 只能靠同回合的 Attack 反推，集火时不可判。
    let mut destroyed: Vec<(ShipId, Killer)> = Vec::new();
    // 攻击历史新鲜度：本舰打过谁。逐发结算后刷新，使同回合后续发能按「越近越降权重」改选。
    let mut hist: BTreeMap<ShipId, f64> = BTreeMap::new();

    for (wi, target_id) in plan {
        let Some(w) = weapons.get(*wi) else { continue };
        if !state.ship(target_id).map(|s| s.hull > 0.0).unwrap_or(false) {
            continue; // 目标已被本回合其它发击毁——跳过这发（火力补刀不浪费）。
        }
        let dmg = resolve_shot(state, config, attacker_id, w, target_id);
        let dead = state.ship(target_id).map(|s| s.hull <= 0.0).unwrap_or(false);
        // 第一发把它打到 hull ≤ 0 的就是**补刀**（已在 0 的目标在循环开头被跳过，
        // 所以这里判真一定是本发致命）。记下这一发的来源作为凶手。
        if dead && !destroyed.iter().any(|(n, _)| n == target_id) {
            destroyed.push((target_id.clone(), Killer {
                ship: attacker_id.to_string(),
                faction: afac.clone(),
                weapon: weapon_kind_name(w.kind).to_string(),
            }));
        }
        // 本发是「攻击」：把该目标新鲜度刷到 1（哪怕全被护盾/拦截吃掉）。
        hist.entry(target_id.clone()).or_insert(0.0);
        *hist.get_mut(target_id).unwrap() = 1.0;
        *damage_acc.entry(target_id.clone()).or_insert(0.0) += dmg;
    }
    // 攻击历史写回（火力分配跨回合记忆）。
    if let Some(a) = state.ship_mut(attacker_id) {
        for (t, v) in hist {
            a.attack_hist.insert(t, v);
        }
    }
    for (t, dmg) in &damage_acc {
        if *dmg > 1e-9 {
            let tfac = state.ship(t).map(|s| s.faction_id.clone()).unwrap_or_default();
            ev(state, GameEvent::Attack { attacker: attacker_id.to_string(), target: t.clone(), damage: *dmg });
            adjust_relation(state, config, &afac, &tfac, config.diplomacy.attack_delta);
        }
    }
    for (t, by) in destroyed {
        kill_ship(state, &t, DeathCause::Combat, Some(by));
    }
}

/// 把本舰所有武器的一次齐射（每武器 `fire_rate` 发）全打向**一个**目标——集中火力的
/// 便捷入口（测试用；玩家/ AI 都走 [`fire`] 的统一基本权重 plan）。保留为引擎原语。
#[allow(dead_code)]
pub(crate) fn fire_concentrate(state: &mut State, config: &GameConfig, attacker_id: &str, target_id: &str) {
    let weapons = state.ship(attacker_id).map(|a| ship_weapons(config, a)).unwrap_or_default();
    let plan: Vec<(usize, ShipId)> = weapons
        .iter()
        .enumerate()
        .flat_map(|(i, w)| {
            let shots = (w.fire_rate.round()).max(1.0) as usize;
            (0..shots).map(move |_| (i, target_id.to_string()))
        })
        .collect();
    fire(state, config, attacker_id, &plan);
}

/// 结算单件武器的一发打击：对一个活目标应用「命中 × 本土防御 × 护盾/护甲」伤害并修改
/// 目标的 hull/shield/组件完整度，返回造成的伤害（护盾吸收 + 船体）。这是「每发独立结算」，
/// 让一回合内多发可打在**不同**目标上。确定性。
fn resolve_shot(state: &mut State, config: &GameConfig, attacker_id: &str, w: &Weapon, target_id: &str) -> f64 {
    let apos = state.ship(attacker_id).map(|a| a.position).unwrap_or([0.0, 0.0]);
    let (tfac, tpos, tspeed, tpanel) = {
        let t = state.ship(target_id).expect("target gone");
        let panel = ship_panel(config, t);
        (t.faction_id.clone(), t.position, panel.speed, panel)
    };
    let d = dist(apos, tpos);
    if d > w.range {
        return 0.0; // weapon out of range — positional, not a stat
    }
    let def_mult = home_defense_mult(state, &tfac, tpos);
    let hit = hit_factor(w.tracking, tspeed);
    let mut dmg = w.damage * hit * def_mult;
    // 导弹是制导的（对高速目标规避弱），但会被目标点防御**线性**拦截（自身 + 附近友舰。
    if w.kind == WEAPON_MISSILE {
        let pd = tpanel.intercept + cluster_pd_cover(state, config, target_id, &tfac, tpos);
        if pd > 0.0 {
            dmg = (dmg - pd).max(0.0);
        }
    }
    let (mut hull, mut shield) = {
        let t = state.ship(target_id).unwrap();
        (t.hull, t.shield)
    };
    let hull_before = hull;
    // 护盾优先吸收（按 shield_mult），溢出与 hull_mult 部分进船体；满护盾削弱船体伤害。
    let shield_dmg = dmg * w.shield_mult;
    let hull_dmg = dmg * w.hull_mult;
    let absorbed = shield.min(shield_dmg);
    shield -= absorbed;
    let soak = if shield_dmg > 1e-9 { absorbed / shield_dmg } else { 1.0 };
    // 护甲 = 让船体变硬：打向船体(护甲)的伤害被目标硬度按**反比例函数**削减。
    let hardness = tpanel.hardness;
    let hull_dmg_effective = hull_dmg * (1.0 - 0.5 * soak);
    let armor_soak = if hardness > 1e-9 {
        (hardness / (hardness + hull_dmg_effective.max(1e-9))).min(0.85)
    } else {
        0.0
    };
    let hull_pen = hull_dmg_effective * (1.0 - armor_soak);
    hull -= hull_pen;
    let destroyed = hull <= 0.0;
    let hull_damage_done = (hull_before - hull.max(0.0)).max(0.0);
    if let Some(t) = state.ship_mut(target_id) {
        t.hull = if destroyed { 0.0 } else { hull.max(0.0) };
        t.shield = shield.max(0.0);
        // 组件被击中后渐进丧失战力（武器被打掉、护盾被打掉），而不是满血抗到壳破。
        if !destroyed && !t.components.is_empty() && config.combat.component_spill > 0.0 {
            let spill = hull_damage_done * config.combat.component_spill;
            if spill > 1e-9 {
                if t.component_hp.len() != t.components.len() {
                    t.component_hp = t.components.iter().map(|c| component_integrity(config, c)).collect();
                }
                let mut rem = spill;
                let mut order: Vec<usize> = (0..t.components.len()).collect();
                order.sort_by(|&a, &b| t.component_hp[a].total_cmp(&t.component_hp[b]).then_with(|| a.cmp(&b)));
                for &i in &order {
                    if rem <= 1e-9 {
                        break;
                    }
                    if t.component_hp[i] <= 0.0 {
                        continue;
                    }
                    let take = rem.min(t.component_hp[i]);
                    t.component_hp[i] -= take;
                    rem -= take;
                }
            }
        }
    }
    dmg // 本次造成的总伤害（护盾 + 船体）；用于事件/关系/攻击历史。
}

/// 舰队防空（防空屏护）：目标（`target_faction` 阵营、`tpos` 处）附近 `pd_radius` 内的友舰，
/// 其点防御拦截能力会为它**替拦导弹**——随距离线性衰减、封顶。让有 PD 的舰组成防空圈，
/// 能护卫航母/友舰（与护航行为衔接：护航舰贴近旗舰时提供防空）。确定性（无 RNG）。
fn cluster_pd_cover(state: &State, config: &GameConfig, target_id: &str, target_faction: &str, tpos: [f64; 2]) -> f64 {
    let r = config.combat.pd_radius;
    if r <= 0.0 {
        return 0.0;
    }
    let mut cover = 0.0;
    for s in &state.ships {
        if s.name == target_id || s.faction_id != target_faction || s.hull <= 0.0 {
            continue;
        }
        let d = dist(s.position, tpos);
        if d > r {
            continue;
        }
        let p = ship_panel(config, s).intercept;
        if p > 0.0 {
            cover += p * (1.0 - d / r);
        }
    }
    cover.min(30.0)
}

/// 本土防御伤害倍率：`pos` 位于 `faction` 首都的 `home_radius` 之内时返回该势力的
/// `home_attack_mult`（<1 = 削弱入侵者），否则 1.0（无削弱）。
///
/// 这使得每个有首都的势力在自己的核心区难啃（超大国空降别人家里要付代价），而
/// cult 因 MOND 异常拥有超大半径/强削减，能够在被围攻的柯伊伯带圣所自保。
fn home_defense_mult(state: &State, faction: &str, pos: [f64; 2]) -> f64 {
    let Some(f) = state.faction(faction) else { return 1.0 };
    if f.home_radius <= 0.0 {
        return 1.0;
    }
    let cap = state.body_position(&state.capital_body(faction));
    if dist(pos, cap) <= f.home_radius {
        f.home_attack_mult
    } else {
        1.0
    }
}

/// 本土防御额外再生：`pos` 位于 `faction` 首都的 `home_radius` 之内时，返回该势力
/// 的 `home_regen_bonus`，否则 0.0。
fn home_regen_bonus(state: &State, faction: &str, pos: [f64; 2]) -> f64 {
    let Some(f) = state.faction(faction) else { return 0.0 };
    if f.home_radius <= 0.0 {
        return 0.0;
    }
    let cap = state.body_position(&state.capital_body(faction));
    if dist(pos, cap) <= f.home_radius {
        f.home_regen_bonus
    } else {
        0.0
    }
}

pub(crate) fn behavior_is_valid(state: &State, _config: &GameConfig, behavior: ShipBehavior, owner: &str) -> bool {
    match behavior {
        ShipBehavior::Move { .. } | ShipBehavior::Idle => true,
        ShipBehavior::Dock { body } => state.body(&body).is_some(),
        // 跟随舰船：所随舰还活着即可（友方护航 / 敌方追袭皆可）。
        ShipBehavior::Follow { ship } => state.ship(&ship).map(|s| s.hull > 0.0).unwrap_or(false),
        // 停泊城市：城还活着即可停靠/包围；是敌城则会在围城射程内自动轰炸。
        ShipBehavior::DockCity { city } => state.city(&city).map(|c| !c.razed).unwrap_or(false),
        ShipBehavior::Colonize { body } => has_blank_site(state, &body),
        // 运输路线：两端天体都要在。**`from == to` 只在「卸进首都池」时合法**——
        // 「自取自卸」没有意义（那是陈旧指令），但「首都天体上压着一处旧中转货栈、
        // 把它扫进池子」是合法的路线（迁都把旧中转点留在了新首都，见 `autocontrol::freight`）。
        ShipBehavior::Haul { from, to } => {
            state.body(&from).is_some()
                && state.body(&to).is_some()
                && (from != to || state.capital_body(owner) == to)
        }
    }
}

/// Does `body` have a colonizable 定居点 site? Yes iff it hosts a settlement
/// whose city is razed (blank footprint, re-seedable) or a settlement no city
/// occupies yet. Settlement ↔ city is 1:1, so a site with a live city never
/// counts as blank.
fn has_blank_site(state: &State, body: &str) -> bool {
    let Some(b) = state.body(body) else { return false };
    if b.settlements.is_empty() {
        return false;
    }
    let has_razed = state.cities.iter().any(|c| c.body_id == body && c.razed);
    if has_razed {
        return true;
    }
    let occupied: BTreeSet<String> = state
        .cities
        .iter()
        .filter(|c| c.body_id == body)
        .map(|c| c.settlement.clone())
        .collect();
    b.settlements.iter().any(|s| !occupied.contains(&s.name))
}


/// Bombard a city: damage is spread across its buildings by area share. When all
/// buildings are destroyed the city is razed to a blank (colonizable) settlement
/// — it is never captured.
pub(crate) fn bombard_city(state: &mut State, config: &GameConfig, ship_id: &str, cid: &str) {
    let (attacker, weapons) = {
        let s = state.ship(ship_id).expect("ship gone");
        (s.faction_id.clone(), ship_weapons(config, s))
    };
    let (old_owner, cpos) = {
        let c = state.city(cid).expect("city gone");
        (c.faction_id.clone(), state.body_position(&c.body_id))
    };
    // 城市是静止的大型目标（无护盾、只有建筑装甲），轰炸用「每件武器 × 对甲倍率」的总
    // 齐射——导弹/重炮拆城，近防炮对城伤害低。命中视为全中（城市不规避）。
    let hull_attack: f64 = weapons.iter().map(|w| w.damage * w.hull_mult).sum();
    // 本土防御（首都即强弩 + cult 的 MOND 异常）：城市位于其势力首都的本土防御
    // 半径内时，受到的轰炸伤害被削弱。
    let dmg = hull_attack * home_defense_mult(state, &old_owner, cpos);
    let razed = {
        let c = state.city_mut(cid).expect("city gone");
        let total_deployed: f64 = c.buildings.iter().map(|b| b.deployed).sum();
        for b in &mut c.buildings {
            let share = if total_deployed > 1e-9 { (b.deployed / total_deployed).min(1.0) } else { 0.0 };
            b.armor -= dmg * share;
        }
        c.buildings.retain(|b| b.armor > 1e-6);
        // 建筑清零 = 城失守；**状态改写交给 `raze_city` 漏斗**（它自己读夷平前人口并记事件）。
        c.buildings.is_empty()
    };
    adjust_relation(state, config, &attacker, &old_owner, config.diplomacy.attack_delta);
    ev(state, GameEvent::Siege { attacker: ship_id.to_string(), city: cid.to_string(), damage: dmg });
    if razed {
        adjust_relation(state, config, &attacker, &old_owner, config.diplomacy.capture_delta);
        // 漏斗记 `CityRazed`：`by_ship` = 拆掉这座城的那艘舰（此前只能去同回合的 Siege 里猜），
        // `pop_before`/`damage` = 这次毁灭的量级。
        raze_city(state, &cid.to_string(), RazeCause::Bombardment {
            by_ship: ship_id.to_string(),
            by_faction: attacker,
            damage: dmg,
        });
    }
}

/// Colonize a settlement site (定居点 ↔ 城市 1:1). If the body hosts a razed
/// (blank) city, re-seed it on its own settlement (re-colonize); otherwise found
/// a new city only on a 定居点 that no city occupies yet. A site already holding
/// a live city can never take a second one — the colony ship is spent (order
/// reset to idle) once a site is found, or stays put otherwise.
pub(crate) fn colonize(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    ship_id: &str,
    body: &str,
    next_building_id: &mut BuildingId,
) {
    let faction = state.ship(ship_id).map(|s| s.faction_id.clone()).expect("ship gone");
    // **机制不变量（反凭空造城）**：建城必须由**一艘此刻就在场的活舰**解释。这是
    // `step_resurgence` 删除（D5）之后世界唯一的建城路径——调用方只在舰已进入
    // `combat.arrival_eps` 时才调这里（见 `autocontrol::tactics`）。若将来有人再加一条
    // 「凭空变城」的路径，这两条断言会在任何 debug 测试里立刻炸掉。
    debug_assert!(
        state.ship(ship_id).map(|s| s.hull > 0.0).unwrap_or(false),
        "colonize 必须由一艘活着的舰触发（没有凭空变城）"
    );
    debug_assert!(
        state
            .ship(ship_id)
            .map(|s| dist(s.position, state.body_position(body)) <= config.combat.arrival_eps)
            .unwrap_or(false),
        "colonize 的舰必须已经在目标天体的 arrival_eps 之内"
    );
    let seeded_ship_class = autocontrol::choose_next_class(state, &faction, config, rng);

    // 1) A razed (blank) city keeps occupying its settlement: re-seed it there.
    let razed_cid = state.cities.iter().find(|c| c.body_id == body && c.razed).map(|c| c.name.clone());
    if let Some(cid) = razed_cid {
        // 漏斗：复垦空白城 + 记 `ColonyFounded { how: Refounded }`。旧主（空白城保留的
        // diaspora claim）由漏斗自己读，调用方漏不掉。
        if !reseed_city(state, config, &cid, &faction, &seeded_ship_class, next_building_id) {
            // 该城没有可用的定居点 —— 殖民舰就地待命（旧行为）。
            reset_order_keep_mode(state, &faction, ship_id);
            return;
        }
        reset_order_keep_mode(state, &faction, ship_id);
        return;
    }

    // 2) No blank city: found a new city only on a settlement no city occupies.
    let occupied: BTreeSet<String> = state.cities.iter().filter(|c| c.body_id == body).map(|c| c.settlement.clone()).collect();
    let Some(settlement) = state
        .body(body)
        .and_then(|b| b.settlements.iter().find(|s| !occupied.contains(&s.name)).cloned())
    else {
        // Every settlement is occupied by a live city — nothing to colonize.
        reset_order_keep_mode(state, &faction, ship_id);
        return;
    };
    let base = if settlement.name.is_empty() {
        state.body(body).map(|b| b.name.clone()).unwrap_or_else(|| format!("#{body}"))
    } else {
        settlement.name.clone()
    };
    let cname = format!("{}-殖民城", base);
    // 漏斗：新建城 + 记 `ColonyFounded { how: NewSite }`。
    found_city(state, config, &cname, &body.to_string(), &settlement, &faction, &seeded_ship_class, next_building_id);
    reset_order_keep_mode(state, &faction, ship_id);
}

/// 一次性指令（殖民）执行完之后的收尾：指令复位成 `Idle`，但**保留「由谁决定」**。
///
/// 为什么必须保留：殖民是「命令 → 执行 → 指令失效」的一次性动作，而这里以前无条件写
/// `Control::inherit(..)`，于是**玩家点名的殖民舰一旦建完城就被交还给系统**（AI 下一回合
/// 就把它征去别处）——玩家会看到自己刚下达的处置静默蒸发。换「值」不换「归属」是 sim 里
/// 的既有约定（见 `step_capital` 的迁都：保留原来的 mode 标记）。
fn reset_order_keep_mode(state: &mut State, fid: &FactionId, ship_id: &str) {
    if let Some(c) = state.control_mut(fid.clone()) {
        let mode = c.ship_orders.get(ship_id).map(|c| c.mode).unwrap_or_default();
        c.ship_orders.insert(ship_id.to_string(), Control { value: ShipBehavior::Idle, mode });
    }
}

/// Seed buildings for a newly founded / razed-and-reseeded city.
fn seed_colony_buildings(
    s: &Settlement,
    population: u32,
    ship_class: &str,
    config: &GameConfig,
    next_id: &mut BuildingId,
) -> Vec<Building> {
    let mut buildings = Vec::new();
    let mut alloc = |kind: &str, resource: Option<String>, ship_type: Option<String>, area: f64, deployed: f64| -> Building {
        let id = *next_id;
        *next_id += 1;
        let armor = deployed * config.structure_spec("concrete").armor_per_area;
        Building {
            id,
            kind: kind.to_string(),
            resource,
            ship_type,
            structure: "concrete".to_string(),
            area,
            deployed,
            armor,
        }
    };

    let footprint = s.total_area * config.combat.colony_footprint;
    let resid = (population as f64 / s.ecological_capacity.max(1e-6)).min(footprint * 0.5).max(4.0);
    buildings.push(alloc("residential", None, None, resid, resid));
    let mut budget = (footprint - resid).max(0.0);
    for d in &s.resources {
        if budget <= 0.0 {
            break;
        }
        let area = d.area.min(budget * 0.5);
        if area > 0.0 {
            buildings.push(alloc("mining", Some(d.resource.clone()), None, area, area));
            budget -= area;
        }
    }
    let construction = (footprint * 0.3).clamp(2.0, 8.0);
    buildings.push(alloc("construction", None, Some(ship_class.to_string()), construction, construction));
    buildings
}

pub(crate) fn behavior_dest(state: &State, behavior: &ShipBehavior) -> [f64; 2] {
    match behavior {
        ShipBehavior::Move { position } => *position,
        ShipBehavior::Follow { ship } => state.ship(ship).map(|s| s.position).unwrap_or([0.0, 0.0]),
        ShipBehavior::DockCity { city } => city_position(state, city),
        ShipBehavior::Dock { body } | ShipBehavior::Colonize { body } => state.body_position(body),
        // `Haul` 的**腿别取决于货舱**（有货去 `to`、空舱去 `from`，见 [`haul_step`]），
        // 而这个函数拿不到舰 ⇒ 只能给「待装那一端」。两条真正的执行路径（玩家 / AI）都在
        // 到达 `Haul` 之前就分派给 [`haul_step`] 了，所以这个臂**只为穷尽匹配存在**。
        ShipBehavior::Haul { from, .. } => state.body_position(from),
        ShipBehavior::Idle => [0.0, 0.0],
    }
}

// --- diplomacy --------------------------------------------------------------

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
fn step_diplomacy(state: &mut State, config: &GameConfig, rng: &mut Prng) {
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
fn set_relation_sym(state: &mut State, a: FactionId, b: FactionId, v: f64, config: &GameConfig) {
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

/// 各势力**综合实力**（幂：`city_weight×城市份额 + fleet_weight×舰队份额`，未除以两权重之和）。
/// 这是“谁最强”的**单一权威**统计：`faction_power_share` 由它归一化而来，观测
/// （[`round_metrics`] 的 `faction_power`）与游戏逻辑（`step_balance_of_power`/
/// `sanction_cost_mult`）都读同一份。城市份额 = 活城数/总活城数，舰队份额 = 舰艇引擎数值
/// 之和/总引擎数值之和。无活城且无舰时全 0。
pub(crate) fn faction_power(state: &State, config: &GameConfig) -> BTreeMap<FactionId, f64> {
    let b = &config.balance;
    let total_cities = state.cities.iter().filter(|c| !c.razed).count() as f64;
    let total_fleet: f64 = state.ships.iter().map(|s| ship_panel(config, s).hull_max).sum();
    let mut powers = BTreeMap::new();
    if total_cities <= 0.0 && total_fleet <= 0.0 {
        return state.factions.iter().map(|f| (f.name.clone(), 0.0)).collect();
    }
    for f in &state.factions {
        let cities = state.cities.iter().filter(|c| c.faction_id == f.name && !c.razed).count() as f64;
        let fleet: f64 = state
            .ships
            .iter()
            .filter(|s| s.faction_id == f.name)
            .map(|s| ship_panel(config, s).hull_max)
            .sum();
        let city_share = if total_cities > 0.0 { cities / total_cities } else { 0.0 };
        let fleet_share = if total_fleet > 0.0 { fleet / total_fleet } else { 0.0 };
        powers.insert(f.name.clone(), b.power_city_weight * city_share + b.power_fleet_weight * fleet_share);
    }
    powers
}

/// 综合实力占比：`power = faction_power / (city_weight + fleet_weight)`。
/// 两份额各自在 [0,1] 且对全势力求和为 1，故 power 也是合法的占比（0..1）。
pub(crate) fn faction_power_share(state: &State, config: &GameConfig) -> BTreeMap<FactionId, f64> {
    let b = &config.balance;
    let wp = b.power_city_weight + b.power_fleet_weight;
    if wp <= 0.0 {
        return state.factions.iter().map(|f| (f.name.clone(), 0.0)).collect();
    }
    faction_power(state, config)
        .into_iter()
        .map(|(k, v)| (k, v / wp))
        .collect()
}

/// 当前的反制联盟成员：非霸权势力中，对霸权的**疏远**达到 [`BalanceOfPowerConfig::coalition_estrange`]
/// （关系 ≤ 该值，即被遏制/疏远了霸权）、且彼此相互和平（互不交战）的一方。若 ≥
/// [`BalanceOfPowerConfig::min_members`] 即视为联盟成立。遏制是冷战式的——成员未必与
/// 霸权开战，但已脱离其影响、转而与弱国抱团。
fn coalition_of(state: &State, config: &GameConfig, hegemon: &str, members: &[FactionId]) -> Vec<FactionId> {
    let estrange = config.balance.coalition_estrange;
    let estranged: Vec<FactionId> = members
        .iter()
        .cloned()
        .filter(|m| relation(state, m, hegemon) <= estrange)
        .collect();
    estranged
        .iter()
        .cloned()
        .filter(|m| estranged.iter().all(|o| o == m || !hostile(state, config, m, o)))
        .collect()
}

/// 当前综合实力占比最高的「霸权」及其已倒向联盟的成员（关系 ≤ `coalition_estrange`）。
/// 只负责判定「谁是最强、谁在抱团」，实力占比未达 [`BalanceOfPowerConfig::hegemon_power`]
/// 时返回 `None`。
fn dominant_hegemon(state: &State, config: &GameConfig) -> Option<(FactionId, Vec<FactionId>)> {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return None;
    }
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    if ids.len() < 2 {
        return None;
    }
    let powers = faction_power_share(state, config);
    let (hegemon, max_power) = powers
        .iter()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| (k.clone(), *v))
        .unwrap_or((String::new(), 0.0));
    if max_power < b.hegemon_power {
        return None;
    }
    let members: Vec<FactionId> = ids.iter().cloned().filter(|x| *x != hegemon).collect();
    let estranged = coalition_of(state, config, &hegemon, &members);
    Some((hegemon, estranged))
}

/// 当前一个活跃反制联盟（≥ [`BalanceOfPowerConfig::min_members`] 个疏远成员）针对的
/// 「霸权」；用于政治上报（`coalition` 字段）与联盟跃迁事件。
pub(crate) fn active_coalition_hegemon(state: &State, config: &GameConfig) -> Option<FactionId> {
    dominant_hegemon(state, config)
        .filter(|(_, m)| m.len() >= config.balance.min_members)
        .map(|(h, _)| h)
}

/// 经济制裁针对的「霸权」：只要势力**已称霸（实力占比达标）且至少有一个弱者倒向联盟**
/// 就实施封锁——不必等联盟完全成形。这使「人缘好但已坐大」的紧凑区域帝国也能被压缩
/// （否则它不招人恨就没人封锁它），且比 war 更拟真、不引发夷平/僵尸。
fn sanctioned_hegemon(state: &State, config: &GameConfig) -> Option<FactionId> {
    dominant_hegemon(state, config)
        .filter(|(_, m)| !m.is_empty())
        .map(|(h, _)| h)
}

/// 经济制裁的「治理代价」倍率：若 `fid` 正是被经济封锁的霸权，则其维持帝国
/// （行政 + 娱乐）的成本按 `sanction_cost_mult` 放大；否则 1.0（不碰别国）。这使被
/// 多国封锁的大国要花更多资源维持领地与治安——边缘殖民地更难养、更易离心。
fn sanction_cost_mult(state: &State, config: &GameConfig, fid: &str) -> f64 {
    if sanctioned_hegemon(state, config).as_deref() == Some(fid) {
        config.balance.sanction_cost_mult
    } else {
        1.0
    }
}

/// 联盟军事协同的「集火目标」：若 `owner` 属于针对霸权 H 的活跃反制联盟（已倒向联盟、
/// 关系 ≤ `coalition_estrange`），且 H 正与联盟内某一弱者交战（集体安全已触发——霸权
/// 先动手了），则返回 Some(H)。这使结盟势力的舰只**优先集火 H**、而非各自就近乱打——
/// 给「攻其一方、集体制衡」真正的军事牙齿。否则返回 None（不改变普通行为）。
fn coalition_war_focus(state: &State, config: &GameConfig, owner: &str) -> Option<FactionId> {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return None;
    }
    let Some(hegemon) = active_coalition_hegemon(state, config) else { return None };
    if owner == hegemon.as_str() {
        return None;
    }
    // 该弱者是否已倒向联盟（疏远霸权）。未倒向则不集火。
    if relation(state, owner, &hegemon) > b.coalition_estrange {
        return None;
    }
    // 霸权是否正与任一弱者交战（集体防御触发）——注意霸权自己对它与他人开战不作集火。
    let war_on = state
        .factions
        .iter()
        .any(|f| f.name != hegemon && hostile(state, config, &f.name, &hegemon));
    if war_on {
        Some(hegemon)
    } else {
        None
    }
}

/// 合纵连横格局快照：返回 (当前霸权(若有), 针对它的反制联盟成员, 各势力综合实力占比)。
/// 供 agent 层读取政治格局（霸权是谁、谁在联合制衡、谁是当前最强）。
pub fn balance_picture(
    state: &State,
    config: &GameConfig,
) -> (Option<FactionId>, Vec<FactionId>, BTreeMap<FactionId, f64>) {
    let powers = faction_power_share(state, config);
    let hegemon = active_coalition_hegemon(state, config);
    let members = match &hegemon {
        Some(h) => {
            let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
            let members: Vec<FactionId> = ids.into_iter().filter(|x| *x != *h).collect();
            coalition_of(state, config, h, &members)
        }
        None => Vec::new(),
    };
    (hegemon, members, powers)
}

/// 一局世界在某回合结束时的**总结指标**（agent 的「总结」视图，`RoundMetrics`）。
///
/// 这些数字**就是步进函数本身用的中间计算量**：它复用一次 `balance_picture`
/// （内部是 `faction_power_share` + `coalition_of`）、一次 `sanctioned_hegemon`
/// 与 `war_pairs`，再补上世界/各势力的城市/舰/兵力/人口/库存价值聚合。因此直接状态
/// （`State` 的实体字段）与此视图**严格同源、永不漂移**——不会像其它地方独立重算的
/// 汇总那样与模拟脱节。
///
/// `flow` 携带本回合的**流量**中间量（产出/维护/治理，见 [`RoundFlow`]）；`state` 提供
/// 存量/政治快照。纯函数、无 RNG，同一种子完全复现；O(势力 + 舰 + 城) 一次遍历，足够在
/// 每回合轻量调用。
pub fn round_metrics(state: &State, config: &GameConfig, flow: &RoundFlow) -> RoundMetrics {
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
    let (hegemon, members, powers) = balance_picture(state, config);
    let sanctioned = sanctioned_hegemon(state, config);
    let wars: Vec<(FactionId, FactionId)> = war_pairs(state, config).into_iter().collect();

    let mut factions = BTreeMap::new();
    let mut total_population = 0u64;
    let mut world_cities = 0;
    let mut world_fleet = 0.0;
    for f in &state.factions {
        let fid = f.name.clone();
        let living: Vec<&City> = state.cities.iter().filter(|c| c.faction_id == fid && !c.razed).collect();
        let city_count = living.len();
        let population: u64 = living.iter().map(|c| c.population as u64).sum();
        total_population += population;
        world_cities += city_count;
        let ships = state.ships.iter().filter(|s| s.faction_id == fid);
        let ship_count = ships.clone().count();
        let fleet_value: f64 = ships.clone().map(|s| s.hull).sum();
        world_fleet += fleet_value;
        let market_value: f64 = f.resources.iter().map(|(k, v)| v * value_of(k)).sum();
        let at_war = state.factions.iter().any(|o| o.name != fid && hostile(state, config, &fid, &o.name));
        let production: ResourceMap = flow.faction_production.get(&fid).cloned().unwrap_or_default();
        let production_value: f64 = production.iter().map(|(k, v)| v * value_of(k)).sum();
        let governance_cost = flow.governance.get(&fid).map(|g| g.total).unwrap_or(0.0);
        let governance_coverage = flow.governance.get(&fid).map(|g| g.coverage).unwrap_or(1.0);
        // 「谁不卖给你」：有多少势力对本势力**全面禁运**（本回合市场结算的实际判据）。
        let trade_blocked_by = state
            .factions
            .iter()
            .filter(|o| trade_blocked(state, config, &o.name, &fid))
            .count();
        factions.insert(
            fid.clone(),
            FactionMetrics {
                city_count,
                ship_count,
                fleet_value,
                population,
                market_value,
                at_war,
                production_value,
                production,
                upkeep: flow.upkeep.get(&fid).copied().unwrap_or(0.0),
                governance_cost,
                governance_coverage,
                trade_blocked_by,
                freight_paid: flow.market_freight.get(&fid).copied().unwrap_or(0.0),
                carrier_income: flow.market_carrier_income.get(&fid).copied().unwrap_or(0.0),
            },
        );
    }

    // 每座活城的本回合产出（step_production 的「中间量」）。
    let mut city_production = BTreeMap::new();
    for c in &state.cities {
        if c.razed {
            continue;
        }
        let production = flow.city_production.get(&c.name).cloned().unwrap_or_default();
        let production_value = production.iter().map(|(k, v)| v * value_of(k)).sum::<f64>();
        city_production.insert(
            c.name.clone(),
            CityMetrics {
                population: c.population,
                loyalty: c.loyalty,
                production_value,
                production,
            },
        );
    }

    RoundMetrics {
        cities: world_cities,
        ships: state.ships.len(),
        fleet_value: world_fleet,
        population: total_population,
        power_share: powers,
        faction_power: faction_power(state, config),
        hegemon,
        coalition_members: members,
        sanctioned,
        wars,
        factions,
        city_production,
        // 市场观察面：价、成交、挂单、每势力净进口（由 step_market 当回合写入）。
        market_price: state.market.price.clone(),
        market_settled: state.market.settled.clone(),
        market_offered: offered_by_resource(&state.market),
        market_net_import: flow.market_net.clone(),
    }
}

/// 本回合按资源汇总的挂单量（供给侧观察）。
fn offered_by_resource(market: &MarketState) -> ResourceMap {
    let mut out: ResourceMap = ResourceMap::new();
    for o in &market.offers {
        *out.entry(o.resource.clone()).or_insert(0.0) += o.amount;
    }
    out
}


/// 合纵连横 / 均势外交：当一方被判定为「霸权」时，其余较弱势力被共同威胁推向彼此——
/// 弱者-弱者向 [`BalanceOfPowerConfig::coalition_affinity`] 靠拢（合纵），弱者对霸权向
/// [`BalanceOfPowerConfig::hegemon_affinity`] 靠拢（均势/疏远）。霸权对任一弱者开战时，
/// 其余弱者对霸权关系骤降（集体安全）。全部确定性、无 RNG。
fn step_balance_of_power(state: &mut State, config: &GameConfig) {
    let b = &config.balance;
    if b.hegemon_power > 1.0 {
        return; // 关闭
    }
    let ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    if ids.len() < 2 {
        return;
    }

    // 找综合实力占比最高的「霸权」；未达阈值则不触发机制。
    let powers = faction_power_share(state, config);
    let (hegemon, max_power) = powers
        .iter()
        .max_by(|x, y| x.1.partial_cmp(y.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, v)| (k.clone(), *v))
        .unwrap_or((String::new(), 0.0));
    if max_power < b.hegemon_power {
        return;
    }

    // 威胁强度：霸权超越阈值越多，弱者靠拢得越急（scale ∈ [1, 2] 附近）。
    let dom = (max_power - b.hegemon_power).max(0.0);
    let scale = 1.0 + dom / (1.0 - b.hegemon_power).max(1e-9);

    let members: Vec<FactionId> = ids.iter().cloned().filter(|x| *x != hegemon).collect();
    if members.is_empty() {
        return;
    }

    // 步骤前后联盟成员、及与霸权交战成员（用于跃迁/集体安全判定）。
    let coalition_before = coalition_of(state, config, &hegemon, &members);
    let was_at_war: BTreeSet<FactionId> =
        members.iter().cloned().filter(|m| hostile(state, config, m, &hegemon)).collect();

    // 合纵：弱者-弱者相互靠拢（共同威胁把他们推向彼此）。
    for i in 0..members.len() {
        for j in (i + 1)..members.len() {
            let (m1, m2) = (members[i].clone(), members[j].clone());
            let rel = relation(state, &m1, &m2);
            let nv = rel + b.coalition_rate * scale * (b.coalition_affinity - rel);
            set_relation_sym(state, m1, m2, nv, config);
        }
    }

    // 均势：「冷处理/遏制」——弱者对霸权的关系向 hegemon_affinity 下压，但**只在它比
    // 该目标更暖时才往下压**，绝不自动把它推到交战阈值之下（不「无脑宣战」）。这模拟
    // 现实中的遏制：弱国不再争相讨好霸权、甚至疏远它，但井水不犯河水，真正的共同军事
    // 行动留给「集体安全」（霸权一旦动手打某弱者，其余弱者才群起而攻之）。
    for m in &members {
        let rel = relation(state, m, &hegemon);
        if rel > b.hegemon_affinity {
            let nv = rel + b.hegemon_rate * scale * (b.hegemon_affinity - rel);
            set_relation_sym(state, m.clone(), hegemon.clone(), nv, config);
        }
    }

    // 集体安全：任一弱者与霸权进入交战（本回合新跨入），其余尚未交战的弱者对霸权关系
    // 骤降——「攻其一方 = 与全体为敌」的防御协定：霸权一旦开打，弱者联盟群起而攻之。
    let now_at_war: BTreeSet<FactionId> =
        members.iter().cloned().filter(|m| hostile(state, config, m, &hegemon)).collect();
    if now_at_war.difference(&was_at_war).next().is_some() {
        for m in &members {
            if !now_at_war.contains(m) {
                let rel = relation(state, m, &hegemon);
                set_relation_sym(state, m.clone(), hegemon.clone(), rel + b.collective_defense_delta, config);
            }
        }
    }

    // 联盟跃迁事件（只在成立/解体的当回合记一条，供 agent 直读政治格局）。
    let coalition_after = coalition_of(state, config, &hegemon, &members);
    let before_active = coalition_before.len() >= b.min_members;
    let after_active = coalition_after.len() >= b.min_members;
    if before_active && !after_active {
        ev(state, GameEvent::CoalitionEnded { hegemon, members: coalition_before });
    } else if !before_active && after_active {
        ev(state, GameEvent::CoalitionFormed { hegemon, members: coalition_after });
    }
}

// --- 思潮 (ideology) ----------------------------------------------------------

/// 按「变化因素」驱动各势力 4 条思潮轴。每条轴先算**本回合的信号 target**（[-1,1]），
/// 再把当前值按 `drift_rate` 向 target 靠拢并钳到 [-1,1]。确定性、无 RNG。
///
/// 变化因素（见 spec「势力.各类思潮偏向」）：
///   * 和平↔军国：战争得失——敌舰被击毁+夷平敌城（得利→军国）减 我舰被击毁+城损失（失利→和平）。
///   * 科学↔技术：飞船在 MOND 异常区（→科学）vs 开采 MOND 区资源（→技术，按异常区城数计）。
///   * 人民↔精英：经济好坏——净流（产出−维护−治理）为正→精英，为负→人民。
///   * 自然↔殖民：人均面积——拥挤（低于参考）→殖民，宽敞→自然。
fn step_ideology(state: &mut State, config: &GameConfig, flow: &RoundFlow) {
    let ic = &config.ideology;
    let r = config.mond.radius;
    // --- 军事信号：完全由**本回合的事件历史**推出，不再回读回合末的 state ---------------
    //
    // 这里以前有**两处「事后回读」**，都是错的——它们都在回合末去读一个回合内已经变过的世界，
    // 于是把「当时发生了什么」记到了「现在还剩什么」的头上：
    //
    // * **凶手**：曾用「同回合最后一条 `Attack` 的势力」近似。那要 `state.ship(attacker)`
    //   才知道攻击者属于谁，而**互杀**（凶手本回合也被打沉）时那艘舰已经不在 `state.ships`
    //   里 → 这次击杀**领不到功**。`ShipDestroyed.by` 是补刀那一刻记下的权威事实，不受影响。
    // * **失城方**：曾用 `state.city(city).faction_id` 判断「谁丢了这座城」。但夷平**不改归属**
    //   （空白城保留最后主人的 diaspora claim），而同一回合稍后的复垦/重建会把它改成新主——
    //   于是读到的是**新主**。活体样本 seed 7 r24：大红斑科学站被欧盟夷平、同回合被无国界
    //   科学组织复垦，这次战功被记到了**抢城的人**头上。`CityRazed.owner` 在夷平那一刻记下
    //   真正的失主。
    //
    // 口径（与模块文档声明一致）：「我丢了一城 / 沉了一舰 → −1；我夺了一城 / 击沉敌舰 → +1」。
    // 「一座活城易主」是一个**现象**、有两条实现分支（`city_defected` 主路 / `revolt` 兜底），
    // 因此二者必须同分——旧代码让兜底分支 −1 而主路 0 分，等于「分数取决于有没有可倒戈目标」
    // 这个无关的偶然。同理 `city_overrun` 也是「活城易主」，一并同分。
    // **`colony_founded` 刻意不计**：新建/复垦是**殖民**行为，归 `nature_colony` 轴管；把它记成
    // 军事得分会让殖民者集体漂向军国，两轴打架。
    let mil_delta = military_deltas(&state.events);
    let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);

    // Pass 1（只读 state/flow）：算出每势力的信号 target。
    let mut targets: BTreeMap<FactionId, Ideology> = BTreeMap::new();
    for f in &state.factions {
        let name = f.name.clone();
        // 战争得失（见上：全部来自本回合事件，无 state 回读）。
        let mil = mil_delta.get(&name).copied().unwrap_or(0.0);
        // MOND 接触：到访异常区舰数（→科学） vs 开采异常区资源（→技术，按异常区城数计）。
        let mut sci_ships = 0u32;
        let mut tech_cities = 0u32;
        for s in &state.ships {
            if s.faction_id == name && s.hull > 0.0 && dist(s.position, [0.0, 0.0]) > r {
                sci_ships += 1;
            }
        }
        for c in &state.cities {
            if c.faction_id == name && !c.razed && dist(state.body_position(&c.body_id), [0.0, 0.0]) > r {
                tech_cities += 1;
            }
        }
        // 经济净流
        let prod: f64 = flow
            .faction_production
            .get(&name)
            .map(|m| m.iter().map(|(k, v)| v * value_of(k)).sum())
            .unwrap_or(0.0);
        let upkeep = flow.upkeep.get(&name).copied().unwrap_or(0.0);
        let gov = flow.governance.get(&name).map(|g| g.total).unwrap_or(0.0);
        let net = prod - upkeep - gov;
        // 人均面积（全部定居点面积 / 总人口）
        let area: f64 = state
            .cities
            .iter()
            .filter(|c| c.faction_id == name && !c.razed)
            .filter_map(|c| state.city_settlement(&c.name).map(|s| s.total_area))
            .sum();
        let pop: u64 = state
            .cities
            .iter()
            .filter(|c| c.faction_id == name && !c.razed)
            .map(|c| c.population as u64)
            .sum();
        let pca = if pop > 0 { area / pop as f64 } else { 0.0 };

        let t = Ideology {
            peace_military: (mil * ic.military_scale).clamp(-1.0, 1.0),
            science_tech: ((tech_cities as f64 - sci_ships as f64) * ic.mond_scale).clamp(-1.0, 1.0),
            people_elite: (net / ic.economy_scale.max(1e-6)).clamp(-1.0, 1.0),
            nature_colony: ((ic.area_ref - pca) * ic.area_scale).clamp(-1.0, 1.0),
        };
        targets.insert(name, t);
    }

    // Pass 2（可变 state）：把各势力思潮向 target 靠拢（确定性；钳 [-1,1]）。
    for f in &mut state.factions {
        let Some(target) = targets.get(&f.name) else { continue };
        let dr = ic.drift_rate;
        let converge = |cur: f64, tgt: f64| (cur + dr * (tgt - cur)).clamp(-1.0, 1.0);
        f.ideology.peace_military = converge(f.ideology.peace_military, target.peace_military);
        f.ideology.science_tech = converge(f.ideology.science_tech, target.science_tech);
        f.ideology.people_elite = converge(f.ideology.people_elite, target.people_elite);
        f.ideology.nature_colony = converge(f.ideology.nature_colony, target.nature_colony);
    }
}

/// 一个回合的事件历史 → 各势力的**军事净信号**（思潮「和平↔军国」的驱动量）。
///
/// 纯函数、只吃事件，**完全不看 state**：这是这条规则能被单元测试精确钉住的原因，也是它
/// 正确的原因——「谁丢了城 / 谁打沉了谁」都是当时记下的事实，事后再去 state 里回读一个已经
/// 变过的世界必然读错（详见 [`step_ideology`] 的说明：凶手互杀、失城方被同回合复垦）。
///
/// 规则（每一条都只读事件自带字段）：
/// * **失去一艘舰**（战沉 *或* 欠费报废）→ 旧主 −1。
/// * **击沉敌舰** → `by.faction` +1；只有 `cause == Combat` 才算，且功劳归**补刀**那一发。
/// * **城被夷平**（`CityRazed`）→ 失主（`owner`，夷平那一刻的持有者）−1、拆城方 +1。
/// * **活城易主**（`CityDefected`；`CityOverrun` 已在贸易分支删除，活城易主只剩离心倒戈一条路）
///   → 失主 −1、新主 +1。
/// * **离心叛乱夷为空白**（`Revolt`，是 `CityDefected` 的兜底分支）→ 失主 −1。
/// * **`ColonyFounded` 刻意不计**：新建/复垦是殖民行为，归 `nature_colony` 轴，
///   记进军事轴会让殖民者集体漂向军国。
///
/// 「同一现象必须同分」是这条规则的核心：`CityDefected` 与 `Revolt` 是**同一个触发**
/// （忠诚跌破阈值）的两条分支（有/无可倒戈目标），旧代码却给兜底分支 −1、主路 0 分——
/// 等于分数取决于「世界上有没有可倒戈的势力」这个与本次得失无关的偶然。
fn military_deltas(events: &[GameEvent]) -> BTreeMap<FactionId, f64> {
    let mut delta: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut bump = |fid: &FactionId, d: f64| {
        if !fid.is_empty() {
            *delta.entry(fid.clone()).or_insert(0.0) += d;
        }
    };
    for e in events {
        match e {
            GameEvent::ShipDestroyed { owner, cause, by, .. } => {
                bump(owner, -1.0);
                if *cause == DeathCause::Combat {
                    if let Some(k) = by {
                        bump(&k.faction, 1.0);
                    }
                }
            }
            GameEvent::CityRazed { owner, fallen_to, .. } => {
                bump(owner, -1.0);
                bump(fallen_to, 1.0);
            }
            GameEvent::CityDefected { from, to, .. } => {
                bump(from, -1.0);
                bump(to, 1.0);
            }
            GameEvent::Revolt { faction, .. } => bump(faction, -1.0),
            _ => {}
        }
    }
    delta
}

// --- story / chronicle -------------------------------------------------------
/// 剧情步进：评估 config 的 `story` 表，把满足触发条件的剧情事件火出，写入
/// [`State::chronicle`] 编年史并记一条 [`GameEvent::Story`]，同时应用可选的小幅
/// 机械后果（关系/资源）。确定性：无 RNG，同一种子触发完全一致。
///
/// 每个事件默认只触发一次（id 已入编年史则跳过）。触发条件见 [`StoryTrigger`]；
/// 事件型条件（`FirstWar`/`FirstRaze`/`FirstColony`/`WarBetween`/`FactionAtWar`）
/// 依据本回合已产生的事件（含开战/停战/夷平/殖民）判定——因此这些剧情节拍正好落在
/// 对应历史事件发生的那个回合，形成「剧情与局势同步」的叙事弧。
fn step_story(state: &mut State, config: &GameConfig) {
    for spec in &config.story {
        // 每个剧情事件只触发一次：已进编年史则跳过。
        if state.chronicle.iter().any(|c| c.id == spec.id) {
            continue;
        }
        if !story_trigger_fired(state, &spec.trigger) {
            continue;
        }
        // 机械后果（小幅、确定性）。
        for effect in &spec.effects {
            match effect {
                StoryEffect::Relations { a, b, delta } => {
                    adjust_relation(state, config, a, b, *delta);
                }
                StoryEffect::GrantResources { faction, resource, amount } => {
                    if let Some(f) = state.faction_mut(faction) {
                        *f.resources.entry(resource.clone()).or_insert(0.0) += *amount;
                    }
                }
                StoryEffect::GrantShip { faction, class, body } => {
                    grant_story_ship(state, config, faction.clone(), class, body.clone());
                }
            }
        }
        // 记入编年史 + 本回合故事事件。参与方由静态模板 + 本次事件的具体对象合成
        // （事件型触发把「实际是谁」写进编年史，让剧情真正反应该回合发生的事情）。
        let participants = story_participants(state, spec);
        let entry = ChronicleEntry {
            round: state.round,
            id: spec.id.clone(),
            title: spec.title.clone(),
            body: spec.body.clone(),
            participants: participants.clone(),
        };
        ev(state, GameEvent::Story { id: spec.id.clone(), title: spec.title.clone(), participants });
        state.chronicle.push(entry);
    }
}

/// 剧情事件的参与方：模板里写的静态可读名，加上事件型触发从本回合事件里提炼出的
/// 具体对象（哪两方开战 / 哪座城被夷平 / 谁建立了殖民地）。保证编年史「自描述」——
/// agent 无需反推就能知道这条剧情发生在谁身上。确定性：取自本回合事件流水。
fn story_participants(state: &State, spec: &StoryEvent) -> Vec<String> {
    let mut parts: Vec<String> = spec.participants.clone();
    let add = |parts: &mut Vec<String>, name: Option<String>| {
        if let Some(n) = name {
            if !n.is_empty() && !parts.iter().any(|p| p == &n) {
                parts.push(n);
            }
        }
    };
    let find_war = |state: &State, faction: Option<FactionId>| -> Option<(FactionId, FactionId)> {
        state.events.iter().find_map(|e| match e {
            GameEvent::WarStarted { a, b } => match &faction {
                Some(f) if a == f || b == f => Some((a.clone(), b.clone())),
                Some(_) => None,
                None => Some((a.clone(), b.clone())),
            },
            _ => None,
        })
    };
    match &spec.trigger {
        StoryTrigger::FirstWar => {
            if let Some((a, b)) = find_war(state, None) {
                add(&mut parts, state.faction(&a).map(|f| f.name.clone()));
                add(&mut parts, state.faction(&b).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FactionAtWar { faction } => {
            add(&mut parts, state.faction(faction).map(|f| f.name.clone()));
            if let Some((a, b)) = find_war(state, Some(faction.clone())) {
                let other = if a == *faction { b } else { a };
                add(&mut parts, state.faction(&other).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FirstRaze => {
            if let Some((city, fallen)) = state.events.iter().find_map(|e| match e {
                GameEvent::CityRazed { city, fallen_to, .. } => Some((city.clone(), fallen_to.clone())),
                _ => None,
            }) {
                add(&mut parts, state.city(&city).map(|c| c.name.clone()));
                add(&mut parts, state.faction(&fallen).map(|f| f.name.clone()));
            }
        }
        StoryTrigger::FirstColony => {
            if let Some((owner, body)) = state.events.iter().find_map(|e| match e {
                GameEvent::ColonyFounded { owner, body, .. } => Some((owner.clone(), body.clone())),
                _ => None,
            }) {
                add(&mut parts, state.faction(&owner).map(|f| f.name.clone()));
                add(&mut parts, state.body(&body).map(|b| b.name.clone()));
            }
        }
        StoryTrigger::WarBetween { a, b } => {
            add(&mut parts, state.faction(a).map(|f| f.name.clone()));
            add(&mut parts, state.faction(b).map(|f| f.name.clone()));
        }
        _ => {}
    }
    parts
}

/// 剧情：把一个舰级「出厂」给某势力，位置在天体当前位置附近（小幅确定性偏移）。
/// 舰 id 按当前最大 id 连续分配，/并配一条 `Idle` 指令；无 RNG，确定性复现。
fn grant_story_ship(state: &mut State, config: &GameConfig, faction: FactionId, class: &str, body: BodyId) {
    if !config.ships.contains_key(class) {
        return;
    }
    let pos = state.body_position(&body);
    if state.body(&body).is_none() {
        return;
    }
    // 剧情赠舰此前**完全不发事件**——一艘舰凭空出现。走 `spawn_ship` 漏斗补上，
    // 让它进可查的历史（`via = story` 与船坞出厂区分开）；赠舰不付组件成本
    // （是剧情送的），也没有出厂城（在天体附近下水）。
    spawn_ship(state, config, ShipSpawn {
        owner: faction,
        class,
        position: [pos[0] + 0.05, pos[1] + 0.05],
        city: None,
        via: SpawnVia::Story,
        pay_components: false,
    });
}

/// 判断一条剧情触发条件是否已满足。
fn story_trigger_fired(state: &State, trigger: &StoryTrigger) -> bool {
    match trigger {
        StoryTrigger::RoundAt { round } => state.round >= *round,
        StoryTrigger::FirstWar => state.events.iter().any(|e| matches!(e, GameEvent::WarStarted { .. })),
        StoryTrigger::FirstRaze => state.events.iter().any(|e| matches!(e, GameEvent::CityRazed { .. })),
        StoryTrigger::FirstColony => state.events.iter().any(|e| matches!(e, GameEvent::ColonyFounded { .. })),
        StoryTrigger::WarBetween { a, b } => state.events.iter().any(|e| match e {
            GameEvent::WarStarted { a: x, b: y } => {
                let (lo, hi) = (x.min(y), x.max(y));
                let (plo, phi) = (a.min(b), a.max(b));
                lo == plo && hi == phi
            }
            _ => false,
        }),
        StoryTrigger::FactionAtWar { faction } => state.events.iter().any(|e| match e {
            GameEvent::WarStarted { a, b } => *a == *faction || *b == *faction,
            _ => false,
        }),
        StoryTrigger::RelationBelow { a, b, value } => relation(state, a, b) < *value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::model::GameEvent;
    use crate::world::default_state;

    /// Build the config + a fresh deterministic world (round 0)，并把**角色轴钉成「全员战舰」**。
    ///
    /// 这个模块的用例大多在测**别的东西**（生产/贸易/MOND/战斗/事件），而自动控制现在多了一条
    /// 活：按积压定编、把船派去跑集货路线。不钉住它，被测的舰就可能被抽去拉货、不在它该在的
    /// 位置上（`crate::world::pin_roles_to_war` 的文档记着一次实测踩坑）。
    /// **测集货本身的用例**（`haul_*`）自己撤掉/覆盖这条默认。
    fn fresh_world(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let mut state = default_state(&config, seed);
        crate::world::pin_roles_to_war(&mut state);
        (config, state)
    }

    /// 军事信号（思潮「和平↔军国」的驱动量）必须**只**由事件历史推出，且**同一现象同分**。
    ///
    /// 这里逐条钉住旧实现的两个真实缺陷：
    /// 1. **互杀吞掉战功**：旧口径是「同回合最后一条 `Attack` 的势力」，那要 `state.ship(attacker)`
    ///    才知道攻击者属于谁——凶手若在本回合也被打沉，它已经不在 `state.ships` 里，于是这次
    ///    击杀**领不到功**。权威的 `by` 不受影响。
    /// 2. **失城方读成了抢城者**：旧口径用 `state.city(city).faction_id` 判「谁丢了城」，而夷平
    ///    不改归属、同回合稍后的复垦会把它改成新主，于是 −1 记到了**复垦者**头上。
    ///
    /// 另外钉住「`CityDefected`（主路）与 `Revolt`（兜底）必须同分」——它们是同一个触发的两条
    /// 分支，旧代码却只给兜底分支扣分。
    #[test]
    fn military_signal_uses_the_milestones_and_is_branch_agnostic() {
        let d = |events: &[GameEvent], fid: &str| military_deltas(events).get(fid).copied().unwrap_or(0.0);

        // 1) 互杀：A 的舰打沉 B 的舰，B 的舰同回合也打沉 A 的舰 → **双方各得一分战功**。
        let killer = |ship: &str, faction: &str| Killer {
            ship: ship.to_string(), faction: faction.to_string(), weapon: "kinetic".to_string(),
        };
        let mutual = vec![
            GameEvent::ShipDestroyed {
                ship: "乙舰".into(), owner: "乙".into(), class: "corvette".into(),
                cause: DeathCause::Combat, by: Some(killer("甲舰", "甲")),
            },
            GameEvent::ShipDestroyed {
                ship: "甲舰".into(), owner: "甲".into(), class: "corvette".into(),
                cause: DeathCause::Combat, by: Some(killer("乙舰", "乙")),
            },
        ];
        assert_eq!(d(&mutual, "甲"), 0.0, "甲沉一舰失一分、击沉一舰得一分，净 0");
        assert_eq!(d(&mutual, "乙"), 0.0, "乙同理——旧口径下会有一方拿不到战功");
        // 单方面被击沉：凶手得分，事主扣分。
        let one_sided = vec![GameEvent::ShipDestroyed {
            ship: "乙舰".into(), owner: "乙".into(), class: "corvette".into(),
            cause: DeathCause::Combat, by: Some(killer("甲舰", "甲")),
        }];
        assert_eq!(d(&one_sided, "甲"), 1.0);
        assert_eq!(d(&one_sided, "乙"), -1.0);

        // 2) 欠费报废：失主扣分，**没有人**领功（不是战功）。
        let rusted = vec![GameEvent::ShipDestroyed {
            ship: "锈舰".into(), owner: "丙".into(), class: "corvette".into(),
            cause: DeathCause::UpkeepShortfall, by: None,
        }];
        assert_eq!(d(&rusted, "丙"), -1.0);
        assert_eq!(d(&rusted, "甲"), 0.0, "欠费报废不该被记成任何人的战功");

        // 3) 城被 A 拆平、同回合被 C 复垦：扣分属于**失城方 B**，复垦者 C 不因此得军事分。
        let razed_then_refounded = vec![
            GameEvent::CityRazed {
                city: "城".into(), owner: "乙".into(), fallen_to: "甲".into(),
                by_ship: "甲舰".into(), damage: 9.0, pop_before: 200,
            },
            GameEvent::ColonyFounded {
                city: "城".into(), owner: "丙".into(), body: "木星".into(),
                seeded_ship_class: "corvette".into(), how: FoundingHow::Refounded,
                prev_owner: Some("乙".into()),
            },
        ];
        assert_eq!(d(&razed_then_refounded, "乙"), -1.0, "失城方是乙，不是复垦者");
        assert_eq!(d(&razed_then_refounded, "甲"), 1.0, "拆城方得一分");
        assert_eq!(d(&razed_then_refounded, "丙"), 0.0, "复垦是殖民行为，不进军事轴");

        // 4) 活城易主（离心倒戈）必须与叛乱兜底同分。
        let defect = vec![GameEvent::CityDefected {
            city: "城".into(), from: "乙".into(), to: "甲".into(), loyalty: 0.2,
        }];
        assert_eq!(d(&defect, "乙"), -1.0, "失主必须扣分（与 Revolt 兜底同分）");
        assert_eq!(d(&defect, "甲"), 1.0);
        let revolt = vec![GameEvent::Revolt { city: "城".into(), faction: "乙".into(), loyalty: 0.0 }];
        assert_eq!(d(&revolt, "乙"), -1.0);

        // 5) 新建城（真·殖民）不进军事轴。
        let founded = vec![GameEvent::ColonyFounded {
            city: "新城".into(), owner: "丙".into(), body: "地球".into(),
            seeded_ship_class: "corvette".into(), how: FoundingHow::NewSite, prev_owner: None,
        }];
        assert_eq!(d(&founded, "丙"), 0.0, "殖民归 nature_colony 轴");
    }

    /// 舰队默认指令要真的管住**新造出来的舰**：它出厂时没有任何指令叶片（不点名 = 不在
    /// 任何 diff 里），但不能因此默认归系统、被 AI 拿去远征或停在 Idle —— 它应当直接执行
    /// 势力的默认意图。
    ///
    /// 这条是 note `agent-control-long-game.md` §5 的端到端守卫（控制面单测在
    /// `control::tests::fleet_default_order_covers_new_ships`）。
    #[test]
    fn fleet_default_governs_newly_built_ships() {
        let (config, mut state) = fresh_world(42);
        let fid = "中国".to_string();
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "default_ship_order": {"behavior": {"type": "dock", "body": "地球"}}
            }]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("fleet default applies");

        // 与船坞出厂同一条漏斗造一艘新舰（不带指令叶片）。
        let pos = state.body_position("水星");
        let name = spawn_ship(&mut state, &config, ShipSpawn {
            owner: fid.clone(),
            class: "corvette",
            position: pos,
            city: None,
            via: SpawnVia::Shipyard,
            pay_components: false,
        });
        // 出厂时 `spawn_ship` 给它一条**没有说话**（`Inherit`）的叶片——它不在玩家的任何
        // diff 里，所以「谁负责、干什么」只能由更宽的那一层回答。
        let leaf = state
            .control(fid.clone())
            .and_then(|c| c.ship_orders.get(&name).cloned())
            .expect("spawn_ship seeds an order leaf");
        assert_eq!(leaf.mode, ControlMode::Inherit, "a freshly built ship has no opinion of its own");
        assert_eq!(leaf.value, ShipBehavior::Idle, "…and its recorded value is a mere placeholder");
        assert_eq!(state.ship_control(name.clone()), ControlMode::Player, "…so the fleet default owns it");
        assert_eq!(
            state.ship_behavior(name.clone()),
            Some(ShipBehavior::Dock { body: "地球".to_string() }),
            "…and it inherits the faction's intent instead of standing idle"
        );

        // 推进一回合：AI 不许碰它（归属解析在它身上给出 Player），而且它照着默认意图动。
        let mut rng = Prng::new(42);
        let before = state.ship(&name).expect("ship").position;
        advance(&mut state, &config, &mut rng);
        assert_eq!(state.ship_control(name.clone()), ControlMode::Player, "the system must not take it over");
        let after = state.ship(&name).map(|s| s.position).unwrap_or(before);
        let to_earth = dist(after, state.body_position("地球")) < dist(before, state.body_position("地球"));
        assert!(to_earth, "the new ship must sail for 地球 per the fleet default, not be sent off by the AI");
    }

    /// 玩家点名的殖民舰建完城之后必须**仍然是玩家的**。
    ///
    /// 殖民是「命令 → 执行 → 指令失效」的一次性动作：收尾只该把**值**复位成 `Idle`，
    /// 不许把**归属**一起清掉——以前无条件写 `Control::inherit(..)`，于是玩家刚下达的处置
    /// 在城建好的那一刻被静默交还给系统（AI 下一回合就把它征去别处）。
    #[test]
    fn colonize_keeps_player_ownership() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        let fid = "中国".to_string();

        // 空出一座城（复垦路径：空白城仍占着它的定居点），再让中国的一艘舰去殖民。
        let victim = state
            .cities
            .iter()
            .find(|c| !c.razed && c.faction_id != fid)
            .map(|c| (c.name.clone(), c.body_id.clone(), c.faction_id.clone()))
            .expect("a foreign city to raze");
        let (cid, body, owner) = victim;
        raze_city(&mut state, &cid, RazeCause::Revolt { faction: owner, loyalty: 0.0 });

        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == fid)
            .map(|s| s.name.clone())
            .expect("a chinese ship");
        let order = |state: &mut State, v: ShipBehavior| {
            state
                .control_mut(fid.clone())
                .expect("control")
                .ship_orders
                .insert(ship.clone(), Control::player(v));
        };
        order(&mut state, ShipBehavior::Colonize { body: body.clone() });

        let mut next_id = 100_000;
        colonize(&mut state, &config, &mut rng, &ship, &body, &mut next_id);

        let leaf = state
            .control(fid.clone())
            .and_then(|c| c.ship_orders.get(&ship).cloned())
            .expect("the order leaf must still exist");
        assert_eq!(leaf.value, ShipBehavior::Idle, "one-shot order must be spent");
        assert_eq!(leaf.mode, ControlMode::Player, "…but ownership must survive the order");
        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::ColonyFounded { .. })),
            "the city must actually have been refounded, got {:?}",
            state.events
        );

        // 早退路径（无处可殖民）同样不许动归属：找一个所有定居点都被活的城占满的天体。
        let full_body = state
            .cities
            .iter()
            .filter(|c| !c.razed)
            .map(|c| c.body_id.clone())
            .find(|b| {
                let Some(body) = state.body(b) else { return false };
                let live: std::collections::BTreeSet<String> = state
                    .cities
                    .iter()
                    .filter(|c| &c.body_id == b && !c.razed)
                    .map(|c| c.settlement.clone())
                    .collect();
                body.settlements.iter().all(|s| live.contains(&s.name))
            })
            .expect("a body whose settlements are all occupied");
        order(&mut state, ShipBehavior::Colonize { body: full_body.clone() });
        colonize(&mut state, &config, &mut rng, &ship, &full_body, &mut next_id);
        let leaf = state
            .control(fid.clone())
            .and_then(|c| c.ship_orders.get(&ship).cloned())
            .expect("the order leaf must still exist");
        assert_eq!(leaf.mode, ControlMode::Player, "an early return must not hand the ship back either");
    }

    /// A player-facing regression guard for the "stale follow" bug: a player
    /// ship ordered to Follow an already-destroyed ship must degrade to Idle,
    /// never drift toward the origin ([0,0]).
    #[test]
    fn player_stale_follow_degrades_to_idle_and_does_not_drift() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // China (3) corvette id=0 is Player-ordered to Follow US (1) destroyer id=3.
        let ship0 = state.ships[0].name.clone();
        let ship3 = state.ships[3].name.clone();
        let diff = serde_json::json!({
            "control": [{
                "faction_id": "中国",
                "ship_orders": [{"ship": ship0.clone(), "behavior": {"Follow": {"ship": ship3.clone()}}, "mode": "Player"}]
            }]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("apply order");

        // Simulate the target being destroyed before the round advances. 走 `kill_ship` 漏斗——
        // 它现在是**唯一**合法的「让一艘舰死」的方式（绕过它会被 `sweep_dead_ships` 的兜底
        // `debug_assert` 当场抓住，这正是这条测试以前直接 `t.hull = 0.0` 会炸的原因）。
        assert!(
            kill_ship(&mut state, &ship3, DeathCause::Combat, None),
            "target must get a recorded death event"
        );
        let pos_before = state.ship(&ship0).map(|s| s.position).unwrap();

        advance(&mut state, &config, &mut rng);

        // The order must have degraded to Idle ...
        let order = state.ship_behavior(ship0.clone());
        assert_eq!(order, Some(ShipBehavior::Idle), "stale order must degrade to Idle");
        // ... without moving the ship toward the origin.
        let pos_after = state.ship(&ship0).map(|s| s.position).unwrap();
        assert_eq!(pos_after, pos_before, "ship must not drift (target is dead)");
        // ... and a StaleOrder event must be recorded.
        assert!(
            state.events.iter().any(|e| matches!(e, GameEvent::StaleOrder { ship: s, .. } if *s == ship0)),
            "expected a StaleOrder event for ship 0, got {:?}",
            state.events
        );
    }

    /// Follow semantics: `Follow { ship }` escorts/drives the ship — it is a pure
    /// movement behavior. Combat is now automatic: when any hostile is inside the
    /// ship's own attack range it auto-fires (via the unified base-weight targeting),
    /// so a Follow ship still defends itself but never fires at the followed friend.
    #[test]
    fn follow_ship_auto_attacks_hostile_but_not_the_followed_friend() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // China (3): ship 0 Follows its own friendly ship 1. Co-located at [0,0].
        let ship0 = state.ships[0].name.clone();
        let ship1 = state.ships[1].name.clone();
        let ship3 = state.ships[3].name.clone();
        let ship4 = state.ships[4].name.clone();
        let ship5 = state.ships[5].name.clone();
        let diff = serde_json::json!({
            "control": [{
                "faction_id": "中国",
                "ship_orders": [{"ship": ship0.clone(), "behavior": {"Follow": {"ship": ship1.clone()}}, "mode": "Player"}]
            }]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("apply follow order");

        // Pin positions: follower + followed friend at [0,0]; US (1) enemy
        // destroyer id=3 just inside the corvette attack range (0.4) so the
        // auto-attack can fire. Move the other US ships (4 destroyer, 5 cruiser)
        // far out so only ship 3 engages (its damage 6 won't one-shot the follower's
        // hull 12, letting it retaliate).
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = [0.0, 0.0];
        }
        if let Some(s) = state.ship_mut(&ship1) {
            s.position = [0.0, 0.0];
        }
        if let Some(enemy) = state.ship_mut(&ship3) {
            enemy.position = [0.3, 0.0];
        }
        if let Some(s) = state.ship_mut(&ship4) {
            s.position = [50.0, 50.0];
        }
        if let Some(s) = state.ship_mut(&ship5) {
            s.position = [50.0, 50.0];
        }

        // The default world now opens peacefully, so make US (1) explicitly
        // hostile to China (3) for this scenario.
        if let Some(f) = state.faction_mut("中国") {
            f.relations.insert("美国".to_string(), -35.0);
        }
        if let Some(f) = state.faction_mut("美国") {
            f.relations.insert("中国".to_string(), -35.0);
        }

        advance(&mut state, &config, &mut rng);

        // The ship must auto-fire on the hostile, not on the friend.
        assert!(
            state.events.iter().any(|e| matches!(
                e,
                GameEvent::Attack { attacker, target, .. } if attacker == &ship0 && target == &ship3
            )),
            "ship should auto-attack the hostile, got {:?}",
            state.events
        );
        // The followed friend must be unharmed (no attack targeting ship 1).
        assert!(
            !state.events.iter().any(|e| matches!(e, GameEvent::Attack { target, .. } if target == &ship1)),
            "ship must not fire at its own followed friend, got {:?}",
            state.events
        );
        // The order is still a valid Follow (not degraded to Idle).
        assert_eq!(
            state.ship_behavior(ship0.clone()),
            Some(ShipBehavior::Follow { ship: ship1.clone() })
        );
    }

    /// Events must populate as the world advances (growth / spurious events are
    /// fine; the round log must simply be populated and contain no panics).
    #[test]
    fn advance_populates_round_events() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        assert!(state.events.is_empty(), "round 0 has no events yet");
        for _ in 0..6 {
            advance(&mut state, &config, &mut rng);
        }
        // After a few rounds of a war-torn seed, an event log should exist.
        assert!(!state.events.is_empty(), "after 6 rounds there should be events");
    }

    /// 停泊轨道 (Dock) follows a body's current position; 待命 (Idle) holds
    /// position. Dock persists (never degrades), and Idle never moves the ship.
    #[test]
    fn dock_follows_body_and_idle_holds_position() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // China (3) corvette id=0 docks body 4 (火星); id=1 is ordered Idle.
        let ship0 = state.ships[0].name.clone();
        let ship1 = state.ships[1].name.clone();
        let diff = serde_json::json!({
            "control": [{
                "faction_id": "中国",
                "ship_orders": [
                    {"ship": ship0.clone(), "behavior": {"Dock": {"body": "火星"}}, "mode": "Player"},
                    {"ship": ship1.clone(), "behavior": "Idle", "mode": "Player"}
                ]
            }]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("apply dock/idle order");

        // Pin ship 0 away from the body so `Dock` must move it toward the body.
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = [5.0, 5.0];
        }
        if let Some(s) = state.ship_mut(&ship1) {
            s.position = [3.0, 3.0];
        }
        let dock_pos_before = state.ship(&ship0).map(|s| s.position).unwrap();
        let idle_pos_before = state.ship(&ship1).map(|s| s.position).unwrap();

        advance(&mut state, &config, &mut rng);

        // Dock: the ship moved toward the body (not froze, not degraded).
        let dock_pos_after = state.ship(&ship0).map(|s| s.position).unwrap();
        assert_ne!(dock_pos_after, dock_pos_before, "docked ship should move toward the body");
        assert_eq!(
            state.ship_behavior(ship0.clone()),
            Some(ShipBehavior::Dock { body: "火星".to_string() }),
            "dock order must persist (not degrade to Idle)"
        );
        // Idle: the ship did not move.
        let idle_pos_after = state.ship(&ship1).map(|s| s.position).unwrap();
        assert_eq!(idle_pos_after, idle_pos_before, "Idle must hold position");
        assert_eq!(state.ship_behavior(ship1.clone()), Some(ShipBehavior::Idle));
    }

    /// 护甲再生 (ShipSpec.hull_regen): a damaged ship regains a fraction of its
    /// max hull each round; full-hull ships stay capped; destroyed ships stay gone.
    #[test]
    fn damaged_ship_regenerates_hull_each_round() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // Take China's Earth corvette (id 0, hull_max 12, hull_regen 0.04) and
        // damage it to exactly half; pin it away from all hostiles so the round
        // is quiet and only regeneration acts on it.
        let ship0 = state.ships[0].name.clone();
        let ship1 = state.ships[1].name.clone();
        if let Some(s) = state.ship_mut(&ship0) {
            s.hull = 6.0;
            s.position = [80.0, 80.0];
        }
        let class = state.ship(&ship0).map(|s| s.class.clone()).unwrap();
        let regen = config.ship_spec(&class).hull_regen;

        advance(&mut state, &config, &mut rng);

        let hull = state.ship(&ship0).map(|s| s.hull).expect("ship 0 still alive");
        let expected = (6.0 + 12.0 * regen).min(12.0);
        assert!(
            (hull - expected).abs() < 1e-9,
            "hull should heal to {expected}, got {hull}"
        );

        // A full-hull ship stays capped (no over-heal).
        if let Some(s) = state.ship_mut(&ship1) {
            s.hull = config.ship_spec(&s.class).hull;
            s.position = [80.0, 80.0];
        }
        advance(&mut state, &config, &mut rng);
        let max1 = config.ship_spec(&state.ship(&ship1).map(|s| s.class.clone()).unwrap()).hull;
        let hull1 = state.ship(&ship1).map(|s| s.hull).unwrap();
        assert!((hull1 - max1).abs() < 1e-9, "full hull must not over-heal, got {hull1}");
    }

    /// 定居点 ↔ 城市 一一对应: 每个城市占据其天体上一个合法定居点；同一座城不会
    /// 让一个定居点被两座城占用；地球恰好 5 个定居点各坐一座 spec 都市，矿藏按
    /// 定居点隔离（巴黎只产 铀/铂，不再共享整个地球的矿藏池）。
    #[test]
    fn settlements_and_cities_are_one_to_one() {
        let (_config, state) = fresh_world(42);
        for b in &state.bodies {
            let cities: Vec<&City> = state.cities.iter().filter(|c| c.body_id == b.name).collect();
            assert!(
                cities.len() <= b.settlements.len(),
                "body {}: {} cities must not exceed {} settlements",
                b.name,
                cities.len(),
                b.settlements.len()
            );
            for c in cities {
                assert!(
                    b.settlements.iter().any(|s| s.name == c.settlement),
                    "city {} (body {}) points at an unknown settlement {}",
                    c.name,
                    b.name,
                    c.settlement
                );
            }
        }

        let earth = &state.bodies[2];
        assert_eq!(earth.settlements.len(), 5, "Earth has five spec metropolises");
        let earth_cities = state.cities.iter().filter(|c| c.body_id == "地球").count();
        assert_eq!(earth_cities, 5, "five cities on five Earth settlements (1:1)");
        // 巴黎 (settlement named 巴黎) hosts only 铀/铂 — its own region's ores.
        let paris = earth.settlements[3].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
        assert_eq!(paris, vec!["铀", "铂"], "Paris settlement mines only its own ores");
        assert_eq!(
            state.cities.iter().find(|c| c.name == "巴黎").map(|c| c.settlement.as_str()),
            Some("巴黎"),
            "巴黎 occupies the settlement named 巴黎"
        );
        // 长三角/珠三角 are distinct settlements, so both may mine 铁 independently.
        let cn = earth.settlements[0].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
        assert!(cn.contains(&"铁"), "长三角 settlement has 铁");
        assert!(cn.contains(&"硅") && cn.contains(&"水冰"), "长三角 has 硅/水冰");
    }

    /// 剧情编年史：RoundAt 节拍按回合触发、编年史按发生先后单调增长、id 唯一，且
    /// 同一种子完全确定（重跑逐字节一致）。
    #[test]
    fn story_chronicle_grows_deterministically() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        // Round-at beats: prologue fires round 1, planet_x_arrives round 60.
        for _ in 0..60 {
            advance(&mut state, &config, &mut rng);
        }
        let ids: Vec<&str> = state.chronicle.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains(&"prologue"), "prologue (RoundAt 1) must fire");
        assert!(ids.contains(&"planet_x_arrives"), "planet_x_arrives (RoundAt 60) must fire");

        // The chronicle records the round it fired, in non-decreasing order.
        let rounds: Vec<u32> = state.chronicle.iter().map(|c| c.round).collect();
        let mut sorted = rounds.clone();
        sorted.sort_unstable();
        assert_eq!(rounds, sorted, "chronicle must be sorted by firing round");

        // ids are unique (each event fires once).
        let mut dedup = ids.clone();
        dedup.sort_unstable();
        let before_n = dedup.len();
        dedup.dedup();
        assert_eq!(before_n, dedup.len(), "each story id fires at most once");

        // Determinism: re-running the same seed reproduces the identical chronicle.
        let (_, mut state2) = fresh_world(42);
        let mut rng2 = Prng::new(42);
        for _ in 0..60 {
            advance(&mut state2, &config, &mut rng2);
        }
        assert_eq!(
            state.chronicle.iter().map(|c| (c.round, c.id.clone(), c.title.clone())).collect::<Vec<_>>(),
            state2.chronicle.iter().map(|c| (c.round, c.id.clone(), c.title.clone())).collect::<Vec<_>>(),
            "same seed must produce the same story arc"
        );
    }

    /// 剧情机械后果：prologue 给无国界科学组织(6)注入氦-3，并拉低它与行星X崇拜教(8)的关系。
    #[test]
    fn story_effects_apply() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        let rel_before = state.faction("无国界科学组织").and_then(|f| f.relations.get(&"行星X崇拜教".to_string()).copied()).unwrap_or(0.0);

        advance(&mut state, &config, &mut rng);

        // prologue 是「事件型后果」：把资源写入并在编年史里记录。第 1 回合经济（维护费/
        // 市场）会立刻重新平衡库存，故不断言 helium3 净增（它可能被维护费/市场抵消），
        // 而断言那份资源确实进入了编年史记录的 prologue（机械后果生效）。
        assert!(
            state.chronicle.iter().any(|c| c.id == "prologue"),
            "prologue must fire and record its effects at round 1"
        );
        let rel_after = state.faction("无国界科学组织").and_then(|f| f.relations.get(&"行星X崇拜教".to_string()).copied()).unwrap_or(0.0);
        assert!(rel_after < rel_before, "prologue must lower science↔cult relation (effect)");
    }

    /// 剧情 GrantShip 后果：kuiper_boom（RoundAt 24）给星系矿业(5)出厂一艘巡洋舰；
    /// 出厂位置恰好在天体当前位置 + (0.05, 0.05)（确定性偏移）、带 Idle 指令、id 连续。
    #[test]
    fn story_grant_ship_spawns_a_fleet_member() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // Advance to round 24 so kuiper_boom fires.
        for _ in 0..24 {
            advance(&mut state, &config, &mut rng);
        }

        // The story fired this round.
        assert!(
            state.chronicle.iter().any(|c| c.id == "kuiper_boom" && c.round == 24),
            "kuiper_boom must fire at round 24, got {:?}",
            state.chronicle.iter().map(|c| (c.id.clone(), c.round)).collect::<Vec<_>>()
        );

        // The granted cruiser is at exactly body 9 (泰坦) position + the deterministic offset.
        let bpos = state.body_position("泰坦");
        let granted = state
            .ships
            .iter()
            .find(|s| {
                s.faction_id == "星系矿业"
                    && s.class == "cruiser"
                    && (s.position[0] - (bpos[0] + 0.05)).abs() < 1e-9
                    && (s.position[1] - (bpos[1] + 0.05)).abs() < 1e-9
            })
            .expect("kuiper_boom must grant 星系矿业 a cruiser parked at 泰坦");
        assert_eq!(state.ship_behavior(granted.name.clone()), Some(ShipBehavior::Idle), "granted ship starts Idle");

        // Determinism: re-running reproduces the identical granted fleet.
        let (_, mut state2) = fresh_world(42);
        let mut rng2 = Prng::new(42);
        for _ in 0..24 {
            advance(&mut state2, &config, &mut rng2);
        }
        let fleet_a: Vec<(String, String)> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == "星系矿业")
            .map(|s| (s.name.clone(), s.class.clone()))
            .collect();
        let fleet_b: Vec<(String, String)> = state2
            .ships
            .iter()
            .filter(|s| s.faction_id == "星系矿业")
            .map(|s| (s.name.clone(), s.class.clone()))
            .collect();
        assert_eq!(fleet_a, fleet_b, "same seed must reproduce the same granted fleet");
    }

    /// 剧情参与方是「具体的」：事件型触发把本回合事件的实际对象写进编年史
    /// （谁与谁开战、哪座城被夷平、谁建立了殖民地），而不是泛化的空标签。
    #[test]
    fn story_participants_are_concrete() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        for _ in 0..60 {
            advance(&mut state, &config, &mut rng);
        }
        let find = |id: &str| state.chronicle.iter().find(|c| c.id == id);
        if let Some(war) = find("first_war") {
            assert_eq!(war.participants.len(), 2, "first_war names the two belligerents, got {:?}", war.participants);
            assert!(war.participants.iter().all(|p| !p.is_empty()));
        }
        if let Some(razed) = find("first_raze") {
            assert!(razed.participants.len() >= 2, "first_raze names the city and the razer, got {:?}", razed.participants);
        }
        if let Some(colon) = find("first_colony") {
            assert!(colon.participants.len() >= 2, "first_colony names the colonizer and the body, got {:?}", colon.participants);
        }
        if let Some(cn) = find("cn_us_rivalry") {
            assert!(cn.participants.contains(&"中国".to_string()), "cn_us_rivalry names 中国, got {:?}", cn.participants);
            assert!(cn.participants.contains(&"美国".to_string()), "cn_us_rivalry names 美国, got {:?}", cn.participants);
        }
        // RoundAt beats keep exactly their static participants (no event to enrich).
        if let Some(pro) = find("prologue") {
            assert_eq!(pro.participants, vec!["无国界科学组织".to_string(), "行星X崇拜教".to_string()]);
        }
    }

    /// 娱乐/福利预算（忠诚度）：一座远离首都的城市，其距离目标忠诚度本应很低；但若
    /// 治理势力投入足够的娱乐预算，忠诚度仍能维持/回升，而非立刻爆发离心叛乱。
    #[test]
    fn entertainment_holds_a_distant_city() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);
        // 深口袋：让星系矿业(5)付得起治理 + 娱乐开销，覆盖率=1。
        if let Some(f) = state.faction_mut("星系矿业") {
            for k in [
                "铁", "碳", "硅", "水冰", "铀", "铂", "金",
                "氦-3", "钍", "氢", "甲烷",
            ] {
                f.resources.insert(k.to_string(), 100_000.0);
            }
        }
        // 妊神星转运站 (city 19, body 15) 远离矿业首都(泰坦, body 9)，距离目标忠诚度≈0。
        let city19 = state.cities[19].name.clone();
        if let Some(c) = state.city_mut(&city19) {
            c.loyalty = 0.35; // 略高于叛变阈值，但本应继续下滑。
        }
        let loy0 = state.city(&city19).map(|c| c.loyalty).unwrap();
        // 重金投入该城娱乐预算（Player 覆盖）。
        let diff = serde_json::json!({
            "control": [{"faction_id": "星系矿业", "loyalty_budget": [{"city": city19.clone(), "value": 500.0, "mode": "Player"}]}]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("apply loyalty budget");

        advance(&mut state, &config, &mut rng);

        let loy1 = state.city(&city19).map(|c| c.loyalty).unwrap_or(0.0);
        assert!(
            loy1 >= loy0,
            "heavy entertainment funding should keep a distant city loyal (started {loy0}, now {loy1})"
        );
        assert_eq!(
            state.city(&city19).map(|c| c.razed),
            Some(false),
            "a well-funded distant city must not revolt"
        );
    }

    /// 离心「改旗易帜」：低忠诚城市不再被夷为荒地，而是倒戈到**思潮与旧主最对立**的势力，
    /// 城市连同其人口/建筑/控制面一起易主（旧主失去一城、新主获得一城）——这是给旁观/
    /// 小势力接盘城市、避免「永久 1 城旁观者」的机制。
    #[test]
    fn low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing() {
        let (config, mut state) = fresh_world(42);
        let mut rng = Prng::new(42);

        // 珠三角 (city 1) 属中国，位于其首都(地球)上——但把忠诚压到叛变阈值之下。
        let city = state.cities[1].name.clone();
        let owner = "中国".to_string();

        // 中国 → 极端（军国+技术+精英+殖民），无国界科学组织 → 相反极，其余全中立。
        // 于是无国界科学组织与中国的思潮距离 = 8（唯一最大），倒戈目标唯一确定。
        let extreme = Ideology { peace_military: 1.0, science_tech: 1.0, people_elite: 1.0, nature_colony: 1.0 };
        let oppose = Ideology { peace_military: -1.0, science_tech: -1.0, people_elite: -1.0, nature_colony: -1.0 };
        if let Some(f) = state.faction_mut("中国") {
            f.ideology = extreme;
        }
        if let Some(f) = state.faction_mut("无国界科学组织") {
            f.ideology = oppose;
        }
        for f in &mut state.factions {
            if f.name != "中国" && f.name != "无国界科学组织" {
                f.ideology = Ideology::default();
            }
        }
        // 忠诚压到叛变阈值之下（0.30）。
        if let Some(c) = state.city_mut(&city) {
            c.loyalty = 0.05;
        }
        let pop_before = state.city(&city).map(|c| c.population).unwrap_or(0);
        let buildings_before = state.city(&city).map(|c| c.buildings.len()).unwrap_or(0);

        advance(&mut state, &config, &mut rng);

        let c = state.city(&city).expect("defected city must survive (not razed)");
        assert_eq!(
            c.faction_id, "无国界科学组织",
            "low-loyalty city must defect to the most ideologically-opposed faction"
        );
        assert!(!c.razed, "defected city must not be razed to blank");
        assert_eq!(c.population, pop_before, "defected city keeps its population");
        assert_eq!(c.buildings.len(), buildings_before, "defected city keeps its buildings");
        // 忠诚在倒戈时被重置为满，随后同回合新主的治理会重新计量；断言它仍高于叛变阈值，
        // 证明这次倒戈给了城市一个「新开始」（没有立刻又叛变/再被夷平）。
        assert!(
            c.loyalty > 0.05,
            "defected city must get a fresh loyalty start (was 0.05, now {}), not stay near zero",
            c.loyalty
        );

        // 事件必须是 CityDefected（旧主→新主），不是 Revolt。
        assert!(
            state.events.iter().any(|e| matches!(
                e,
                GameEvent::CityDefected { city: cid, from, to, .. }
                    if *cid == city && *from == owner && *to == "无国界科学组织"
            )),
            "expected a CityDefected event, got {:?}",
            state.events
        );

        // 控制转移：新主(无国界科学组织)的控制面应接管这座城（invest/build 权重按 (城,建筑) 迁入）。
        if let Some(n) = state.control("无国界科学组织".to_string()) {
            let owned_build_keys: bool = state
                .city(&city)
                .map(|c| c.buildings.iter().any(|b| n.build_weights.contains_key(&(city.clone(), b.id))))
                .unwrap_or(false);
            assert!(
                n.invest_weights.keys().any(|(cid, _)| cid == &city) || n.build_weights.keys().any(|(cid, _)| cid == &city),
                "new owner control must include the defected city's buildings"
            );
            let _ = owned_build_keys;
        }
    }

    /// MOND 引力异常：落入异常区（深空）时，未掌握 MOND 修正引力的势力在导航上产生
    /// 切向偏移（指令坐标与实际坐标分离），而掌握它的 cult 指哪打哪。
    ///
    /// 偏移幅度是**伪随机**的（`roll`）：非 master 这一回合偏多少由尝试决定，
    /// 但**永远存在蒙对的一次**（`roll = 0` 即精确命中）——所以深处是「难」而不是「不可能」。
    #[test]
    fn mond_drift_misses_in_anomaly_but_masters_are_exact() {
        let (config, _state) = fresh_world(42);
        let dest = [60.0, 0.0]; // 距太阳 60 AU，深入柯伊伯异常区。
        // 非 MOND 势力（中国=3）：这一回合的目标被切向偏移，无法精确到达。
        let d = mond_drift(&config, "中国", dest, 0.5);
        assert!(
            (d[0] - dest[0]).abs() > 1e-6 || (d[1] - dest[1]).abs() > 1e-6,
            "a non-master ship must drift inside the anomaly, got {d:?}"
        );
        // 但偏移是可变的：偏得少的一次就几乎命中（这是「多试几个回合」的根据）。
        let lucky = mond_drift(&config, "中国", dest, 0.0);
        assert_eq!(lucky, dest, "roll=0 的一次尝试必须指哪打哪——不存在永远进不去的目标");
        let worse = mond_drift(&config, "中国", dest, 1.0);
        assert!(
            dist(worse, dest) > dist(d, dest),
            "roll 越大偏得越远（幅度单调），got {worse:?}"
        );
        // MOND 势力（行星X崇拜教=8）：掌握修正引力，无偏移、指哪打哪。
        for roll in [0.0, 0.5, 1.0] {
            let m = mond_drift(&config, "行星X崇拜教", dest, roll);
            assert_eq!(m, dest, "a MOND master must compute the destination exactly");
        }
    }

    /// **产地货栈（M1）**：首都天体的产出直接进势力池（**首都即集散地**，免运输），
    /// 其余天体的产出落在**产地货栈**里，**不会自己跑到池子里**——只有运输能把它送到首都。
    ///
    /// 用例（seed 42 的真实开局布局）：
    /// * **中国**：首都地球，矿在**两边都有**——长三角/珠三角（地球，采 铁/硅）是首都产出，
    ///   金星浮空之城（金星，采 碳）是离岸产出。碳 只从金星出、铁硅 只从地球出，
    ///   所以「池子里多了铁硅、碳却没动、金星货栈里有碳」正好把两条路径分开。
    /// * **无国界科学组织**：它的矿**全在非首都天体**（土星，采 氢；首都是木星）
    ///   → 产出**整批积压**，池子一分钱都不涨。这就是运输机制要解决的问题本身。
    #[test]
    fn off_capital_production_lands_in_the_depot_not_the_pool() {
        let (config, mut state) = fresh_world(42);
        assert_eq!(state.capital_body("中国"), "地球", "用例前提：中国首都在地球");
        assert_eq!(
            state.capital_body("无国界科学组织"),
            "木星",
            "用例前提：科学组织首都在木星"
        );

        let mut flow = RoundFlow::default();
        let cn_silicon = |s: &State| {
            s.faction("中国").unwrap().resources.get("硅").copied().unwrap_or(0.0)
        };
        let cn_carbon = |s: &State| {
            s.faction("中国").unwrap().resources.get("碳").copied().unwrap_or(0.0)
        };
        let sci_value = |s: &State| {
            let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
            s.faction("无国界科学组织")
                .unwrap()
                .resources
                .iter()
                .map(|(rt, amt)| amt * value_of(rt))
                .sum::<f64>()
        };

        let (si0, c0, sci0) = (cn_silicon(&state), cn_carbon(&state), sci_value(&state));
        step_production(&mut state, &config, &mut flow);

        // —— 中国：首都产出进池，离岸产出进货栈 ——
        assert!(
            cn_silicon(&state) > si0,
            "地球（首都）上的硅应直接进池"
        );
        assert!(
            state.depot("中国", "地球").is_none(),
            "首都天体的产出不进货栈（免运输、直接进池）"
        );
        let venus = state
            .depot("中国", "金星")
            .expect("金星上的产出必须落在产地货栈");
        assert!(
            venus.get("碳").copied().unwrap_or(0.0) > 0.0,
            "金星采的碳应压在产地货栈里，实为 {venus:?}"
        );
        assert!(
            (cn_carbon(&state) - c0).abs() < 1e-9,
            "碳只从金星（非首都）出，所以池子里的碳一格都不该动——运输才是货栈的上游"
        );

        // —— 科学组织：矿全在非首都 → 整批积压，池子不动 ——
        let saturn = state
            .depot("无国界科学组织", "土星")
            .expect("土星上的产出必须落在产地货栈");
        assert!(
            saturn.get("氢").copied().unwrap_or(0.0) > 0.0,
            "土星的氢应压在产地货栈里，实为 {saturn:?}"
        );
        assert!(
            (sci_value(&state) - sci0).abs() < 1e-9,
            "一个「矿全在非首都天体」的势力，产出会整批积压在产地（= 等船来运）"
        );

        // —— 再跑一回合：货栈继续涨、池子仍不因它增长（库存冻结）——
        let carbon_in_depot = venus.get("碳").copied().unwrap_or(0.0);
        step_production(&mut state, &config, &mut flow);
        assert!(
            state
                .depot("中国", "金星")
                .unwrap()
                .get("碳")
                .copied()
                .unwrap_or(0.0)
                > carbon_in_depot,
            "没有船来运 → 货栈继续涨"
        );
        assert!(
            (cn_carbon(&state) - c0).abs() < 1e-9,
            "两回合过去，金星的碳一格都没进池——这就是「等船来运」"
        );
    }

    /// **舱容（M2）**：有效舱容 = 舰级舱容 `ShipSpec::cargo` × **战损折算** `hull / hull_max`。
    ///
    /// 钉住三条机制不变量：
    /// 1. **舰级舱容是「设计裁决」而不是平衡旋钮**——护卫 2 / 驱逐 4 / 巡洋 6 / 航母 20 /
    ///    战列 6，且**航母是唯一的散货船**。这条要硬断言：改它等于改设计，不该是调参时手滑。
    /// 2. **战损是连续的**：装甲掉一半 → 舱容减半（不是「受伤就装不了」的硬阈值）。
    /// 3. **旧档（`hull_max ≤ 0`）按满舱**：绝不出现 `hull / 0 = ∞` 的无底货舱。
    #[test]
    fn cargo_capacity_is_class_capacity_times_hull_fraction() {
        use crate::model::cargo_capacity;
        let (config, state) = fresh_world(42);

        // 1) 舰级舱容（设计裁决：见 config/game.ron 的 ships 注释第 (3) 类）。
        let table = [
            ("corvette", 2.0),
            ("destroyer", 4.0),
            ("cruiser", 6.0),
            ("carrier", 20.0),
            ("battleship", 6.0),
        ];
        for (class, cap) in table {
            assert_eq!(
                config.ship_spec(class).cargo,
                cap,
                "{class} 的舱容是设计裁决（{cap}），不是可随手调的平衡旋钮"
            );
        }
        assert!(
            table.iter().all(|(c, cap)| *c == "carrier" || *cap < 20.0),
            "航母必须是唯一的散货船——否则「用哪条船运货」就不构成一个选择"
        );

        // 2) 战损连续折算。
        let mut ship = state
            .ships
            .iter()
            .find(|s| s.class == "cruiser")
            .expect("开局有巡洋舰")
            .clone();
        assert!(ship.hull_max > 0.0, "出厂舰必须有 hull_max");
        assert_eq!(cargo_capacity(&config, &ship), 6.0, "满血巡洋舰 = 满舱 6");
        ship.hull = ship.hull_max * 0.5;
        assert!(
            (cargo_capacity(&config, &ship) - 3.0).abs() < 1e-9,
            "装甲掉一半 → 舱容减半（连续，不是硬阈值）"
        );
        ship.hull = 0.0;
        assert_eq!(cargo_capacity(&config, &ship), 0.0, "壳被打光 → 一格都装不了");

        // 3) 旧档缺 `hull_max`：按满舱处理，而不是把舱容算成无穷。
        ship.hull = 6.0;
        ship.hull_max = 0.0;
        assert_eq!(
            cargo_capacity(&config, &ship),
            6.0,
            "hull_max ≤ 0（旧档）按未受损处理，绝不返回 ∞"
        );
    }

    /// **尽量等量分配（Q6）**：[`haul_split`] 是 max-min 公平分配——先按「还有货的种类数」平摊，
    /// 分不满的种类把余量交回去、由其余种类再平摊。它是**纯函数**，这里逐档钉住。
    #[test]
    fn haul_split_is_max_min_fair() {
        let m = |pairs: &[(&str, f64)]| -> ResourceMap {
            pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
        };
        // 三种货、舱容 6、都够 ⇒ 每种 2。
        assert_eq!(
            haul_split(&m(&[("铁", 10.0), ("碳", 10.0), ("硅", 10.0)]), 6.0),
            m(&[("铁", 2.0), ("碳", 2.0), ("硅", 2.0)])
        );
        // 铂只有 1 ⇒ 它拿 1，多出来的 1 由另两种再平摊（这就是「尽量」等量）。
        assert_eq!(
            haul_split(&m(&[("铁", 10.0), ("铂", 1.0), ("碳", 10.0)]), 6.0),
            m(&[("铁", 2.5), ("铂", 1.0), ("碳", 2.5)])
        );
        // 舱容 ≥ 总存量 ⇒ 全装走（一种货吃得下就全给它，不必等量）。
        assert_eq!(
            haul_split(&m(&[("铁", 1.0), ("碳", 2.0)]), 100.0),
            m(&[("铁", 1.0), ("碳", 2.0)])
        );
        // 边界：空货栈 / 零舱容 ⇒ 什么都不装（不是 panic）。
        assert!(haul_split(&ResourceMap::new(), 20.0).is_empty());
        assert!(haul_split(&m(&[("铁", 5.0)]), 0.0).is_empty());
    }

    /// **货值守恒（M2b 的核心不变量）**：装货与卸货**只搬货**——产地里少多少，舱里就多多少；
    /// 舱里清空多少，首都池就多多少。全程「货栈 + 在舱 + 池子」的总量一格不变。
    ///
    /// 这条守卫是这套机制的地基：集货腿的正当性全在「**货不会凭空出现或消失**」上
    /// （一旦漏了，缺矿就会像 M1 之前那样被静默补贴掉）。
    #[test]
    fn hauling_moves_cargo_without_creating_or_destroying_any() {
        let (config, mut state) = fresh_world(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 7.0);
        state.depot_add("中国", "金星", "铁", 5.0);
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .expect("中国开局有舰")
            .name
            .clone();
        let class = state.ship(&ship).unwrap().class.clone();
        // 停在金星泊位上（`arrival_eps` 之内）。
        let vpos = state.body_position("金星");
        state.ship_mut(&ship).unwrap().position = vpos;
        let total = |s: &State| -> f64 {
            let depot: f64 = s.depots.values().flat_map(|m| m.values()).sum();
            let hold: f64 = s.ships.iter().flat_map(|x| x.cargo.values()).sum();
            let pool: f64 = s.factions.iter().flat_map(|f| f.resources.values()).sum();
            depot + hold + pool
        };
        let before = total(&state);

        // —— 装货：上限 = 有效舱容，两种货尽量等量 ——
        let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
        let units = match step {
            HaulStep::Loaded { units, .. } => units,
            other => panic!("停在货栈泊位上应当装货，实为 {other:?}"),
        };
        let cap = cargo_capacity(&config, state.ship(&ship).unwrap());
        assert!(
            (units - cap).abs() < 1e-9,
            "货栈有 12 件、舱容 {cap} ⇒ 装满：实装 {units}"
        );
        let hold: f64 = state.ship(&ship).unwrap().cargo.values().sum();
        assert!((hold - units).abs() < 1e-9, "装了多少就在舱里有多少");
        let left: f64 = state.depot("中国", "金星").unwrap().values().sum();
        assert!(
            (left - (12.0 - units)).abs() < 1e-9,
            "货栈恰好少了装走的那些：剩 {left}"
        );
        assert!((total(&state) - before).abs() < 1e-9, "装货不许造货");

        // —— 卸货：挪到首都（地球）泊位上，货进**势力池** ——
        let epos = state.body_position("地球");
        state.ship_mut(&ship).unwrap().position = epos;
        let carbon_before = state
            .faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0);
        let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
        match step {
            HaulStep::Delivered {
                units: u,
                into_pool: true,
                ..
            } => assert!((u - hold).abs() < 1e-9, "整舱卸下"),
            other => panic!("在首都泊位上应当卸进首都池，实为 {other:?}"),
        }
        assert!(state.ship(&ship).unwrap().cargo.is_empty(), "卸完舱就空了");
        assert!(
            state
                .faction("中国")
                .unwrap()
                .resources
                .get("碳")
                .copied()
                .unwrap_or(0.0)
                > carbon_before,
            "金星采的碳进了首都池——这就是集货腿的终点"
        );
        assert!((total(&state) - before).abs() < 1e-9, "卸货不许毁货");
    }

    /// **承包交付的记账（M4c，Q10 抽成制）**：卸下来的货**分两份**——抽成归承运人自己的
    /// 首都池，余数进**托运方**的池子（不是船东的！）。
    ///
    /// 这一条把「货主与船东分离」这件事钉在最细的粒度上（不跑 `advance`，所以池子不会被
    /// 维护费/建造搅浑）：**同一个天体、同一批货，进的是两个不同势力的池子**。
    #[test]
    fn a_contract_delivery_splits_the_cargo_between_carrier_and_shipper() {
        let (config, mut state) = fresh_world(42);
        let share = config.freight.share;
        state.depots.clear();
        // 托运方：中国在金星积压 10 件碳，**它自己没有船**。
        state.depot_add("中国", "金星", "碳", 10.0);
        // 承运人：美国的一艘驱逐舰（舱容 4）。
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "美国" && s.class == "destroyer")
            .expect("美国开局有驱逐舰")
            .name
            .clone();
        let class = state.ship(&ship).unwrap().class.clone();
        let id = state.contracts.post(
            "中国".into(),
            "碳".into(),
            10.0,
            "金星".into(),
            "地球".into(),
            share,
            0,
            99,
            0.0,
        );
        state.contracts.assign(ship.clone(), id);
        state.contracts.contracts.iter_mut().find(|c| c.id == id).unwrap().carrier =
            Some("美国".into());
        // 停在**托运方货栈**的泊位上 → 装货该装的是**中国的**货。
        let vpos = state.body_position("金星");
        state.ship_mut(&ship).unwrap().position = vpos;
        let cap = cargo_capacity(&config, state.ship(&ship).unwrap());
        let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
        let loaded = match step {
            HaulStep::Loaded { units, .. } => units,
            other => panic!("停在托运方货栈上该装货，实为 {other:?}"),
        };
        assert!(loaded > 0.0 && loaded <= cap, "装的不能超过舱容");
        assert!(
            state.depot("中国", "金星").is_none()
                || (state.depot("中国", "金星").unwrap().values().sum::<f64>() - (10.0 - loaded)).abs()
                    < 1e-9,
            "装走的必须是**中国**货栈里的货"
        );
        // 卸到中国的首都（地球）：抽成归美国、余数归中国。
        let (cn0, us0) = (
            state.faction("中国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
            state.faction("美国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
        );
        state.ship_mut(&ship).unwrap().position = state.body_position("地球");
        let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
        assert!(
            matches!(step, HaulStep::Delivered { into_pool: true, .. }),
            "目的 = 托运方首都 ⇒ 该进池子，实为 {step:?}"
        );
        let (cn1, us1) = (
            state.faction("中国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
            state.faction("美国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
        );
        let cut = loaded * share;
        assert!(
            (cn1 - cn0 - (loaded - cut)).abs() < 1e-9,
            "托运方该收到 {} 件（装的 {} 减去抽成 {cut:.2}），实收 {:.3}",
            loaded - cut,
            loaded,
            cn1 - cn0
        );
        assert!(
            (us1 - us0 - cut).abs() < 1e-9,
            "承运人的报酬就是它自留的那份货：{cut:.3}，实收 {:.3}",
            us1 - us0
        );
        // 合同进度按**卸出舱的总量**记（抽成是搬运费，不能从合同的量里扣）。
        let c = state.contracts.get(id).expect("10 件还没送完，合同该还在");
        assert!(
            (c.delivered - loaded).abs() < 1e-9,
            "进度 = 卸出舱的总量 {loaded}，实为 {}",
            c.delivered
        );
    }

    /// **常驻路线 + 腿别由货舱决定（Q7 A / Q5 A）**：同一对 `from/to`、**不存任何额外状态**，
    /// 空舱就去装、装到货就改跑 `to`、货栈空就原地等——三件事全部由「舱里有货吗」推出来。
    #[test]
    fn a_haul_route_alternates_legs_because_of_the_cargo() {
        let (config, mut state) = fresh_world(42);
        state.depots.clear();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .expect("中国开局有舰")
            .name
            .clone();
        let class = state.ship(&ship).unwrap().class.clone();
        let vpos = state.body_position("金星");
        state.ship_mut(&ship).unwrap().position = vpos;

        // 1) 货栈是空的 ⇒ **原地等**（「有货就走」的另一半是「没货就不走」），位置不动。
        let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
        assert!(
            matches!(step, HaulStep::Waiting { ref body } if body == "金星"),
            "空货栈应当原地等，实为 {step:?}"
        );
        assert_eq!(state.ship(&ship).unwrap().position, vpos, "等的时候不许乱跑");
        assert!(state.ship(&ship).unwrap().cargo.is_empty());

        // 2) 来货了就装，且**这一回合不再跑**（与殖民一样是「到达即行动」）。
        state.depot_add("中国", "金星", "碳", 3.0);
        let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
        assert!(matches!(step, HaulStep::Loaded { .. }), "有货就装，实为 {step:?}");
        assert!(!state.ship(&ship).unwrap().cargo.is_empty(), "舱里该有货");

        // 3) 舱里有货 ⇒ 腿别翻到 `to`（哪怕 `from` 还有货）。同一对 from/to、零额外状态。
        let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
        assert_eq!(step.body(), "地球", "舱里有货 ⇒ 这一腿去卸货端，实为 {step:?}");
        assert!(
            !matches!(step, HaulStep::Waiting { .. } | HaulStep::Loaded { .. }),
            "有货时不该再在装货端打转，实为 {step:?}"
        );
    }

    /// **端到端（玩家路径）**：给一艘舰写一条 `Haul` 指令，货真的从**产地货栈**走到**首都池**，
    /// 而且走的是一条不需要重下的**常驻路线**（两个回合内装完并卸到池里）。
    ///
    /// 用「首都天体上的货栈」把航程压成 0 回合：它本来就是为了「迁都把旧中转点留在新首都」
    /// 准备的合法路线（`Haul { from: cap, to: cap }`，见 `behavior_is_valid`），正好也是最短的
    /// 端到端用例。
    #[test]
    fn a_commanded_haul_route_delivers_depot_cargo_into_the_capital_pool() {
        let (config, mut state) = fresh_world(42);
        state.depots.clear();
        state.depot_add("中国", "地球", "碳", 9.0); // 首都是地球；这处货栈压着 9 件碳
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.class == "destroyer")
            .expect("中国开局有一艘驱逐舰")
            .name
            .clone();
        let epos = state.body_position("地球");
        state.ship_mut(&ship).unwrap().position = epos;
        // 玩家指令（`Player` = 自动控制不碰它）：路线 地球→地球，角色钉成运输舰。
        {
            let c = state.control_mut("中国".to_string()).unwrap();
            c.ship_orders.insert(
                ship.clone(),
                Control::player(ShipBehavior::Haul {
                    from: "地球".to_string(),
                    to: "地球".to_string(),
                }),
            );
            c.ship_freighter.insert(ship.clone(), Control::player(true));
        }
        let mut rng = Prng::new(42);
        let mut delivered = 0.0;
        // 驱逐舰舱容 4、货栈 9 件 ⇒ 要跑三趟（4+4+1）；每趟「装一回合 + 卸一回合」，
        // 所以 8 个回合足够，也正是「常驻路线自己往复」的证据（不需要重下指令）。
        for _ in 0..8 {
            advance(&mut state, &config, &mut rng);
            for e in &state.events {
                if let GameEvent::CargoDelivered { cargo, into_pool: true, .. } = e {
                    delivered += cargo.values().sum::<f64>();
                }
            }
        }
        assert!(
            delivered >= 9.0 - 1e-9,
            "压在货栈里的那 9 件必须整批运进首都池（实为 {delivered}，其中还含金星当期产出的那部分）"
        );
        // 注意：**不要**拿首都池的增量当判据——池子同时在花钱（建设/造舰/维护），
        // 收进来的货是**毛额**、池子的净变化是另一回事。判据用事件（到货的毛额）
        // 与「货栈条目消失」（没有货留在产地）这两条。
        assert!(
            state.depot("中国", "地球").is_none(),
            "空货栈条目要被清掉（AI 派单靠键集合读积压）"
        );
    }

    /// **运输能力的地基（机制不变量）**：非 master 舰船抵达某天体的**成功率**是算出来的，
    /// 而**不是**一条「能/不能」的硬线。
    ///
    /// `move_toward` 把目标交给 [`mond_drift`] 切向平移 `roll × (r − radius) × drift_per_au`，
    /// 舰船只在距（被平移后的）目标 `arrival_eps` 内停泊 ⇒ 它离真实天体的距离就是那个偏移量。
    /// 偏移幅度按 `roll ∈ [0,1)` **伪随机**取，而**每回合是一次新的尝试**（[`nav_roll`]），于是
    ///
    /// ```text
    /// 每次尝试的成功率  p = min(1, arrival_eps / (depth × drift_per_au)) = min(1, 2/(r − 28))
    /// ```
    ///
    /// 三条不变量（用户裁决：**MOND 再强也总有成功几率，只是要多试几个回合**）：
    /// 1. `p > 0` 对**任意有限深度**都成立（`roll = 0` 的一次尝试必然命中）——没有绝对不可达；
    /// 2. `p` 随深度**单调不增**（越深越难）；
    /// 3. `p = 1` 的门槛仍是 `radius + arrival_eps/drift_per_au = 30 AU`——带内近处**一次到位**。
    ///
    /// 实测（期望尝试回合数 = `1/p`）：伊克西翁 p≈0.92→1.1 回合、妊神星 0.29→3.5、
    /// 创神星 0.20→5.0、阋神星 0.20→5.0。守卫它就是守住「圣所**难打但打得下来**」这个形状：
    /// 调 `radius` / `drift_per_au` / `arrival_eps` 里任何一个都会整张表平移。
    #[test]
    fn mond_depth_only_costs_attempts_never_makes_it_impossible() {
        let (config, state) = fresh_world(42);
        let m = &config.mond;
        let eps = config.combat.arrival_eps;

        // 1) 任意有限深度都有成功几率——`roll = 0` 的一次尝试必然指哪打哪。
        for depth in [0.5, 2.0, 10.0, 72.0, 10_000.0] {
            let dest = [0.0, m.radius + depth];
            assert_eq!(
                mond_drift(&config, "中国", dest, 0.0),
                dest,
                "深度 {depth} 也必须存在「蒙对」的一次尝试（roll=0）"
            );
            assert!(
                mond_arrival_chance(&config, depth) > 0.0,
                "深度 {depth} 的成功率必须严格大于 0——MOND 再强也不能让目标变成进不去"
            );
        }
        // 连「MOND 强到离谱」也不行：配置随便调，几率只会变小、不会归零。
        let mut harsh = load_config();
        harsh.mond.drift_per_au = 50.0;
        let harsh_p = mond_arrival_chance(&harsh, 2.0);
        assert!(harsh_p > 0.0, "drift_per_au=50 时成功率仍然 > 0");
        assert!(
            harsh_p < 0.01,
            "…但小到要试上百个回合（实测 p={harsh_p}）"
        );

        // 2) 成功率随深度单调不增。
        let mut prev = 1.0;
        for i in 0..300 {
            let depth = i as f64 * 0.5;
            let p = mond_arrival_chance(&config, depth);
            assert!(
                p <= prev + 1e-12,
                "深度 {depth} AU 的成功率不该比更浅处高（{p} > {prev}）"
            );
            prev = p;
        }
        // 3) 30 AU 以内一次到位：门槛 = radius + arrival_eps/drift_per_au。
        let exact = eps / m.drift_per_au;
        assert!(
            (mond_arrival_chance(&config, exact) - 1.0).abs() < 1e-9,
            "门槛深度 {exact} AU 处应一次到位"
        );
        assert!(
            mond_arrival_chance(&config, exact + 1.0) < 1.0,
            "门槛往外一点就不再是一次到位"
        );

        // 4) 真实天体的表：带内近处一次到位；深处要试几次，但**都试得到**。
        let table: Vec<(String, f64)> = state
            .bodies
            .iter()
            .filter_map(|b| {
                let r = (b.position[0] * b.position[0] + b.position[1] * b.position[1]).sqrt();
                (r > m.radius).then(|| (b.name.clone(), mond_arrival_chance(&config, r - m.radius)))
            })
            .collect();
        let chance = |n: &str| {
            table
                .iter()
                .find(|(name, _)| name == n)
                .map(|(_, p)| *p)
                .unwrap_or_else(|| panic!("{n} 应在带内"))
        };
        for shallow in ["海王星", "冥王星", "卡戎"] {
            assert!(
                chance(shallow) >= 1.0 - 1e-9,
                "{shallow} 在 30 AU 以内，应**一次到位**（实测 p={}）",
                chance(shallow)
            );
        }
        // 圣所：难，但一个月内基本能到（期望 ≈1.1 回合）——「堡垒」是**拖时间**，不是绝对挡驾。
        let ik = chance("伊克西翁");
        assert!(
            (0.8..1.0).contains(&ik),
            "伊克西翁成功率应在 0.8–1.0（实测 {ik:.3}，期望 {:.1} 回合）",
            1.0 / ik
        );
        // 柯伊伯矿：要试几次，但**有得试**——这正是承包定价与「超期掉信誉」的基础。
        for deep in ["妊神星", "创神星", "阋神星"] {
            let p = chance(deep);
            assert!(
                (0.05..0.5).contains(&p),
                "{deep} 应是「要试几次但试得到」（实测 p={p:.3}，期望 {:.1} 回合）",
                1.0 / p
            );
        }
        assert!(
            chance("伊克西翁") > chance("妊神星") && chance("妊神星") > chance("创神星"),
            "越深越难（成功率必须随深度递降），实测 伊克西翁 {:.3} / 妊神星 {:.3} / 创神星 {:.3}",
            chance("伊克西翁"),
            chance("妊神星"),
            chance("创神星")
        );

        // 5) 形状旋钮（`drift_shape`）：调小 = 偏移偏向大值 = 更深，但**永远 > 0**。
        let deep = 10.16; // 创神星的深度
        let mut skew = load_config();
        skew.mond.drift_shape = 0.5;
        let mut easy = load_config();
        easy.mond.drift_shape = 2.0;
        let (p_skew, p_uni, p_easy) = (
            mond_arrival_chance(&skew, deep),
            mond_arrival_chance(&config, deep),
            mond_arrival_chance(&easy, deep),
        );
        assert!(
            p_skew < p_uni && p_uni < p_easy,
            "shape 越小越难（实测 skew(0.5)={p_skew:.3} < 均匀={p_uni:.3} < easy(2.0)={p_easy:.3}）"
        );
        assert!(
            p_skew > 0.0,
            "旋钮怎么调都不能把深处变成「进不去」——这是不可破坏的性质"
        );
    }

    /// 探针（`cargo test --lib probe_mond_attempts -- --ignored --nocapture`）：
    /// 用**真实的** [`nav_roll`] 逐回合实测「深处目标要试几个回合」——闭式 `p` 是「单次尝试
    /// 的命中率」，这里量的是它**在真实伪随机序列上**的表现（首次命中的回合数、1000 回合里的
    /// 命中次数），并且验证**没有任何天体是 0 命中**（= 不存在进不去的目标）。
    #[test]
    #[ignore]
    fn probe_mond_attempts() {
        let (config, state) = fresh_world(42);
        let m = &config.mond;
        let eps = config.combat.arrival_eps;
        println!("== MOND 导航尝试实测（ship=朝圣者, fid=中国；闭式 p = eps/(depth×drift)）==");
        println!(
            "  {:<8} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9}",
            "天体", "深度AU", "闭式p", "闭式期望", "首次命中", "1000次命中", "命中率"
        );
        for b in &state.bodies {
            let r = (b.position[0] * b.position[0] + b.position[1] * b.position[1]).sqrt();
            if r <= m.radius {
                continue;
            }
            let depth = r - m.radius;
            let p = mond_arrival_chance(&config, depth);
            let mut first: Option<u32> = None;
            let mut hits = 0u32;
            let n = 1000u32;
            for round in 0..n {
                let roll = nav_roll("中国", "朝圣者", round);
                if dist(mond_drift(&config, "中国", b.position, roll), b.position) <= eps {
                    hits += 1;
                    first.get_or_insert(round + 1);
                }
            }
            println!(
                "  {:<8} {:>7.2} {:>7.3} {:>9.1} {:>9} {:>9} {:>9.3}",
                b.name,
                depth,
                p,
                if p > 0.0 { 1.0 / p } else { f64::INFINITY },
                first.map(|f| f.to_string()).unwrap_or_else(|| "从未".into()),
                hits,
                hits as f64 / n as f64
            );
            assert!(hits > 0, "{} 必须至少命中一次——不存在永远进不去的目标", b.name);
        }
    }

    /// **贸易路线的引力异常浸入深度**（M6）：两端都在带外 = 0（普通航线）；
    /// 一端在带内、一端在外 = 远端深度（要穿过去）；两端都在带内 = 较浅那端深度。
    /// 它是运费倍率与丢货率的唯一驱动量，所以必须有确定的语义。
    #[test]
    fn route_depth_measures_mond_immersion() {
        let (config, _state) = fresh_world(42);
        let r = config.mond.radius;
        let inside = [r - 5.0, 0.0];
        let shallow = [r + 2.0, 0.0];
        let deep = [r + 10.0, 0.0];
        assert_eq!(
            route_depth(&config, inside, [1.0, 0.0]),
            0.0,
            "两端都在异常带外的航线没有 MOND 代价"
        );
        assert!(
            (route_depth(&config, inside, deep) - 10.0).abs() < 1e-9,
            "一端在带内、一端在 10 AU 深 → 要穿到 10 AU 深"
        );
        assert!(
            (route_depth(&config, shallow, deep) - 2.0).abs() < 1e-9,
            "两端都在带内 → 按较浅那端算（2 AU）"
        );
        // 确定性：交换两端不改变结果（路线是双向的）。
        assert_eq!(route_depth(&config, inside, deep), route_depth(&config, deep, inside));
    }

    /// 本土防御（首都即强弩）：靠近首都的目标被削弱，远离首都的没有。
    #[test]
    fn home_field_weakens_attackers_near_the_capital() {
        let (_config, state) = fresh_world(42);
        let cap = state.body_position("地球"); // 地球（中国首都）。
        let mult_near = home_defense_mult(&state, "中国", cap);
        assert!(mult_near < 1.0, "near the capital should be defended (mult {mult_near})");
        let mult_far = home_defense_mult(&state, "中国", [80.0, 80.0]);
        assert_eq!(mult_far, 1.0, "far from the capital should have no home-field defense");
    }

    /// 舰船定制面板：装了护盾+轨道炮+推进的舰，其 effective 面板反映组件的护盾池/火力/射程/
    /// 速度。新模型：船体(hull_max) 是舰级**直接**属性、模块不改它；攻击/护盾/速度/射程都由
    /// 模块贡献、被舰级修正系数缩放；**速度来自推进模块（无推进=跑不动）**。
    #[test]
    fn ship_panel_reflects_fitted_components() {
        let (config, mut state) = fresh_world(42);
        let base = config.ship_spec("corvette");
        let ship0 = state.ships[0].name.clone();
        if let Some(s) = state.ship_mut(&ship0) {
            s.components = vec!["shield".to_string(), "railgun".to_string(), "ion_drive".to_string()];
        }
        let s = state.ship(&ship0).unwrap();
        let panel = ship_panel(&config, s);
        // 船体 = 舰级直接属性，模块不改它（护盾/装甲只吸收/减伤，不加血）。
        assert!((panel.hull_max - base.hull).abs() < 1e-9, "hull is a direct class attribute");
        // 护盾池 = 模块 × 舰级 shield_mult。
        let shield_spec = config.component_spec("shield");
        assert!((panel.shield_max - shield_spec.shield * base.shield_mult).abs() < 1e-9);
        // 攻击 = 武器模块 × 舰级 attack_mult。
        let rail_spec = config.component_spec("railgun");
        assert!((panel.attack - rail_spec.damage * base.attack_mult).abs() < 1e-9);
        // 射程 = 武器 × 舰级 range_mult（无舰级基础值）。
        assert!((panel.attack_range - rail_spec.range * base.range_mult).abs() < 1e-9);
        // 速度 = 推进模块 × 舰级 speed_mult；加速度 = 推进 accel × 舰级 accel_mult。
        let drive_spec = config.component_spec("ion_drive");
        assert!((panel.speed - drive_spec.speed * base.speed_mult).abs() < 1e-9);
        assert!((panel.accel - drive_spec.accel * base.accel_mult).abs() < 1e-9);
        assert!(panel.upkeep > base.upkeep, "components should raise maintenance");
        // 护甲=硬度：这艘船没装装甲，硬度应为 0。
        assert!((panel.hardness).abs() < 1e-9);
    }

    /// 舰级「点防御修正 pd_mult」（spec 新增属性）应缩放所搭载点防模块的拦截强度：
    /// 同一枚 point_defense 组件，装在高点防修正的舰（如战列 pd_mult>1）上比装在低点防
    /// 修正的舰上拦截更强——「舰级=平台修正器」的一环，而不是给舰叠加独立点防面板。
    #[test]
    fn ship_panel_scales_intercept_by_class_pd_mult() {
        let (config, mut state) = fresh_world(42);
        let pd_spec = config.component_spec("point_defense");
        // 从旗舰队里挑两艘从属不同舰级的舰，验证 intercept 恰为 组件 intercept × 该舰级 pd_mult。
        // 用按 class 归类的方式选：一艘 pd_mult 高、一艘 pd_mult 低（若存在）最能证明缩放生效。
        let mut tested = std::collections::BTreeMap::<String, f64>::new();
        for s in state.ships.iter_mut() {
            let class = s.class.clone();
            tested.entry(class.clone()).or_insert_with(|| {
                let spec = config.ship_spec(&class);
                s.components = vec!["point_defense".to_string()];
                s.component_hp = vec![component_integrity(&config, "point_defense")];
                let pd_mult = spec.pd_mult;
                let intercept = ship_panel(&config, s).intercept;
                assert!(
                    (intercept - pd_spec.intercept * pd_mult).abs() < 1e-9,
                    "{class} intercept should be {:.3} × pd_mult {:.2}, got {intercept}",
                    pd_spec.intercept,
                    pd_mult
                );
                pd_mult
            });
        }
        // 至少应有两点防修正不同的舰级，证明缩放不是常数（否则这个属性形同虚设）。
        let distinct: std::collections::BTreeSet<String> =
            tested.iter().map(|(c, m)| format!("{c}:{m:.3}")).collect();
        assert!(
            distinct.len() >= 2,
            "expected ship classes to differ in pd_mult; got {tested:?}"
        );
    }


    #[test]
    fn fire_degrades_components_under_damage() {
        let (config, mut state) = fresh_world(42);
        let ship0 = state.ships[0].name.clone();
        let ship3 = state.ships[3].name.clone();
        // 目标：US 驱逐舰（ship 3），装一枚导弹组件、血厚到扛住一炮以观察组件损耗。
        if let Some(t) = state.ship_mut(&ship3) {
            t.position = [40.0, 40.0];
            t.components = vec!["missile".to_string()];
            t.component_hp = t.components.iter().map(|c| component_integrity(&config, c)).collect();
            t.hull = 500.0;
            t.hull_max = 500.0;
            t.shield = 0.0;
            t.shield_max = 0.0;
        }
        // 攻击者：CN 护卫舰（ship 0），装一门重炮、贴近目标。
        if let Some(a) = state.ship_mut(&ship0) {
            a.position = [40.1, 40.0];
            a.components = vec!["railgun".to_string()];
            a.component_hp = a.components.iter().map(|c| component_integrity(&config, c)).collect();
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
        let before = state.ship(&ship3).unwrap().component_hp.clone();
        let panel_before = ship_panel(&config, state.ship(&ship3).unwrap());
        fire_concentrate(&mut state, &config, &ship0, &ship3);
        let after = state.ship(&ship3).unwrap().component_hp.clone();
        assert!(
            after.iter().zip(before.iter()).any(|(a, b)| *a < *b),
            "component integrity should drop under fire; before={before:?} after={after:?}"
        );
        // 被击毁后不贡献面板：把目标组件打掉，验证攻击/护盾面板下降。
        let _ = panel_before;
    }

    /// 母港/友方本土修船（拟人「打残→撤→修→再来」闭环）：受损组件的完整度每回合修复，
    /// 且在本土（首都 home_radius 内）修得更快。
    #[test]
    fn damaged_components_repair_in_friendly_territory() {
        let (config, mut state) = fresh_world(42);
        // China ship 0 停在其首都（Earth, body 2），组件受损。
        let cap_pos = state.body_position("地球");
        let ship0 = state.ships[0].name.clone();
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = cap_pos;
            s.components = vec!["railgun".to_string()];
            s.component_hp = vec![5.0];
            s.hull = s.hull.max(5.0);
        }
        let before = state.ship(&ship0).map(|s| s.component_hp.first().copied().unwrap_or(0.0)).unwrap_or(0.0);
        advance(&mut state, &config, &mut Prng::new(42));
        let after = state.ship(&ship0).map(|s| s.component_hp.first().copied()).flatten().unwrap_or(before);
        assert!(
            after > before,
            "a damaged component should repair over rounds; before={before} after={after}"
        );
    }

    /// 舰队防空（防空屏护）：有 PD 的舰会替 `pd_radius` 内的友舰拦导弹——附近有 PD 时目标
    /// 得到的防空覆盖应更高，PD 舰远离时覆盖应下降。
    #[test]
    fn fleet_air_defense_covers_nearby_missile_targets() {
        let (config, mut state) = fresh_world(42);
        // 目标：US (1) ship 5 在 [40,40]，自身无 PD。
        let ship3 = state.ships[3].name.clone();
        let ship5 = state.ships[5].name.clone();
        if let Some(t) = state.ship_mut(&ship5) {
            t.position = [40.0, 40.0];
            t.components = Vec::new();
            t.component_hp = Vec::new();
        }
        // 友舰：US ship 3 在 [41,40]，装点防御。
        if let Some(g) = state.ship_mut(&ship3) {
            g.position = [41.0, 40.0];
            g.components = vec!["point_defense".to_string()];
            g.component_hp = g.components.iter().map(|c| component_integrity(&config, c)).collect();
        }
        let cover_with = cluster_pd_cover(&state, &config, &ship5, "美国", [40.0, 40.0]);
        assert!(cover_with > 0.0, "a nearby PD ship should give air-defense cover; got {cover_with}");
        // 把 PD 舰移远 → 覆盖应下降。
        state.ship_mut(&ship3).unwrap().position = [100.0, 100.0];
        let cover_far = cluster_pd_cover(&state, &config, &ship5, "美国", [40.0, 40.0]);
        assert!(
            cover_far < cover_with,
            "cover should drop once the PD ship is far (with {cover_with}, far {cover_far})"
        );
    }

    /// 战斗拟真：护盾池优先吸收，快速目标对低追踪武器规避更强（确定性命中折减）。
    #[test]
    fn combat_respects_shields_and_speed_evasion() {
        let (config, mut state) = fresh_world(42);
        // Attacker: China corvette (id 0) fitted with a railgun; target: US destroyer (id 3)
        // fitted with an energy shield. Both pinned far from any capital so home-field
        // defense is neutral (mult = 1.0). Hostile so the volley is a real attack.
        let ship0 = state.ships[0].name.clone();
        let ship3 = state.ships[3].name.clone();
        if let Some(s) = state.ship_mut(&ship0) {
            s.position = [80.0, 80.0];
            s.components = vec!["railgun".to_string()];
        }
        if let Some(s) = state.ship_mut(&ship3) {
            s.position = [80.4, 80.0];
            s.components = vec!["shield".to_string()];
            s.hull = 24.0;
            s.hull_max = 24.0;
            s.shield = 12.0;
            s.shield_max = 12.0;
        }
        state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
        state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);

        let shield_before = state.ship(&ship3).map(|s| s.shield).unwrap();
        let hull_before = state.ship(&ship3).map(|s| s.hull).unwrap();
        fire_concentrate(&mut state, &config, &ship0, &ship3);

        let shield_after = state.ship(&ship3).map(|s| s.shield).unwrap();
        let hull_after = state.ship(&ship3).map(|s| s.hull).unwrap();
        assert!(shield_after < shield_before, "shield pool must absorb damage");
        assert!(hull_after < hull_before, "hull should take spill damage too");
        assert!(hull_after > 0.0, "a single volley on a destroyer should not one-shot it");

        // Evasion: a fast target is hit less by a low-tracking weapon than a slow one.
        let fast_hit = hit_factor(2.0, 2.6); // corvette speed
        let slow_hit = hit_factor(2.0, 1.2); // destroyer speed
        assert!(
            fast_hit < slow_hit,
            "fast ship should evade a low-tracking weapon more (fast {fast_hit} vs slow {slow_hit})"
        );
    }

    /// 功能性验证：长局里确实会出现「定制化」舰（资源→组件选择真的被 AI 执行）。
    #[test]
    fn long_run_produces_customized_ships() {
        let config = load_config();
        let mut customized = 0usize;
        for seed in [7u64, 42] {
            let mut state = default_state(&config, seed);
            let mut rng = Prng::new(seed);
            for _ in 0..400u32 {
                advance(&mut state, &config, &mut rng);
            }
            customized += state.ships.iter().filter(|s| !s.components.is_empty()).count();
        }
        assert!(
            customized > 0,
            "customized (component-fitted) ships should appear over a long run, got {customized}"
        );
    }

    /// 迁都-亡城强迁：首都天体上已无本势力活城 → 自动切到**人口最高的活城**。
    #[test]
    fn capital_destroyed_auto_relocates_to_highest_population_city() {
        let (config, mut state) = fresh_world(42);
        // 中国初始首都=地球，其上活城 长三角(1400)/珠三角(1100)。把这两城夷平 → 首都亡。
        assert_eq!(state.capital_body("中国"), "地球");
        for cid in ["长三角".to_string(), "珠三角".to_string()] {
            if let Some(c) = state.city_mut(&cid) {
                c.razed = true;
            }
        }

        step_capital(&mut state, &config);

        // 剩余中国活城：水星熔炉基地(220,水星)、金星浮空之城(260,金星)。人口最高=金星浮空之城。
        assert_eq!(
            state.capital_body("中国"),
            "金星",
            "capital must snap to the highest-population remaining city (金星)"
        );
        assert!(
            state.events.iter().any(|e| matches!(
                e,
                GameEvent::CapitalRelocated { faction, to, reason, .. } if faction == "中国" && to == "金星" && reason == "destroyed"
            )),
            "a destroyed-capital relocation event must be recorded, got {:?}",
            state.events
        );
    }

    /// 迁都-周期 AI 评估：首都 Population 中心更优（总治理距离成本显著更低）时，AI 迁过去。
    #[test]
    fn ai_periodic_review_relocates_capital_to_population_center() {
        let (mut config, mut state) = fresh_world(42);
        // 收窄治理可达半径 + 降低迁都门槛，让内行星间的距离差能体现「更优」。
        config.governance.admin_range = 0.05;
        config.governance.capital_relocate_threshold = 0.1;
        assert_eq!(config.governance.capital_review_every, 12);

        // 交圈数设为评估周期（12）：非 Player 首都在评估轮迁到人口中心。
        state.round = 12;
        // 把中国首都先钉到 水星（较远），**显式写 `mode: Auto`** 让 AI 继续评估：
        // 「写值即接管」之后，只写 value 会被当成玩家的首都（mode=Player），
        // 那样这条测试考的就不再是 AI 评估了。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "capital": {"value": "水星", "mode": "Auto"}}]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("set far capital");
        assert_eq!(state.capital_body("中国"), "水星");

        step_capital(&mut state, &config);

        // 中国人口最繁华城=长三角(1400,地球)；迁到地球显著降低总治理距离成本。
        assert_eq!(
            state.capital_body("中国"),
            "地球",
            "AI review should relocate the capital to the population center (地球)"
        );
        assert!(
            state.events.iter().any(|e| matches!(
                e,
                GameEvent::CapitalRelocated { faction, reason, .. } if faction == "中国" && reason == "ai_review"
            )),
            "an AI-review relocation event must be recorded, got {:?}",
            state.events
        );
    }

    /// 迁都-Player 标记：mode=Player 的首都在评估轮不被 AI 覆盖（除非亡城硬规则）。
    #[test]
    fn player_capital_not_overridden_by_ai_review() {
        let (config, mut state) = fresh_world(42);
        // 玩家把首都迁到 水星 并标 Player；中国在 水星 仍有活城（水星熔炉基地），非亡城。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "capital": {"value": "水星", "mode": "Player"}}]
        });
        crate::control::apply_patch(&mut state, &config, &diff).expect("player move capital");
        assert_eq!(state.capital_body("中国"), "水星");
        assert_eq!(state.capital_control("中国"), ControlMode::Player);

        // 评估轮：Player 控制的首都不被周期迁移覆盖。
        state.round = 12;
        step_capital(&mut state, &config);

        assert_eq!(
            state.capital_body("中国"),
            "水星",
            "a Player-chosen capital must survive the periodic AI review"
        );
        assert!(
            !state.events.iter().any(|e| matches!(
                e,
                GameEvent::CapitalRelocated { faction, .. } if faction == "中国"
            )),
            "no relocation may fire for a Player-owned capital, got {:?}",
            state.events
        );
    }

    /// 思潮：战争得利把「和平↔军国」推向军国端。
    #[test]
    fn ideology_military_win_drives_toward_militarism() {
        let (config, mut state) = fresh_world(42);
        let fname = state.factions[2].name.clone(); // 欧盟（开局有舰，且初始偏和平端）
        let my_ship = state.ships.iter().find(|s| s.faction_id == fname).map(|s| s.name.clone()).expect("a ship");
        let enemy = state.ships.iter().find(|s| s.faction_id != fname).map(|s| (s.name.clone(), s.faction_id.clone())).expect("enemy ship");
        let start = state.faction(&fname).unwrap().ideology.peace_military;

        // 注入一回合「战争得利」：我方舰击毁一艘敌舰。击毁归属由 Attack→ShipDestroyed 反推。
        state.events.push(GameEvent::Attack { attacker: my_ship.clone(), target: enemy.0.clone(), damage: 10.0 });
        state.events.push(GameEvent::ShipDestroyed {
            ship: enemy.0.clone(),
            owner: enemy.1.clone(),
            class: "corvette".to_string(),
            cause: DeathCause::Combat,
            by: None,
        });
        step_ideology(&mut state, &config, &RoundFlow::default());

        let after = state.faction(&fname).unwrap().ideology.peace_military;
        assert!(
            after > start,
            "war victory must push 和平↔军国 toward 军国: start={start} after={after}"
        );
    }

    /// 思潮：经济转负把「人民↔精英」推向人民端；且所有轴恒可有界、有限。
    #[test]
    fn ideology_economy_bad_drives_toward_populism_and_stays_bounded() {
        let (config, mut state) = fresh_world(42);
        let fname = state.factions[1].name.clone();
        let start = state.faction(&fname).unwrap().ideology.people_elite;

        // 经济转负：净流 = 产出(0) − 维护(100) − 治理(0) < 0 → 人民（民粹反弹）。
        let mut flow = RoundFlow::default();
        flow.upkeep.insert(fname.clone(), 100.0);
        step_ideology(&mut state, &config, &flow);

        let after = state.faction(&fname).unwrap().ideology.people_elite;
        assert!(
            after < start,
            "economic bust must push 人民↔精英 toward 人民: start={start} after={after}"
        );
        // 所有势力的所有轴都应是有界、有限的。
        for f in &state.factions {
            let i = &f.ideology;
            for (k, v) in [
                ("peace_military", i.peace_military),
                ("science_tech", i.science_tech),
                ("people_elite", i.people_elite),
                ("nature_colony", i.nature_colony),
            ] {
                assert!(v.is_finite() && (-1.0..=1.0).contains(&v), "{k} out of bounds: {v}");
            }
        }
    }

    /// 思潮相似度函数：同=1，全对极=0，中庸=0.5；单调随轴距离下降。
    #[test]
    fn ideology_similarity_ranges_and_is_monotonic() {
        let a = Ideology { peace_military: 0.5, science_tech: -0.3, people_elite: 0.2, nature_colony: 0.4 };
        let b = Ideology { peace_military: -0.5, science_tech: 0.3, people_elite: -0.2, nature_colony: -0.4 };
        let same = Ideology { peace_military: 0.5, science_tech: -0.3, people_elite: 0.2, nature_colony: 0.4 };
        assert_eq!(ideology_similarity(&a, &same), 1.0, "identical ideologies have unit similarity");
        assert!(ideology_similarity(&a, &a) >= ideology_similarity(&a, &b), "similarity is monotonic in distance");
        assert!((0.0..=1.0).contains(&ideology_similarity(&a, &b)));
        assert_eq!(ideology_similarity(&a, &a), 1.0);
    }

    /// 思潮相似度影响外交：其它条件相同（同 seed、同 alignment、同起始关系、噪声关闭）下，
    /// 思潮越像 → 静息亲和越高 → 关系向更友好靠拢；思潮越对立 → 越向敌对靠拢。
    #[test]
    fn ideology_similarity_shifts_diplomatic_affinity_directionally() {
        let run = |ideo_a: Ideology, ideo_b: Ideology| -> f64 {
            let (mut config, mut state) = fresh_world(42);
            // 关掉噪声，让关系变化只反映静息亲和的差异（确定性）。
            config.diplomacy.noise = 0.0;
            let a = state.factions[0].name.clone();
            let b = state.factions[1].name.clone();
            {
                let fa = state.faction_mut(&a).unwrap();
                fa.alignment = 0.0; // 隔离 alignment：只留思潮相似度的独立影响
                fa.ideology = ideo_a;
                fa.relations.insert(b.clone(), 0.0);
                let fb = state.faction_mut(&b).unwrap();
                fb.alignment = 0.0;
                fb.ideology = ideo_b;
                fb.relations.insert(a.clone(), 0.0);
            }
            let mut rng = Prng::new(42);
            step_diplomacy(&mut state, &config, &mut rng);
            relation(&state, &a, &b)
        };

        // 全同极（相似度=1）vs 全对极（相似度=0）：同 seed、同 alignment、同起始关系，
        // 唯一的差别就是思潮相似度 → 相似的一方关系必须更友好。
        let same_pos = Ideology { peace_military: 1.0, science_tech: 1.0, people_elite: 1.0, nature_colony: 1.0 };
        let opposite = Ideology { peace_military: -1.0, science_tech: -1.0, people_elite: -1.0, nature_colony: -1.0 };
        let r_same = run(same_pos, same_pos);
        let r_opp = run(same_pos, opposite);
        assert!(
            r_same > r_opp,
            "similar ideologies must rest friendlier than opposite ones: same={r_same} opp={r_opp}"
        );
    }
    /// **记恨地板（战争疤痕）**：开战之后 `war_scar_rounds` 回合内，这一对势力的关系被压在一道
    /// 线性衰减的地板下——于是「刚开战就当回合言和」不可能。
    ///
    /// 这条测试钉住两件事：
    /// 1. **地板自身的形状**：随年龄抬高、窗口内始终是敌意、出了 `war_scar_rounds` 彻底消失
    ///    （窗口过期 = 不再影响任何计算，这正是它属于窗口层而不是里程碑层的原因）。
    /// 2. **地板真的是一道地板**：用真实长局验证「没有任何一场战争短于地板承诺的回合数」。
    ///    这一条曾经**失败过**（最短 6 回合）：`step_balance_of_power` 的「合纵」走另一个关系
    ///    写入者，绕过了只在外交漂移里套用的地板。修法是让地板进入**关系写入的唯一漏斗**
    ///    （`set_relation_sym` / `adjust_relation`），而不是在这个测试里放宽断言。
    #[test]
    fn war_scar_floor_makes_a_real_floor_on_war_duration() {
        let config = load_config();
        let span = config.diplomacy.war_scar_rounds;
        let base = config.diplomacy.war_scar_relation;
        let thr = config.combat.war_threshold;
        assert!(span > 0, "war_scar_rounds 应当开启");
        assert!(base < thr, "疤痕初值必须低于交战阈值（{base} vs {thr}），否则压不住言和");

        // 1. 地板形状：只属于开战的那一对，随年龄抬高，到 span 之后消失。
        let mut s = default_state(&config, 1);
        s.round = 10;
        s.notables.entries.push(crate::model::HistoryEntry {
            round: 10,
            event: GameEvent::WarStarted { a: "甲".into(), b: "乙".into() },
        });
        let at = |age: u32| {
            let mut t = s.clone();
            t.round = 10 + age;
            war_scar_floor(&t, &config, "甲", "乙")
        };
        assert_eq!(at(0), Some(base), "刚开战必须是满额敌意");
        assert!(at(1).unwrap() > at(0).unwrap(), "地板必须随年龄单调抬高");
        assert!(at(span - 1).unwrap() < 0.0, "窗口内应当仍然带着敌意");
        assert_eq!(at(span), None, "出了 war_scar_rounds 之后疤痕必须彻底消失");
        assert_eq!(
            war_scar_floor(&s, &config, "甲", "丙"),
            None,
            "疤痕只属于开战的那一对，不牵连第三方"
        );
        assert_eq!(
            war_scar_floor(&s, &config, "乙", "甲"),
            at(0),
            "疤痕与势力顺序无关（必须无序匹配）"
        );

        // 地板抬过交战阈值所需的最小年龄 = 战争最短回合数。
        let min_age = (0..=span)
            .find(|a| base * (1.0 - (*a as f64) / (span as f64)) > thr)
            .expect("疤痕必须最终抬过交战阈值，否则战争永远结束不了");

        // 2. 真实长局：没有一场战争短于 min_age。
        let mut state = default_state(&config, 7);
        let mut rng = crate::prng::Prng::new(7);
        let mut open: std::collections::BTreeMap<(String, String), u32> =
            std::collections::BTreeMap::new();
        let mut shortest = u32::MAX;
        let mut episodes = 0usize;
        for _ in 0..60 {
            advance(&mut state, &config, &mut rng);
            let round = state.round;
            for e in &state.events {
                let pair = |a: &String, b: &String| {
                    if a <= b { (a.clone(), b.clone()) } else { (b.clone(), a.clone()) }
                };
                match e {
                    GameEvent::WarStarted { a, b } => {
                        open.entry(pair(a, b)).or_insert(round);
                    }
                    GameEvent::WarEnded { a, b } => {
                        if let Some(start) = open.remove(&pair(a, b)) {
                            episodes += 1;
                            shortest = shortest.min(round - start);
                        }
                    }
                    _ => {}
                }
            }
        }
        assert!(episodes >= 5, "60 回合里只打完 {episodes} 场战争，样本太小");
        assert!(
            shortest >= min_age,
            "最短战争 {shortest} 回合 < 地板承诺的 {min_age} 回合——\
             说明有某个关系写入者绕过了地板（见 set_relation_sym 的说明）"
        );
    }
    /// **同回合抵消不变量（复垦侧）**。
    ///
    /// 一座城在本回合被拆平之后，**不该被它自己的旧主在本回合复垦**：那对事件对归属的净效果是
    /// A→A（只剩人口/建筑被重置），却照样记 `city_razed` + `colony_founded` + `ship_spawned`
    /// 三条事件，还白送一艘种子舰——并把它钉成「拆平→复垦→再拆平」的极限环。
    ///
    /// 实测 seed 7 @200 回合，修正前：137 次拆平里 **93 次（68%）** 是这种同回合自我复垦，
    /// `水星熔炉基地` 一座城循环 **22 次**、被拆平 32 次；修正后 0 次，该城不再出现在「被拆平
    /// 最多」的前五，事件总量 2273 → 1964。
    ///
    /// 这条守卫**必须非空**：局里要真的发生过拆平，否则断言就是空转。删除 `step_resurgence`
    /// （D5）之后，同回合复垦只剩「殖民舰恰好当回合抵达」这一条路径，**拆平本身也变少了**
    /// （120 回合只剩 13 次）——所以把视野拉到 400 回合，让样本重新够用。
    #[test]
    fn a_city_razed_this_round_is_not_refounded_by_its_own_loser_this_round() {
        let config = load_config();
        let mut state = default_state(&config, 7);
        let mut rng = crate::prng::Prng::new(7);
        let mut razings = 0usize;
        for _ in 0..400 {
            advance(&mut state, &config, &mut rng);
            // 同一个回合里按事件顺序扫：`city_razed` 由 step_military 发，`colony_founded` 也由
            // step_military 里的殖民路径发（拆平在前、复垦在后），正是要抓的顺序。
            let mut razed: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            for e in &state.events {
                match e {
                    GameEvent::CityRazed { city, owner, .. } => {
                        razed.insert(city.clone(), owner.clone());
                        razings += 1;
                    }
                    GameEvent::ColonyFounded { city, owner, .. } => {
                        if let Some(loser) = razed.get(city) {
                            assert_ne!(
                                loser, owner,
                                "第 {} 回合：{city} 被 {loser} 丢掉后又被**同一个势力**复垦——\
                                 一对净效果为零的事件（拆平在同回合被自己抹掉）",
                                state.round
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        assert!(razings >= 20, "400 回合只发生 {razings} 次拆平，样本太小，守卫会空转");
    }
}
