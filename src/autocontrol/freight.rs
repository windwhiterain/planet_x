//! **集货派单**：自动控制怎么给运输舰排路线、又该让几艘舰去跑运输。
//!
//! 三件事，都是**纯函数**（只读 `State`，不掷骰、不写状态）：
//!
//! 1. [`needed_freighters`] / [`freighter_quota`]：本势力**该有多少艘运输舰**——一处有
//!    积压的货栈配一条船（需求），再乘上**思潮倾向**（[`freight_lean`]：军国少投、
//!    和平/殖民多投）；
//! 2. [`should_be_role`]：**这艘舰**是不是该跑运输——按「目标头数 − 现状头数」
//!    这个**缺口抽签**（概率分布 = 想要的比例 ⇒ 期望入伙数 = 缺口），与遍历顺序无关；
//! 3. [`route_for`]：这艘舰**该跑哪条线**——`from` 按**积压占比抽签**、`to` 永远是首都。
//!
//! # 为什么「选哪处积压」用抽签而不是贪心
//!
//! 用户裁决：**每艘船去哪个积压点的伪随机概率分布 = 积压占比**。这条规则一条话就说完，
//! 却自带三个好处：
//!
//! * **分配自动按积压成比例**：积压 100 的货栈期望分到 10 倍于积压 10 的那处的运力，
//!   不必维护「已派了几条船」的账（那本账还得考虑船在路上、船被击沉、船改行……）；
//! * **不需要协商**：每艘舰自己掷一次骰子就走，天然无中心、无顺序依赖；
//! * **仍然完全确定**：骰子是 `(势力, 舰名, 回合, "route")` 派生出来的（[`sim::derived_roll`]），
//!   同种子同回合逐字复现，且**不消费主 `Prng` 流**（否则「多派一艘船」会改变整个世界
//!   后续的掷骰，同种子可复现就退化成了「舰队数量一变后面全变」）。
//!
//! 抽签只在**需要选一条新线**时掷：`route_for` 优先续用现有路线（货栈还有货、或舱里载着货），
//! 所以船不会每回合在几处积压之间反复改道——「一票货永远运不回家」的抖动没有落点。
//!
//! # 与「角色」的分工
//!
//! 这一层**只给运输舰排线**。谁是运输舰是第三条风格轴
//! （[`State::ship_role`](crate::model::State::ship_role)）说了算，
//! 而「该有几艘」是这里的 [`needed_freighters`]——自动控制把结论写回那片叶
//! （`Player` 的叶不碰），于是玩家能覆写、AI 也不必每回合重新发明结论。
//!
//! 那片叶同时是**现状的记忆**：角色判据读的正是它（上一回合的结论），所以算的是
//! 「**还缺/超了几条腿**」而不是「我是谁」——于是头数正好等于目标时**谁都不动**，
//! 不会出现「每回合重掷身份 ⇒ 路线反复作废」。

use crate::model::*;
use crate::sim;
use std::collections::{BTreeMap, BTreeSet};

/// 一批货栈存货的**总件数**（不折算价值：「把东西搬回来」与「值多少钱」是两件事，
/// 后者交给市场）。
fn units_of(map: &ResourceMap) -> f64 {
    map.values().sum()
}

/// 本势力**有积压的货栈**（天体名 + 积压件数），按天体名序（`BTreeMap` 遍历顺序 ⇒ 确定性）。
///
/// 含**首都天体上的货栈**（若存在）：那通常意味着迁都把一处旧中转点留在了首都——
/// 把它扫进池子也是一条合法路线（`Haul { from: cap, to: cap }`）。
pub fn stocked_depots(state: &State, fid: &str) -> Vec<(BodyId, f64)> {
    state
        .depots
        .iter()
        .filter(|((f, _), _)| f == fid)
        .map(|((_, b), m)| (b.clone(), units_of(m)))
        .filter(|(_, u)| *u > 1e-9)
        .collect()
}

/// 本势力**该有多少艘运输舰**：一处有积压的货栈配一条船。
///
/// 这是个**刻意粗糙**的定编（用户的指示是「先确定机制的正确性，不着急管平衡性」）：
/// 它不含任何阈值常数，且「积压清空一处 ⇒ 那艘船自然改回战舰」（见 [`should_be_role`]）。
/// 真要做细，该考虑的是「按积压量 + 航程折算需要几艘」，那是平衡层的事。
pub fn needed_freighters(state: &State, fid: &str) -> usize {
    stocked_depots(state, fid).len()
}

/// 一艘舰的**集货运力**（定编的排序键）：`有效舱容 × 巡航速度 ÷ 维护费`。
///
/// * **舱容**只说明「一趟能装多少」；单位时间的运力还要乘**速度**——航程一定时，跑得快就是
///   跑得勤（来回时间 ≈ `2 × 航程 ÷ 巡航速度`）。只看舱容会把「装得多但慢」的船排错，
///   而在本作里速度**完全来自推进模块**：*没有推进模块的船速度是 0*，派它去运货等于派一尊
///   雕像——所以 0 速的舰**根本不该出现在运力名单里**（本函数返回 0，调用处据此剔除）。
/// * 两项都取**有效值**：舱容乘战损折算（[`cargo_capacity`]），速度取 [`ship_panel`] 的巡航速度
///   ——推进模块的**完整度**已经折在里面了。于是打残的船自动让位给完好的船。
/// * 除以**维护费**：运货的成本是养船，同样的钱能搬多少货才是舰级的运输效率；
///   这也顺手把「主力舰（战列）别去拉货」变成排序的自然结果，而不是一条特判。
pub fn freight_tonnage(config: &GameConfig, ship: &Ship) -> f64 {
    let panel = ship_panel(config, ship);
    if panel.speed <= 0.0 {
        return 0.0; // 动不了 ⇒ 运力是**零**，不是「很小」。
    }
    cargo_capacity(config, ship) * panel.speed / panel.upkeep.max(1e-6)
}

