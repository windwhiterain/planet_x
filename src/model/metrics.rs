use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyId, CityId, FactionId, HaulStep, ResourceMap, RoundInputs, ShipId};

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
    /// 各势力：一行装下这个势力的观测（城/舰/人口/库存价值/是否交战/被谁禁运）与**过程**
    /// （本回合产出/维护费/治理/贸易/预算/集货运力账）。
    pub factions: BTreeMap<FactionId, FactionRow>,
    /// 各城：一行装下观测（人口/忠诚）与**过程**（本回合开采产出）。
    pub cities: BTreeMap<CityId, CityRow>,

    // ── 本回合的结算事实（过程；`pre` 里为空）────────────────────────
    /// 本回合**真的成交**的星际贸易，一笔一行（`buyer` × `seller`，按买卖双方名排序）：
    /// 价格是怎么算出来的、货运路上丢了多少。**稀疏**——没成交的回合是空数组。
    pub market_trades: Vec<MarketTrade>,
    /// 本回合**每艘在跑运输的舰**走了哪一步（`ship_id → HaulStep`）。
    /// **稀疏**：没跑运输的舰没有键（不是「走了空步」——那是 `waiting`）。
    pub haul_steps: BTreeMap<ShipId, HaulStep>,

    // ── AI 的判定（过程；`pre` 里为空）───────────────────────────────
    /// 本回合 **AI 的判定**（“掷了什么”）：逐舰的行为判定 + 船坞改装 + 风格重估 + 设计图。
    /// **纯追加、行为中性**——见 [`crate::model::RoundDecisions`]（那里解释了为什么它必须单独
    /// 捕获：指令叶只记结果，不记过程）。
    pub decisions: crate::model::RoundDecisions,
}

/// 本回合的一笔**星际贸易结算**（[`RoundView::market_trades`] 的一项）。
///
/// 粒度是 **(买方, 卖方) 一对一行**，不是「一对 × 一资源一行」：下面这些数**全部只由这一对
/// 决定**（距离/异常带深度/关系），与该笔买的是哪种矿无关——而每种矿的成交价就是
/// `view.market_price[资源] × (rel_mult + freight_rate)`（[`crate::sim::step_market`] 里就是
/// 这么算的：`p_eff = p × (rel_mult + freight_rate)`，`p` 就是市场价）。
/// 「这笔买卖买了些什么、各多少件」在 `moved` 里。
///
/// 这些数此前**算完就扔**：`view.market_settled` 只给全世界的成交量、`net_import` 只给各家的
/// 净值，于是「为什么是这个价」「我的货为什么少了」都没有解释面。
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct MarketTrade {
    /// 买方（掏钱的一方）。
    pub buyer: FactionId,
    /// 卖方（出货的一方）。
    pub seller: FactionId,
    /// 这一对之间的**实得成交**，按资源（只列真成交的 ⇒ 稀疏）。
    ///
    /// 语义 = **卖方交出的件数**（途中失联的那部分还没扣）：买方收到的是
    /// `moved[资源] × (1 − loss)`，那部分在途中消失（`view.market_settled` 记的是收到的那份）。
    pub moved: ResourceMap,
    /// 两个贸易锚点（各自首都天体）之间的**距离**（AU）。
    pub dist_au: f64,
    /// 这条线路上**引力异常带的浸入深度**（0 = 不穿带）。
    pub depth: f64,
    /// 穿带的**额外运费倍率** = `mond_freight_mult × depth ÷ (depth + 1)`（0 = 不穿带）。
    pub mond_extra: f64,
    /// **运费率** = `freight_per_au × dist_au × (1 + mond_extra)`——加在价格上的那一份。
    pub freight_rate: f64,
    /// **关系倍率**（[`crate::sim::relation_price_mult`]）：向敌人买更贵、向朋友买更便宜。
    pub rel_mult: f64,
    /// 这条线上**最好的掌握度** = `max(买卖双方的 mond_control)`（0 = 都是凡人，1 = 有一方到顶）。
    /// 它决定丢货率，也是「谁在深空贸易里当承运人」的那个量。
    pub mastery: f64,
    /// **丢货比例**（0..`mond_loss_cap`）：非 0 说明这批货走异常带时**部分失联**——
    /// 「我买到的货为什么少了」的答案。确定性比例，不是掷骰。
    pub loss: f64,
}

