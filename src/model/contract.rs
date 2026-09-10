//! **雇佣运力**：托运方挂单请人跑一条常驻线，承运方接下、自己派船。
//!
//! # 单子是**雇佣**，不是一票货（用户裁决）
//!
//! > 单子以雇佣船的形式，单主派发单的逻辑和派发自己运输船的逻辑一样，自己船不够了就派
//! > 雇佣单子，单子要求的是提供相应的运力（/时间），然后每个雇主会周期性对评估受雇方的
//! > 运力是否达标来反馈对受雇方的信誉，再根据信誉决定是否切换受雇方。受雇方也会根据当前
//! > 运力决定是否接受雇佣，是否提前结束雇佣。
//!
//! 于是本模块存的是**一份运力合同**：
//!
//! * [`Contract::capacity`] 是**要求的运力**（单位/回合）——不是「要搬多少件」。雇主真正想要的
//!   从来不是「把这 47 件搬完」，而是「这条线上每月有人搬走这么多」；积压是**流动的**
//!   （产地每回合都在产），所以「雇一条常驻运力」比「买一票搬运」更贴近它要的东西。
//!   口径是**一条参考船在这条线上的吞吐**（[`required_throughput`]）：雇主不知道自己会请到
//!   多大的船，但它知道「我少一条船」——把缺的那条船折成吞吐，受雇方派几条船来都行
//!   （用户：「对方派几艘船都无所谓」）。
//! * [`Contract::delivered`] 与 [`Contract::served_rounds`] 一起给出**实测吞吐**
//!   （[`Contract::throughput_ratio`]）：雇主每 [`Contract::review_round`] 考核一次，
//!   按达标率反馈信誉（`autocontrol::contract::review_contract`）。
//! * 期限是**固定期**（[`Contract::expires_round`]）：到期由雇主按信誉决定续约还是换人。
//!
//! # 被这次改写作废的旧机制
//!
//! 「一票货」形态下必需、雇佣形态下多余的东西**全部删掉**：总量（`amount`）、还差多少
//! （`outstanding`）、截止期（`deadline`）、超期标志（`late_penalized`）、趟数估算
//! （`job_rounds` / `deadline_for`）。它们的存在理由只有一个——「这张单要跑几趟才搬得完」
//! ——而雇佣形态里**没有这个量**：一条常驻线不存在「跑完」。连带地：
//!
//! * **超期**没了：没有截止期，就没有超期。承诺的兑现与否由**周期考核**回答（用户要的形态）。
//! * **丢单**没了：船沉了不关雇主的事（用户：「船沉没不管，只管统计运输量」）——运输量
//!   会说话：这个考核期交付不够，信誉自己会掉。同一件事**不该罚两次**。
//! * **货值风险**的口径变小了：旧形态押在陌生人手里的是**整单货值**（实测平均 47 件 ⇒ 门槛
//!   被抬得很高），新形态每一刻暴露的只是**一个货舱**（[`required_reputation`] 用
//!   `nominal_hold` 折算）。Q1(b)（承运人不赔货值、只掉信誉）不变。
//!
//! # 仍然成立的裁决
//!
//! * **Q1(b)**——承运人不赔货值，掉信誉就是全部代价；信誉是唯一的抵押品。
//! * **Q3 / P4**——信誉是**势力级**的**全局单值**（[`Faction::reputation`](crate::model::Faction::reputation)）：
//!   没有舰船级信用，也没有「A 对 B 的履历」——担责的是势力，派哪条船是它的内部事务。
//! * **Q10**——报酬是**抽成制**（[`Contract::share`]）：承运人交付时从货里自留，余数进托运方首都池。
//!   没有货币转移、零递归、无瞬移。
//! * **动态平衡**（用户裁决「一切数值都用动态平衡/博弈来产生」）——要求运力、考核周期、固定期
//!   全都**从航程算出来**（[`hire_terms`] / [`required_throughput`]），抽成由市场自己抬
//!   （`freight::escalate_open_contracts`），续约与解约由双方各自按自己的处境掷骰
//!   （`autocontrol::contract::settle_contracts`）。配置里只剩**口径**（要求运力 = 一条参考船、
//!   固定期 = 几个来回），没有一个是「猜出来的市场价」。
//!
//! 本模块只管**数据与条款**。挂单簿上只留「还在等人接」与「已有人接、还在雇佣期内」的合同；
//! **结束的合同不留在状态里**——簿子无限增长没有读者，「这单怎么了」由当回合的事件
//! （`contract_*`）与流水账记录。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, FactionId, ShipId};