// --- 思潮 → 角色（用户裁决：「**由国家思潮决定自动控制下舰船倾向于运输还是战斗**」）----
//
// 「谁是运输舰」从**硬定编**（按运力排名取前 N、一刀切）改成**思潮驱动的概率**：
// 需求（[`needed_freighters`]：一处有积压的货栈 = 一条腿）仍然说**要多少条腿**，
// 思潮说**本势力愿意投多少条腿**。
//
// 系数**写死在这里、不进 config**（用户裁决：「不要配置了，直接耦合思潮写死」）：
// 改行为就改下面这几个带量纲注释的常数。

/// 思潮轴权重（**和平端 −1 ⟷ 军国端 +1**）：军国 ⇒ 尚武 ⇒ **少**跑运输。
const LEAN_MILITARY: f64 = 1.0;
/// 思潮轴权重（**自然端 −1 ⟷ 殖民端 +1**）：殖民 ⇒ 要给远方殖民地送补给 ⇒ **多**跑运输
/// ⇒ 对「尚武度」是**负**贡献。
///
/// 两轴**同权反号**（用户裁决：就这两轴），于是「既军国又殖民」的势力两股力量互相抵消
/// （`+1, +1 ⇒ 0`）——扩张既要打仗也要补给，它们撞在同一个标量上，不是巧合。
const LEAN_COLONY: f64 = -1.0;
/// 尚武度 → 头数倍数的斜率。**中庸（尚武度 0）⇒ 倍数正好 1.0** = 旧硬定编的行为，
/// 于是这条改动在世界的中位上**行为中性**：思潮只负责把它往两边推。
const LEAN_GAIN: f64 = 1.5;
/// **岗位轮换率**（用户裁决：「运输/战斗是**动态调整**的，而非固定」）：即使头数正好等于
/// 配额，也按 `它 × 现状头数` 的期望换手——**入伙与退伍两侧的期望相等**，所以**头数不动、
/// 换的只是「谁来干」**。岗位平均任期 ≈ `1 ÷ 它`（0.05 ⇒ 约 20 个回合，够跑几趟来回）。
///
/// 没有它，配额处两侧概率都恰好是 0 ⇒ 谁去运货**一次定终身**（那是「固定」而不是「动态」）。
const ROLE_ROTATION: f64 = 0.05;
/// **效率票的温度**：一张票 = `e^(效率加成 ÷ 它)`（见 [`should_be_role`] 的抽签）。
/// 越小越接近「只让最好的船去运」（断崖就在那个极限里），越大越是「谁去都行」。
/// 取 0.5 时最好的船与最差的船票数之比 = `e^(ROLE_EFF_GAIN ÷ 0.5)` ≈ 20 倍。
const ROLE_WIDTH: f64 = 0.5;
/// **运力效率偏好**：票数按 `运力 ÷ 队内最大运力 − 1 ∈ [−1, 0]` 加成。
///
/// 它是旧「按运力排名取前 N」的**软版本**：最好的船加成 0、最差的 `−ROLE_EFF_GAIN`，
/// 两者入伙概率之比 = `e^(ROLE_EFF_GAIN ÷ ROLE_WIDTH)` ≈ 20 倍——**偏好很硬、但没有断崖**
///（遵 `AGENTS.md`：不设进不去的目标——真没人运货时，战列舰照样会去跑）。
const ROLE_EFF_GAIN: f64 = 1.5;

// --- 三个角色怎么瓜分一支舰队（用户裁决：不许加阈值，要自然）---------------------------
//
// 角色轴上有三支力量在抢同一批船，各自有一个**主张**（头数，连续量）：
//   战舰：`威胁`（被强敌压的程度）—— 压得越狠越要多留人打仗；
//   运输：`积压 × 思潮倾向`（[`freighter_quota`]）—— 货堆得越多越想派人去搬；
//   观测：`离学满的缺口 × 思潮倾向`（`knowledge::observe_claim`）—— 想学的人才会派人去蹲。
//
// 配给规则是**水位**（water-filling），**没有任何角色上限**：
//   1. 战舰那一份先按威胁定：`war_share = WAR_BASE + WAR_THREAT_GAIN × threat_motive`，
//      剩下的 `预算 = 舰队 × (1 − war_share)` 留给运输与观测；
//   2. 两支主张都装得进预算 ⇒ **各得其所**（想要多少给多少，剩下的船留在战位上）；
//   3. 加起来超了预算 ⇒ **按主张的相对大小成比例缩水**（谁的主张大谁少挨刀）。
//
// 为什么不是「每个角色一条上限」（第一版给观测写死「最多占一半」，用户当场否掉：
// 「加硬阈值只能说明动机设计的不够好，把资源堆积的运输动机和战争威胁动机覆盖了，
// 不能加阈值要自然」）：上限会**越过**另外两个动机——积压堆成山、大军压境都压不动它，
// 因为那个数是写死的。水位配给里三支力量**互相挤压**：积压涨 ⇒ 运输的主张涨 ⇒ 观测分到的少；
// 威胁涨 ⇒ 战舰那一份涨 ⇒ 可分的余量小 ⇒ 运输与观测一起缩。这就是「自然」。
//
// 威胁读的是 [`super::shipbuilding::threat_motive`]——实测它**确实是情境量、不是常量**：
// 长局里当霸权的中国/俄罗斯 ≈ 0.01（没人威胁得了它），被压着打的星系矿业/无国界科学组织
// ≈ 0.8–0.9。
const WAR_BASE: f64 = 0.25;
/// 威胁 → 战舰份额的斜率。威胁 1.0 ⇒ `0.25 + 0.6 = 0.85`：**极端威胁下几乎全留作战舰，
/// 观测与运输一起被挤到边上**——那正是「要被打死了谁还去搞科研、谁还去搬货」。
const WAR_THREAT_GAIN: f64 = 0.6;

