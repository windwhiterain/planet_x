use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::control::resolve_chain;
use super::faction::default_capital_body;
use crate::model::*;

/// The current persisted `State` schema version. Bump this whenever `State`'s
/// field structure or semantics change, and add a matching arm to [`migrate`] so
/// old `.ron` files are explicitly upgraded — or clearly rejected as "too new" —
/// instead of being silently loaded under new semantics.
/// **v13 = 两条独立历史的汇合点**（设计图分支曾用 10、承包市场分支曾用到 12）：
/// 合并之后取 **13**，且 `migrate` 把 **10..=12 整段**都当成「设计图/承包市场之前的世界」
/// 处理——见 [`migrate`] 的 `v10..=12` 一档（那一段里同一个号在两条历史中含义不同，
/// 所以不能按号细判，只能整段按最保守的方式接）。
/// **v14–v16 是被两条历史各自用过的号**（合并时逐次发现，故合并后取 **17**）：
///
/// | 号 | `main` 那条线 | `feature/tech-system-mond` 这条线 |
/// |---|---|---|
/// | v14 | `feature/step-intermediates-b1`：读面追加治理/忠诚中间量（`State` 没动）；再往前 v14 还被 `feature/pre-post-unify`（派生读面换代）用过 | **MOND 掌握度连续化**：`Faction::mond_control` 取代 `config.mond.masters` 名单（**只有这一条真动了 `State`**） |
/// | v15 | `feature/capital-decisions`：`FactionRow.capital` 搬进 `RoundDecisions`（读面，`State` 没动） | 与 main 的读面换代合并之后的号 |
/// | v16 | 中性值一处声明（读面缺省值，`State` 没动） | 与 step-intermediates 合并之后的号 |
///
/// 同一个号在多处含义不同，**号本身已经不能再判语义** ⇒ 合并后取 **17**，
/// `13 | 14 | 15 | 16` **整段只推号**：
///
/// * 这几档里只有**本支那条线的 v14/v15 档**真的存过 `mond_control`——那是本支自己的档，
///   键在结构里，读进来照旧生效（**不丢**）；**v13 及更早、以及 main 那条线的档**里压根没有
///   这个键 ⇒ serde 缺省 **0（凡人）**，**不保真**（用户裁决：不考虑向前兼容；那些档只存在于
///   本次开发的 worktree 里，没有真实损失）；
/// * 两条历史动过的**派生读面**量本来就不持久（每回合重算），推号即可。
///
/// **v17 又被撞了一次**（第三次）：`feature/b2-money`（「钱去哪了」的中间量）与上面那个 17
/// 是同号不同义——它**只动派生读面**（各势力实际花掉的投资/建造预算、欠付维护费与生锈比例；
/// 各城用工系数、住房容量、是否集散地、每舰级造舰速率与实得进度），`State` 一个字段没动。
/// 于是合并后取 **18**，`13 | 14 | 15 | 16 | 17` **整段只推号**（17 那一档：main 线的档没有
/// `mond_control` 键 ⇒ 缺省 0，与本支 v14/v15 档的处理同一条约定）。
///
/// **v18 = 「钱去哪了」的中间量**（`feature/b2-money`）：派生读面增列（各势力实际花掉的投资/建造
/// 预算、欠付维护费与生锈比例；各城用工系数、住房容量、是否集散地、每舰级造舰速率与实得进度），
/// `State` 一个字段没动。
/// **v19 = 「市场与运输」的中间量**（`feature/b3-market`）：派生读面再添两片——本回合**真的成交的
/// 贸易**（一笔一对一行：价格分解 + 丢货率）与**每艘在跑运输的舰走了哪一步**；另加各势力的
/// 购买力/买方名次、每一处货栈的运力账，`FactionRow.trade_blocked_by` 从计数升级成「名单 + 三档
/// 原因」。`State` 仍然一个字段没动。
/// **v20 = 战斗中间量进事件层**（`feature/b4-combat`，用户裁决 Q1 走 (b)）：`GameEvent::Attack`
/// 长出 `shots`（逐发明细：选择三项分 + 命中/点防/护盾/护甲/破甲），**并且 0 伤害的齐射也发**
/// （「被点防吃光」此前什么事件都不留）。这一档**真的动了 `State`**（`State::events` 是持久字段）
/// ⇒ 旧档里的 `Attack` 事件靠 `#[serde(default)]` 补成空 `shots`（那些档只看聚合量，不失真）。
/// **v21 = 输入面（B5）**：`pre` 从「回合开始的观测」（一份 `RoundView` 副本，零信息量）换成
/// **输入面** [`RoundInputs`]（掷出的随机数 + 判定输入；C7 解算顺序 / C13 关系噪声已接）。
/// `State` 一个字段没动，但**档的形状变了**：旧档的 `pre` 里是观测，新档里是输入——按本仓库
/// 的惯例（读面/档的形状变化也推号，见 v18/v19 那两档）推号，让「旧档在这一面上不保真」明摆着。
/// **v22 = 指令只剩逐舰叶；图改带倾向**（`feature/blueprint-stance`，用户裁决 2026-10）：
/// * **删** `ControllableState::default_ship_order`（舰队默认指令）与 [`Blueprint::order`]；
/// * **加** `Blueprint::{doctrine, kiting, role}`（图能表态的是**长期倾向**，不是指令），
///   三条风格链变成 `叶 → 出厂图 → 舰队默认 → 舰上记录值`。
///
/// **这一档旧档会丢东西**（不迁移）：旧档里那两片叶**没有等价物**——`default_ship_order`
/// 的价值（"全舰队一条站桩令"）正是被裁决删掉的东西，而图上的 `order` 换成了三条倾向轴、
/// 语义不同（不是改名）。按本仓库「不考虑向前兼容」的约定：旧 `.ron` 里那两个键**会被
/// serde 忽略**（`Blueprint` 多出的三轴 `#[serde(default)]` ⇒ `None` = 图对倾向沉默），
/// 于是**旧档能加载**，但**旧的指令归属会回到作用域链**（`Auto`）：那些依赖"全舰队默认"
/// 的局面会在这一档改变行为。要让旧局面对齐，把指令**逐舰**重写一遍（或在图上写角色）。
/// **v24 = 国内市场（第一版）**：`State::market`（[`MarketState`]）新增
/// `domestic: BTreeMap<FactionId, DomesticMarket>`，存每势力的开发/建造国内价格与未用额度。
/// 旧档缺该键 ⇒ serde default 空表 = 未启用；`config.domestic_market.enabled` 默认 `false`，
/// 世界逐字节不变。见 `.agents/notes/domestic-market.md`。
/// **v25 = 控制面 + 读面连接键的中文名**（`feature/control-nouns`，用户裁决 2026-10）：字段的
/// serde 名就是给人看的中文名词（批次 A 已把实体字段改完，这一档把**控制面**与**投影的连接键**
/// 补上）：`ControllableState` 的 17 片叶、`FactionControlPatch` 的 17 片叶 + `势力`/`建筑`、
/// `leaves.rs` 的 `field`/`keys`/`values`/`carries`/`read_only`、`projection` 的
/// `key`/`id_col`/`join_on`（`舰名`/`城名`/`势力`/`图名`/`合同号`/`天体名`/`定居点` 与 `*表`）。
///
/// **档的形状变了**（同一个世界的 JSON 键名不同）⇒ 推号：旧档里的英文键按新名找不到
/// （**不迁移**，按本仓库「不考虑向前兼容」的约定）。**语义与随机流一律不变**：
/// `--seed 42 --round 240 --digest 20` 的 sha256 逐字节相同（改的只是名字）。
/// 见 `.agents/notes/field-naming.md`。
///
/// **v26 = 事件载荷字段的中文名**（`feature/event-nouns`，用户裁决 2026-10，第 10 步）：批 B 的
/// 事件词汇——[`GameEvent`] 每个变体的载荷字段、[`Shot`]、[`Killer`] 全部加
/// `#[serde(rename = "中文名")]` 与 `///` 解释（`--nouns` 的 `state` 那一半因此把这批名字与
/// 解释一起发出去），投影 `events` 表的 `data` 载荷键（`GameEvent::history_row`）同步换成
/// 同一批中文名。**判别键 `type` 与变体标签（`attack`/`city_razed`…）逐字不变**——它们是线上
/// 格式与词表枚举，不是显示名词（见 `.agents/notes/field-naming.md` §8）。
///
/// **档的形状变了**（事件载荷的 JSON 键名不同）⇒ 推号：旧档里那些英文键按新名读不到
/// （**不迁移**，按本仓库「不考虑向前兼容」的约定；旧档能加载，只是事件载荷的键名对不上）。
/// **语义与随机流一律不变**（改的只是名字，`derived_roll` 的盐一个字没动）。
pub const SCHEMA_VERSION: u32 = 26;
fn default_schema_version() -> u32 {
    0
}
/// **完整的世界快照**（一个回合的世界状态）。
#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
pub struct State {
    /// 状态/schema 版本。每次改动 `State` 的**语义/字段结构**时递增（见 [`SCHEMA_VERSION`]
    /// 与 [`migrate`]）；旧 `.ron` 缺该字段时 serde default 为 0，由 [`migrate`] 逐档升级，
    /// 避免「旧档案被按新语义静默错载」。
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// 当前回合（从 0 起：回合 0 = 初始世界，之后每步 +1）。
    pub round: u32,
    /// 已流逝的总月数（一回合 = 一个月；= `round`，用浮点便于与时间序列计算）。
    pub time_month: f64,
    pub bodies: Vec<Body>,
    pub cities: Vec<City>,
    pub factions: Vec<Faction>,
    pub ships: Vec<Ship>,
    /// 各势力可控状态（指令控制量的集合），随状态一起序列化。
    pub control: BTreeMap<FactionId, ControllableState>,
    /// 城市/天体/势力/全局 的控制作用域树：谁负责 AI 决策、谁收玩家指令。
    pub scope: ControlScope,
    /// 本回合事件日志（`#[serde(default)]` 以便旧状态/旧 .ron 加载时缺字段不报错）。
    ///
    /// 这是**回合内明细层**：每回合被清空、不累计，只描述「这一回合发生了什么」。事件本身
    /// 自带因果（谁被谁击毁、哪艘舰夷平了哪座城、城从谁手里易主），所以 agent 不必反推状态差。
    /// 「跨回合的历史」由投影层承担：`planet_x --index` 把每个回合的事件归一化成
    /// `idx/events.jsonl`（一行一事件、固定列、统一参与方槽位），Python kit 用
    /// `q.history('city', 城名)` / `q.cause('ship', 舰名)` 做 join 查询。
    #[serde(default)]
    pub events: Vec<GameEvent>,
    /// **长存里程碑层**：后续计算需要访问**无限过去**的事件，按发生顺序累计、**不随回合清空**。
    /// 只收 [`Salience::Milestone`]，而按当前判据**没有任何 variant 属于这一层**——所以它现在
    /// 是空的（见 [`Milestones`] 的文档：这是判据的正确结果，不是遗漏）。
    #[serde(default)]
    pub milestones: Milestones,
    /// **窗口层**：后续计算需要访问**一定事件窗口**的事件，只保留最近
    /// `history.notable_window` 个回合。当前唯一的成员是战争（[`GameEvent::WarStarted`] /
    /// [`GameEvent::WarEnded`]），读者是「记恨地板」（[`crate::sim::war_scar_floor`]）。
    ///
    /// 与 [`State::milestones`] 的差别只有保留期：本层被裁剪是**预期行为**，因此不记 `dropped`。
    #[serde(default)]
    pub notables: Notables,
    /// 剧情编年史：本局已发生的叙事事件（按发生先后追加）。这是「剧情丰富」的载体——
    /// agent 用 `story` 命令/查询即可读到整段已展开的故事弧；`#[serde(default)]` 让旧的
    /// `.ron` 状态缺字段也能正常加载。
    #[serde(default)]
    pub chronicle: Vec<ChronicleEntry>,
    /// 每势力「已命名舰只」的单调计数器（名字库轮转、保证舰名唯一且不复用）。
    /// 纯显示用（不参与战斗/经济语义）；`#[serde(default)]` 让旧档缺字段也能加载。
    #[serde(default)]
    pub ship_name_seq: BTreeMap<FactionId, u64>,
    /// **建筑 id 的单调计数器**（**永不复用**）。
    ///
    /// `Building` 是唯一没有名字的实体（地址 = `(城名, 建筑序号)`，见
    /// [`BuildingId`](crate::model::BuildingId) 与 `.agents/notes/name-as-unique-key.md` §2），
    /// 所以那个序号就是它的身份。以前分配器是「每回合扫全场取 `max(id) + 1`」——**最高 id 的
    /// 建筑一被拆，下个新建筑就拿回那个号**：此刻不会撞号（活着的建筑唯一），但把 id 当**长期
    /// 引用**（跨回合的 UI 选中态、agent 笔记、两份存档对比）会指错人。
    ///
    /// 现在它是一次性校准 + 单向递增：分配 = `max(这个数, 场上 max+1)`（后者是**老档/手改档**
    /// 的兜底——档可以存成 JSON 被 Python 直接改），写回时只增不减。
    /// `#[serde(default)]`：0 = 老档缺字段 ⇒ 由 [`migrate`] 校准到 `场上 max + 1`。
    #[serde(default)]
    pub next_building_id: BuildingId,
    /// 星际市场的持久状态（挂单/价格/成交量/滑窗需求）。见 [`MarketState`] 与
    /// `.agents/notes/trade-and-sanctions.md`：市场是**真实交换所**（有卖家、有价、
    /// 可禁运、有配给），不是常数价无限供货的自动贩卖机。
    #[serde(default)]
    pub market: MarketState,
    /// **产地货栈**：`(势力, 天体) → 库存`。非首都天体的产出落在这里，**必须靠船运回首都**
    /// 才进入 [`Faction::resources`]（那个池子代表「首都集散地手上的现货」）。
    ///
    /// 依据：`.agents/notes/freight-collection.md` —— **首都即集散地**（对外路线只有
    /// 首都↔首都），所以首都天体的产出免运输直接进池，其余地方的货得等人来运。
    /// 货**不会消失**：没船就冻在产地（矿物不会烂），势力可以攒着等重建舰队、
    /// 或挂单请承运人来取。这使「运输任务 = 舰船的真实行为」有了物理落点。
    #[serde(default)]
    #[serde(with = "crate::json::key2")]
    #[schemars(with = "std::collections::BTreeMap<String, ResourceMap>")]
    pub depots: BTreeMap<(FactionId, BodyId), ResourceMap>,
    /// **承包市场**（托运方挂单、承运方接单）的持久状态：挂单簿 + 单号分配器。
    ///
    /// 依据：`.agents/notes/freight-collection.md` §4——集货腿的**第二条路**：自己没有运力
    /// （或运力不够）的势力，把「搬不动的那部分积压」挂出去请人来运。报酬是**抽成**
    /// （承运人交付时从货里自留，见 [`Contract::share`]），砸单**只掉信誉、不赔货值**。
    #[serde(default)]
    pub contracts: ContractState,
}

