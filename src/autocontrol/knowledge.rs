//! **观测定编与选靶**：第三条角色轴的第三态（[`ShipRole::Observe`]）该有几艘、去哪、谁去。
//!
//! 用户裁决（`.agents/notes/tech-system.md` §11，裁决 9b = (a)）：角色轴加**第三态**
//! 「观测舰」——人工把舰队派去引力异常区蹲着，喂 [`crate::sim::mond_presence`] 那条
//! **唯一**的知识渠道。没有这一态，渠道就是空转的：实测（§9.2）世界里**没有任何势力**
//! 有理由把舰留在带内，掌握度全线锈回 0。
//!
//! 这一层与 [`crate::autocontrol::freight`]（集货）**同形**、共用同一条纪律：
//!
//! | 件 | 集货 | 观测 |
//! |---|---|---|
//! | 需求 | 有几处货栈有积压 | 离「学满」还差多少**在场强度** |
//! | 配额 | `需求 × 思潮倾向`（头数） | `缺口在场强度 ÷ 每艘在目标深度的价值`（头数） |
//! | 派谁 | 缺口 × 本舰票 ÷ 同侧总票数 | 同一条公式，票按**速度**（谁先到得了） |
//! | 去哪 | `from` 按积压占比抽签 | 天体按**期望在场收益**抽签 |
//! | 骰子 | `derived_roll(fid, ship, round, "route"/"role")` | `derived_roll(fid, ship, round, "observe*")` |
//!
//! **优先级：观测 > 运输 > 战斗**（[`super::freight::should_be_role`] 的判据顺序）。
//! 理由：集货缺一条船可以雇别人（承包市场就是干这个的），而观测**没有替代品**——
//! 渠道空转就是零。但观测也不是无底洞：见 [`OBSERVER_FLEET_SHARE`]。
//!
//! 全部是**纯函数**（只读 `State`，不掷主 `Prng`），所以同一回合里谁先调用都得到同一个答案。

use crate::model::*;
use crate::sim;

/// 观测编队**最多占舰队（活舰）的比例**。
///
/// 这是**政策**常数（与集货的 `ROLE_ROTATION` 同类），不是物理量。它在拦一个很具体的蠢事：
/// 一支三艘舰的小势力若把三艘全派去蹲异常区，它既没有商船也没有战舰——而**三艘舰的在场
/// 强度（约 4.5）根本够不到学满（12）**，于是它会永远蹲在那里、永远拿不到棘轮、还被人灭国。
/// 「观测是副业，主力得打仗/运货」这条常识用比例写出来就够了；比例而不是硬上限，是为了
/// 不造出「第 N+1 艘舰永远进不去」那种断崖（`AGENTS.md`）。
const OBSERVER_FLEET_SHARE: f64 = 0.5;

/// 观测岗位的**轮换率**：与集货同义——即使头数正好等于配额也换手（`它 × 现状头数` 的期望
/// 换手，入伙与退伍两侧相等 ⇒ **头数不动、换的只是谁来干**）。用户裁决：「角色是动态调整的，
/// 而非固定」。岗位任期 ≈ `1 ÷ 它`。
const OBSERVER_ROTATION: f64 = 0.05;

/// **效率票的温度**（越小越接近「只让最快的船去」）。取 0.5 时最快与最慢的票数之比
/// ≈ `e^(OBSERVER_EFF_GAIN ÷ 0.5)` ≈ 20 倍——偏好很硬、但没有断崖。
const OBSERVER_WIDTH: f64 = 0.5;
/// **速度偏好**：票按 `速度 ÷ 队内最快速度 − 1 ∈ [−1,0]` 加成。观测与集货不同——观测舰要
/// **先到位**（路上不产生任何在场强度），所以效率的第一项是速度，不是舱容。
const OBSERVER_EFF_GAIN: f64 = 1.5;

/// **重新选靶的周期**（回合）：观测舰每隔这么多回合重新抽一次目标天体。
///
/// 为什么不是每回合抽：目标是**长期驻地**（舰要飞过去、要待住），每回合重掷等于让舰队在
/// 天体之间来回漂。为什么不是抽一次定终身：掌握度上来之后**该去的地方会变**（前沿往外移，
/// 深处从「期望 0.2 次到位」变成「当月到位」），所以目标必须能**跟着掌握度迁移**。
/// 12 回合（= 一年）是「够飞一趟、又不至于一辈子钉在近处」的量级。
/// 抽签的键是**回合 ÷ 它**（整数商），所以同一个周期内全势力的选择恒定。
const RETARGET_EPOCH: u32 = 12;

