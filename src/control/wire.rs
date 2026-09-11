//! 控制的**线上形状**：`--apply` / `POST /api/command` 的 patch 类型与读面视图（Entry / Patch / View / Report）——读面即写面，两侧共用同一批结构。

use super::*;

/// `remove` 的序列化开关：**只在真的要删叶时才出现在线格式里**。
///
/// 为什么需要它：读面（`--control` / web 的 `control` 段）复用 `DefaultShipOrder` /
/// `DefaultDoctrine` / `DefaultKiting` 这几个结构体来**回显**叶片，而 `remove` 是**写面**
/// 的东西（读面表达"没有这片叶"的方式是 `null`）。不跳过的话读面里会多出一堆
/// `"remove": false`，而 kit 的 `verify` 是按字段比对读面的——那一列会立刻变成
/// 每次都出现的"假变动"。
pub fn is_false(b: &bool) -> bool {
    !*b
}

/// `null` 与「字段缺席」必须分得开（presence-aware 写面的经典需求）。
///
/// `Option<Option<T>>` 的 serde 默认实现会把 `null` 与「缺席」**都**落成外层的 `None`，
/// 于是「不改这个字段」与「把它清空」在写面上就没法区分了（`BuildingPatch.blueprint`
/// 与 `BlueprintPatch.order` 都需要这个区分）。这个 `deserialize_with` 把**出现过的**值
/// 一律包成 `Some(..)`：`"x"` ⇒ `Some(Some(x))`、`null` ⇒ `Some(None)`；缺席时才走
/// `#[serde(default)]` ⇒ 外层 `None`。
pub fn double_option<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

// --- read-side wire types (the editable control surface) --------------------

