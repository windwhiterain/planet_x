//! **承包市场**：托运方挂单、承运方接单（用户裁决 Q2）的持久状态。
//!
//! 设计见 `.agents/notes/freight-collection.md` 的 §4。裁决摘要：
//!
//! * **Q1(b)**——承运人**不赔货值**，砸单只掉信誉（用户：「赔钱太麻烦了，就用扣信誉来
//!   抵扣」）。连带后果：**信誉是唯一的抵押品**，所以托运方的信誉门槛是**主闸**（M4b）。
//! * **Q3**——信誉是**势力级**的（[`Faction::reputation`](crate::model::Faction::reputation)），
//!   没有舰船级/名船长信用：担责的是势力，派哪条船是它的内部事务。
//! * **Q10**——报酬是**抽成制**：[`Contract::share`]，承运人交付时从货里自留，
//!   余数进托运方首都池。游戏没有货币，抽成不需要新造一种；且报酬自动随货值缩放、
//!   **零递归**（自留的那份本来就在它手上）、**无瞬移**（不像「首都池付实物」那样让货跨星际闪现）。
//! * **Q11**——超期**只扣一次信誉，合同不作废、货不消失**：承运人照旧送到、照旧拿抽成。
//!
//! 本模块只管**数据**。挂单簿上只留「还在等人接」与「已有人接、正在履行」的单子；
//! **完成的单子不留在状态里**——挂单簿无限增长没有读者，而「这一单成交了」由当回合的
//! 事件（`contract_*`）与流水账记录。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, FactionId, ShipId};

/// 一条**运输承包挂单**：`shipper` 要人把 `amount` 单位的 `resource` 从 `from` 运到 `to`，
/// 承运人凭**抽成** [`Contract::share`] 取酬（交付时从货里自留）。
///
/// 难度、门槛、撮合（M4b）**不在这里**：那些是**算出来**的（航程/货值/截止期），
/// 存进状态只会多一份会与实际漂移的副本。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Contract {
    /// 单调递增的挂单号（由 [`ContractState::next_id`] 分配）——事件与投影靠它引用这一单。
    pub id: u64,
    /// 托运方（挂单的人）。
    pub shipper: FactionId,
    /// 承运方。`None` = 还在挂单簿上等人接。
    #[serde(default)]
    pub carrier: Option<FactionId>,
    /// 货物资源 key（可读中文名，见 `wysiwyg-resource-keys.md`）。
    pub resource: String,
    /// 挂单总量（单位）。
    pub amount: f64,
    /// **已交付给托运方**的量（**不含**承运人自留的抽成，见 [`Contract::carrier_cut`]）。
    #[serde(default)]
    pub delivered: f64,
    /// 起运天体（托运方的**产地货栈**——公理「首都即集散地」下，集货腿的另一端）。
    pub from: BodyId,
    /// 目的天体（照公理 = **托运方首都**）。
    pub to: BodyId,
    /// 承运人抽成比例（挂单时定死；`0.15` = 承运人自留 15%）。定死是为了**确定性**：
    /// 同一回合里所有承运人看到的是同一张价目表，不会因结算顺序而变。
    pub share: f64,
    /// 超期的信誉扣减**已经执行过**了（Q11：超期**只扣一次**，合同不作废）。
    ///
    /// 用显式标志而不是「每回合都判一次」，是因为合同在超期后**还要继续履行**（货照运、
    /// 抽成照拿）——没有这个标志，一份迟到的合同会在超期后的每个回合都再扣一次信誉。
    #[serde(default)]
    pub late_penalized: bool,
    /// 挂单回合。
    pub posted_round: u32,
    /// 截止回合（`state.round > deadline` = 超期）。超期**只扣一次信誉**，合同继续有效（Q11）。
    pub deadline: u32,
    /// 托运方的**最低信誉门槛**（M4b）。
    ///
    /// 和 [`Contract::share`] / [`Contract::deadline`] 同性质：**合同的条件在挂单那一刻
    /// 就算好并冻结**（门槛 = `gate_base + gate_slope × (难度 + 货值份数)`，用挂单时的
    /// 位置与参考速度算）。不每回合重算，是因为天体在动——重算会让同一张单的条件漂移，
    /// 而合同一旦挂出去，条件就该是固定的。
    ///
    /// 它不是硬闸：承运人的**合格度** = `σ((信誉 − 门槛) ÷ 宽度)`（见 `autocontrol::contract`），
    /// 低信誉者极少被选中，而不是数学上绝无可能。
    pub min_reputation: f64,
}

