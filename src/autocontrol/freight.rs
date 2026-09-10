//! **集货派单**：自动控制怎么给运输舰排路线、又该让几艘舰去跑运输。
//!
//! 三件事，都是**纯函数**（只读 `State`，不掷骰、不写状态）：
//!
//! 1. [`needed_freighters`] / [`freighter_quota`]：本势力**该有多少艘运输舰**——一处有
//!    积压的货栈配一条船（需求），再乘上**思潮倾向**（[`freight_lean`]：军国少投、
//!    和平/殖民多投）；
//! 2. [`should_be_freighter`]：**这艘舰**是不是该跑运输——按「目标头数 − 现状头数」
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
//! （[`State::ship_freighter`](crate::model::State::ship_freighter)）说了算，
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
/// 它不含任何阈值常数，且「积压清空一处 ⇒ 那艘船自然改回战舰」（见 [`should_be_freighter`]）。
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
/// **效率票的温度**：一张票 = `e^(效率加成 ÷ 它)`（见 [`should_be_freighter`] 的抽签）。
/// 越小越接近「只让最好的船去运」（断崖就在那个极限里），越大越是「谁去都行」。
/// 取 0.5 时最好的船与最差的船票数之比 = `e^(ROLE_EFF_GAIN ÷ 0.5)` ≈ 20 倍。
const ROLE_WIDTH: f64 = 0.5;
/// **运力效率偏好**：票数按 `运力 ÷ 队内最大运力 − 1 ∈ [−1, 0]` 加成。
///
/// 它是旧「按运力排名取前 N」的**软版本**：最好的船加成 0、最差的 `−ROLE_EFF_GAIN`，
/// 两者入伙概率之比 = `e^(ROLE_EFF_GAIN ÷ ROLE_WIDTH)` ≈ 20 倍——**偏好很硬、但没有断崖**
///（遵 `AGENTS.md`：不设进不去的目标——真没人运货时，战列舰照样会去跑）。
const ROLE_EFF_GAIN: f64 = 1.5;

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
        .filter(|s| state.ship_freighter(s.name.clone()))
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
pub fn should_be_freighter(state: &State, config: &GameConfig, fid: &str, ship_id: &str) -> bool {
    // 1) 硬承诺（见上）。
    if state.contracts.assignment_of(ship_id).is_some() {
        return true;
    }
    let Some(ship) = state.ship(ship_id) else { return false };
    if !ship.cargo.is_empty() {
        return true;
    }
    // 2) 玩家表态：AI 不掷骰，直接用玩家的值（`Player` 的逐舰叶或舰队默认）。
    let cur = state.ship_freighter(ship_id.to_string());
    if state.ship_freighter_control(ship_id.to_string()).is_player() {
        return cur;
    }
    // 3) 物理：动不了的舰运不了货（不是阈值，是「没有推进模块就没有速度」）。
    if freight_tonnage(config, ship) <= 0.0 {
        return false;
    }
    // 4) 配额 → 抽签。
    let quota = freighter_quota(state, fid);
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
        if tonnage(s) <= 0.0 || state.ship_freighter(s.name.clone()) != cur {
            continue;
        }
        if s.name != ship_id {
            // 钉住的舰不在这张名单上（玩家表态 / 舱里有货 / 正在执行承包单）。
            if state.ship_freighter_control(s.name.clone()).is_player()
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
    if cur {
        !flip
    } else {
        flip
    }
}

/// **本回合的定编**：把「谁是运输舰」一次性写进第三条风格轴
/// （[`State::ship_freighter`](crate::model::State::ship_freighter) 那片叶）。
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
        let mut plan: Vec<(String, bool)> = Vec::new();
        for s in state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
        {
            if state.ship_control(s.name.clone()) != ControlMode::Auto {
                continue;
            }
            if state.ship_freighter_control(s.name.clone()).is_player() {
                continue;
            }
            plan.push((s.name.clone(), should_be_freighter(state, config, &fid, &s.name)));
        }
        for (name, role) in plan {
            let unchanged = state
                .control(fid.clone())
                .and_then(|c| c.ship_freighter.get(&name))
                .map(|l| l.value == role && l.mode == ControlMode::Inherit)
                .unwrap_or(false);
            if unchanged {
                continue;
            }
            if let Some(c) = state.control_mut(fid.clone()) {
                c.ship_freighter.insert(name, Control::inherit(role));
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

/// 本势力**此刻能去跑运输的舰**（[`should_be_freighter`] 的名单，**扣掉正在替别人跑的**）。
///
/// 挂单发生在 `assign_roles` **之前**（见 `sim::step_contracts` 的注解），所以这里不能读
/// 角色叶——那片叶还是上一回合的结论。`should_be_freighter` 是**纯函数**，拿它算出来的
/// 正是本回合稍后会写进叶子、并据此派单的那批舰，因此估算与实际派单同口径。
///
/// **受雇在外的舰不算我的集货运力**：`should_be_freighter` 的第 0 条说「替别人跑的舰也是
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
        .filter(|s| should_be_freighter(state, config, fid, &s.name))
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
        // 自有运力落到**每一处货栈**的那一份（期望值，与派单抽签同口径）。
        // 整块算完再进写循环：`serve` 借用着 `state`，而下面的计划要可变借用它。
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
        // 这一处**已经雇到**的运力（已接单的承诺，按 capacity 计）。
        let committed: BTreeMap<BodyId, f64> = {
            let mut m: BTreeMap<BodyId, f64> = BTreeMap::new();
            for c in state.contracts.contracts.iter().filter(|c| c.shipper == *fid && c.is_hired())
            {
                *m.entry(c.from.clone()).or_insert(0.0) += c.capacity;
            }
            m
        };
        for (body, _) in &depots {
            let need = crate::model::required_throughput(state, config, body, &to);
            let own = own_of.get(body).copied().unwrap_or(0.0);
            let hired = committed.get(body).copied().unwrap_or(0.0);
            let uncovered = (need - own - hired).max(0.0); // 连续量，无阈值
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
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::world::default_state;

    fn fresh(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let state = default_state(&config, seed);
        (config, state)
    }

    /// 某势力此刻的**运输舰名单**（按舰名序）——用例里到处要看它。
    fn roster(state: &State, fid: &str) -> Vec<String> {
        let mut v: Vec<String> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0 && state.ship_freighter(s.name.clone()))
            .map(|s| s.name.clone())
            .collect();
        v.sort();
        v
    }

    /// 钉住一个势力的**思潮两轴**（只有这两轴进集货倾向，见 `LEAN_MILITARY`/`LEAN_COLONY`）。
    fn set_ideology(state: &mut State, fid: &str, military: f64, colony: f64) {
        let f = state
            .factions
            .iter_mut()
            .find(|f| f.name == fid)
            .unwrap_or_else(|| panic!("没有势力 {fid}"));
        f.ideology.peace_military = military;
        f.ideology.nature_colony = colony;
    }

    /// 把某势力的舰队克隆 `times` 倍（名字加后缀）——用例需要一支**够大的**舰队，
    /// 否则「按比例投几条腿」会被舰队规模顶住，看不出思潮的差别。
    fn grow_fleet(state: &mut State, fid: &str, times: usize) {
        let base: Vec<Ship> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid)
            .cloned()
            .collect();
        for k in 1..=times {
            for s in &base {
                let mut c = s.clone();
                c.name = format!("{}-{k}", s.name);
                state.ships.push(c);
            }
        }
    }

    /// 连跑 `rounds` 个回合的**角色定编**（每回合先推进 `round` 再定编，与 `sim` 同步：
    /// 骰子是 `(势力, 舰名, 回合, "role")` 派生的，**换回合才换骰子**）。
    /// 返回每回合的运输舰名单。
    fn run_roles(state: &mut State, config: &GameConfig, fid: &str, rounds: u32) -> Vec<Vec<String>> {
        let mut hist = Vec::new();
        for _ in 0..rounds {
            state.round += 1;
            assign_roles(state, config);
            hist.push(roster(state, fid));
        }
        hist
    }

    /// **定编 = 有积压的货栈数**，且它随积压清空自动归零（船自然改回战舰）。
    #[test]
    fn crew_size_is_one_ship_per_stocked_depot() {
        let (_config, mut state) = fresh(42);
        state.depots.clear();
        assert_eq!(needed_freighters(&state, "中国"), 0, "没有积压就没有运输舰");
        state.depot_add("中国", "金星", "碳", 1.0);
        assert_eq!(needed_freighters(&state, "中国"), 1, "一处积压配一条船");
        state.depot_add("中国", "水星", "铁", 1.0);
        assert_eq!(needed_freighters(&state, "中国"), 2, "两处积压配两条船");
        // 清空积压 ⇒ 定编回 0（「积压清空那艘船就改回战舰」的机制落点）。
        state.depots.clear();
        assert_eq!(needed_freighters(&state, "中国"), 0);
    }

    /// **抽签分布 = 积压占比**（用户裁决）：两处货栈积压 3:1 时，多条舰抽出来的比例要贴近 3:1。
    ///
    /// 这里直接验证机制而不跑模拟：同一回合里换舰名掷骰子，看落点分布。
    #[test]
    fn route_lottery_is_proportional_to_the_backlog() {
        let (_config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 30.0);
        state.depot_add("中国", "水星", "铁", 10.0);
        let mut hits = std::collections::BTreeMap::<String, usize>::new();
        for i in 0..4000 {
            let ship = format!("抽签舰{i}");
            if let Some((from, _)) = route_for(&state, "中国", &ship) {
                *hits.entry(from).or_insert(0) += 1;
            }
        }
        let venus = hits.get("金星").copied().unwrap_or(0) as f64;
        let mercury = hits.get("水星").copied().unwrap_or(0) as f64;
        assert!(venus + mercury > 3900.0, "每艘舰都该抽到一处：{hits:?}");
        let ratio = venus / mercury;
        assert!(
            (2.6..3.4).contains(&ratio),
            "积压 3:1 ⇒ 抽中比例应贴近 3:1，实为 {ratio:.2}（{hits:?}）"
        );
    }

    /// **角色是控制属性、AI 会写它、玩家能压住它**（用户裁决：像风格一样）。
    ///
    /// 三件事一起钉：有积压时 AI 会把运输舰定出来（结论确实落在叶子上、模式是 `Inherit`）；
    /// **运力最好的船优先**（旧的硬排序现在是**软**的，但偏好仍要看得出来）；**玩家把叶设成
    /// `Player` 之后自动控制再也不碰它**（哪怕积压清空——否则「我明明钉了角色却没生效」）。
    #[test]
    fn the_ai_writes_the_role_leaf_but_never_over_a_player() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 100.0);
        let hist = run_roles(&mut state, &config, "中国", 60);
        let with_hauler = hist.iter().filter(|r| !r.is_empty()).count();
        assert!(with_hauler > 35, "有积压就该有人跑运输（60 回合里只有 {with_hauler} 回合有）");
        // 运力最好的船优先：开局是「护卫 ×2 + 驱逐 ×1」，驱逐的运力最高
        //（4×1.3÷2.5 = 2.08 vs 2×1.0÷1.5 = 1.33），它被选中的回合数该多于任何一艘护卫。
        let destroyer = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.class == "destroyer")
            .expect("中国开局有驱逐舰")
            .name
            .clone();
        let d = hist.iter().filter(|r| r.contains(&destroyer)).count();
        let c_max = hist
            .iter()
            .map(|r| r.iter().filter(|n| **n != destroyer).count())
            .max()
            .unwrap_or(0);
        assert!(d > c_max, "运力高的船该被优先选中：驱逐 {d} 回合 vs 单艘护卫最多 {c_max} 回合");
        // 结论落在叶子上，且模式是 `Inherit`（玩家把**舰队默认**设成 Player 时能压过 AI）。
        let (hauler, leaf) = state
            .control("中国".to_string())
            .and_then(|c| {
                c.ship_freighter
                    .iter()
                    .find(|(_, l)| l.value)
                    .map(|(n, l)| (n.clone(), l.clone()))
            })
            .expect("AI 该在某个回合写过一片 true 的叶");
        assert_eq!(
            leaf.mode,
            ControlMode::Inherit,
            "AI 写的是 Inherit（「这一层没有说话」）——与舰指令同一条规矩：\
             于是玩家把**舰队默认**设成 Player 时，玩家的意图能压过 AI 的逐舰结论"
        );

        // 玩家钉死这艘舰的角色 ⇒ 自动定编一个字都不许写（哪怕积压已经清空）。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_freighter
            .insert(hauler.clone(), Control::player(true));
        state.depots.clear();
        assign_roles(&mut state, &config);
        assert!(
            state.ship_freighter(hauler.clone()),
            "玩家钉的角色：AI 不得改写（哪怕没有积压）"
        );
        assert_eq!(
            state
                .control("中国".to_string())
                .unwrap()
                .ship_freighter
                .get(&hauler)
                .unwrap()
                .mode,
            ControlMode::Player,
            "那片叶仍然归玩家"
        );
    }

    /// **删叶 = 交回自动定编**：玩家给某艘舰钉过角色（`Player`）之后 AI 一个字都不写；
    /// 把这片叶删掉，这艘舰立刻回到「AI 按积压 + 思潮定编」的自由状态——之后 AI 会把结论
    /// 重新写进一片新叶。这正是这条轴与另两条风格轴的差别：**删叶不是"锁成某个值"，而是"放手"**。
    #[test]
    fn deleting_the_role_leaf_hands_the_ship_back_to_auto_planning() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .map(|s| s.name.clone())
            .expect("中国至少有一艘舰");
        // 玩家钉死「它是运输舰」，而此刻没有任何积压 ⇒ AI 本来不会给它这个角色。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_freighter
            .insert(ship.clone(), Control::player(true));
        assign_roles(&mut state, &config);
        assert!(state.ship_freighter(ship.clone()));
        assert!(
            state.control("中国".to_string()).unwrap().ship_freighter.contains_key(&ship),
            "玩家的叶 AI 不碰，所以它还在"
        );

        // 删叶：玩家放手 ⇒ 归属不再拦着 AI。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "ship_freighter": [{"ship": ship, "remove": true}]}]
        });
        let r = crate::control::apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
        assert!(
            !state.control("中国".to_string()).unwrap().ship_freighter.contains_key(&ship),
            "叶必须真的没了"
        );

        // 有积压 ⇒ 定编重新生效（角色是掷骰定的，所以看的是「若干回合内有人被定上」）。
        state.depot_add("中国", "金星", "碳", 100.0);
        let hist = run_roles(&mut state, &config, "中国", 40);
        assert!(
            hist.iter().any(|r| !r.is_empty()),
            "删掉玩家的钉子之后 AI 重新定编：40 回合里一个运输舰都没定出来"
        );
    }

    /// **运力要算速度**（用户点破的那条）：一趟装多少只是**每趟**的量，单位时间的运力是
    /// `舱容 × 速度`（航程一定时，跑得快 = 跑得勤）。而且速度完全来自推进模块 ⇒
    /// **没有推进模块的船速度是 0，派它去运货等于派一尊雕像**：它必须被剔出运力名单。
    #[test]
    fn freight_tonnage_counts_speed_and_never_picks_a_ship_that_cannot_move() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 100.0);
        state.depot_add("中国", "火星", "铁", 100.0);
        state.depot_add("中国", "水星", "硅", 100.0);
        // 把一艘护卫拆成**裸舰**：没有推进模块 ⇒ 巡航速度 0。
        let stripped = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.class == "corvette")
            .expect("中国开局有护卫舰")
            .name
            .clone();
        {
            let s = state.ship_mut(&stripped).unwrap();
            s.components.clear();
            s.component_hp.clear();
        }
        assert_eq!(
            ship_panel(&config, state.ship(&stripped).unwrap()).speed,
            0.0,
            "用例前提：裸舰没有推进模块 ⇒ 速度 0"
        );
        assert_eq!(
            freight_tonnage(&config, state.ship(&stripped).unwrap()),
            0.0,
            "速度 0 ⇒ 运力为零（不是「很小」）"
        );
        // 公式本身：运力 = 舱容 × 速度 ÷ 维护费，其中舱容按战损**连续**折算
        //（把一艘完好的舰打到半血 ⇒ 运力减半）。
        let intact = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.name != stripped && s.hull > 0.0)
            .unwrap()
            .clone();
        let t = freight_tonnage(&config, &intact);
        assert!(t > 0.0, "完好的舰运力必须为正");
        let mut hurt = intact.clone();
        hurt.hull = hurt.hull_max * 0.5;
        assert!(
            (freight_tonnage(&config, &hurt) - t * 0.5).abs() < 1e-9,
            "装甲掉一半 ⇒ 舱容减半 ⇒ 运力减半"
        );

        // 速度 0 的舰**一个回合都不会**被定成运输舰（物理，不是「排序靠后」）。
        let hist = run_roles(&mut state, &config, "中国", 200);
        assert!(
            hist.iter().all(|r| !r.contains(&stripped)),
            "速度 0 的舰物理上运不了货——一回合都不该被派去跑运输"
        );
        // 三处积压 ⇒ 目标头数 3（中庸），而只有 2 艘动得了 ⇒ 那两艘该基本都常在名单上。
        let mean = hist.iter().map(|r| r.len() as f64).sum::<f64>() / hist.len() as f64;
        assert!(mean > 1.85, "缺口大过候选数 ⇒ 两艘动得了的基本常驻名单（平均 {mean:.2}）");
        // **软排序**：两艘里运力高的那艘被选中的回合数不少于低的那艘。
        let movable: Vec<(String, f64)> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == "中国" && s.hull > 0.0 && s.name != stripped)
            .map(|s| (s.name.clone(), freight_tonnage(&config, s)))
            .collect();
        assert_eq!(movable.len(), 2, "用例前提：只剩两艘动得了的船");
        let hits = |n: &str| hist.iter().filter(|r| r.contains(&n.to_string())).count();
        let (a, ta) = &movable[0];
        let (b, tb) = &movable[1];
        let (hi, lo) = if ta > tb { (a, b) } else { (b, a) };
        assert!(
            hits(hi) >= hits(lo),
            "运力高的船该更容易被选中：{hi}（{:.2}）{hits_hi} 回合 vs {lo}（{:.2}）{hits_lo} 回合",
            ta.max(*tb),
            ta.min(*tb),
            hits_hi = hits(hi),
            hits_lo = hits(lo)
        );
    }

    /// **有效角色的取值链**：叶 → 舰队默认 → 舰上记录值，与前两条风格轴同形。
    /// 舰队默认要是 `Player`，逐舰的叶就说了不算（这是「玩家意图压过 AI 定编」的机制落点）。
    #[test]
    fn the_effective_role_follows_the_leaf_then_the_fleet_default_then_the_record() {
        let (_config, mut state) = fresh(42);
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .unwrap()
            .name
            .clone();
        // 记录值（出厂快照）：护卫舰 = 战舰。
        assert!(!state.ship_freighter(ship.clone()), "护卫舰出厂不是运输舰");
        // 舰队默认（Player）⇒ 全舰队改口。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .default_freighter = Some(Control::player(true));
        assert!(
            state.ship_freighter(ship.clone()),
            "叶没有说话（压根没有）时，Player 的舰队默认说了算"
        );
        // 逐舰的叶（Player）更具体 ⇒ 压过舰队默认。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_freighter
            .insert(ship.clone(), Control::player(false));
        assert!(
            !state.ship_freighter(ship.clone()),
            "更具体的叶（逐舰 Player）压过舰队默认"
        );
    }

    /// **思潮决定倾向**（用户裁决：「由国家思潮决定自动控制下舰船倾向于运输还是战斗」）。
    ///
    /// 同一个世界、同一批骰子、同一份积压，**只改思潮两轴**：军国端少跑运输、殖民/和平端
    /// 多跑运输；而**中庸端正好是 1.0 = 旧硬定编**（这条改动在世界的中位上是行为中性的）。
    #[test]
    fn ideology_decides_how_much_of_the_fleet_hauls() {
        let (config, base) = fresh(42);
        let stock_four = |st: &mut State| {
            st.depots.clear();
            for b in ["金星", "水星", "火星", "木星"] {
                st.depot_add("中国", b, "碳", 100.0);
            }
        };
        let mean_headcount = |mil: f64, col: f64| -> f64 {
            let mut st = base.clone();
            set_ideology(&mut st, "中国", mil, col);
            grow_fleet(&mut st, "中国", 3); // 12 艘 ⇒ 舰队规模不顶住「按比例投几条腿」
            stock_four(&mut st);
            let hist = run_roles(&mut st, &config, "中国", 120);
            hist.iter().map(|r| r.len() as f64).sum::<f64>() / hist.len() as f64
        };

        // 中庸（尚武度 0）⇒ 倍数**恰好** 1.0：`2σ(0) = 1` ⇒ 目标头数 = 需求 = 4 处货栈。
        let mut neutral_state = base.clone();
        set_ideology(&mut neutral_state, "中国", 0.0, 0.0);
        stock_four(&mut neutral_state);
        assert!(
            (freight_lean(&neutral_state, "中国") - 1.0).abs() < 1e-12,
            "中庸必须回到旧硬定编（倍数 1.0），实为 {}",
            freight_lean(&neutral_state, "中国")
        );
        assert!((freighter_quota(&neutral_state, "中国") - 4.0).abs() < 1e-12);

        let militarist = mean_headcount(1.0, 0.0);
        let neutral = mean_headcount(0.0, 0.0);
        let pacifist = mean_headcount(-1.0, 0.0);
        let colonist = mean_headcount(0.0, 1.0);
        // 抽签的期望**正好**是配额（实测 200 回合：1.52 vs 1.46、3.97 vs 4.00、6.47 vs 6.54）。
        // 这条是「概率分布 = 想要的比例」那条纪律的守卫：机制走形（比如每人各掷一次身份）
        // 时它立刻会炸——实测过那种写法会在 0 与 12 之间两极震荡。
        for (mil, col, quota) in [(1.0, 0.0, 1.46), (0.0, 0.0, 4.0), (-1.0, 0.0, 6.54)] {
            let got = mean_headcount(mil, col);
            assert!(
                (got - quota).abs() < 0.35,
                "平均头数该贴着配额（思潮 {mil}×军事 + {col}×殖民 ⇒ 配额 {quota}）：实为 {got:.2}"
            );
        }
        assert!(militarist < neutral, "军国端该少跑运输：{militarist:.2} vs {neutral:.2}");
        assert!(pacifist > neutral, "和平端该多跑运输：{pacifist:.2} vs {neutral:.2}");
        assert!(
            colonist > neutral,
            "殖民端要给远方殖民地送补给 ⇒ 该多跑运输（所以它在「尚武度」上是负权重）：{colonist:.2} vs {neutral:.2}"
        );
        // 两轴**同权反号**：既军国又殖民 ⇒ 两股力量抵消（回到中庸附近）。
        let both = mean_headcount(1.0, 1.0);
        assert!(
            (both - neutral).abs() < 1.0,
            "军国 + 殖民该互相抵消：{both:.2} vs 中庸 {neutral:.2}"
        );
    }

    /// **AI 端到端（线路接通）**：有积压时自动控制会定出运输舰并给它排一条线；积压清空后
    /// 那名额自然收回（船改回战舰）。
    #[test]
    fn the_ai_assigns_a_route_when_there_is_a_backlog_and_recalls_it_after() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 100.0);
        // 角色是**掷骰**定的（有积压只是「有人去运」的概率高），所以这里跑几个回合而不是一个：
        // 这正是与旧版硬定编的行为差别，用例必须照新语义写，而不是照旧结论写。
        let mut rng = crate::prng::Prng::new(42);
        let mut found = None;
        for _ in 0..20 {
            sim::advance(&mut state, &config, &mut rng);
            if let Some(n) = roster(&state, "中国").into_iter().next() {
                found = Some(n);
                break;
            }
        }
        let hauler = found.expect("有积压 ⇒ 若干回合内该定出运输舰");
        assert!(
            matches!(state.ship_behavior(hauler.clone()), Some(ShipBehavior::Haul { .. })),
            "运输舰该有一条路线，实为 {:?}",
            state.ship_behavior(hauler.clone())
        );
        // 积压清空 + 舱里也没货 ⇒ 名额收回。这里**每回合都清货栈**（模拟里金星会当期产出新的碳、
        // 货栈立刻又有货——那是正确行为，不是这个用例要测的事）。
        let mut recalled = false;
        for _ in 0..20 {
            state.depots.clear();
            for s in state.ships.iter_mut() {
                s.cargo.clear();
            }
            state.round += 1;
            assign_roles(&mut state, &config);
            if !state.ship_freighter(hauler.clone()) {
                recalled = true;
                break;
            }
        }
        assert!(recalled, "没有积压了 ⇒ 该把运输舰的名额收回去（船改回战舰）");
        // 配额为 0 时**超额是确定的**（带上我就是超一条），所以收回是必然的、且很快。
    }

    /// **头数稳、人员流动**（用户裁决：「运输/战斗是**动态调整**的，而非固定」；同时也是
    /// 笔记里那条「判据里不要出现被这个动作本身改变的量」的守卫——角色读的是上一回合的结论）。
    ///
    /// 两件事一起钉：
    /// 1. **条数**钉在配额上（±1 的呼吸），不会两极震荡；
    /// 2. **谁去干**每回合都在换（轮换）——既不钉死，也不每回合翻烙饼。
    #[test]
    fn the_headcount_holds_at_the_quota_while_the_crew_rotates() {
        let (config, base) = fresh(42);
        let setup = |mil: f64, col: f64, depots: &[&str]| {
            let mut st = base.clone();
            set_ideology(&mut st, "中国", mil, col);
            grow_fleet(&mut st, "中国", 3);
            st.depots.clear();
            for b in depots {
                st.depot_add("中国", b, "碳", 100.0);
            }
            st
        };
        // 1) 头数围着目标（4）站住，不会在两极之间摆。
        let mut st = setup(0.0, 0.0, &["金星", "水星", "火星", "木星"]);
        let hist = run_roles(&mut st, &config, "中国", 200);
        // 整数配额（需求 4 × 中庸 1.0）⇒ 头数该贴着 4（±1 的呼吸，而不是两极震荡）。
        let in_band = hist.iter().filter(|r| (3..=5).contains(&r.len())).count();
        assert!(
            in_band as f64 / hist.len() as f64 > 0.9,
            "头数该贴着目标：{in_band} / {} 回合落在 3..=5",
            hist.len()
        );
        // 2) 换岗是**慢**的：平均每回合进出的船数远小于 1。
        let churn: usize = hist
            .windows(2)
            .map(|w| w[1].iter().filter(|n| !w[0].contains(n)).count())
            .sum();
        let per_round = churn as f64 / (hist.len() - 1) as f64;
        // **动态但不抖**：轮换让岗位一直换手，而缺口项把换手量压在「岗位数」这个量级里
        //（实测 0.52 条/回合 ⇒ 每条岗位平均 8 个回合换人 ≈ 跑得完几趟来回）。
        assert!(
            (0.2..1.5).contains(&per_round),
            "换手该是「一直在动、但不成片翻烙饼」（实为 {per_round:.3} 条/回合）"
        );
        // 3) 没有积压 ⇒ 全员战舰；新积压一出现 ⇒ 几回合内补得上（不是「一旦改成战舰就回不去」）。
        let mut st = setup(0.0, 0.0, &[]);
        run_roles(&mut st, &config, "中国", 10);
        assert!(roster(&st, "中国").is_empty(), "没有积压 ⇒ 谁都不该占着运输舰的名额");
        st.depot_add("中国", "金星", "碳", 100.0);
        let mut waited = 0;
        for _ in 0..20 {
            st.round += 1;
            assign_roles(&mut st, &config);
            waited += 1;
            if !roster(&st, "中国").is_empty() {
                break;
            }
        }
        assert!(waited <= 10, "新积压该在几回合内被顶上（实为 {waited} 回合）");
    }

    /// **续用现有路线**：货栈还有货时不改道（常驻路线不抖动）；货栈空了才重掷。
    #[test]
    fn an_existing_route_is_kept_while_it_still_has_cargo() {
        let (_config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 5.0);
        state.depot_add("中国", "水星", "铁", 500.0); // 积压大变（若重掷，几乎必去水星）
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .unwrap()
            .name
            .clone();
        // 先给这艘舰写一条去金星的路线。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_orders
            .insert(
                ship.clone(),
                Control::auto(ShipBehavior::Haul {
                    from: "金星".to_string(),
                    to: "地球".to_string(),
                }),
            );
        let picked = route_for(&state, "中国", &ship).unwrap();
        assert_eq!(picked.0, "金星", "金星还有货 ⇒ 续用现有路线，不按积压重掷");
        // 金星清空 ⇒ 才重掷（这次必然去水星，因为只剩它一处）。
        state.depots.remove(&("中国".to_string(), "金星".to_string()));
        let picked = route_for(&state, "中国", &ship).unwrap();
        assert_eq!(picked.0, "水星", "原路线没货了 ⇒ 重新抽签");
    }

    // --- 雇佣挂单（雇主的缺口口径）---------------------------------------------

    /// 挂的是**自己派不出船的运力缺口**：`要求运力 − 自有运力（落到这处的期望份额）− 已雇到的`。
    ///
    /// 这里**不重算实现里的公式**，而是先量出「一条船都没有时挂多少」（= 这条线的要求运力），
    /// 再用它预测「已经有受雇方顶掉一部分」时该挂多少。这样验证的是**形状**
    /// （仿射、斜率 −1、下界处夹到 0），而不是把实现抄一遍——公式改了但形状错了，它照样报错。
    #[test]
    fn the_order_asks_for_the_capacity_the_employer_cannot_cover() {
        let (config, mut state) = fresh(42);
        state.ships.retain(|s| s.faction_id != "中国"); // 中国的船全没了 ⇒ 自有运力 0
        let gap_at = |state: &mut State, hired: f64| -> f64 {
            state.depots.clear();
            state.contracts.contracts.clear();
            state.depot_add("中国", "金星", "碳", 100.0);
            if hired > 0.0 {
                // 一张**已接单**的合同，承诺了 `hired` 的运力（它就该顶掉缺口）。
                let id = state.contracts.post(
                    "中国".into(), "碳".into(), hired, "金星".into(), "地球".into(), 0.1, 0, 0.0,
                );
                let c = state.contracts.get_mut(id).unwrap();
                c.carrier = Some("美国".into());
                c.accepted_round = Some(0);
                c.expires_round = 99;
            }
            post_contracts(state, &config);
            state.contracts.contracts.iter().filter(|c| c.is_open()).map(|c| c.capacity).sum()
        };
        let need = gap_at(&mut state, 0.0);
        assert!(need > 0.0, "一条船都没有 ⇒ 该把整条线的要求运力挂出去（实为 {need:.3}）");
        // 已经雇到四成 ⇒ 只该挂剩下的六成。
        let partial = gap_at(&mut state, need * 0.4);
        assert!(
            (partial - need * 0.6).abs() < 1e-9,
            "已雇到 40% ⇒ 该挂 60%（{:.3}），实为 {partial:.3}",
            need * 0.6
        );
        // 雇够了（甚至雇多了）⇒ 一件都不挂（`max(0, ·)` 的下界，不是阈值判断）。
        assert_eq!(gap_at(&mut state, need * 1.2), 0.0, "雇够了 ⇒ 不必再请人");
        // 自己有船 ⇒ 缺口变小（缺口可以小到 0：自己的船把这条线顶上了，那就不必请人）。
        let with_ships = {
            let (config, mut st) = fresh(42);
            // 只留**一艘**护卫（舱容 2、速度 1.0）：能顶掉一部分，但顶不满整条线的要求运力。
            let keep = st
                .ships
                .iter()
                .find(|s| s.faction_id == "中国" && s.class == "corvette")
                .expect("中国开局有护卫舰")
                .name
                .clone();
            st.ships.retain(|s| s.faction_id != "中国" || s.name == keep);
            // 这艘船必须**确实在跑运输**：角色现在是掷骰定的（思潮驱动），所以这里用一片
            // `Player` 的叶把它钉住——`should_be_freighter` 对归玩家的轴不掷骰。
            st.control_mut("中国".to_string())
                .unwrap()
                .ship_freighter
                .insert(keep.clone(), Control::player(true));
            st.depots.clear();
            st.contracts.contracts.clear();
            st.depot_add("中国", "金星", "碳", 100.0);
            post_contracts(&mut st, &config);
            st.contracts.contracts.iter().filter(|c| c.is_open()).map(|c| c.capacity).sum::<f64>()
        };
        assert!(
            with_ships > 0.0 && with_ships < need,
            "一条小船顶不满 ⇒ 缺口该在 (0, {need:.3}) 之间，实为 {with_ships:.3}"
        );
    }

    /// **一处货栈只有一张未接单，而且它每回合跟着缺口走；已接单的冻结成承诺。**
    ///
    /// 三个必须成立的行为：缺口变了**改的是同一张单**（不新开、不留旧数）；
    /// 货栈被搬空 ⇒ **撤单**（受雇方不该照着不存在的需求派船过来）；
    /// 有人接了 ⇒ 一个字都不再动。
    #[test]
    fn an_open_order_follows_the_gap_while_a_hired_one_is_frozen() {
        let (config, mut state) = fresh(42);
        state.ships.retain(|s| s.faction_id != "中国"); // 没有运力 ⇒ 挂单 = 整条线的要求运力
        state.depots.clear();
        state.contracts.contracts.clear();
        let post = |state: &mut State, stock: f64| {
            state.depots.clear();
            if stock > 0.0 {
                state.depot_add("中国", "金星", "碳", stock);
            }
            post_contracts(state, &config);
        };
        post(&mut state, 1000.0);
        assert_eq!(state.contracts.contracts.len(), 1, "一处货栈一张单");
        let id = state.contracts.contracts[0].id;
        let capacity = state.contracts.contracts[0].capacity;
        assert!(capacity > 0.0, "该挂出这条线的要求运力");
        assert_eq!(state.contracts.contracts[0].resource, "碳", "主货种 = 积压最多的那种");

        // 积压变小/变大 ⇒ **同一张单**（运力要求与积压量无关，所以这里该一个字都不变）。
        post(&mut state, 400.0);
        assert_eq!(state.contracts.contracts.len(), 1, "仍是同一张单，不是第二张");
        assert_eq!(state.contracts.contracts[0].id, id, "单号不变（改数不是新单）");

        // 有人接了 ⇒ 冻结：此后货栈怎么变都不再改这张单（它已经是**承诺**）。
        state.contracts.contracts[0].carrier = Some("美国".into());
        state.contracts.contracts[0].accepted_round = Some(0);
        state.contracts.contracts[0].expires_round = 99;
        post(&mut state, 100.0);
        assert!(
            (state.contracts.contracts[0].capacity - capacity).abs() < 1e-9,
            "已接单的合同冻结，实为 {}",
            state.contracts.contracts[0].capacity
        );
        assert_eq!(state.contracts.contracts.len(), 1, "有人接了就不再开第二张");

        // 没人接 + 货没了 ⇒ **撤单**（需求信号必须跟着现实走，哪怕现实是「没货了」）。
        state.contracts.contracts[0].carrier = None;
        state.contracts.contracts[0].accepted_round = None;
        post(&mut state, 0.0);
        assert!(state.contracts.contracts.is_empty(), "货栈空了 ⇒ 未接单的该撤回");
    }

    /// **没人接 ⇒ 每过一个考核周期抬一档抽成**（用户裁决：价格做成**动态平衡**）。
    ///
    /// 抬价的节拍就是**这条线的一个往返**（与受雇方的验货节拍同一把尺子），不是另设一个
    /// 「多久没人接就加价」的数；抬到 `share_max` 就不再加。**已接单的冻结**。
    #[test]
    fn an_unaccepted_order_escalates_once_per_review_period() {
        let (config, mut state) = fresh(42);
        let open = state.contracts.post(
            "中国".into(), "碳".into(), 3.0, "金星".into(), "地球".into(), config.freight.share, 0, 0.6,
        );
        let taken = state.contracts.post(
            "中国".into(), "铁".into(), 3.0, "水星".into(), "地球".into(), config.freight.share, 0, 0.6,
        );
        {
            let c = state.contracts.get_mut(taken).unwrap();
            c.carrier = Some("美国".into());
            c.accepted_round = Some(0);
            c.expires_round = 99;
        }
        let interval = crate::model::hire_terms(&state, &config, "金星", "地球").interval;
        assert!(interval >= 1, "考核周期至少一回合");
        let share0 = state.contracts.get(open).unwrap().share;
        // 还没过一个周期 ⇒ 不加价（单子该有机会在开叫价上被接走）。
        for r in 0..interval {
            state.round = r;
            escalate_open_contracts(&mut state, &config);
        }
        assert_eq!(
            state.contracts.get(open).unwrap().share,
            share0,
            "一个考核周期之内不该加价"
        );
        // 满一个周期 ⇒ 抬一档，并把叫价起点挪到本回合。
        state.round = interval;
        escalate_open_contracts(&mut state, &config);
        let c = state.contracts.get(open).expect("没人接的单**留在簿上**继续叫价");
        assert!(
            (c.share - share0 * config.freight.share_escalation).abs() < 1e-9,
            "满一个周期该抬一档：{share0:.3} → {:.3}",
            c.share
        );
        assert_eq!(c.posted_round, state.round, "抬价后重新起叫（下一档要再等一个完整周期）");
        assert_eq!(
            state.contracts.get(taken).unwrap().share,
            share0,
            "已接单的合同抽成**冻结**（那是承诺）"
        );
        // 反复过期 ⇒ 抬到上限为止。
        for _ in 0..40 {
            state.round += 1000;
            escalate_open_contracts(&mut state, &config);
        }
        let c = state.contracts.get(open).unwrap();
        assert!(
            (c.share - config.freight.share_max).abs() < 1e-9,
            "抬价有上限（{:.2}）：实为 {:.3}",
            config.freight.share_max,
            c.share
        );
        assert!(state.contracts.get(taken).is_some(), "已接单的合同不会因为加价被动过");
    }

    /// **AI 端到端**：一条船都没有 ⇒ 把整条线的**要求运力**挂到雇佣市场上，并发一条事件。
    #[test]
    fn the_ai_posts_an_order_for_the_capacity_it_cannot_cover() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 100.0);
        state.ships.retain(|s| s.faction_id != "中国"); // 中国没有舰 ⇒ 自有运力 0
        let mut rng = crate::prng::Prng::new(42);
        sim::advance(&mut state, &config, &mut rng);
        // 挂出来的那张单**可能已经在本回合被接走**（撮合与派工都在 `step_contracts` 里）——
        // 所以要看的是「簿上那张属于中国的单」，而不是「还没人接的单」。
        let mine: Vec<&crate::model::Contract> =
            state.contracts.contracts.iter().filter(|c| c.shipper == "中国").collect();
        assert_eq!(mine.len(), 1, "一处积压一张单，实为 {:?}", state.contracts.contracts);
        let need =
            crate::model::required_throughput(&state, &config, "金星", &state.capital_body("中国"));
        assert!(
            (mine[0].capacity - need).abs() < 1e-9,
            "没有运力 ⇒ 该挂整条线的要求运力（应挂 {need:.3}，实为 {:.3}）",
            mine[0].capacity
        );
        assert_eq!(mine[0].from, "金星", "起运 = 产地货栈");
        assert_eq!(mine[0].to, state.capital_body("中国"), "目的照公理 = 雇主首都");
        assert!((mine[0].share - config.freight.share).abs() < 1e-12, "抽成 = 配置里的费率");
        assert!(mine[0].min_reputation > 0.0, "门槛要在挂单时算好并冻结");
        assert!(
            state
                .events
                .iter()
                .any(|e| matches!(e, GameEvent::ContractPosted { .. })),
            "挂单要发事件（否则投影/故事板里这件事不存在）"
        );
    }

    /// **挂单是确定性的**：同一个世界跑两次，挂出来的单子逐字相同。
    ///
    /// 这条是 `AGENTS.md` 那条纪律的守卫：新机制**绝不消费主 `Prng` 流**——挂单用的是纯
    /// 公式（连派生骰子都没用），所以「多挂一张单」不会改变世界后续的掷骰。
    ///
    /// 布景要**明确造出缺口**（把一个势力的船全撤走）：默认开局里各家舰队基本都能顶上自己
    /// 那几处货栈，一回合下来往往一张单都不挂——那样这条守卫就是空转的。
    #[test]
    fn posting_the_same_world_twice_yields_the_same_orders() {
        let (config, state0) = fresh(42);
        let run = || {
            let mut state = state0.clone();
            state.ships.retain(|s| s.faction_id != "中国"); // 中国没有船 ⇒ 必然要请人
            let mut rng = crate::prng::Prng::new(42);
            sim::advance(&mut state, &config, &mut rng);
            state
                .contracts
                .contracts
                .iter()
                .map(|c| (c.id, c.shipper.clone(), c.from.clone(), c.capacity, c.share, c.min_reputation))
                .collect::<Vec<_>>()
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "同种子同回合的挂单必须逐字相同");
        assert!(!a.is_empty(), "造了缺口就该有单子可测（否则这条守卫是空转的）");
    }
    /// 【探针·思潮→角色】逐思潮打印：倾向倍数、目标头数、平均头数、换岗率、头数分布。
    /// 跑法：`cargo test --lib probe_ideology_roles -- --ignored --nocapture`。
    #[test]
    #[ignore]
    fn probe_ideology_roles() {
        let (config, base) = fresh(42);
        println!("--- 思潮 → 集货倾向（配额 4 处货栈、12 艘舰、200 回合）---");
        for (tag, mil, col) in [
            ("军国 +1", 1.0, 0.0),
            ("偏军国 +0.5", 0.5, 0.0),
            ("中庸  0", 0.0, 0.0),
            ("偏和平 -0.5", -0.5, 0.0),
            ("和平 -1", -1.0, 0.0),
            ("殖民 +1", 0.0, 1.0),
            ("军国+殖民", 1.0, 1.0),
        ] {
            let mut st = base.clone();
            set_ideology(&mut st, "中国", mil, col);
            grow_fleet(&mut st, "中国", 3);
            st.depots.clear();
            for b in ["金星", "水星", "火星", "木星"] {
                st.depot_add("中国", b, "碳", 100.0);
            }
            let lean = freight_lean(&st, "中国");
            let quota = freighter_quota(&st, "中国");
            let hist = run_roles(&mut st, &config, "中国", 200);
            let mean = hist.iter().map(|r| r.len() as f64).sum::<f64>() / hist.len() as f64;
            let churn: usize = hist
                .windows(2)
                .map(|w| w[1].iter().filter(|n| !w[0].contains(n)).count())
                .sum();
            let mut dist = std::collections::BTreeMap::<usize, usize>::new();
            for r in &hist {
                *dist.entry(r.len()).or_insert(0) += 1;
            }
            println!(
                "{tag:>14}: lean={lean:.3} 配额={quota:.2} 平均头数={mean:.2} 换岗={:.3}/回合 分布={dist:?}",
                churn as f64 / (hist.len() - 1) as f64
            );
        }
        println!("--- 单处货栈（需求 1）：第一艘运输舰要等几回合 ---");
        for (tag, mil, col) in [("军国 +1", 1.0, 0.0), ("中庸 0", 0.0, 0.0), ("和平 -1", -1.0, 0.0)] {
            let mut st = base.clone();
            set_ideology(&mut st, "中国", mil, col);
            grow_fleet(&mut st, "中国", 3);
            st.depots.clear();
            st.depot_add("中国", "金星", "碳", 100.0);
            let mut waited = 0;
            for _ in 0..200 {
                st.round += 1;
                assign_roles(&mut st, &config);
                waited += 1;
                if !roster(&st, "中国").is_empty() {
                    break;
                }
            }
            println!("{tag:>14}: 第 {waited} 回合出现第一条运输舰");
        }
        println!("--- 单处货栈清空之后：名额收回要几回合 ---");
        for (tag, mil, col) in [("军国 +1", 1.0, 0.0), ("中庸 0", 0.0, 0.0), ("和平 -1", -1.0, 0.0)] {
            let mut st = base.clone();
            set_ideology(&mut st, "中国", mil, col);
            grow_fleet(&mut st, "中国", 3);
            st.depots.clear();
            st.depot_add("中国", "金星", "碳", 100.0);
            for _ in 0..40 {
                st.round += 1;
                assign_roles(&mut st, &config);
            }
            let before = roster(&st, "中国").len();
            st.depots.clear();
            let mut waited = 0;
            for _ in 0..200 {
                st.round += 1;
                assign_roles(&mut st, &config);
                waited += 1;
                if roster(&st, "中国").is_empty() {
                    break;
                }
            }
            println!("{tag:>14}: 清空前 {before} 条 ⇒ 第 {waited} 回合清空");
        }
    }

}
