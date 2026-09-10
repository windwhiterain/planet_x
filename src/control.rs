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
#[derive(Serialize, Deserialize, Clone)]
pub struct ShipDoctrineEntry {
    pub ship: ShipId,
    pub temper: f64,
    pub lone_wolf: f64,
}

/// 一艘舰的风筝<->贴脸姿态（普通舰船控制属性，per-舰）：[-1,1]，0 = 基线。它是软属性——
/// Move/Follow/Dock/Idle 皆为软目标，附近有敌舰时自动按它软移动（玩家也不能硬控制）。
#[derive(Serialize, Deserialize, Clone)]
pub struct ShipKitingEntry {
    pub ship: ShipId,
    pub kiting: f64,
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

#[derive(Serialize, Deserialize, Clone)]
pub struct FactionControlView {
    pub faction_id: FactionId,
    pub capital: Option<Control<BodyId>>,
    pub ship_orders: Vec<ShipOrderEntry>,
    pub ship_doctrine: Vec<ShipDoctrineEntry>,
    pub ship_kiting: Vec<ShipKitingEntry>,
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

/// 一艘舰的指令补丁：`behavior` 用它替换该舰行为；`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipOrderPatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    /// 新行为（Idle/Move/Follow/DockCity/Dock/Colonize）。缺省 = 保留现值。
    #[serde(default)]
    pub behavior: Option<ShipBehavior>,
    /// 由谁决定：Inherit（继承，撤销本层的表态）/ Auto（系统自动）/ Player（玩家）。
    /// 缺省 = 保留现值。
    #[serde(default)]
    pub mode: Option<ControlMode>,
}

/// 一艘舰的行为风格补丁（per-舰 可配置）：覆盖 `ship` 的某条轴；缺省轴保留现值。
/// 每条轴会被钳制到 [-1,1]（技能就是在这个区间里取值的）。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipDoctrinePatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    #[serde(default)]
    pub temper: Option<f64>,
    #[serde(default)]
    pub lone_wolf: Option<f64>,
}

/// 一艘舰的风筝<->贴脸姿态补丁（普通舰船控制属性，per-舰）：覆盖 `ship` 的 `kiting`；
/// 被钳制到 [-1,1]。缺省 = 保留现值。
#[derive(Deserialize, Default, JsonSchema)]
pub struct ShipKitingPatch {
    /// 目标舰（唯一名 identity）。
    pub ship: ShipId,
    #[serde(default)]
    pub kiting: Option<f64>,
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
}

/// 某城「娱乐/福利预算」补丁：`value` 替换预算额，`mode` 指定由谁决定。
#[derive(Deserialize, Default, JsonSchema)]
pub struct LoyaltyBudgetPatch {
    pub city: CityId,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub mode: Option<ControlMode>,
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
    /// For a new or modified building: structure key (concrete | steel).
    #[serde(default)]
    pub structure: Option<String>,
    /// For a new building: planned area.
    #[serde(default)]
    pub area: Option<f64>,
    /// Remove the referenced building.
    #[serde(default)]
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
    /// 本势力各舰的指令补丁。
    #[serde(default)]
    pub ship_orders: Vec<ShipOrderPatch>,
    /// 本势力各舰的行为风格补丁（per-舰 可配置）。
    #[serde(default)]
    pub ship_doctrine: Vec<ShipDoctrinePatch>,
    /// 本势力各舰的风筝<->贴脸姿态补丁（per-舰 普通控制属性）。
    #[serde(default)]
    pub ship_kiting: Vec<ShipKitingPatch>,
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
/// 丢了哪些（[`SkippedLeaf`]）。CLI 在 stderr 上以 `WARN_APPLY_SKIPPED` 报出
/// 丢弃项（stdout 必须保持零噪声的状态流）；web 的 `POST /api/command` 整面
/// 回传，丢弃是预期内的，故刻意忽略。
#[derive(Debug, Default, Clone, Serialize, JsonSchema)]
pub struct ApplyReport {
    /// 成功落到状态上的叶片数（一个 `ship_orders[]` 条目 / 一条预算 / 一次迁都… 算一个）。
    pub applied: usize,
    /// 没落地的叶片，附带为什么。
    pub skipped: Vec<SkippedLeaf>,
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

