use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{CityId, FactionId, ResourceMap};

/// 一回合的**视图**——`pre`（推进前）与 `post`（推进后）用的是**同一个类型**：
///
/// * [`RoundState::pre`](crate::model::RoundState) —— **回合开始时**看到的世界；
/// * [`RoundState::post`](crate::model::RoundState) —— **回合结束时**看到的世界 + 本回合的**过程量**。
///
/// 两个槽**同形**，所以读的人只要记住一条：**`pre` 是回合前，`post` 是回合后**；`post` 比
/// `pre` 多的就是「这一回合发生了什么」。下面标了 **过程** 的字段在 `pre` 里是 0 / 空——那时
/// 它们还没发生（不是「没有」，是「还没算」）。
///
/// **过程量** = 各 step 算过、用过、但不落持久 [`State`](crate::model::State) 的数：本回合产出、
/// 舰队维护费、治理开销与覆盖率、市场运费/承运费/净进口，以及 [`RoundView::decisions`]（AI 这一
/// 回合**选了什么、为什么**）。它们由 `sim::advance` 在步进时捕获（引擎内部装在一个
/// [`RoundSink`] 里），因此与模拟**逐回合完全一致**，不是事后从状态反推的近似值。纯数据、无 RNG。
///
/// **同一个量只有一个位置**：观测与过程同处「每势力一行/[`FactionRow`]」「每城一行/[`CityRow`]」，
/// 所以不会再出现「总结说覆盖 100%、流水说覆盖 0%」那种两个读面各说各话。
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct RoundView {
    // ── 世界总量（观测）────────────────────────────────────────────────
    /// 活城数。
    pub city_count: usize,
    /// 当前存在的舰数。
    pub ship_count: usize,
    /// 舰队价值（当前船体总和）。
    pub fleet_value: f64,
    /// 活城人口总和。
    pub population: u64,

    // ── 政治（观测）──────────────────────────────────────────────────
    /// 各势力综合实力占比（0..1，全势力求和≈1）——均势/霸权判定的中间量。
    pub power_share: BTreeMap<FactionId, f64>,
    /// 各势力**综合实力**（`city_weight×城市份额 + fleet_weight×舰队份额`，未归一化为占比，
    /// 即 `power_share` 的分子）。这是“谁最强”的**单一权威**统计：一次算好，供游戏逻辑
    /// （`step_balance_of_power`/`sanction_cost_mult`）与观测/测试共同读取，杜绝「测试以为
    /// 的霸权 ≠ 游戏针对的霸权」这类不一致。语义同舰只的 `deterrence`：派生、非持久实体字段。
    pub faction_power: BTreeMap<FactionId, f64>,
    /// 当前「霸权」：综合实力占比达阈值的最大势力；无则 None。
    pub hegemon: Option<FactionId>,
    /// 针对霸权的反制联盟成员（关系 ≤ coalition_estrange 的弱者，且彼此不交战）。
    pub coalition_members: Vec<FactionId>,
    /// 被多国经济制裁的霸权（已有至少一个弱者倒向联盟即封锁）；无则 None。
    pub sanctioned: Option<FactionId>,
    /// 当前交战中的势力对（关系 ≤ war_threshold，无序归一化）。
    pub wars: Vec<(FactionId, FactionId)>,

    // ── 市场（观测）──────────────────────────────────────────────────
    /// 本回合**市场价**（资源 → 每单位价格 = 基价 × 稀缺系数）。这是「缺某种矿 →
    /// 市场上超高价」的观察面：价格由「世界库存够用几回合」算出，上限 `price_ceiling`。
    pub market_price: ResourceMap,
    /// 本回合各资源的**成交量**（真实成交的实物量）。成交 0 = 没人卖给你（或没人买）。
    pub market_settled: ResourceMap,
    /// 本回合各资源的**挂单总量**（供给侧：世界上真的有人拿出来卖多少）。
    pub market_offered: ResourceMap,

    // ── 每势力一行 / 每城一行（观测 + 本回合过程同处一行）──────────────
    /// 各势力：一行装下这个势力的观测（城/舰/人口/库存价值/是否交战/被几家禁运）与**过程**
    /// （本回合产出/维护费/治理/贸易）。
    pub factions: BTreeMap<FactionId, FactionRow>,
    /// 各城：一行装下观测（人口/忠诚）与**过程**（本回合开采产出）。
    pub cities: BTreeMap<CityId, CityRow>,

    // ── AI 的判定（过程；`pre` 里为空）───────────────────────────────
    /// 本回合 **AI 的判定**（“掷了什么”）：逐舰的行为判定 + 船坞改装 + 风格重估 + 设计图。
    /// **纯追加、行为中性**——见 [`crate::model::RoundDecisions`]（那里解释了为什么它必须单独
    /// 捕获：指令叶只记结果，不记过程）。
    pub decisions: crate::model::RoundDecisions,
}

