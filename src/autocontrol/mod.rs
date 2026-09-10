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
//! * [`tactics`]      战术选目标（克制/距离/行为风格）+ 单舰 AI 回合。
//! * [`economy`]      经济→成本预览（`--control-plan`）。
//!
//! 各子模块之间尽量不互相依赖；唯一的例外是 [`economy`] 阅读 [`budget`] 的预算并把
//! 引擎原语向 [`crate::sim`] 借。对外的接口统一由本文件 `pub use` 重新导出，保持
//! `autocontrol::<name>` 的调用方式不变。

pub mod budget;
pub mod economy;
pub mod freight;
pub mod shipbuilding;
pub mod tactics;

pub(crate) use budget::{read_budget, write_budget, BudgetKind};
pub use economy::{control_plan, control_plan_all};
pub(crate) use shipbuilding::{choose_loadout, choose_next_class, resolve_loadout, retool_shipyards};
pub(crate) use tactics::{ai_ship_turn, auto_combat, kiting_dest};

/// Round a float to 2 decimals (token-noise reduction); `+ 0.0` normalizes IEEE `-0.0`.
pub(crate) fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0 + 0.0
}

/// Fixed seed for the [`control_plan`] dry-run ("PLAN"). Production / upkeep / governance are
/// RNG-independent, so this just keeps the preview deterministic across runs.
pub(crate) const PLAN_SEED: u64 = 0x50514f4e;