/// 一份**运力雇佣合同**：`shipper` 请人在 `from` → `to`（= 它的首都）这条线上提供
/// [`Contract::capacity`] 单位/回合的运力，承运人凭**抽成** [`Contract::share`] 取酬。
///
/// 条款（要求运力、门槛、期限、考核周期）在**挂单那一刻算好并冻结**——天体在动，
/// 每回合重算会让同一张单的条件漂移。唯一每回合跟着现实走的是**未接单的**
/// [`Contract::capacity`]（需求信号必须反映此刻的缺口），一旦有人接下就冻结成承诺。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Contract {
    /// 单调递增的挂单号（由 [`ContractState::next_id`] 分配）——事件与投影靠它引用这一单。
    pub id: u64,
    /// 托运方（挂单的人、雇主）。
    pub shipper: FactionId,
    /// 承运方（受雇方）。`None` = 还在挂单簿上等人接。
    #[serde(default)]
    pub carrier: Option<FactionId>,
    /// **主货种**（挂单时该货栈积压最多的那种，可读中文名）。
    ///
    /// 它只用来**折算货值**（门槛与自评闸的报酬估计）与给人看，**不是**承运人的约束：
    /// 受雇的船到了货栈装的是**当时有什么**——与雇主自己的运输舰完全一样
    /// （用户：「单主派发单的逻辑和派发自己运输船的逻辑一样」）。所以同一处货栈的
    /// 几种货**共用一张单**（旧形态是一处货栈、一种货各一张）。
    pub resource: String,
    /// **要求的运力**（单位/回合），口径见 [`required_throughput`]。
    ///
    /// 未接单时它每回合被改成**此刻的缺口**（雇主自己搬不动的部分）；接下之后冻结。
    pub capacity: f64,
    /// 起运天体（雇主的**产地货栈**——公理「首都即集散地」下，集货腿的另一端）。
    pub from: BodyId,
    /// 目的天体（照公理 = **雇主首都**）。
    pub to: BodyId,
    /// 承运人抽成比例（`0.15` = 承运人自留 15%）。挂单时定死、无人接时由市场抬价
    /// （`freight::escalate_open_contracts`）、接下之后冻结——一份合同的条件不会因结算顺序而变。
    pub share: f64,
    /// 雇主愿意雇佣的**最低信誉**（挂单时算好并冻结，见 [`required_reputation`]）。
    ///
    /// 它不是硬闸：受雇方的**合格度** = `σ((信誉 − 门槛) ÷ 宽度)`（`autocontrol::contract`），
    /// 低信誉者极少被选中，而不是数学上绝无可能。
    /// 到期**续约**用的是同一个闸（「你当初是怎么被选上的，现在就按同一条线续」）——
    /// 于是不需要再为「换人」发明一个阈值。
    pub min_reputation: f64,
    /// 挂单回合（抬价的基准：叫了这么久还没人接 ⇒ 加价档）。
    pub posted_round: u32,
    /// **雇佣从哪一回合开始**（`None` = 还挂在簿上等人接）。
    ///
    /// 用显式的 `Option` 而不是哨兵值：回合 0 是一个合法回合，而「未接单」与「第 0 回合
    /// 接单」是两件不同的事（前者没有期限与进度，后者都有）。
    #[serde(default)]
    pub accepted_round: Option<u32>,
    /// **固定期的到期回合**（`accepted_round + hire_trips × 考核周期`）。到期由雇主决定续约或换人。
    #[serde(default)]
    pub expires_round: u32,
    /// **下次考核回合**。周期 = 这条线的一个往返（[`hire_terms`] 的 `interval`）。
    #[serde(default)]
    pub review_round: u32,
    /// 本雇佣期内**已交付给雇主**的量（单位，**不含**承运人自留的抽成，见 [`Contract::carrier_cut`]）。
    ///
    /// 与 [`Contract::served_rounds`] 一起就是**实测吞吐**。记的是「从雇主货栈搬走的量」
    /// （而不是雇主实收的量）：抽成是搬运费，不该从运力里扣——否则每一趟都天生差 15%，
    /// 而门槛是按「一条参考船」定的，13% 的缺口会被系统性地算成不达标。
    #[serde(default)]
    pub delivered: f64,
    /// 本雇佣期内**起运货栈有货**的回合数——考核的分子/分母的原料。
    ///
    /// 不能用「过了几个回合」代替：没货可运的回合不该算在受雇人头上（雇主自己的船刚把货栈
    /// 扫空、产地当期还没产出，都是正常事）。这一条把「运力是否达标」问成**真正能被考核的
    /// 那件事**：有活干的时候，你干得够不够。
    #[serde(default)]
    pub served_rounds: u32,
}

