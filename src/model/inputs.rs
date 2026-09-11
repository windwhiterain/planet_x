//! **输入面**：一回合里「引擎**消费**掉的东西」——掷出的随机数 + 判定时看到的输入。
//!
//! 它与 [`RoundView`](crate::model::RoundView)（**结算面**）是一对，两者的分工是**用户裁决**：
//!
//! | 面 | 装什么 | 判据 | 能不能事后重算 |
//! | --- | --- | --- | --- |
//! | `post`（[`RoundView`](crate::model::RoundView)） | 这一回合**结算出来**的：观测 + 过程量 | 世界变成了什么样 | 观测能、B1–B4 捕获的过程量**不能** |
//! | `pre`（本类型） | 这一回合**消费掉**的：随机数 + 判定输入 | **凡是与随机/输入有关的都归这里** | **不能**（骰子流已前进、判据的输入已变） |
//!
//! # 三条纪律
//!
//! 1. **判据是「可能未来与随机/输入有关」**（用户原话：「不一定要求当前的实现有关」）：
//!    一个量只要**概念上**是输入或掷骰（现在确定、将来可能引入抖动也算），就放这里——
//!    不要因为它今天恰好是确定值而塞进 `post`。
//! 2. **这里**不装观测。曾经的 `pre` 是「回合开始时的观测副本」——那是零信息量的一份重复
//!    （同一份 `state`、同一个 `observe`、空 sink 只把过程量抹成中性值 ⇒ 与**上一回合的
//!    `post`** 逐字段相同）。要读「回合开始时的世界」请读上一行的 `post`。这一条是 B5 改的。
//! 3. **只增不改语义**：新发现一类输入就往对应小节里加（`rolls` 是给 `derived_roll` 家族的
//!    通用记录，`order` / `relation_noise` 是主 `Prng` 的两处）。加字段时**不必**在
//!    `model::neutral` 里声明中性值——那些声明是给「过程量缺失」用的（缺了要补一个**具体**的
//!    缺省值），而输入面的空是**自明**的：没掷就是没有（`[]` / `{}`）。

use serde::{Deserialize, Serialize};

use crate::model::{FactionId, ShipId};

/// 一回合的**输入面**（`RoundState::pre`）：这一回合消费掉的随机数与判定输入。
///
/// 空 = 这一回合没跑（回合 0 / `--start` 载入的起点行），不是「掷出了 0」。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct RoundInputs {
    /// **C7 · 本回合的逐舰解算顺序**（`sim::step_military` 开头由**主 `Prng`** 洗出）。
    ///
    /// 它回答的是「**为什么这艘舰一炮未发就被击沉**」——互杀时，它排在击沉它的那艘舰
    /// **之后**（`hull <= 0` 的舰在循环里被跳过，目标已毁的计划直接作废）。
    /// ⚠ 它是**回合中段那一瞬**的 `state.ships`：含**这一回合稍后会被打沉/除名**的舰
    /// （死亡清扫在本步进收尾才做），也**不含**这一回合稍后才下水的舰。所以它的长度可以
    /// **大于**回合末的舰数——这不是漏记，是「洗牌就发生在那一刻」。
    pub order: Vec<ShipId>,
    /// **C13 · 本回合每对势力的关系噪声**（`sim::step_diplomacy` 里由**主 `Prng`** 掷出，
    /// `rel += rng.range_f64(-noise, noise)`）。
    ///
    /// 它回答的是「**关系为什么无端抖了一下**」——`aff`（静息亲和）与漂移率都是确定的，
    /// 唯一让关系「无缘无故」动一动的就是这个掷骰。
    /// 键是**有序**的两个势力名（`(i, j)` 的下标序，`i < j`）：同一对只出现一次。
    pub relation_noise: std::collections::BTreeMap<FactionId, std::collections::BTreeMap<FactionId, f64>>,
    /// **`derived_roll` 家族的抽签记录**（定编 / 派单 / 合同闸门 / 风格 / 蓝图 / 知识……）。
    ///
    /// 这些骰子**不消费主 `Prng`**（`(势力, 对象, 回合, 用途)` 的 FNV-1a 哈希 ⇒ 独立、可复现），
    /// 但**它比较的对象拿不回来**：候选池、权重、当时的机会值 `p` 全是那一刻的状态算出来的。
    /// 所以记录里既有**掷出的值**，也有**当时的判据**。
    ///
    /// ⚠ 同一枚骰子可能在**两处**被问到（例如 `should_be_role` 既被「挂单估运力」问、
    /// 又被「定编拍板」问）：**只在拍板处记一条**——记的是「谁做了什么决定」，不是「谁算过」。
    #[serde(default)]
    pub rolls: Vec<Roll>,
    /// **本回合定编的「分布」，而不是「样本」**（`autocontrol::freight::assign_roles` 填）。
    ///
    /// 定编由两种成分合成：**早退档**（在役承包舰的硬承诺 / 玩家表态 / 观测优先 / 运力为 0
    /// ⇒ 确定的 0 或 1）与**抽签档**（`p = (缺口 + 轮换) × 我的票 ÷ 同侧总票数` ⇒ 期望 `p`、
    /// 实得 0/1）。只记抽签（[`Self::rolls`]）时，「期望头数」要读的人**再抄一遍早退规则**
    /// 才补得齐——那正是「判据只有一处」要避免的。这里把**引擎自己算的期望**记下来：
    /// `expected + 外生（被玩家的叶钉死的）+ 未定编 == quota` 于是成了能**逐字对账**的恒等式，
    /// 而不是「跑 400 回合看均值贴不贴配额」那种抽样判据（它双向都弱：分布错了可能过、
    /// 分布对了可能红）。
    ///
    /// 空 = 这一回合没有势力需要定编（与 `rolls` 同一条口径：**没掷就是没有**）。
    #[serde(default)]
    pub role_distribution: std::collections::BTreeMap<FactionId, RoleDistribution>,
}

