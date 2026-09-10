//! **观测定编与选靶**：第三条角色轴的第三态（[`ShipRole::Observe`]）该有几艘、去哪、谁去。
//!
//! 用户裁决（`.agents/notes/tech-system.md` §11，裁决 9b = (a)）：角色轴加**第三态**
//! 「观测舰」——人工把舰队派去引力异常区蹲着，喂 [`crate::sim::mond_presence`] 那条
//! **唯一**的知识渠道。没有这一态，渠道就是空转的：实测（§9.2）世界里**没有任何势力**
//! 有理由把舰留在带内，掌握度全线锈回 0。
//!
//! # 「该有几艘」是**争来的**，不是定死的（用户裁决）
//!
//! 第一版给观测加了一条「最多占舰队一半」的上限，用户当场否掉：
//! **「加硬阈值只能说明动机设计的不够好，把资源堆积的运输动机和战争威胁动机覆盖了，
//! 不能加阈值要自然」**。那条上限的毛病很具体：它**越过**了另外两个动机——一处堆成山的
//! 积压（运输动机）或一支压过来的敌军（威胁动机）都压不动它，因为 50% 是写死的。
//!
//! 现在的做法：**三个角色各有自己的「主张」（头数），舰队按主张的相对大小瓜分**
//! （`freight::role_quotas` 的水位配给）。所以：
//!
//! * 积压越多 ⇒ 运输的主张越大 ⇒ 观测分到的越少（**运输动机真的能把它顶回去**）；
//! * 威胁越大 ⇒ 战舰那一份越大 ⇒ 可分的余量越小（**威胁动机真的能把它顶回去**）；
//! * 离开学满越远 ⇒ 观测的主张越大，而**思潮**（科学↔技术，见 [`observe_lean`]）
//!   决定这个主张值多少——与集货的 `freight_lean` 完全同形。
//!
//! 本模块只负责**观测那一支的主张与选靶/派单**；三支怎么分是 `freight::role_quotas`
//! 的事（所以本模块**不**依赖 `freight`，依赖是单向的）。
//!
//! 与 [`crate::autocontrol::freight`]（集货）**同形**、共用同一条纪律：
//!
//! | 件 | 集货 | 观测 |
//! |---|---|---|
//! | 主张 | 有几处货栈有积压 × 思潮倾向 | 离「学满」还差的在场强度 × 思潮倾向 |
//! | 派谁 | 缺口 × 本舰票 ÷ 同侧总票数 | 同一条公式，票按**速度**（谁先到得了） |
//! | 去哪 | `from` 按积压占比抽签 | 天体按**期望在场收益**抽签 |
//! | 骰子 | `derived_roll(fid, ship, round, "route"/"role")` | `derived_roll(fid, ship, round, "observe*")` |
//!
//! **优先级：观测 > 运输 > 战斗**（[`super::freight::should_be_role`] 的判据顺序）：
//! 集货缺一条船可以雇别人（承包市场就是干这个的），而观测**没有替代品**——渠道空转就是零。
//! 注意优先级管的是**谁先挑**（观测挑剩的才进集货抽签），不管**各有几艘**：条数由上面那套
//! 相对主张的配给定。
//!
//! 全部是**纯函数**（只读 `State`，不掷主 `Prng`），所以同一回合里谁先调用都得到同一个答案。

use crate::model::*;
use crate::sim;

/// **科学↔技术**思潮轴 → **观测倾向**（`2σ(它 × 科学端度)`）：科学端（该轴的负端）> 1、
/// 技术端 < 1，中庸 = 1.0（= 不偏不倚）。
///
/// 为什么是这根轴、这个方向：仓库里**已经**按这个方向读过它一次——`sim::governance` 的
/// 「思潮优势端自平衡」把「科学端 + 舰不在异常区」判为**言行不符**并扣忠诚
/// （`viol_sci`）。同一个世界的两处读法必须同向，否则玩家看到的是两个自相矛盾的信号。
/// 系数写死在这里、不进 config（与 `freight_lean` 的 `LEAN_GAIN` 同处置）。
const OBSERVE_LEAN_GAIN: f64 = 1.0;

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
pub(crate) const RETARGET_EPOCH: u32 = 12;

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
        .map(|b| {
            (
                b.name.clone(),
                sim::dist(b.position, [0.0, 0.0]) - config.mond.radius,
            )
        })
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
    let epoch = state.round / RETARGET_EPOCH.max(1);
    let (dest, _) = target_body_with_roll(state, config, fid, epoch);
    dest
}

/// [`target_body`] 的**判据**（纯函数，吃骰子）：返回选中的天体 + **这次抽签的账**
/// （掷出的值、池子总权重、每个候选的权重）。
///
/// `roll` 是 `(势力, "", 周期, "observe_body")` 那一枚 ∈ `[0,1)`；`None` = 没得挑
/// （带里没有候选、或权重全为 0）⇒ **不掷、也不记**。
///
/// 拍板那条路（`tactics::ai_ship_turn` 真的把观测舰派出去时）拿它去记账；
/// 读面（投影显示「这艘观测舰去哪儿」）走 [`target_body`]，**不记**——读面不该产生流水。
pub fn target_body_with_roll(
    state: &State,
    config: &GameConfig,
    fid: &str,
    epoch: u32,
) -> (Option<(BodyId, f64)>, Option<(f64, f64, Vec<(String, f64)>)>) {
    let control = sim::mond_control(state, fid);
    let cands = band_bodies(state, config);
    if cands.is_empty() {
        return (None, None);
    }
    let weights: Vec<f64> = cands
        .iter()
        .map(|(_, d)| body_weight(config, control, *d))
        .collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return (None, None);
    }
    let roll = sim::derived_roll(fid, "", epoch, "observe_body");
    let mut x = roll * total;
    let pool: Vec<(String, f64)> = cands
        .iter()
        .zip(&weights)
        .map(|((b, _), w)| (b.clone(), *w))
        .collect();
    for (i, w) in weights.iter().enumerate() {
        if x < *w {
            return (Some(cands[i].clone()), Some((roll, total, pool)));
        }
        x -= w;
    }
    (
        cands.last().cloned(),
        Some((roll, total, pool)),
    )
}

