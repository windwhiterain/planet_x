use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::{BodyKindSpec, BuildingSpec, ComponentSpec, FactionId, ResourceDef, ShipSpec, StoryEvent};

/// Economy tuning (production and population).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EconomyConfig {
    /// Mined resource units per unit area per round at full staffing.
    pub production_rate: f64,
    /// Fractional population growth toward housing capacity each round.
    pub pop_growth: f64,
    /// Floor on production efficiency.
    pub min_efficiency: f64,
    /// Command-controlled construction budget: fraction of each resource
    /// stockpile that may be invested into building infrastructure per round.
    pub invest_fraction: f64,
    /// Housing target multiplier: residential area is kept at
    /// `population / ecological_capacity * housing_buffer`.
    pub housing_buffer: f64,
    /// AI 造舰的「维护费保留」：自动指挥的势力在建新舰前，会从库存里**预留**一定倍数的
    /// 舰队维护费（市场价值），只把超出部分用于造舰——「把海军养在经济能承受的规模」。
    /// 避免基线 AI 无脑大建（库存×invest_fraction）、随后维护费拖垮经济、军备崩盘。
    /// 越大越保守；0 = 不保留（旧行为）。
    #[serde(default = "default_upkeep_reserve_mult")]
    pub upkeep_reserve_mult: f64,
}

fn default_upkeep_reserve_mult() -> f64 {
    4.0
}
/// Combat tuning. Cities have no separate defense pool: a city's hardness is
/// the sum of its buildings' armor and bombardment destroys buildings (by area
/// share) until the city is razed to blank.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CombatConfig {
    /// A relation at or below this value is treated as hostile (war).
    pub war_threshold: f64,
    /// Distance, in AU, within which a ship can besiege a city.
    pub siege_range: f64,
    /// Distance, in AU, at which a ship is considered to have arrived.
    pub arrival_eps: f64,
    /// 贴脸（face-hug，`kiting>0`）时舰对敌的**最小交战距离**（AU）：贴脸舰压近到这
    /// 一距离；0 = 贴近到接触（在 `arrival_eps` 停）。默认 0。
    #[serde(default = "default_min_engage_range")]
    pub min_engage_range: f64,
    /// Fraction of a building's lost armor repaired each round when not under
    /// bombardment.
    pub armor_regen: f64,
    /// Fraction of the settlement area a freshly founded (colonized) city may
    /// claim, capped for the initial footprint.
    pub colony_footprint: f64,
    /// 自动指挥舰的「自保撤退」阈值：舰当前护甲占最大护甲低于此比例、且敌方舰在本舰
    /// 有效射程内时，向后撤往其首都/本土修整充能（而非死战）。这是拟人的「别送死」
    /// 行为：打残就撤、养好再回来，让战争有损耗与恢复的循环。
    #[serde(default = "default_retreat_hull")]
    pub retreat_hull: f64,
    /// 自保撤退的最小距离（AU）：舰距其首都**小于**此距离时不撤退（在主场原地驻防/
    /// 充能），避免「已到家还一直撤退、白白不还手」的僵局。离首都越远、越难得到本土
    /// 防御与再生时，才值得后撤修整。
    #[serde(default = "default_retreat_min_dist")]
    pub retreat_min_dist: f64,
    /// 模块损毁：被击中时，每点船体伤害中溢出去损坏组件的比例。组件完整度降到 0 即被
    /// 击毁、不再贡献面板/武器——军舰随战况**渐进丧失战力**（武器被打掉、护盾被打掉），
    /// 而不是满血抗到壳破。0 = 关闭。
    #[serde(default = "default_component_spill")]
    pub component_spill: f64,
    /// 模块修复：每回合按此比例恢复组件完整度（占组件初始完整度的百分比）。受损舰在
    /// 友方本土/母港（首都 `home_radius` 内）修理得**更快**——与自保撤退闭环：打残→撤
    /// →修→再来。0 = 关闭。
    #[serde(default = "default_component_repair")]
    pub component_repair: f64,
    /// 护航半径（AU）：交战时，闲着且距本势力旗舰（航母）在此半径内的 AI 舰会就近护卫
    /// 它（`Follow{旗舰}` 的跟随行为：贴近旗舰）。攻击/拦截由射程内自动接战完成。保护
    /// 高价值舰种。0 = 关闭。
    #[serde(default = "default_escort_range")]
    pub escort_range: f64,
    /// 追击半径（AU）：AI 舰只在**此半径内**才会去追敌对舰——避免跨越全图去追一艘远逃
    /// 的敌舰（过度延伸、漂移、被伏击）。超过此半径的敌舰不在追击范围（转而去轰炸城市/
    /// 殖民/护卫）。这是拟人的「不追远敌、就近平守」。0 = 不限（总是追）。
    #[serde(default = "default_pursuit_range")]
    pub pursuit_range: f64,
    /// 舰队防空半径（AU）：舰的点防御除了拦自己吃到的导弹，还会在**此半径内**替附近友舰
    /// 拦截导弹（防空屏护，随距离线性衰减、封顶）。这让有 PD 的舰组成防空圈、能护卫航母/
    /// 友舰——与护航行为衔接。0 = 只护自己。
    #[serde(default = "default_pd_radius")]
    pub pd_radius: f64,
    /// 威慑叠加半径（AU）：计算某舰威慑时，与其**同势力**、在此半径内的友舰战力会叠加上来
    /// ——「威慑 = 综合战力 + 附近同势力战力互相叠加」。理智<->热血据此挑「威慑低于自己
    /// 的」或「威慑高于自己的」目标打。
    #[serde(default = "default_deterrence_radius")]
    pub deterrence_radius: f64,
    /// 攻击历史的衰减因子（每回合）：本舰攻击某目标后，该目标的新鲜度会被刷新到 1，
    /// 之后每回合乘以此因子衰减（越久越淡）。越小「记仇」越短。
    #[serde(default = "default_attack_hist_decay")]
    pub attack_hist_decay: f64,
}