/// **三支力量抢舰队的结果**：`(战舰, 运输, 观测)` 的目标头数（连续量；差额留在战位上）。
///
/// 纯函数、只读 `State`（[`should_be_role`] 每艘舰都会调它，所以它**必须与调用顺序无关**）。
pub fn role_quotas(state: &State, config: &GameConfig, fid: &str) -> (f64, f64, f64) {
    let fleet = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .count() as f64;
    if fleet <= 0.0 {
        return (0.0, 0.0, 0.0);
    }
    let threat = super::shipbuilding::threat_motive(state, config, fid);
    let war_share = (WAR_BASE + WAR_THREAT_GAIN * threat).clamp(0.0, 1.0);
    let war = fleet * war_share;
    let budget = fleet - war;
    let freight = freighter_quota(state, fid);
    let observe = super::knowledge::observe_claim(state, config, fid);
    let claims = freight + observe;
    if claims <= 1e-9 {
        // 两支都没主张（没有积压、也学满了）⇒ 全军留在战位上。
        return (fleet, 0.0, 0.0);
    }
    if claims <= budget {
        // 装得下 ⇒ 各得其所；余下的船留在战位（没人主张就不该派活，而不是「补齐给谁」）。
        return (fleet - claims, freight, observe);
    }
    // 装不下 ⇒ 按相对主张成比例缩水。
    let scale = budget / claims;
    (war, freight * scale, observe * scale)
}

/// 本势力本回合的**观测配额**（= [`role_quotas`] 里那一支）——观察面与调用方用它。
pub fn observer_quota(state: &State, config: &GameConfig, fid: &str) -> f64 {
    role_quotas(state, config, fid).2
}

/// 本势力本回合的**运输配额**（水位配给**之后**的那一支）。注意与 [`freighter_quota`]
/// 那个**主张**不同：主张是「想派多少」，这里是「抢完舰队之后真的能派多少」。
pub fn freighter_quota_share(state: &State, config: &GameConfig, fid: &str) -> f64 {
    role_quotas(state, config, fid).1
}

/// 本势力此刻的**尚武度**（两轴加权和，权重见上面两个常数）。
fn martial(state: &State, fid: &str) -> f64 {
    let Some(f) = state.faction(fid) else { return 0.0 };
    LEAN_MILITARY * f.ideology.peace_military + LEAN_COLONY * f.ideology.nature_colony
}

/// 思潮 → **集货倾向**：本势力愿意投在集货上的**头数倍数**（`目标头数 = 需求 × 它`）。
///
/// `2σ(−LEAN_GAIN × 尚武度)`：中庸 ⇒ 1.0；军国 ⇒ **< 1**（宁可缺货、宁可雇人，也要把船留在
/// 战线上）；和平 / 殖民 ⇒ **> 1**（囤运力，多出来的船正好去做承运人）。
pub fn freight_lean(state: &State, fid: &str) -> f64 {
    2.0 * super::contract::sigmoid(-LEAN_GAIN * martial(state, fid))
}

/// 本回合的**目标头数**（连续量，不取整）：`需求 × 思潮倾向`。
///
/// 需求是 [`needed_freighters`]（一处有积压的货栈 = 一条腿）——**倾向乘在需求上**，
/// 所以「**没有积压 ⇒ 目标 0 ⇒ 谁都不去跑运输**」这条不变量不会被思潮冲掉。
pub fn freighter_quota(state: &State, fid: &str) -> f64 {
    needed_freighters(state, fid) as f64 * freight_lean(state, fid)
}

/// 本势力此刻**已经是运输舰**的舰数（不含 `except`）——抽签的**现状项**。
///
/// 取的是**有效角色**（叶 → 舰队默认 → 记录值）= **上一回合定下来的那个结论**，这就是迟滞
/// 的来源：船不是每回合从头掷「我是谁」，而是掷「**要不要换岗**」。玩家的钉子与舱里有货的舰
/// 都算进来——它们**确实在跑运输**，占着运力的名额。
fn hauler_headcount(state: &State, fid: &str, except: &str) -> f64 {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0 && s.name != except)
        .filter(|s| state.ship_role(s.name.clone()) == ShipRole::Freight)
        .count() as f64
}

