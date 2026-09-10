use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::*;
use super::control::resolve_chain;
use super::faction::default_capital_body;

/// The current persisted `State` schema version. Bump this whenever `State`'s
/// field structure or semantics change, and add a matching arm to [`migrate`] so
/// old `.ron` files are explicitly upgraded — or clearly rejected as "too new" —
/// instead of being silently loaded under new semantics.
pub const SCHEMA_VERSION: u32 = 4;
fn default_schema_version() -> u32 {
    0
}
/// The complete world snapshot.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct State {
    /// 状态/schema 版本。每次改动 `State` 的**语义/字段结构**时递增（见 [`SCHEMA_VERSION`]
    /// 与 [`migrate`]）；旧 `.ron` 缺该字段时 serde default 为 0，由 [`migrate`] 逐档升级，
    /// 避免「旧档案被按新语义静默错载」。
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Current round number (0 = the start state).
    pub round: u32,
    /// Total elapsed time in months.
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
}

/// 一个可复现的**回合**: 规范的持久世界 + pre(pre==rng 派生态) + post(post==state 派生态)。
/// 整个结构落盘（`--save`/`--start`）。`advance` 就地改 `state` 并把本回合的 `pre`/`post`
/// 写回这两个字段。
///
/// * **`pre`**：依赖本次 rng **掷出的随机决策**的快照（造什么舰、海军重组、出战顺序、外交
///   扰动……），是 `(state, rng)` 的函数。**不当作冻结的真理**——回退到某回合、换一个 rng，
///   `pre` 就按新 rng 重算，故事自然分叉。这就是 `rng → pre_derived` 的关系。
/// * **`post`**：依赖**当前 `state`** 的纯观测（实力占比/霸权/联盟/制裁/战争/产量/维护费/
///   治理……），是 `state` 的函数。同一 `state` 恒定，供 agent 与测试读取。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RoundState {
    /// 状态/schema 版本（同 [`State::schema_version`] 语义）。
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// 规范的持久世界（实体）：天体/城/势力/舰/控制面/作用域/事件/编年史。
    pub state: State,
    /// 本回合依赖 rng 的随机决策快照（`(state, rng)` 的函数，换 rng 即重算）。
    pub pre: Derived,
    /// 本回合依赖 state 的纯观测快照（`state` 的函数）。
    pub post: Derived,
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

    /// Current behavior (指令) of a ship, if any. `ship_id` is the ship's unique name.
    pub fn ship_behavior(&self, ship_id: ShipId) -> Option<ShipBehavior> {
        let s = self.ship(&ship_id)?;
        self.control(s.faction_id.clone())?
            .ship_orders
            .get(&ship_id)
            .map(|c| c.value.clone())
    }

    // --- 控制模式判定（沿作用域链上溯，最具体者优先） ----------------------

    /// 决定一艘舰的指令由谁控制：舰 → 势力 → 全局。
    pub fn ship_control(&self, ship_id: ShipId) -> ControlMode {
        let Some(s) = self.ship(&ship_id) else {
            return ControlMode::Ai;
        };
        let fid = s.faction_id.clone();
        let leaf = self.control(fid.clone()).and_then(|c| c.ship_orders.get(&ship_id)).and_then(|c| c.mode);
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某投资预算（建设用）由谁控制：资源 → 势力 → 全局。
    pub fn investment_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = self.control(fid.clone()).and_then(|c| c.investment_budget.get(resource)).and_then(|c| c.mode);
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某建造预算（造舰用）由谁控制：资源 → 势力 → 全局。
    pub fn construction_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = self.control(fid.clone()).and_then(|c| c.construction_budget.get(resource)).and_then(|c| c.mode);
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某城「娱乐/福利预算」由谁控制：城市 → 天体 → 势力 → 全局。
    pub fn loyalty_budget_control(&self, fid: FactionId, cid: CityId) -> ControlMode {
        let leaf = self.control(fid.clone()).and_then(|c| c.loyalty_budget.get(&cid)).and_then(|c| c.mode);
        let city = self.scope.cities.get(&cid).copied().flatten();
        let body_id = self.city(&cid).map(|c| c.body_id.clone());
        let body = body_id.and_then(|bid| self.scope.bodies.get(&bid).copied().flatten());
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定某建筑「建设投资权重」由谁控制：建筑 → 城市 → 天体 → 势力 → 全局。
    pub fn invest_control(&self, fid: FactionId, key: &InvestKey) -> ControlMode {
        let (cid, _) = key;
        let leaf = self.control(fid.clone()).and_then(|c| c.invest_weights.get(key)).and_then(|c| c.mode);
        let city = self.scope.cities.get(cid).copied().flatten();
        let body_id = self.city(cid).map(|c| c.body_id.clone());
        let body = body_id.and_then(|bid| self.scope.bodies.get(&bid).copied().flatten());
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定某建造区「建造投资权重」由谁控制：建造区 → 城市 → 天体 → 势力 → 全局。
    pub fn build_control(&self, fid: FactionId, key: &BuildKey) -> ControlMode {
        let (cid, _) = key;
        let leaf = self.control(fid.clone()).and_then(|c| c.build_weights.get(key)).and_then(|c| c.mode);
        let city = self.scope.cities.get(cid).copied().flatten();
        let body_id = self.city(cid).map(|c| c.body_id.clone());
        let body = body_id.and_then(|bid| self.scope.bodies.get(&bid).copied().flatten());
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定「迁都」由谁控制：首都叶子 → 势力 → 全局（沿作用域链上溯，最具体者优先）。
    /// `Player` 时 sim 的周期迁移不覆盖（除非首都亡城——硬规则仍强迁）；`Ai` 或缺省
    /// None 时由 sim 的周期迁都步骤重估。
    pub fn capital_control(&self, fid: &str) -> ControlMode {
        let leaf = self.control(fid.to_string()).and_then(|c| c.capital.as_ref()).and_then(|c| c.mode);
        let faction = self.scope.factions.get(fid).copied().flatten();
        resolve_chain(&[leaf, faction, self.scope.global])
    }
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
/// 此后的约定：一旦某层真的积累了「计算回看」的历史，升版就**不该**再继续「只升版本号」，
/// 届时应在此处写真正的迁移（而不是把历史一起丢掉）。
pub fn migrate(state: &mut State) -> Result<(), String> {
    match state.schema_version {
        0 | 1 | 2 | 3 => {
            state.schema_version = SCHEMA_VERSION;
            Ok(())
        }
        v if v == SCHEMA_VERSION => Ok(()),
        v => Err(format!(
            "cannot load state: schema_version {v} is newer than this binary ({SCHEMA_VERSION})"
        )),
    }
}