impl Contract {
    /// 还差多少没送到（单位，不含抽成）。
    pub fn outstanding(&self) -> f64 {
        (self.amount - self.delivered).max(0.0)
    }

    /// 承运人交付 `units` 单位时**自留**的抽成；其余 `units - carrier_cut(units)` 进托运方首都池。
    ///
    /// 这是 Q10 的机制落点：**没有货币转移**，承运人的报酬就是它没交出去的那一部分货。
    pub fn carrier_cut(&self, units: f64) -> f64 {
        units * self.share.clamp(0.0, 1.0)
    }

    /// 这一单是否还在挂单簿上等人接。
    pub fn is_open(&self) -> bool {
        self.carrier.is_none()
    }

    /// 这一单是否已经履行完毕（量已交足）。
    pub fn is_fulfilled(&self) -> bool {
        self.outstanding() <= 1e-9
    }
}

/// 承包市场的持久状态（进 [`State`](crate::model::State)，随存档一起持久化）。
///
/// `#[serde(default)]`（在 `State` 上）保证 v9 及更早的存档/`.ron` 缺这一节时退化成
/// 「空挂单簿」而不是加载失败——那时的世界本来就没有承包这件事，**零信息损失**。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ContractState {
    /// 挂单簿：**只留未完成的单子**（等人接的 + 正在履行的）。完成即移出。
    #[serde(default)]
    pub contracts: Vec<Contract>,
    /// 挂单号分配器（单调递增、永不复用；不复用是为了让历史事件里的单号永远指得准）。
    #[serde(default)]
    pub next_id: u64,
    /// **哪艘舰在执行哪张单**（`舰名 → 单号`，M4b）。
    ///
    /// 为什么要把这条链接显式存下来，而不是从「舰的 `Haul` 路线 == 单的 from/to」反推：
    /// 同一个承运人可以同时接**同一航线上的不同单**（甚至是同一处货栈的两种货），
    /// 光看路线分不出卸下来的货该进谁的池子、抽成按哪张单算。
    ///
    /// 它同时是「这艘舰**必须**继续当运输舰」的依据（见 `freight::should_be_freighter`）：
    /// 角色轴每回合由自动控制重写，若不认识这条链接，接到一半的单会被定编收走。
    #[serde(default)]
    pub assignments: BTreeMap<ShipId, u64>,
}

impl ContractState {
    /// 按单号取单。
    pub fn get(&self, id: u64) -> Option<&Contract> {
        self.contracts.iter().find(|c| c.id == id)
    }

    /// 挂出一张新单，返回分配到的单号（`next_id` 从 0 起，先取号再自增）。
    #[allow(clippy::too_many_arguments)]
    pub fn post(
        &mut self,
        shipper: FactionId,
        resource: String,
        amount: f64,
        from: BodyId,
        to: BodyId,
        share: f64,
        posted_round: u32,
        deadline: u32,
        min_reputation: f64,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.contracts.push(Contract {
            id,
            shipper,
            carrier: None,
            resource,
            amount,
            delivered: 0.0,
            from,
            to,
            share,
            late_penalized: false,
            posted_round,
            deadline,
            min_reputation,
        });
        id
    }

    /// 某艘舰**在执行哪张单**（`None` = 它在跑自己势力的集货路线）。
    pub fn assignment_of(&self, ship: &str) -> Option<u64> {
        self.assignments.get(ship).copied()
    }

    /// 某势力**正在执行的承包单数**（定编时要为它们留出运力）。
    pub fn carried_by(&self, carrier: &str) -> usize {
        self.contracts.iter().filter(|c| c.carrier.as_deref() == Some(carrier)).count()
    }

    /// 把某艘舰派给某张单（调用方保证这张单确实是它的）。
    pub fn assign(&mut self, ship: ShipId, contract: u64) {
        self.assignments.insert(ship, contract);
    }

    /// 解除某艘舰的执行关系（单子完成/放弃/船毁时收尾）。
    pub fn unassign(&mut self, ship: &str) -> Option<u64> {
        self.assignments.remove(ship)
    }