/// **这艘舰本回合的角色**（`true` = 运输舰）。**纯函数**：同一回合对同一艘舰多次调用结果
/// 一致（骰子由 `(势力, 舰名, 回合, "role")` 派生 ⇒ `assign_roles` 与更早的 `step_contracts`
/// 拿到同一个答案），且**不消费主 `Prng`**。
///
/// 四层，从硬到软：
/// 1. **硬承诺**：正在执行承包单的舰、舱里有货的舰 ⇒ **必须是**运输舰。角色一改，那条线就
///    再没人跑、那票货就烂在舱里（所以这两条压过抽签）；
/// 2. **玩家表态**：这条轴归属解析为 `Player` ⇒ 用玩家的值，AI 一个骰子都不掷；
/// 3. **物理**：运力为 0 的舰（没有推进模块 ⇒ 速度 0）**运不了货**——派它去等于派一尊雕像；
/// 4. **思潮驱动的配额 → 按票抽签**：目标头数 = `需求 × 思潮倾向`（[`freighter_quota`]），
///    而**每个候选舰的入伙概率 = 缺口 × 它的票 ÷ 同侧总票数**——与 [`route_for`] 的
///    「按积压占比抽签」是同一条纪律（**概率分布 = 想要的比例**），于是：
///    * **期望入伙数正好 = 缺口**，不多不少（不是「每人各掷一次身份」，那样缺口大时
///      全舰队会**一起**入伙、下一回合又一起退伍——实测 12 艘舰配额 4 时会在 0 与 12
///      之间两极震荡）；
///    * **头数钉在目标上、但人员是流动的**：目标处净变化为 0（判据里不出现被这个动作本身
///      改变的量），再叠一层**轮换**（[`ROLE_ROTATION`]）——走一个、来一个，头数不动而
///      「谁来干」每回合都在动（用户裁决：「运输/战斗是**动态调整**的，而非固定」）；
///    * **票**把旧的「按运力排名取前 N」变成软的：票 = `e^(效率加成 ÷ ROLE_WIDTH)`，
///      最好的船票最重（≈ 最差船的 20 倍）⇒ **偏好很硬、但没有断崖**
///      （`AGENTS.md`：不设进不去的目标——真没人运货时，战列舰照样会被抽中）；
///    * **退伍的票按低效率**：超额时先走的是运力最差的那些（于是名单会自己换成好船）。
///
/// 名单（**同侧总票数**的分母）只算**掷得动的船**：玩家钉住的、舱里有货的、正在执行承包单的
/// 舰都不在名单上——票不该投给动不了的人，否则期望入伙数会凭空少掉。
pub fn should_be_role(state: &State, config: &GameConfig, fid: &str, ship_id: &str) -> ShipRole {
    // 1) 硬承诺（见上）。
    if state.contracts.assignment_of(ship_id).is_some() {
        return ShipRole::Freight;
    }
    let Some(ship) = state.ship(ship_id) else { return ShipRole::War };
    if !ship.cargo.is_empty() {
        return ShipRole::Freight;
    }
    // 2) 玩家表态：AI 不掷骰，直接用玩家的值（`Player` 的逐舰叶或舰队默认）。
    let role = state.ship_role(ship_id.to_string());
    if state.ship_role_control(ship_id.to_string()).is_player() {
        return role;
    }
    // 3) **三个动机抢舰队**（水位配给，见 [`role_quotas`]）：先算出本回合观测与运输各自的
    //    配额。观测**先挑**（优先级，见下一条），但**挑几条**由配给说了算——所以一处积压
    //    成山（运输主张大）或一支大军压境（战舰那一份大）都会真的把观测挤小。
    let (_, freighter_quota_share, observe_quota) = role_quotas(state, config, fid);
    // 4) **观测优先**（用户裁决：观测 > 运输 > 战斗）：观测是**唯一没有替代品**的角色——
    //    渠道空转就是零掌握度，而运输缺一条船还能雇人（承包市场就是干这个的）。选靶与抽签
    //    在 `autocontrol::knowledge`（与这里**同形**的缺口抽签）；它自己读 `state.ship_role`
    //    判断「我现在是不是观测舰」，所以入伙与退伍都在那一个函数里定。
    if super::knowledge::should_observe(state, config, fid, ship_id, observe_quota) {
        return ShipRole::Observe;
    }
    // 5) 当前角色不是运输舰 ⇒ 归零成「战舰」基线再掷运输的骰子。
    let cur = role == ShipRole::Freight;
    // 6) 物理：动不了的舰运不了货（不是阈值，是「没有推进模块就没有速度」）。
    if freight_tonnage(config, ship) <= 0.0 {
        return ShipRole::War;
    }
    // 7) 配额 → 抽签（用**水位配给之后**的那一支，不是主张）。
    let quota = freighter_quota_share;
    let others = hauler_headcount(state, fid, ship_id);
    let temp = ROLE_WIDTH.max(1e-9);
    // 运力效率加成（以**队内最大运力**为基准，尺度无关）：最好的船 = 0、最差的 = −gain。
    let best = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| freight_tonnage(config, s))
        .fold(0.0f64, f64::max);
    let tonnage = |s: &Ship| freight_tonnage(config, s);
    // 一张票：**入伙**按高效率（最好的船票最重）、**退伍**按低效率（最差的船先走）。
    let ticket = |s: &Ship| -> f64 {
        let eff = if best > 0.0 { ROLE_EFF_GAIN * (tonnage(s) / best - 1.0) } else { 0.0 };
        if cur { (-eff / temp).exp() } else { (eff / temp).exp() }
    };
    let mut mine = 0.0;
    let mut tickets = 0.0;
    for s in state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0) {
        // ⚠ **观测那一支是另一本账**：它**先挑**（优先级 观测 > 运输 > 战斗），挑走的船这一回合
        // 不再参与集货的抽签。不排掉它们的话，分母里会一直挂着「永远不加入」的观测舰，
        // 于是集货的期望入伙数被稀释、头数系统性低于配额（实测 4.30 的配额只跑到 3.65）。
        // 被观测那一支释放出来的船**下一回合**才回到这本账上——一轮的延迟，换一本干净的账。
        let s_role = state.ship_role(s.name.clone());
        if tonnage(s) <= 0.0
            || s_role == ShipRole::Observe
            || (s_role == ShipRole::Freight) != cur
        {
            continue;
        }
        if s.name != ship_id {
            // 钉住的舰不在这张名单上（玩家表态 / 舱里有货 / 正在执行承包单）。
            if state.ship_role_control(s.name.clone()).is_player()
                || state.contracts.assignment_of(&s.name).is_some()
                || !s.cargo.is_empty()
            {
                continue;
            }
        }
        let t = ticket(s);
        tickets += t;
        if s.name == ship_id {
            mine = t;
        }
    }
    let tickets = tickets.max(1e-9);
    // 缺口（我入伙时是「还缺几条腿」，我退伍时是「带上我超了几条腿」）——两者都由同一个
    // `others` 算出，所以这个动作**不改变判据本身**。
    let gap = if cur { (others + 1.0 - quota).max(0.0) } else { (quota - others).max(0.0) };
    // **轮换**：配额处也要换手（用户裁决：角色是动态调整的）。两侧都是 `ROLE_ROTATION × h`
    // ⇒ 期望「走的」与「来的」一样多 ⇒ **头数不动，换的只是谁来干**（效率票决定换谁：
    // 低效率的先走、高效率的先上）。
    let headcount = others + if cur { 1.0 } else { 0.0 };
    let flow = gap + ROLE_ROTATION * headcount;
    let p = (flow * mine / tickets).min(1.0);
    let flip = sim::derived_roll(fid, ship_id, state.round, "role") < p;
    let stay = if cur { !flip } else { flip };
    if stay {
        ShipRole::Freight
    } else {
        ShipRole::War
    }
}