fn default_retreat_hull() -> f64 {
    0.28
}

fn default_retreat_min_dist() -> f64 {
    1.5
}

fn default_min_engage_range() -> f64 {
    0.0
}

fn default_component_spill() -> f64 {
    0.12
}

fn default_component_repair() -> f64 {
    0.04
}

fn default_escort_range() -> f64 {
    10.0
}

fn default_pursuit_range() -> f64 {
    12.0
}

fn default_pd_radius() -> f64 {
    3.0
}

fn default_deterrence_radius() -> f64 {
    8.0
}

fn default_attack_hist_decay() -> f64 {
    0.6
}
/// Building structure attribute (混凝土 / 钢结构).
///
/// Structures are an attribute *of the building itself* — they modify the
/// building's hardness (armor-per-area) and its construction cost. Not a
/// cross-cutting material multiplier: each building carries exactly one.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StructureSpec {
    pub name: String,
    /// 护甲 per unit deployed area.
    pub armor_per_area: f64,
    /// Construction-cost multiplier for this structure.
    pub cost_mult: f64,
}
/// Diplomacy tuning. Drives a dynamic international-relations model: each pair
/// drifts toward an "affinity" resting level derived from the two factions'
/// ideologies (alignment), aggressive powers accelerate hostility with rivals,
/// wars wind down via fatigue once fighting stops, and a little noise keeps
/// relations fluctuating so wars both begin and end over time.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiplomacyConfig {
    /// Relation change caused by a hostile attack.
    pub attack_delta: f64,
    /// Relation change caused by capturing an enemy city.
    pub capture_delta: f64,
    /// Per-round pull toward each pair's resting affinity (bloc formation).
    pub drift_rate: f64,
    /// While at war and not currently fighting, pull relations toward
    /// `ceasefire_relation` (war fatigue) so conflicts wind down to peace.
    pub war_fatigue: f64,
    /// Relation level a war cools toward when fighting stops (above the war
    /// threshold, so the pair crosses back into peace).
    pub ceasefire_relation: f64,
    /// **记恨（战争疤痕）**：一场战争的影响持续多少回合。窗口内，开过战的那一对势力关系被压在
    /// 一道地板下，地板从 `war_scar_relation` **线性衰减到 0**。
    ///
    /// 这是**窗口层**（[`crate::model::Notables`]）的唯一读者——`WarStarted` 定级为
    /// [`crate::model::Salience::Notable`] 的依据就是它：开战之后「相当一段时间两国互相记恨」，
    /// 所以外交计算要回看**一定窗口**，而窗口之外的那场战争不再影响任何计算。
    ///
    /// 副作用（也是此前 `war_started`/`war_ended` 反复闪烁的一个成因的解药）：新鲜疤痕低于
    /// `combat.war_threshold`，所以**刚开战的对手不可能当回合就言和**，战争不会一闪即灭。
    /// `0` = 关闭本机制。
    pub war_scar_rounds: u32,
    /// 战争疤痕的初值（负值 = 敌意）。新鲜时应当低于 `combat.war_threshold`，否则压不住
    /// 「当回合言和」。`>= 0` = 关闭本机制。
    pub war_scar_relation: f64,
    /// Resting affinity at maximum ideological distance (opposite blocs).
    pub affinity_floor: f64,
    /// Extra affinity at full ideological closeness (same bloc allies).
    pub affinity_span: f64,
    /// 思潮相似度对静息亲和的**对称**修正幅度：两方思潮越像，亲和越高；越对立，亲和越低。
    /// 思潮相似度 = `1 − 四轴平均 |Δ|/2`（每轴 [-1,1]，故相似度在 [0,1]），
    /// 该项对亲和的贡献 = `ideology_affinity_span × (2×相似度 − 1)`（在
    /// `[-ideology_affinity_span, +ideology_affinity_span]` 对称）。0 = 关闭（只按
    /// 静态 alignment 决定亲和）。与 alignment（历史静态阵营亲缘）叠加，构成
    /// 「历史静态 + 思潮可变」双因子。
    #[serde(default)]
    pub ideology_affinity_span: f64,
    /// Random fluctuation per round, so relations oscillate and can cross the
    /// war threshold.
    pub noise: f64,
    /// Lower clamp on any relation (bounded hostility).
    pub hostility_floor: f64,
    /// Upper clamp on any relation (bounded friendliness).
    pub friendship_ceiling: f64,
}
/// Market tuning. 星际市场是**真实交换所**：供给来自各势力真实的富余（挂单记名卖家），
/// 价格由「库存够全世界用几回合」逐回合算出（稀缺 → 高价），成交按挂单**配给**
/// （买不到就是买不到），并且禁运可以让某个卖家**根本不卖给你**。
/// 见 `.agents/notes/trade-and-sanctions.md`。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MarketConfig {
    /// Per-round cap, in market value (credits), on how much a faction may buy
    /// this round. `0.0` disables the market entirely (无市场：谁缺料谁自己扛).
    pub auto_trade_limit: f64,
    /// 买方每资源想维持的库存（单位）——低于它才会去买。这是**军工目标库存**。
    pub working_buffer: f64,
    /// 市场手续费：买方要交出 `cost × (1+spread)` 的实物才能换到价值 `cost` 的货，
    /// 差额是**市场烧掉的价值**（sink）——贸易不是免费的价值搬运。
    pub spread: f64,
    /// 卖方保留比例：挂单前先留下自己库存的这个比例（1.0 = 一毛不卖）。
    /// 卖方供给 = 库存 − max(自己需要的 working_buffer, 库存 × reserve_fraction)。
    pub reserve_fraction: f64,
    /// 价格发现的目标「覆盖回合数」：世界的库存应当够全世界用这么多回合。
    /// 库存只够用 1 回合 → 价格顶到 [`Self::price_ceiling`]；够用这么多回合 → 基价。
    pub coverage_rounds: f64,
    /// 稀缺指数：`mult = (coverage_rounds / 覆盖回合数)^price_alpha`。1.0 = 线性。
    pub price_alpha: f64,
    /// 价格下限（倍率）：严重过剩时最多折到这里（0.4 = 四折）。
    pub price_floor: f64,
    /// 价格上限（倍率）：断供时最高涨到这里（8.0 = 八倍）——「超高价」的上限。
    pub price_ceiling: f64,
    /// 「覆盖回合数」的下限：库存为 0 时用它代替，免得除零（0.5 = 半个月的用量）。
    pub cover_floor: f64,
    /// 需求滑窗的每回合更新比例（0.1 = 用 10% 的本回合消费修正滑窗）。
    pub demand_smoothing: f64,
    /// 需求地板（单位/回合）：低于它的资源视为「几乎没人消费」，价格不升不降（基价）。
    /// 否则一个没人用的矿会因为「库存 0」被永久顶在天价上。
    pub demand_min: f64,
    /// 友好门槛：某势力对另一势力的关系达到此值即算友好，享价格折扣。
    pub friendly_relation: f64,
    /// 友好折扣上限（0.15 = 最多打 85 折买对方的货）。
    pub friendly_price_discount: f64,
    /// 敌对加价上限（1.5 = 关系冷到交战边缘时，买对方的货要付 2.5 倍价）。
    /// 这让「关系」直接变成**成本**——同一个矿，向敌人买贵得多。
    pub hostile_price_markup: f64,
    /// **全面禁运**阈值：某势力对另一势力的关系 ≤ 此值即「根本不卖给你」（所有资源）。
    /// 比交战阈值更早生效：还没开打，供货就已经断了。
    pub embargo_relation: f64,
    /// **运费率**：每 1 AU 距离、按货值计的运费（0.02 = 30 AU 加价 60%）。
    /// 货物不是瞬移的——深空的货真的更贵（与 `economy-depth.md` 的「距离运费」同一条）。
    pub freight_per_au: f64,
    /// **穿越引力异常带的运费倍率**：运费按「浸入异常带的深度」再加这么多倍（见
    /// [`crate::sim::route_depth`]）。没有掌握 MOND 的势力只能花大价钱（或冒险）走这条线。
    pub mond_freight_mult: f64,
    /// **异常区丢货率**：非 master 的货物每「浸入异常带 1 AU」损失的货值比例
    /// （确定性比例，不是掷骰——掷骰会污染 `Prng` 流、破坏同种子复现）。
    pub mond_loss_per_au: f64,
    /// 丢货率上限（整批货最多损失到这个比例）。
    pub mond_loss_cap: f64,
    /// 异常带运费中**付给承运人**（掌握了 MOND 的 master）的比例：
    /// 只有 master 能可靠穿越异常带，于是它天然成为柯伊伯带的垄断承运人、对这条线上的
    /// 贸易抽税。其余部分（以及没有 master 存世时的全部）与手续费一样被**烧掉**（sink）。
    pub carrier_share: f64,
}
/// 光速治理 (lightspeed governance) tuning。
///
/// 以「距离首都」为代价的行政管理开销与忠诚度：一座城越远离其统治势力的首都，
/// 管理（通讯/后勤）越难。这给超大帝国一个自然的上限——既能管的领地有限，遥远的
/// 殖民地在治理不到位时也会因「离心叛乱」而丢失，从而让游戏在上千回合后依然是
/// 多方参与的格局，而不是收敛成少数几个永久霸权。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GovernanceConfig {
    /// 每座城每回合的治理开销基数（市场价值/回合）。
    pub admin_base: f64,
    /// 治理开销随距离（超过 [`Self::admin_range`] 的部分，单位 AU）递增的系数。
    pub admin_per_au: f64,
    /// 治理「可达」半径（AU）：处于该半径内的城市没有距离带来的额外开销。
    pub admin_range: f64,
    /// 治理到位（能付清全数管理费）时，城市忠诚度每回合向其距离目标恢复的比例。
    pub loyalty_recover: f64,
    /// 治理不到位（欠费）时，城市忠诚度每回合的下降量（乘以欠费比例）。
    pub loyalty_penalty: f64,
    /// 忠诚度「距离目标」随距离（超过 [`Self::loyalty_range`] 的部分）递减的系数：
    /// 目标忠诚 = clamp(1 - loyalty_distance * a, 0, 1)。
    pub loyalty_distance: f64,
    /// 忠诚度完全不受距离影响的半径（AU）。
    pub loyalty_range: f64,
    /// 忠诚度低于此值即爆发「离心叛乱」，城市被夷平为空白（可再殖民）。
    pub loyalty_revolt: f64,
    /// 治理「容量」：势力的总人口（跨所有活城）超过此值即进入「超载」，管理能力被
    /// 人口分散，使远距离治理更难（与距离叠加）。这给超大/人口稠密帝国一个自然上限。
    pub population_capacity: f64,
    /// AI 默认每座城每回合投入的「娱乐/福利」预算（市场价值/回合），用于提升忠诚度。
    /// 玩家可逐城覆盖（见 [`ControllableState::loyalty_budget`]）。
    pub default_entertainment: f64,
    /// 娱乐投入换算：投入「市场价值」使忠诚度目标 +1.0 所需的花费（即忠诚度来自娱乐
    /// 的增量 = paid / entertainment_cost）。
    pub entertainment_cost: f64,
    /// 迁都（自动控制）的评估周期（回合）：AI 每这么多回合重新评估一次首都选址；0 =
    /// 关闭会自动迁都，首都只在「亡城」时被强迁。确定性：只在证明候选更优（见
    /// [`Self::capital_relocate_threshold`]）时才动，避免反复横跳。
    #[serde(default = "default_capital_review_every")]
    pub capital_review_every: u32,
    /// 迁都须达到的「总治理距离成本」最小改善量（AU）：候选首都必须比当前首都低至少
    /// 这么多么才值得迁。太高=迁都太保守，太低=频繁微调。
    #[serde(default = "default_capital_relocate_threshold")]
    pub capital_relocate_threshold: f64,
    /// 首都「人口占比」带来的**全国忠诚度 buff**：首都人口占该势力全部活城人口的比例 ×
    /// 此系数 = 每座城的目标忠诚加成（0..1）。首都人口稠密 = 强引力/强心，全国向心力更强；
    /// 用占比而非绝对人口，是让「战略性地把首都放在人口中心」有真实收益。
    #[serde(default = "default_capital_share_loyalty_buff")]
    pub capital_share_loyalty_buff: f64,
    /// 迁都的**全国忠诚度代价**：旧首都人口占比 × 此系数 = 每座城的忠诚下降（迁离大都会
    /// 更动荡、动摇国本）。与首都 buff 互为反制：迁首都 = 用全国忠诚换治理距离收益，不是
    /// 免费优化——这保住光速治理给超大帝国的自然上限。
    #[serde(default = "default_capital_share_relocate_cost")]
    pub capital_share_relocate_cost: f64,
}