/// 一个可复现的**回合**：规范的持久世界 + `pre`（**输入面**）+ `post`（**结算面**）。
/// 整个结构落盘（`--save`/`--start`）。
///
/// # 两个面的分工（用户裁决，B5）
///
/// | 面 | 类型 | 装什么 | 能不能事后重算 |
/// | --- | --- | --- | --- |
/// | **`post`** | [`RoundView`] | 这一回合**结算出来**的：**观测**（回合末世界的纯观测）+ **过程量**（产出/维护/治理/成交/运输/判定……，B1–B4 逐步捕获） | 观测能（[`crate::sim::view_from_state`] 同一把尺子）；**过程量不能**——回合中段算完就扔，回合末重算会给出另一个数（B2/B3 实测过） |
/// | **`pre`** | [`RoundInputs`] | 这一回合**消费掉**的：**掷出的随机数** + **判定时看到的输入** | **不能**——主 `Prng` 的流已前进；`derived_roll` 的骰子虽可重算，但它比较的判据已经变了 |
///
/// **归属判据（用户原话）**：*「凡是可能未来与随机/输入有关的东西都放 `pre`，不一定要求当前的
/// 实现有关」*——一个量只要**概念上**是输入或掷骰（哪怕今天恰好是确定值）就归 `pre`；
/// 「这一回合结算出了什么」归 `post`。
///
/// ⚠ **`pre` 不再装观测**（B5 改的）：从前它是「回合开始时的观测副本」——同一份 `state`、同一个
/// `observe`、空 sink 只把过程量抹成中性值，于是它与**上一回合的 `post`** 逐字段相同，**零信息量**。
/// 要读「回合开始时的世界」请读上一行的 `post`（round 0 那份用 [`crate::sim::view_from_state`]）。
#[derive(Serialize, Deserialize, Clone, Debug, schemars::JsonSchema)]
pub struct RoundState {
    /// 状态/schema 版本（同 [`State::schema_version`] 语义）。
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// 规范的持久世界（实体）：天体/城/势力/舰/控制面/作用域/事件/编年史。
    pub state: State,
    /// **输入面**：本回合掷出的随机数与判定输入（[`RoundInputs`]）。空 = 这一回合没跑。
    #[serde(default)]
    pub pre: RoundInputs,
    /// **结算面**：本回合的观测 + 过程量（[`RoundView`]）。
    pub post: RoundView,
}
impl State {
    /// Look up a body by its unique **name** (the schema's identity key).
    pub fn body(&self, name: &str) -> Option<&Body> {
        self.bodies.iter().find(|b| b.name == name)
    }

