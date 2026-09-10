//! 事件日志与舰船死亡：`ev` / `kill_ship` / 清尸，以及由事件历史推导的军事信号。

use super::*;

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
pub fn ev(state: &mut State, e: GameEvent) {
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
pub fn kill_ship(state: &mut State, ship: &ShipId, cause: DeathCause, by: Option<Killer>) -> bool {
    if state
        .events
        .iter()
        .any(|e| matches!(e, GameEvent::ShipDestroyed { ship: s, .. } if s == ship))
    {
        return false;
    }
    let Some((owner, class)) = state
        .ship(ship)
        .map(|s| (s.faction_id.clone(), s.class.clone()))
    else {
        return false;
    };
    if let Some(s) = state.ship_mut(ship) {
        s.hull = 0.0;
    }
    ev(
        state,
        GameEvent::ShipDestroyed {
            ship: ship.clone(),
            owner,
            class,
            cause,
            by,
        },
    );
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
pub fn sweep_dead_ships(state: &mut State, watched: &BTreeSet<ShipId>) -> usize {
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
pub fn military_deltas(events: &[GameEvent]) -> BTreeMap<FactionId, f64> {
    let mut delta: BTreeMap<FactionId, f64> = BTreeMap::new();
    let mut bump = |fid: &FactionId, d: f64| {
        if !fid.is_empty() {
            *delta.entry(fid.clone()).or_insert(0.0) += d;
        }
    };
    for e in events {
        match e {
            GameEvent::ShipDestroyed {
                owner, cause, by, ..
            } => {
                bump(owner, -1.0);
                if *cause == DeathCause::Combat {
                    if let Some(k) = by {
                        bump(&k.faction, 1.0);
                    }
                }
            }
            GameEvent::CityRazed {
                owner, fallen_to, ..
            } => {
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