fn default_capital_review_every() -> u32 {
    12
}

fn default_capital_relocate_threshold() -> f64 {
    1.0
}

fn default_capital_share_loyalty_buff() -> f64 {
    0.2
}

fn default_capital_share_relocate_cost() -> f64 {
    0.5
}
/// 本土防御 (home-field defense) tuning——「首都即强弩」。
///
/// 在一座城距离其统治势力首都 `Faction::home_radius` AU 以内时，该势力获得本土
/// 防御：敌人对它造成的伤害乘以 `Faction::home_attack_mult`（<1 = 敌弱我强），它
/// 自己的舰获得额外 `Faction::home_regen_bonus` 的护甲再生。这让每个有首都的势力
/// 在自己的核心区显得难啃，超大国空降别人家里会付出代价——自然遏制一家独大。
/// cult（行星X崇拜教）的本土防御按 MOND 异常放大（见 `Faction` 的字段），使孤悬
/// 柯伊伯带的它在被围攻时能自保。

/// MOND / 柯伊伯引力异常 tuning。
///
/// 在「异常区」（距太阳超过 [`Self::radius`] 的深空）内，真实引力按 MOND（Modified
/// Newtonian Dynamics）修正，偏离标准牛顿假定。没有掌握 MOND 修正引力的势力（即除
/// [`Self::masters`] 之外的所有势力）在异常区内轨道计算错误，其指令坐标与实际到达
/// 坐标产生偏移——舰船无法精确机动到目标点，因而难以精确轰炸/殖民/停靠深处目标。
/// 这让 cult（掌握了 MOND 的势力）偏僻的柯伊伯带圣所成为天然堡垒：围攻者的舰队在
/// 那里「迷航」，而 cult 自己的舰指哪打哪。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MondConfig {
    /// 异常区起始半径（距太阳，AU）：超出此距离进入「柯伊伯异常区」。
    pub radius: f64,
    /// 非 master 势力在异常区内，每超出 [`Self::radius`] 1 AU 的**最大**导航偏移量（AU）。
    /// 实际偏移是伪随机的（见 [`Self::drift_shape`]），所以它是上界而不是定值。
    pub drift_per_au: f64,
    /// 伪随机偏移的分布形状：`偏移 = 上界 × roll^drift_shape`（`roll ∈ [0,1)` 均匀）。
    ///
    /// 单次尝试的成功率 = `P(偏移 ≤ arrival_eps)`：
    /// * `1.0`（默认）→ 均匀分布，`p = eps/(depth×drift)`：伊克西翁 0.92、妊神星 0.29、创神星 0.20；
    /// * `< 1` → 偏移**偏向大值**（更常在远处迷航）：`p = (eps/(depth×drift))^(1/shape)`，
    ///   `0.5` 时创神星降到 0.04（≈26 回合才蒙对一次）——深处更难，但**永远不是 0**；
    /// * `> 1` → 偏向小值（更好到）。
    ///
    /// 这是「带内到底有多难」的总旋钮：往小调 = 圣所与柯伊伯矿更难被外人碰到。
    pub drift_shape: f64,
    /// 掌握了 MOND 修正引力的势力 id（在异常区内无导航偏移）。
    pub masters: Vec<FactionId>,
}
/// 合纵连横 / 均势外交 (balance-of-power) tuning——「弱者联盟对抗霸权」。
///
/// 当某个势力的综合实力占比（按其在星系内的城市数 + 舰队质量加权）达到
/// [`Self::hegemon_power`] 时，它被判定为「霸权」。此时其余的较弱势力被同一个
/// 「共同威胁」推向彼此：
///   * 弱者-弱者之间的关系向 [`Self::coalition_affinity`] 靠拢（合纵——弱者团结）；
///   * 弱者对霸权的关系向 [`Self::hegemon_affinity`] 靠拢（均势——联手制衡最强者）。
/// 这天然产生「一家独大 → 众人围剿」的政治动力学，让上千回合的博弈不至于收敛成
/// 「少数永久霸权 + 一堆旁观者」，而是维持多方参与。
///
/// 全部确定性、数据驱动：速率与目标都在配置里，agent 可调；不引入任何未播种随机。
/// 此外当霸权对某一弱者开战时，其余弱者对霸权的关系施加 [`Self::collective_defense_delta`]
/// 的骤降——「攻其一方 = 与全体为敌」的集体安全反应。把 [`Self::hegemon_power`] 设成
/// >1 即整体关闭此机制（默认关闭，靠 config/game.ron 开启）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BalanceOfPowerConfig {
    /// 霸权判定阈值：某势力的综合实力占比（0..1）达到此值即被视为「霸权」并引发弱者
    /// 反制；>1 则关闭整个机制。
    pub hegemon_power: f64,
    /// 综合实力权重：城市数（领土/经济基础）在实力占比中的相对权重。
    pub power_city_weight: f64,
    /// 综合实力权重：舰队质量（各舰引擎数值之和）在实力占比中的相对权重。
    pub power_fleet_weight: f64,
    /// 弱者-弱者关系向此值靠拢（联盟目标亲和；彼此友好/结盟）。
    pub coalition_affinity: f64,
    /// 弱者-弱者向联盟靠拢的每回合比例（0..1）。
    pub coalition_rate: f64,
    /// 弱者对霸权的关系向此值靠拢（目标亲和；即疏远/敌意）。
    pub hegemon_affinity: f64,
    /// 弱者对霸权靠拢的每回合比例（0..1）。
    pub hegemon_rate: f64,
    /// 集体安全：霸权对任一弱者开战时，其余弱者对霸权的关系骤降幅度。
    pub collective_defense_delta: f64,
    /// 一个「联盟」至少需几个成员（不含霸权）才算成立，用于事件/上报判定。
    pub min_members: usize,
    /// 弱国「倒向联盟」的疏远阈值：某弱者对霸权的关系 ≤ 此值即视为已加入反制联盟
    /// （被遏制/疏远了霸权、转而与弱国抱团）。与交战阈值（war_threshold）无关——
    /// 遏制是冷战式的「疏远 + 经济封锁」，不必然导致开战。
    ///
    /// **这是经济封锁的判据**：已倒向联盟的弱者 ↔ 被锁定的霸权之间**全面禁运**
    /// （见 `sim::trade_blocked`）——取代了旧的 `sanction_trade_mult`（那只是「少卖一点」，
    /// 现在是真的「不卖给你」）。
    pub coalition_estrange: f64,
    /// 经济制裁的「治理代价」：被封锁的霸权维持帝国（行政 + 娱乐/福利）的成本倍率。
    /// >1 表示被孤立/封锁的霸权要把更多稀缺资源中转去维持领地与治安，导致**远端/边缘
    /// 殖民地更难养、更易离心叛乱**——把「多国资源封锁」转化为「霸权扩张受限」，让
    /// 一家独大的体量自然回落。这个倍率只作用于被反制联盟锁定的霸权（不碰其他国家）。
    pub sanction_cost_mult: f64,
}