/// 一个势力**在某处货栈**的运力账（[`FactionRow::freight_gap`] 的一项）。
///
/// 这就是 [`crate::autocontrol::freight::capacity_ledger`] 的一行（雇主挂单用的同一本账）：
/// 「这处积压挂不出单（缺口 0）、或挂了多少 = 要求 − 自有期望份额 − 已雇」。
/// 此前只有势力级的 `haul_gap`（= `Σ缺口 ÷ Σ要求`）会露出来，**看不到是哪一处货栈在积压**。
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct FreightGap {
    /// **要求运力**（单位/回合）：把这处的积压按一个往返运回首都所需要的吞吐。
    pub need: f64,
    /// **自有运力的期望份额**：派单是按积压占比抽签的（`route_for`），所以「期望落到这处的
    /// 那一份」= `Σ(各运输舰在这条线上的吞吐) × (这处积压 ÷ 总积压)`——与真实派单同口径。
    ///
    /// ⚠ 若自己**此刻一艘能派的运输舰都没有**（例如全被雇去跑别人的线了），这里是 **`-0.0`**：
    /// 那是 Rust 对**空迭代器求和**的符号位（`sum()` 从 `-0.0` 起折），**数值上等于 0**——
    /// 判「有没有自有运力」请用 `== 0.0`，别写 `< 0.0` 或 `> 0.0` 的变体去区分正负零。
    pub own: f64,
    /// **已雇运力**（已接单合同的 `capacity` 之和：接单就是承诺）。
    pub hired: f64,
    /// **缺口** = `max(0, need − own − hired)`：连续量、无阈值，雇够了自己归零。
    pub uncovered: f64,
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
    /// **谁对本势力全面禁运、为什么**（`{对方势力: 原因}`；「不卖给你」的观察面）。
    /// 原因三档（判据同市场结算的可见性过滤，见 [`crate::sim::trade_block_cause`]）：
    /// `war`（交战）/ `cold`（关系冷到 `embargo_relation`）/ `coalition`（倒向联盟者封锁霸权）
    /// ——**三档对策不同**，所以只给一个计数不够用。
    /// 空 map = 谁都跟我做生意（中性值 `{}`）。
    pub trade_blocked_by: BTreeMap<FactionId, String>,

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
    /// 本回合收到的**承运费**（市场价值）：承运人由**掌握度最高**的第三方充当，抽成 `∝` 它的
    /// `mond_control`（**不是布尔**——见 `sim::market`、`.agents/notes/tech-system.md` §2），
    /// 所以这一列量的是「它在深空贸易里抽到多少税」，而不是「它是不是那个 master」。
    pub carrier_income: f64,
    /// 本回合**贸易净额**（买 − 卖，按市场价值；>0 = 净进口）。这是「谁靠贸易活着」
    /// 的观察面：一个净进口常年为 0 的势力，其实没在参与市场。
    pub net_import: f64,
    /// 本回合治理开销的**行政部分**（`Σ (admin_base + admin_per_au × 距离超程) × scale`）——
    /// 「我把娱乐预算拉满，钱却被行政吃掉」里的那个行政。
    pub governance_admin: f64,
    /// 本回合治理开销的**娱乐/福利部分**（势力级 `welfare_budget` 按城市福利权重分给各城之和）。
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

    // ── 过程：钱去哪了（B2）──
    /// 本回合**实际花掉**的**投资**（建设建筑）预算，按资源（`step_construction` 的中间量，
    /// 写完即弃的局部变量）。只列真花过的资源（稀疏 map）。
    ///
    /// **批了多少不在这里**：那是控制面的持久叶 `control`（`kind="投资预算"`，每资源
    /// 一行），引擎每回合把当回合用的额度写回去。所以「批了 100 铁为何只花 30」= 限额 − 这里，
    /// 读面不重复存第三个数（未花掉的余额）。
    pub investment_spent: ResourceMap,
    /// 本回合**实际花掉**的**建造**（造舰）预算，按资源——语义同
    /// [`FactionRow::investment_spent`]（限额见 `control` 的 `建造预算` 叶）。
    pub construction_spent: ResourceMap,
    /// 本回合**付不起**的那部分舰队维护费（市场价值 = `max(0, 维护费 − 库存价值)`）：付不出的
    /// 每一分钱都变成生锈（见 [`FactionRow::fleet_rust`]）。0 = 付清了。
    ///
    /// ⚠ 它是「**欠费并因此生锈**的那部分」，不是「付了多少」的反面：**流亡舰队**（无活城）与
    /// 零舰队势力的这一格同样是 0——前者被引擎豁免抽库存（欠着，但没锈，见 `sim::step_upkeep`
    /// 的例外），后者根本没账。所以别拿 `upkeep − 这个数` 当「实际付出去的钱」。
    pub upkeep_unpaid: f64,
    /// 本回合**每艘舰被锈掉的船体比例**（`step_upkeep` 的中间量）：该舰本回合掉的船体 =
    /// `hull_max × 这个比例`。只有锈到 0 才留 `DeathCause::UpkeepShortfall` 事件——**掉血本身
    /// 就靠这一个数才看得见**。
    ///
    /// ⚠ 它**不是**「欠费比例」：引擎有个可见性下限（欠一丁点也至少锈 `0.2`），所以欠费很小时
    /// 这个数反而比 `upkeep_unpaid ÷ upkeep` 大——**读这个数，别自己按欠费比例重算**。
    pub fleet_rust: f64,

    // ── 过程：市场里的位置（B3）──
    /// 本回合**购买力**（市场价值）：结算开始那一刻，本势力**可出口富余**的总价值
    /// （`listed_value`）——买方就是**按它降序排队**的（同额按名字）。
    /// 「为什么有货在卖我却没买到」的第一个答案就是它：钱多的人先买。
    pub purchasing_power: f64,
    /// **在买方队列里的位置**（0 = 第一个挑，按购买力降序）：`None` = 这一回合没排过队
    /// （`pre` 面）。它与 [`FactionRow::purchasing_power`] 一起读——名次是**排序的结果**，
    /// 别自己拿购买力重排一遍（同额时的名字序在引擎里是写死的 tie-break）。
    pub market_rank: Option<usize>,
    /// 本回合**每一处货栈的运力账**（`天体 → 账`）：哪处在积压、缺口多少、自己的船期望能搬多少、
    /// 已经雇到多少。**稀疏**（没积压的货栈不占键）。势力级的总账仍在 `haul_gap`（= `Σ缺口 ÷ Σ要求`）。
    pub freight_gap: BTreeMap<BodyId, FreightGap>,
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
    /// **过程**：本回合的**用工系数**（人口 ÷ 建筑用工需求，钳到 `[min_efficiency, 1]`）——
    /// 直接乘在采矿产出与造舰速率上。「这座城产量低」= 人手不足。
    ///
    /// ⚠ 它是**生产那一步用的人手**（`step_production` 在人口增长**之前**取的数）；建造那一步
    /// 会重新算一把（那时人口已经涨过了），而它已经折进每舰级的
    /// [`BuildLine::rate`] 里，所以这里不存第二份。
    pub labor: f64,
    /// **过程**：本回合的**住房容量**（住宅面积 × 该天体生态容量）——人口增长的**天花板**。
    /// 「为什么人口不涨了、产出提不上去」的答案。0 = 这一回合没算（`pre`）。
    pub housing_capacity: f64,
    /// **过程**：本城天体是不是本势力的**首都**（集散地）：是 ⇒ 产出**直进势力池**，否 ⇒ 先落
    /// **产地货栈**等船运。「我挖出来的矿为什么用不了」的答案（`view.cities[].production` 只记
    /// 开采量，不分入库路径）。
    ///
    /// ⚠ 它在 `pre` 里是中性值 `false`（这个月的入库路径**还没定**）。要读「此刻谁是集散地」，
    /// 别用 `pre` 面：拿 `control` 的 `capital` 叶 + 城的 `body_id` 比。
    pub is_hub: bool,
    /// **过程**：本回合**造舰**的每舰级速率与实得进度（`step_construction` 的中间量）。键 =
    /// 舰级；**本城有这个舰级的建造区就有行**，包括 `increment = 0` 的那种（有产能却一分钱没
    /// 批到——正是要看的那一格）。
    pub build: BTreeMap<String, BuildLine>,
}