/// **本回合的定编**：把「谁是运输舰」一次性写进第三条风格轴
/// （[`State::ship_role`](crate::model::State::ship_role) 那片叶）。
///
/// 每回合跑一次，且**只看本回合开始时的状态**（在 `step_ships` 的逐舰循环**之前**调用）
/// ——逐舰现算会让结论依赖舰的处理顺序，而那个顺序是按 `rng` 打乱的。
///
/// 写叶有两道闸，缺一不可：
/// 1. **只写自动控制自己开的舰**（`ship_control == Auto`）：玩家开的舰一个字节都不碰；
/// 2. **归属链判定是 `Player` 就不写**：玩家在叶上或舰队默认上表过态 ⇒ 这条轴归玩家。
///
/// 值没变就不重写：控制面的 diff 是给人读的，把同一个值每回合重写一遍只会制造噪声。
pub(crate) fn assign_roles(state: &mut State, config: &GameConfig) {
    let mut fids: Vec<String> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort();
    for fid in fids {
        // 先算完整个势力的名单再写：同一回合内几个势力的结论互不影响（也更好推理）。
        let mut plan: Vec<(String, ShipRole)> = Vec::new();
        for s in state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
        {
            if state.ship_control(s.name.clone()) != ControlMode::Auto {
                continue;
            }
            if state.ship_role_control(s.name.clone()).is_player() {
                continue;
            }
            plan.push((s.name.clone(), should_be_role(state, config, &fid, &s.name)));
        }
        for (name, role) in plan {
            let unchanged = state
                .control(fid.clone())
                .and_then(|c| c.ship_role.get(&name))
                .map(|l| l.value == role && l.mode == ControlMode::Inherit)
                .unwrap_or(false);
            if unchanged {
                continue;
            }
            if let Some(c) = state.control_mut(fid.clone()) {
                c.ship_role.insert(name, Control::inherit(role));
            }
        }
    }
}

/// 给某舰挑一条集货路线：`from` 按**积压占比**抽签，`to` 永远是本势力首都
/// （公理：首都即集散地）。`None` = 没有货要运（或势力连首都都没有）。
///
/// **优先续用现有路线**（舱里有货、或那处货栈还有货）——常驻路线不该每回合重掷。
/// 抽签细节见本模块的文档。
pub fn route_for(state: &State, fid: &str, ship_id: &str) -> Option<(BodyId, BodyId)> {
    // **执行承包单的舰**跑的是那张单的路线（接单时立的承诺，不是抽签抽出来的）：
    // 起运在**托运方**的货栈、目的在**托运方**的首都——与自有集货的目标（自己的首都）
    // 完全不同，所以这条要压在最前面，不能让它去抽自己的签。
    if let Some(id) = state.contracts.assignment_of(ship_id) {
        if let Some(c) = state.contracts.get(id) {
            return Some((c.from.clone(), c.to.clone()));
        }
    }
    let to = state.capital_body(fid);
    if to.is_empty() || state.body(&to).is_none() {
        return None;
    }
    let holding = state
        .ship(ship_id)
        .map(|s| !s.cargo.is_empty())
        .unwrap_or(false);
    let cands = stocked_depots(state, fid);
    // 续用现有路线：只要那处还有货（或舱里载着货要送），就不改道。
    if let Some(ShipBehavior::Haul { from, .. }) = state.ship_behavior(ship_id.to_string()) {
        if state.body(&from).is_some() {
            if holding || cands.iter().any(|(b, _)| *b == from) {
                return Some((from, to));
            }
        }
    }
    // 舱里有货但**没有**路线（例如玩家把指令清掉了）：先把货送回家再说。
    // `from = to = 首都` 是合法的「只卸不装」路线——`haul_step` 只看 `to`（舱里有货时腿别就是 `to`）。
    if holding {
        return Some((to.clone(), to));
    }
    if cands.is_empty() {
        return None;
    }
    // 抽签：把 [0, 总积压) 按各处积压切成区间，落在哪段就去哪儿 ⇒ 概率 = 积压占比。
    let total: f64 = cands.iter().map(|(_, u)| *u).sum();
    if total <= 0.0 {
        return None;
    }
    let mut x = sim::derived_roll(fid, ship_id, state.round, "route") * total;
    let mut from = cands.last().map(|(b, _)| b.clone())?; // 浮点兜底：落到末尾之外就取最后一处
    for (b, u) in &cands {
        if x < *u {
            from = b.clone();
            break;
        }
        x -= u;
    }
    Some((from, to))
}