impl Default for BalanceOfPowerConfig {
    /// 默认完全关闭合纵连横（`hegemon_power` 设为 >1，永不判定霸权），使不含
    /// `balance` 字段的旧配置照常工作。
    fn default() -> Self {
        Self {
            hegemon_power: 2.0,
            power_city_weight: 1.0,
            power_fleet_weight: 1.0,
            coalition_affinity: 30.0,
            coalition_rate: 0.08,
            hegemon_affinity: -50.0,
            hegemon_rate: 0.10,
            collective_defense_delta: -15.0,
            min_members: 2,
            coalition_estrange: -10.0,
            sanction_cost_mult: 1.6,
        }
    }
}
/// 历史层（[`crate::model::Milestones`] 里程碑层 / [`crate::model::Notables`] 窗口层）的容量配置。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct HistoryConfig {
    /// 里程碑层最多保留多少条（`0` = 不设上限，默认）。超出时丢弃**最旧**的记录，并把丢弃量与
    /// 丢弃到的回合记进本层自身（[`crate::model::Milestones::dropped`]）——截断可见。
    #[serde(default)]
    pub max_milestones: usize,
    /// 窗口层保留多少个回合（默认 24 = 2 年；`0` = 不裁剪）。
    ///
    /// 与 `max_milestones` 的 `0` 同义（无损），但**语义相反的那一头值得注意**：这里被裁掉是
    /// **预期行为**，不是损失——出了窗口的历史按判据就不该再影响任何计算。
    /// 设成 `0` 等于让窗口层退化成无限长存，那说明判据被绕过了（真要无限长存应提升进里程碑层）。
    #[serde(default = "default_notable_window")]
    pub notable_window: usize,
}

