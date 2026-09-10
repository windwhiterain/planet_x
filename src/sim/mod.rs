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

pub mod capital;
pub mod cities;
pub mod construction;
pub mod events;
pub mod geometry;
pub mod governance;
pub mod haul;
pub mod ideology;
pub mod knowledge;
pub mod market;
pub mod metrics;
pub mod military;
pub mod mond;
pub mod power;
pub mod production;
pub mod relations;
pub mod ships;
pub mod story;

pub use capital::*;
pub use cities::*;
pub use construction::*;
pub use events::*;
pub use geometry::*;
pub use governance::*;
pub use haul::*;
pub use ideology::*;
pub use knowledge::*;
pub use market::*;
pub use metrics::*;
pub use military::*;
pub use mond::*;
pub use power::*;
pub use production::*;
pub use relations::*;
pub use ships::*;
pub use story::*;

/// **观测一个没有被推进过的世界**：一份 [`RoundView`]，其中的过程量全为 0 / 空（这一回合还没
/// 跑：产出/维护/治理/判定都还没发生），观测部分由 [`observe`] 从**真实 state** 汇总（各势力
/// 聚合、实力占比/霸权/战争等政治）。
///
/// 用途：回合 0、`--start` 载入的 checkpoint、以及任何「只看当前世界观测、不推进」的场合——
/// 使视图不是空壳，且与游戏逻辑同源。`RoundState::pre` 走的就是它。
pub fn view_from_state(state: &State, config: &GameConfig) -> RoundView {
    observe(state, config, &RoundSink::default())
}

/// Advance the world by one round, writing the new controllable state into
/// [`State::control`].
///
/// Returns the round's **`post` view** ([`RoundView`]) — the world as it stands **after** the round,
/// plus the round's **process quantities**: per-city/per-faction production, fleet upkeep,
/// governance cost/coverage, market freight/carrier income/net import, and the AI decision ledger.
/// Those are used by the step functions and do not land in the persisted state; they are captured
/// in an internal [`RoundSink`] while the steps run and folded into the view by [`observe`] at the
/// end of the round — so the reader's numbers are exactly the ones the simulation used (never
/// re-derived twice).
///
/// This is the `(state, rng) -> (state', rng', RoundView)` data-flow principle: `state` is mutated
/// in place, `rng` is consumed via `&mut`, and all derived data rides out in one view.
pub fn advance(state: &mut State, config: &GameConfig, rng: &mut Prng) -> RoundView {
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

    let mut sink = RoundSink::default();
    step_production(state, config, &mut sink);
    // 承包市场（挂单侧）：把「自己一个回合搬不动的积压」挂出去。放在 `step_production` 之后
    // （货栈是本回合刚更新过的），而在 `step_military` 的逐舰循环之前——挂单估运力用的是
    // `should_be_role`（纯函数），它与本回合稍后真正写进角色叶、并据此派单的那批舰
    // **同口径**，所以不存在「先挂单、再发现自己其实有闲船」的错位。
    step_contracts(state, config);
    step_upkeep(state, config, &mut sink);
    step_market(state, config, &mut sink);
    step_construction(state, config, rng, &mut sink);
    step_military(state, config, rng, &mut sink);
    // 光速治理：以距离首都为代价的管理/忠诚度，给超大帝国一个自然上限。
    step_governance(state, config, &mut sink);
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
        ev(
            state,
            GameEvent::WarStarted {
                a: a.clone(),
                b: b.clone(),
            },
        );
    }
    for (a, b) in wars_before.difference(&wars_after) {
        ev(
            state,
            GameEvent::WarEnded {
                a: a.clone(),
                b: b.clone(),
            },
        );
    }

    // 剧情：推进叙事弧/编年史（数据驱动，见 config/game.ron 的 `story` 表）。
    step_story(state, config);

    // 思潮（可变化意识形态）：按「变化因素」（战争得失/MOND 接触/经济好坏/人均面积）驱动。
    // 放在回合末：此时事件（战争得失/城夷平/叛乱）与流量（产出/维护/治理）均已就位。
    step_ideology(state, config, &sink);

    // MOND 知识（科技体系的干线）：**飞船在异常区的在场强度**驱动掌握度涨落
    // （用户裁决：第一版只做这一条渠道）。放在这里是因为它读「回合末的舰位」——
    // 与本回合的 `step_ideology`（同一个 MOND 接触口径）读的是同一份世界。
    step_knowledge(state, config);

    // 历史层收尾：两层各自按配置裁剪（这是唯一拿得到 config 的地方）。
    // `max_milestones` 默认 0 = 无损；`notable_window` 默认 24 回合，滑窗过期是**预期行为**。
    state.milestones.trim(config.history.max_milestones);
    state
        .notables
        .trim(state.round, config.history.notable_window);

    // 结回合：把 `sink` 里的过程量折进本回合的 `post` 视图。`observe` 复用
    // `balance_picture`/`sanctioned_hegemon`/`faction_power` 等 step 同源计算，因此视图里的
    // 观测与游戏逻辑**严格一致**；`faction_power` 是单一权威。
    observe(state, config, &sink)
}

#[cfg(test)]
#[path = "../tests/sim/mod.rs"]
mod tests;