/// 一艘舰的**指令**读面条目。与三条风格轴同形——**本势力每一艘舰都有一行**。
///
/// * `behavior` = **有效值**（[`State::ship_behavior`]：叶 → 出厂图 → 舰队默认）。
///   `null` = **链上没有任何一层说话**（调用方按 `Idle` 兜底）——这正是「叶不存在」
///   那一侧；它与「叶写着 `Inherit`」在**归属**上等价、在**取值**上**不等价**
///   （叶存在就用叶里的值，与 `mode` 无关）。
/// * `mode` = **这片叶自己的表态**（没有叶 = `Inherit`）。
///
/// 于是「模板原样回传」安全，而且**读面是不动点**：
/// * 值 = 有效值 + `mode: Inherit` ⇒ 写回去以后有效值不变（叶值 = 有效值，或高层照旧供值）；
/// * `behavior: null` + `mode: Inherit` ⇒ 补丁**不会**建出一片叶（见 [`apply_ship_order`]），
///   否则「链上没人说话」会被静默变成「叶里记着 `Idle`」。
///
/// ⚠ **它以前只列「有叶的舰」**（`control_view` 遍历的是 `c.ship_orders`），于是
/// `remove: true` 删掉一片指令叶之后，这艘舰就**整行从控制树里消失**（web 上连它的风格 /
/// 角色两行也一起没了），玩家/agent 再也没法在界面上单独给它设归属。现在与风格三轴一样
/// **每舰一行**（`control-live-layers.md` §10.5 记的那个读面缺口）。
#[derive(Serialize, Deserialize, Clone)]
pub struct ShipOrderEntry {
    pub ship: ShipId,
    /// **有效指令**；`null` = 链上没有任何一层说话（引擎按 `Idle` 兜底）。
    pub behavior: Option<ShipBehavior>,
    /// 本舰指令叶**自己的**表态（没有叶片 = `Inherit`）。
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
pub struct ShipRoleEntry {
    pub ship: ShipId,
    pub role: ShipRole,
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
    /// 本图给这型舰的**行为风格**（`None` = 本图对该轴沉默）。
    pub doctrine: Option<ShipDoctrine>,
    /// 本图给这型舰的**风筝↔贴脸姿态**（`None` = 本图对该轴沉默）。
    pub kiting: Option<f64>,
    /// 本图给这型舰的**角色**（`None` = 本图对该轴沉默）。
    pub role: Option<ShipRole>,
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
    /// 舰队默认**行为风格**（势力级）：叶 Inherit 的舰取它的值。
    pub default_doctrine: Option<DefaultDoctrine>,
    /// 舰队默认**风筝<->贴脸姿态**（势力级）：与 `default_doctrine` 同形的另一片。
    pub default_kiting: Option<DefaultKiting>,
    /// 舰队默认**角色**（势力级，第三条风格轴）。
    pub default_role: Option<DefaultShipRole>,
    /// **势力级设计图库**：一行 = 一张图（厂房里「还不存在的舰」的出厂规格）。
    /// 建造区指向其中一张（`buildings[].blueprint` → 结构叶 [`BuildingPatch::blueprint`]）。
    pub blueprints: Vec<BlueprintEntry>,
    pub ship_orders: Vec<ShipOrderEntry>,
    pub ship_doctrine: Vec<ShipDoctrineEntry>,
    pub ship_kiting: Vec<ShipKitingEntry>,
    /// 本势力各舰的**角色**（有效值 + 那片叶自己的表态）。
    pub ship_role: Vec<ShipRoleEntry>,
    pub investment_budget: Vec<BudgetEntry>,
    pub construction_budget: Vec<BudgetEntry>,
    /// **势力级福利预算**（每资源一行）。
    pub welfare_budget: Vec<BudgetEntry>,
    pub invest_weights: Vec<InvestWeightEntry>,
    pub build_weights: Vec<BuildWeightEntry>,
    /// 城市**福利权重**（旧字段名 `loyalty_budget`，语义已改成权重）。
    pub loyalty_budget: Vec<LoyaltyBudgetEntry>,
    /// 逐城开发货币预算（国内市场开启时使用）。
    pub development_money: Vec<LoyaltyBudgetEntry>,
    /// 逐城建造货币预算（国内市场开启时使用）。
    pub construction_money: Vec<LoyaltyBudgetEntry>,
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

/// 舰队的默认**倾向**三片（`DefaultDoctrine` / `DefaultKiting` / `DefaultShipRole`）与逐舰叶片
/// 共用同一套「读面即写面」形状。
///
/// ⚠ **没有"舰队默认指令"那一片了**（2026-10 删除，用户裁决）：指令是**即时操作**，
/// 只写逐舰叶（[`ShipOrderPatch`]）。原 `DefaultShipOrder` 已删——理由见
/// [`State::ship_behavior`](crate::model::State::ship_behavior)。

/// 舰队默认**行为风格**（势力级，两片之一）：**读面即写面**。
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
pub struct DefaultShipRole {
    /// 默认角色（缺省 = 保留现值；写值即接管）。
    #[serde(default)]
    pub role: Option<ShipRole>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 保留现模式。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**（势力级这一层回到"没有说话"）。与 `role`/`mode` 同时出现 ⇒ 拒绝；
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
pub struct ShipRolePatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    #[serde(default)]
    pub role: Option<ShipRole>,
    /// 由谁决定：Inherit / Auto / Player。缺省 = 写了值就接管。
    #[serde(default)]
    pub mode: Option<ControlMode>,
    /// **删掉这片叶**：这艘舰回到"没有自己的角色" ⇒ **交回自动定编**（`Inherit` 之下 AI 下回合
    /// 可能立刻又写下它的结论——想让结论稳定就得写 `Player` 而不是删叶）。叶不存在时是幂等成功；
    /// 与 `role`/`mode` 同时出现 ⇒ 拒绝。舰已战沉也能删（删的是控制面里的叶）。
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
    /// 本图给这型舰的**行为风格**（长期倾向之一）。`null` = 本图对该轴没有说话
    /// （三层含义见 [`double_option`]：缺席 = 不动 / `null` = 清空这一层 / 给值 = 表态）。
    #[serde(default, deserialize_with = "double_option")]
    pub doctrine: Option<Option<ShipDoctrine>>,
    /// 本图给这型舰的**风筝↔贴脸姿态**（长期倾向之二）。`null` = 本图对该轴没有说话。
    #[serde(default, deserialize_with = "double_option")]
    pub kiting: Option<Option<f64>>,
    /// 本图给这型舰的**角色**（长期倾向之三，也是"新舰一造出来就干什么"的落点）。
    /// `null` = 本图对该轴没有说话。
    #[serde(default, deserialize_with = "double_option")]
    pub role: Option<Option<ShipRole>>,
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
    /// 舰队默认行为风格（势力级，两片之一）：叶 Inherit 的舰取它的值。
    #[serde(default)]
    pub default_doctrine: Option<DefaultDoctrine>,
    /// 舰队默认风筝<->贴脸姿态（势力级，两片之二）。
    #[serde(default)]
    pub default_kiting: Option<DefaultKiting>,
    /// 舰队默认**角色**（势力级，第三条风格轴）。
    #[serde(default)]
    pub default_role: Option<DefaultShipRole>,
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
    pub ship_role: Vec<ShipRolePatch>,
    /// 投资预算补丁（建设）。
    #[serde(default)]
    pub investment_budget: Vec<BudgetPatch>,
    /// 建造预算补丁（造舰）。
    #[serde(default)]
    pub construction_budget: Vec<BudgetPatch>,
    /// **福利预算补丁**（每资源一行）。
    #[serde(default)]
    pub welfare_budget: Vec<BudgetPatch>,
    /// 建设投资权重补丁。
    #[serde(default)]
    pub invest_weights: Vec<InvestWeightPatch>,
    /// 建造投资权重补丁。
    #[serde(default)]
    pub build_weights: Vec<BuildWeightPatch>,
    /// 城市福利权重补丁（旧名 `loyalty_budget`）。
    #[serde(default)]
    pub loyalty_budget: Vec<LoyaltyBudgetPatch>,
    /// 逐城开发货币预算补丁。
    #[serde(default)]
    pub development_money: Vec<LoyaltyBudgetPatch>,
    /// 逐城建造货币预算补丁。
    #[serde(default)]
    pub construction_money: Vec<LoyaltyBudgetPatch>,
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
    pub fn skip(
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
    pub fn took_over(&mut self, path: impl Into<String>) {
        self.took_over.push(path.into());
    }

    /// 记一条删叶（只记**真的**删掉了的；幂等删除不记）。
    pub fn removed(&mut self, path: impl Into<String>) {
        self.removed.push(path.into());
    }

    /// 是否一切都落地了。
    pub fn is_clean(&self) -> bool {
        self.skipped.is_empty()
    }
}

// --- read builders ----------------------------------------------------------