/// 一个势力本回合的**定编分布**（见 [`RoundInputs::role_distribution`]）。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct RoleDistribution {
    pub war: RoleShare,
    pub freight: RoleShare,
    pub observe: RoleShare,
}

/// 某一支（战舰 / 运输 / 观测）的**目标 / 期望 / 实得 / 外生**头数。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, schemars::JsonSchema)]
pub struct RoleShare {
    /// [`crate::autocontrol::freight::role_quotas`] 给的**目标头数**（设计上的分配）。
    pub quota: f64,
    /// **早退档的确定头数**（硬承诺 / 玩家表态 / 观测优先 / 运力为 0 ⇒ 不掷骰也算 1）。
    pub fixed: f64,
    /// **抽签档的 `p` 之和**（那些真的掷了骰子的舰的期望）。
    pub rolled: f64,
    /// `fixed + rolled`（引擎自己算的期望，不在别处重算）。
    pub expected: f64,
    /// 本回合**实得**的头数（早退档 1、抽签档 `roll < p` 才算 1）。
    pub actual: f64,
    /// **外生**头数：轮不到自动控制决定的活舰（被玩家的叶钉死 / 订单叶不是 Auto）
    /// —— 配额算的是全舰队的头数，所以对账时要把这一项单独摆出来，别混进 `expected`。
    pub exogenous: f64,
}