    /// Mutably borrow a body by its unique **name**.
    pub fn body_mut(&mut self, name: &str) -> Option<&mut Body> {
        self.bodies.iter_mut().find(|b| b.name == name)
    }

    /// Look up a city by its unique **name** (the schema's identity key).
    pub fn city(&self, name: &str) -> Option<&City> {
        self.cities.iter().find(|c| c.name == name)
    }

    /// Mutably borrow a city by its unique **name**.
    pub fn city_mut(&mut self, name: &str) -> Option<&mut City> {
        self.cities.iter_mut().find(|c| c.name == name)
    }

    /// Look up a ship by its unique **name** (the schema's identity key).
    pub fn ship(&self, name: &str) -> Option<&Ship> {
        self.ships.iter().find(|s| s.name == name)
    }

    /// Mutably borrow a ship by its unique **name**.
    pub fn ship_mut(&mut self, name: &str) -> Option<&mut Ship> {
        self.ships.iter_mut().find(|s| s.name == name)
    }

    /// Look up a faction by its unique **name** (the schema's identity key).
    pub fn faction(&self, name: &str) -> Option<&Faction> {
        self.factions.iter().find(|f| f.name == name)
    }

    /// Mutably borrow a faction by its unique **name**.
    pub fn faction_mut(&mut self, name: &str) -> Option<&mut Faction> {
        self.factions.iter_mut().find(|f| f.name == name)
    }

    /// 某势力在某天体的**产地货栈**（非首都产出积压处；首都天体的产出直接进池，
    /// 不会在这里）。见 [`State::depots`]。
    pub fn depot(&self, fid: &str, body_id: &str) -> Option<&ResourceMap> {
        self.depots.get(&(fid.to_string(), body_id.to_string()))
    }

    /// 可变的产地货栈；不存在则建空的（产出落地、承运人装货都走这里）。
    pub fn depot_mut(&mut self, fid: &str, body_id: &str) -> &mut ResourceMap {
        self.depots
            .entry((fid.to_string(), body_id.to_string()))
            .or_default()
    }

    /// 把一笔货**卸进**某势力在某天体的货栈（数量 ≤0 时什么都不做）。
    /// 装货（船提走）走 [`State::depot_take`]。
    pub fn depot_add(&mut self, fid: &str, body_id: &str, resource: &str, amount: f64) {
        if amount <= 0.0 {
            return;
        }
        *self
            .depot_mut(fid, body_id)
            .entry(resource.to_string())
            .or_insert(0.0) += amount;
    }

    /// 从某势力在某天体的货栈里**提走**一笔货（装船），返回**实际提走的量**
    /// （0 = 那里没有这种货；请求量超过存量就提光）。
    ///
    /// 提空后**删掉空货栈条目**：`depots` 里只留「真有货」的条目，这样「哪些天体还有
    /// 积压」可以直接从键集合读出来（AI 派单、观察面都靠它），不必到处判空。
    /// 数量 ≤0 一律当作 0（不做事、也不建空条目）。
    pub fn depot_take(&mut self, fid: &str, body_id: &str, resource: &str, amount: f64) -> f64 {
        if amount <= 0.0 {
            return 0.0;
        }
        let key = (fid.to_string(), body_id.to_string());
        let Some(d) = self.depots.get_mut(&key) else {
            return 0.0;
        };
        let taken = d.get(resource).copied().unwrap_or(0.0).min(amount);
        if taken > 0.0 {
            if let Some(x) = d.get_mut(resource) {
                *x -= taken;
            }
            d.retain(|_, v| *v > 1e-9);
        }
        if d.is_empty() {
            self.depots.remove(&key);
        }
        taken.max(0.0)
    }

    /// **某势力在某天体手上能直接动用的实物**——本作「即时可用库存」的**唯一读法**。
    ///
    /// * **首都天体** ⇒ [`Faction::resources`]（**势力池** = 首都集散地手上的现货）；
    /// * **其余天体** ⇒ 该处的**产地货栈**（本地产出 + 运进来的补给）。
    ///
    /// 这一个是把 `.agents/notes/freight-collection.md` §2 的公理（首都即集散地）兑现成
    /// **唯一路径**的落点：非首都天体手上没有的东西，**只能靠船运过去**——消耗
    /// （建楼 / 造舰 / 装模块）与运输（装船）都只读它、只写它，不存在第二条
    /// 「从池子里直接扣到别人家门口」的路（用户裁决：**完全禁止瞬移**）。
    pub fn stock_at(&self, fid: &str, body: &str) -> Option<&ResourceMap> {
        if self.capital_body(fid) == body {
            return self.faction(fid).map(|f| &f.resources);
        }
        self.depot(fid, body)
    }

    /// [`State::stock_at`] 的**总件数**（那里什么都没有 ⇒ 0）。
    pub fn stock_units_at(&self, fid: &str, body: &str) -> f64 {
        self.stock_at(fid, body)
            .map(|m| m.values().sum())
            .unwrap_or(0.0)
    }

    /// 从 [`State::stock_at`] 提走一笔（装船 / 建造消耗），返回**实际提走的量**
    /// （0 = 那里没有这种货；请求量超过存量就提光）。
    pub fn stock_take(&mut self, fid: &str, body: &str, resource: &str, amount: f64) -> f64 {
        if amount <= 0.0 {
            return 0.0;
        }
        if self.capital_body(fid) == body {
            let Some(f) = self.factions.iter_mut().find(|f| f.name == fid) else {
                return 0.0;
            };
            let got = f
                .resources
                .get(resource)
                .copied()
                .unwrap_or(0.0)
                .min(amount);
            if got > 0.0 {
                if let Some(x) = f.resources.get_mut(resource) {
                    *x -= got;
                }
            }
            return got.max(0.0);
        }
        self.depot_take(fid, body, resource, amount)
    }