impl Contract {
    /// 承运人交付 `units` 单位时**自留**的抽成；其余 `units - carrier_cut(units)` 进雇主首都池。
    ///
    /// 这是 Q10 的机制落点：**没有货币转移**，承运人的报酬就是它没交出去的那一部分货。
    pub fn carrier_cut(&self, units: f64) -> f64 {
        units * self.share.clamp(0.0, 1.0)
    }

    /// 这份合同是否还在挂单簿上等人接。
    pub fn is_open(&self) -> bool {
        self.accepted_round.is_none()
    }

    /// 这份合同是否**已经在雇佣期内**（有人接了）。
    pub fn is_hired(&self) -> bool {
        self.accepted_round.is_some()
    }

    /// 合同是否已到期（`state.round >= expires_round`）。未接单的合同永远不算到期。
    pub fn is_expired(&self, round: u32) -> bool {
        self.is_hired() && round >= self.expires_round
    }

    /// **在途期的宽免**（回合）：`nominal_hold ÷ capacity`。
    ///
    /// 由条文自己推出来，不是另设一个数：要求运力的口径就是「一个考核周期搬回**一个货舱**」
    /// （`capacity × interval = nominal_hold`），所以这个比值**就是**这条线的一个往返回合数。
    ///
    /// 为什么要宽免它：受雇方接下一张单的时候人还在别处，它得先**开到**起运货栈、装上货、
    /// 再飞过去卸——第一批产出落地至少要一个来回。拿这段「在路上」的时间去判它不达标，
    /// 是罚它没有瞬移的能力（实测就是这么翻车的：第一期的达标率几乎必然是 0）。
    pub fn grace_rounds(&self, config: &crate::model::GameConfig) -> f64 {
        if self.capacity <= 1e-9 {
            return 0.0;
        }
        config.freight.nominal_hold.max(0.0) / self.capacity
    }

    /// **产出期**（回合）= 有货可运的回合数 − 在途宽免（见 [`Contract::grace_rounds`]）。
    pub fn output_rounds(&self, config: &crate::model::GameConfig) -> f64 {
        (self.served_rounds as f64 - self.grace_rounds(config)).max(0.0)
    }

    /// **实测吞吐达标率** = `实交 ÷ (要求运力 × 产出期)`。
    ///
    /// `None` = **还不到看账的时候**：账上该有的产出还不满**一个货舱**
    /// （`capacity × 产出期 < nominal_hold`，等价于「有货可运的回合还不到两个来回」）。
    /// 两个理由都指向同一个判断：
    ///
    /// * 受雇方第一批货落地要一个来回（在途宽免），再加上**一个完整产出期**才谈得上「一个
    ///   考核周期搬回一舱货」——账期不足就去判分，只会判出一个 0；
    /// * 要求运力很小的深空航线（一个来回几十回合、要求运力不到 0.2 件/回合）如果只按
    ///   「过了几个回合」算，随便搬一趟就是几十倍达标率——那不是干得好，是尺子坏了。
    ///
    /// 于是 1.0 的语义始终是**一个考核周期搬回一舱货**（`capacity × interval == nominal_hold`）：
    /// 一条参考船的水准。> 1 = 雇到了比参考船更好的运力（几条船组队也算）。
    pub fn throughput_ratio(&self, config: &crate::model::GameConfig) -> Option<f64> {
        let expected = self.capacity * self.output_rounds(config);
        if expected < config.freight.nominal_hold.max(0.0) * 0.999 {
            return None;
        }
        Some(self.delivered / expected)
    }
}