    /// 是否一切都落地了。
    pub fn is_clean(&self) -> bool {
        self.skipped.is_empty()
    }
}

// --- read builders ----------------------------------------------------------

pub fn control_view(state: &State, fid: FactionId, c: &ControllableState) -> FactionControlView {
    let ship_orders = c
        .ship_orders
        .iter()
        .map(|(sid, ctrl)| ShipOrderEntry { ship: sid.clone(), behavior: ctrl.value.clone(), mode: ctrl.mode })
        .collect();
    // 读面：本势力每艘舰当前的行为风格（per-舰 可配置）。
    let ship_doctrine = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ShipDoctrineEntry {
            ship: s.name.clone(),
            temper: s.doctrine.temper,
            lone_wolf: s.doctrine.lone_wolf,
        })
        .collect();
    let ship_kiting = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ShipKitingEntry { ship: s.name.clone(), kiting: s.kiting })
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
    FactionControlView {
        faction_id: fid,
        capital: c.capital.clone(),
        ship_orders,
        ship_doctrine,
        ship_kiting,
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

/// Round to 2 decimals (token-noise reduction, matching the agent output).
fn r2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn round_behavior(b: ShipBehavior) -> ShipBehavior {
    match b {
        ShipBehavior::Move { position } => ShipBehavior::Move {
            position: [r2(position[0]), r2(position[1])],
        },
        other => other,
    }
}

/// Round every numeric field of a control view so the agent template has no
/// float noise. Only used by the agent `control` command; web `state_view`
/// keeps raw values.
fn round_view(v: FactionControlView) -> FactionControlView {
    FactionControlView {
        faction_id: v.faction_id,
        capital: v.capital,
        ship_orders: v
            .ship_orders
            .into_iter()
            .map(|o| ShipOrderEntry { ship: o.ship, behavior: round_behavior(o.behavior), mode: o.mode })
            .collect(),
        ship_doctrine: v
            .ship_doctrine
            .into_iter()
            .map(|d| ShipDoctrineEntry {
                ship: d.ship,
                temper: r2(d.temper),
                lone_wolf: r2(d.lone_wolf),
            })
            .collect(),
        ship_kiting: v
            .ship_kiting
            .into_iter()
            .map(|k| ShipKitingEntry { ship: k.ship, kiting: r2(k.kiting) })
            .collect(),
        investment_budget: v
            .investment_budget
            .into_iter()
            .map(|b| BudgetEntry { resource: b.resource, value: r2(b.value), mode: b.mode })
            .collect(),
        construction_budget: v
            .construction_budget
            .into_iter()
            .map(|b| BudgetEntry { resource: b.resource, value: r2(b.value), mode: b.mode })
            .collect(),
        invest_weights: v
            .invest_weights
            .into_iter()
            .map(|i| InvestWeightEntry { value: r2(i.value), ..i })
            .collect(),
        build_weights: v
            .build_weights
            .into_iter()
            .map(|i| BuildWeightEntry { value: r2(i.value), ..i })
            .collect(),
        loyalty_budget: v
            .loyalty_budget
            .into_iter()
            .map(|l| LoyaltyBudgetEntry { value: r2(l.value), ..l })
            .collect(),
    }
}

/// Render the current editable control surface (control + scope) as JSON —
/// the template an agent edits and posts back as a diff. Values are rounded to
/// 2 decimals so the template is clean for an LLM.
pub fn control_surface(state: &State) -> serde_json::Value {
    let control = state
        .control
        .iter()
        .map(|(fid, c)| round_view(control_view(state, fid.clone(), c)))
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

/// 校验一对 `(city, building)`：两个名字/下标都必须在**同一座城**里对得上。
/// `building` 是 u32 下标（在它所属城内部唯一，见 `.agents/notes/name-as-unique-key.md`
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
        for (i, sp) in fac.ship_orders.iter().enumerate() {
            // Resolve the ship by its **name** (the unique key): an order only
            // applies to a ship that exists and that this faction actually owns,
            // so ordering another faction's ship (or a vanished one) is a no-op.
            let path = format!("{fid}.ship_orders[{i}].ship");
            let Some(ship_name) = resolve_own_ship(state, &fid, &sp.ship, &path, &mut report) else {
                continue;
            };
            let ctrl = state
                .control
                .entry(fid.clone())
                .or_default()
                .ship_orders
                .entry(ship_name)
                .or_insert_with(|| Control {
                    value: sp.behavior.clone().unwrap_or(ShipBehavior::Idle),
                    mode: sp.mode.unwrap_or_default(),
                });
            if let Some(v) = &sp.behavior {
                ctrl.value = v.clone();
            }
            if let Some(m) = sp.mode {
                ctrl.mode = m;
            }
            report.applied += 1;
        }
        // 行为风格补丁：只作用于本势力确实拥有的舰；每条轴钳制到 [-1,1]。
        for (i, d) in fac.ship_doctrine.iter().enumerate() {
            let path = format!("{fid}.ship_doctrine[{i}].ship");
            if resolve_own_ship(state, &fid, &d.ship, &path, &mut report).is_none() {
                continue;
            }
            let ship = state.ships.iter_mut().find(|s| s.name == d.ship).expect("just resolved");
            if let Some(v) = d.temper {
                ship.doctrine.temper = v.clamp(-1.0, 1.0);
            }
            if let Some(v) = d.lone_wolf {
                ship.doctrine.lone_wolf = v.clamp(-1.0, 1.0);
            }
            report.applied += 1;
        }
        // 风筝<->贴脸姿态补丁：普通舰船控制属性，只作用于本势力确实拥有的舰；钳制到 [-1,1]。
        for (i, k) in fac.ship_kiting.iter().enumerate() {
            let path = format!("{fid}.ship_kiting[{i}].ship");
            if resolve_own_ship(state, &fid, &k.ship, &path, &mut report).is_none() {
                continue;
            }
            let ship = state.ships.iter_mut().find(|s| s.name == k.ship).expect("just resolved");
            if let Some(v) = k.kiting {
                ship.kiting = v.clamp(-1.0, 1.0);
            }
            report.applied += 1;
        }
        for (i, bp) in fac.investment_budget.iter().enumerate() {
            if !config.resources.contains_key(&bp.resource) {
                report.skip(
                    format!("{fid}.investment_budget[{i}].resource"),
                    &bp.resource,
                    "no_such_resource",
                    format!("没有资源 key「{}」（WYSIWYG：状态里的 key 就是 diff 里的 key，用 --meta 的 resources 看全表）。", bp.resource),
                );
                continue;
            }
            let ctrl = state.control.entry(fid.clone()).or_default().investment_budget.entry(bp.resource.clone()).or_insert_with(|| Control {
                value: bp.value.unwrap_or(0.0),
                mode: bp.mode.unwrap_or_default(),
            });
            if let Some(v) = bp.value {
                ctrl.value = v;
            }
            if let Some(m) = bp.mode {
                ctrl.mode = m;
            }
            report.applied += 1;
        }
        for (i, bp) in fac.construction_budget.iter().enumerate() {
            if !config.resources.contains_key(&bp.resource) {
                report.skip(
                    format!("{fid}.construction_budget[{i}].resource"),
                    &bp.resource,
                    "no_such_resource",
                    format!("没有资源 key「{}」（WYSIWYG：状态里的 key 就是 diff 里的 key，用 --meta 的 resources 看全表）。", bp.resource),
                );
                continue;
            }
            let ctrl = state.control.entry(fid.clone()).or_default().construction_budget.entry(bp.resource.clone()).or_insert_with(|| Control {
                value: bp.value.unwrap_or(0.0),
                mode: bp.mode.unwrap_or_default(),
            });
            if let Some(v) = bp.value {
                ctrl.value = v;
            }
            if let Some(m) = bp.mode {
                ctrl.mode = m;
            }
            report.applied += 1;
        }
        for (i, ip) in fac.invest_weights.iter().enumerate() {
            if !check_city_building(state, &fid, &ip.city, ip.building, &format!("{fid}.invest_weights[{i}]"), &mut report) {
                continue;
            }
            let key = (ip.city.clone(), ip.building);
            let ctrl = state.control.entry(fid.clone()).or_default().invest_weights.entry(key).or_insert_with(|| Control {
                value: ip.value.unwrap_or(0.0),
                mode: ip.mode.unwrap_or_default(),
            });
            if let Some(v) = ip.value {
                ctrl.value = v;
            }
            if let Some(m) = ip.mode {
                ctrl.mode = m;
            }
            report.applied += 1;
        }
        for (i, bp) in fac.build_weights.iter().enumerate() {
            if !check_city_building(state, &fid, &bp.city, bp.building, &format!("{fid}.build_weights[{i}]"), &mut report) {
                continue;
            }
            let key = (bp.city.clone(), bp.building);
            let ctrl = state.control.entry(fid.clone()).or_default().build_weights.entry(key).or_insert_with(|| Control {
                value: bp.value.unwrap_or(0.0),
                mode: bp.mode.unwrap_or_default(),
            });
            if let Some(v) = bp.value {
                ctrl.value = v;
            }
            if let Some(m) = bp.mode {
                ctrl.mode = m;
            }
            report.applied += 1;
        }
        for (i, lp) in fac.loyalty_budget.iter().enumerate() {
            match state.city(&lp.city) {
                None => {
                    report.skip(
                        format!("{fid}.loyalty_budget[{i}].city"),
                        &lp.city,
                        "no_such_city",
                        format!("没有名为「{}」的城（城被夷平后名字会从活城列表里消失）。", lp.city),
                    );
                    continue;
                }
                Some(city) if city.faction_id != fid => {
                    report.skip(
                        format!("{fid}.loyalty_budget[{i}].city"),
                        &lp.city,
                        "not_your_city",
                        format!("「{}」属于 {}，不是 {fid} 的城。", lp.city, city.faction_id),
                    );
                    continue;
                }
                Some(_) => {}
            }
            let ctrl = state.control.entry(fid.clone()).or_default().loyalty_budget.entry(lp.city.clone()).or_insert_with(|| Control {
                value: lp.value.unwrap_or(0.0),
                mode: lp.mode.unwrap_or_default(),
            });
            if let Some(v) = lp.value {
                ctrl.value = v;
            }
            if let Some(m) = lp.mode {
                ctrl.mode = m;
            }
            report.applied += 1;
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
            let cur = state.capital_body(&fac.faction_id);
            let new_value = match cap.value.as_ref() {
                Some(v) if state.body(v).is_none() => {
                    report.skip(
                        format!("control[{fi}].capital.value"),
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
                if let Some(v) = new_value {
                    ctrl.value = v;
                }
                if let Some(m) = cap.mode {
                    ctrl.mode = m;
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
        let b = Building {
            id,
            kind: kind.clone(),
            resource,
            ship_type,
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
                if is_shipyard {
                    b.ship_type = Some(s.clone());
                    touched = true;
                } else {
                    report.skip(
                        format!("{path}.ship_type"),
                        s,
                        "not_a_shipyard",
                        format!("「{cid}」的 building={bid} 不是建造区（kind={target_kind}），只有建造区能定 ship_type（决定该区造哪一级舰）。"),
                    );
                }
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

/// Walk a control diff and normalize every `ship_orders[].behavior` (see
/// [`normalize_behavior`]). Only the apply-side JSON path; the state's `order`
/// view is untouched. Errors carry the **diff path** of the offending order so
/// the agent knows which line to fix.
fn normalize_control_diffs(value: &mut serde_json::Value) -> Result<(), String> {
    let Some(control) = value.get_mut("control").and_then(|c| c.as_array_mut()) else { return Ok(()) };
    for (fi, fac) in control.iter_mut().enumerate() {
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

    /// Applying a ship-doctrine patch sets only the given axes on the faction's
    /// own ships, clamps each axis to [-1,1], and ignores other-faction / unknown
    /// ships (per-舰 可配置覆写).
    #[test]
    fn apply_ship_doctrine_patch() {
        let config = crate::config::load_config();
        let mut state = crate::world::default_state(&config, 42);
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
        let d = state.ship("长城").expect("长城 exists").doctrine;
        assert_eq!(d.temper, -1.0);
        assert_eq!(d.lone_wolf, 1.0, "axis must be clamped to [-1,1]");
        assert_eq!(state.ship("长城").unwrap().kiting, -0.5);
        // 华盛顿 belongs to 美国, not 中国 → the 中国 patch must be a no-op.
        let w = state.ship("华盛顿").expect("华盛顿 exists").doctrine;
        assert_eq!(w.temper, 0.0, "other-faction ship must be untouched");
        assert_eq!(state.ship("华盛顿").unwrap().kiting, 0.0, "other-faction ship must be untouched");
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
        let surface = control_surface(&state);
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
        let mut surface = control_surface(&state);
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
}
