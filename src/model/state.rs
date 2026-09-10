use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::model::*;
use super::control::resolve_chain;
use super::faction::default_capital_body;

/// The current persisted `State` schema version. Bump this whenever `State`'s
/// field structure or semantics change, and add a matching arm to [`migrate`] so
/// old `.ron` files are explicitly upgraded — or clearly rejected as "too new" —
/// instead of being silently loaded under new semantics.
pub const SCHEMA_VERSION: u32 = 8;
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
    pub depots: BTreeMap<(FactionId, BodyId), ResourceMap>,
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
    /// 装货（船提走）由 `sim` 的运输行为负责，删除空货栈条目也由它负责。
    pub fn depot_add(&mut self, fid: &str, body_id: &str, resource: &str, amount: f64) {
        if amount <= 0.0 {
            return;
        }
        *self
            .depot_mut(fid, body_id)
            .entry(resource.to_string())
            .or_insert(0.0) += amount;
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

    /// 这艘舰当前的**有效指令**——由「归属」决定取哪一层的值：
    ///
    /// * 叶子自己有意见（`Player`/`Auto`）→ 取**叶子**的值（单舰特例，最具体）；
    /// * 叶子没有说话（`Inherit` / 压根没有叶片——**新下水的舰就是这样**）→ 取势力级
    ///   [`default_ship_order`](ControllableState::default_ship_order) 的值；
    /// * 都没有 → `None`（调用方按 `Idle` 兜底）。
    ///
    /// 这条规则是「新舰默认归 AI、每段必须重新点名」的正解：舰的归属与意图都在更宽的
    /// 那一层有答案，所以**点名单舰只剩「例外」这一种用途**。
    pub fn ship_behavior(&self, ship_id: ShipId) -> Option<ShipBehavior> {
        let s = self.ship(&ship_id)?;
        let c = self.control(s.faction_id.clone())?;
        let leaf = c.ship_orders.get(&ship_id);
        // 叶子没有说话 → 高层（舰队默认）说了算——但**只在高层是玩家的表态时取值**：
        // 高层若说「自动」，那么这一艘的值应当由系统每回合现写（叶子上的记录值），
        // 而不是去用高层里那个可能早已过期的值。
        if leaf.map(|l| l.mode).unwrap_or_default() == ControlMode::Inherit {
            if let Some(d) = &c.default_ship_order {
                if d.mode.is_player() {
                    return Some(d.value.clone());
                }
            }
        }
        leaf.map(|l| l.value.clone())
    }

    /// 决定一艘舰的指令由谁控制：叶子 → 舰队默认 → 势力 → 全局。
    pub fn ship_control(&self, ship_id: ShipId) -> ControlMode {
        let Some(s) = self.ship(&ship_id) else {
            return ControlMode::Auto;
        };
        let fid = s.faction_id.clone();
        let (leaf, default) = match self.control(fid.clone()) {
            Some(c) => (
                leaf_mode(c.ship_orders.get(&ship_id)),
                leaf_mode(c.default_ship_order.as_ref()),
            ),
            None => (ControlMode::Inherit, ControlMode::Inherit),
        };
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, default, faction, self.scope.global])
    }

    // --- 控制模式判定（沿作用域链上溯，最具体者优先） ----------------------
    //
    // 每个 `*_control` 都返回三态之一：最具体的那一层**有意见**（`Auto`/`Player`）就
    // 算它的；一路「继承」到全局也没人说话，就落到 `Auto`（系统自动决定）。所以
    // 「叶子/作用域不存在」与「显式写着 Inherit」完全等价——都是没有说话。

    /// 这艘舰当前的**有效行为风格**：叶 → 舰队默认 → **舰上的记录值**（出厂快照）。
    ///
    /// 与 [`State::ship_behavior`] 同一条取值规则（叶 Inherit + 舰队默认是 `Player` ⇒ 取默认值；
    /// 否则取叶上的值），只是最后兜底到 `Ship.doctrine`——因为 `doctrine` 出厂时就有一份
    /// 快照，而指令没有。这条兜底让「老存档里只写过 `Ship.doctrine`」的行为完全不变。
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
            if let Some(d) = &c.default_doctrine {
                if d.mode.is_player() {
                    return d.value;
                }
            }
        }
        leaf.map(|l| l.value).unwrap_or(record)
    }

    /// 这艘舰当前的**有效风筝<->贴脸姿态**：叶 → 舰队默认 → 舰上的记录值。
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
            if let Some(d) = &c.default_kiting {
                if d.mode.is_player() {
                    return d.value;
                }
            }
        }
        leaf.map(|l| l.value).unwrap_or(record)
    }

    /// 决定这艘舰的**行为风格**由谁控制：叶子 → 舰队默认 → 势力 → 全局。
    pub fn ship_doctrine_control(&self, ship_id: ShipId) -> ControlMode {
        self.ship_style_chain(ship_id, true)
    }

    /// 决定这艘舰的**风筝<->贴脸姿态**由谁控制：叶子 → 舰队默认 → 势力 → 全局。
    pub fn ship_kiting_control(&self, ship_id: ShipId) -> ControlMode {
        self.ship_style_chain(ship_id, false)
    }

    /// 两条风格轴共用的归属链（`doctrine=true` 取 `ship_doctrine`/`default_doctrine`）。
    fn ship_style_chain(&self, ship_id: ShipId, doctrine: bool) -> ControlMode {
        let Some(s) = self.ship(&ship_id) else {
            return ControlMode::Auto;
        };
        let fid = s.faction_id.clone();
        let (leaf, default) = match self.control(fid.clone()) {
            Some(c) => {
                if doctrine {
                    (
                        leaf_mode(c.ship_doctrine.get(&ship_id)),
                        leaf_mode(c.default_doctrine.as_ref()),
                    )
                } else {
                    (
                        leaf_mode(c.ship_kiting.get(&ship_id)),
                        leaf_mode(c.default_kiting.as_ref()),
                    )
                }
            }
            None => (ControlMode::Inherit, ControlMode::Inherit),
        };
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, default, faction, self.scope.global])
    }

    /// 决定某投资预算（建设用）由谁控制：资源 → 势力 → 全局。
    pub fn investment_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = leaf_mode(self.control(fid.clone()).and_then(|c| c.investment_budget.get(resource)));
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某建造预算（造舰用）由谁控制：资源 → 势力 → 全局。
    pub fn construction_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = leaf_mode(self.control(fid.clone()).and_then(|c| c.construction_budget.get(resource)));
        let faction = self.scope.factions.get(&fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某城「娱乐/福利预算」由谁控制：城市 → 天体 → 势力 → 全局。
    pub fn loyalty_budget_control(&self, fid: FactionId, cid: CityId) -> ControlMode {
        let leaf = leaf_mode(self.control(fid.clone()).and_then(|c| c.loyalty_budget.get(&cid)));
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
        let leaf = leaf_mode(self.control(fid.clone()).and_then(|c| c.invest_weights.get(key)));
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
        let leaf = leaf_mode(self.control(fid.clone()).and_then(|c| c.build_weights.get(key)));
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
        let leaf = leaf_mode(self.control(fid.to_string()).and_then(|c| c.capital.as_ref()));
        let faction = self.scope.factions.get(fid).copied().unwrap_or_default();
        resolve_chain(&[leaf, faction, self.scope.global])
    }
}

/// 叶子的归属：**叶子不存在 ≡ [`ControlMode::Inherit`]**（这一层没有说话）。
fn leaf_mode<T>(leaf: Option<&Control<T>>) -> ControlMode {
    leaf.map(|c| c.mode).unwrap_or_default()
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
pub fn migrate(state: &mut State) -> Result<(), String> {
    match state.schema_version {
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 => {
            state.schema_version = SCHEMA_VERSION;
            Ok(())
        }
        v if v == SCHEMA_VERSION => Ok(()),
        v => Err(format!(
            "cannot load state: schema_version {v} is newer than this binary ({SCHEMA_VERSION})"
        )),
    }
}