/// 一座城本回合**某个舰级**的造舰过程量（[`CityRow::build`] 的一项）。
///
/// 造舰进度池按**舰级**合并（`City.ship_progress`），所以这一对数是「造舰慢是因为缺钱还是缺
/// 产能」的唯一入口：`increment` 是实得进度，`rate` 是这个舰级本回合的产能上限——
/// **`increment < rate` ⇒ 钱是瓶颈**（建造预算批光了），**`increment ≈ rate` ⇒ 产能封顶**
/// （预算还剩着，是船坞/人手不够）。
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct BuildLine {
    /// 该舰级的产能速率上限 = `Σ(建造区面积 × 生产率 × 用工系数)`（所有同舰级的建造区相加）。
    pub rate: f64,
    /// 该舰级本回合**实得进度**（受建造预算与 `rate` 双向封顶）。
    pub increment: f64,
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
/// 读面里**没有这个名字**：`--derived`/`--index`/web 看到的是 `pre`（[`RoundInputs`]，
/// **输入面**）与 `post`（[`RoundView`]，**结算面**）两个面。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RoundSink {
    /// 本回合的**输入面**（B5）：掷出的随机数 + 判定时看到的输入。
    ///
    /// 与上面那些「过程量」的区别：那些是「世界变成了什么样」（`post` 那一侧），
    /// 这一格是「引擎**消费**掉了什么」（`pre` 那一侧）——两者都不可事后重算，但归属不同面。
    #[serde(default)]
    pub inputs: RoundInputs,
    /// 每城每资源的本回合开采产出。
    pub city_production: BTreeMap<CityId, ResourceMap>,
    /// 每势力每资源的本回合开采产出。
    pub faction_production: BTreeMap<FactionId, ResourceMap>,
    /// 每势力本回合**舰队维护**流：该付多少 / 欠了多少 / 锈掉多少比例。
    pub upkeep: BTreeMap<FactionId, UpkeepFlow>,
    /// 每势力本回合**贸易净额**（买 − 卖，按市场价值；>0 = 净进口）。
    /// 由 `sim::step_market` 在结算时记录——这是「谁真的在市场上买卖」的权威账。
    pub market_net: BTreeMap<FactionId, f64>,
    /// 每势力本回合**付出的运费**（市场价值）——距离与引力异常带的代价。
    pub market_freight: BTreeMap<FactionId, f64>,
    /// 每势力本回合**收到的承运费**（市场价值）——由当时**掌握度最高**的第三方抽走，
    /// 抽成 `∝` 它的 `mond_control`（连续量，不再是「master 名单」里的布尔身份）。
    /// 这是「垄断承运人」的观察面：一个势力靠承运挣多少，说明它在深空贸易里的地位。
    pub market_carrier_income: BTreeMap<FactionId, f64>,
    /// 每势力本回合治理流（总成本 / 覆盖率 / 行政娱乐拆分 / 人口超载倍率 / 思潮惩罚）。
    pub governance: BTreeMap<FactionId, GovernanceFlow>,
    /// 每城本回合的**忠诚目标值**分项（`step_governance` 的中间量）。
    pub city_loyalty: BTreeMap<CityId, LoyaltyTarget>,
    /// 每势力本回合**投资/建造预算实际花掉的**（按资源；`step_construction` 的中间量）。
    pub spend: BTreeMap<FactionId, SpendFlow>,
    /// 每城本回合的**用工系数 / 住房容量 / 是否集散地 / 每舰级造舰进度**（B2 的中间量，
    /// `step_production` 与 `step_construction` 各写自己那几格）。
    pub city_flow: BTreeMap<CityId, CityFlow>,
    /// 本回合**真的成交**的星际贸易（一笔一对一行；`step_market` 的中间量）。
    pub market_trades: Vec<MarketTrade>,
    /// 每势力本回合**购买力**（结算开始时的可出口富余价值；买方的排队键）。
    pub market_power: BTreeMap<FactionId, f64>,
    /// 每势力本回合**在买方队列里的名次**（0 = 第一个挑）。
    pub market_rank: BTreeMap<FactionId, usize>,
    /// 每势力每处货栈的**运力账**（`step_contracts` 挂单时算的那一本，`capacity_ledger`）。
    pub freight_gap: BTreeMap<FactionId, BTreeMap<BodyId, FreightGap>>,
    /// 本回合**每艘在跑运输的舰**走了哪一步（`haul_step` 的唯一产出口）。
    pub haul_steps: BTreeMap<ShipId, HaulStep>,
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

/// 一个势力的本回合**舰队维护**流（`RoundSink::upkeep` 的一项）——维护费是「预算压顶」那一类
/// 问题的入口：一个数说该付多少，另两个数说付不起时发生了什么。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UpkeepFlow {
    /// 本回合该付的舰队维护费（市场价值）。
    pub total: f64,
    /// 付不起的那部分（`max(0, total − 库存价值)`；0 = 付清）。
    pub unpaid: f64,
    /// 每艘舰实际被锈掉的船体比例（含可见性下限；0 = 没锈）。
    pub rust: f64,
}