    /// 某势力货栈里**所有天体**的存货总价值（按 `value_of` 计价）。
    /// 这是「冻结在产地、还没运回首都」的那部分资产——观察面用它，
    /// 也是「无船势力库存冻结」这一机制的可读信号。
    pub fn depot_value(&self, fid: &str, value_of: &impl Fn(&str) -> f64) -> f64 {
        self.depots
            .iter()
            .filter(|((f, _), _)| f == fid)
            .flat_map(|(_, m)| m.iter())
            .map(|(rt, amt)| amt * value_of(rt))
            .sum()
    }

    /// Resolve the current world position of a body. Uses the stored
    /// `position` field, which the simulation keeps current.
    pub fn body_position(&self, name: &str) -> [f64; 2] {
        self.body(name).map(|b| b.position).unwrap_or([0.0, 0.0])
    }

    /// The 定居点 (settlement) a city occupies — settlement ↔ city 1:1.
    pub fn city_settlement(&self, cname: &str) -> Option<&Settlement> {
        let c = self.city(cname)?;
        self.body(&c.body_id)?.settlement(&c.settlement)
    }

    /// A body's settlement site at settlement name `sname` (唯一 key).
    pub fn body_settlement(&self, bname: &str, sname: &str) -> Option<&Settlement> {
        self.body(bname)?.settlement(sname)
    }

    /// Read one faction's controllable state.
    pub fn control(&self, fid: FactionId) -> Option<&ControllableState> {
        self.control.get(&fid)
    }

    /// Mutably borrow one faction's controllable state.
    pub fn control_mut(&mut self, fid: FactionId) -> Option<&mut ControllableState> {
        self.control.get_mut(&fid)
    }

    /// 该势力当前的**有效首都**天体——唯一的存量为命令控制的
    /// [`ControllableState::capital`]（迁都的唯一事实来源，无 shadow 双状态）。若某
    /// 势力尚未有任何首都控制（防御性兜底），回落到 [`default_capital_body`]。所有
    /// 「首都」读法（光速治理距离、本土防御半径、舰的撤退目的地、投影展示）都应走
    /// 这里，保证迁都即时生效。
    pub fn capital_body(&self, fid: &str) -> BodyId {
        self.control
            .get(fid)
            .and_then(|c| c.capital.as_ref())
            .map(|c| c.value.clone())
            .unwrap_or_else(default_capital_body)
    }

    /// 这艘舰当前的**有效指令**——**只有逐舰那片叶**能供值。
    ///
    /// * 叶存在 → 取**叶里**的值（与 `mode` 无关：叶写着 `Inherit` 也算"叶里有这个数"）；
    /// * 叶不存在 → `None`（调用方按 `Idle` 兜底，自动控制下一回合会给它写一条）。
    ///
    /// # 为什么指令没有"更高的一层"（用户裁决 2026-10）
    ///
    /// 指令是**即时操作**（去那里 / 跟随那艘船 / 跑哪条运输线），不是"这型舰是什么"。
    /// 于是"舰队默认指令"（`default_ship_order`）与设计图上的 `order` 两片叶**都已删除**：
    ///
    /// * 前者实测不是"默认值"而是**全舰队接管开关**——写它 ⇒ 全舰队归属解析成 `Player`
    ///   ⇒ `autocontrol` 的 style/freight/contract 闸门全部跳过这些舰、连自保撤退也不生效，
    ///   而全舰队被钉死在同一条站桩指令上（名字与作用不符，且会给几十回合后的新舰继承一条
    ///   过期命令）；
    /// * 后者**语义错位**：图描述的是"这型舰是什么"（长期倾向），于是它改为携带**风格/角色**
    ///   （见 [`Blueprint`] 与 [`State::ship_doctrine`]）。
    ///
    /// 长期倾向（风格两轴 + 角色）依旧有舰队级默认叶与图层；**行为**只有逐舰叶 + 自动控制的
    /// 每回合现写。
    pub fn ship_behavior(&self, ship_id: ShipId) -> Option<ShipBehavior> {
        let s = self.ship(&ship_id)?;
        let c = self.control(s.faction_id.clone())?;
        c.ship_orders.get(&ship_id).map(|l| l.value.clone())
    }

    /// **这条有效指令是谁供的值**——读面要能回答「这条意图是谁下的」。
    ///
    /// 指令只剩逐舰叶这一个供值者（见 [`State::ship_behavior`] 的说明），所以：
    /// * `leaf` —— 本舰的指令叶**存在**（`mode` 是 `Inherit` 也算：叶存在，值就来自它）；
    /// * `None` —— **没有任何一层说话**（调用方按 `Idle` 兜底）。
    ///
    /// `scope` / `record` 两个取值**在指令链上不会出现**（作用域节点只表态「谁负责」、
    /// 不携带值；指令没有"出厂记录值"——那是风格三轴的兜底，见 [`State::ship_doctrine`]）。
    /// 它们留在**取值域**里是为了让读面的枚举与「控制属性的层次链」一一对应，不是漏了分支
    /// （投影的 `column_docs` 里也写明这一点，免得后人以为是 bug）。
    pub fn ship_behavior_source(&self, ship_id: ShipId) -> Option<OrderSource> {
        let s = self.ship(&ship_id)?;
        let c = self.control(s.faction_id.clone())?;
        c.ship_orders.get(&ship_id).map(|_| OrderSource::Leaf)
    }

    /// 本舰出厂那张图**在势力库里的那一片叶**（图不存在 / 没有出厂图 ⇒ `None`）。
    fn ship_blueprint_leaf<'a, 'b>(
        &'a self,
        s: &'b Ship,
    ) -> Option<(&'b BlueprintId, &'a Control<Blueprint>)> {
        let id = s.blueprint.as_ref()?;
        let c = self.control(s.faction_id.clone())?;
        let leaf = c.blueprints.get(id)?;
        Some((id, leaf))
    }