fn default_notable_window() -> usize {
    24
}

/// 思潮（Ideology）驱动 tuning——4 条轴逐回合按「变化因素」向信号 target 靠拢。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IdeologyConfig {
    /// 每回合思潮向信号 target 靠拢的比例（0..1）。
    pub drift_rate: f64,
    /// 军事净信号（敌舰被击毁+夷平敌城 − 我舰被击毁−我的城损失）换算为「军国」target 的系数。
    pub military_scale: f64,
    /// MOND 接触换算：（开采异常区城数 − 到访异常区舰数）× 此系数 = 技术/科学 target。
    pub mond_scale: f64,
    /// 经济净流（产出 − 维护 − 治理）换算为「精英/人民」target 的分母（净流 / 此值归一化）。
    pub economy_scale: f64,
    /// 人均面积参考（面积/人口）：高于它 → 自然，低于它 → 殖民。
    pub area_ref: f64,
    /// 人均面积差换算为「殖民/自然」target 的系数。
    pub area_scale: f64,
    /// 思潮「优势端」的自平衡 debuff（见 [`IdeologyDebuffConfig`]）。`#[serde(default)]` 容忍旧配置无此节。
    #[serde(default)]
    pub debuff: IdeologyDebuffConfig,
}

/// 思潮优势端的**自平衡 debuff** tuning——针对单极化（某优势思潮跑成一家独大）。
///
/// 机制：当某势力处于**优势端思潮**（军国/科学/精英/殖民）时，它会被一个**全国忠诚度
/// 惩罚**打击——惩罚力度随**该势力在星系中的体量（活城人口占比，单极化坐大）连续上升**
/// （`dom`），再乘上该势力「思潮 vs 行为不符」的连续程度（`violate`）。因此打击随单极化
/// 持续、不随「行为一致化」消退。全函数用 `smoothstep` 连续成形，**无硬阈值断点**；且
/// 打击只对「优势端思潮」生效，非该端势力不受影响（自指向，不误伤中立/对立端）。
///
/// 四条轴的「优势端 / 不符判定」：
///   * 和平↔军国 → 优势端=**军国**(+)；不符 = 不战争 且 低军事实力占比。
///   * 科学↔技术 → 优势端=**科学**(−)；不符 = 舰在 MOND 异常区占比低。
///   * 人民↔精英 → 优势端=**精英**(+)；不符 = 精英（坐大体量由 `dom` 承担）。
///   * 自然↔殖民 → 优势端=**殖民**(+)；不符 = 不殖民。
///
/// 忠诚度惩罚 = `max_loyalty_penalty × clamp01(dom × Σ_轴 w_轴 × violate_轴)`，再作为每城
/// 忠诚目标的扣减（`target_eff −= penalty`）。`smoothstep(a,b,x)` 为 C¹ 连续斜坡：
/// `t=clamp01((x−a)/(b−a))`，`t²(3−2t)`。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IdeologyDebuffConfig {
    /// 单势力**总忠诚度惩罚上限**（0..1，每回合从忠诚目标里扣的最大比例）。
    pub max_loyalty_penalty: f64,
    /// 体量斜坡**下界**：该势力活城人口占全星系比例低于此 = 完全不起效。
    pub gate_lo: f64,
    /// 体量斜坡**上界**：占比达此 = 对该势力全量惩罚（单极化坐大即被打）。
    pub gate_hi: f64,
    /// 战争强度平滑斜坡带宽（AU·关系档）：该势力与敌关系越低于交战阈值，战争强度越贴近 1。
    pub war_band: f64,
    /// 军事占比平滑中枢：某势力军事占比低于此 → 「低军事」不符度趋近 1。
    pub mil_share_ref: f64,
    /// 各轴惩罚权重（0..1），按轴微调力度。
    pub w_military: f64,
    pub w_science: f64,
    pub w_elite: f64,
    pub w_colony: f64,
}