/// 一个势力的本回合**投资/建造预算花销**（`RoundSink::spend` 的一项）。
///
/// 与限额的关系：限额是控制面的持久叶（`control` 的 `投资预算`/`建造预算`），
/// 引擎每回合把当回合用的额度写回去，所以两者相减就是「批了没花掉的」——**同一个数不存两处**。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SpendFlow {
    /// 实际花掉的投资预算，按资源（只列真花过的）。
    pub investment: ResourceMap,
    /// 实际花掉的建造（造舰）预算，按资源。
    pub construction: ResourceMap,
}

/// 一座城本回合的**产出与建造过程量**（`RoundSink::city_flow` 的一项）。
///
/// 写它的有**两步**（所以是「一格一格填」而不是「一次构造」）：`step_production` 填用工系数 /
/// 住房容量 / 是否集散地，`step_construction` 填每舰级的造舰速率与实得进度。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct CityFlow {
    /// 生产那一步用的用工系数（见 [`CityRow::labor`]）。
    pub labor: f64,
    /// 住房容量（见 [`CityRow::housing_capacity`]）。
    pub housing_capacity: f64,
    /// 本城天体是不是本势力的首都集散地（见 [`CityRow::is_hub`]）。
    pub is_hub: bool,
    /// 每舰级的造舰速率与实得进度（见 [`CityRow::build`]）。
    pub build: BTreeMap<String, BuildLine>,
}