/// 一次 `derived_roll` 抽签的完整记录（掷出的值 + 当时的判据 + 结果）。
///
/// 两种用法都能表达（见各字段）：**闸门**（`value < threshold` ⇒ 走哪一支）与**加权抽签**
/// （`value × pool_total` 落在哪一段 ⇒ 选中谁）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct Roll {
    /// 用途 = `derived_roll` 的 `salt`（`"role"` / `"route"` / `"gate"` / `"accept"` / `"pick"` / …）。
    pub purpose: String,
    /// 谁在掷（势力名）。
    pub faction: FactionId,
    /// 掷骰的对象（舰名 / 合同号 / 舰级 / 天体名……）——与 `purpose` 合起来唯一确定这枚骰子。
    pub subject: String,
    /// **掷出的值** ∈ `[0, 1)`（`derived_roll` 的产物）。
    pub value: f64,
    /// **闸门阈值**：判据是 `value < threshold`（概率 `p`）。`None` = 这一枚不是闸门。
    pub threshold: Option<f64>,
    /// **抽签池的总权重**（加权抽签那一档：`value × pool_total` 落在哪一段）。
    /// `None` = 这一枚不是抽签。
    pub pool_total: Option<f64>,
    /// **结果**：闸门 ⇒ 走的那一支的可读标签（`"freight"` / `"war"` / `"accepted"` / `"refused"`…）；
    /// 抽签 ⇒ 选中的那一段（货栈天体名 / 合同号 / 设计主题）。`None` = 掷了但结果不由这枚骰子决定。
    pub picked: Option<String>,
    /// **候选池**（B5c）：加权抽签时，**参与抽签的每个候选**各自占多少权重。
    ///
    /// 为什么要有它：`pool_total` 只说了「池子多大」，说不了「**为什么是它而不是别人**」——
    /// 例如「为什么这艘运输舰去了木星而不是火星」要的正是「那两条腿各有多少货」。
    /// 闸门与幅度骰那一档为空（它们的判据是**一个数**，没有池子）。
    ///
    /// ⚠ **只记池子，不记「落选者被算了多少次」**：像集货那样「一次派单问一枚骰子」的抽签，
    /// 池子就是全部信息；而「逐候选各掷一枚」的形态在本仓库里不存在（那会造出顺序依赖）。
    #[serde(default)]
    pub pool: Vec<PoolEntry>,
}

/// 加权抽签池里的一个候选（[`Roll::pool`] 的一项）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, schemars::JsonSchema)]
pub struct PoolEntry {
    /// 候选的名字（天体名 / 势力名 / 主题名 / 舰名……）。
    pub name: String,
    /// 它的权重（= 它被抽中的概率 × `pool_total`）。
    pub weight: f64,
}

impl RoundInputs {
    /// 这一回合**什么都没掷**（回合 0 / 起点行的中性态）。
    pub fn is_empty(&self) -> bool {
        self.order.is_empty() && self.relation_noise.is_empty() && self.rolls.is_empty()
    }

    /// 记一条抽签记录（`derived_roll` 家族的统一入口）。
    pub fn push_roll(&mut self, roll: Roll) {
        self.rolls.push(roll);
    }

    /// 记一次**闸门**：判据是 `value < threshold`，`branch` 是走的那一支的可读标签。
    ///
    /// 只在**拍板处**调用（同一枚骰子可能被「估算」与「拍板」问两次——记的是决定，不是算过）。
    pub fn record_gate(
        &mut self,
        purpose: &str,
        faction: &str,
        subject: &str,
        value: f64,
        threshold: f64,
        branch: &str,
    ) -> &mut Roll {
        self.push_roll(Roll {
            purpose: purpose.to_string(),
            faction: faction.to_string(),
            subject: subject.to_string(),
            value,
            threshold: Some(threshold),
            pool_total: None,
            picked: Some(branch.to_string()),
            // 闸门的判据是**一个数**（机会值）——没有池子时留空；定编那两处由调用方补。
            pool: Vec::new(),
        });
        self.rolls.last_mut().expect("刚 push 过")
    }

    /// 记一次**加权抽签**：`value × pool_total` 落在哪一段（`picked` = 选中的那一段）。
    ///
    /// 返回刚记下的那一条：**池子**（B5c，见 [`Roll::pool`]）由调用方在下一行挂上——
    /// `inputs.record_draw(…).pool = 每条腿各有多少货;`
    pub fn record_draw(
        &mut self,
        purpose: &str,
        faction: &str,
        subject: &str,
        value: f64,
        pool_total: f64,
        picked: &str,
    ) -> &mut Roll {
        self.push_roll(Roll {
            purpose: purpose.to_string(),
            faction: faction.to_string(),
            subject: subject.to_string(),
            value,
            threshold: None,
            pool_total: Some(pool_total),
            picked: Some(picked.to_string()),
            pool: Vec::new(),
        });
        self.rolls.last_mut().expect("刚 push 过")
    }
}