/// 一艘舰在深度 `depth` 处的**在场价值**——与 [`sim::mond_presence`] **同一把尺子**
/// （那一条是唯一的知识来源，这里再算一份就必须逐字一致，所以共用这个函数）。
pub fn presence_value(config: &GameConfig, depth: f64) -> f64 {
    1.0 + depth.max(0.0) * config.mond.knowledge.depth_weight
}

/// **带内天体**：`(天体, 深度)`，深度 = 日心距 − `mond.radius`，只留深度 > 0 的。
/// 按天体名排序（与 `state.bodies` 的顺序无关 ⇒ 抽签的候选次序恒定，同种子可复现）。
///
/// **含卫星**（卡戎、冥卫这种伴星也在异常区里，位置是它们的**世界坐标**，所以深度算得对）：
/// 「观测点在不在行星边上」与知识无关——要的只是**待在带里**。
pub fn band_bodies(state: &State, config: &GameConfig) -> Vec<(BodyId, f64)> {
    let mut v: Vec<(BodyId, f64)> = state
        .bodies
        .iter()
        .map(|b| (b.name.clone(), sim::dist(b.position, [0.0, 0.0]) - config.mond.radius))
        .filter(|(_, d)| *d > 0.0)
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// 一个天体的**期望在场收益**：`p(深度, 掌握度) × 在场价值(深度)`。
///
/// 为什么不是「深度 × p」（§11 初稿的写法）：实测两者在**凡人**这一端给出**相反**的答案——
/// 掌握度 0 时 `创神星`（深 10.2、p = 0.20）的 `深度 × p = 2.0` 与 `海王星`（深 2、p = 1.0）
/// 的 `2.0` **打平**，于是抽签会把人派去一个「大概率迷航、到了也未必待得住」的深空目标；
/// 而按**在场收益**算：海王星 `1.5 × 1.0 = 1.5`、创神星 `3.55 × 0.20 = 0.71` ——
/// 答案与常识一致（凡人先蹲前沿边上），且**掌握度一升，权重自动往外挪**（control = 1 时
/// 每个天体的 p 都是 1，收益 = 在场价值，最深的天体必然最重）。选这个尺子的真正理由：
/// 它衡量的正是知识渠道**真正消费的那个量**（`1 + 深度 × depth_weight`），不是它的替身。
pub fn body_weight(config: &GameConfig, control: f64, depth: f64) -> f64 {
    sim::mond_arrival_chance(config, depth, control) * presence_value(config, depth)
}

/// 本势力本回合该去**哪个天体**蹲：候选按 [`body_weight`] **抽签**（不是取最大的那个）。
///
/// 抽签而不是贪心，理由与集货派单完全一样（`AGENTS.md`）：一艘船掷一次骰子就走，
/// 无中心、无顺序依赖，而**期望上自动等于按收益成比例**。骰子的键是
/// `(势力, 周期)`——同一周期内全势力一个答案，跨周期才会迁移（见 [`RETARGET_EPOCH`]）。
pub fn target_body(state: &State, config: &GameConfig, fid: &str) -> Option<(BodyId, f64)> {
    let control = sim::mond_control(state, fid);
    let cands = band_bodies(state, config);
    if cands.is_empty() {
        return None;
    }
    let weights: Vec<f64> = cands.iter().map(|(_, d)| body_weight(config, control, *d)).collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let epoch = state.round / RETARGET_EPOCH.max(1);
    let mut roll = sim::derived_roll(fid, "", epoch, "observe_body") * total;
    for (i, w) in weights.iter().enumerate() {
        if roll < *w {
            return Some(cands[i].clone());
        }
        roll -= w;
    }
    cands.last().cloned()
}

/// 本势力本回合的**观测配额**（目标头数，连续量，不取整）。
///
/// = `学满所需的在场强度 ÷ 一艘舰在目标天体处值多少`，再被两条上界压住：
/// 1. **学满的资格**（[`KnowledgeConfig::mastery_presence`]，绝对量）：这是「观测到底要多少人」
///    的**唯一**来源——注意它**不能**写成 `(1 − 掌握度) × 它`：那样一个已经到 0.9 的势力
///    只会留一艘舰，而在场强度低于门槛时**够格**就不成立（`sim::step_knowledge` 的学满判据
///    要 48 个够格回合），掌握度反而会**锈**下去。所以人要一直派够，直到真的到顶。
/// 2. **舰队的一半**（[`OBSERVER_FLEET_SHARE`]）：观测是副业。
///
/// 掌握度已经到顶（1.0，棘轮）⇒ 配额 **0**：没有东西可学了。这不是阈值断崖，而是
/// `sim::step_knowledge` 里那条棘轮的**同一句话**（到顶就不再变化 ⇒ 也没必要派人）。
pub fn observer_quota(state: &State, config: &GameConfig, fid: &str) -> f64 {
    if sim::mond_control(state, fid) >= 1.0 {
        return 0.0;
    }
    let Some((_, depth)) = target_body(state, config, fid) else { return 0.0 };
    let per_ship = presence_value(config, depth).max(1e-9);
    let need = config.mond.knowledge.mastery_presence / per_ship;
    let fleet = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .count() as f64;
    need.min(OBSERVER_FLEET_SHARE * fleet)
}

/// 本势力此刻**已经在观测**的舰数（不含 `except`）——抽签的**现状项**。
fn observer_headcount(state: &State, fid: &str, except: &str) -> f64 {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0 && s.name != except)
        .filter(|s| state.ship_role(s.name.clone()) == ShipRole::Observe)
        .count() as f64
}