/// 雇佣市场的持久状态（进 [`State`](crate::model::State)，随存档一起持久化）。
///
/// `#[serde(default)]`（在 `State` 上）保证 v9 及更早的存档/`.ron` 缺这一节时退化成
/// 「空挂单簿」而不是加载失败——那时的世界本来就没有承包这件事，**零信息损失**。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ContractState {
    /// 挂单簿：**只留没结束的合同**（等人接的 + 还在雇佣期内的）。结束即移出。
    #[serde(default)]
    pub contracts: Vec<Contract>,
    /// 挂单号分配器（单调递增、永不复用；不复用是为了让历史事件里的单号永远指得准）。
    #[serde(default)]
    pub next_id: u64,
    /// **哪艘舰此刻在替哪张单跑**（`舰名 → 单号`）。
    ///
    /// 雇佣形态下它**不是**「押上去的船」那种承诺，而是**此刻的派工记录**，有两个用途：
    ///
    /// 1. **装卸归属**：受雇的船装的是雇主的货、卸进雇主的池子、抽成算在雇主那张单上
    ///    （见 `sim::cargo_owner`）。不去猜「路线像不像」——同一处货栈可以有两张不同雇主的单。
    /// 2. **缺口核算**：雇主靠它算「这条线上此刻已经有多少受雇运力在跑」（[`served_by`]），
    ///    才不会再挂一张重复的单。
    ///
    /// **一张单可以同时有多艘舰，也可以随时换**（用户：「对方派几艘船都无所谓」）：
    /// 派哪条船、派几条，是承运方的内部事务。所以这里是**多对一**的映射，
    /// 与旧形态「一张单一艘舰」的承诺完全不同。
    #[serde(default)]
    pub assignments: BTreeMap<ShipId, u64>,
}

impl ContractState {
    /// 按单号取单。
    pub fn get(&self, id: u64) -> Option<&Contract> {
        self.contracts.iter().find(|c| c.id == id)
    }

    /// 按单号取可变的单。
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Contract> {
        self.contracts.iter_mut().find(|c| c.id == id)
    }

    /// 挂出一张新单，返回分配到的单号（`next_id` 从 0 起，先取号再自增）。
    #[allow(clippy::too_many_arguments)]
    pub fn post(
        &mut self,
        shipper: FactionId,
        resource: String,
        capacity: f64,
        from: BodyId,
        to: BodyId,
        share: f64,
        posted_round: u32,
        min_reputation: f64,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.contracts.push(Contract {
            id,
            shipper,
            carrier: None,
            resource,
            capacity,
            from,
            to,
            share,
            min_reputation,
            posted_round,
            accepted_round: None,
            expires_round: 0,
            review_round: 0,
            delivered: 0.0,
            served_rounds: 0,
        });
        id
    }

    /// 某艘舰**此刻在替哪张单跑**（`None` = 它在跑自己势力的集货路线）。
    pub fn assignment_of(&self, ship: &str) -> Option<u64> {
        self.assignments.get(ship).copied()
    }

    /// **此刻在替某张单跑的舰**（按舰名序 ⇒ 确定性）。
    pub fn ships_of(&self, contract: u64) -> Vec<ShipId> {
        self.assignments
            .iter()
            .filter(|(_, id)| **id == contract)
            .map(|(s, _)| s.clone())
            .collect()
    }

    /// 某势力**此刻受雇在跑的合同**（按单号序）。
    pub fn carried_by(&self, carrier: &str) -> Vec<&Contract> {
        let mut v: Vec<&Contract> = self
            .contracts
            .iter()
            .filter(|c| c.carrier.as_deref() == Some(carrier))
            .collect();
        v.sort_by_key(|c| c.id); // 单号序（合同的排列已经很接近，但不保证）
        v
    }

    /// 把某艘舰派给某张单（调用方保证这张单确实是它雇主的）。
    pub fn assign(&mut self, ship: ShipId, contract: u64) {
        self.assignments.insert(ship, contract);
    }

    /// 解除某艘舰的派工关系，返回它原先在跑的单号。
    pub fn unassign(&mut self, ship: &str) -> Option<u64> {
        self.assignments.remove(ship)
    }

    /// **放掉某张单上的全部舰**（合同结束/被收回时收尾），返回被放掉的舰。
    pub fn release(&mut self, contract: u64) -> Vec<ShipId> {
        let ships: Vec<ShipId> = self.ships_of(contract);
        for s in &ships {
            self.assignments.remove(s);
        }
        ships
    }