    /// 决定一艘舰的指令由谁控制：**叶子 → 势力 → 全局**。
    ///
    /// 指令是即时操作，所以链上没有"舰队默认"也没有"出厂图"（两片叶都已删除，见
    /// [`State::ship_behavior`]）。⚠ 这里刻意**不看图上写没写倾向**：钉死选装（把图设为
    /// `Player`）**不该**连带把整支舰队的指令权收走（Q5）——玩家的图只决定"这型舰是什么"，
    /// 不决定"这艘舰现在去干什么"。
    pub fn ship_control(&self, ship_id: ShipId) -> ControlMode {
        let Some(s) = self.ship(&ship_id) else {
            return ControlMode::Auto;
        };
        let fid = s.faction_id.clone();
        let leaf = self
            .control(fid.clone())
            .map(|c| leaf_mode(c.ship_orders.get(&ship_id)))
            .unwrap_or(ControlMode::Inherit);
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 谁负责**这张设计图**：图叶 → 势力 scope → 全局（**没有**「舰队默认」这一档——
    /// 设计图是**势力的库**，不是某支舰队的指令）。
    ///
    /// `Player` = 系统不许重估这张图（出厂按图装配；图上写了 `order` 时那艘舰的意图也归
    /// 玩家）；`Auto`/全链继承 ⇒ `Auto` = 系统可重估它（[`crate::autocontrol::retool_shipyards`]）。
    /// 与 [`State::investment_budget_control`] 同形。
    pub fn blueprint_control(&self, fid: &FactionId, bp: &BlueprintId) -> ControlMode {
        let leaf = leaf_mode(self.control(fid.clone()).and_then(|c| c.blueprints.get(bp)));
        let faction = self.scope.factions.get(fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    // --- 控制模式判定（沿作用域链上溯，最具体者优先） ----------------------
    //
    // 每个 `*_control` 都返回三态之一：最具体的那一层**有意见**（`Auto`/`Player`）就
    // 算它的；一路「继承」到全局也没人说话，就落到 `Auto`（系统自动决定）。所以
    // 「叶子/作用域不存在」与「显式写着 Inherit」完全等价——都是没有说话。

    /// 这艘舰当前的**有效行为风格**：叶 → **出厂图** → 舰队默认 → **舰上的记录值**。
    ///
    /// 链比指令多两层，因为风格是**长期倾向**（"这型舰是什么"）：
    /// * **出厂图**（本舰下水那张图上的 `doctrine`，只有图上真写了这条轴、且那张图归属解析为
    ///   `Player` 时才供值）——它比舰队默认**更具体**（"这型舰" 比 "全势力默认" 具体），
    ///   所以插在舰队默认**之前**（与原行为链 ① ② 同序）；
    /// * 最后兜底到 `Ship.doctrine`——因为 `doctrine` 出厂时就有一份快照（继承舰级的
    ///   [`ShipSpec::default_doctrine`](crate::model::ShipSpec::default_doctrine)）。
    pub fn ship_doctrine(&self, ship_id: ShipId) -> ShipDoctrine {
        let Some(s) = self.ship(&ship_id) else {
            return ShipDoctrine::default();
        };
        let record = s.doctrine;
        let Some(c) = self.control(s.faction_id.clone()) else {
            return record;
        };
        let leaf = c.ship_doctrine.get(&ship_id);
        if leaf_mode(leaf) == ControlMode::Inherit {
            // ① 出厂图（"这型舰是什么"，比舰队默认更具体）
            if let Some(v) = self.blueprint_stance_doctrine(s) {
                return v;
            }
            // ② 势力级舰队默认
            if let Some(d) = &c.default_doctrine {
                if d.mode.is_player() {
                    return d.value;
                }
            }
        }
        leaf.map(|l| l.value).unwrap_or(record)
    }

    /// 这艘舰当前的**有效风筝<->贴脸姿态**：叶 → 出厂图 → 舰队默认 → 舰上的记录值。
    pub fn ship_kiting(&self, ship_id: ShipId) -> f64 {
        let Some(s) = self.ship(&ship_id) else {
            return 0.0;
        };
        let record = s.kiting;
        let Some(c) = self.control(s.faction_id.clone()) else {
            return record;
        };
        let leaf = c.ship_kiting.get(&ship_id);
        if leaf_mode(leaf) == ControlMode::Inherit {
            if let Some(v) = self.blueprint_stance_kiting(s) {
                return v;
            }
            if let Some(d) = &c.default_kiting {
                if d.mode.is_player() {
                    return d.value;
                }
            }
        }
        leaf.map(|l| l.value).unwrap_or(record)
    }

    /// 这艘舰当前的**有效角色**（[`ShipRole`]：打仗 / 跑运输 / 观测）。取值规则与前两条
    /// 风格轴完全同形：叶 → 出厂图 → 舰队默认（`Player` 时） → 舰上的记录值（出厂继承舰级
    /// [`ShipSpec::default_role`](crate::model::ShipSpec::default_role)）。
    ///
    /// ⚠ **它只管「自动控制的活是哪一种」**：不影响自动开火（射程内的敌舰照打），
    /// 也不影响 kiting（那条轴独立生效）。见 [`Ship::role`] 的说明。
    ///
    /// 舰不存在 ⇒ [`ShipRole::War`]（旧档的 serde 缺省也是它）。
    pub fn ship_role(&self, ship_id: ShipId) -> ShipRole {
        let Some(s) = self.ship(&ship_id) else {
            return ShipRole::War;
        };
        let record = s.role;
        let Some(c) = self.control(s.faction_id.clone()) else {
            return record;
        };
        let leaf = c.ship_role.get(&ship_id);
        if leaf_mode(leaf) == ControlMode::Inherit {
            if let Some(v) = self.blueprint_stance_role(s) {
                return v;
            }
            if let Some(d) = &c.default_role {
                if d.mode.is_player() {
                    return d.value;
                }
            }
        }
        leaf.map(|l| l.value).unwrap_or(record)
    }

    // --- 出厂图的**倾向**（三条风格轴共用的一层） ------------------------------
    //
    // 图能表态的是"这型舰是什么"，不是"这艘舰现在去干什么"（用户裁决 2026-10）。
    // 三个前置条件与原来那片 `order` 逐条相同（缺一即"这一层没有说话"）：
    //   ① 本舰有出厂图（`Ship.blueprint`；旧档/预置舰队/剧情赠舰都是 `None`）；
    //   ② 图上真写了这条轴（`None` = 本图对该轴沉默——建图 ≠ 表态，Q1(c)）；
    //   ③ 那张图的**归属解析为 `Player`**（`Auto` 图上的倾向是 AI 重估出来的流水，
    //      不能当玩家的表态——与舰队默认叶同一条规则）。

    fn blueprint_stance_doctrine(&self, s: &Ship) -> Option<ShipDoctrine> {
        let (id, bp) = self.ship_blueprint_leaf(s)?;
        let v = bp.value.doctrine?;
        self.blueprint_control(&s.faction_id, id).is_player().then_some(v)
    }

    fn blueprint_stance_kiting(&self, s: &Ship) -> Option<f64> {
        let (id, bp) = self.ship_blueprint_leaf(s)?;
        let v = bp.value.kiting?;
        self.blueprint_control(&s.faction_id, id).is_player().then_some(v)
    }

    fn blueprint_stance_role(&self, s: &Ship) -> Option<ShipRole> {
        let (id, bp) = self.ship_blueprint_leaf(s)?;
        let v = bp.value.role?;
        self.blueprint_control(&s.faction_id, id).is_player().then_some(v)
    }

    /// 这张图**这条轴有没有在说话**（图上写了该轴）——归属链用它决定要不要插图层。
    fn blueprint_speaks(&self, s: &Ship, axis: StyleAxis) -> bool {
        match self.ship_blueprint_leaf(s) {
            Some((_, bp)) => match axis {
                StyleAxis::Doctrine => bp.value.doctrine.is_some(),
                StyleAxis::Kiting => bp.value.kiting.is_some(),
                StyleAxis::Role => bp.value.role.is_some(),
            },
            None => false,
        }
    }

    /// 决定这艘舰的**行为风格**由谁控制：叶子 → 出厂图 → 舰队默认 → 势力 → 全局。
    pub fn ship_doctrine_control(&self, ship_id: ShipId) -> ControlMode {
        self.ship_style_chain(ship_id, StyleAxis::Doctrine)
    }

    /// 决定这艘舰的**风筝<->贴脸姿态**由谁控制：叶子 → 出厂图 → 舰队默认 → 势力 → 全局。
    pub fn ship_kiting_control(&self, ship_id: ShipId) -> ControlMode {
        self.ship_style_chain(ship_id, StyleAxis::Kiting)
    }

    /// 决定这艘舰的**角色**由谁控制：叶子 → 出厂图 → 舰队默认 → 势力 → 全局。
    /// 自动控制据此判断「这片叶能不能写」（`Player` = 玩家说了算，AI 不碰）。
    pub fn ship_role_control(&self, ship_id: ShipId) -> ControlMode {
        self.ship_style_chain(ship_id, StyleAxis::Role)
    }

    /// 三条风格轴共用的归属链：叶子 → **出厂图**（图上写了这条轴时）→ 舰队默认 → 势力 → 全局。
    fn ship_style_chain(&self, ship_id: ShipId, axis: StyleAxis) -> ControlMode {
        let Some(s) = self.ship(&ship_id) else {
            return ControlMode::Auto;
        };
        let fid = s.faction_id.clone();
        let (leaf, default) = match self.control(fid.clone()) {
            Some(c) => match axis {
                StyleAxis::Doctrine => (
                    leaf_mode(c.ship_doctrine.get(&ship_id)),
                    leaf_mode(c.default_doctrine.as_ref()),
                ),
                StyleAxis::Kiting => (
                    leaf_mode(c.ship_kiting.get(&ship_id)),
                    leaf_mode(c.default_kiting.as_ref()),
                ),
                StyleAxis::Role => (
                    leaf_mode(c.ship_role.get(&ship_id)),
                    leaf_mode(c.default_role.as_ref()),
                ),
            },
            None => (ControlMode::Inherit, ControlMode::Inherit),
        };
        // 图层**只在图上真写了这条轴时才参与**（建图 ≠ 表态）：否则「钉死选装」会把整支
        // 舰队的倾向权一起收走（Q5 明确说那个耦合不该出现）。
        let blueprint = match self.ship_blueprint_leaf(s) {
            Some((id, _)) if self.blueprint_speaks(s, axis) => self.blueprint_control(&fid, id),
            _ => ControlMode::Inherit,
        };
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, blueprint, default, faction, self.scope.global])
    }

    /// 决定某投资预算（建设用）由谁控制：资源 → 势力 → 全局。
    pub fn investment_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.investment_budget.get(resource)),
        );
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某建造预算（造舰用）由谁控制：资源 → 势力 → 全局。
    pub fn construction_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.construction_budget.get(resource)),
        );
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某「福利预算」资源由谁控制：资源 → 势力 → 全局。
    pub fn welfare_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.welfare_budget.get(resource)),
        );
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某城「娱乐/福利预算」由谁控制：城市 → 天体 → 势力 → 全局。
    pub fn loyalty_budget_control(&self, fid: FactionId, cid: CityId) -> ControlMode {
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.loyalty_budget.get(&cid)),
        );
        let city = self.scope.cities.get(&cid).copied().unwrap_or_default();
        let body_id = self.city(&cid).map(|c| c.body_id.clone());
        let body = body_id
            .and_then(|bid| self.scope.bodies.get(&bid).copied())
            .unwrap_or_default();
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定某城「开发货币预算」由谁控制：城市 → 天体 → 势力 → 全局。
    pub fn development_money_control(&self, fid: FactionId, cid: CityId) -> ControlMode {
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.development_money.get(&cid)),
        );
        let city = self.scope.cities.get(&cid).copied().unwrap_or_default();
        let body_id = self.city(&cid).map(|c| c.body_id.clone());
        let body = body_id
            .and_then(|bid| self.scope.bodies.get(&bid).copied())
            .unwrap_or_default();
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定某城「建造货币预算」由谁控制：城市 → 天体 → 势力 → 全局。
    pub fn construction_money_control(&self, fid: FactionId, cid: CityId) -> ControlMode {
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.construction_money.get(&cid)),
        );
        let city = self.scope.cities.get(&cid).copied().unwrap_or_default();
        let body_id = self.city(&cid).map(|c| c.body_id.clone());
        let body = body_id
            .and_then(|bid| self.scope.bodies.get(&bid).copied())
            .unwrap_or_default();
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定某建筑「建设投资权重」由谁控制：建筑 → 城市 → 天体 → 势力 → 全局。
    pub fn invest_control(&self, fid: FactionId, key: &InvestKey) -> ControlMode {
        let (cid, _) = key;
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.invest_weights.get(key)),
        );
        let city = self.scope.cities.get(cid).copied().unwrap_or_default();
        let body_id = self.city(cid).map(|c| c.body_id.clone());
        let body = body_id
            .and_then(|bid| self.scope.bodies.get(&bid).copied())
            .unwrap_or_default();
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定某建造区「建造投资权重」由谁控制：建造区 → 城市 → 天体 → 势力 → 全局。
    pub fn build_control(&self, fid: FactionId, key: &BuildKey) -> ControlMode {
        let (cid, _) = key;
        let leaf = leaf_mode(
            self.control(fid.clone())
                .and_then(|c| c.build_weights.get(key)),
        );
        let city = self.scope.cities.get(cid).copied().unwrap_or_default();
        let body_id = self.city(cid).map(|c| c.body_id.clone());
        let body = body_id
            .and_then(|bid| self.scope.bodies.get(&bid).copied())
            .unwrap_or_default();
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定「迁都」由谁控制：首都叶子 → 势力 → 全局（沿作用域链上溯，最具体者优先）。
    /// `Player` 时 sim 的周期迁移不覆盖（除非首都亡城——硬规则仍强迁）；
    /// `Auto`/`Inherit` 时由 sim 的周期迁都步骤重估。
    pub fn capital_control(&self, fid: &str) -> ControlMode {
        let leaf = leaf_mode(
            self.control(fid.to_string())
                .and_then(|c| c.capital.as_ref()),
        );
        let faction = self.scope.factions.get(fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }
}

/// 叶子的归属：**叶子不存在 ≡ [`ControlMode::Inherit`]**（这一层没有说话）。
fn leaf_mode<T>(leaf: Option<&Control<T>>) -> ControlMode {
    leaf.map(|c| c.mode).unwrap_or_default()
}

/// 一条**有效指令**的出处：这条值到底是谁供的（用户裁决 Q2=(b) 的读面答案）。
///
/// 读面（投影 `ships.order_source` 列）给的是**引擎解析后的答案**——Python 侧不要自己
/// 重实现这条链（那是漂移源，见 `.agents/notes/engine-data-plane.md`）。
///
/// ⚠ **2026-10 起指令链只剩逐舰叶这一个供值者**（舰队默认指令与图上的 `order` 两片叶都已
/// 删除，见 [`State::ship_behavior`]），所以实际只会出现 `Leaf`；`Scope`/`Record` 两个取值
/// 留在**取值域**里，是为了让读面的枚举与「控制属性的层次链」一一对应（投影的
/// `column_docs` 也写明这一点，免得后人以为是 bug）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderSource {
    /// 本舰的指令叶**存在**（`mode` 是 `Inherit` 也算——叶存在就用叶里的值）。
    Leaf,
    /// 作用域节点——**指令链上不会出现**（作用域只表态「谁负责」，不携带值）。
    Scope,
    /// 舰上的出厂记录值——**指令链上不会出现**（指令没有记录值，那是风格三轴的兜底）。
    Record,
}