// --- 雇佣运力市场：雇主侧（挂单）-----------------------------------------------
//
// 集货腿有**两条路**：自己派船（上面那套定编 + 抽签派单），或**雇人来运**。
// 这一层负责第二条路里**雇主**的那一半：把自己派不出船的**运力缺口**挂出去。
//
// 用户裁决把这条腿定成**雇佣**：单子要求的是**运力**（单位/回合），不是一票货；
// 雇主挂单的逻辑与派自己的船**同源**（一处积压配一条船的运力，缺多少挂多少）；
// 受雇方自己派船（派几条都无所谓）。机制依据见 `.agents/notes/freight-collection.md` §4。
// **接单/派工/考核/续约/解约**（受雇方那一半 + 雇主的验货）在 `autocontrol::contract`。

/// 一艘运输舰跑**某条具体航线**的吞吐（单位/回合）：`舱容 × 每回合能跑几趟`。
///
/// 每回合的趟数 = `巡航速度 ÷ 往返航程`（往返 = `2 × 距离`）——**距离必然要进来**：
/// 同样的船，跑 0.3 AU 的金星和跑 30 AU 的柯伊伯带，单位时间的运力差两个数量级。
/// 距离为 0（起终点同一天体，例如迁都留下的旧中转货栈）时按**一回合一趟**算。
///
/// 与 [`freight_tonnage`] 的分工：那个是**定编**用的排序键（跨舰比较，不含航程——
/// 比的是船本身的运输效率），这个是**某条航线**上的实际吞吐（含航程）。两者不可互换。
pub fn trip_throughput(state: &State, config: &GameConfig, ship: &Ship, from: &str, to: &str) -> f64 {
    let panel = ship_panel(config, ship);
    if panel.speed <= 0.0 {
        return 0.0; // 动不了 ⇒ 吞吐是零（与定编同一个判据）。
    }
    let d = sim::dist(state.body_position(from), state.body_position(to));
    let round_trip = 2.0 * d;
    let trips = if round_trip <= 1e-9 { 1.0 } else { panel.speed / round_trip };
    cargo_capacity(config, ship) * trips
}

/// 本势力**此刻能去跑运输的舰**（[`should_be_role`] 的名单，**扣掉正在替别人跑的**）。
///
/// 挂单发生在 `assign_roles` **之前**（见 `sim::step_contracts` 的注解），所以这里不能读
/// 角色叶——那片叶还是上一回合的结论。`should_be_role` 是**纯函数**，拿它算出来的
/// 正是本回合稍后会写进叶子、并据此派单的那批舰，因此估算与实际派单同口径。
///
/// **受雇在外的舰不算我的集货运力**：`should_be_role` 的第 0 条说「替别人跑的舰也是
/// 运输舰」（它得跑完那条线），但那是**别人的**线——把它算进「我自己能搬多少」会让雇主
/// 以为积压有着落了，从而少雇人（旧形态里这条估算还不会露馅，因为一张单只押一艘舰）。
fn serving_freighters<'a>(
    state: &'a State,
    config: &GameConfig,
    fid: &str,
) -> Vec<&'a Ship> {
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .filter(|s| state.contracts.assignment_of(&s.name).is_none())
        .filter(|s| should_be_role(state, config, fid, &s.name) == ShipRole::Freight)
        .collect()
}

/// 一处货栈里**积压最多的那种货**（同量按名字序 ⇒ 确定性）。`None` = 空了。
///
/// 它只是合同的**主货种**（用来折算货值、给人看）：受雇的船到了货栈装的是**当时有什么**
/// ——与雇主自己的运输舰完全一样（用户：「单主派发单的逻辑和派发自己运输船的逻辑一样」）。
/// 所以一处货栈**只有一张单**（旧形态是「一处货栈 × 一种货」各一张）。
fn principal_resource(map: &ResourceMap) -> Option<String> {
    let mut best: Option<(String, f64)> = None;
    for (r, v) in map {
        if *v > 1e-9 && best.as_ref().map(|(_, bv)| *v > *bv).unwrap_or(true) {
            best = Some((r.clone(), *v));
        }
    }
    best.map(|(r, _)| r)
}

/// **没人接的单不收回，而是「加价」**（用户裁决：价格做成**动态平衡**）。
///
/// * 挂单的**开叫价**统一（`freight.share`，人人都从 15% 起叫）；
/// * 一个**考核周期**（= 这条线的一个往返，与受雇方的验货节拍同一把尺子）没人接
///   ⇒ **抽成抬一档**，并把叫价起点挪到本回合（下一档要再等一个周期）；
/// * 抬到 `freight.share_max` 就不再加（雇主宁可让货烂在产地，也不会把大半货送人）。
///
/// 于是**深空/战区的价格是市场自己抬上去的**，而不是设计者用一个难度公式猜出来的。
/// **已有人接的单一个字都不动**：那时抽成是**承诺**（一份合同的条件下不该因结算顺序而变）。
fn escalate_open_contracts(state: &mut State, config: &GameConfig) {
    let f = &config.freight;
    let factor = f.share_escalation.max(1.0);
    let cap = f.share_max.max(f.share);
    let round = state.round;
    // 先算完再写（同一回合内几个势力的结论互不影响；也免得边遍历边借用）。
    let plan: Vec<(u64, f64)> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.is_open())
        .filter(|c| {
            let interval = crate::model::hire_terms(state, config, &c.from, &c.to).interval;
            round >= c.posted_round.saturating_add(interval)
        })
        .map(|c| (c.id, (c.share * factor).min(cap)))
        .collect();
    for (id, share) in plan {
        if let Some(c) = state.contracts.get_mut(id) {
            c.share = share;
            c.posted_round = round; // 重新起叫：下一档要再等一个完整周期
        }
    }
}