    /// 清掉所有指向**已不在挂单簿上**的单号的执行关系（单子完成或收回后的收尾）。
    ///
    /// 返回清掉的条目数。它保证 `assignments` 不会指向幽灵单号——那种悬挂引用会让
    /// 一艘舰永远停在一张不存在的单子上（而它的角色也被那条链接钉着）。
    pub fn drop_dangling_assignments(&mut self) -> usize {
        let live: std::collections::BTreeSet<u64> = self.contracts.iter().map(|c| c.id).collect();
        let before = self.assignments.len();
        self.assignments.retain(|_, id| live.contains(id));
        before - self.assignments.len()
    }

    /// 某人是否**已经**为「这处货栈的这种货」挂过一张还没完成的单。
    pub fn has_unfinished(&self, shipper: &str, from: &str, resource: &str) -> bool {
        self.contracts
            .iter()
            .any(|c| c.shipper == shipper && c.from == from && c.resource == resource)
    }

    /// 找一张「某人挂出、**还没人接**」的单——用于把它的数量**改成此刻的实况**。
    ///
    /// 只有在 `carrier` 为空时才改得动：一旦有人接了，数量就是**承诺**，不能再动
    /// （Q11：超期也不作废，见 [`Contract::carrier_cut`] 与模块文档）。
    pub fn open_mut(&mut self, shipper: &str, from: &str, resource: &str) -> Option<&mut Contract> {
        self.contracts
            .iter_mut()
            .find(|c| c.shipper == shipper && c.from == from && c.resource == resource && c.is_open())
    }

    /// 某人**挂出且还开着**的单子（等人接的）。
    pub fn open_by(&self, shipper: &str) -> Vec<&Contract> {
        self.contracts
            .iter()
            .filter(|c| c.shipper == shipper && c.is_open())
            .collect()
    }

    /// 某人**正在履行**的单子（已经有人接了）。
    pub fn carrying_by(&self, carrier: &str) -> Vec<&Contract> {
        self.contracts
            .iter()
            .filter(|c| c.carrier.as_deref() == Some(carrier))
            .collect()
    }

    /// 移出已完成的单子（挂单簿只留未完成的）。返回移出的张数。
    pub fn retire_fulfilled(&mut self) -> usize {
        let before = self.contracts.len();
        self.contracts.retain(|c| !c.is_fulfilled());
        before - self.contracts.len()
    }
}

// --- 条款的**物理量**（挂单时算好并冻结）---------------------------------------
//
// 这几个是「合同长什么样」的计算，放在 model 而不是 autocontrol：它们与
// `cargo_capacity` / `ship_panel` / `route_depth` 同类——**由 config 与位置算出来的量**，
// 不含任何策略。谁来挂单、谁去接单、接不接，那些才是 autocontrol 的事。
//
// §C1 原本把难度写成 `f(航程回合数 + 导航期望, 截止期, 货值, 战争)` 四合一。实现时**拆成三处**，
// 因为它们的**归属方不同**，混在一起会让「谁在承担什么风险」变得不可读：
// * **航程 + 导航** ⇒ [`difficulty`]——**承运人**视角的「这单多费劲」；
// * **货值** ⇒ [`required_reputation`]——**托运方**的风险（Q1(b)：承运人不赔货值，
//   所以押在陌生人手里的是托运方的**全部货值**，这才是门槛该盯的东西）；
// * **截止期** ⇒ `autocontrol::contract::accept_chance` 里的**按时概率**（承运人自评）；
// * **战争/拦截** ⇒ 由 Q4 的禁运在门口就挡掉了（被禁运方看一眼的资格都没有）。

