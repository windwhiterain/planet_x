//! 本回合 **AI 做的判定**（“AI 掷了什么”）——那些**不落持久状态、也不发事件**的中间量。
//!
//! # 为什么需要单独捕获
//!
//! 舰的指令叶（[`ControllableState::ship_orders`](crate::model::ControllableState)）只告诉你
//! 「它现在在干什么」（`Follow` 某艘舰），**不告诉你 AI 为什么这么选**：
//!
//! * 是**接战索敌**的结果，还是**自保撤退**？
//! * 撤退判定的两个输入当时是多少（血量比 vs 撤退阈值）？
//! * 那艘舰这一回合**根本没被派活**（`Hold`），还是一直在执行一条旧指令？
//! * 某个船坞什么时候、为什么开始改产战列舰？
//!
//! 这些判定全部发生在 [`crate::autocontrol`] 内部，用完就丢；事件里也没有它们
//! （`Withdraw`/`ShipDestroyed` 只记结果，不记判定过程）。于是「我的舰怎么跑那么远送死」
//! 这类最常被问的问题，在旧读面里**无处可查**。这个模块把它们记下来。
//!
//! # 记录是纯追加 ⇒ 行为中性
//!
//! 捕获只做 `Vec::push`：不读 RNG、不改状态、不影响任何分支。判据（见
//! `.agents/notes/engine-data-plane.md` §5）：同 seed 同回合的 `--digest` 输出与捕获前**逐字相同**，
//! 长局 harness 全绿。
//!
//! # 读它
//!
//! * 单回合：`planet_x --start ckpt.ron --derived` → `post.flow.decisions`（原始嵌套结构）。
//! * 整段轨迹：`planet_x --index out/` → `out/idx/decisions.jsonl` 一行一条判定，
//!   按 `round` / `faction_id` / `actor` join（Python: `planet_xq.load('out').decisions()`）。

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BuildingId, CityId, FactionId, ShipBehavior, ShipId};

/// 一艘 AI 舰本回合的判定结果。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShipVerdict {
    /// **自保撤退**：受创（血量比 < 撤退阈值）+ 敌在射程内 + 离首都有一定距离 ⇒ 后撤回首都。
    Withdraw,
    /// **接战**：射程内有敌舰且火力可分派 ⇒ 写 `Follow{目标}` 并开火。
    Engage,
    /// **殖民/复垦**：已抵达某个定居点天体。
    Colonize,
    /// **就地轰炸**：敌对城进入围城射程（轰炸不需要行为）。
    Bombard,
    /// **常规机动**：驶向行为目的地（含风筝姿态的软目标调整）。
    Move,
    /// **运输**：没有仗可打，去跑一条集货路线（`Haul{from,to}`）——AI 按积压自动选线；
    /// 也可能是「保持上回合那条还在跑的路线」。装/卸/在途都记成这一个判定，
    /// 细节看 `order` 与事件流里的 `cargo_loaded`/`cargo_delivered`。
    Haul,
    /// **没派活**：`resolve_target` 没有任何目标可给——不是「待命指令」，而是这一回合 AI 没说话
    /// （叶上那条值可能是很久以前的）。
    Hold,
}

/// 一艘 AI 舰本回合的判定（投影表 `decisions` 的 `kind = "ship_order"` 行）。
///
/// 字段分两类，别混：
/// * **判定**：`verdict` / `target` / `destination` / `order`（AI 选了什么）；
/// * **输入**：`hull_ratio` / `retreat_hull` / `kiting` / `enemy_in_range`（判定那一刻观察到的事实）
///   ——有了它们，「为什么」才是可复核的，而不是一句自述。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ShipDecision {
    pub ship: ShipId,
    pub faction: FactionId,
    pub verdict: ShipVerdict,
    /// 判定的对象（按 `verdict` 解释）：舰名（接战/撤退）／城名（轰炸）／天体名（殖民）。
    pub target: Option<String>,
    /// 位置型目标（`Move`）：驶向的坐标（可能是风筝姿态算出来的软目标）。
    pub destination: Option<[f64; 2]>,
    /// 判定时的血量比 `hull / hull_max`（撤退判定的输入）。
    pub hull_ratio: f64,
    /// 判定时的**有效**撤退阈值（随风格 `kiting` 变：风筝更早撤、贴脸打得更久）。
    pub retreat_hull: f64,
    /// 判定时的**有效**风筝距离（叶 → 舰队默认 → 记录值）。
    pub kiting: f64,
    /// 判定时射程内是否有敌舰。
    pub enemy_in_range: bool,
    /// AI 实际写回指令叶的行为（`None` = 这一回合没写叶：就地轰炸、或没派活）。
    pub order: Option<ShipBehavior>,
    /// 这次判定发生在**移动之后**吗？
    ///
    /// 一艘舰一回合最多留**两条**判定：先「机动到软目标」（`verdict: move`），移动到位后可能
    /// 再判一次（`engage`/`bombard`/`colonize`）。没有这一列，读表的人会把同一艘舰的两行当成
    /// 「它同时做了两件事」而算错；有了它，"先去了哪、到了之后又改了什么主意"才是可读的。
    #[serde(default)]
    pub after_move: bool,
}