/// 雇主这一回合要动的一张单（先只读算完，再一次性写状态 ⇒ 同回合内几个势力互不影响）。
enum Plan {
    /// 改一张**未接单**的缺口（需求信号跟着现实走）。
    Revise { id: u64, resource: String, capacity: f64 },
    /// 撤回一张**没人接**的单（这条线不再缺运力，或货栈空了）。
    Drop { id: u64 },
    /// 挂一张新单。
    Post { shipper: FactionId, resource: String, capacity: f64, from: BodyId, to: BodyId },
}

/// 本势力**每一处货栈的运力账**：`(天体, 要求运力, 自有运力, 已雇运力, 缺口)`。
///
/// * **自有运力**落到每一处的那一份是**期望值**：派单是**按积压占比抽签**的（[`route_for`]），
///   所以「期望落到这处的那一份」= `Σ(各运输舰在这条线上的吞吐) × (这处积压 ÷ 总积压)`
///   ——与真实派单**同口径**，不是另编一个模型；
/// * **已雇运力**按**已接单合同的 `capacity`** 算（接单就是承诺），不看此刻有几条船在跑；
/// * **缺口** = `max(0, 要求 − 自有 − 已雇)`：**连续量、无阈值**，雇够了自己归零。
///
/// 一本账供两处用：雇主挂单（[`post_contracts`]）与「该不该腾个船坞去造货船」
/// （[`crate::autocontrol::shipbuilding::retool_haulers`]）。
pub(crate) fn capacity_ledger(
    state: &State,
    config: &GameConfig,
    fid: &str,
) -> Vec<(BodyId, f64, f64, f64, f64)> {
    let to = state.capital_body(fid);
    if to.is_empty() || state.body(&to).is_none() {
        return Vec::new();
    }
    let depots = stocked_depots(state, fid);
    let total: f64 = depots.iter().map(|(_, u)| *u).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let own_of: BTreeMap<BodyId, f64> = {
        let serve = serving_freighters(state, config, fid);
        depots
            .iter()
            .map(|(body, units)| {
                let rate: f64 = serve
                    .iter()
                    .map(|s| trip_throughput(state, config, s, body, &to))
                    .sum();
                (body.clone(), rate * (units / total))
            })
            .collect()
    };
    let mut committed: BTreeMap<BodyId, f64> = BTreeMap::new();
    for c in state.contracts.contracts.iter().filter(|c| c.shipper == fid && c.is_hired()) {
        *committed.entry(c.from.clone()).or_insert(0.0) += c.capacity;
    }
    depots
        .iter()
        .map(|(body, _)| {
            let need = crate::model::required_throughput(state, config, body, &to);
            let own = own_of.get(body).copied().unwrap_or(0.0);
            let hired = committed.get(body).copied().unwrap_or(0.0);
            (body.clone(), need, own, hired, (need - own - hired).max(0.0))
        })
        .collect()
}

/// 本势力**搬不动的比例** = `Σ缺口 ÷ Σ要求运力` ∈ [0, 1]（0 = 自己的船够，1 = 一件也搬不动）。
///
/// 这是**造货船的需求信号**（[`crate::autocontrol::shipbuilding::retool_haulers`]）：
/// **已经雇到人**的那部分不算缺口——雇佣市场本来就该顶掉它。所以「一直雇不到人、或雇到了
/// 也不够」才会推动船坞改产货船；而「雇得到」的势力本来就不必自己造船（分工，而不是重复建设）。
/// 没有积压（或没有首都）⇒ 0。
pub fn haul_gap(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let ledger = capacity_ledger(state, config, fid);
    let need: f64 = ledger.iter().map(|(_, n, _, _, _)| n).sum();
    let uncovered: f64 = ledger.iter().map(|(_, _, _, _, u)| u).sum();
    if need <= 0.0 {
        0.0
    } else {
        (uncovered / need).clamp(0.0, 1.0)
    }
}