/// 一项活跑完一趟的**回合数**：`ceil(往返时间) + 1`，再乘上导航的期望尝试次数。
///
/// 两个修正都来自实measured的离散性，不是保险系数：
/// 1. **回合是离散的**——`haul_step` 是「到达即行动」，但**装货那一回合不赶路**：在货栈
///    泊位上装完，下一回合才走得到对面。所以一趟至少 `ceil(往返回合数)` **再加一回合**
///    （金星↔地球只要 0.28 AU，一个往返的"纯飞行时间"是 0.56 回合，可实际跑起来是
///    **两个回合一趟**：一回合装、一回合飞到并卸下）。
/// 2. **导航是概率的**：非 master 走异常带时到位要试好几次（§1 实测深处 3–8 回合），
///    所以旅行时间要乘 `1/p`——掌握 MOND 的承运人在同一条线上期望回合数低一个数量级，
///    垄断在这里表现为**时间优势**（设计 D 的形态），而不是一道进不去的墙。
///
/// `speed ≤ 0`（动不了的舰）返回 `f64::INFINITY`：**给它算多少回合都不对**，
/// 而无穷大正好让下游的「难度/净收益」自动把它排除掉。
pub fn trip_rounds(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    from: &str,
    to: &str,
    speed: f64,
    mond_master: bool,
) -> f64 {
    if speed <= 0.0 {
        return f64::INFINITY;
    }
    let a = state.body_position(from);
    let b = state.body_position(to);
    let flight = 2.0 * crate::sim::dist(a, b) / speed;
    let depth = crate::sim::route_depth(config, a, b);
    let travel = if depth <= 0.0 || mond_master {
        flight
    } else {
        let p = crate::sim::mond_arrival_chance(config, depth).clamp(1e-3, 1.0);
        flight / p
    };
    travel.ceil().max(1.0) + 1.0
}

/// 这份活要跑**几个回合**：`趟数 × 单趟往返`，其中 `趟数 = ⌈还差的量 ÷ 一趟能装多少⌉`。
///
/// **不能只算一趟**：实测（seed 7 / 200 回合）平均单量 **47 件**，而本作最大的货舱是 **20**
/// （航母），常见舱容是 4/6 —— 也就是说**任何一张单都是多趟的活**。只按一趟估截止期，
/// 会让几乎每一张单从挂出去那一刻就注定超期（实测 35 张成交里 **21 张超期**），
/// 而承运人因此被扣到信誉归零、市场随之冻死。
///
/// `capacity` 是「谁在算」的舱容：托运方用 [`crate::model::GameConfig`] 里的
/// `freight.nominal_hold`（挂单时还不知道谁来运），承运人用自己的真实舱容。
pub fn job_rounds(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    c: &Contract,
    speed: f64,
    capacity: f64,
    mond_master: bool,
) -> f64 {
    let trips = (c.outstanding() / capacity.max(1e-9)).ceil().max(1.0);
    trips * trip_rounds(state, config, &c.from, &c.to, speed, mond_master)
}

/// 这单对某个承运人的**难度**（无量纲，1.0 = 「往返 `difficulty_rounds_ref` 个回合」那种费劲）。
///
/// 只由**航程 + 导航 + 趟数**构成：见本节的模块注解（货值进门槛、截止期进按时概率）。
pub fn difficulty(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    c: &Contract,
    speed: f64,
    capacity: f64,
    mond_master: bool,
) -> f64 {
    let r = job_rounds(state, config, c, speed, capacity, mond_master);
    if !r.is_finite() {
        return f64::INFINITY;
    }
    r / config.freight.difficulty_rounds_ref.max(1e-6)
}

/// 这单的**货值**（按市场基价折算的「市场价值」，与 `sim` 用同一把尺子）。
pub fn cargo_value(config: &crate::model::GameConfig, c: &Contract) -> f64 {
    value_of_amount(config, &c.resource, c.amount)
}

/// `amount` 单位的 `resource` 值多少「市场价值」。
pub fn value_of_amount(config: &crate::model::GameConfig, resource: &str, amount: f64) -> f64 {
    let unit = config.resources.get(resource).map(|r| r.value).unwrap_or(1.0);
    amount * unit
}

