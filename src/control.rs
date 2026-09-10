//! 命令面 (command surface) —— agent 写 `--apply` diff / 读控制面的领域逻辑。
//!
//! 这是与 HTTP/UI **无关** 的引擎能力：CLI（`--control` / `--apply` /
//! `--control-schema`）、`planet_x_web` crate 的 `POST /api/command`、以及
//! `sim`/`autocontrol` 的测试都消费这里暴露的补丁解析/应用与读面构建。
//! 它只依赖 [`crate::model`]，不依赖任何 HTTP/web 栈，因此引擎本体可以
//! 完全不带 axum/tokio/tower-http。
//!
//! 两类出口：
//! * **读面** —— 当前可控状态渲染成可编辑模板（`[`control_surface`]`）或
//!   web 的 `StateView.control` 片段（`[`control_view`]`/`[`scope_view`]`）。
//! * **写面** —— 一个 presence-aware 的多级 diff（`[`CommandReq`]`）被
//!   `[`apply_diff`]`/`[`apply_patch`]` 叠加到 [`State`] 上：只改「出现在 diff 里」
//!   的势力/叶子，缺省的 `value`/`behavior` 保留现值、缺省的 `mode` 保留现模式。

use crate::model::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// `remove` 的序列化开关：**只在真的要删叶时才出现在线格式里**。
///
/// 为什么需要它：读面（`--control` / web 的 `control` 段）复用 `DefaultShipOrder` /
/// `DefaultDoctrine` / `DefaultKiting` 这几个结构体来**回显**叶片，而 `remove` 是**写面**
/// 的东西（读面表达"没有这片叶"的方式是 `null`）。不跳过的话读面里会多出一堆
/// `"remove": false`，而 kit 的 `verify` 是按字段比对读面的——那一列会立刻变成
/// 每次都出现的"假变动"。
fn is_false(b: &bool) -> bool {
    !*b
}

/// `null` 与「字段缺席」必须分得开（presence-aware 写面的经典需求）。
///
/// `Option<Option<T>>` 的 serde 默认实现会把 `null` 与「缺席」**都**落成外层的 `None`，
/// 于是「不改这个字段」与「把它清空」在写面上就没法区分了（`BuildingPatch.blueprint`
/// 与 `BlueprintPatch.order` 都需要这个区分）。这个 `deserialize_with` 把**出现过的**值
/// 一律包成 `Some(..)`：`"x"` ⇒ `Some(Some(x))`、`null` ⇒ `Some(None)`；缺席时才走
/// `#[serde(default)]` ⇒ 外层 `None`。
fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

// --- read-side wire types (the editable control surface) --------------------

/// 一艘舰的读面条目：行为 + 由谁决定（三态，读面永远给全三态之一）。
#[derive(Serialize, Deserialize, Clone)]
pub struct ShipOrderEntry {
    pub ship: ShipId,
    pub behavior: ShipBehavior,
    pub mode: ControlMode,
}

/// 一艘舰的行为风格（per-舰 可配置）读面：两条轴各取 [-1,1]，0 = 基线。
/// 〈风筝<->贴脸〉不在行为风格里，是普通舰船控制属性 [`ShipKitingEntry`]。
///
/// `temper`/`lone_wolf` 给的是**有效值**（叶 → 舰队默认 → 舰上记录值），`mode` 给的是
/// **叶片自己的表态**（没有叶片 = `Inherit`）。所以「模板原样回传」安全：没被改过的行
/// 写回去仍然没有意见。**改值请把 `mode` 改成 `Player`/`Auto`**——只改值而留着 `Inherit`
/// 等于说"这一层没有意见"，除非舰队默认也是 `Player`，否则那个值不会被采用。
#[derive(Serialize, Deserialize, Clone)]
pub struct ShipDoctrineEntry {
    pub ship: ShipId,
    pub temper: f64,
    pub lone_wolf: f64,
    pub mode: ControlMode,
}

/// 一艘舰的风筝<->贴脸姿态（普通舰船控制属性，per-舰）：[-1,1]，0 = 基线。它是软属性——
/// Move/Follow/Dock/Idle 皆为软目标，附近有敌舰时自动按它软移动（玩家也不能硬控制）。
/// 值与 `mode` 的语义见 [`ShipDoctrineEntry`]。
#[derive(Serialize, Deserialize, Clone)]
pub struct ShipKitingEntry {
    pub ship: ShipId,
    pub kiting: f64,
    pub mode: ControlMode,
}