/// 船坞改装（`kind = "retool"`）：AI 把某个船坞从「造 A」改成「造 B」。
///
/// 触发条件是「舰队被单一舰型统治 > 60% 且正在交战」，新舰型由 `choose_next_class`
/// 用 seeded RNG 选出——这是**少数几个不留事件的 AI 决策之一**（事后只能从
/// `ship_type` 的变化反推，且看不出是什么时候改的）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct RetoolDecision {
    pub faction: FactionId,
    /// 船坞所在城。
    pub city: CityId,
    /// 船坞在该城内的建筑下标（`BuildingId` 只在城内唯一——与 `invest_weights` 同一个坑）。
    pub building: BuildingId,
    /// 改装前生产的舰级。
    pub from: String,
    /// 改装后生产的舰级。
    pub to: String,
}

/// 一条**风格重估**（`kind = "style_retune"`）：自动控制把某艘舰的某条风格轴从 `from` 改成 `to`。
///
/// 这是「风格轴上的 `Auto`」这个承诺的**流水**（`autocontrol::style`）：风格是慢变量，
/// 它每回合被**概率**触发、走**分布**步长（指数松弛）——所以"它什么时候改、朝哪儿改、
/// 为什么"必须能回答，否则读面只看到一个数变了。
///
/// `from`/`to`/`target` 都已按 2 位小数落盘（与写进叶里的值同一个数）；`drivers` 是这次
/// 重估**当时读到的战况输入**，键随轴不同：
/// * `temper`：`war` / `win` / `damage` / `withdraw`；
/// * `lone_wolf`：`neighbors`（护航半径内的友舰数）；
/// * `kiting`：`power`（敌我火力比）/ `hardness`（硬度对比）/ `hurt`（挨打程度）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct StyleDecision {
    pub faction: FactionId,
    pub ship: ShipId,
    /// 哪条轴：`temper` / `lone_wolf` / `kiting`（前两条同处一片叶 `ship_doctrine`）。
    pub axis: String,
    pub from: f64,
    pub to: f64,
    /// 这次重估朝它走的目标值（`from` → `to` 是这段距离的一小步）。
    pub target: f64,
    /// 驱动这次重估的可读输入（见上）。
    pub drivers: std::collections::BTreeMap<String, f64>,
}

/// 一条**设计图**决策（`kind = "blueprint"`）：AI 给自己的建造区建图 / 重估 / 复用 / 回收。
///
/// 设计图是「还不存在的舰的规则」的家（[`crate::model::Blueprint`]），而 `Auto` 图这一层的
/// 执行者就是这里（`autocontrol::blueprints`）：按资源优势与战况生成设计，**按
/// `(舰级, 选装签名)` 归并复用**（长局里图库不该爆炸），没人指向的自建图回收掉。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct BlueprintDecision {
    pub faction: FactionId,
    pub blueprint: String,
    /// `created`（新建一张）/ `retuned`（重估已有那张的选装）/ `reused`（复用了同签名的另一张）
    /// / `reaped`（回收没人指向的自建图）。
    pub action: String,
    pub class: String,
    /// 设计主题（`强袭`/`堡垒`/…）：这个势力此刻认为"这一型舰该干什么"。
    pub theme: String,
    /// 落到图上的选装（`reaped` 时是它被回收前的选装）。
    pub components: Vec<String>,
    /// 这次决策发生在哪个建造区（`reaped` 没有建造区 ⇒ `None`）。
    pub city: Option<CityId>,
    pub building: Option<BuildingId>,
}

/// 本回合 AI 的判定集合（挂在 [`RoundSink`](super::RoundSink) 上随回合一起带出）。
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct RoundDecisions {
    /// 每艘 AI 舰本回合的判定（**玩家名下的舰不在里面**：它们的指令是你下的，
    /// 但注意玩家舰的自动接战/轰炸也走 `auto_combat`，那一层目前**没有**被记录——见笔记）。
    pub ships: Vec<ShipDecision>,
    /// 船坞改装决策。
    pub retools: Vec<RetoolDecision>,
    /// **风格重估**（`autocontrol::style`：`Auto` 风格叶的执行者）——每改一条轴一行。
    #[serde(default)]
    pub styles: Vec<StyleDecision>,
    /// **设计图**决策（`autocontrol::blueprints`：`Auto` 图的执行者）——建图/重估/复用/回收。
    #[serde(default)]
    pub blueprints: Vec<BlueprintDecision>,
}