impl OrderSource {
    /// 读面的稳定拼写：`leaf` / `scope` / `record`。
    pub fn label(&self) -> String {
        match self {
            OrderSource::Leaf => "leaf".to_string(),
            OrderSource::Scope => "scope".to_string(),
            OrderSource::Record => "record".to_string(),
        }
    }
}

#[cfg(test)]
#[path = "../tests/model/state.rs"]
mod tests;

/// 三条**风格轴**（都是「叶 → 舰队默认 → 舰上记录值」的同形链，共用归属判定）。
/// 名字里的「风格」是仓库里的旧称：它们都是「比指令更持久、比舰级更具体」的 per-舰 控制属性。
#[derive(Clone, Copy, PartialEq, Eq)]
enum StyleAxis {
    /// 行为风格（`temper`/`lone_wolf`）。
    Doctrine,
    /// 风筝<->贴脸姿态。
    Kiting,
    /// 角色：打仗 / 运输 / 观测（三态）。
    Role,
}
/// 把 `State` 从 `schema_version` 逐档升级到 [`SCHEMA_VERSION`]。在加载 `.ron` /
/// checkpoint 之后调用；无法迁移或版本比当前二进制还新则返回显式 `Err`（宁抛错，
/// 不错载）。v0 → v1：`schema_version` 字段本身就是 v1 引入的——旧档案缺字段由 serde
/// default 填 0，其既有字段无需任何数据变换（所有 v1 新字段都带 serde default）。
/// 真正的语义迁移（改字段含义/重算派生值）在将来某版本于此处补档。
///
/// v1 → v2：**事件（[`GameEvent`]）的结构变了**——`ShipDestroyed` 增加 `cause`/`by`（死因与
/// 凶手）、`CityRazed` 增加 `by_ship`/`damage`/`pop_before`、`CityDefected`/`Revolt` 增加
/// `loyalty`、`Resurgence` 增加 `city`、新增 `CityOverrun`，`ShipSpawned`/`ColonyFounded`
/// 各加一个来源字段。而 [`State::events`] 是**回合内流水**（每回合被清空、不累计），所以
/// 「旧档里那一回合的事件」本身就只是当回合的残留——**没有值得迁移的历史**，也无法凭空
/// 补齐缺失的因果。故 v1 → v2 只升版本号：旧 checkpoint 若含旧结构的事件会**在反序列化时
/// 明确报错**（而不是静默错载）；从 seed 重新生成即可（本模拟确定性可复现）。
///
/// v2 → v3：新增 [`State::milestones`]（长存里程碑）+ `CityRazed` 增加 `owner`（失去这座城的
/// 一方）。里程碑是 `#[serde(default)]` 的新字段：v2 档根本没有它，而 v2 档里的 `events` 只是
/// 当回合残留，**变不出跨回合的历史**——所以同样只升版本号。旧档若含旧结构的 `CityRazed`
/// 会在反序列化时报错（明确失败，不静默错载）。
///
/// v3 → v4：**分层判据换了**（从「重要性」换成「后续计算的访问需求」）+ 新增
/// [`State::notables`]（窗口层）+ 战争从 [`Salience::Milestone`] 改判为 [`Salience::Notable`]。
/// 同样只升版本号：v3 档的 `milestones` 里装的是**按旧判据预填**的事件（易主/存亡/开战/结盟/
/// 剧情），它们与新判据不符，但**都不是任何计算会回看的东西**——按新判据的正确处理不是把它们
/// 迁移进 `notables`（那只会把窗口塞满过期条目），而是丢弃：agent 要查那些历史，本来就走磁盘
/// 投影 `idx/events.jsonl`。**这一档丢掉的不是真历史，是错分级的残留。**
///
/// v4 → v5：**控制归属从两态变三态**——`ControlMode` 的 `Ai` 改名 [`ControlMode::Auto`]，
/// 而原来的第三态（`Control` 的 `mode: Option`，`None` = 继承）提升为显式变体
/// [`ControlMode::Inherit`]（`Control.mode` / `ControlScope` 的各节点都不再是 `Option`）。
///
/// **这一档是真迁移、且零信息损失**：旧 `.ron` 里存的是 `Option<ControlMode>`，
/// 而映射是双射——`None` ≡ `Inherit`、`Some(Ai)` ≡ `Auto`、`Some(Player)` ≡ `Player`。
/// 所以不需要「只升版本号丢掉什么」：由 [`ControlMode`] 手写的宽容 `Deserialize`
/// 在**加载时**就完成了映射（旧档照常反序列化成功），`migrate` 只负责把版本号推上来。
/// （区分：旧档 → 新二进制可以；新档 → 旧二进制不行，旧二进制会报版本过新。）
///
/// 此后的约定：一旦某层真的积累了「计算回看」的历史，升版就**不该**再继续「只升版本号」，
/// 届时应在此处写真正的迁移（而不是把历史一起丢掉）。
///
/// v5 → v6（贸易分支）：新增 [`State::market`]（[`MarketState`]：挂单/价格/成交量/滑窗需求）。
/// 它是 `#[serde(default)]` 的新字段，v4 档没有它、也没有任何可迁移的等价物
/// （旧市场是**无状态**的常数价兑换，不产生跨回合的价格/挂单历史），所以同样只升版本号：
/// 旧档加载后市场从空开始，**第一回合就由各势力的当期富余重新挂出**——这一点新旧档案
/// 完全一致，不构成信息损失。
///
/// v5 → v6（控制分支）：**风格（doctrine / kiting）也变成活层**——`ControllableState` 多了四片：
/// 每舰的 `ship_doctrine`/`ship_kiting` 叶片 + 势力级 `default_doctrine`/`default_kiting`；
/// `Ship.doctrine`/`Ship.kiting` 降级为**记录值**（出厂快照 + AI 流水）。
///
/// **这一档同样是零信息损失**：旧档没有那四片（`#[serde(default)]` ⇒ 空），于是
/// [`State::ship_doctrine`]/[`State::ship_kiting`] 的链一路继承、最后兜底到舰上的记录值——
/// **旧档的有效风格逐舰不变**。旧二进制读不了新档（版本过新会被拒），这是既定的方向。
///
/// **合并说明（两条并行分支撞了同一个档号）**：贸易分支与控制分支各自都从 v4 升到 v5，
/// 而两件事互不相干——所以 v5 这个号在合并后**有两种历史**。这不妨碍加载：两档都是
/// 「`#[serde(default)]` 补齐 + [`ControlMode`] 宽容 `Deserialize`」的零损失档，所以
/// **`0..=5` 一律直接推到 6**（= 同时具备市场与风格活层），「旧档来自哪条分支」不影响结果。
///
/// v6 → v7（运输分支）：新增 [`State::depots`]（**产地货栈**：非首都天体的产出落在这里，
/// 要靠船运回首都才进势力池）。它是 `#[serde(default)]` 的新字段，v6 档没有它——而
/// 「旧档里那些远在天边的城，产出是不是已经运回来了」**无法反推**（旧语义下产出是
/// 瞬间入库的，没有运输这件事）。
///
/// 这一档的处理是**把旧档的既成事实当作「已经运到首都」**：旧档加载后 `depots` 为空，
/// 于是它此刻的库存原样留在首都池里，只有**此后新产出的**离岸货才会开始积压在产地。
/// 这不是信息损失（旧档的库存本来就在池子里），而是新旧语义之间唯一自洽的接法。
///
/// v7 → v8（运输分支）：新增 [`Ship::cargo`]（**在舱货物**：这艘舰此刻实际装着什么）。
/// 它是 `#[serde(default)]` 的新字段，v7 档没有它——而「旧档里那些正在路上的货」**不存在**：
/// v7 里运输还不是舰船的真实行为，货只可能躺在某地（池子或货栈）里，不可能在半路。
/// 所以旧档一律按**空舱**处理，**零信息损失**（没有货在途中，也没有货凭空出现/消失）。
///
/// v8 → v9（运输分支）：[`ShipBehavior::Haul`](crate::model::ShipBehavior) 是控制叶的**新取值**，
/// 新增 `Ship::freighter` + 第三条风格轴（`ship_role`/`default_role` 叶片），
/// 事件流里也多了 `CargoLoaded` / `CargoDelivered` 两种事件。旧档里不可能有它们
/// （v8 的 `ShipBehavior` 没有 `Haul`，货也不会动、也没有「运输舰」这个角色），
/// 所以这一档同样**零信息损失**：旧档加载后没有任何舰在跑路线、没有货在舱里、
/// 每艘舰都按 `Ship.freighter = false`（= 战舰）继续过——那正是旧档的真实状态。
///
/// v9 → v10（设计图分支）：新增**舰船设计图**（[`Blueprint`]）这条链——
/// [`ControllableState::blueprints`]（势力级设计图库）、[`Building::blueprint`]（建造区
/// 指向一张图）、[`Ship::blueprint`]（出厂归因）、[`Ship::spawned_round`]（下水回合）、
/// [`GameConfig::blueprints`](crate::model::GameConfig::blueprints)（开局种子表，默认空）。
///
/// **这一档同样是零信息损失**，四个新字段全部 `#[serde(default)]`：
/// * `ControllableState.blueprints` ⇒ 空库；`Building.blueprint` ⇒ `None`；
///   `Ship.blueprint` ⇒ `None`；`GameConfig.blueprints` ⇒ 空种子表。
/// * ⇒ **每一条建造路径都走 `choose_loadout`**（`resolve_loadout` 没有图时就是今天那条
///   代码路径、同一时点：`sim.rs` 的船坞出厂那一刻按当时库存算）；
/// * ⇒ **面板/成本完全不变**（图里不存数值，config 仍是一元真值）；
/// * ⇒ **意图**：`Ship.blueprint = None` ⇒ 指令链上新增的那一层恒为空 ⇒
///   [`State::ship_behavior`] 的取值与今天逐值相同；
/// * ⇒ **归属**：`ship_control` 的链多一个恒为 `Inherit` 的层（没有图就没有这一层），
///   最具体的「有意见者」仍是原来那个；
/// * ⇒ **`retool_shipyards`**：旧档没有图 ⇒ 新的归属 gate 不触发，改的仍是 `ship_type`；
/// * ⇒ `spawned_round` 是纯归因（谁都不读它做决策），旧档一律「未知」。
///
/// 于是「**旧档 + 新二进制**」与「旧档 + 旧二进制」在同一 seed 下 `--digest` **逐字相同**
/// （实测见 `.agents/notes/ship-blueprint.md` 的实现记录）。
///
/// v9 → v10（运输分支）：新增 [`State::contracts`]（**承包市场**：[`ContractState`] 的挂单簿）
/// 与 [`Faction::reputation`]（**势力级信誉**）。两者都是 `#[serde(default)]` 的新字段，
/// v9 档没有它们——而那个世界里**根本没有承包这件事**：没人挂过单，也就没人有履约履历。
/// 所以这一档的处理是**让所有势力从中性信誉起步**（[`REPUTATION_NEUTRAL`]），
/// 与全新开局在同一条起跑线上：旧档里不存在任何可以折算成信誉的东西
///（旧语义下集货腿还只是「自己派船运」，没有对手方，也就没有「谁说话算数」这个问题）。
///
/// v10 → v11（运输分支）：承包市场有了**受雇方**——[`Contract`] 多了 `min_reputation`
/// （雇主的信誉门槛，挂单时算好冻结）与 [`ContractState::assignments`]（哪艘舰此刻在跑哪张单）。
/// 两者都是 `#[serde(default)]` 的新语义，而 v10 档里的单子**一个受雇方都没有**（那时还只有
/// 挂单侧），所以「门槛按 0 起步、没有任何派工」正是它的真实状态：**零信息损失**。
///
/// v11 → v12（运输分支）：**单子从「一票货」改写成「一份运力雇佣」**（用户裁决）。
/// [`Contract`] 的字段换了一茬：`amount`/`outstanding`/`deadline`/`late_penalized` 被
/// `capacity`（单位/回合）/`accepted_round`/`expires_round`/`review_round`/`served_rounds` 取代，
/// 事件也从 `contract_late`/`contract_lost` 换成 `contract_reviewed`/`contract_ended`。
/// 旧形态的单子在新语义下没有任何意义（它写的是「搬 47 件铁」，新语义问的是「每月几件运力」），
/// 两者之间没有等价的折算——硬凑一个只会让第一期的考核凭空判人不达标。
///
/// **v10–v12 汇流成 v13**（设计图分支与承包市场分支的合并）：两条分支各自从 v9 出发，
/// **同一个号在两条历史里含义不同**（设计图分支的 v10 = [`Blueprint`] 那条链；
/// 承包市场分支的 v10/v11/v12 = 挂单簿/信誉/派工，其中 v12 含那次「一票货 → 雇佣运力」的改写）。
/// 号本身因此**不能再判语义**了。合并后整段 `10..=12` 按最保守的方式接：
/// **清空挂单簿** + 推版本号。为什么清簿不算信息损失：挂单簿是**瞬时状态**（谁此刻想雇人），
/// 每回合都会由 `freight::post_contracts` 重新挂——旧档里那些没被接走的单子本来就没人接，
/// 清了它们不影响任何势力的实际处境；而**已经发生过的**成交都留在事件流里。
/// 设计图那几个字段本来就有 `#[serde(default)]`，缺了会退化成「空库 / 无指针 / 未知回合」，
/// 与它们各自那一档的接法一致。
pub fn migrate(state: &mut State) -> Result<(), String> {
    // **建筑 id 计数器的校准（v23）**：老档没有这个字段（serde default = 0）⇒ 一次性补到
    // 「场上 max(id) + 1」。写在 `match` **之前**而不是某个版本臂里：它是幂等的（有值就不动），
    // 而档现在可以被 Python 直接改（JSON 档），手改过的档也该走同一条兜底。
    if state.next_building_id == 0 {
        state.next_building_id = state
            .cities
            .iter()
            .flat_map(|c| c.buildings.iter().map(|b| b.id))
            .max()
            .map_or(0, |m| m + 1);
    }
    match state.schema_version {
        // v9 及更早：承包市场与设计图都还不存在（各自都是 serde default 的新增字段）⇒ 推号即可。
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 => {
            state.schema_version = SCHEMA_VERSION;
            Ok(())
        }
        // v10–v12：**两条独立历史共用过这一段号**（见上面的说明）⇒ 整段保守处理。
        10 | 11 | 12 => {
            state.contracts = Default::default();
            state.schema_version = SCHEMA_VERSION;
            Ok(())
        }
        // **v13–v20：这些号被两条历史各自用过**（见 [`SCHEMA_VERSION`] 的对照表）⇒ 整段只推号。
        // 这几档里 `State` 只在**两条**历史上真动过字段：`mond_control`（tech 支线）与
        // `Attack.shots`（`feature/b4-combat`）。前者在本支之外的档里 serde 缺省 0 = 凡人；
        // 后者靠 `#[serde(default)]` 补成空 vec（旧档的 `Attack` 只看聚合量，空 `shots`
        // 与「没记」同义——不是「打了一发没有任何分解」）。
        // v21（B5）就更轻：`State` 没动，动的是**档里 `pre` 那一格的含义**（观测 → 输入面）。
        // 旧档的 `pre` 会被 serde 当成「全是未知字段」而忽略 ⇒ 输入面读出来是空的。
        // 这与它的真相同义（那些档本来就没记过掷骰），所以**不补任何东西**。
        // v22（`feature/blueprint-stance`）：`default_ship_order` 与图上的 `order` 两片叶
        // **没有等价物**（正是被裁决删掉的东西）——旧档的这两个键会被 serde 忽略（拒绝未知
        // 字段只在 `--control` 的补丁面，不在 `State` 上），归属因此回到作用域链（`Auto`）。
        // 这是**有意的行为变化**，不是迁移漏了：要复原旧局面的做法是**逐舰重写指令**。
        // **这里不做「把舰队默认摊到每艘舰」的补丁**：那会把一条早就过期的站桩令**变成**
        // 全舰队的显式指令叶（玩家以后再也看不出它是哪来的），比丢失它更糟。
        13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21 | 22 | 23 => {
            state.schema_version = SCHEMA_VERSION;
            Ok(())
        }
        v if v == SCHEMA_VERSION => Ok(()),
        v => Err(format!(
            "cannot load state: schema_version {v} is newer than this binary ({SCHEMA_VERSION})"
        )),
    }
}