    /// 移出一张单（合同结束）。返回是否真的移出了。
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.contracts.len();
        self.contracts.retain(|c| c.id != id);
        self.contracts.len() != before
    }

    /// 清掉所有指向**已不在挂单簿上**的单号的派工关系（合同结束后的收尾）。
    ///
    /// 返回清掉的条目数。它保证 `assignments` 不会指向幽灵单号——那种悬挂引用会让
    /// 一艘舰永远停在一张不存在的单子上（而它的角色也被那条链接钉着）。
    pub fn drop_dangling_assignments(&mut self) -> usize {
        let live: std::collections::BTreeSet<u64> = self.contracts.iter().map(|c| c.id).collect();
        let before = self.assignments.len();
        self.assignments.retain(|_, id| live.contains(id));
        before - self.assignments.len()
    }

    /// 某人挂出、**还没人接**、且起运地是 `from` 的那张单（可改成此刻的缺口）。
    ///
    /// 一处货栈**只有一张未接单**：它是**需求信号**（我这条线缺多少运力），不是报价单；
    /// 挂成一片只会让同一份缺口被反复请人。已接单的不在这里（那是承诺，改不得）。
    pub fn open_mut(&mut self, shipper: &str, from: &str) -> Option<&mut Contract> {
        self.contracts
            .iter_mut()
            .find(|c| c.shipper == shipper && c.from == from && c.is_open())
    }

    /// 某人**挂出且还开着**的单子（等人接的）。
    pub fn open_by(&self, shipper: &str) -> Vec<&Contract> {
        self.contracts
            .iter()
            .filter(|c| c.shipper == shipper && c.is_open())
            .collect()
    }
}

// --- 条款的**物理量**（挂单时算好并冻结）---------------------------------------
//
// 这几个是「合同长什么样」的计算，放在 model 而不是 autocontrol：它们与
// `cargo_capacity` / `ship_panel` / `route_depth` 同类——**由 config 与位置算出来的量**，
// 不含任何策略。谁来挂单、谁去接单、接不接，那些才是 autocontrol 的事。
//
// 雇佣形态让这一节**大幅变小**：旧形态要估「这单得跑几趟」（`job_rounds`）、
// 据此定截止期（`deadline_for`），而「一趟」这个概念在新形态里根本不存在
// ——受雇方跑的是**常驻线**，一个考核期一个来回。

/// 一项活跑完一趟的**回合数**：`ceil(往返时间) + 1`，再乘上导航的期望尝试次数。
///
/// 两个修正都来自实测的离散性，不是保险系数：
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
    mond_control: f64,
) -> f64 {
    if speed <= 0.0 {
        return f64::INFINITY;
    }
    let a = state.body_position(from);
    let b = state.body_position(to);
    let flight = 2.0 * crate::sim::dist(a, b) / speed;
    let depth = crate::sim::route_depth(config, a, b);
    let travel = if depth <= 0.0 {
        flight
    } else {
        // 深处一腿的期望回合数 = `flight ÷ 一次尝试的胜算`：掌握度越高，这条线越短。
        // 掌握度连续化之后这里**不再分「master / 非 master」两档**，而是同一条公式。
        let p = crate::sim::mond_arrival_chance(config, depth, mond_control).clamp(1e-3, 1.0);
        flight / p
    };
    travel.ceil().max(1.0) + 1.0
}

/// 这条线的**参考往返回合数**（用参考速度与 MOND 掌握者的期望尝试次数）。
///
/// 全部条款都从它派生：要求运力 = `nominal_hold ÷ 它`，考核周期 = 它，固定期 = 几个它。
/// 于是**没有一个是设计者猜出来的数**——它们全是「这条线有多远」的函数
/// （用户裁决：数值要用动态平衡产生）。
///
/// 用掌握度 `1.0` 是**刻意的**：要求运力是「一条**称职的**参考船」的水准。
/// 近地航线 `route_depth ≤ 0`，人人都是这个速度；而深处的凡人期望回合数要高一个
/// 数量级 ⇒ 它们的达标率天然上不去 ⇒ 深空雇佣单**自然只落在掌握 MOND 的人手里**。
/// 这正是设计 D（垄断表现为时间优势）在市场里的落点，不需要另写一条「只有 master 能接深单」。
pub fn lane_rounds(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    from: &str,
    to: &str,
) -> f64 {
    trip_rounds(state, config, from, to, config.freight.reference_speed, 1.0)
}

/// 一条线的**雇佣节奏**：考核周期 `interval` 与固定期 `term`（都是回合数）。
///
/// * `interval` = 参考往返 = **一个来回考核一次**：雇主按「一个来回该搬回多少货」验货，
///   这既是承运人能兑现的最小节拍，也是雇主能看见问题的最快节奏。
/// * `term` = `hire_trips` 个来回：到期雇主按信誉决定续约或换人——**固定期**（用户裁决）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HireTerms {
    /// 一个考核期（= 一个参考往返）的回合数，至少 1。
    pub interval: u32,
    /// 固定雇佣期的回合数。
    pub term: u32,
}