/// 托运方给这单定的**最低信誉门槛**（挂单时算好并冻进 [`Contract::min_reputation`]）。
///
/// ```text
/// 门槛 = gate_base + gate_slope × (难度 + 货值 ÷ risk_value_ref)
/// ```
///
/// 难度用**参考速度**（`config.freight.reference_speed`）算，理由与截止期完全相同：
/// 挂单时还不知道谁来接，而条件必须**在挂单那一刻定死**（天体还在动，每回合重算会让
/// 同一张单的条件漂移）。于是「慢船接远单」会真的超期——那是设计要的风险。
///
/// 参数是**条款本身**（资源/数量/起讫）而不是 `&Contract`：这个值要在**建单之前**算出来
/// 才能填进那张单（否则就成了「单子需要一个只有单子才有的字段」的循环）。
pub fn required_reputation(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    resource: &str,
    amount: f64,
    from: &str,
    to: &str,
) -> f64 {
    // 难度按「往返 × 趟数」算（挂单时假定的舱容 = `nominal_hold`）。
    let trips = (amount / config.freight.nominal_hold.max(1e-9)).ceil().max(1.0);
    let r = trip_rounds(state, config, from, to, config.freight.reference_speed, true) * trips;
    let r = if r.is_finite() { r } else { 0.0 };
    let d = r / config.freight.difficulty_rounds_ref.max(1e-6);
    let v = value_of_amount(config, resource, amount) / config.freight.risk_value_ref.max(1e-9);
    config.freight.gate_base + config.freight.gate_slope * (d + v)
}

/// 这单的**截止回合**：`宽限 + 余量 × 估算的活有多长`（用参考速度与假定舱容，理由同上）。
pub fn deadline_for(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    amount: f64,
    from: &str,
    to: &str,
) -> u32 {
    let trips = (amount / config.freight.nominal_hold.max(1e-9)).ceil().max(1.0);
    let r = trip_rounds(state, config, from, to, config.freight.reference_speed, true) * trips;
    let r = if r.is_finite() { r } else { 0.0 };
    let rounds = config.freight.deadline_base + config.freight.deadline_slack * r;
    state.round + rounds.ceil().max(1.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Contract {
        let mut cs = ContractState::default();
        cs.post(
            "中国".to_string(),
            "碳".to_string(),
            10.0,
            "金星".to_string(),
            "地球".to_string(),
            0.15,
            3,
            9,
            0.6,
        );
        cs.contracts[0].clone()
    }

    /// 单号单调递增且不复用（历史事件里的单号永远指得准）。
    #[test]
    fn contract_ids_are_allocated_monotonically() {
        let mut cs = ContractState::default();
        let a = cs.post("中国".into(), "碳".into(), 1.0, "金星".into(), "地球".into(), 0.1, 0, 5, 0.6);
        cs.contracts.clear(); // 即使单子被移出，号也不回头
        let b =
            cs.post("美国".into(), "铁".into(), 1.0, "水星".into(), "火星".into(), 0.1, 1, 6, 0.6);
        assert_eq!((a, b), (0, 1), "号必须单调递增、不复用");
    }

    /// **抽成 = 承运人的报酬**（Q10）：它交出 `units` 里的 `1-share`，自留 `share`。
    #[test]
    fn the_carrier_keeps_its_share_of_each_delivery() {
        let c = sample();
        assert!((c.carrier_cut(10.0) - 1.5).abs() < 1e-9, "15% 抽成 ⇒ 10 件里自留 1.5");
        assert!((10.0 - c.carrier_cut(10.0) - 8.5).abs() < 1e-9, "余数 8.5 进托运方首都池");
        // 抽成比例越界也要被夹住（配置写错不该让货凭空翻倍）。
        let mut wild = c.clone();
        wild.share = 3.0;
        assert!((wild.carrier_cut(10.0) - 10.0).abs() < 1e-9, "比例被夹到 1.0");
    }

    /// **一张一张来**：同一处货栈的同一种货，未完成期间不再挂第二张（否则挂单簿会被淹掉）。
    #[test]
    fn posting_is_one_contract_per_depot_and_resource_at_a_time() {
        let mut cs = ContractState::default();
        cs.post("中国".into(), "碳".into(), 5.0, "金星".into(), "地球".into(), 0.1, 0, 3, 0.6);
        assert!(cs.has_unfinished("中国", "金星", "碳"));
        assert!(!cs.has_unfinished("中国", "金星", "铁"), "别的资源各自成单");
        assert!(!cs.has_unfinished("美国", "金星", "碳"), "别人挂的单不算我的");
        // 完成 ⇒ 移出 ⇒ 下一回合可以再挂一张。
        cs.contracts[0].delivered = 5.0;
        assert_eq!(cs.retire_fulfilled(), 1);
        assert!(!cs.has_unfinished("中国", "金星", "碳"));
    }
}
