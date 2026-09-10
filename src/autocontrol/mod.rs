//! 自动控制（Auto 决策）——「AI 大脑」。
//!
//! 这个 module 只装「谁来控制那些可控量、AI 到底怎么决策」的逻辑。它刻意允许复杂：
//! 战术选目标、舰种选择、组件选装、预算重算、经济预览、海军重构、舰队撤退… 这些是
//! 系统在 `mode = Auto` 时自己做的决定。**它不碰纯引擎机制**（开采/维护/市场/建造/战斗结算/
//! 治理/外交/剧情都是 [`crate::sim`] 的活）——sim 只把「引擎原语」借给这里，这里再
//! 把「决定 + 对可控状态的改写」回交给 sim 去执行。
//!
//! 边界划得很干净，整个项目其余部分保持精简：
//! * [`crate::sim`] = 模拟引擎（回合步进、物理/经济/军事结算、指标）。
//! * 本 module = 自动控制（谁在控制、AI 怎么想）。
//!
//! 所有函数要么 `pub(crate)`（被引擎调用）、要么 `pub`（被 agent CLI/`world` 调用），
//! 其余辅助一律私有，绝不外泄到项目其它角落。
//!
//! 按**关注点**拆成若干子模块，每个只装一类「决定」：
//! * [`budget`]       预算重算（Ai 每回合从库存重算，Player 只读命令）。
//! * [`shipbuilding`] 舰种 / 组件选装 + 海军随威胁重构。
//! * [`blueprints`]   **设计图**：AI 自己建图 / 重估 / 去重复用 / 回收（`Auto` 图的执行者）。
//! * [`style`]        **风格三轴**（temper / lone_wolf / kiting）的逐舰重估（`Auto` 风格叶的执行者）。
//! * [`tactics`]      战术选目标（克制/距离/行为风格）+ 单舰 AI 回合。
//! * [`freight`]      集货：自有舰队怎么跑（定编 + 按积压占比抽签派单）+ **承包的挂单侧**。
//! * [`knowledge`]    **观测**：第三条角色轴的第三态——谁去异常区蹲着（定编 + 按期望在场收益
//!                    抽签选靶）。它是 MOND 掌握度**唯一**的知识来源（`sim::step_knowledge`），
//!                    所以角色的优先级里它排第一（观测 > 运输 > 战斗）。
//! * [`contract`]     **承包的承运方**：门槛 / 影子价格 λ / σ 软化 / 按信誉加权抽签撮合。
//! * [`economy`]      经济→成本预览（`--control-plan`）。
//!
//! 各子模块之间尽量不互相依赖；两个例外：[`economy`] 阅读 [`budget`] 的预算，
//! 而 [`contract`] 借 [`freight`] 的**运力助手**（`trip_throughput` / `route_for` /
//! `should_be_role`）来挑船、排线、钉角色——承包与自有集货用的是同一套「谁在运货、
//! 运力怎么算」的定义，各写一份必然漂移，所以这里**故意复用**。
//! 对外的接口统一由本文件 `pub use` 重新导出，保持 `autocontrol::<name>` 的调用方式不变。

pub mod blueprints;
pub mod budget;
pub mod contract;
pub mod economy;
pub mod freight;
pub mod knowledge;
pub mod shipbuilding;
pub mod style;
pub mod tactics;

pub(crate) use blueprints::design_fleets;
pub(crate) use budget::{BudgetKind, read_budget, write_budget};
pub use economy::{control_plan, control_plan_all};
pub use shipbuilding::minimum_loadout;
pub(crate) use shipbuilding::{
    choose_loadout, choose_next_class, resolve_loadout, retool_haulers, retool_shipyards,
};
pub(crate) use style::regulate_styles;
pub(crate) use tactics::{ai_ship_turn, auto_combat, kiting_dest};

/// Round a float to 2 decimals (token-noise reduction); `+ 0.0` normalizes IEEE `-0.0`.
pub(crate) fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0 + 0.0
}

/// Fixed seed for the [`control_plan`] dry-run ("PLAN"). Production / upkeep / governance are
/// RNG-independent, so this just keeps the preview deterministic across runs.
pub(crate) const PLAN_SEED: u64 = 0x50514f4e;
