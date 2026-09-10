use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{CityId, FactionId, ResourceMap};

/// 一回合的**总结指标**：游戏步进函数（`sim::advance` 及其各 step）里实际计算的那些
/// 中间聚合量。agent 除了读到直接状态（`Trajectory` 的实体字段），还拿到这些「总结」——
/// 它们由 [`crate::sim::round_metrics`] 汇总，**复用步进函数本身所用的同一套计算**
/// （`faction_power_share`/`coalition_of`/`sanctioned_hegemon`/`war_pairs`），因此与直接
/// 状态**严格一致**，不会像一份独立重算的汇总那样与模拟漂移。纯数据、无 RNG。
///
/// `cities`/`ships`/`fleet_value`/`population`/`power_share`/`hegemon`/`coalition_members`/
/// `sanctioned`/`wars` 是「存量/政治」快照；`factions[].production_*`、`upkeep`、
/// `governance_*` 与 `city_production` 是「流量」——由 [`crate::sim::RoundFlow`] 在步进时
/// 捕获，逐回合与模拟所用数值一致。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct RoundMetrics {
    /// 世界级总量：活城数。
    pub cities: usize,
    /// 世界级总量：当前存在的舰数。
    pub ships: usize,
    /// 世界级总量：舰队价值（当前船体总和）。
    pub fleet_value: f64,
    /// 世界级总量：活城人口总和。
    pub population: u64,
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
    /// 各势力聚合指标（key = faction id）。
    pub factions: BTreeMap<FactionId, FactionMetrics>,
    /// 每座活城的本回合产出（`cities` 的细分）：人口、忠诚与按资源开采量。
    pub city_production: BTreeMap<CityId, CityMetrics>,
    /// 本回合**市场价**（资源 → 每单位价格 = 基价 × 稀缺系数）。这是「缺某种矿 →
    /// 市场上超高价」的观察面：价格由「世界库存够用几回合」算出，上限 `price_ceiling`。
    pub market_price: ResourceMap,
    /// 本回合各资源的**成交量**（真实成交的实物量）。成交 0 = 没人卖给你（或没人买）。
    pub market_settled: ResourceMap,
    /// 本回合各资源的**挂单总量**（供给侧：世界上真的有人拿出来卖多少）。
    pub market_offered: ResourceMap,
    /// 每势力本回合的**贸易净额**（买 − 卖，按市场价值；>0 = 净进口）。这是「谁靠贸易活着」
    /// 的观察面：一个净进口常年为 0 的势力，其实没在参与市场。
    pub market_net_import: BTreeMap<FactionId, f64>,
}