/// 一艘舰的**观测效率**（定编的排序键）：速度 ÷ 维护费。
///
/// 为什么不看舱容：观测舰**不搬货**，它要在异常区**待住**。到得了（速度）与养得起
/// （维护费）才是它的效率；速度 0 的舰（没有推进模块）**根本到不了**，返回 0 ⇒ 不进名单。
pub fn observer_tonnage(config: &GameConfig, ship: &Ship) -> f64 {
    let panel = ship_panel(config, ship);
    if panel.speed <= 0.0 {
        return 0.0;
    }
    panel.speed / panel.upkeep.max(1e-6)
}

/// **这艘舰本回合该不该去观测**。**纯函数**（骰子由 `(势力, 舰名, 回合, "observe_role")`
/// 派生 ⇒ 同一回合里 `assign_roles` 与别处拿到同一个答案，且**不消费主 `Prng`**）。
///
/// 判据层（从硬到软）：
/// 1. **没得学就不去**：配额 0（已到顶 / 没有目标天体）⇒ `false`（在观测的舰就此退伍）；
/// 2. **物理**：速度 0 的舰到不了异常区 ⇒ `false`；
/// 3. **玩家表态**：这条轴归 `Player` ⇒ 调用方（`should_be_role`）根本不会走到这里；
/// 4. **缺口 → 按票抽签**：与集货**同一条公式** `p = (缺口 + 轮换) × 我的票 ÷ 同侧总票数`
///    ⇒ **期望入伙数正好等于缺口**（不是「每人各掷一次身份」，那样缺口大时会全舰队一起
///    入伙、下回合又一起退伍）。同侧总票数只算**掷得动的船**（钉住的、舱里有货的、
///    正在执行承包单的不在名单上——票不该投给动不了的人）。
pub fn should_observe(state: &State, config: &GameConfig, fid: &str, ship_id: &str) -> bool {
    let quota = observer_quota(state, config, fid);
    if quota <= 0.0 {
        return false;
    }
    let Some(ship) = state.ship(ship_id) else { return false };
    if observer_tonnage(config, ship) <= 0.0 {
        return false;
    }
    let cur = state.ship_role(ship_id.to_string()) == ShipRole::Observe;
    let others = observer_headcount(state, fid, ship_id);
    let temp = OBSERVER_WIDTH.max(1e-9);
    let best = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| observer_tonnage(config, s))
        .fold(0.0f64, f64::max);
    // 一张票：**入伙**按高效率（快的先上）、**退伍**按低效率（慢的先走）。
    let ticket = |s: &Ship| -> f64 {
        let eff = if best > 0.0 {
            OBSERVER_EFF_GAIN * (observer_tonnage(config, s) / best - 1.0)
        } else {
            0.0
        };
        if cur {
            (-eff / temp).exp()
        } else {
            (eff / temp).exp()
        }
    };
    let mut mine = 0.0;
    let mut tickets = 0.0;
    for s in state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0) {
        if observer_tonnage(config, s) <= 0.0
            || (state.ship_role(s.name.clone()) == ShipRole::Observe) != cur
        {
            continue;
        }
        if s.name != ship_id
            && (state.ship_role_control(s.name.clone()).is_player()
                || state.contracts.assignment_of(&s.name).is_some()
                || !s.cargo.is_empty())
        {
            continue;
        }
        let t = ticket(s);
        tickets += t;
        if s.name == ship_id {
            mine = t;
        }
    }
    let tickets = tickets.max(1e-9);
    let gap = if cur { (others + 1.0 - quota).max(0.0) } else { (quota - others).max(0.0) };
    let headcount = others + if cur { 1.0 } else { 0.0 };
    let flow = gap + OBSERVER_ROTATION * headcount;
    let p = (flow * mine / tickets).min(1.0);
    let flip = sim::derived_roll(fid, ship_id, state.round, "observe_role") < p;
    if cur {
        !flip
    } else {
        flip
    }
}

#[cfg(test)]
#[path = "../tests/autocontrol/knowledge.rs"]
mod tests;