/// 本势力**科学↔技术**思潮给出的**观测倾向**（头数倍数，中庸 = 1.0）。
///
/// 与集货的 [`super::freight::freight_lean`] 完全同形：**思潮决定倾向，缺口决定量级**。
/// 科学端（该轴的**负**端）> 1 —— 想看的人多派船；技术端 < 1 —— 先把手上的东西做好。
pub fn observe_lean(state: &State, fid: &str) -> f64 {
    let sci = state
        .faction(fid)
        .map(|f| f.ideology.science_tech)
        .unwrap_or(0.0);
    2.0 * super::contract::sigmoid(OBSERVE_LEAN_GAIN * -sci)
}

/// 本势力对观测的**主张**（头数，连续量、**没有上限**）：
/// `离学满还差的头数 × 思潮倾向`，其中「还差几艘」= `在本场强度缺口 ÷ 一艘舰在目标天体
/// 处值多少`（`presence_value`，与知识渠道同一把尺子）。
///
/// 这是**主张**不是配额：配额是它和另外两支主张一起按水位分配出来的结果
/// （`freight::role_quotas`）。所以这里**故意不设「最多占舰队一半」这种上限**——上限会把
/// 「积压成山」与「大军压境」这两个动机一并越过（用户裁决：不能加阈值，要自然）。
///
/// 掌握度已经到顶（1.0，棘轮）⇒ 主张 **0**：没有东西可学了。这不是阈值断崖，而是
/// `sim::step_knowledge` 里那条棘轮的**同一句话**（到顶就不再变化 ⇒ 也没必要派人）。
pub fn observe_claim(state: &State, config: &GameConfig, fid: &str) -> f64 {
    if sim::mond_control(state, fid) >= 1.0 {
        return 0.0;
    }
    let Some((_, depth)) = target_body(state, config, fid) else {
        return 0.0;
    };
    let per_ship = presence_value(config, depth).max(1e-9);
    config.mond.knowledge.mastery_presence / per_ship * observe_lean(state, fid)
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
/// `quota` 由调用方给（= `freight::role_quotas` 分配出来的**观测配额**）——本模块不自己定
/// 条数：那是三个动机抢一支舰队的**结果**，属于 `freight` 那一层的活（依赖因此是单向的）。
///
/// 判据层（从硬到软）：
/// 1. **没分到船就不去**：配额 0（已到顶 / 没有目标天体 / 被另外两个动机挤掉）⇒ `false`
///    （在观测的舰就此退伍）；
/// 2. **物理**：速度 0 的舰到不了异常区 ⇒ `false`；
/// 3. **玩家表态**：这条轴归 `Player` ⇒ 调用方（`should_be_role`）根本不会走到这里；
/// 4. **缺口 → 按票抽签**：与集货**同一条公式** `p = (缺口 + 轮换) × 我的票 ÷ 同侧总票数`
///    ⇒ **期望入伙数正好等于缺口**（不是「每人各掷一次身份」，那样缺口大时会全舰队一起
///    入伙、下回合又一起退伍）。同侧总票数只算**掷得动的船**（钉住的、舱里有货的、
///    正在执行承包单的不在名单上——票不该投给动不了的人）。
pub fn should_observe(
    state: &State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    quota: f64,
) -> bool {
    let roll = sim::derived_roll(fid, ship_id, state.round, "observe_role");
    observe_with_roll(state, config, fid, ship_id, quota, roll).0
}

/// 观测定编的**判据**（纯函数，吃骰子）：返回是否入伙 + **这次抽签的机会值**。
///
/// 第二个值是 `Some(p)` 当且仅当骰子真被用到（`roll < p`）——早退档（没配额 / 没这条舰 /
/// 没观测能力）返回 `None`，于是记账那边不必复制早退逻辑。
///
/// **谁记账**：定编的拍板入口是 `freight::decide_role`（它同时管运输与观测两支的骰子，
/// 因为「观测优先」就长在那条判据里）；估算路（`should_be_role`）拿同一个骰子但不记。
pub(crate) fn observe_with_roll(
    state: &State,
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    quota: f64,
    roll: f64,
) -> (bool, Option<(f64, Vec<(String, f64)>)>) {
    // 1) 没分到船就不去（配额是三个动机抢完舰队的结果，见 `freight::role_quotas`）。
    if quota <= 0.0 {
        return (false, None);
    }
    let Some(ship) = state.ship(ship_id) else {
        return (false, None);
    };
    if observer_tonnage(config, ship) <= 0.0 {
        return (false, None);
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
    let mut pool: Vec<(String, f64)> = Vec::new();
    for s in state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
    {
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
        pool.push((s.name.clone(), t));
        if s.name == ship_id {
            mine = t;
        }
    }
    let tickets = tickets.max(1e-9);
    let gap = if cur {
        (others + 1.0 - quota).max(0.0)
    } else {
        (quota - others).max(0.0)
    };
    let headcount = others + if cur { 1.0 } else { 0.0 };
    let flow = gap + OBSERVER_ROTATION * headcount;
    let p = (flow * mine / tickets).min(1.0);
    let flip = roll < p;
    let observe = if cur { !flip } else { flip };
    (observe, Some((p, pool)))
}

#[cfg(test)]
#[path = "../tests/autocontrol/knowledge.rs"]
mod tests;