impl Default for IdeologyDebuffConfig {
    fn default() -> Self {
        Self {
            max_loyalty_penalty: 0.20,
            gate_lo: 0.06,
            gate_hi: 0.30,
            war_band: 10.0,
            mil_share_ref: 0.30,
            w_military: 1.0,
            w_science: 1.0,
            w_elite: 1.0,
            w_colony: 1.0,
        }
    }
}

impl Default for IdeologyConfig {
    fn default() -> Self {
        Self {
            drift_rate: 0.05,
            military_scale: 0.5,
            mond_scale: 0.4,
            economy_scale: 25.0,
            area_ref: 0.15,
            area_scale: 8.0,
            debuff: IdeologyDebuffConfig::default(),
        }
    }
}

/// **雇佣运力市场**（集货腿的第二条路：请人来运）tuning。
///
/// 机制见 `.agents/notes/freight-collection.md` §4。**单子以雇佣船的形式**（用户裁决）：
/// 雇主挂「我这条线要多少运力」，受雇方自己派船（派几条都行），雇主周期性考核受雇方的
/// **实测吞吐**并反馈信誉，再按信誉决定续约还是换人；受雇方按自己的运力决定接不接、
/// 要不要提前结束。裁决：承运人**不赔货值、只掉信誉**（Q1(b)）、信誉是**势力级的全局单值**
/// （Q3/P4）、报酬是**抽成制**（Q10）。
///
/// # 这里只剩下「口径」，没有一个市场价
///
/// 用户裁决「**一切数值都用动态平衡/博弈来产生**」，所以这一节刻意**不含任何猜出来的
/// 市场量**：要求运力、考核周期、固定期全部从**航程**算（[`crate::model::hire_terms`] /
/// [`crate::model::required_throughput`]），抽成由市场自己抬（[`Self::share_escalation`]），
/// 续约与解约由双方各自按自己的处境掷骰。留下的常数只回答**口径**问题：
/// 「一条参考船」是什么（[`Self::nominal_hold`] / [`Self::reference_speed`]）、
/// 雇几个来回（[`Self::hire_trips`]）、雇主验货有多宽严（[`Self::review_width`]）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FreightConfig {
    /// **开叫价**：承运人的抽成比例（0.15 = 交付时从货里自留 15%，其余进雇主首都池）。
    /// 它是 Q10 的机制落点：**没有货币转移**，报酬就是承运人没交出去的那部分货。
    ///
    /// 这只是**起叫**：没人接的单子会一路抬价（见 [`Self::share_escalation`] / [`Self::share_max`]），
    /// 所以「难单该报多厚」不由配置决定，而由市场自己找出来。
    pub share: f64,
    /// **加价档**：一张没人接的单子过了**一个考核周期**还没人接，抽成乘这个系数
    /// （1.12 = 每次 +12%）。
    ///
    /// 这是「难单自然报厚」的机制：近地小单开叫就被抢走（价格留在开叫价），深空没人接
    /// ⇒ 价格一路抬到有人点头。**上限**见 [`Self::share_max`]。
    /// 抬价的节拍从**考核周期**来（= 这条线一个往返），不是另设一个「多久没人接就加价」的数
    /// ——同一件事只有一把尺子。
    pub share_escalation: f64,
    /// **抽成上限**：抬价抬到这里就不再抬（0.5 = 最多把一半的货给承运人）。
    /// 过了这条线还雇不到人，雇主宁可让货烂在产地——这也是给「自己造船运」留的价值。
    pub share_max: f64,
    /// 参考**巡航速度**（AU/回合）：把「往返距离」折成回合数（`往返 ≈ 2 × 距离 ÷ 它`）。
    ///
    /// 为什么用**参考**速度而不是受雇方的真实速度：挂单时还不知道谁会来接，而条款必须
    /// **在挂单那一刻定死**（否则同一张单的条件会随接单者漂移，确定性就没了）。取参考值
    /// 意味着「慢船/没掌握 MOND 的船接远单」会真的达不到要求——那是**设计要的**筛选
    /// （设计 D：垄断表现为时间优势），不是漏洞。
    pub reference_speed: f64,
    /// **一条参考船的舱容**（单位/趟）。它是「要求运力」的分子：
    /// `要求运力 = nominal_hold ÷ 参考往返回合数`（见 `model::required_throughput`）。
    ///
    /// 取舰队里最常见的舱容（驱逐 4 / 巡洋 6 / 战列 6 / 航母 20），**不取航母的 20**：
    /// 那等于要求每个雇主都雇到一条航母。
    ///
    /// 它同时是**雇主风险暴露**的口径（[`crate::model::required_reputation`]）：雇佣形态下
    /// 押在陌生人手里的每一刻最多是**一个货舱**的货值——不再随积压量暴涨。
    pub nominal_hold: f64,
    /// **固定雇佣期 = 几个考核周期（来回）**。到期雇主按信誉决定续约还是换人。
    ///
    /// 用户裁决：「期限是固定期」。用一个**来回数**而不是写死的回合数，是因为「多长时间
    /// 算一期」在不同航线上差一个数量级（金星↔地球 2 回合，柯伊伯带几十回合）——写死
    /// 会让深处连一个来回都跑不完就到期。
    pub hire_trips: u32,
    /// **门槛基准**：难度为 0、货值近乎为 0 的单，也要的最低信誉。
    /// 门槛 = `gate_base + gate_slope × (难度 + 货值份数)`，见 [`crate::model::required_reputation`]。
    pub gate_base: f64,
    /// 门槛对「难度 + 货值份数」的斜率。
    pub gate_slope: f64,
    /// 门槛的**软化宽度**：合格度 = `σ((信誉 − 门槛) ÷ gate_width)`——低信誉者**极少**
    /// 被选中，而不是数学上绝无可能（遵 `AGENTS.md`：不设断崖）。越小越接近硬门槛。
    ///
    /// **到期续约用的是同一个闸**：雇主不需要为「换人」再发明一个阈值——「你当初是怎么
    /// 被选上的，现在就按同一条线续」。
    pub gate_width: f64,
    /// **难度**的量纲参照：往返期望这么多回合 = 1 份难度。
    pub difficulty_rounds_ref: f64,
    /// **货值**的量纲参照：这么多「市场价值」的货 = 1 份风险（一个货舱的货值进门槛，
    /// 因为 Q1(b) 下承运人不赔货值 ⇒ 押在陌生人手里的是**雇主**的货）。
    pub risk_value_ref: f64,
    /// 信誉的**影子价格** `λ(信誉)` 的上限（信誉极低时会接近它）。
    /// `λ` 随信誉**单调递减**：信誉越低越稀缺 ⇒ 越舍不得拿它去冒险（§C1 的那条循环）。
    ///
    /// **量纲很重要**：λ 的单位是「一点信誉值多少市场价值」，所以它的合理量级是
    /// `一期的报酬 ÷ 一期的信誉增量`（报酬 10 上下、增量 0.08 上下 ⇒ 百级）。
    /// 若把它写成 1.0 那样的"小数字"，信誉项在报酬面前**完全不可见**，这套阶梯就白写了。
    pub shadow_lambda0: f64,
    /// `λ` 的中点信誉。
    pub shadow_mid: f64,
    /// `λ` 的软化宽度。
    pub shadow_width: f64,
    /// **接单 σ 的温度**：接单概率 = `σ(决策值 ÷ 温度)` ——**略亏的单也会有人接**
    /// （性格不同的承运人温度不同），而不是「净收益 ≥ 0 才接」的一刀切。
    pub match_temperature: f64,
    /// **考核幅度**：一次考核给信誉加减多少（好/坏各一次，见 `autocontrol::contract::review_contract`）。
    ///
    /// 旧形态里它叫「按时交付的增量」，因为涨信誉挂在交付上；雇佣形态下**信誉只由考核产生**
    /// （用户：「每个雇主会周期性评估受雇方的运力是否达标来反馈对受雇方的信誉」）。
    pub reputation_gain: f64,
    /// **考核的宽严**（达标率的 σ 宽度）：达标概率 = `σ((达标率 − 1) ÷ review_width)`。
    ///
    /// 越小越严格（差一点就算不达标）。它是**雇主性格**的位置：宽容的雇主器重稳定，
    /// 苛刻的雇主换人频繁。留着不调死的理由与其它 σ 一样：硬阈值会让「恰好达标」与
    /// 「差一点」之间出现断崖，而现实里雇主的耐心是连续的。
    pub review_width: f64,
    /// **受雇方的抽手闸**：自己缺几条船时，它才认真考虑撤回受雇在外的运力。
    ///
    /// 解约概率 = `σ((自家缺口 − recall_bar) ÷ recall_width)`。`recall_bar = 1.0` 的语义是
    /// 「**缺一条船**就是五五开」——它不是一个调出来的数，而是「一条船就是一条船」：
    /// 缺得越多越坚决，不缺时也偶有抽手（船被打残、航线变远，都是局部信息）。
    pub recall_bar: f64,
    /// 抽手闸的软化宽度（缺一条船上下多少才叫「差不多」）。
    pub recall_width: f64,
    /// 信誉上限（下限恒为 0：信誉不能是负数——它没有「欠债」的语义）。
    pub reputation_max: f64,
}