/// 一艘舰的**角色**（普通舰船控制属性，第三条风格轴）：`true` = 运输舰（自动控制给它排集货
/// 路线），`false` = 战舰（找仗打）。值与 `mode` 的语义见 [`ShipDoctrineEntry`]。
///
/// ⚠ 与另两条轴唯一的差别：**自动控制会写这片叶**（每回合按积压定编，见
/// `autocontrol::freight`）。所以「有效值不是玩家写的那份」是正常的——`mode = Player`
/// 才是「玩家钉的、AI 不碰」。它**只管自动控制派哪种活**，不解除武装（照样自动开火/kiting）。
#[derive(Serialize, Deserialize, Clone)]
pub struct ShipFreighterEntry {
    pub ship: ShipId,
    pub freighter: bool,
    pub mode: ControlMode,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BudgetEntry {
    pub resource: String,
    pub value: f64,
    pub mode: ControlMode,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct InvestWeightEntry {
    pub city: CityId,
    pub building: BuildingId,
    pub kind: String,
    pub resource: Option<String>,
    pub ship_type: Option<String>,
    pub structure: String,
    pub value: f64,
    pub mode: ControlMode,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BuildWeightEntry {
    pub city: CityId,
    pub building: BuildingId,
    pub ship_type: Option<String>,
    pub value: f64,
    pub mode: ControlMode,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LoyaltyBudgetEntry {
    pub city: CityId,
    pub value: f64,
    pub mode: ControlMode,
}

/// 一张设计图的**读面条目**（读面即写面）。
///
/// * `mode` = **图叶自己的表态**（三态：`Inherit` 这一层没有说话 / `Auto` 系统可重估 /
///   `Player` 系统不许动）。有效归属看 `--control` 之外的地方（[`State::blueprint_control`]，
///   投影 `blueprints.effective_mode`）。
/// * `components` **必须全量输出**（不是 `null`）：读面即写面要能「dump → 改 → 回传」，
///   按 presence-aware 规则，`components` 缺席 = 不动、`[]` = 清空（交给生成器）。少输出
///   就等于回传时清空选装。
/// * `order` = 本图给**新舰**的默认意图（`null` = 本图对意图没有说话——Q1(c) 的
///   「意图轴默认 `Inherit`/沉默」）。
/// * `ship_count` / `launch_waiting` = **读面附加的派生量**（引擎算，不落状态）。它们只读：
///   写面收下这两个键但**不写它们**（[`BlueprintPatch::ship_count`] /
///   [`BlueprintPatch::launch_waiting`]）。
#[derive(Serialize, Deserialize, Clone)]
pub struct BlueprintEntry {
    pub name: BlueprintId,
    pub class: String,
    pub components: Vec<String>,
    pub order: Option<ShipBehavior>,
    pub mode: ControlMode,
    /// 本图造了多少艘（`state.ships` 里 `blueprint == name` 的条数，现算、不落状态）。
    pub ship_count: usize,
    /// **本图此刻是不是「买不起 ⇒ 没下水」**（用户裁决 Q4(b) 的可见标记，现算、不落状态）：
    /// 某个挂着这张图的城里，该舰级的进度已经攒够 `build_points` 却**没有下水**——这张图
    /// （`Player` 归属、带选装）的组件此刻买不起。判据与投影 `blueprints.launch_waiting` 列
    /// **同一个函数**（[`crate::sim::blueprint_launch_waiting`]），免得两个读面各说各话。
    pub launch_waiting: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FactionControlView {
    pub faction_id: FactionId,
    pub capital: Option<Control<BodyId>>,
    /// 舰队默认指令（势力级）：新舰出生就继承它，一次性指令执行完也回落到它。
    pub default_ship_order: Option<DefaultShipOrder>,
    /// 舰队默认**行为风格**（势力级）：叶 Inherit 的舰取它的值。
    pub default_doctrine: Option<DefaultDoctrine>,
    /// 舰队默认**风筝<->贴脸姿态**（势力级）：与 `default_doctrine` 同形的另一片。
    pub default_kiting: Option<DefaultKiting>,
    /// 舰队默认**角色**（势力级，第三条风格轴）。
    pub default_freighter: Option<DefaultFreighter>,
    /// **势力级设计图库**：一行 = 一张图（厂房里「还不存在的舰」的出厂规格）。
    /// 建造区指向其中一张（`buildings[].blueprint` → 结构叶 [`BuildingPatch::blueprint`]）。
    pub blueprints: Vec<BlueprintEntry>,
    pub ship_orders: Vec<ShipOrderEntry>,
    pub ship_doctrine: Vec<ShipDoctrineEntry>,
    pub ship_kiting: Vec<ShipKitingEntry>,
    /// 本势力各舰的**角色**（有效值 + 那片叶自己的表态）。
    pub ship_freighter: Vec<ShipFreighterEntry>,
    pub investment_budget: Vec<BudgetEntry>,
    pub construction_budget: Vec<BudgetEntry>,
    pub invest_weights: Vec<InvestWeightEntry>,
    pub build_weights: Vec<BuildWeightEntry>,
    pub loyalty_budget: Vec<LoyaltyBudgetEntry>,
}

/// The editable control surface, exactly what the frontend edits and posts back
/// to `/api/command`. Emitted by the agent CLI `control` command as the
/// "template" the agent edits, and accepted by `apply` / `--apply` as a diff.
#[derive(Serialize, Clone)]
pub struct ControlSurface {
    pub control: Vec<FactionControlView>,
    pub scope: ControlScopePatch,
}

// --- presence-aware control patches (the "diff" the agent writes) ----------

/// 舰队默认指令（势力级）：**读面即写面**，与其它叶片同形——`behavior` = 默认干什么，
/// `mode` = 谁负责。
///
/// 它是「新舰默认归谁、干什么」的正解，也是「一次性指令执行完回落到哪」的答案：
/// 叶子上没有说话（`Inherit`）或压根没有叶子（**刚下水的新舰**）的舰，都取这里的值。
/// 单舰特例仍写在 `ship_orders[]`（更具体的层优先）。
#[derive(Serialize, Deserialize, Default, Clone, JsonSchema)]
pub struct DefaultShipOrder {
    /// 默认行为（缺省 = 保留现值；写值即接管，见 [`apply_diff`]）。
    #[serde(default)]
    pub behavior: Option<ShipBehavior>,
    /// 由谁决定：Inherit（这一层没有说话）/ Auto（系统自动）/ Player（玩家）。
    /// 缺省 = 保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（这一层回到"没有说话"）。与 `behavior`/`mode` 同时出现 ⇒ 拒绝（见 [`apply_diff`] 的「删叶」）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 舰队默认**行为风格**（势力级，两片之一）：**读面即写面**，与 `default_ship_order` 同形。
///
/// "全舰队风筝、战列舰贴脸"这类意图 = 一片默认叶 + 几片特例叶，不必逐舰点名；新下水的舰
/// 也自动跟随（它没有自己的叶）。两条轴各取 [-1,1]，0 = 基线。
///
/// ⚠ **两轴一片叶**：这片叶**还不存在**时，必须**两条轴一起给**——只给一条的话另一条会
/// 静默变成 `0.0`（= 基线），而 `0.0` 是个正常取值，事后从读面完全看不出来（见
/// [`apply_diff`] 的 `partial_doctrine_leaf`：那种补丁会被**拒绝**）。
#[derive(Serialize, Deserialize, Default, Clone, JsonSchema)]
pub struct DefaultDoctrine {
    /// 默认理智<->热血（缺省 = 保留现值；写值即接管，见 [`apply_diff`]）。
    #[serde(default)]
    pub temper: Option<f64>,
    /// 默认护航<->独狼（缺省 = 保留现值）。
    #[serde(default)]
    pub lone_wolf: Option<f64>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（这一层回到"没有说话"。注意：与它自己的两条轴同时出现 ⇒ 拒绝）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 舰队默认**风筝<->贴脸姿态**（势力级，两片之二），与 [`DefaultDoctrine`] 同形。
#[derive(Serialize, Deserialize, Default, Clone, JsonSchema)]
pub struct DefaultKiting {
    /// 默认姿态（缺省 = 保留现值；写值即接管）。
    #[serde(default)]
    pub kiting: Option<f64>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（这一层回到"没有说话"）。与 `kiting`/`mode` 同时出现 ⇒ 拒绝。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 舰队默认**角色**（势力级，第三条风格轴），与 [`DefaultKiting`] 同形。
///
/// 写它 = 「全舰队按这个角色走」（`true` = 全转运输）。玩家把它设成 `Player` 后，
/// 自动控制的**逐舰定编不再生效**（那片叶归玩家）——这正是「AI 定编 vs 玩家意图」的闸门。
#[derive(Serialize, Deserialize, Default, Clone, JsonSchema)]
pub struct DefaultFreighter {
    /// 默认角色（缺省 = 保留现值；写值即接管）。
    #[serde(default)]
    pub freighter: Option<bool>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（势力级这一层回到"没有说话"）。与 `freighter`/`mode` 同时出现 ⇒ 拒绝；
    /// 叶不存在时是幂等成功。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 一艘舰的指令补丁：`behavior` 用它替换该舰行为；`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipOrderPatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    /// 新行为（Idle/Move/Follow/DockCity/Dock/Colonize）。缺省 = 保留现值。
    /// **写了值却没写 mode = 接管**（该叶变成玩家指令），免得「我明明写了指令却没生效」。
    #[serde(default)]
    pub behavior: Option<ShipBehavior>,
    /// 由谁决定：Inherit（继承，撤销本层的表态）/ Auto（系统自动）/ Player（玩家）。
    /// 缺省 = 保留现值。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（这艘舰回到"没有自己的指令" ⇒ 取舰队默认）。与 `behavior`/`mode` 同时出现 ⇒ 拒绝。
    /// 舰已战沉也能删（删的是**控制面**里的叶，不要求实体还在 ⇒ 顺带是清理陈叶的路）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 一艘舰的行为风格补丁（per-舰 可配置）：覆盖 `ship` 的某条轴；缺省轴保留现值。
/// 每条轴会被钳制到 [-1,1]（技能就是在这个区间里取值的）。
///
/// 写的是**叶片**（`ControllableState::ship_doctrine`），不是舰上那个记录值：
/// **写了值却没写 `mode` = 接管**（这片叶变成玩家风格），免得"我明明写了风格却没生效"。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipDoctrinePatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    #[serde(default)]
    pub temper: Option<f64>,
    #[serde(default)]
    pub lone_wolf: Option<f64>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 写了值就接管，没写值就保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（这艘舰回到"没有自己的风格" ⇒ 有效风格回落到舰队默认 / **出厂快照**）。
    /// 叶不存在时是**幂等成功**（目标状态就是"没有这片叶"）。与值/`mode` 同时出现 ⇒ 拒绝。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 一艘舰的风筝<->贴脸姿态补丁（普通舰船控制属性，per-舰）：覆盖 `ship` 的姿态叶；
/// 被钳制到 [-1,1]。缺省 = 保留现值。语义同 [`ShipDoctrinePatch`]（叶片 + 写值即接管）。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipKitingPatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    #[serde(default)]
    pub kiting: Option<f64>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 写了值就接管。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（回落到舰队默认 / 出厂快照）。叶不存在时是幂等成功；与值/`mode` 同时出现 ⇒ 拒绝。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 一艘舰的**角色**补丁（per-舰，第三条风格轴）：覆盖 `ship` 的角色叶。
/// 缺省 = 保留现值。语义同 [`ShipKitingPatch`]（叶片 + 写值即接管）。
///
/// 玩家写它 = 手动给这艘舰定活（`true` 运货 / `false` 打仗），自动控制的定编从此不碰这艘舰。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipFreighterPatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    #[serde(default)]
    pub freighter: Option<bool>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 写了值就接管。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**：这艘舰回到"没有自己的角色" ⇒ **交回自动定编**（`Inherit` 之下 AI 下回合
    /// 可能立刻又写下它的结论——想让结论稳定就得写 `Player` 而不是删叶）。叶不存在时是幂等成功；
    /// 与 `freighter`/`mode` 同时出现 ⇒ 拒绝。舰已战沉也能删（删的是控制面里的叶）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 资源预算补丁（投资/建造共用）：`value` 替换预算额，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct BudgetPatch {
    /// 资源 raw-key（见 --meta 的 resources：raw-key→中文名）。
    pub resource: String,
    /// 新的预算额（资源投放量）。缺省 = 保留现值。
    #[serde(default)]
    pub value: Option<f64>,
    /// 由谁决定：Inherit（继承）/ Auto（系统自动）/ Player（玩家）。缺省 = 保留现值。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（该资源回到"这一层没有说话"）。与值/`mode` 同时出现 ⇒ 拒绝。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 某城某「建设投资权重」补丁：`value` 替换权重，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct InvestWeightPatch {
    pub city: CityId,
    pub building: BuildingId,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**。与值/`mode` 同时出现 ⇒ 拒绝；建筑已经没了也能删（清理陈叶）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 某城某建造区「建造投资权重」补丁：`value` 替换权重，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct BuildWeightPatch {
    pub city: CityId,
    pub building: BuildingId,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**。与值/`mode` 同时出现 ⇒ 拒绝；建筑已经没了也能删（清理陈叶）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 某城「娱乐/福利预算」补丁：`value` 替换预算额，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct LoyaltyBudgetPatch {
    pub city: CityId,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**。与值/`mode` 同时出现 ⇒ 拒绝；城已易主/被夷平也能删（清理陈叶）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 一张**设计图**的补丁（势力级设计图库的一项）：**读面即写面**。
///
/// * **写值即接管**：只写 `class` / `components` / `order` 而没写 `mode` ⇒ 这片图叶变
///   `Player`（并记 `NOTE_APPLY_TOOKOVER`）——「我明明写了图却没生效」是不可能的；
/// * 只写 `mode` 合法（值不动）：`{"name":"重甲巡洋","mode":"Auto"}` = 交回系统重估；
/// * **图名不存在时不许凭空造图**（只写 `mode` ⇒ 报 `no_such_blueprint`，同
///   `no_such_faction` 防幽灵势力的理由）；
/// * `remove: true` ⇒ **删掉整张图**（与「让意图轴沉默」是两件事，见下）。
///
/// `order` 是**三层含义**的双 Option（见 [`double_option`]）：
/// * **缺席** = 不动这一层；
/// * `null` = **本图对意图没有说话**（意图轴回到沉默 ⇒ 链继续往下降到舰队默认）。
///   这与「删掉这张图」（`remove: true`）后果完全不同：删图会让挂它的建造区变成
///   **悬空指针 ⇒ 停产**（Q10(a)），而清空 `order` 只是收回这一层的表态；
/// * 给值 = 表态（`{"type":"dock","body":"地球"}` 这种 tagged 写法 `--apply` 同样接受）。
#[derive(Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlueprintPatch {
    /// 图名（势力内的唯一 key）。改名 = 删旧建新（指向旧名的建造区会变成悬空指针）。
    pub name: BlueprintId,
    /// 舰级（`config.ships` 的 key）。新建图**必须**给；口径 A 下它必须与该建造区的
    /// `ship_type` 相等（不等报 `blueprint_class_mismatch`）。
    #[serde(default)]
    pub class: Option<String>,
    /// 选装表（组件 id，顺序 = 槽位顺序）。`[]` = 交给生成器（`choose_loadout`）。
    /// 校验：组件必须存在（`no_such_component`）、不许重复（`duplicate_component`）、
    /// 数量不许超过该舰级的槽位（`too_many_components`）。
    #[serde(default)]
    pub components: Option<Vec<String>>,
    /// 本图给**新舰**的默认意图。`null` = 本图对意图没有说话（三层含义见上）。
    #[serde(default, deserialize_with = "double_option")]
    pub order: Option<Option<ShipBehavior>>,
    /// 三态归属：Inherit / Auto / Player。缺省 = 写了值就接管、没写值就保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉整张图**（挂它的建造区随后是悬空指针 ⇒ 停产，见 Q10(a)）。
    /// 图不存在时是**幂等成功**；与值/`mode` 同时出现 ⇒ 拒绝。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
    /// **只读回显**：本图造了多少艘（读面给的派生量）。写面**收下但不写**它——它不落状态，
    /// 由引擎现算。收下是为了「读面即写面、模板原样回传安全」（否则整面回传会被
    /// `deny_unknown_fields` 判成非法）。
    #[serde(default)]
    pub ship_count: Option<usize>,
    /// **只读回显**：本图此刻是不是在等钱（Q4(b) 的可见标记，读面给的派生量）。同
    /// [`Self::ship_count`]：收下但不写。
    #[serde(default)]
    pub launch_waiting: Option<bool>,
}

/// 挂了这张图的建造区（城名、建筑下标、该区当前的 `ship_type`）。
fn referencing_yards(state: &State, fid: &str, bp: &BlueprintId) -> Vec<(CityId, BuildingId, String)> {
    state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        .flat_map(|c| {
            c.buildings
                .iter()
                .filter(|b| b.blueprint.as_deref() == Some(bp.as_str()))
                .map(|b| (c.name.clone(), b.id, b.ship_type.clone().unwrap_or_default()))
        })
        .collect()
}

/// 本份 diff 打算把**哪些建造区**改成**哪个舰级**（键 = `(城, 建筑下标)`）。
///
/// 为什么需要它：口径 A 要求「图的 `class` == 建造区的 `ship_type`」，而这个约束的校验
/// 发生在两处（改图 / 改区）。若两处各自只看**当前**状态，「两处一起写」（这是 spec §4.5
/// 教的正解）就会被先落地的那一半拒掉——正确的判据是**这份 diff 之后的意图**。
fn yard_ship_type_intent(fac: &FactionControlPatch) -> BTreeMap<(CityId, BuildingId), String> {
    let mut m = BTreeMap::new();
    for b in &fac.buildings {
        if let (Some(city), Some(bid), Some(st)) =
            (b.city.as_ref(), b.building, b.ship_type.as_ref())
        {
            m.insert((city.clone(), bid), st.clone());
        }
    }
    m
}

/// **设计图**补丁：新建 / 改值（舰级、选装、意图）/ 改归属 / 删图。
///
/// 校验与丢弃码见 [`BlueprintPatch`] 与 `.agents/notes/ship-blueprint-spec.md` §4.6。
/// 顺序与其它叶一致：**删叶（含冲突检查）→ 校验 → 写**。
fn apply_blueprint(
    state: &mut State,
    config: &GameConfig,
    fid: &FactionId,
    patch: &BlueprintPatch,
    i: usize,
    yard_intent: &BTreeMap<(CityId, BuildingId), String>,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.blueprints[{i}]");
    let mut present = Vec::new();
    if patch.class.is_some() {
        present.push("class");
    }
    if patch.components.is_some() {
        present.push("components");
    }
    if patch.order.is_some() {
        present.push("order");
    }
    if patch.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(patch.remove, &present, &path, report) {
        return;
    }
    // 删**整张图**：挂它的建造区随后是悬空指针 ⇒ 停产（Q10(a)）。图不存在时是幂等成功。
    if patch.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .blueprints
            .remove(&patch.name)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let current = state
        .control(fid.clone())
        .and_then(|c| c.blueprints.get(&patch.name))
        .cloned();
    let wrote_value = patch.class.is_some() || patch.components.is_some() || patch.order.is_some();
    // 图名不存在 + 没有写任何值 ⇒ **绝不凭空造图**（同 `no_such_faction` 防幽灵势力的理由：
    // 一个错别字会造出一张谁都不认识的图，它随后出现在读面里，看起来像真的）。
    if current.is_none() && !wrote_value {
        report.skip(
            format!("{path}.name"),
            &patch.name,
            "no_such_blueprint",
            format!(
                "「{}」这张设计图不在 {fid} 的设计图库里（图名是唯一 key，会被改名/删除）。想建一张新图请把 `class` 一起写上（写值即接管）；只写 `mode` 不会凭空造图。",
                patch.name
            ),
        );
        return;
    }
    // 目标值：写了的用写的，没写的保留现值（新建时 `class` 必给）。
    let class = match (patch.class.as_ref(), current.as_ref()) {
        (Some(c), _) => c.clone(),
        (None, Some(v)) => v.value.class.clone(),
        (None, None) => {
            report.skip(
                format!("{path}.class"),
                "",
                "missing_class",
                "新建一张设计图必须给 `class`（舰级 = config.ships 的 key）：图是「还不存在的舰」的出厂规格，没有舰级的图印不出舰。",
            );
            return;
        }
    };
    if !config.ships.contains_key(&class) {
        let all: Vec<&str> = config.ships.keys().map(String::as_str).collect();
        report.skip(
            format!("{path}.class"),
            &class,
            "no_such_class",
            format!("没有舰级「{class}」（可选：{}）。", all.join(" / ")),
        );
        return;
    }
    let components = patch.components.clone().unwrap_or_else(|| {
        current
            .as_ref()
            .map(|v| v.value.components.clone())
            .unwrap_or_default()
    });
    for c in &components {
        if !config.components.contains_key(c) {
            let all: Vec<&str> = config.components.keys().map(String::as_str).collect();
            report.skip(
                format!("{path}.components"),
                c,
                "no_such_component",
                format!("没有组件「{c}」（可选：{}；也可用 --meta 看全表）。", all.join(" / ")),
            );
            return;
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &components {
        if !seen.insert(c.clone()) {
            report.skip(
                format!("{path}.components"),
                c,
                "duplicate_component",
                format!(
                    "组件「{c}」在同一张图里出现了两次。一件组件一个槽位——与 `choose_loadout` 的 `!chosen.contains(id)` 同一条规则；「双主炮」是新机制，要先重审槽位与平衡。"
                ),
            );
            return;
        }
    }
    let slots = config.ship_spec(&class).slots as usize;
    if components.len() > slots {
        report.skip(
            format!("{path}.components"),
            components.len().to_string(),
            "too_many_components",
            format!(
                "这张图装了 {} 件，而 {class} 只有 {slots} 个槽位（槽位上限必须仍然生效，否则设计图就是新的失衡入口）。",
                components.len()
            ),
        );
        return;
    }
    // 口径 A（Q3）：图的 `class` 必须与**挂它的每个建造区**的 `ship_type` 相等。
    // 「两处一起写」是正解（spec §4.5）⇒ 本份 diff 里那个区的目标舰级也算数。
    for (cid, bid, st) in referencing_yards(state, fid, &patch.name) {
        if st == class {
            continue;
        }
        if yard_intent.get(&(cid.clone(), bid)) == Some(&class) {
            continue;
        }
        report.skip(
            format!("{path}.class"),
            &class,
            "blueprint_class_mismatch",
            format!(
                "「{}」的建造区（{cid} / building={bid}）产的是「{st}」，而这张图的 class 要写成「{class}」：口径 A 下二者必须相等。要么同一份 diff 里把这个建造区的 `ship_type` 也改成同一级，要么别动图的舰级。",
                patch.name
            ),
        );
        return;
    }
    // 写：值 + 归属（写值即接管，与其它叶同一条规则）。
    let leaf = state
        .control
        .entry(fid.clone())
        .or_default()
        .blueprints
        .entry(patch.name.clone())
        .or_insert_with(|| {
            Control::inherit(Blueprint {
                class: class.clone(),
                components: Vec::new(),
                order: None,
            })
        });
    leaf.value.class = class;
    leaf.value.components = components;
    if let Some(order) = &patch.order {
        // `Some(None)` = 本图对**意图**没有说话（清空这一层，链继续往下降到舰队默认）；
        // `Some(Some(v))` = 表态。缺席 = 不动。
        leaf.value.order = order.clone();
    }
    write_mode_leaf(&mut leaf.mode, patch.mode, wrote_value, path, report);
    report.applied += 1;
}

/// A structural building patch: add a new building, remove an existing one, or
/// change an existing building's attributes (structure / ship_type / kind).
#[derive(Deserialize, Default, JsonSchema)]
pub struct BuildingPatch {
    /// Which city to add to / remove from.
    #[serde(default)]
    pub city: Option<CityId>,
    /// Some(id) = target an existing building; None = add a new one.
    #[serde(default)]
    pub building: Option<BuildingId>,
    /// For a new building: kind key (residential | mining | construction).
    #[serde(default)]
    pub kind: Option<String>,
    /// For a new (mining) building: the mined resource key.
    #[serde(default)]
    pub resource: Option<String>,
    /// For a new (建造区) building: the ship class it produces.
    #[serde(default)]
    pub ship_type: Option<String>,
    /// 这个建造区的**设计图**（名字，在所属势力的设计图库里查）。**三层含义**（见
    /// [`double_option`]）：**缺席** = 不动；`null` = **拆掉指针**（回到
    /// `ship_type` + `choose_loadout` 的旧路径）；给名字 = 指向那张图。
    ///
    /// 校验（都点名到叶）：不是建造区 ⇒ `not_a_shipyard`；库里没有这个名字 ⇒
    /// `no_such_blueprint`（**响亮**，绝不静默回落生成器）；图的 `class` 与该区的
    /// `ship_type` 不等 ⇒ `blueprint_class_mismatch`（口径 A；两处**一起写**就都合法）。
    #[serde(default, deserialize_with = "double_option")]
    pub blueprint: Option<Option<BlueprintId>>,
    /// For a new or modified building: structure key (concrete | steel).
    #[serde(default)]
    pub structure: Option<String>,
    /// For a new building: planned area.
    #[serde(default)]
    pub area: Option<f64>,
    /// Remove the referenced building.
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 迁都（首都天体）补丁：`value` 指定新的首都天体（BodyId = 天体唯一名）；
/// `mode` 指定由谁决定（Inherit/Auto/Player）。缺省 `value` 保留现值、缺省 `mode`
/// 保留现模式。迁都的唯一事实来源是 [`ControllableState::capital`]（无 shadow 双状态）。
#[derive(Deserialize, Default, JsonSchema)]
pub struct CapitalPatch {
    /// 新的首都天体（天体唯一名）。缺省 = 保留现值。
    #[serde(default)]
    pub value: Option<BodyId>,
    /// 由谁决定：Auto（系统周期性迁移）/Player（玩家，系统不改写，除非首都亡城强迁）/
    /// Inherit（撤销本层的表态，沿作用域链上溯）。缺省 = 保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（回落到 [`default_capital_body`](crate::model::default_capital_body) 的兜底）。
    /// 与 `value`/`mode` 同时出现 ⇒ 拒绝。
    #[serde(default, skip_serializing_if = "is_false")]
    pub remove: bool,
}

/// 单个势力的可控状态补丁（`--apply` / `POST /api/command` 的 `control[]` 元素）。
/// 只改动**出现在这里**的叶片；缺省的 `Vec` 字段/`Option` 叶子一律保持不变。
///
/// `deny_unknown_fields`：字段名打错（`ship_order` vs `ship_orders`）必须**当场报错**，
/// 而不是被 serde 静默忽略——静默忽略的后果是「整条意图蒸发，退出码 0」，agent 会
/// 以为下达成功。读面 [`FactionControlView`] 的键集是这里的子集（少一个 `buildings`），
/// 所以「编辑模板再回传」这条路径仍然合法（web 的 `POST /api/command` 正是这么用的）。
#[derive(Deserialize, Default, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FactionControlPatch {
    pub faction_id: FactionId,
    /// 迁都（首都天体）补丁。
    #[serde(default)]
    pub capital: Option<CapitalPatch>,
    /// 舰队默认指令（势力级）：新舰出生与一次性指令收尾都回落到它。
    #[serde(default)]
    pub default_ship_order: Option<DefaultShipOrder>,
    /// 舰队默认行为风格（势力级，两片之一）：叶 Inherit 的舰取它的值。
    #[serde(default)]
    pub default_doctrine: Option<DefaultDoctrine>,
    /// 舰队默认风筝<->贴脸姿态（势力级，两片之二）。
    #[serde(default)]
    pub default_kiting: Option<DefaultKiting>,
    /// 舰队默认**角色**（势力级，第三条风格轴）。
    #[serde(default)]
    pub default_freighter: Option<DefaultFreighter>,
    /// **设计图库补丁**（势力级）：新建/改值/改归属/删图。写值即接管（⇒ `Player`）。
    ///
    /// ⚠ 它们在 `apply_diff` 里**先于** `buildings` 应用：同一份 diff 里「建图 + 把某个
    /// 建造区指过去」必须一次成功（否则 agent 得写两条命令，中间那一条会报
    /// `no_such_blueprint`）。
    #[serde(default)]
    pub blueprints: Vec<BlueprintPatch>,
    /// 本势力各舰的指令补丁。
    #[serde(default)]
    pub ship_orders: Vec<ShipOrderPatch>,
    /// 本势力各舰的行为风格补丁（per-舰 可配置）。
    #[serde(default)]
    pub ship_doctrine: Vec<ShipDoctrinePatch>,
    /// 本势力各舰的风筝<->贴脸姿态补丁（per-舰 普通控制属性）。
    #[serde(default)]
    pub ship_kiting: Vec<ShipKitingPatch>,
    /// 本势力各舰的**角色**补丁（per-舰，第三条风格轴）。
    #[serde(default)]
    pub ship_freighter: Vec<ShipFreighterPatch>,
    /// 投资预算补丁（建设）。
    #[serde(default)]
    pub investment_budget: Vec<BudgetPatch>,
    /// 建造预算补丁（造舰）。
    #[serde(default)]
    pub construction_budget: Vec<BudgetPatch>,
    /// 建设投资权重补丁。
    #[serde(default)]
    pub invest_weights: Vec<InvestWeightPatch>,
    /// 建造投资权重补丁。
    #[serde(default)]
    pub build_weights: Vec<BuildWeightPatch>,
    /// 娱乐/福利预算补丁。
    #[serde(default)]
    pub loyalty_budget: Vec<LoyaltyBudgetPatch>,
    /// 结构性建筑补丁（新增/删除/改属性）。
    #[serde(default)]
    pub buildings: Vec<BuildingPatch>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandReq {
    /// Factions' controllable-state patches. Only the factions/leaves that are
    /// present are touched; everything else is left as-is.
    #[serde(default)]
    pub control: Vec<FactionControlPatch>,
    /// Optional scope (AI/玩家 boundary tree) overlay.
    #[serde(default)]
    pub scope: Option<ControlScopePatch>,
}

// --- write-side result (what a diff actually did) ---------------------------

/// 一条**没落地**的控制叶片：diff 里写了它，但状态里没有对应的实体（或这实体不归
/// 该势力），于是它被丢掉了。
///
/// 为什么丢弃**必须**被报出来：`--apply` 的语义是「只触碰 diff 里出现的叶片」，
/// 所以丢叶子本身不是错误——但对写 diff 的 agent 来说，**静默丢掉和成功落地在
/// stdout 与退出码上完全一样**。而丢弃的最常见原因恰恰是必须知道的那种：
/// 舰已战沉、名字换代（`长城` → `长城2`）、城被夷平/易主、`building` 下标属于
/// 另一座城。agent 会带着「命令已下达」的错觉继续玩下去，再把后果归因到别处。
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SkippedLeaf {
    /// 在 diff 里的位置（点名到叶），如 `中国.ship_orders[0].ship`。
    pub path: String,
    /// diff 里写的那个值（名字 / 下标）。
    pub value: String,
    /// 稳定的机器可读原因码（见 [`apply_diff`] 各处的字面量）。
    pub code: String,
    /// 人读的一句话解释（含「接下来怎么办」的线索）。
    pub reason: String,
}

/// 一次 `--apply` / `POST /api/command` **实际做了什么**：落地了几个叶片、
/// 丢了哪些（[`SkippedLeaf`]）、**隐含接管**了哪些、**删掉**了哪些。CLI 在 stderr 上以
/// `WARN_APPLY_SKIPPED` / `NOTE_APPLY_TOOKOVER` / `NOTE_APPLY_REMOVED` 报出（stdout 必须
/// 保持零噪声的状态流）；web 的 `POST /api/command` 把它**原样回给页面**
/// （`planet_x_web::StateView::report`）——界面必须能把 `blueprint_class_mismatch` /
/// `duplicate_component` 这类拒绝**显示出来**，静默吞掉就是「失败看起来像成功」。
/// （在此之前 web 刻意忽略它，理由是"整面回传时丢弃是预期内的"；界面有了**建图/改图**
/// 之后这条不再成立：玩家手写的东西会被守卫拒掉，而那正是他最需要看到的一句话。）
#[derive(Debug, Default, Clone, Serialize, JsonSchema)]
pub struct ApplyReport {
    /// 成功落到状态上的叶片数（一个 `ship_orders[]` 条目 / 一条预算 / 一次迁都… 算一个）。
    /// **删叶也算**（包括"本来就没有那片叶"的幂等删除：目标状态达成了）。
    pub applied: usize,
    /// 没落地的叶片，附带为什么。
    pub skipped: Vec<SkippedLeaf>,
    /// **只写了值、没写 mode** 而被隐含接管成玩家指令的叶片路径（见 [`apply_diff`] 的
    /// 「写值即接管」）。它不是错误，但 agent 需要知道「这一条从这一刻起不再由系统改写」。
    pub took_over: Vec<String>,
    /// 真的被**删掉**的叶片路径（`remove: true` 且那片叶确实存在）。它值得一条回执，因为
    /// 「删叶」的后果是**有效值换来源**（逐舰风格回出厂快照、舰队默认回"没有说话"），
    /// 而"删了一片本来就不存在的叶"不进这个列表（那是幂等的 no-op，见 [`apply_diff`]）。
    pub removed: Vec<String>,
}

impl ApplyReport {
    /// 记一条丢弃。
    fn skip(
        &mut self,
        path: impl Into<String>,
        value: impl Into<String>,
        code: &str,
        reason: impl Into<String>,
    ) {
        self.skipped.push(SkippedLeaf {
            path: path.into(),
            value: value.into(),
            code: code.to_string(),
            reason: reason.into(),
        });
    }

    /// 记一条隐含接管（只写值、没写 mode）。
    fn took_over(&mut self, path: impl Into<String>) {
        self.took_over.push(path.into());
    }

    /// 记一条删叶（只记**真的**删掉了的；幂等删除不记）。
    fn removed(&mut self, path: impl Into<String>) {
        self.removed.push(path.into());
    }

    /// 是否一切都落地了。
    pub fn is_clean(&self) -> bool {
        self.skipped.is_empty()
    }
}

// --- read builders ----------------------------------------------------------

/// `config` 只用于**派生读面**（`launch_waiting` 要按舰级的 `build_points` 判进度是否攒够），
/// 它不参与任何取值决策——写了什么就是什么，所以这条参数不改变控制面的语义。
pub fn control_view(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    c: &ControllableState,
) -> FactionControlView {
    let ship_orders = c
        .ship_orders
        .iter()
        .map(|(sid, ctrl)| ShipOrderEntry { ship: sid.clone(), behavior: ctrl.value.clone(), mode: ctrl.mode })
        .collect();
    // 读面：本势力每艘舰当前的行为风格。**值取有效值**（叶 → 舰队默认 → 舰上记录值），
    // **mode 取叶片自己的表态**（没有叶片 = Inherit）——于是"模板原样回传"安全：没被改过的
    // 行写回去仍然没有意见（有效值原样落进叶，而叶说 Inherit，取值回到原处）。
    let ship_doctrine = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| {
            let eff = state.ship_doctrine(s.name.clone());
            ShipDoctrineEntry {
                ship: s.name.clone(),
                temper: eff.temper,
                lone_wolf: eff.lone_wolf,
                mode: c.ship_doctrine.get(&s.name).map(|l| l.mode).unwrap_or_default(),
            }
        })
        .collect();
    let ship_kiting = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ShipKitingEntry {
            ship: s.name.clone(),
            kiting: state.ship_kiting(s.name.clone()),
            mode: c.ship_kiting.get(&s.name).map(|l| l.mode).unwrap_or_default(),
        })
        .collect();
    // 角色：**有效值**（自动控制可能刚写过它）+ 那片叶自己的表态（`Player` = 玩家钉的）。
    let ship_freighter = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ShipFreighterEntry {
            ship: s.name.clone(),
            freighter: state.ship_freighter(s.name.clone()),
            mode: c.ship_freighter.get(&s.name).map(|l| l.mode).unwrap_or_default(),
        })
        .collect();
    let investment_budget = c
        .investment_budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry { resource: rt.clone(), value: ctrl.value, mode: ctrl.mode })
        .collect();
    let construction_budget = c
        .construction_budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry { resource: rt.clone(), value: ctrl.value, mode: ctrl.mode })
        .collect();
    let invest_weights = c
        .invest_weights
        .iter()
        .map(|((cid, bid), ctrl)| {
            let b = state.city(cid).and_then(|cty| cty.buildings.iter().find(|b| b.id == *bid));
            InvestWeightEntry {
                city: cid.clone(),
                building: *bid,
                kind: b.map(|x| x.kind.clone()).unwrap_or_default(),
                resource: b.and_then(|x| x.resource.clone()),
                ship_type: b.and_then(|x| x.ship_type.clone()),
                structure: b.map(|x| x.structure.clone()).unwrap_or_default(),
                value: ctrl.value,
                mode: ctrl.mode,
            }
        })
        .collect();
    let build_weights = c
        .build_weights
        .iter()
        .map(|((cid, bid), ctrl)| {
            let b = state.city(cid).and_then(|cty| cty.buildings.iter().find(|b| b.id == *bid));
            BuildWeightEntry {
                city: cid.clone(),
                building: *bid,
                ship_type: b.and_then(|x| x.ship_type.clone()),
                value: ctrl.value,
                mode: ctrl.mode,
            }
        })
        .collect();
    let loyalty_budget = c
        .loyalty_budget
        .iter()
        .map(|(cid, ctrl)| LoyaltyBudgetEntry { city: cid.clone(), value: ctrl.value, mode: ctrl.mode })
        .collect();
    // 设计图库：**每张图一行**。`ship_count` 是**现算的派生量**（不落状态），`mode` 是图叶
    // 自己的表态；有效归属（图叶 → 势力 scope → 全局）走 `State::blueprint_control`，
    // 读面在投影的 `blueprints.effective_mode` 列里给（`--control` 是**写面模板**，
    // 多给派生列只会让模板与写面漂移）。
    //
    // ⚠ `ship_count` / `launch_waiting` 是这条规则的两个**例外**：它们确实是派生量，但
    // **必须**在写面模板里（读面即写面 ⇒ 写面得先收下它们；而且「这张图在等钱」是
    // 玩家做决定要看的东西，投影列在 CLI 侧够用、在 web 里够不着）。它们**只读**：
    // `BlueprintPatch` 收下但不写回状态。
    let blueprints = c
        .blueprints
        .iter()
        .map(|(name, ctrl)| BlueprintEntry {
            name: name.clone(),
            class: ctrl.value.class.clone(),
            components: ctrl.value.components.clone(),
            order: ctrl.value.order.clone(),
            mode: ctrl.mode,
            ship_count: state
                .ships
                .iter()
                .filter(|s| s.faction_id == fid && s.blueprint.as_deref() == Some(name.as_str()))
                .count(),
            launch_waiting: crate::sim::blueprint_launch_waiting(state, config, &fid, name),
        })
        .collect();
    FactionControlView {
        faction_id: fid,
        capital: c.capital.clone(),
        // 读面这几片是**值 + 表态**（"这一层说了什么"），`remove` 只存在于**写面**：
        // 读面表达"没有这片叶"的方式就是 `None`/不给这一行（见 `scope_view` 同理）。
        default_ship_order: c.default_ship_order.as_ref().map(|d| DefaultShipOrder {
            behavior: Some(d.value.clone()),
            mode: Some(d.mode),
            remove: false,
        }),
        default_doctrine: c.default_doctrine.as_ref().map(|d| DefaultDoctrine {
            temper: Some(d.value.temper),
            lone_wolf: Some(d.value.lone_wolf),
            mode: Some(d.mode),
            remove: false,
        }),
        default_kiting: c.default_kiting.as_ref().map(|d| DefaultKiting {
            kiting: Some(d.value),
            mode: Some(d.mode),
            remove: false,
        }),
        default_freighter: c.default_freighter.as_ref().map(|d| DefaultFreighter {
            freighter: Some(d.value),
            mode: Some(d.mode),
            remove: false,
        }),
        blueprints,
        ship_orders,
        ship_doctrine,
        ship_kiting,
        ship_freighter,
        investment_budget,
        construction_budget,
        invest_weights,
        build_weights,
        loyalty_budget,
    }
}

/// 作用域树的**读面**：只列出**有意见**的节点/键（`Inherit` ≡ 没有说话，不必列出，
/// 与「这个键不存在」等价）。读面即写面，所以这份模板原样回传安全：没列出的层不会被
/// 意外清掉。
pub fn scope_view(s: &ControlScope) -> ControlScopePatch {
    ControlScopePatch {
        global: (s.global != ControlMode::Inherit).then_some(s.global),
        factions: explicit(&s.factions),
        bodies: explicit(&s.bodies),
        cities: explicit(&s.cities),
    }
}

/// 过滤掉「没有说话」（`Inherit`）的键。
fn explicit<K: Clone + Ord>(m: &std::collections::BTreeMap<K, ControlMode>) -> Vec<(K, ControlMode)> {
    m.iter()
        .filter(|(_, v)| **v != ControlMode::Inherit)
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

/// Render the current editable control surface (control + scope) as JSON —
/// the template an agent edits and posts back as a diff.
///
/// **这里刻意不做任何数值舍入**（曾经把所有数字四舍五入到 2 位小数以求 token 干净）。
/// 理由：这个函数是「**读面即写面、模板原样回传安全**」这句话的兑现处，
/// 而往模板里塞一个**有损**变换，等于把那句承诺变成假的 —— `0.125` 会被显示成 `0.13`，
/// 原样回传就真的把叶值改成了 `0.13`：一次静默的、没人要求的写操作。
///
/// 代价核算过（`engine-data-plane.md` §8.3）：实测一份跑到 120 回合的真实控制面里，
/// **470 个数值没有一个是 2 位小数舍入会改变的** —— 也就是说这点 token 噪声在当前世界里
/// 根本不存在，舍入**只带来风险、没带来收益**。想要好看的数字是客户端的事
/// （Python kit / LLM 自己 `round()`），引擎的输出是数据。
pub fn control_surface(state: &State, config: &GameConfig) -> serde_json::Value {
    let control = state
        .control
        .iter()
        .map(|(fid, c)| control_view(state, config, fid.clone(), c))
        .collect();
    let surface = ControlSurface { control, scope: scope_view(&state.scope) };
    serde_json::to_value(surface).expect("control surface is serializable")
}

/// The machine-readable JSON Schema for the **control/`--apply` diff**
/// ([`CommandReq`]). Handed to the agent so it can write a steering diff without
/// memorising the contract. Auto-derived from the same structs the diff is
/// deserialised into, so it can never drift from `apply_patch`'s shape.
pub fn control_schema_value() -> serde_json::Value {
    let schema = schemars::schema_for!(CommandReq);
    serde_json::to_value(schema).expect("control schema is serializable")
}

// --- write side (diff application) ------------------------------------------

    // --- 删叶（`remove: true`） --------------------------------------------------
//
// 控制叶的**存在性本身就是一种状态**：「没有叶」= 这一层没有说话。而在取值规则里
// 「叶不存在」与「叶写着 `Inherit`」并**不**等价——`State::ship_doctrine` 是
// `leaf.map(|l| l.value).unwrap_or(record)`：**叶存在就用叶里的值**（与 `mode` 无关），
// 只有叶真的不存在才回落到出厂记录值。于是"碰过一次的风格叶"以前永远钉着那个数
// （`mode: Inherit` 撤不掉它），而补丁接口只能新建/改写叶、删不掉——`remove` 就是那个出口。
//
// 三条规则（2026-10 裁决）：
// 1. 删的是**控制面里那片叶**，不要求实体还在（舰战沉 / 城易主 / 建筑没了 / 资源 key 已删
//    都能删）⇒ 顺带是清理陈叶的路；
// 2. 叶本来就不存在 ⇒ **幂等成功**（目标状态就是"没有这片叶"）：不进 `removed`，也不算丢弃；
// 3. `remove` 与任何值 / `mode` 字段同时出现 ⇒ **拒绝**：一条同时说着"删掉它"和"设成 0.5"
//    的补丁没有正确答案，而任何一种静默优先级都会让写补丁的人以为另一件事发生了。

/// `remove: true` 同时带了别的字段 ⇒ 记一条拒绝。返回 `true` = 这条补丁到此为止。
fn remove_conflicts(remove: bool, present: &[&str], path: &str, report: &mut ApplyReport) -> bool {
    if !remove || present.is_empty() {
        return false;
    }
    report.skip(
        path,
        "",
        "remove_conflicts_with_value",
        format!(
            "`remove: true` 不能再带 {}：删掉这片叶与给它写值/写归属是两件事（要什么值请删完再单独发一条）。",
            present.join(" / ")
        ),
    );
    true
}

/// 记一次删叶的结果：**真的**删掉了才进 `removed`；本来就没有这片叶是幂等成功。
fn leaf_removed(report: &mut ApplyReport, path: String, existed: bool) {
    if existed {
        report.removed(path);
    }
    report.applied += 1;
}

/// 舰队默认指令（势力级）：新舰出生与一次性指令收尾都回落到它。
fn apply_default_ship_order(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultShipOrder,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_ship_order");
    let mut present = Vec::new();
    if d.behavior.is_some() {
        present.push("behavior");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state.control.entry(fid.clone()).or_default().default_ship_order.take().is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let c = state.control.entry(fid.clone()).or_default();
    let ctrl = c
        .default_ship_order
        .get_or_insert_with(|| Control::inherit(ShipBehavior::Idle));
    if let Some(v) = &d.behavior {
        ctrl.value = v.clone();
    }
    // 写值即接管（与叶子同一条规则）：只写默认行为、没写 mode，就是「这是我的默认」。
    match (d.mode, d.behavior.is_some()) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 舰队默认**行为风格**（势力级，两片之一）：与「写值即接管」同一条规则，外加一条
/// **两轴叶**的额外守卫（见 `partial_doctrine_leaf`）。
fn apply_default_doctrine(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultDoctrine,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_doctrine");
    let mut present = Vec::new();
    if d.temper.is_some() {
        present.push("temper");
    }
    if d.lone_wolf.is_some() {
        present.push("lone_wolf");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state.control.entry(fid.clone()).or_default().default_doctrine.take().is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let wrote = d.temper.is_some() || d.lone_wolf.is_some();
    // **两轴叶的"新建"必须两条轴一起给**（`0.0` 是个正常取值：静默把它填进另一条轴，
    // 事后从读面完全看不出来——这正是这条守卫存在的理由）。叶已存在时单轴写仍然合法
    // （缺省轴保留现值）。
    let exists = state
        .control
        .get(fid)
        .and_then(|c| c.default_doctrine.as_ref())
        .is_some();
    if !exists && wrote && !(d.temper.is_some() && d.lone_wolf.is_some()) {
        report.skip(
            path,
            "",
            "partial_doctrine_leaf",
            "`default_doctrine` 是**两轴一片叶**（temper + lone_wolf），而这片叶还不存在：只给一条轴会把另一条静默设成 0.0（= 基线），而 0.0 是个正常取值，事后从读面看不出来。三条路任选：① 两条轴一起给；② 先只写 `mode`（先表态归属，值下次再给）；③ `remove: true` 删掉这片叶（回到「没有说话」）。",
        );
        return;
    }
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .default_doctrine
        .get_or_insert_with(|| Control::inherit(ShipDoctrine::default()));
    if let Some(v) = d.temper {
        ctrl.value.temper = v.clamp(-1.0, 1.0);
    }
    if let Some(v) = d.lone_wolf {
        ctrl.value.lone_wolf = v.clamp(-1.0, 1.0);
    }
    match (d.mode, wrote) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 舰队默认**风筝<->贴脸姿态**（势力级，两片之二）。
fn apply_default_kiting(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultKiting,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_kiting");
    let mut present = Vec::new();
    if d.kiting.is_some() {
        present.push("kiting");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state.control.entry(fid.clone()).or_default().default_kiting.take().is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let wrote = d.kiting.is_some();
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .default_kiting
        .get_or_insert_with(|| Control::inherit(0.0));
    if let Some(v) = d.kiting {
        ctrl.value = v.clamp(-1.0, 1.0);
    }
    match (d.mode, wrote) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 舰队默认**角色**（势力级，第三条风格轴）：与 [`apply_default_kiting`] 同形，
/// 外加这片叶特有的用途——设成 `Player` 是「AI 定编别碰我的舰队」的闸门。
fn apply_default_freighter(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultFreighter,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_freighter");
    let mut present = Vec::new();
    if d.freighter.is_some() {
        present.push("freighter");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .default_freighter
            .take()
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let wrote = d.freighter.is_some();
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .default_freighter
        .get_or_insert_with(|| Control::inherit(false));
    if let Some(v) = d.freighter {
        ctrl.value = v;
    }
    match (d.mode, wrote) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 逐舰指令补丁：`behavior` 替换该舰行为、`mode` 指定由谁决定、`remove` 删掉这片叶。
fn apply_ship_order(
    state: &mut State,
    fid: &FactionId,
    sp: &ShipOrderPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_orders[{i}].ship");
    let mut present = Vec::new();
    if sp.behavior.is_some() {
        present.push("behavior");
    }
    if sp.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(sp.remove, &present, &path, report) {
        return;
    }
    // 删叶**不要求舰还在**：删的是我们控制面里的那片叶（陈叶清理也是它的用途之一）。
    if sp.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_orders
            .remove(&sp.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    // Resolve the ship by its **name** (the unique key): an order only applies to a
    // ship that exists and that this faction actually owns, so ordering another
    // faction's ship (or a vanished one) is a no-op.
    let Some(ship_name) = resolve_own_ship(state, fid, &sp.ship, &path, report) else {
        return;
    };
    // 「写值即接管」：只写了 behavior 而没写 mode，就意味着这是**玩家的指令**
    // （否则值会被系统下一回合按自己的逻辑覆盖，而 agent 以为命令已下达——
    // 这正是 `agent-play-friction` 里那类「失败看起来像成功」）。
    let implied = match (sp.mode, sp.behavior.is_some()) {
        (Some(m), _) => m,
        (None, true) => {
            report.took_over(format!("{fid}.ship_orders[{i}].behavior"));
            ControlMode::Player
        }
        (None, false) => ControlMode::Inherit,
    };
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_orders
        .entry(ship_name)
        .or_insert_with(|| Control {
            value: sp.behavior.clone().unwrap_or(ShipBehavior::Idle),
            mode: implied,
        });
    if let Some(v) = &sp.behavior {
        ctrl.value = v.clone();
    }
    if let Some(m) = sp.mode {
        ctrl.mode = m;
    } else if sp.behavior.is_some() {
        ctrl.mode = ControlMode::Player;
    }
    report.applied += 1;
}

/// 逐舰**行为风格**补丁：写的是**叶片**（值 + 三态），不是舰上那个记录值——`Ship.doctrine`
/// 只是出厂快照 / AI 流水。每条轴钳制到 [-1,1]；缺省轴保留「当前有效」的那条（所以只写一条轴
/// 不会把另一条清零，也**不会**把另一条变成 0）。写值即接管（与其它叶同一条规则）。
fn apply_ship_doctrine(
    state: &mut State,
    fid: &FactionId,
    d: &ShipDoctrinePatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_doctrine[{i}].ship");
    let mut present = Vec::new();
    if d.temper.is_some() {
        present.push("temper");
    }
    if d.lone_wolf.is_some() {
        present.push("lone_wolf");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    // 删叶 ⇒ 有效风格回落到舰队默认 / **出厂快照**（叶不存在 = 这一层没有说话，且叶里的值
    // 不再参与取值）。舰已战沉也能删。
    if d.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_doctrine
            .remove(&d.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if resolve_own_ship(state, fid, &d.ship, &path, report).is_none() {
        return;
    }
    let base = state.ship_doctrine(d.ship.clone());
    let value = ShipDoctrine {
        temper: d.temper.map(|v| v.clamp(-1.0, 1.0)).unwrap_or(base.temper),
        lone_wolf: d.lone_wolf.map(|v| v.clamp(-1.0, 1.0)).unwrap_or(base.lone_wolf),
    };
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_doctrine
        .entry(d.ship.clone())
        .or_insert_with(|| Control::inherit(base));
    ctrl.value = value;
    write_mode_leaf(
        &mut ctrl.mode,
        d.mode,
        d.temper.is_some() || d.lone_wolf.is_some(),
        format!("{fid}.ship_doctrine[{i}]"),
        report,
    );
    report.applied += 1;
}

/// 逐舰**风筝<->贴脸姿态**补丁：与 [`apply_ship_doctrine`] 同一条路（叶片 + 写值即接管 + 删叶）。
fn apply_ship_kiting(
    state: &mut State,
    fid: &FactionId,
    k: &ShipKitingPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_kiting[{i}].ship");
    let mut present = Vec::new();
    if k.kiting.is_some() {
        present.push("kiting");
    }
    if k.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(k.remove, &present, &path, report) {
        return;
    }
    if k.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_kiting
            .remove(&k.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if resolve_own_ship(state, fid, &k.ship, &path, report).is_none() {
        return;
    }
    let base = state.ship_kiting(k.ship.clone());
    let value = k.kiting.map(|v| v.clamp(-1.0, 1.0)).unwrap_or(base);
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_kiting
        .entry(k.ship.clone())
        .or_insert_with(|| Control::inherit(base));
    ctrl.value = value;
    write_mode_leaf(
        &mut ctrl.mode,
        k.mode,
        k.kiting.is_some(),
        format!("{fid}.ship_kiting[{i}]"),
        report,
    );
    report.applied += 1;
}

/// 逐舰**角色**补丁（第三条风格轴）：与 [`apply_ship_kiting`] 同一条路（叶片 + 写值即接管 +
/// 删叶）。唯一与另两条轴的差别：这片叶**自动控制也会写**（按积压定编），所以
/// 「删叶」在这条轴上的意思是**交回自动定编**（AI 可能下回合立刻又写下结论），
/// 而不是「从此保持某个值」——要后者就写 `Player`。
fn apply_ship_freighter(
    state: &mut State,
    fid: &FactionId,
    f: &ShipFreighterPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_freighter[{i}].ship");
    let mut present = Vec::new();
    if f.freighter.is_some() {
        present.push("freighter");
    }
    if f.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(f.remove, &present, &path, report) {
        return;
    }
    // 与另两条轴一样：删叶**不要求舰还在**（陈叶清理）。
    if f.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_freighter
            .remove(&f.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if resolve_own_ship(state, fid, &f.ship, &path, report).is_none() {
        return;
    }
    let base = state.ship_freighter(f.ship.clone());
    let value = f.freighter.unwrap_or(base);
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_freighter
        .entry(f.ship.clone())
        .or_insert_with(|| Control::inherit(base));
    ctrl.value = value;
    write_mode_leaf(
        &mut ctrl.mode,
        f.mode,
        f.freighter.is_some(),
        format!("{fid}.ship_freighter[{i}]"),
        report,
    );
    report.applied += 1;
}

/// 资源预算补丁（投资 / 建造共用）：`value` 替换预算额、`mode` 指定由谁决定、`remove` 删叶。
fn apply_budget(
    state: &mut State,
    config: &GameConfig,
    fid: &FactionId,
    which: BudgetKind,
    bp: &BudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let name = which.name();
    let path = format!("{fid}.{name}[{i}].resource");
    let mut present = Vec::new();
    if bp.value.is_some() {
        present.push("value");
    }
    if bp.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(bp.remove, &present, &path, report) {
        return;
    }
    // 删叶先于「资源是否存在」的检查：config 里已经没有的 key 也能删（清理陈叶）。
    if bp.remove {
        let c = state.control.entry(fid.clone()).or_default();
        let map = match which {
            BudgetKind::Investment => &mut c.investment_budget,
            BudgetKind::Construction => &mut c.construction_budget,
        };
        let existed = map.remove(&bp.resource).is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if !config.resources.contains_key(&bp.resource) {
        report.skip(
            path,
            &bp.resource,
            "no_such_resource",
            format!("没有资源 key「{}」（WYSIWYG：状态里的 key 就是 diff 里的 key，用 --meta 的 resources 看全表）。", bp.resource),
        );
        return;
    }
    let c = state.control.entry(fid.clone()).or_default();
    let map = match which {
        BudgetKind::Investment => &mut c.investment_budget,
        BudgetKind::Construction => &mut c.construction_budget,
    };
    let ctrl = map.entry(bp.resource.clone()).or_insert_with(|| Control {
        value: bp.value.unwrap_or(0.0),
        mode: bp.mode.unwrap_or_default(),
    });
    write_value_leaf(ctrl, bp.value, bp.mode, format!("{fid}.{name}[{i}].value"), report);
    report.applied += 1;
}

/// `apply_budget` 的两条路（投资 / 建造）——同一套代码，只有 map 与名字不同。
#[derive(Clone, Copy)]
enum BudgetKind {
    Investment,
    Construction,
}

impl BudgetKind {
    fn name(self) -> &'static str {
        match self {
            BudgetKind::Investment => "investment_budget",
            BudgetKind::Construction => "construction_budget",
        }
    }
}

/// 某城某建筑的两类权重补丁（建设投资权重 / 建造投资权重）：`value` 替换权重、`remove` 删叶。
fn apply_weight(
    state: &mut State,
    fid: &FactionId,
    which: WeightKind,
    city: &CityId,
    building: &BuildingId,
    value: Option<f64>,
    mode: Option<ControlMode>,
    remove: bool,
    i: usize,
    report: &mut ApplyReport,
) {
    let name = which.name();
    let path = format!("{fid}.{name}[{i}]");
    let mut present = Vec::new();
    if value.is_some() {
        present.push("value");
    }
    if mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(remove, &present, &path, report) {
        return;
    }
    let key = (city.clone(), *building);
    // 删叶先于「城/建筑是否存在」的检查：建筑没了、城易主了也能删（清理陈叶）。
    if remove {
        let c = state.control.entry(fid.clone()).or_default();
        let map = match which {
            WeightKind::Invest => &mut c.invest_weights,
            WeightKind::Build => &mut c.build_weights,
        };
        let existed = map.remove(&key).is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if !check_city_building(state, fid, city, *building, &path, report) {
        return;
    }
    let c = state.control.entry(fid.clone()).or_default();
    let map = match which {
        WeightKind::Invest => &mut c.invest_weights,
        WeightKind::Build => &mut c.build_weights,
    };
    let ctrl = map.entry(key).or_insert_with(|| Control {
        value: value.unwrap_or(0.0),
        mode: mode.unwrap_or_default(),
    });
    write_value_leaf(ctrl, value, mode, format!("{fid}.{name}[{i}].value"), report);
    report.applied += 1;
}

/// `apply_weight` 的两条路（建设投资权重 / 建造投资权重）。
#[derive(Clone, Copy)]
enum WeightKind {
    Invest,
    Build,
}

impl WeightKind {
    fn name(self) -> &'static str {
        match self {
            WeightKind::Invest => "invest_weights",
            WeightKind::Build => "build_weights",
        }
    }
}

/// 某城娱乐/福利预算补丁：`value` 替换预算额、`remove` 删叶。
fn apply_loyalty_budget(
    state: &mut State,
    fid: &FactionId,
    lp: &LoyaltyBudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.loyalty_budget[{i}]");
    let mut present = Vec::new();
    if lp.value.is_some() {
        present.push("value");
    }
    if lp.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(lp.remove, &present, &path, report) {
        return;
    }
    if lp.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .loyalty_budget
            .remove(&lp.city)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    match state.city(&lp.city) {
        None => {
            report.skip(
                format!("{path}.city"),
                &lp.city,
                "no_such_city",
                format!("没有名为「{}」的城（城被夷平后名字会从活城列表里消失）。", lp.city),
            );
            return;
        }
        Some(city) if city.faction_id != *fid => {
            report.skip(
                format!("{path}.city"),
                &lp.city,
                "not_your_city",
                format!("「{}」属于 {}，不是 {fid} 的城。", lp.city, city.faction_id),
            );
            return;
        }
        Some(_) => {}
    }
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .loyalty_budget
        .entry(lp.city.clone())
        .or_insert_with(|| Control {
            value: lp.value.unwrap_or(0.0),
            mode: lp.mode.unwrap_or_default(),
        });
    write_value_leaf(ctrl, lp.value, lp.mode, format!("{fid}.loyalty_budget[{i}].value"), report);
    report.applied += 1;
}

/// 把一条 `ship` 引用解析成「本势力确实拥有的那艘舰」，或一条**丢弃记录**。
/// 舰名是唯一 key，但**会变**（战沉后重建的护卫舰叫 `长城2`），所以「查无此舰」
/// 与「这舰不是你的」必须分开报——前者要 agent 重新查名字，后者是写错了势力。
fn resolve_own_ship(
    state: &State,
    fid: &str,
    ship: &str,
    path: &str,
    report: &mut ApplyReport,
) -> Option<String> {
    match state.ships.iter().find(|s| s.name == ship) {
        None => {
            report.skip(
                path,
                ship,
                "no_such_ship",
                format!("世界里没有名为「{ship}」的舰。舰名会换代（战沉后重建的是「{ship}2」），用 --index + planet_xq 的 q.fleet(r, \"{fid}\") 查现名。"),
            );
            None
        }
        Some(s) if s.faction_id != fid => {
            report.skip(
                path,
                ship,
                "not_your_ship",
                format!("「{ship}」属于 {}，不是 {fid} 的舰——指令只对本势力的舰生效。", s.faction_id),
            );
            None
        }
        Some(s) => Some(s.name.clone()),
    }
}

/// 落一条「值 + 三态」的叶片补丁（预算 / 权重 / 忠诚预算共用）：**写值即接管**——
/// 只写了值、没写 `mode`，就意味着这是玩家的指令（该叶变成 `Player`），并在回执里记一笔。
///
/// 为什么：值写进去、而 mode 仍解析成 `Auto` 时，系统下一回合就会按自己的逻辑覆盖它。
/// stdout 与退出码一切正常，agent 却会带着「命令已下达」的错觉玩下去——这正是本项目
/// 反复吃过的「失败看起来像成功」。
/// 「写值即接管」的三态落点（叶片共用）：显式 `mode` 优先；只写了值没写 `mode` ⇒ `Player`
/// 并在回执里记一笔；什么都没写 ⇒ 保留现模式。
fn write_mode_leaf(
    ctrl: &mut ControlMode,
    mode: Option<ControlMode>,
    wrote_value: bool,
    path: String,
    report: &mut ApplyReport,
) {
    match mode {
        Some(m) => *ctrl = m,
        None if wrote_value => {
            *ctrl = ControlMode::Player;
            report.took_over(path);
        }
        None => {}
    }
}

fn write_value_leaf<T>(
    ctrl: &mut Control<T>,
    value: Option<T>,
    mode: Option<ControlMode>,
    path: String,
    report: &mut ApplyReport,
) {
    let wrote_value = value.is_some();
    if let Some(v) = value {
        ctrl.value = v;
    }
    match mode {
        Some(m) => ctrl.mode = m,
        None if wrote_value => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        None => {}
    }
}

/// 校验一对 `(city, building)`：两个名字/下标都必须在**同一座城**里对得上。/// `building` 是 u32 下标（在它所属城内部唯一，见 `.agents/notes/name-as-unique-key.md`
/// 的裁决），所以换一座城
/// 就得换下标——这是 agent 手写权重时最容易错的地方。
fn check_city_building(
    state: &State,
    fid: &str,
    city: &str,
    building: BuildingId,
    path: &str,
    report: &mut ApplyReport,
) -> bool {
    let Some(c) = state.city(city) else {
        report.skip(
            format!("{path}.city"),
            city,
            "no_such_city",
            format!("没有名为「{city}」的城（城被夷平后名字会从活城列表里消失）。"),
        );
        return false;
    };
    if c.faction_id != fid {
        report.skip(
            format!("{path}.city"),
            city,
            "not_your_city",
            format!("「{city}」属于 {}，不是 {fid} 的城。", c.faction_id),
        );
        return false;
    }
    if !c.buildings.iter().any(|b| b.id == building) {
        let ids: Vec<String> = c.buildings.iter().map(|b| b.id.to_string()).collect();
        report.skip(
            format!("{path}.building"),
            building.to_string(),
            "no_such_building",
            format!("「{city}」里没有 building={building}；它的建筑下标是 [{}]（下标只在城内部唯一，换城要换下标）。", ids.join(", ")),
        );
        return false;
    }
    true
}

/// Apply a presence-aware control patch (a structural multi-level diff) to
/// `state`. Only the factions and leaves present in `req` are modified; for a
/// leaf that is present, an omitted `value`/`behavior` keeps the current value
/// and an omitted `mode` keeps the current mode. `scope` is overlaid when given.
///
/// 返回 [`ApplyReport`]：落地了几个叶片、**丢了哪些以及为什么**。丢弃不是错误
/// （「只触碰 diff 里出现的叶片」本来就允许引用已经不存在的实体），但它是 agent
/// 唯一能发现「命令其实没下达」的渠道，所以每一条 `continue`/`return` 都必须
/// 先记一笔——**新增分支时别忘了 `report.skip`**。
pub fn apply_diff(state: &mut State, config: &GameConfig, req: &CommandReq) -> ApplyReport {
    let mut report = ApplyReport::default();
    for (fi, fac) in req.control.iter().enumerate() {
        let fid = fac.faction_id.clone();
        // 势力必须真实存在：否则 `control.entry().or_default()` 会在可控状态里
        // **凭空造出一个幽灵势力**，它随后出现在 `--control` 模板与 web 控制面里，
        // 看起来像一个真的（只是永远不动）势力——一个错别字污染整个世界。
        if state.faction(&fid).is_none() {
            report.skip(
                format!("control[{fi}].faction_id"),
                &fid,
                "no_such_faction",
                format!("没有名为「{fid}」的势力（势力名是唯一 key；用 --index + planet_xq 的 q.facts 看现名）。"),
            );
            continue;
        }
        // 注意：这里**不能**提前 `let c = state.control.entry(..)`——那会把
        // `state.control` 借出去，后面所有需要 `state.city(..)` 的校验都借不动。
        // 每个写点各自取一次 entry（同名 `fid` 的 `Control` 是同一个）。
        //
        // 每片叶一个 `apply_*` 助手：它们各自处理「删叶 / 写值 / 写归属」三件事，
        // 顺序统一是 **删叶（含冲突检查）→ 实体校验 → 写**。抽出来的原因不是行数：
        // `remove` 的冲突检查与幂等语义要在**每一片**叶上完全一致。
        if let Some(d) = &fac.default_ship_order {
            apply_default_ship_order(state, &fid, d, &mut report);
        }
        if let Some(d) = &fac.default_doctrine {
            apply_default_doctrine(state, &fid, d, &mut report);
        }
        if let Some(d) = &fac.default_kiting {
            apply_default_kiting(state, &fid, d, &mut report);
        }
        // 舰队默认**角色**（势力级，第三条风格轴）。写它 = 全舰队按这个角色走；
        // 设成 `Player` 之后自动控制的逐舰定编不再生效（那片叶归玩家）。
        if let Some(d) = &fac.default_freighter {
            apply_default_freighter(state, &fid, d, &mut report);
        }
        // **设计图库**（势力级）：**先于** `buildings` 应用——同一份 diff 里「建图 + 把某个
        // 建造区指过去」必须一次成功（否则 agent 得写两条命令，中间那条会报
        // `no_such_blueprint`）。校验用的是**本份 diff 之后**的意图（`yard_intent`），
        // 所以「图与区的舰级两处一起写」也一次成功。
        let yard_intent = yard_ship_type_intent(fac);
        for (i, bp) in fac.blueprints.iter().enumerate() {
            apply_blueprint(state, config, &fid, bp, i, &yard_intent, &mut report);
        }
        for (i, sp) in fac.ship_orders.iter().enumerate() {
            apply_ship_order(state, &fid, sp, i, &mut report);
        }
        for (i, d) in fac.ship_doctrine.iter().enumerate() {
            apply_ship_doctrine(state, &fid, d, i, &mut report);
        }
        for (i, k) in fac.ship_kiting.iter().enumerate() {
            apply_ship_kiting(state, &fid, k, i, &mut report);
        }
        // **角色**补丁（per-舰，第三条风格轴）：同一条路（叶片 + 写值即接管）。
        // 与另两条轴的差别：这片叶自动控制**也会写**，但**玩家写过（`Player`）之后 AI 不再碰**
        // ——所以「手动给某艘舰定活」是一次性的、且能一直压住自动定编。
        for (i, f) in fac.ship_freighter.iter().enumerate() {
            apply_ship_freighter(state, &fid, f, i, &mut report);
        }
        for (i, bp) in fac.investment_budget.iter().enumerate() {
            apply_budget(state, config, &fid, BudgetKind::Investment, bp, i, &mut report);
        }
        for (i, bp) in fac.construction_budget.iter().enumerate() {
            apply_budget(state, config, &fid, BudgetKind::Construction, bp, i, &mut report);
        }
        for (i, ip) in fac.invest_weights.iter().enumerate() {
            apply_weight(
                state,
                &fid,
                WeightKind::Invest,
                &ip.city,
                &ip.building,
                ip.value,
                ip.mode,
                ip.remove,
                i,
                &mut report,
            );
        }
        for (i, bp) in fac.build_weights.iter().enumerate() {
            apply_weight(
                state,
                &fid,
                WeightKind::Build,
                &bp.city,
                &bp.building,
                bp.value,
                bp.mode,
                bp.remove,
                i,
                &mut report,
            );
        }
        for (i, lp) in fac.loyalty_budget.iter().enumerate() {
            apply_loyalty_budget(state, &fid, lp, i, &mut report);
        }
        for (i, bpatch) in fac.buildings.iter().enumerate() {
            let path = format!("{fid}.buildings[{i}]");
            if apply_building_patch(state, config, fid.clone(), bpatch, &path, &mut report) {
                report.applied += 1;
            }
        }
    }
    // 迁都：写「有效首都」的唯一事实来源（`ControllableState::capital`）。独立的循环
    // 以拿到干净的借用：先做只读（当前首都、天体存在性），再在独立作用域里写控制。
    // 只允许迁到真实存在的天体；若指向的天体上并无本势力活城，则由 sim 的亡城强迁
    // 规则兜底，避免把首都钉在虚天体上。`mode` 沿作用域链与其它叶子一致。
    for (fi, fac) in req.control.iter().enumerate() {
        if let Some(cap) = &fac.capital {
            if state.faction(&fac.faction_id).is_none() {
                continue; // 已在上面的循环里记过 no_such_faction。
            }
            let path = format!("control[{fi}].capital");
            let mut present = Vec::new();
            if cap.value.is_some() {
                present.push("value");
            }
            if cap.mode.is_some() {
                present.push("mode");
            }
            if remove_conflicts(cap.remove, &present, &path, &mut report) {
                continue;
            }
            if cap.remove {
                let existed = state
                    .control
                    .entry(fac.faction_id.clone())
                    .or_default()
                    .capital
                    .take()
                    .is_some();
                leaf_removed(&mut report, path, existed);
                continue;
            }
            let cur = state.capital_body(&fac.faction_id);
            let new_value = match cap.value.as_ref() {
                Some(v) if state.body(v).is_none() => {
                    report.skip(
                        format!("{path}.value"),
                        v,
                        "no_such_body",
                        format!("没有名为「{v}」的天体（天体名是唯一 key）。"),
                    );
                    None
                }
                other => other.cloned(),
            };
            {
                let ctrl = state.control.entry(fac.faction_id.clone()).or_default();
                let ctrl = ctrl.capital.get_or_insert_with(|| Control::inherit(cur));
                // 无效天体（上面刚跳过）不算「写了值」，否则会记一条假的隐含接管。
                let wrote = new_value.is_some();
                if let Some(v) = new_value {
                    ctrl.value = v;
                }
                match (cap.mode, wrote) {
                    (Some(m), _) => ctrl.mode = m,
                    (None, true) => {
                        ctrl.mode = ControlMode::Player;
                        report.took_over(format!("{path}.value"));
                    }
                    (None, false) => {}
                }
            }
            report.applied += 1;
        }
    }
    if let Some(sv) = &req.scope {
        // 作用域补丁：**每个被触碰的节点算一个叶片**（`applied` 是「落地了几个叶片」
        // 的口径，作用域节点与叶子同权）。
        let touched = usize::from(sv.global.is_some())
            + sv.factions.len()
            + sv.bodies.len()
            + sv.cities.len();
        state.scope.overlay(sv);
        report.applied += touched;
    }
    report
}

/// Apply a single structural building patch: add / remove / modify a building.
///
/// 返回 `true` = 这条补丁确实改到了状态；`false` = 被丢弃（并在 `report` 里留了
/// 一笔）。`buildings` 是整份控制面里**静默失败点最密**的一处（未知 kind / 未知
/// structure / 不是本势力的城 / 下标对不上 / 对非建造区设 ship_type…），所以每个
/// `return` 都换成了带路径与原因的 `skip`。
fn apply_building_patch(
    state: &mut State,
    config: &GameConfig,
    fid: FactionId,
    patch: &BuildingPatch,
    path: &str,
    report: &mut ApplyReport,
) -> bool {
    let Some(cid) = patch.city.clone() else {
        report.skip(format!("{path}.city"), "", "missing_city", "建筑补丁必须指明 city（城名）。");
        return false;
    };

    if patch.building.is_none() {
        // Add a new building.
        let kind = patch.kind.clone().unwrap_or_else(|| "residential".to_string());
        let Some(spec) = config.buildings.get(&kind) else {
            let kinds: Vec<&str> = config.buildings.keys().map(String::as_str).collect();
            report.skip(
                format!("{path}.kind"),
                &kind,
                "no_such_kind",
                format!("没有建筑类型「{kind}」（可选：{}）。", kinds.join(" / ")),
            );
            return false;
        };
        // City identity = its unique name; the new building only lands in a city
        // owned by this faction.
        match state.city(&cid) {
            None => {
                report.skip(
                    format!("{path}.city"),
                    &cid,
                    "no_such_city",
                    format!("没有名为「{cid}」的城。"),
                );
                return false;
            }
            Some(c) if c.faction_id != fid => {
                report.skip(
                    format!("{path}.city"),
                    &cid,
                    "not_your_city",
                    format!("「{cid}」属于 {}，不是 {fid} 的城——新建建筑只能落在自己的城里。", c.faction_id),
                );
                return false;
            }
            Some(_) => {}
        }
        let structure = patch.structure.clone().unwrap_or_else(|| "concrete".to_string());
        if !config.structures.contains_key(&structure) {
            let all: Vec<&str> = config.structures.keys().map(String::as_str).collect();
            report.skip(
                format!("{path}.structure"),
                &structure,
                "no_such_structure",
                format!("没有结构「{structure}」（可选：{}）。", all.join(" / ")),
            );
            return false;
        }
        let area = patch.area.unwrap_or(4.0).max(0.0);
        let id = state
            .cities
            .iter()
            .flat_map(|c| c.buildings.iter().map(|b| b.id))
            .max()
            .map_or(0, |m| m + 1);
        let resource = if kind == "mining" { patch.resource.clone() } else { None };
        let ship_type = if kind == "construction" { patch.ship_type.clone().or_else(|| Some("corvette".to_string())) } else { None };
        // **设计图指针**（新建建造区可以一步挂上图）：非建造区 ⇒ `not_a_shipyard`；
        // 库里没有 ⇒ `no_such_blueprint`；舰级对不上 ⇒ `blueprint_class_mismatch`。
        let mut blueprint: Option<BlueprintId> = None;
        if let Some(want) = &patch.blueprint {
            if let Some(name) = want {
                if kind != "construction" {
                    report.skip(
                        format!("{path}.blueprint"),
                        name,
                        "not_a_shipyard",
                        format!("新建的是「{kind}」而不是建造区（kind=construction），只有建造区能挂设计图。"),
                    );
                    return false;
                }
                let bp_class = state
                    .control(fid.clone())
                    .and_then(|c| c.blueprints.get(name))
                    .map(|l| l.value.class.clone());
                match bp_class {
                    None => {
                        report.skip(
                            format!("{path}.blueprint"),
                            name,
                            "no_such_blueprint",
                            format!("{fid} 的设计图库里没有「{name}」——新建造区要么不挂图（走 `ship_type` + 生成器），要么挂一张已经存在的图。"),
                        );
                        return false;
                    }
                    Some(bp_class) if ship_type.as_deref() != Some(bp_class.as_str()) => {
                        report.skip(
                            format!("{path}.blueprint"),
                            name,
                            "blueprint_class_mismatch",
                            format!(
                                "「{name}」是 {bp_class} 级的图，而这个新建造区产的是 {}：口径 A 下二者必须相等（同一份补丁里写上 `ship_type`）。",
                                ship_type.clone().unwrap_or_else(|| "（没有舰级）".to_string())
                            ),
                        );
                        return false;
                    }
                    Some(_) => blueprint = Some(name.clone()),
                }
            }
        }
        let b = Building {
            id,
            kind: kind.clone(),
            resource,
            ship_type,
            blueprint,
            structure: structure.clone(),
            area,
            deployed: 0.0,
            armor: 0.0,
        };
        if let Some(city) = state.city_mut(&cid) {
            city.buildings.push(b);
        }
        let ctrl = state.control.entry(fid.clone()).or_default();
        ctrl.invest_weights.insert((cid.clone(), id), Control::player(spec.default_invest_weight));
        if kind == "construction" {
            ctrl.build_weights.insert((cid.clone(), id), Control::player(spec.default_build_weight));
        }
        return true;
    }

    let bid = patch.building.unwrap_or(u32::MAX);
    if patch.remove {
        let owned = state.city(&cid).map(|c| c.faction_id.as_str()) == Some(fid.as_str());
        if !owned {
            report.skip(
                format!("{path}.city"),
                &cid,
                "not_your_city",
                format!("「{cid}」不是 {fid} 的城（或不存在）——不能删别家的建筑。"),
            );
            return false;
        }
        if state.city(&cid).is_some_and(|c| !c.buildings.iter().any(|b| b.id == bid)) {
            report.skip(
                format!("{path}.building"),
                bid.to_string(),
                "no_such_building",
                format!("「{cid}」里没有 building={bid}，没有可删的东西。"),
            );
            return false;
        }
        if let Some(city) = state.city_mut(&cid) {
            city.buildings.retain(|b| b.id != bid);
        }
        if let Some(c) = state.control_mut(fid.clone()) {
            c.invest_weights.remove(&(cid.clone(), bid));
            c.build_weights.remove(&(cid.clone(), bid));
        }
        return true;
    }

    // Modify an existing building's attributes (e.g. structure / ship_type).
    if state.city(&cid).map(|c| c.faction_id.as_str()) != Some(fid.as_str()) {
        report.skip(
            format!("{path}.city"),
            &cid,
            "not_your_city",
            format!("「{cid}」不是 {fid} 的城（或不存在）。"),
        );
        return false;
    }
    let Some(target) = state.city(&cid).and_then(|c| c.buildings.iter().find(|b| b.id == bid)) else {
        report.skip(
            format!("{path}.building"),
            bid.to_string(),
            "no_such_building",
            format!("「{cid}」里没有 building={bid}（下标只在城内部唯一，换城要换下标）。"),
        );
        return false;
    };
    let is_shipyard = target.is_shipyard();
    let target_kind = target.kind.clone();
    let cur_ship_type = target.ship_type.clone();
    let cur_blueprint = target.blueprint.clone();
    // 口径 A 的反向守卫可能会拒掉 `ship_type` 这一笔（见下）——那时**不许**写进去。
    let mut ship_type_blocked = false;
    // **设计图指针**的校验（写面三层：缺席 = 不动 / `null` = 拆掉指针 / 名字 = 指过去）。
    // 校验先于写入：借不到第二遍状态（下面的可变块已经把城借走了）。
    let mut next_blueprint: Option<Option<BlueprintId>> = None;
    if let Some(want) = &patch.blueprint {
        if !is_shipyard {
            report.skip(
                format!("{path}.blueprint"),
                want.clone().unwrap_or_default(),
                "not_a_shipyard",
                format!("「{cid}」的 building={bid} 不是建造区（kind={target_kind}），只有建造区能挂设计图（图决定这个区把「还不存在的舰」造成什么样）。"),
            );
        } else {
            match want {
                // `null` = 拆掉指针 ⇒ 回到 `ship_type` + `choose_loadout`（旧路径）。
                None => next_blueprint = Some(None),
                Some(name) => {
                    let bp_class = state
                        .control(fid.clone())
                        .and_then(|c| c.blueprints.get(name))
                        .map(|l| l.value.class.clone());
                    match bp_class {
                        None => report.skip(
                            format!("{path}.blueprint"),
                            name,
                            "no_such_blueprint",
                            format!("{fid} 的设计图库里没有「{name}」。**绝不静默回落生成器**：引用不存在的图会让这个建造区**停产**（进度不再增加）——想回到自动选装就写 `\"blueprint\": null`。"),
                        ),
                        // 口径 A：图的 class 必须与该区的 ship_type 相等（同补丁里写了
                        // `ship_type` 就按那个新值比，否则按现值）。
                        Some(bp_class) => {
                            let target_st = patch.ship_type.clone().or_else(|| cur_ship_type.clone());
                            if target_st.as_deref() != Some(bp_class.as_str()) {
                                report.skip(
                                    format!("{path}.blueprint"),
                                    name,
                                    "blueprint_class_mismatch",
                                    format!(
                                        "「{name}」是 {bp_class} 级的图，而这个建造区产的是 {}：口径 A 下二者必须相等（把 `ship_type` 也一起写，或换一张对得上舰级的图）。",
                                        target_st.unwrap_or_else(|| "（没有舰级）".to_string())
                                    ),
                                );
                            } else {
                                next_blueprint = Some(Some(name.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    // **反方向**的守卫（spec §9.3）：改 `ship_type` 时，若这个区挂着一张 class 对不上的图
    // 而图不在同一份 diff 里跟着改，那就是把玩家的图**间接作废**（图与区对不上 ⇒ 以后
    // 只会报 mismatch）。响亮报出来，并给两条出路。
    if let Some(st) = &patch.ship_type {
        if is_shipyard {
            if let Some(bp) = &cur_blueprint {
                if let Some(bp_class) = state
                    .control(fid.clone())
                    .and_then(|c| c.blueprints.get(bp))
                    .map(|l| l.value.class.clone())
                {
                    // 蓝图表在本份 diff 里已经先落地了：若那边也改了 `class`，这里读到的
                    // 就是**新** class，于是不会误报。
                    if bp_class != *st {
                        ship_type_blocked = true;
                        report.skip(
                            format!("{path}.ship_type"),
                            st,
                            "blueprint_class_mismatch",
                            format!("这个建造区挂着设计图「{bp}」（{bp_class} 级），而你要把它改成 {st} 级。要么同一份 diff 里把图的 `class` 也改成 {st}（`blueprints[].class`），要么先拆掉指针（`\"blueprint\": null`）——留着对不上的图等于把它作废。"),
                        );
                    }
                }
            }
        }
    }
    let mut touched = false;
    if let Some(city) = state.city_mut(&cid) {
        if let Some(b) = city.buildings.iter_mut().find(|b| b.id == bid) {
            if let Some(s) = &patch.structure {
                if config.structures.contains_key(s) {
                    b.structure = s.clone();
                    touched = true;
                } else {
                    let all: Vec<&str> = config.structures.keys().map(String::as_str).collect();
                    report.skip(
                        format!("{path}.structure"),
                        s,
                        "no_such_structure",
                        format!("没有结构「{s}」（可选：{}）。", all.join(" / ")),
                    );
                }
            }
            if let Some(s) = &patch.ship_type {
                if !is_shipyard {
                    report.skip(
                        format!("{path}.ship_type"),
                        s,
                        "not_a_shipyard",
                        format!("「{cid}」的 building={bid} 不是建造区（kind={target_kind}），只有建造区能定 ship_type（决定该区造哪一级舰）。"),
                    );
                } else if !ship_type_blocked {
                    b.ship_type = Some(s.clone());
                    touched = true;
                }
                // `ship_type_blocked` ⇒ 上面那条 `blueprint_class_mismatch` 已经报了，
                // **这一笔不许落地**（否则「响亮报错 + 悄悄改掉」= 最难查的一种）。
            }
            if let Some(k) = &patch.kind {
                if config.buildings.contains_key(k) {
                    b.kind = k.clone();
                    touched = true;
                } else {
                    let kinds: Vec<&str> = config.buildings.keys().map(String::as_str).collect();
                    report.skip(
                        format!("{path}.kind"),
                        k,
                        "no_such_kind",
                        format!("没有建筑类型「{k}」（可选：{}）。", kinds.join(" / ")),
                    );
                }
            }
            if let Some(r) = &patch.resource {
                if b.kind == "mining" {
                    b.resource = Some(r.clone());
                    touched = true;
                } else {
                    report.skip(
                        format!("{path}.resource"),
                        r,
                        "not_a_mining_building",
                        format!("「{cid}」的 building={bid} 不是开采区（kind={}），只有开采区能定开采哪种资源。", b.kind),
                    );
                }
            }
            if let Some(a) = patch.area {
                b.area = a.max(0.0);
                touched = true;
            }
            // 设计图指针（上面已经校验过：建造区 / 图存在 / 舰级对得上）。
            if let Some(next) = &next_blueprint {
                b.blueprint = next.clone();
                touched = true;
            }
        }
    }
    touched
}

/// The tags the tagged form accepts, in the order an agent should read them.
/// Kept next to [`normalize_behavior`] so the error message can never drift from
/// the parser: **the list of legal tags is written once**.
const BEHAVIOR_TAGS: &[&str] = &["idle", "move", "follow", "dock_city", "dock", "colonize"];

/// Normalize a single ship `behavior` value so `apply` accepts BOTH shapes:
///   * the default serde enum form (`{"Follow":{"ship":"华盛顿"}}`,
///     `"Idle"`) — what the `control` template emits and what `.ron` uses; and
///   * the tagged agent-state form (`{"type":"follow","ship":"华盛顿"}`,
///     `{"type":"idle"}`) — exactly what an agent sees in a ship's `order`.
/// The latter is rewritten into the former so the rest of the pipeline stays
/// unchanged.
///
/// **An unknown tag is an error, not a pass-through.** It used to be left as-is
/// "so it fails downstream cleanly" — but the downstream failure was serde's
/// `invalid value: map, expected map with a single key`, which names neither the
/// field nor the legal alternatives, and the single most likely cause is an agent
/// following a stale manual (the removed `target_ship` / `target_settlement`
/// behaviors). So this reports the offending tag **and** what to write instead.
fn normalize_behavior(v: &mut serde_json::Value, where_: &str) -> Result<(), String> {
    let Some(ty) = v.get("type").and_then(|t| t.as_str()).map(str::to_string) else {
        return Ok(()); // already the default form (object or "Idle")
    };
    let obj = v.as_object().expect("behavior with type is an object");
    let mut inner = serde_json::Map::new();
    for (k, val) in obj {
        if k != "type" {
            inner.insert(k.clone(), val.clone());
        }
    }
    let variant = match ty.as_str() {
        "idle" => {
            *v = serde_json::Value::String("Idle".to_string());
            return Ok(());
        }
        "move" => "Move",
        "follow" => "Follow",
        "dock_city" => "DockCity",
        "dock" => "Dock",
        "colonize" => "Colonize",
        _ => {
            // 「攻击」与「守卫」曾经是行为，现在不是——这两条最常被写错，
            // 所以把替代写法直接写进错误里。
            let hint = match ty.as_str() {
                "target_ship" | "attackship" | "targetship" => {
                    "「攻击」不再是行为：敌舰进入射程会自动开火。要追袭某舰写 {\"type\":\"follow\",\"ship\":\"<敌舰名>\"}；要守卫友舰写 {\"type\":\"follow\",\"ship\":\"<友舰名>\"}。"
                }
                "target_settlement" | "bombard" | "targetsettlement" => {
                    "「轰炸」不再是行为：敌对城进入围城射程会自动轰炸。要压向某城写 {\"type\":\"dock_city\",\"city\":\"<城名>\"}。"
                }
                "guard" | "escort" | "guard_ship" => {
                    "没有 guard 这个行为：守卫友舰 = {\"type\":\"follow\",\"ship\":\"<友舰名>\"}（Follow 不主动开火，但射程内会自动接战）。"
                }
                _ => "",
            };
            return Err(format!(
                "{where_}: 行为 \"type\":\"{ty}\" 不是合法行为（合法：{}）。{hint}",
                BEHAVIOR_TAGS.join(" / ")
            ));
        }
    };
    let mut m = serde_json::Map::new();
    m.insert(variant.to_string(), serde_json::Value::Object(inner));
    *v = serde_json::Value::Object(m);
    Ok(())
}

/// Walk a control diff and normalize every ship behavior leaf
/// (`ship_orders[].behavior` **与** `default_ship_order.behavior`，见
/// [`normalize_behavior`]）。Only the apply-side JSON path; the state's `order`
/// view is untouched. Errors carry the **diff path** of the offending order so
/// the agent knows which line to fix.
fn normalize_control_diffs(value: &mut serde_json::Value) -> Result<(), String> {
    let Some(control) = value.get_mut("control").and_then(|c| c.as_array_mut()) else { return Ok(()) };
    for (fi, fac) in control.iter_mut().enumerate() {
        // 舰队默认指令也是「行为」字段，tagged 写法同样要认（手册 §4.3 的承诺对每个
        // behavior 字段都成立，否则这个字段只能用默认枚举形式写，成为暗坑）。
        if let Some(behavior) = fac
            .get_mut("default_ship_order")
            .and_then(|d| d.get_mut("behavior"))
        {
            normalize_behavior(behavior, &format!("control[{fi}].default_ship_order"))?;
        }
        // 设计图的**意图轴**也是「行为」字段（手册 §4 的例子写的是 tagged 形式）：
        // 漏掉这一处，那个字段就只能用默认枚举形式写，成为暗坑。
        if let Some(bps) = fac.get_mut("blueprints").and_then(|b| b.as_array_mut()) {
            for (bi, bp) in bps.iter_mut().enumerate() {
                let name = bp.get("name").and_then(|n| n.as_str()).unwrap_or("?").to_string();
                if let Some(order) = bp.get_mut("order") {
                    // `null` = 本图对意图没有说话（不是行为，跳过）。
                    if !order.is_null() {
                        normalize_behavior(order, &format!("control[{fi}].blueprints[{bi}] (图「{name}」)"))?;
                    }
                }
            }
        }
        let Some(orders) = fac.get_mut("ship_orders").and_then(|o| o.as_array_mut()) else { continue };
        for (oi, order) in orders.iter_mut().enumerate() {
            let ship = order.get("ship").and_then(|s| s.as_str()).unwrap_or("?").to_string();
            if let Some(behavior) = order.get_mut("behavior") {
                normalize_behavior(behavior, &format!("control[{fi}].ship_orders[{oi}] (ship 「{ship}」)"))?;
            }
        }
    }
    Ok(())
}

/// Parse a control diff file (JSON) and apply it to `state` as a structural
/// multi-level patch. Accepts the same shape as `POST /api/command`
/// (`{control:[...],scope:{...}}`). Ship behaviors may be written in either the
/// default enum form or the tagged agent-state form (see [`normalize_behavior`]).
///
/// Returns the [`ApplyReport`] (what landed / what was dropped and why) so the
/// caller can tell the agent. Callers that don't care (the web command route)
/// may ignore it.
pub fn apply_patch(state: &mut State, config: &GameConfig, value: &serde_json::Value) -> Result<ApplyReport, String> {
    let mut v = value.clone();
    normalize_control_diffs(&mut v)?;
    let req: CommandReq =
        serde_json::from_value(v).map_err(|e| format!("invalid control diff: {e}"))?;
    Ok(apply_diff(state, config, &req))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tagged agent-state `order` form must be accepted and rewritten into
    /// the default enum form that the rest of the pipeline expects.
    #[test]
    fn normalize_behavior_accepts_tagged_form() {
        let cases = [
            (serde_json::json!({"type":"idle"}), serde_json::json!("Idle")),
            (
                serde_json::json!({"type":"follow","ship":"华盛顿"}),
                serde_json::json!({"Follow":{"ship":"华盛顿"}}),
            ),
            (
                serde_json::json!({"type":"dock_city","city":"长三角"}),
                serde_json::json!({"DockCity":{"city":"长三角"}}),
            ),
            (
                serde_json::json!({"type":"move","position":[-0.5,0.3]}),
                serde_json::json!({"Move":{"position":[-0.5,0.3]}}),
            ),
            (
                serde_json::json!({"type":"colonize","body":"地球"}),
                serde_json::json!({"Colonize":{"body":"地球"}}),
            ),
        ];
        for (tagged, expected) in cases {
            let mut v = tagged.clone();
            normalize_behavior(&mut v, "test").expect("legal tag normalizes");
            assert_eq!(v, expected, "tagged input {tagged:?} must normalize to {expected:?}");
        }
    }

    /// An unknown tag must be **rejected with a self-correcting message**: the
    /// removed `target_ship` behavior is the single most likely thing an agent
    /// writes (it is what the old manual taught), and the old code let it fall
    /// through to serde's `invalid value: map, expected map with a single key`
    /// — which names neither the field nor the alternatives.
    #[test]
    fn unknown_behavior_tag_is_rejected_with_the_legal_tags_and_a_hint() {
        let mut v = serde_json::json!({"type":"target_ship","ship":"华盛顿","attack":true});
        let err = normalize_behavior(&mut v, "中国.ship_orders[0]").expect_err("removed behavior must be rejected");
        for needle in ["target_ship", "合法", "follow", "自动开火"] {
            assert!(err.contains(needle), "error must mention {needle:?}, got: {err}");
        }
        // 未知但也不像旧行为的标签同样被拒（不再静默流过）。
        let mut v = serde_json::json!({"type":"teleport"});
        assert!(normalize_behavior(&mut v, "x").is_err());
    }

    /// The default form must pass through unchanged.
    #[test]
    fn normalize_behavior_keeps_default_form() {
        let mut v = serde_json::json!({"Follow":{"ship":"华盛顿"}});
        normalize_behavior(&mut v, "test").expect("default form is untouched");
        assert_eq!(v, serde_json::json!({"Follow":{"ship":"华盛顿"}}));
    }

    /// Applying a tagged-form diff to a real world must produce the same
    /// controllable behavior as the equivalent default-form diff.
    #[test]
    fn apply_patch_accepts_tagged_ship_order() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        // 长城 = 中国 (faction 3) 的起始护卫舰；华盛顿 = 美国的一艘舰。舰名即唯一 key。
        let tagged = serde_json::json!({
            "control": [{
                "faction_id": "中国",
                "ship_orders": [{"ship": "长城", "behavior": {"type": "follow", "ship": "华盛顿"}, "mode": "Player"}]
            }]
        });
        apply_patch(&mut state, &config, &tagged).expect("tagged diff applies");
        let b = state.ship_behavior("长城".to_string()).expect("长城 has an order");
        assert_eq!(b, ShipBehavior::Follow { ship: "华盛顿".to_string() });
    }

    /// 应用一条舰风格补丁：只写给定轴、钳制到 [-1,1]、只作用于本势力自己的舰。
    ///
    /// 而且它写的是**叶片**：舰上的 `Ship.doctrine`/`Ship.kiting` 是**记录值**（出厂快照 +
    /// AI 流水），补丁不该动它——有效值走 `State::ship_doctrine`/`State::ship_kiting`。
    #[test]
    fn apply_ship_doctrine_patch() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let record_before = state.ship("长城").expect("长城 exists").doctrine;
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_doctrine": [
                    {"ship": "长城", "temper": -1.0, "lone_wolf": 3.0},
                    {"ship": "华盛顿", "temper": 1.0}
                ],
                "ship_kiting": [
                    {"ship": "长城", "kiting": -0.5},
                    {"ship": "华盛顿", "kiting": 1.0}
                ]
            }]
        });
        apply_patch(&mut state, &config, &diff).expect("doctrine patch applies");
        let d = state.ship_doctrine("长城".to_string());
        assert_eq!(d.temper, -1.0);
        assert_eq!(d.lone_wolf, 1.0, "axis must be clamped to [-1,1]");
        assert_eq!(state.ship_kiting("长城".to_string()), -0.5);
        assert_eq!(
            state.ship("长城").unwrap().doctrine,
            record_before,
            "补丁写的是叶片；舰上的 doctrine 是记录值，不该被改"
        );
        // 写值即接管：这片叶从此归玩家。
        assert_eq!(state.ship_doctrine_control("长城".to_string()), ControlMode::Player);
        assert_eq!(state.ship_kiting_control("长城".to_string()), ControlMode::Player);
        // 华盛顿 belongs to 美国, not 中国 → the 中国 patch must be a no-op.
        assert_eq!(
            state.ship_doctrine("华盛顿".to_string()).temper,
            0.0,
            "other-faction ship must be untouched"
        );
        assert_eq!(state.ship_kiting("华盛顿".to_string()), 0.0, "other-faction ship must be untouched");
    }

    /// 舰队默认**风格**（`default_doctrine` / `default_kiting`）：一片叶改全舰队、
    /// **新舰（还没有任何叶片）也自动跟随**、单舰特例仍然优先。
    ///
    /// 这就是「按舰级默认」在控制面上的正解形态：不需要引擎加一层 `BTreeMap<舰级, …>`，
    /// 「全舰队风筝、战列舰贴脸」= 一片默认叶 + 几片特例叶。
    #[test]
    fn fleet_default_style_covers_ships_without_leaves() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .map(|s| s.name.clone())
            .expect("中国 has a starting ship");

        // 1) 只写势力级默认：没有叶片的舰全部取它（写值即接管 ⇒ mode = Player）。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "default_kiting": {"kiting": -1.0}}]
        });
        apply_patch(&mut state, &config, &diff).expect("default style applies");
        assert_eq!(state.ship_kiting(ship.clone()), -1.0, "没有叶片的舰必须跟随舰队默认");
        assert_eq!(
            state.ship_kiting_control(ship.clone()),
            ControlMode::Player,
            "只写值不写 mode ⇒ 接管（与其它叶同一条规则）"
        );

        // 2) 单舰特例（更具体的层）优先。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "ship_kiting": [{"ship": ship, "kiting": 1.0, "mode": "Player"}]}]
        });
        apply_patch(&mut state, &config, &diff).expect("per-ship override applies");
        assert_eq!(state.ship_kiting(ship.clone()), 1.0, "单舰叶片比舰队默认更具体");

        // 3) 把叶片交回上层（Inherit）⇒ 又回到舰队默认的值。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "ship_kiting": [{"ship": ship, "mode": "Inherit"}]}]
        });
        apply_patch(&mut state, &config, &diff).expect("release applies");
        assert_eq!(state.ship_kiting(ship.clone()), -1.0, "叶 Inherit + 舰队默认 Player ⇒ 取默认值");
    }

    /// 舰队默认指令（`default_ship_order`）：**新舰出生就有意图**，而且**一个叶片改全舰队**。
    ///
    /// 这是 note `agent-control-long-game.md` §5 的正解：以前新下水的舰不在任何 diff 里
    /// → 默认归系统 → 玩家每段都要重新枚举活舰名（而舰名会换代）。现在归属与意图都在
    /// 更宽的那一层有答案，点名单舰只剩「例外」一种用途。
    #[test]
    fn fleet_default_order_covers_new_ships() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let fid = "中国".to_string();

        // 造一艘锚点：舰队默认是**势力级**的，所以先只对「没有任何叶片的舰」验证语义。
        // 拿一艘中国的舰、**删掉它的叶片**来模拟「刚下水、还没人点名」。
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == fid)
            .map(|s| s.name.clone())
            .expect("a chinese ship");
        state.control_mut(fid.clone()).expect("control").ship_orders.remove(&ship);
        assert_eq!(state.ship_control(ship.clone()), ControlMode::Auto, "no leaf, no default → system");
        assert_eq!(state.ship_behavior(ship.clone()), None, "no leaf, no default → no order at all");

        // 写一个舰队默认（不带 mode → 写值即接管 = Player）。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "default_ship_order": {"behavior": {"type": "dock", "body": "地球"}}
            }]
        });
        apply_patch(&mut state, &config, &diff).expect("fleet default applies");

        assert_eq!(
            state.ship_control(ship.clone()),
            ControlMode::Player,
            "a ship with no leaf inherits the faction default's ownership"
        );
        assert_eq!(
            state.ship_behavior(ship.clone()),
            Some(ShipBehavior::Dock { body: "地球".to_string() }),
            "…and its intent (this is the whole point: the new ship has orders without being named)"
        );

        // 单舰特例仍然压过舰队默认（更具体的层优先）——而且这是「改主意」的批量手段：
        // 改**一个**势力级叶片 = 全舰队改主意（B 不需要了）。
        let batch = serde_json::json!({
            "control": [{"faction_id": "中国",
                "default_ship_order": {"behavior": {"type": "idle"}, "mode": "Player"}
            }]
        });
        apply_patch(&mut state, &config, &batch).expect("fleet default retarget applies");
        assert_eq!(state.ship_behavior(ship.clone()), Some(ShipBehavior::Idle), "one leaf, whole fleet");

        let exception = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_orders": [{"ship": ship.clone(), "behavior": {"type": "colonize", "body": "火星"}, "mode": "Player"}]
            }]
        });
        apply_patch(&mut state, &config, &exception).expect("per-ship exception applies");
        assert_eq!(
            state.ship_behavior(ship.clone()),
            Some(ShipBehavior::Colonize { body: "火星".to_string() }),
            "a named ship overrides the fleet default"
        );

        // 舰队默认读面即写面：`--control` 里看得见它，且值能原样回传。
        let view = control_view(&state, &config, fid.clone(), state.control(fid.clone()).expect("control"));
        let d = view.default_ship_order.expect("the fleet default is part of the read surface");
        assert_eq!(d.mode, Some(ControlMode::Player));
        assert_eq!(d.behavior, Some(ShipBehavior::Idle));
    }

    /// 「写值即接管」：只写值、不写 mode 的 diff 必须真的生效（而不是被系统下一回合
    /// 按自己的逻辑覆盖掉），并在回执里有一条 `took_over` 记录。
    #[test]
    fn writing_a_value_without_mode_takes_over() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);

        // 不带 mode 写一条舰指令 + 一条预算。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_orders": [{"ship": "长城", "behavior": {"type": "dock", "body": "地球"}}],
                "construction_budget": [{"resource": "铁", "value": 3.5}]
            }]
        });
        let report = apply_patch(&mut state, &config, &diff).expect("diff applies");

        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Player, "a written value is an order");
        assert_eq!(
            state.ship_behavior("长城".to_string()),
            Some(ShipBehavior::Dock { body: "地球".to_string() })
        );
        assert_eq!(
            state.construction_budget_control("中国".to_string(), "铁"),
            ControlMode::Player,
            "same rule for budgets: the value would otherwise be recomputed away"
        );
        assert_eq!(report.took_over.len(), 2, "the receipt must name every implicitly taken-over leaf: {:?}", report.took_over);
        assert!(report.took_over.iter().any(|p| p.contains("ship_orders[0].behavior")), "{:?}", report.took_over);
        assert!(report.took_over.iter().any(|p| p.contains("construction_budget[0].value")), "{:?}", report.took_over);

        // 显式写 `Inherit` 仍然能把叶片交还给作用域链（这不是接管，是撤销表态）。
        let give_back = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_orders": [{"ship": "长城", "mode": "Inherit"}]
            }]
        });
        let report = apply_patch(&mut state, &config, &give_back).expect("diff applies");
        assert!(report.took_over.is_empty(), "an explicit mode is not a takeover: {:?}", report.took_over);
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Auto, "Inherit hands it back to the system");
    }

    /// Setting a faction's scope to Player must actually take over its leaves
    /// (which are inert `inherit` now), even though the AI has "written" values.
    #[test]
    fn scope_player_takes_over_independent_faction() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        // Default scope (all Inherit) → everything resolves to Auto.
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Auto, "default scope is Auto");

        // Take over faction 中国 via a scope-only diff.
        let scope_diff = serde_json::json!({"scope": {"factions": [["中国", "Player"]]}});
        apply_patch(&mut state, &config, &scope_diff).expect("scope diff applies");
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Player, "ship of a Player faction is player-owned");
        assert_eq!(state.investment_budget_control("中国".to_string(), "铁"), ControlMode::Player, "budget leaf follows scope");
        assert_eq!(state.construction_budget_control("中国".to_string(), "铁"), ControlMode::Player);
        // Other factions are untouched (still Auto): 华盛顿 is a US ship (美国).
        assert_eq!(state.ship_control("华盛顿".to_string()), ControlMode::Auto, "untouched faction stays Auto");

        // An explicit leaf mode still overrides scope in the opposite direction:
        // hand 长城 back to the system inside a Player faction.
        let leaf_diff = serde_json::json!({
            "control": [{"faction_id": "中国", "ship_orders": [{"ship": "长城", "mode": "Auto"}]}]
        });
        apply_patch(&mut state, &config, &leaf_diff).expect("leaf diff applies");
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Auto, "explicit Auto leaf beats Player scope");

        // …and an explicit `Inherit` leaf un-does that override, falling back to scope.
        let back = serde_json::json!({
            "control": [{"faction_id": "中国", "ship_orders": [{"ship": "长城", "mode": "Inherit"}]}]
        });
        apply_patch(&mut state, &config, &back).expect("inherit leaf applies");
        assert_eq!(state.ship_control("长城".to_string()), ControlMode::Player, "Inherit leaf falls back to the faction scope");
    }

    /// 三态的**旧档拼写**必须继续能读进来（`.ron` 存档里存的是旧的
    /// `Option<ControlMode>`：`None` = 没有说话、`Some(Ai)` / `Some(Player)` = 显式指定）。
    /// 这是「改名不丢档」的守卫：映射是双射，所以加载时就能完成迁移。
    #[test]
    fn legacy_mode_spellings_still_load() {
        use crate::model::ControlMode;

        // JSON 读面：null / 旧名 "Ai" / 三个新名。
        let from_json = |s: &str| -> ControlMode { serde_json::from_str(s).expect("mode must load") };
        assert_eq!(from_json("null"), ControlMode::Inherit, "null = 没有说话");
        assert_eq!(from_json("\"Ai\""), ControlMode::Auto, "Ai is the pre-rename spelling of Auto");
        assert_eq!(from_json("\"Auto\""), ControlMode::Auto);
        assert_eq!(from_json("\"Inherit\""), ControlMode::Inherit);
        assert_eq!(from_json("\"Player\""), ControlMode::Player);
        assert!(serde_json::from_str::<ControlMode>("\"玩家\"").is_err(), "unknown spellings must fail loudly");

        // RON 存档里叶子写的是 `mode: None` / `Some(Ai)` / `Some(Player)`。
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Leaf {
            #[serde(default)]
            mode: ControlMode,
        }
        let load = |s: &str| -> ControlMode { ron::from_str::<Leaf>(s).expect("legacy leaf must load").mode };
        assert_eq!(load("(mode: None)"), ControlMode::Inherit, "legacy `None` = Inherit");
        assert_eq!(load("(mode: Some(Ai))"), ControlMode::Auto, "legacy `Some(Ai)` = Auto");
        assert_eq!(load("(mode: Some(Player))"), ControlMode::Player, "legacy `Some(Player)` = Player");
        assert_eq!(load("(mode: Some(Inherit))"), ControlMode::Inherit, "new spelling, same `Some(..)` shell");
        assert_eq!(load("()"), ControlMode::Inherit, "missing field defaults to Inherit");
        // 手写 `.ron` 也必须带 `Some(..)` 外壳（RON 里 `deserialize_option` 不认裸标识符）；
        // 这是**有意的**：JSON 是写面，`.ron` 只由 `--save` 写。写错了会当场报 ExpectedOption，
        // 而不是静默当成 Inherit。
        assert!(ron::from_str::<Leaf>("(mode: Player)").is_err(), "a bare RON identifier must fail loudly");

        // 作用域树同理：整棵树（含旧的 `global: None` 与 `Some(x)` 键值）必须能读。
        let scope: crate::model::ControlScope = ron::from_str(
            r#"(global: None, factions: {"中国": Some(Player), "美国": None}, bodies: {}, cities: {})"#,
        )
        .expect("legacy scope tree must load");
        assert_eq!(scope.global, ControlMode::Inherit);
        assert_eq!(scope.factions.get("中国").copied(), Some(ControlMode::Player));
        assert_eq!(scope.factions.get("美国").copied(), Some(ControlMode::Inherit));

        // 写面（线格式）：JSON 是干净的字符串三态，RON 是与旧档同形的 `Some(标识符)`
        // —— 而且**自己写的必须读得回来**（RON 的裸标识符 vs 引号字符串在这里是坑）。
        for m in [ControlMode::Inherit, ControlMode::Auto, ControlMode::Player] {
            let json = serde_json::to_string(&Leaf { mode: m }).expect("json");
            assert_eq!(json, format!("{{\"mode\":\"{}\"}}", m.name()), "JSON read面 must be a bare three-state string");
            assert_eq!(serde_json::from_str::<Leaf>(&json).expect("json back").mode, m);

            let text = ron::to_string(&Leaf { mode: m }).expect("ron");
            assert_eq!(text, format!("(mode:Some({}))", m.name()), "RON must stay `Some(<ident>)`, same shape as legacy");
            assert_eq!(ron::from_str::<Leaf>(&text).expect("ron back").mode, m, "own checkpoint must load back: {text}");
        }
    }

    /// 娱乐/福利预算：一座城的忠诚度投入是一个可控叶子。按 Player 覆盖后，治理模型
    /// 会读取它；省略 value 时保留当前值、省略 mode 时保留当前模式。
    #[test]
    fn apply_loyalty_budget_patch() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "loyalty_budget": [{"city": "长三角", "value": 40.0, "mode": "Player"}]}]
        });
        apply_patch(&mut state, &config, &diff).expect("loyalty budget diff applies");
        assert_eq!(state.loyalty_budget_control("中国".to_string(), "长三角".to_string()), ControlMode::Player);
        let v = state
            .control("中国".to_string())
            .and_then(|c| c.loyalty_budget.get("长三角"))
            .map(|c| c.value)
            .unwrap_or(f64::NAN);
        assert!((v - 40.0).abs() < 1e-7, "loyalty budget value should be 40.0, got {v}");
    }

    // --- the apply report: "did my diff actually land?" ---------------------
    //
    // 这组守卫钉住的是**可观测性**，不是模拟：一个 diff 的叶片被丢掉时，退出码
    // 与 stdout 与成功落地完全一样，所以「有没有报出来」是 agent 唯一的信号。

    /// 一条全合法的 diff 必须报 `applied > 0` 且**没有**任何丢弃——否则
    /// `WARN_APPLY_SKIPPED` 会变成噪声，agent 学会无视它，等于没做。
    #[test]
    fn a_valid_diff_reports_clean_and_counts_every_leaf() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_orders": [{"ship": "长城", "behavior": {"type": "dock", "body": "地球"}, "mode": "Player"}],
                "loyalty_budget": [{"city": "长三角", "value": 2.0, "mode": "Player"}],
                "construction_budget": [{"resource": "铁", "value": 1.0, "mode": "Player"}]
            }]
        });
        let report = apply_patch(&mut state, &config, &diff).expect("valid diff applies");
        assert!(report.is_clean(), "a valid diff must report nothing skipped: {:?}", report.skipped);
        assert_eq!(report.applied, 3, "one leaf per order / budget / loyalty entry");
        assert_eq!(
            state.ship_behavior("长城".to_string()),
            Some(ShipBehavior::Dock { body: "地球".to_string() })
        );
    }

    /// **已战沉 / 已改名**的舰：从前是静默 no-op（退出码 0，看不出来）。现在必须
    /// 报出来，并点名到叶、给出原因码——这是 agent 发现「命令没下达」的唯一渠道。
    #[test]
    fn a_vanished_ship_is_reported_not_dropped_silently() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_orders": [{"ship": "长城2", "behavior": {"type": "idle"}, "mode": "Player"}]
            }]
        });
        let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped.len(), 1, "the dropped order must be reported: {report:?}");
        let s = &report.skipped[0];
        assert_eq!(s.code, "no_such_ship");
        assert_eq!(s.value, "长城2");
        assert!(s.path.contains("ship_orders[0]"), "path must point at the leaf: {}", s.path);
    }

    /// 别人的舰：与「查无此舰」必须分开报（一个要改名、一个是写错势力）。
    #[test]
    fn another_factions_ship_is_reported_with_its_own_code() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_orders": [{"ship": "华盛顿", "behavior": {"type": "idle"}, "mode": "Player"}]
            }]
        });
        let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped[0].code, "not_your_ship");
        assert!(report.skipped[0].reason.contains("美国"), "reason should name the real owner");
    }

    /// **幽灵势力**：一个错别字从前会在 `state.control` 里凭空造出一个势力，
    /// 它随后出现在 `--control` 模板与 web 控制面里，看起来像一个真的势力。
    /// 现在整条补丁被丢弃并报出来，世界不受污染。
    #[test]
    fn a_typo_faction_id_does_not_invent_a_phantom_faction() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let before = state.control.len();
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国洋", "construction_budget": [{"resource": "铁", "value": 9.9, "mode": "Player"}]}]
        });
        let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped[0].code, "no_such_faction");
        assert_eq!(state.control.len(), before, "a typo must not create a control entry");
        assert!(state.control.get("中国洋").is_none(), "no phantom faction");
        // 而且它绝不能出现在读面（模板）里——那正是它从前最有害的地方。
        let surface = control_surface(&state, &config);
        let ids: Vec<&str> = surface["control"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["faction_id"].as_str())
            .collect();
        assert!(!ids.contains(&"中国洋"), "phantom faction leaked into the template: {ids:?}");
    }

    /// `building` 是 u32 下标、只在城内部唯一，所以「另一座城的合法下标」在本城
    /// 是错的。这条从前静默 no-op，现在报出**该城真实的下标列表**——agent 手写
    /// 权重时最常踩的一脚。
    #[test]
    fn a_building_index_from_another_city_is_reported_with_the_real_indices() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "build_weights": [{"city": "长三角", "building": 21, "value": 0.5, "mode": "Player"}]
            }]
        });
        let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
        assert_eq!(report.applied, 0);
        assert_eq!(report.skipped[0].code, "no_such_building");
        assert!(report.skipped[0].reason.contains('0'), "reason should list the real indices: {}", report.skipped[0].reason);
    }

    /// 未知**字段名**（`ship_order` 少个 s）从前被 serde 静默忽略 → 整条意图蒸发、
    /// 退出码 0。现在当场报错并列出合法字段。
    #[test]
    fn a_misspelled_control_field_is_rejected_rather_than_ignored() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国",
                "ship_order": [{"ship": "长城", "behavior": {"type": "idle"}, "mode": "Player"}]
            }]
        });
        let err = apply_patch(&mut state, &config, &diff).expect_err("a typo'd field must be an error");
        assert!(err.contains("ship_order"), "error must name the bad field: {err}");
        assert!(err.contains("ship_orders"), "error must list the legal fields: {err}");
    }

    /// 读面回传必须仍然合法：web 的 `POST /api/command` 把**整面**
    /// `FactionControlView` 发回来，所以 `deny_unknown_fields` 不能把模板自己的
    /// 键判成非法。（读面键集 ⊆ 写面键集。）
    ///
    /// 这里刻意**逐字模仿前端**：`web/static/app.js` 拿到 `world.control` 后
    /// `structuredClone` 一份并给每个势力补 `buildings = c.buildings || []`，
    /// 发回来的就是「读面 + buildings」。少了这一步，守卫就测不到真实载荷。
    #[test]
    fn the_control_template_round_trips_back_through_apply() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let mut surface = control_surface(&state, &config);
        for fac in surface["control"].as_array_mut().expect("control is an array") {
            fac.as_object_mut().expect("faction is an object").insert("buildings".to_string(), serde_json::json!([]));
        }
        // 整面回传：应当被接受，且没有任何叶片被丢。
        let report = apply_patch(&mut state, &config, &surface).expect("the web payload must round-trip");
        assert!(
            report.is_clean(),
            "the editable template must be a valid diff ({{}}): {:?}",
            report.skipped
        );
        assert!(report.applied >= 40, "the whole template should touch many leaves, got {}", report.applied);
    }

    /// 「读面即写面、模板原样回传安全」是一条**可检查**的承诺：读面里出现的数字必须**逐位**
    /// 等于状态里存着的那个数 —— 否则"原样回传"就成了一次没人要求的写操作。
    ///
    /// 历史：这里曾把所有数值四舍五入到 2 位小数（为了 token 干净），于是 `0.7131` 显示成
    /// `0.71`、回传后**真的**变成 `0.71`。这条守卫就是那次教训的化身：三个"舍入会改变它"的值，
    /// 落在三种不同的叶上（势力级默认风格 / 逐舰风格 / 资源预算）。
    #[test]
    fn the_control_template_never_rounds_a_leaf_value() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let fid = "中国".to_string();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == fid)
            .map(|s| s.name.clone())
            .expect("中国至少有一艘舰");
        let noisy = 0.7131_f64;
        let diff = serde_json::json!({
            "control": [{
                "faction_id": fid,
                // 两轴一片叶：这片叶还不存在，必须两条轴一起给（否则 `partial_doctrine_leaf` 拒绝）。
                "default_doctrine": {"temper": noisy, "lone_wolf": 0.0},
                "ship_kiting": [{"ship": ship, "kiting": noisy}],
                "investment_budget": [{"resource": "铁", "value": noisy}]
            }]
        });
        apply_patch(&mut state, &config, &diff).expect("diff applies");

        let surface = control_surface(&state, &config);
        let fac = surface["control"]
            .as_array()
            .expect("control 是数组")
            .iter()
            .find(|f| f["faction_id"] == serde_json::json!(fid))
            .expect("控制面里必须有这个势力")
            .clone();
        let exact = serde_json::json!(noisy);
        assert_eq!(fac["default_doctrine"]["temper"], exact, "势力级默认风格被舍入了");
        let kite = fac["ship_kiting"]
            .as_array()
            .expect("ship_kiting 是数组")
            .iter()
            .find(|k| k["ship"] == serde_json::json!(ship))
            .expect("刚写过的那艘舰必须在读面里");
        assert_eq!(kite["kiting"], exact, "逐舰风筝距离被舍入了");
        let budget = fac["investment_budget"]
            .as_array()
            .expect("investment_budget 是数组")
            .iter()
            .find(|b| b["resource"] == serde_json::json!("铁"))
            .expect("刚写过的资源预算必须在读面里");
        assert_eq!(budget["value"], exact, "投资预算被舍入了");
        // 而且它必须就是状态里真的存着的那个数（读面 = 真值，不是"看起来像"）。
        assert_eq!(state.ship_kiting(ship), noisy);
    }

    /// **删叶**（`remove: true`）：控制叶的"存在性"本身就是一种状态——「没有叶」= 这一层
    /// 没有说话。而取值规则里「叶不存在」与「叶写着 `Inherit`」**不**等价
    /// （`leaf.map(|l| l.value).unwrap_or(record)`：叶存在就用叶里的值，与 `mode` 无关），
    /// 所以"碰过一次的风格叶"以前永远钉着那个数。这条测试把三段都钉住：
    /// ① 写叶 ⇒ 钉住；② 「恢复继承」撤**不掉**它；③ **删叶**才真的回到出厂快照。
    #[test]
    fn removing_a_leaf_returns_the_value_to_its_source() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let fid = "中国".to_string();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == fid)
            .map(|s| s.name.clone())
            .expect("中国至少有一艘舰");
        // 出厂记录值给成非零：`config/*.ron` 从来没填过 `default_doctrine`，开局记录值是 {0,0}，
        // 那样子"回到出厂快照"与"钉在 0"分不出来。
        for s in state.ships.iter_mut().filter(|s| s.faction_id == fid) {
            s.doctrine = ShipDoctrine { temper: 0.71, lone_wolf: -0.2 };
        }
        let record = state.ship(&ship).unwrap().doctrine;

        // ① 写一片逐舰风格叶：有效值 = 叶里的值，这片叶**钉住**了它。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "temper": -1.0, "lone_wolf": 0.5}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean(), "{:?}", r.skipped);
        assert_eq!(state.ship_doctrine(ship.clone()).temper, -1.0);

        // ② 「恢复继承」（只写 mode）撤不掉那个数：叶还在，取值优先用叶里的值。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "mode": "Inherit"}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean(), "{:?}", r.skipped);
        assert_eq!(
            state.ship_doctrine(ship.clone()).temper,
            -1.0,
            "叶存在就用叶里的值（哪怕它写着 Inherit）——这正是「恢复继承」不够用的原因"
        );

        // ③ 删叶 ⇒ 有效值回到**出厂快照**，而且这片叶真的从控制面里消失。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean(), "{:?}", r.skipped);
        assert_eq!(r.removed.len(), 1, "真的删掉了要留一条 NOTE_APPLY_REMOVED 回执：{:?}", r.removed);
        assert!(r.removed[0].contains("ship_doctrine"), "{:?}", r.removed);
        assert_eq!(state.ship_doctrine(ship.clone()), record, "删叶之后有效风格必须回到出厂快照");
        assert!(
            state.control.get(&fid).and_then(|c| c.ship_doctrine.get(&ship)).is_none(),
            "叶必须真的没了"
        );

        // ④ 幂等：再删一次不报错、不算丢弃、也不进 `removed`（目标状态已经达成）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean(), "{:?}", r.skipped);
        assert!(r.removed.is_empty(), "删一片本来就不存在的叶不进回执：{:?}", r.removed);
        assert_eq!(r.applied, 1, "但它是**成功的**（目标状态达成），不是被丢弃");

        // ⑤ `remove` 与值同时出现 ⇒ **拒绝**（任何一种静默优先级都会让人误判另一件事发生了）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true, "temper": 0.5}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert_eq!(r.skipped.len(), 1, "{:?}", r);
        assert_eq!(r.skipped[0].code, "remove_conflicts_with_value");
        assert_eq!(state.ship_doctrine(ship.clone()), record, "被拒绝的补丁一个字节都不许动");
    }

    /// 删叶的三个边角：**势力级默认叶**（删了 ⇒ 这一层不再供值）、**舰已不在**（陈叶清理）、
    /// 以及**预算/迁都**这几片同形的叶（同一套规则，不是只给风格轴开的后门）。
    #[test]
    fn removing_works_for_fleet_defaults_stale_ships_and_budgets() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let fid = "中国".to_string();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == fid)
            .map(|s| s.name.clone())
            .expect("中国至少有一艘舰");

        // 势力级默认风格：建成"玩家表态"的叶 ⇒ 叶 Inherit 的舰取它的值。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_doctrine": {"temper": 0.4, "lone_wolf": -0.6, "mode": "Player"}}]
        });
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        assert_eq!((state.ship_doctrine(ship.clone()).temper, state.ship_doctrine(ship.clone()).lone_wolf), (0.4, -0.6));

        // 删掉这片默认叶 ⇒ 这一层不再供值（回落到舰上记录值 / 作用域链）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_doctrine": {"remove": true}}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
        assert!(state.control.get(&fid).and_then(|c| c.default_doctrine.as_ref()).is_none());
        let rec = state.ship(&ship).unwrap().doctrine;
        assert_eq!(state.ship_doctrine(ship.clone()), rec, "默认叶没了 ⇒ 回落到舰上记录值");

        // 舰已不在（战沉/换代）：它的陈叶仍然能被删掉——删的是**控制面**里的叶，不要求实体还在。
        state.ships.retain(|s| s.name != ship);
        state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_doctrine
            .insert(ship.clone(), Control::inherit(ShipDoctrine { temper: 0.9, lone_wolf: 0.9 }));
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(r.is_clean(), "删陈叶不该因为舰没了而被丢弃：{:?}", r.skipped);
        assert_eq!(r.removed.len(), 1, "陈叶也是真的被删掉了：{:?}", r.removed);

        // 预算叶与迁都叶：同一套 `remove` 语义（这里只钉"删得掉"，值语义由各自的取值规则决定）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid,
                "investment_budget": [{"resource": "铁", "value": 3.0}],
                "capital": {"value": "地球", "mode": "Player"}}]
        });
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        let diff = serde_json::json!({
            "control": [{"faction_id": fid,
                "investment_budget": [{"resource": "铁", "remove": true}],
                "capital": {"remove": true}}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(r.is_clean(), "{:?}", r.skipped);
        assert_eq!(r.removed.len(), 2, "{:?}", r.removed);
        let c = state.control.get(&fid).expect("control");
        assert!(c.investment_budget.get("铁").is_none() && c.capital.is_none());
    }

    /// **第三条风格轴（角色）也守同一套删叶规矩**——并且它有一条另两条轴没有的含义：
    /// 删叶 = **交回自动定编**（`Inherit` 之下 AI 下回合可以立刻又写下结论），而不是
    /// 「从此保持某个值」（要后者得写 `Player`）。
    #[test]
    fn the_role_axis_obeys_the_same_delete_rules() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let fid = "中国".to_string();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == fid)
            .map(|s| s.name.clone())
            .expect("中国至少有一艘舰");
        // 出厂记录值给成 `true`：否则「删叶回到记录值」与「钉在 false」分不出来。
        for s in state.ships.iter_mut().filter(|s| s.faction_id == fid) {
            s.freighter = true;
        }

        // ① 逐舰角色叶（玩家钉「打仗」）：有效值 = 叶里的值，归属 = Player（AI 从此不许碰）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "freighter": false}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean(), "{:?}", r.skipped);
        assert!(!state.ship_freighter(ship.clone()));
        assert_eq!(state.ship_freighter_control(ship.clone()), ControlMode::Player);

        // ② 删叶 ⇒ 回到出厂记录值，叶真的没了，并且**交回自动定编**（归属不再是 Player）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean(), "{:?}", r.skipped);
        assert_eq!(r.removed.len(), 1, "{:?}", r.removed);
        assert!(r.removed[0].contains("ship_freighter"), "{:?}", r.removed);
        assert!(state.ship_freighter(ship.clone()), "删叶之后回落到出厂记录值 true");
        assert!(
            state.control.get(&fid).and_then(|c| c.ship_freighter.get(&ship)).is_none(),
            "叶必须真的没了"
        );
        assert_ne!(
            state.ship_freighter_control(ship.clone()),
            ControlMode::Player,
            "删叶 = 交回自动定编：AI 下回合作出的结论可以再写进这片叶"
        );

        // ③ 幂等：再删一次仍然**成功**（目标状态已达成），但不进回执、不算丢弃。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean() && r.removed.is_empty() && r.applied == 1, "{:?}", r);

        // ④ `remove` 带值 / 带归属 ⇒ 拒绝（删与写是两件事）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true, "freighter": false}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert_eq!(r.skipped.len(), 1, "{:?}", r);
        assert_eq!(r.skipped[0].code, "remove_conflicts_with_value");
        assert!(state.ship_freighter(ship.clone()), "被拒绝的补丁一个字节都不许动");

        // ⑤ 势力级默认角色叶：`Player` 时它的值压过叶片值（AI 定编的闸门）；删掉它 ⇒ 不再供值。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_freighter": {"freighter": false, "mode": "Player"}}]
        });
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        assert!(!state.ship_freighter(ship.clone()), "舰队默认是 Player ⇒ 它的值说了算");
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_freighter": {"remove": true}}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
        assert!(r.removed[0].contains("default_freighter"), "{:?}", r.removed);
        assert!(state.ship_freighter(ship.clone()), "默认叶没了 ⇒ 回落到舰上记录值 true");

        // ⑥ 陈叶（舰已不在）照删不误：与另两条轴同一条规矩。
        state.ships.retain(|s| s.name != ship);
        state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_freighter
            .insert(ship.clone(), Control::inherit(true));
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true}]}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(r.is_clean(), "删陈叶不该因为舰没了而被丢弃：{:?}", r.skipped);
        assert_eq!(r.removed.len(), 1, "{:?}", r.removed);
    }

    /// **两轴叶的"新建"必须两条轴一起给**：`default_doctrine` 只给一条轴的话，另一条会静默
    /// 变成 `0.0`（= 基线），而 `0.0` 是个正常取值——事后从读面完全看不出来全舰队的风格被改了。
    /// 叶**已存在**时单轴写仍然合法（那时"缺省 = 保留现值"是真的）。
    #[test]
    fn a_two_axis_fleet_default_must_be_created_with_both_axes() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let fid = "中国".to_string();

        // ① 叶还不存在 + 只给一条轴 ⇒ 拒绝，并**不许留下半片叶**。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_doctrine": {"temper": 0.4}}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert_eq!(r.skipped.len(), 1, "{:?}", r);
        assert_eq!(r.skipped[0].code, "partial_doctrine_leaf");
        assert!(r.skipped[0].reason.contains("两条轴"), "拒绝理由要给改法：{}", r.skipped[0].reason);
        assert!(
            state.control.get(&fid).and_then(|c| c.default_doctrine.as_ref()).is_none(),
            "被拒绝的补丁不许留下半片叶"
        );

        // ② 只写 `mode`（先表态归属）合法 —— 值那两条轴暂时都是 0.0（引擎的 `ShipDoctrine::default()`）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_doctrine": {"mode": "Player"}}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(r.is_clean() && r.took_over.is_empty(), "只写 mode 不是接管：{:?}", r.took_over);
        let leaf = state.control[&fid].default_doctrine.clone().expect("叶建出来了");
        assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.0, 0.0));

        // ③ 叶已存在 ⇒ 单轴写合法，缺省轴保留现值。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_doctrine": {"lone_wolf": -0.5}}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(r.is_clean(), "{:?}", r.skipped);
        let leaf = state.control[&fid].default_doctrine.clone().expect("叶还在");
        assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.0, -0.5));

        // ④ 删掉之后"叶不存在"这条状态又回来了 ⇒ 再单轴写还是被拒（守卫看的是存在性，不是次数）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_doctrine": {"remove": true}}]
        });
        assert!(apply_patch(&mut state, &config, &diff).unwrap().removed.len() == 1);
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "default_doctrine": {"lone_wolf": -0.5}}]
        });
        let r = apply_patch(&mut state, &config, &diff).unwrap();
        assert_eq!(r.skipped.len(), 1, "{:?}", r);
        assert_eq!(r.skipped[0].code, "partial_doctrine_leaf");
    }

    /// 逐舰**两轴叶**的单轴写：缺的那条轴种的是**这艘舰当时在用的那一条**——没有舰队默认时
    /// 正是出厂记录值（`0.71`），有玩家默认时是默认值（界面上显示的就是它）。**绝不是一个
    /// 凭空来的 `0.0`**（那正是 §3.1 那个坑的形态）。
    #[test]
    fn a_single_axis_ship_leaf_seeds_the_other_axis_from_what_is_in_use() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let fid = "中国".to_string();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == fid)
            .map(|s| s.name.clone())
            .expect("中国至少有一艘舰");
        for s in state.ships.iter_mut().filter(|s| s.faction_id == fid) {
            s.doctrine = ShipDoctrine { temper: 0.71, lone_wolf: -0.2 };
        }

        // 没有舰队默认 ⇒ 缺省轴 = 出厂记录值。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "temper": 0.5}]}]
        });
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        let leaf = state.control[&fid].ship_doctrine[&ship].clone();
        assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.5, -0.2), "缺省轴要种出厂记录值，不是 0.0");

        // 舰队默认是玩家表态 ⇒ 缺省轴 = **当时在用的那个数**（界面上显示的就是它）。
        let diff = serde_json::json!({
            "control": [{"faction_id": fid,
                "ship_doctrine": [{"ship": ship, "remove": true}],
                "default_doctrine": {"temper": 0.1, "lone_wolf": 0.9, "mode": "Player"}}]
        });
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        let diff = serde_json::json!({
            "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "temper": 0.5}]}]
        });
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        let leaf = state.control[&fid].ship_doctrine[&ship].clone();
        assert_eq!(
            (leaf.value.temper, leaf.value.lone_wolf),
            (0.5, 0.9),
            "船正在跟随舰队默认 ⇒ 另一条轴种的是默认值（UI 上显示的数）"
        );
    }

    // ---- 舰船设计图（blueprint）的写面/读面契约 -------------------------------

    /// 某势力第一座城的某个建造区：`(城名, 建筑下标, 舰级, 是不是建造区)`。
    fn some_building(state: &State, fid: &str, shipyard: bool) -> (CityId, BuildingId, String) {
        for c in state.cities.iter().filter(|c| c.faction_id == fid) {
            for b in &c.buildings {
                if b.is_shipyard() == shipyard {
                    return (c.name.clone(), b.id, b.kind.clone());
                }
            }
        }
        panic!("{fid} 没有 {} 的建筑", if shipyard { "建造区" } else { "非建造区" });
    }

    /// 建一张图 + 把它挂到某个建造区上（两个写面动作合并成一份 diff：这是正解用法）。
    fn pin_blueprint(
        state: &mut State,
        config: &GameConfig,
        name: &str,
        class: &str,
        components: &serde_json::Value,
        mode: &str,
    ) -> (CityId, BuildingId) {
        let (cid, bid, _) = some_building(state, "中国", true);
        let diff = serde_json::json!({"control": [{"faction_id": "中国",
            "blueprints": [{"name": name, "class": class, "components": components, "mode": mode}],
            "buildings": [{"city": cid, "building": bid, "ship_type": class, "blueprint": name}],
        }]});
        let rep = apply_patch(state, config, &diff).expect("建图 + 挂图必须一次成功");
        assert!(rep.is_clean(), "正解用法不该被丢弃：{:?}", rep.skipped);
        (cid, bid)
    }

    /// 设计图写面的**每一个丢弃码**都点名到叶（§7.3-9 / §4.6）。
    #[test]
    fn blueprint_patch_reports_every_skip_code() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let (cid, bid, _) = some_building(&state, "中国", true);
        let (rcid, rbid, _) = some_building(&state, "中国", false);
        let skip = |rep: &ApplyReport, code: &str| {
            let hit = rep
                .skipped
                .iter()
                .find(|s| s.code == code)
                .unwrap_or_else(|| panic!("要报 {code}，实际 {:?}", rep.skipped));
            assert!(hit.path.contains("blueprint"), "{code} 要点名到叶，got {}", hit.path);
            assert!(!hit.reason.is_empty(), "{code} 要有一句人读的理由");
        };

        // ① 建造区指向一张**不存在**的图（悬空指针）。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
            {"city": cid, "building": bid, "blueprint": "没有这张图"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "no_such_blueprint");
        assert_eq!(
            state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().blueprint,
            None,
            "被丢弃的指针不许落地"
        );

        // ② 图名不存在 + 只写 mode ⇒ 同样 `no_such_blueprint`（不许凭空造图）。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "没有这张图", "mode": "Auto"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "no_such_blueprint");
        assert!(
            !state.control["中国"].blueprints.contains_key("没有这张图"),
            "错别字不许造出一张谁都不认识的图（防幽灵图）"
        );

        // ③ 舰级对不上：图是 cruiser，建造区是 corvette。
        let (cid2, bid2, st2) = some_building(&state, "中国", true);
        let diff = serde_json::json!({"control": [{"faction_id": "中国",
            "blueprints": [{"name": "巡洋图", "class": "cruiser", "components": [], "mode": "Player"}],
            "buildings": [{"city": cid2, "building": bid2, "blueprint": "巡洋图"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        // 建图本身落地了（`applied`），只有指针被丢。
        skip(&rep, "blueprint_class_mismatch");
        assert!(st2.is_empty() || state.control["中国"].blueprints.contains_key("巡洋图"));
        assert_eq!(
            state.city(&cid2).unwrap().buildings.iter().find(|b| b.id == bid2).unwrap().blueprint,
            None,
            "对不上的指针不许落地（口径 A）"
        );

        // ④ 不存在的组件。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "坏图", "class": "corvette", "components": ["没有这个组件"], "mode": "Player"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "no_such_component");
        assert!(!state.control["中国"].blueprints.contains_key("坏图"), "被拒的图不许污染库");

        // ⑤ 组件重复。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "双炮图", "class": "corvette", "components": ["kinetic", "kinetic"], "mode": "Player"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "duplicate_component");
        assert!(!state.control["中国"].blueprints.contains_key("双炮图"));

        // ⑥ 超过槽位（corvette 只有 2 个槽）。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "超载图", "class": "corvette",
             "components": ["kinetic", "ion_drive", "shield"], "mode": "Player"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "too_many_components");
        assert!(!state.control["中国"].blueprints.contains_key("超载图"));

        // ⑦ 建图没给舰级 / 舰级不存在。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "无级图", "components": ["kinetic"], "mode": "Player"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "missing_class");
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "怪级图", "class": "无畏舰", "components": [], "mode": "Player"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "no_such_class");

        // ⑧ 把图挂到**非建造区**上。
        let diff = serde_json::json!({"control": [{"faction_id": "中国",
            "blueprints": [{"name": "民用图", "class": "corvette", "components": [], "mode": "Player"}],
            "buildings": [{"city": rcid, "building": rbid, "blueprint": "民用图"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        skip(&rep, "not_a_shipyard");
    }

    /// **写值即接管**：只写 `components` ⇒ 图叶变 `Player` 并记一笔（§7.3-11）。
    #[test]
    fn writing_a_blueprint_value_without_mode_takes_over() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "我的图", "class": "corvette", "components": ["kinetic", "ion_drive"]}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(rep.is_clean(), "{:?}", rep.skipped);
        let leaf = state.control["中国"].blueprints["我的图"].clone();
        assert_eq!(leaf.mode, ControlMode::Player, "只写值 ⇒ 这一层接管（免得「我写了图却没生效」）");
        assert!(rep.took_over.iter().any(|p| p.contains("blueprints[0]")), "{:?}", rep.took_over);
        assert_eq!(state.blueprint_control(&"中国".to_string(), &"我的图".to_string()), ControlMode::Player);

        // 只写 `mode` 合法（值不动）——交回系统重估。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "我的图", "mode": "Auto"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(rep.is_clean(), "{:?}", rep.skipped);
        let leaf = state.control["中国"].blueprints["我的图"].clone();
        assert_eq!(leaf.mode, ControlMode::Auto);
        assert_eq!(leaf.value.components, vec!["kinetic".to_string(), "ion_drive".to_string()], "值不动");
    }

    /// **读面即写面**：图库非空时，整面模板回传仍然合法，`ship_count` 这种只读列不许炸写面。
    #[test]
    fn the_blueprint_template_round_trips_back_through_apply() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        pin_blueprint(
            &mut state,
            &config,
            "护卫-守家",
            "corvette",
            &serde_json::json!(["kinetic", "ion_drive"]),
            "Player",
        );
        // 给它造一艘舰，让 `ship_count` 有个非零值（读面附加列）。
        let pos = state.body_position("地球");
        let bp = "护卫-守家".to_string();
        crate::sim::spawn_ship(&mut state, &config, crate::sim::ShipSpawn {
            owner: "中国".to_string(),
            class: "corvette",
            position: pos,
            city: None,
            via: SpawnVia::Shipyard,
            pay_components: false,
            blueprint: Some(&bp),
        });

        let mut surface = control_surface(&state, &config);
        let row = surface["control"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["faction_id"] == "中国")
            .and_then(|f| f["blueprints"].as_array())
            .and_then(|b| b.first())
            .cloned()
            .expect("读面必须给出蓝图片");
        assert_eq!(row["components"], serde_json::json!(["kinetic", "ion_drive"]), "选装要**全量**输出（少输出 = 回传时清空）");
        assert_eq!(row["ship_count"], serde_json::json!(1), "读面附加：本图造了多少艘");
        assert_eq!(row["launch_waiting"], serde_json::json!(false), "读面附加：这张图此刻没人在等钱");
        assert!(row["order"].is_null(), "本图对意图没有说话 ⇒ null（不是缺字段）");

        for fac in surface["control"].as_array_mut().unwrap() {
            fac.as_object_mut().unwrap().insert("buildings".to_string(), serde_json::json!([]));
        }
        let rep = apply_patch(&mut state, &config, &surface).expect("模板回传必须合法");
        assert!(rep.is_clean(), "模板回传不许丢叶：{:?}", rep.skipped);
        assert!(rep.applied >= 40, "整面模板要触碰很多叶，got {}", rep.applied);
        let leaf = state.control["中国"].blueprints["护卫-守家"].clone();
        assert_eq!(leaf.mode, ControlMode::Player);
        assert_eq!(leaf.value.components.len(), 2, "回传不改变选装");
    }

    /// **悬空指针**（图被删掉之后）：apply 报 `no_such_blueprint`，读面**原样输出**指针（Q10(a)）。
    #[test]
    fn a_dangling_blueprint_pointer_is_reported_not_silently_ignored() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let (cid, bid) = pin_blueprint(
            &mut state,
            &config,
            "会被删的图",
            "corvette",
            &serde_json::json!(["kinetic", "ion_drive"]),
            "Player",
        );
        // 删掉整张图（挂它的建造区**不会**被自动改指针：那是玩家的话，引擎不替他猜）。
        let diff = serde_json::json!({"control": [{"faction_id": "中国",
            "blueprints": [{"name": "会被删的图", "remove": true}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(rep.is_clean(), "{:?}", rep.skipped);
        assert!(rep.removed.iter().any(|p| p.contains("blueprints")), "{:?}", rep.removed);
        assert!(!state.control["中国"].blueprints.contains_key("会被删的图"));

        // 读面（web/--control 的 `buildings` 是结构补丁面，指针在 state 里读）**原样**输出。
        let b = state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap();
        assert_eq!(b.blueprint.as_deref(), Some("会被删的图"), "指针原样保留（读面据此看出「这个区指着不存在的图」）");

        // 再有人写这个指针 ⇒ 响亮报出来。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
            {"city": cid, "building": bid, "blueprint": "会被删的图"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert_eq!(rep.skipped[0].code, "no_such_blueprint", "{:?}", rep.skipped);

        // 拆指针是**另一件事**（回到 `ship_type` + 生成器）：`null` 与缺席必须分得开。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
            {"city": cid, "building": bid, "blueprint": null}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(rep.is_clean(), "{:?}", rep.skipped);
        assert_eq!(
            state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().blueprint,
            None,
            "`\"blueprint\": null` = 拆掉指针（缺席才是「不动」）"
        );
        assert_ne!(rep.applied, 0, "拆指针是一次落地");
    }

    /// 口径 A 的**双向守卫**：图与建造区的舰级要一起写；只写一处必须**响亮**被拒（§9.3）。
    #[test]
    fn blueprint_and_yard_class_must_be_changed_together() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let (cid, bid) = pin_blueprint(
            &mut state,
            &config,
            "护卫图",
            "corvette",
            &serde_json::json!(["kinetic", "ion_drive"]),
            "Player",
        );

        // ① 只改图 ⇒ 拒绝（否则这张图对不上它自己的建造区）。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "护卫图", "class": "cruiser"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert_eq!(rep.skipped[0].code, "blueprint_class_mismatch", "{:?}", rep.skipped);
        assert_eq!(state.control["中国"].blueprints["护卫图"].value.class, "corvette", "被拒 ⇒ 状态不动");

        // ② 只改建造区 ⇒ 同样拒绝（另一条路，堵一条没用）。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
            {"city": cid, "building": bid, "ship_type": "cruiser"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert_eq!(rep.skipped[0].code, "blueprint_class_mismatch", "{:?}", rep.skipped);
        assert_eq!(
            state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().ship_type.as_deref(),
            Some("corvette"),
            "被拒 ⇒ 建造区不动"
        );

        // ③ **两处一起写** ⇒ 一次成功（正解）。
        let diff = serde_json::json!({"control": [{"faction_id": "中国",
            "blueprints": [{"name": "护卫图", "class": "cruiser", "components": []}],
            "buildings": [{"city": cid, "building": bid, "ship_type": "cruiser"}]}]});
        let rep = apply_patch(&mut state, &config, &diff).unwrap();
        assert!(rep.is_clean(), "两处一起写必须一次成功：{:?}", rep.skipped);
        assert_eq!(state.control["中国"].blueprints["护卫图"].value.class, "cruiser");
        assert_eq!(
            state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().ship_type.as_deref(),
            Some("cruiser")
        );
    }

    /// 「让图的**意图轴**沉默」（`order: null`）与「删掉这张图」（`remove: true`）是两件事。
    #[test]
    fn silencing_the_order_axis_is_not_deleting_the_blueprint() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        let (cid, bid) = pin_blueprint(
            &mut state,
            &config,
            "护卫-守家",
            "corvette",
            &serde_json::json!(["kinetic", "ion_drive"]),
            "Player",
        );
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "护卫-守家", "order": {"type": "dock", "body": "地球"}}]}]});
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        assert_eq!(
            state.control["中国"].blueprints["护卫-守家"].value.order,
            Some(ShipBehavior::Dock { body: "地球".to_string() }),
            "tagged 写法的意图要被认下来（与 default_ship_order.behavior 同一套）"
        );

        // 清空意图轴：图还在、指针还在，只是这一层不再说话。
        let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
            {"name": "护卫-守家", "order": null}]}]});
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        assert_eq!(state.control["中国"].blueprints["护卫-守家"].value.order, None, "意图轴沉默");
        assert!(state.control["中国"].blueprints.contains_key("护卫-守家"), "图还在");
        assert_eq!(
            state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().blueprint.as_deref(),
            Some("护卫-守家"),
            "指针还在（= 选装仍按图装配，只有意图那一层交还给下层）"
        );
    }

    /// 图的**有效归属**沿 scope 链上溯（图叶 → 势力 → 全局）：势力设成 `Player` 也能接管。
    #[test]
    fn blueprint_ownership_follows_the_scope_chain() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
        state.control.entry("中国".to_string()).or_default().blueprints.insert(
            "种子图".to_string(),
            Control::inherit(Blueprint { class: "corvette".to_string(), components: vec![], order: None }),
        );
        let fid = "中国".to_string();
        assert_eq!(state.blueprint_control(&fid, &"种子图".to_string()), ControlMode::Auto, "全链继承 ⇒ Auto");
        let diff = serde_json::json!({"scope": {"factions": [["中国", "Player"]]}});
        assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
        assert_eq!(
            state.blueprint_control(&fid, &"种子图".to_string()),
            ControlMode::Player,
            "势力的 scope 表态也要能接管设计图（与其它叶同一条链的语义）"
        );
    }
}