/// **挂单**：把「自己派不出船的运力缺口」挂到雇佣市场上（一处货栈一张）。
///
/// # 派单逻辑与派自己的船**完全同源**（用户裁决）
///
/// 雇主先按**已有的定编规则**把自己的船派出去：一处有积压的货栈要**一条船的运力**
/// （`needed_freighters` 的那条口径），而「这条线需要多少运力」正是
/// [`crate::model::required_throughput`]（= 一条参考船在这条线上的吞吐）。
/// 两者**同尺度**，所以「我缺多少」不需要另编一套估算：
///
/// ```text
/// 缺口 = 要求运力 − 自有运力（落到这处的期望份额） − 已雇到的运力（已接单的 capacity）
/// ```
///
/// * **自有运力**那一份为什么是期望值：派单是**按积压占比抽签**的（[`route_for`]），
///   所以「期望落到这处的那一份」= `Σ(各运输舰在这条线上的吞吐) × (这处积压 ÷ 总积压)`。
///   这与真实派单**同口径**——不是另编一个模型。
/// * **已雇到的运力**按**已接单合同的 `capacity`** 算，不看此刻有几条船在跑：接下就是承诺，
///   接单那一刻它就该顶掉缺口（否则雇主要在收到第一条船之前反复挂单）。
///
/// 于是「连续量、无阈值」这条纪律自动成立：运力缺口为 0 的线**一件不挂**（`max(0, ·)`），
/// 缺口大的线挂得多——挂的是**吞吐**而不是「几条船」，所以受雇方派几条船都行。
///
/// # 需求信号跟着现实走，接单后冻结
///
/// 同一处货栈**只有一张未接单**，且它**每回合被改成此刻的缺口**（[`ContractState::open_mut`]）：
/// * 积压涨了、自己的船少了 ⇒ 缺口变大；自己的船补上了 ⇒ 缺口变小甚至**撤单**
///   （这条线不缺运力了，没必要继续请人——这是**内生的撤单**，不是「挂出去就等人接」）；
/// * 货栈被自己的船搬空 ⇒ 那张未接单直接撤回（需求信号必须跟着现实走，**哪怕现实是「没货了」**，
///   否则受雇方会照着一条不存在的需求派船过来）。
///
/// **一旦有人接了** ⇒ `capacity` 冻结（`open_mut` 只找 `carrier.is_none()` 的单）：那时它已经
/// 不是需求而是**承诺**了。
pub(crate) fn post_contracts(state: &mut State, config: &GameConfig) {
    // 先加价：一个考核周期没人接的单子，**抬一档抽成并重新起叫**。
    escalate_open_contracts(state, config);
    let mut fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort(); // 确定性：写状态的顺序不依赖势力表的排列
    let mut plan: Vec<Plan> = Vec::new();
    for fid in &fids {
        // 照公理：目的永远是自己的首都（首都即集散地）。没有首都（或首都天体不存在）
        // 就没有「集散地」，也就无从挂单。
        let to = state.capital_body(fid);
        if to.is_empty() || state.body(&to).is_none() {
            continue;
        }
        let depots = stocked_depots(state, fid);
        let total: f64 = depots.iter().map(|(_, u)| *u).sum();
        // 本势力**未接单**的单子（按起运地索引）——下面要么改它、要么撤它，不会堆成一片。
        let open: BTreeMap<BodyId, u64> = state
            .contracts
            .contracts
            .iter()
            .filter(|c| c.shipper == *fid && c.is_open())
            .map(|c| (c.from.clone(), c.id))
            .collect();
        // 货栈**空了**的未接单：撤回（`stocked_depots` 会滤掉空货栈，所以它们不会出现在下面的循环里）。
        let alive: BTreeSet<BodyId> = depots.iter().map(|(b, _)| b.clone()).collect();
        for (body, id) in &open {
            if !alive.contains(body) {
                plan.push(Plan::Drop { id: *id });
            }
        }
        if total <= 0.0 {
            continue; // 没有积压 ⇒ 没有需求（上面的清扫已经把旧单撤掉了）
        }
        // **每一处货栈的运力账**（要求 / 自有期望份额 / 已雇 / 缺口），一本账供两处用：
        // 这里挂单，[`crate::autocontrol::shipbuilding::retool_haulers`] 据此决定要不要
        // 腾个船坞去造货船——各算一份必然漂移。
        let ledger = capacity_ledger(state, config, fid);
        let by_body: BTreeMap<&BodyId, (f64, f64, f64, f64)> =
            ledger.iter().map(|(b, n, o, h, u)| (b, (*n, *o, *h, *u))).collect();
        for (body, _) in &depots {
            let Some((_need, _own, _hired, uncovered)) = by_body.get(body).copied() else {
                continue;
            };
            let Some(map) = state.depots.get(&(fid.clone(), body.clone())) else { continue };
            let Some(resource) = principal_resource(map) else { continue };
            match open.get(body) {
                // 已有的未接单：改成此刻的缺口；不缺了就撤回。
                Some(id) => {
                    if uncovered <= 1e-9 {
                        plan.push(Plan::Drop { id: *id });
                    } else if let Some(c) = state.contracts.get(*id) {
                        if (c.capacity - uncovered).abs() > 1e-9 || c.resource != resource {
                            plan.push(Plan::Revise { id: *id, resource, capacity: uncovered });
                        }
                    }
                }
                None => {
                    if uncovered > 1e-9 {
                        plan.push(Plan::Post {
                            shipper: fid.clone(),
                            resource,
                            capacity: uncovered,
                            from: body.clone(),
                            to: to.clone(),
                        });
                    }
                }
            }
        }
    }
    for p in plan {
        match p {
            // 改数/撤单不发事件：它们只是「需求变了」，不是一件**发生的事**（只有新单才是）。
            Plan::Revise { id, resource, capacity } => {
                if let Some(c) = state.contracts.get_mut(id) {
                    c.resource = resource;
                    c.capacity = capacity;
                }
            }
            Plan::Drop { id } => {
                state.contracts.release(id); // 未接单的本就没有船，收尾而已
                state.contracts.remove(id);
            }
            Plan::Post { shipper, resource, capacity, from, to } => {
                // 条款在**挂单这一刻**算好并冻结（门槛）：天体在动，每回合重算会让
                // 同一张单的条件漂移，而合同一旦挂出去，条件就该是固定的。
                let min_reputation =
                    crate::model::required_reputation(state, config, &resource, &from, &to);
                let share = config.freight.share;
                let id = state.contracts.post(
                    shipper.clone(),
                    resource.clone(),
                    capacity,
                    from.clone(),
                    to.clone(),
                    share,
                    state.round,
                    min_reputation,
                );
                sim::ev(
                    state,
                    GameEvent::ContractPosted {
                        contract: id,
                        shipper,
                        resource,
                        capacity,
                        from,
                        to,
                        share,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/autocontrol/freight.rs"]
mod tests;