impl Default for FreightConfig {
    fn default() -> Self {
        Self {
            share: 0.15,
            share_escalation: 1.12,
            share_max: 0.5,
            reference_speed: 1.0,
            nominal_hold: 6.0,
            hire_trips: 3,
            gate_base: 0.6,
            gate_slope: 0.5,
            gate_width: 0.25,
            difficulty_rounds_ref: 6.0,
            risk_value_ref: 60.0,
            shadow_lambda0: 120.0,
            shadow_mid: 1.4,
            shadow_width: 0.6,
            match_temperature: 2.0,
            reputation_gain: 0.08,
            review_width: 0.35,
            recall_bar: 1.0,
            recall_width: 0.5,
            reputation_max: 5.0,
        }
    }
}

/// The whole game configuration, loaded from `config/game.ron`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameConfig {
    pub economy: EconomyConfig,
    pub combat: CombatConfig,
    pub diplomacy: DiplomacyConfig,
    pub market: MarketConfig,
    /// **承包市场**（托运方挂单、承运方接单）。`#[serde(default)]` 容忍旧配置无此节
    /// （默认值 = 抽成 15%、宽裕的截止期）。
    #[serde(default)]
    pub freight: FreightConfig,
    pub governance: GovernanceConfig,
    pub mond: MondConfig,
    /// 合纵连横 / 均势外交（弱者联盟对抗霸权）。`#[serde(default)]` 容忍旧配置无此节。
    #[serde(default)]
    pub balance: BalanceOfPowerConfig,
    /// 思潮（可变化意识形态）驱动 tuning。`#[serde(default)]` 容忍旧配置无此节。
    #[serde(default)]
    pub ideology: IdeologyConfig,
    /// 长存里程碑历史的容量。`#[serde(default)]` 容忍旧配置无此节（默认 0 = 无损）。
    #[serde(default)]
    pub history: HistoryConfig,
    /// Resource definitions (key -> display metadata). This is the source of
    /// truth for which resource keys exist.
    pub resources: BTreeMap<String, ResourceDef>,
    /// Building-structure definitions (混凝土 / 钢结构). Source of truth for
    /// which structure keys exist.
    pub structures: BTreeMap<String, StructureSpec>,
    /// 天体/行星**类型**表（`BodyKindSpec`）：视觉与类型元数据（颜色/尺寸/类别/星环/着色器
    /// 分支）。每个 [`Body`] 只存一个 **类型 key**（`Body::kind`），由前端经 `/api/meta` 读本表
    /// 解析出实际视觉。纯展示数据。`#[serde(default)]` 容忍旧配置无此节（天体退回默认类型）。
    #[serde(default)]
    pub body_kinds: BTreeMap<String, BodyKindSpec>,
    /// Ship statistics, keyed by class name.
    pub ships: BTreeMap<String, ShipSpec>,
    /// Ship components (舰船定制组件), keyed by id. `#[serde(default)]` 容忍旧配置无此节。
    #[serde(default)]
    pub components: BTreeMap<String, ComponentSpec>,
    /// Building statistics, keyed by building kind name.
    pub buildings: BTreeMap<String, BuildingSpec>,
    /// 剧情事件表（编年史/叙事弧）。`#[serde(default)]` 容忍旧配置无此节。
    #[serde(default)]
    pub story: Vec<StoryEvent>,
    /// 每势力的「名字库」（舰船等实体取名用）：faction 名 → 候选名字数组。
    /// 「名字即唯一 key」——势力所属实体从本库中**确定性取一个唯一名**。`#[serde(default)]`
    /// 容忍旧配置无此节（缺库的势力退回通用名，见 [`GameConfig::ship_pool`] / [`ship_display_name`]）。
    #[serde(default)]
    pub name_pool: BTreeMap<String, Vec<String>>,
}