impl Default for RoundMetrics {
    fn default() -> Self {
        RoundMetrics {
            cities: 0,
            ships: 0,
            fleet_value: 0.0,
            population: 0,
            power_share: BTreeMap::new(),
            faction_power: BTreeMap::new(),
            hegemon: None,
            coalition_members: Vec::new(),
            sanctioned: None,
            wars: Vec::new(),
            factions: BTreeMap::new(),
            city_production: BTreeMap::new(),
            market_price: ResourceMap::new(),
            market_settled: ResourceMap::new(),
            market_offered: ResourceMap::new(),
            market_net_import: BTreeMap::new(),
        }
    }
}
/// 单势力的聚合总结指标（`RoundMetrics::factions` 的一项）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct FactionMetrics {
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
    /// 本回合开采产出的市场价值（= 其城之和，step_production 的中间量）。
    pub production_value: f64,
    /// 本回合开采产出，按资源（step_production 的中间量）。
    pub production: ResourceMap,
    /// 本回合舰队维护费（市场价值，step_upkeep 的中间量）。
    pub upkeep: f64,
    /// 本回合治理总开销（行政 + 娱乐，含制裁倍率；step_governance 的中间量）。
    pub governance_cost: f64,
    /// 治理覆盖率（0..1：库存能覆盖治理开销的比例；<1 = 治理不到位/忠诚在跌）。
    pub governance_coverage: f64,
    /// **有多少势力对本势力全面禁运**（「不卖给你」的观察面）。判据同市场结算：
    /// 交战 / 已倒向联盟的弱者 ↔ 被锁定的霸权 / 关系冷到 `embargo_relation`。
    /// >0 意味着这个势力的船坞只能靠自己挖的料——这是制裁真正咬到的地方。
    pub trade_blocked_by: usize,
    /// 本回合付出的**运费**（市场价值）：距离 × 运费率，穿越引力异常带再加倍。
    pub freight_paid: f64,
    /// 本回合收到的**承运费**（市场价值）：只有掌握了 MOND 的势力能可靠穿越异常带，
    /// 所以它是柯伊伯带贸易的垄断承运人，对这条线上的货运抽税。
    pub carrier_income: f64,
}
/// 单座城的本回合产出（`RoundMetrics::city_production` 的一项）。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct CityMetrics {
    pub population: u32,
    /// 忠诚度（0..1）。
    pub loyalty: f64,
    /// 本回合开采产出的市场价值。
    pub production_value: f64,
    /// 本回合开采产出，按资源。
    pub production: ResourceMap,
}
/// 一回合的**中间量捕获**：各 step 计算并应用、但不落到持久状态、原本不对外暴露的量。
/// `advance` 把 `RoundFlow` 带回 `round_metrics`，使 agent 的「流量总结」（产出/维护/
/// 治理）与模拟**逐回合完全一致**——不是事后从状态反推的近似值。纯数据、无 RNG。
///
/// 这类量有两族，都住在这里（同一条管道：算过 → 带出 → 投影成表）：
/// * **流量**（下面那些 map）：产出/维护/治理/贸易净额；
/// * **判定**（[`RoundFlow::decisions`]）：AI 这一回合**选了什么、为什么**（见
///   [`super::decisions`]）——它同样"不落持久状态、也不发事件"。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct RoundFlow {
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
    /// 每势力本回合治理流（总成本 / 覆盖率）。
    pub governance: BTreeMap<FactionId, GovernanceFlow>,
    /// 本回合 **AI 的判定**（“掷了什么”）：逐舰的行为判定 + 船坞改装。**纯追加、行为中性**
    /// ——见 [`super::decisions`]（那里解释了为什么它必须单独捕获：指令叶只记结果，不记过程）。
    #[serde(default)]
    pub decisions: crate::model::RoundDecisions,
}
/// 一个势力的本回合治理流（`RoundFlow::governance` 的一项）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GovernanceFlow {
    /// 治理总开销（行政 + 娱乐，含制裁倍率）。
    pub total: f64,
    /// 治理覆盖率（0..1）。
    pub coverage: f64,
}
/// 一个回合的**全部派生态**（合并旧 `RoundFlow` 的“流量”与旧 `RoundMetrics` 的“总结”）。
/// 这是所有 derived 数据的**唯一结构**：不再有 `RoundFlow` + `RoundMetrics` 两套。
/// * `flow`   —— 本回合**流量**中间量（每城/每势力产出、舰队维护费、治理总成本/覆盖率）。
/// * `metrics`—— 本回合**存量/政治**总结（世界总量、实力占比/霸权/联盟/制裁/战争、各势力
///   聚合、各城产出），其中 [`RoundMetrics::faction_power`] 是“谁最强”的**单一权威**统计。
///
/// 一个结构装下全部派生数据；同一个结构既用作 `RoundState::pre`（依赖 rng 的随机决策快照）
/// 也用作 `RoundState::post`（依赖 state 的纯观测快照）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Derived {
    /// 本回合**流量**中间量（原本的 [`RoundFlow`]）。
    pub flow: RoundFlow,
    /// 本回合**存量/政治**总结，含单一权威的 `faction_power`（原本的 [`RoundMetrics`]）。
    pub metrics: RoundMetrics,
}