/// 单势力的一行（[`RoundView::factions`] 的一项）：观测 + 本回合过程同处一行——「同一个数只有
/// 一个位置」，所以不会出现「总结表说 100%、流水表说 0%」那种两个读面各说各话。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct FactionRow {
    // ── 观测 ──
    /// 活城数（未 razed）。
    pub city_count: usize,
    /// 当前存在的舰数。
    pub ship_count: usize,
    /// 舰队价值（当前船体总和）。
    pub fleet_value: f64,
    /// 人口总和（其所有活城）。
    pub population: u64,
    /// 库存市场价值（资源量 × 单价）。
    pub market_value: f64,
    /// 该势力当前是否与任意其他势力交战。
    pub at_war: bool,
    /// **有多少势力对本势力全面禁运**（「不卖给你」的观察面）。判据同市场结算：
    /// 交战 / 已倒向联盟的弱者 ↔ 被锁定的霸权 / 关系冷到 `embargo_relation`。
    /// >0 意味着这个势力的船坞只能靠自己挖的料——这是制裁真正咬到的地方。
    pub trade_blocked_by: usize,

    // ── 过程（`pre` 里为 0 / 空）──
    /// 本回合开采产出，按资源（step_production 的中间量）。
    pub production: ResourceMap,
    /// 本回合开采产出的市场价值（= 上面那份产出按价值求和）。
    pub production_value: f64,
    /// 本回合舰队维护费（市场价值，step_upkeep 的中间量）。
    pub upkeep: f64,
    /// 本回合治理总开销（行政 + 娱乐，含制裁倍率；step_governance 的中间量）。
    pub governance_cost: f64,
    /// 治理覆盖率（0..1：库存能覆盖治理开销的比例；<1 = 治理不到位/忠诚在跌）。
    pub governance_coverage: f64,
    /// 本回合付出的**运费**（市场价值）：距离 × 运费率，穿越引力异常带再加倍。
    pub freight_paid: f64,
    /// 本回合收到的**承运费**（市场价值）：只有掌握了 MOND 的势力能可靠穿越异常带，
    /// 所以它是柯伊伯带贸易的垄断承运人，对这条线上的货运抽税。
    pub carrier_income: f64,
    /// 本回合**贸易净额**（买 − 卖，按市场价值；>0 = 净进口）。这是「谁靠贸易活着」
    /// 的观察面：一个净进口常年为 0 的势力，其实没在参与市场。
    pub net_import: f64,
    /// 本回合治理开销的**行政部分**（`Σ (admin_base + admin_per_au × 距离超程) × scale`）——
    /// 「我把娱乐预算拉满，钱却被行政吃掉」里的那个行政。
    pub governance_admin: f64,
    /// 本回合治理开销的**娱乐/福利部分**（各城 [`city_loyalty_budget`](crate::sim::city_loyalty_budget) 之和）。
    pub governance_entertainment: f64,
    /// **人口超载放大倍率**：`1 + max(0, 人口 ÷ 管理容量 − 1)`。它同时乘在行政开销与每座城的
    /// 忠诚距离项上——「为什么治理费比上回合暴涨」的答案。中性缺省 = **1.0**（`pre` 里是 1.0 而
    /// 不是 0：0 会被读成「治理能力归零」，那是另一回事；与 [`FactionRow::governance_coverage`]
    /// 的缺省约定同类）。
    pub governance_scale: f64,
    /// 本回合**思潮优势端自平衡**忠诚惩罚（按势力算一次，0..`max_loyalty_penalty`）：势力身处垄断的
    /// 优势端思潮却「言行不符」时的扣分（军国却不打仗、科学却不探异常区……）。
    ///
    /// 它是每座城忠诚目标式里的一个扣项，但**只在势力行存一份**——城行不重复它
    /// （「同一个数只有一个位置」）。读的时候在势力行上取。
    pub ideology_loyalty_penalty: f64,
    /// 本回合**首都向心项**：首都人口占全势力比例 × `capital_share_loyalty_buff`（按势力算一次）。
    /// 同 [`FactionRow::ideology_loyalty_penalty`]：它是每座城忠诚目标式里的加项，只在势力行存一份。
    pub capital_loyalty_bonus: f64,
}

/// 单座城的一行（[`RoundView::cities`] 的一项）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct CityRow {
    /// 观测：人口。
    pub population: u32,
    /// 观测：忠诚度（0..1）。
    pub loyalty: f64,
    /// **过程**：本回合开采产出，按资源。
    pub production: ResourceMap,
    /// **过程**：本回合开采产出的市场价值。
    pub production_value: f64,
    /// **过程**：本回合忠诚的**目标值**及其分项（`step_governance` 的中间量）。忠诚每回合朝
    /// `effective` 靠近（治理覆盖得住时），所以这些分项就是「这座城的忠诚为什么在掉」的答案。
    pub loyalty_target: LoyaltyTarget,
}