impl GameConfig {
    /// 某势力的名字库（按势力名查）。库里为空/未配置时返回空切片——
    /// 由 [`ship_display_name`] 退回到「舰-{序列}」这种唯一但无含义的名字。
    pub fn ship_pool(&self, faction_name: &str) -> &[String] {
        self.name_pool.get(faction_name).map(Vec::as_slice).unwrap_or(&[])
    }
}

impl GameConfig {
    pub fn ship_spec(&self, class: &str) -> &ShipSpec {
        self.ships
            .get(class)
            .expect("game config is missing a ship class")
    }

    pub fn component_spec(&self, id: &str) -> &ComponentSpec {
        self.components
            .get(id)
            .expect("game config is missing a ship component")
    }

    pub fn building_spec(&self, kind: &str) -> &BuildingSpec {
        self.buildings
            .get(kind)
            .expect("game config is missing a building kind")
    }

    pub fn structure_spec(&self, structure: &str) -> &StructureSpec {
        self.structures
            .get(structure)
            .expect("game config is missing a building structure")
    }

    pub fn structure_name(&self, key: &str) -> String {
        self.structures
            .get(key)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| key.to_string())
    }

    /// Look up a body/planet type spec by its kind key. `None` when the kind is
    /// unknown (e.g. an old `.ron`/config without a configured type) — the front
    /// end falls back to a generic spec. The engine simulation never reads this.
    pub fn body_kind(&self, key: &str) -> Option<&BodyKindSpec> {
        self.body_kinds.get(key)
    }

    /// A body/planet type's label, falling back to the key itself when unknown.
    pub fn body_kind_name(&self, key: &str) -> String {
        self.body_kinds
            .get(key)
            .map(|s| s.label.clone())
            .unwrap_or_else(|| key.to_string())
    }

    pub fn resource_name(&self, key: &str) -> String {
        self.resources
            .get(key)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| key.to_string())
    }
}