/// 见 [`HireTerms`]。
pub fn hire_terms(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    from: &str,
    to: &str,
) -> HireTerms {
    let r = lane_rounds(state, config, from, to);
    // 动不了的线（无穷大）也给一个合法的 1 回合周期：它只是没有意义，不该让算术炸掉。
    let interval = if r.is_finite() { r.ceil().max(1.0) as u32 } else { 1 };
    HireTerms { interval, term: interval.saturating_mul(config.freight.hire_trips.max(1)) }
}

/// 雇主给这条线**要求的运力**（单位/回合）= **一条参考船的吞吐**。
///
/// ```text
/// 要求运力 = nominal_hold ÷ 参考往返回合数
/// ```
///
/// 取这个口径有三个好处，都不是巧合：
/// 1. **与定编同尺度**：雇主的定编规则是「一处积压配一条船」（`freight::needed_freighters`），
///    所以「我这条线缺多少运力」天然就是「我还缺几条船」——雇主不需要为挂单另发明一套估算；
/// 2. **与考核严丝合缝**：`要求运力 × 考核周期 = nominal_hold`（一个来回搬回一舱货），
///    于是达标率 1.0 有**物理意义**，而不是一个调出来的分数线；
/// 3. **受雇方能自由组队**：要求是**吞吐**而不是「派一条多大的船」，所以对方派一条大船、
///    还是三条小船，都只是它自己的事（用户：「对方派几艘船都无所谓」）。
pub fn required_throughput(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    from: &str,
    to: &str,
) -> f64 {
    let r = lane_rounds(state, config, from, to);
    if !r.is_finite() || r <= 0.0 {
        return 0.0;
    }
    config.freight.nominal_hold.max(0.0) / r
}

/// 这条线对承运人的**难度**（无量纲，1.0 = 「往返 `difficulty_rounds_ref` 个回合」那种费劲）。
///
/// 只由**航程 + 导航**构成。旧形态还要乘「趟数」（一票货要跑几趟），雇佣形态里没有趟数。
pub fn difficulty(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    from: &str,
    to: &str,
) -> f64 {
    let r = lane_rounds(state, config, from, to);
    if !r.is_finite() {
        return f64::INFINITY;
    }
    r / config.freight.difficulty_rounds_ref.max(1e-6)
}

/// `amount` 单位的 `resource` 值多少「市场价值」。
pub fn value_of_amount(config: &crate::model::GameConfig, resource: &str, amount: f64) -> f64 {
    let unit = config.resources.get(resource).map(|r| r.value).unwrap_or(1.0);
    amount * unit
}

/// 雇主给这条线定的**最低信誉门槛**（挂单时算好并冻进 [`Contract::min_reputation`]）。
///
/// ```text
/// 门槛 = gate_base + gate_slope × (难度 + 一个货舱的货值 ÷ risk_value_ref)
/// ```
///
/// **两个因子，各有归属方**：
/// * **难度** ⇒ 它决定「这单多难兑现」；
/// * **货值** ⇒ 押在陌生人手里的是**雇主**的风险（Q1(b)：承运人不赔货值）。新形态下这个
///   暴露量是**一个货舱**（`nominal_hold`）而不是整单——雇主每一刻最多损失一船货，
///   不管这条线要跑多久。于是门槛不再随积压量暴涨（旧形态实测：平均单量 47 件 ⇒ 门槛高到
///   市场冻住），这是**机制改对**而不是调参调出来的。
///
/// 它是**计算**而不是 `&Contract` 的方法：这个值要在**建单之前**算出来才能填进那张单。
pub fn required_reputation(
    state: &crate::model::State,
    config: &crate::model::GameConfig,
    resource: &str,
    from: &str,
    to: &str,
) -> f64 {
    let d = difficulty(state, config, from, to);
    let d = if d.is_finite() { d } else { 0.0 };
    let exposure = value_of_amount(config, resource, config.freight.nominal_hold)
        / config.freight.risk_value_ref.max(1e-9);
    config.freight.gate_base + config.freight.gate_slope * (d + exposure)
}

#[cfg(test)]
#[path = "../tests/model/contract.rs"]
mod tests;