/// 一座城本回合的**忠诚目标值**及其分项（[`CityRow::loyalty_target`]）。
///
/// `step_governance` 每回合算出一个目标忠诚 `effective`，再把实际忠诚朝它恢复
/// （`loyalty_recover`）；治理覆盖不住时改用欠费惩罚。此前这几个数**算完就扔**，于是
/// 「忠诚在掉」只能看见结果、看不见原因。
///
/// **只放「逐城不同」的项**：首都向心项与思潮惩罚按势力算一次，所以它们住在
/// [`FactionRow::capital_loyalty_bonus`] / [`FactionRow::ideology_loyalty_penalty`]（同一个数
/// 只有一个位置，别在同势力的每座城里抄一遍）。完整式子：
///
/// ```text
/// effective = clamp(distance + entertainment
///                   + factions[<势力>].capital_loyalty_bonus
///                   - factions[<势力>].ideology_loyalty_penalty, 0, 1)
/// ```
///
/// 缺省（`pre` 里 / 该城这一回合没跑治理）**全 0**——同其它过程量的约定：0 = 还没算。
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct LoyaltyTarget {
    /// 距离项：`1 − loyalty_distance × max(0, 距首都 − loyalty_range) × 治理倍率`（越远越低）。
    pub distance: f64,
    /// 娱乐/福利项：`本城娱乐预算 × 治理覆盖率 ÷ entertainment_cost`（预算真落地的那部分）。
    pub entertainment: f64,
    /// 本城两项之和 clamp 到 0..1 的**结果**（另两项按势力取，见上面那段式子）——忠诚每回合朝它恢复。
    pub effective: f64,
}

/// **引擎内部的写入口袋**（**不是读面的一部分**）：各 step 把「算过、用过、但不落持久状态」的
/// 量写进它，`sim::advance` 在回合末把它折进 [`RoundView`]（见 `sim::observe`）。
///
/// 为什么它是**独立于 [`RoundView`] 的类型**，而不是让 step 直接写视图：
///
/// 1. **形状不同**：这里是「按 step 分组的 map」（`势力 → 数`），读面是「每势力**一行**」——
///    同一个量在两种形状里无法共用一个字段（`upkeep` 在这里是 `BTreeMap`，在 [`FactionRow`]
///    里是 `f64`）；
/// 2. **产生时机不同**：过程量是回合中**逐步**产生的（每个 step 写自己那部分），观测是回合末
///    **一次**算好的（`sim::observe` 复用游戏逻辑本身用的那套计算）——两类量本来就不该在同一个
///    时刻由同一段代码写。
///
/// 读面里**没有这个名字**：`--derived`/`--index`/web 看到的只有 `pre`/`post` 两个 [`RoundView`]。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RoundSink {
    /// 每城每资源的本回合开采产出。
    pub city_production: BTreeMap<CityId, ResourceMap>,
    /// 每势力每资源的本回合开采产出。
    pub faction_production: BTreeMap<FactionId, ResourceMap>,
    /// 每势力本回合舰队维护费（市场价值）。
    pub upkeep: BTreeMap<FactionId, f64>,
    /// 每势力本回合**贸易净额**（买 − 卖，按市场价值；>0 = 净进口）。
    /// 由 `sim::step_market` 在结算时记录——这是「谁真的在市场上买卖」的权威账。
    pub market_net: BTreeMap<FactionId, f64>,
    /// 每势力本回合**付出的运费**（市场价值）——距离与引力异常带的代价。
    pub market_freight: BTreeMap<FactionId, f64>,
    /// 每势力本回合**收到的承运费**（市场价值）——掌握 MOND 的 master 靠穿越异常带抽的税。
    /// 这是「垄断承运人」的观察面：一个势力靠承运挣多少，说明它在深空贸易里的地位。
    pub market_carrier_income: BTreeMap<FactionId, f64>,
    /// 每势力本回合治理流（总成本 / 覆盖率 / 行政娱乐拆分 / 人口超载倍率 / 思潮惩罚）。
    pub governance: BTreeMap<FactionId, GovernanceFlow>,
    /// 每城本回合的**忠诚目标值**分项（`step_governance` 的中间量）。
    pub city_loyalty: BTreeMap<CityId, LoyaltyTarget>,
    /// 本回合 **AI 的判定**（“掷了什么”）：逐舰的行为判定 + 船坞改装。**纯追加、行为中性**
    /// ——见 [`crate::model::RoundDecisions`]（那里解释了为什么它必须单独捕获：指令叶只记结果，
    /// 不记过程）。
    #[serde(default)]
    pub decisions: crate::model::RoundDecisions,
}

/// 一个势力的本回合治理流（`RoundSink::governance` 的一项）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GovernanceFlow {
    /// 治理总开销（行政 + 娱乐，含制裁倍率）。
    pub total: f64,
    /// 治理覆盖率（0..1）。
    pub coverage: f64,
    /// 行政部分（距离 × 人口超载）。
    pub admin: f64,
    /// 娱乐/福利部分（各城预算之和）。
    pub entertainment: f64,
    /// 人口超载放大倍率（1.0 = 未超载）。
    pub scale: f64,
    /// 思潮优势端自平衡忠诚惩罚（按势力算一次）。
    pub ideology_penalty: f64,
    /// 首都向心项（按势力算一次）——每座城忠诚目标式里的加项，见
    /// [`FactionRow::capital_loyalty_bonus`]。
    pub capital_bonus: f64,
}
