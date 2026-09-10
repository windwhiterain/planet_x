use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{BodyId, CityId, FactionId, ShipId};
use serde_json::json;

/// 舰被击毁的**原因**。
///
/// 此前「战死」与「维护费欠缴锈蚀报废」复用同一个 [`GameEvent::ShipDestroyed`]，agent 无法
/// 判断一艘舰到底是打没的还是锈没的。把原因显式化，「谁被谁打沉」与「哪支舰队被经济拖垮」
/// 才能分开统计。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeathCause {
    /// 被敌方火力打沉（凶手见 [`Killer`]）。
    Combat,
    /// 维护费欠缴、舰队「锈蚀」到 0 被拆解（不是战损）。
    UpkeepShortfall,
    /// 其它拆解（Scrapped）。
    Scrapped,
}

/// 击毁一艘舰的**凶手**：补刀的那一发来自哪艘舰、哪个势力、什么弹种。
///
/// 这是「一艘舰被击毁，是被哪艘舰击毁？」的唯一权威答案。此前只能靠同回合的
/// [`GameEvent::Attack`] 反推（且模拟内部 [`crate::sim`] 的思潮步进只能近似取「最后一个
/// 攻击者」），集火时不可判。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Killer {
    pub ship: ShipId,
    pub faction: FactionId,
    /// 弹种：`kinetic` / `plasma` / `missile`。
    pub weapon: String,
}

/// 一艘舰**从哪来**。三条造舰路径都要能被区分，否则「这艘舰哪来的」在历史里答不出。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpawnVia {
    /// 本势力某城的建造区出厂（[`GameEvent::ShipSpawned`] 的 `city` 是出厂城）。
    Shipyard,
    /// 剧情效果白送（[`StoryEffect::GrantShip`]）——此前**完全不发事件**，舰凭空出现。
    Story,
    /// 反僵尸重建的种子舰（[`GameEvent::Resurgence`] 同时发出）——这条路径此前也**不发**
    /// 造舰事件，投影对账实测 63 次出生里 46 次无解释。
    Resurgence,
}

/// 新殖民 / 复垦的**方式**。
///
/// 空白城（razed）保留最后主人的 `faction_id`（diaspora claim），所以复垦时**旧主是可读的
/// 历史**——`prev_owner` 记下它，「谁失去了这座城市」不再丢失。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FoundingHow {
    /// 在**从未被占据**的定居点上新建一座城。
    NewSite,
    /// 复垦一座**被夷平的空白城**（`prev_owner` = 它倒下时的主人）。
    Refounded,
}

/// 一回合内发生的、值得 agent 知道的事件。每回合开始时被清空、回合演化中被
/// 追加；agent 无需反推状态差即可得知「谁开火/谁被毁/哪城被夷平/谁殖民」。
///
/// **Rust 侧的 variant 字段名保持各自领域的可读写法**（`attacker`/`fallen_to`/`owner`…），
/// 归一化不在这里做——投影层用 [`GameEvent::history_row`] 把它们统一映射成
/// `(actor_kind, actor_id, target_kind, target_id)`，因此 Rust 代码可读、Python 侧可 join。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameEvent {
    /// 开火：攻击者对目标舰造成 damage 伤害。
    Attack { attacker: ShipId, target: ShipId, damage: f64 },
    /// 舰被击毁（hull ≤ 0）。`cause` 区分战死/锈蚀报废，`by` 是补刀的凶手（战死时必有）。
    ShipDestroyed { ship: ShipId, owner: FactionId, class: String, cause: DeathCause, by: Option<Killer> },
    /// 围城：攻击者对本回合城市建筑造成 damage 伤害。
    Siege { attacker: ShipId, city: CityId, damage: f64 },
    /// 城市被夷平（razed），可再殖民。
    ///
    /// `by_ship` 是**拆掉它的那艘舰**（把「哪艘舰拆了这座城」直接钉进事件，而不是让查询方
    /// 去同回合的 [`GameEvent::Siege`] 里猜）；`pop_before`/`damage` 是这次毁灭的量级，
    /// 供叙事与统计直读。
    CityRazed { city: CityId, fallen_to: FactionId, by_ship: ShipId, damage: f64, pop_before: u32 },
    /// 新舰从某城出厂（`via` 区分船坞建造 / 剧情赠舰）。`city` 只在船坞出厂时给出
    /// （剧情赠舰在天体附近下水，没有出厂城）。
    ShipSpawned { ship: ShipId, owner: FactionId, class: String, city: Option<CityId>, via: SpawnVia },
    /// 新殖民 / 再殖民城市建立。`how` 区分「全新定居点」与「复垦空白城」，
    /// `prev_owner` 在复垦时给出**这座城倒下时的主人**（diaspora claim），使
    /// 「被夷平然后被殖民」与「改旗易帜」在一条记录里就分得清。
    ColonyFounded {
        city: CityId,
        owner: FactionId,
        body: BodyId,
        seeded_ship_class: String,
        how: FoundingHow,
        prev_owner: Option<FactionId>,
    },
    /// 玩家指令因目标失效而降级（陈旧目标 / 城被夷平 / 无定居点），避免船飞向原点。
    StaleOrder { ship: ShipId, reason: String },
    /// 舰队战术撤退：一艘自动指挥的舰在**损伤过重且敌在本方射程内**时，向后撤往其
    /// 首都/本土修整充能，而不是死战到底（自保行为）。`to_body` 是撤退目的地天体。
    Withdraw { ship: ShipId, to_body: BodyId },
    /// 外交事件：一对势力本回合跨越战争阈值进入交战（war ≤ threshold）。
    WarStarted { a: FactionId, b: FactionId },
    /// 外交事件：一对势力本回合停战（从交战回到和平）。
    WarEnded { a: FactionId, b: FactionId },
    /// 剧情事件：本回合触发了一条叙事事件（详见 [`State::chronicle`] 的编年史全文）。
    /// `participants` 是参与方可读名（事件型触发时为具体对象）。
    Story { id: String, title: String, participants: Vec<String> },
    /// 重建（反僵尸/反垄断）：一支被彻底消灭（既无舰又无活城）的势力在一处它仍
    /// 拥有残骸足迹的定居点上重新建立前进基地，并获一艘种子舰。这保证游戏在上千
    /// 回合后仍是多方参与的局面，而不是收敛成少数几个永久旁观者。
    ///
    /// `city` 是被它重新立足的那座城（复垦 / 新建 / 夺取）——每次成功的重建都必然落在
    /// 一座具体的城上，故非 `Option`。此前这个事件**不带 city**，于是「一座城在夷平后
    /// 同回合被重建」的易主在历史里完全不可见——只能靠 `body` 与城的 `body_id` 反查。
    Resurgence { faction: FactionId, body: BodyId, ship: ShipId, city: CityId },
    /// 离心叛乱（光速治理的代价）：城市忠诚度跌破叛变阈值，居民脱离其统治势力，
    /// 城市被夷平为空白（可再殖民）。这是超大帝国管理廉价的远方殖民地失败的结果。
    /// `loyalty` 是爆发时的忠诚度（可读的量级）。
    Revolt { city: CityId, faction: FactionId, loyalty: f64 },
    /// 离心「改旗易帜」：城市忠诚度跌破叛变阈值后，居民不把城市夷为荒地，而是**倒戈到
    /// 思潮与旧主最对立**的势力（`to`）——城市连同其人口/建筑/舰队坞一起易主，旧主
    /// 失去一座城、新主获得一座城。这既给「过度扩张的大帝国」一个体量回落的口子，又让
    /// 被夷平/旁观的小势力能**接盘**城市、成长为真正的多极棋子，而不是退化成永久旁观者。
    /// 与 [`GameEvent::Revolt`] 并存：`Revolt` 是无可倒戈目标时的兜底（夷为空白）。
    CityDefected { city: CityId, from: FactionId, to: FactionId, loyalty: f64 },
    /// 难民夺城（反僵尸重建的最后一档）：世界每个定居点都已被活城占据、被灭势力既无自己的
    /// 空白足迹也找不到未占用定居点时，难民潮**夺取当前城数最多的那个势力的最小 id 活城**。
    ///
    /// 这此前**完全不发事件**——一座活城从 A 到 B 静默发生，是「城易主查不到原因」最直接的
    /// 一个洞。城市被占据即易主，故单独成事件，不与 [`GameEvent::Resurgence`] 混淆。
    CityOverrun { city: CityId, from: FactionId, to: FactionId },
    /// 合纵连横：一方势力被判定为「霸权」后，其余较弱势力结成反制联盟（`members`
    /// 为联盟成员，不含霸权 `hegemon`）。这是「一家独大 → 众人围剿」的政治跃迁，
    /// 让上千回合的博弈维持多方参与。
    CoalitionFormed { hegemon: FactionId, members: Vec<FactionId> },
    /// 合纵连横：既有的反制联盟解体（`members` 为解体时的成员）。
    CoalitionEnded { hegemon: FactionId, members: Vec<FactionId> },
    /// 迁都：势力把首都从 `from` 天体迁到 `to` 天体。`reason` 是触发原因
    /// （`"destroyed"`=首都亡城自动切到人口最高活城；`"ai_review"`=周期性 AI 评估证明
    /// 候选更优）。首都是光速治理/本土防御的锚点，迁都会即时改变治理距离与防御半径。
    CapitalRelocated { faction: FactionId, from: BodyId, to: BodyId, reason: String },
}

// --- 归一化投影 API（历史/事件查询的唯一契约） --------------------------------

/// 事件参与的**实体种类**。与 id 一起构成「任意实体 → 它的事件」的索引键。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    City,
    Ship,
    Faction,
    Body,
    Settlement,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::City => "city",
            EntityKind::Ship => "ship",
            EntityKind::Faction => "faction",
            EntityKind::Body => "body",
            EntityKind::Settlement => "settlement",
        }
    }
}

/// 参与方在事件里的**语义角色**：谁做的、对谁做的、谁受损、谁受益、以及次要第三方。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventRole {
    /// 动作发起者（攻击者 / 夷平者 / 复垦者 / 倒戈的目的地…）。
    Actor,
    /// 动作直接作用的对象（被攻击的舰 / 被围的城 / 被迁往的天体…）。
    Target,
    /// 受损方（被击毁舰的旧主 / 失去城市的势力…）——与 Target 可以并存（城既是被围对象，
    /// 也是旧主的损失）。
    Victim,
    /// 受益方（获得城市的新主 / 得到舰的势力…）。
    Beneficiary,
    /// 其余参与方（联盟成员 / 剧情参与方）。
    Third,
}

/// 一个参与方引用：`(角色, 实体种类, 实体 id)`。id 永远是**字符串名**（与全局身份约定一致）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Participant {
    pub role: EventRole,
    pub kind: EntityKind,
    pub id: String,
}

impl Participant {
    fn new(role: EventRole, kind: EntityKind, id: impl Into<String>) -> Self {
        Self { role, kind, id: id.into() }
    }
}

/// 事件的**显著性**分级。「长存账本」（随 checkpoint 存活、天然有界）只收
/// [`Salience::Milestone`]；海量逐发流水（开火/围城）只落磁盘投影。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Salience {
    /// 改变身份/归属/存亡的里程碑：城易主或夷平、舰出生或死亡、开战停战、联盟、迁都、剧情。
    Milestone,
    /// 值得注意但不改变归属：重建、撤退、指令降级。
    Notable,
    /// 逐发/逐次的高频流水：开火、围城。
    Detail,
}

/// 投影层要的**归一化事件行**：固定列、无同名多义、无同角色多名。
///
/// 这是「稀疏历史」的落地形态——一条事件一行，参与方在统一的
/// `(actor/target/victim/beneficiary/third)` 槽位里，因此 **Python 侧不需要知道任何 variant
/// 的字段布局**就能按任意实体 join。variant 专属的载荷统一收进 `data` 一个对象列
/// （一列只承载一种类型：不再有「同时是标量和列表」的列，也不再有 `from`/`to` 这种
/// 在 `city_defected` 是势力、在 `capital_relocated` 是天体的同名列）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct EventRow {
    /// 事件类型名（与 serde 的 `type` 判别式**逐字一致**，有守卫测试钉住）。
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// 显著性分级（`milestone` / `notable` / `detail`）。
    pub salience: Salience,
    /// 归一化的主参与方槽位。
    pub actor_kind: Option<&'static str>,
    pub actor_id: Option<String>,
    pub target_kind: Option<&'static str>,
    pub target_id: Option<String>,
    /// 次要参与方（受损/受益/第三方），长表 `(role, kind, id)`。
    pub extra: Vec<Participant>,
    /// 统一的数值强度（伤害；无伤害的事件为 0），便于 `groupby().sum()` 之类统计。
    pub magnitude: f64,
    /// variant 专属载荷（可读名/舰级/原因/忠诚度…），固定只用这一个对象列承载。
    pub data: serde_json::Value,
}

fn set_actor(r: &mut EventRow, kind: EntityKind, id: &str) {
    r.actor_kind = Some(kind.as_str());
    r.actor_id = Some(id.to_string());
}

fn set_target(r: &mut EventRow, kind: EntityKind, id: &str) {
    r.target_kind = Some(kind.as_str());
    r.target_id = Some(id.to_string());
}

fn extra(r: &mut EventRow, role: EventRole, kind: EntityKind, id: &str) {
    r.extra.push(Participant::new(role, kind, id));
}

impl GameEvent {
    /// 投影出**归一化事件行**。这是唯一的、穷尽的 variant → 历史视图映射：
    /// 新增一个 [`GameEvent`] variant 时，编译器会在这里强迫你声明它的参与方、
    /// 显著性、强度与载荷——**历史不可能被「忘记记录」**。
    pub fn history_row(&self) -> EventRow {
        let mut r = EventRow {
            kind: self.kind(),
            salience: self.salience(),
            actor_kind: None,
            actor_id: None,
            target_kind: None,
            target_id: None,
            extra: Vec::new(),
            magnitude: 0.0,
            data: json!({}),
        };
        match self {
            GameEvent::Attack { attacker, target, damage } => {
                set_actor(&mut r, EntityKind::Ship, attacker);
                set_target(&mut r, EntityKind::Ship, target);
                r.magnitude = *damage;
            }
            GameEvent::ShipDestroyed { ship, owner, class, cause, by } => {
                set_target(&mut r, EntityKind::Ship, ship);
                extra(&mut r, EventRole::Victim, EntityKind::Faction, owner);
                if let Some(k) = by {
                    // 凶手是**发起方**：一艘舰被击毁，答「被哪艘舰击毁」直接读 actor。
                    extra(&mut r, EventRole::Actor, EntityKind::Ship, &k.ship);
                    extra(&mut r, EventRole::Actor, EntityKind::Faction, &k.faction);
                }
                r.data = json!({"ship": ship, "owner": owner, "class": class,
                                "cause": cause, "by": by});
            }
            GameEvent::Siege { attacker, city, damage } => {
                set_actor(&mut r, EntityKind::Ship, attacker);
                set_target(&mut r, EntityKind::City, city);
                r.magnitude = *damage;
            }
            GameEvent::CityRazed { city, fallen_to, by_ship, damage, pop_before } => {
                set_actor(&mut r, EntityKind::Faction, fallen_to);
                set_target(&mut r, EntityKind::City, city);
                // 拆城的那艘舰是**次要发起方**：城际易主的「谁做的」在 actor（势力），
                // 「具体哪艘舰」在同一行的 extra 里，一次查询都拿得到。
                extra(&mut r, EventRole::Actor, EntityKind::Ship, by_ship);
                r.magnitude = *damage;
                r.data = json!({"city": city, "fallen_to": fallen_to, "by_ship": by_ship,
                                "damage": damage, "pop_before": pop_before});
            }
            GameEvent::ShipSpawned { ship, owner, class, city, via } => {
                set_actor(&mut r, EntityKind::Faction, owner);
                set_target(&mut r, EntityKind::Ship, ship);
                if let Some(c) = city {
                    extra(&mut r, EventRole::Third, EntityKind::City, c);
                }
                r.data = json!({"ship": ship, "owner": owner, "class": class, "city": city, "via": via});
            }
            GameEvent::ColonyFounded { city, owner, body, seeded_ship_class, how, prev_owner } => {
                set_actor(&mut r, EntityKind::Faction, owner);
                set_target(&mut r, EntityKind::City, city);
                extra(&mut r, EventRole::Third, EntityKind::Body, body);
                if let Some(p) = prev_owner {
                    extra(&mut r, EventRole::Victim, EntityKind::Faction, p);
                }
                r.data = json!({"city": city, "owner": owner, "body": body,
                                "seeded_ship_class": seeded_ship_class, "how": how,
                                "prev_owner": prev_owner});
            }
            GameEvent::StaleOrder { ship, reason } => {
                set_target(&mut r, EntityKind::Ship, ship);
                r.data = json!({"ship": ship, "reason": reason});
            }
            GameEvent::Withdraw { ship, to_body } => {
                set_actor(&mut r, EntityKind::Ship, ship);
                set_target(&mut r, EntityKind::Body, to_body);
                r.data = json!({"ship": ship, "to_body": to_body});
            }
            GameEvent::WarStarted { a, b } => {
                set_actor(&mut r, EntityKind::Faction, a);
                set_target(&mut r, EntityKind::Faction, b);
                r.data = json!({"a": a, "b": b});
            }
            GameEvent::WarEnded { a, b } => {
                set_actor(&mut r, EntityKind::Faction, a);
                set_target(&mut r, EntityKind::Faction, b);
                r.data = json!({"a": a, "b": b});
            }
            GameEvent::Story { id, title, participants } => {
                // 剧情参与方是可读名，未必是实体 id；仍按名字入索引（查得到就查得到）。
                for p in participants {
                    extra(&mut r, EventRole::Third, EntityKind::Faction, p);
                }
                r.data = json!({"id": id, "title": title, "participants": participants});
            }
            GameEvent::Resurgence { faction, body, ship, city } => {
                set_actor(&mut r, EntityKind::Faction, faction);
                // 落脚点是**城**（重建必然落在某座城上）；天体与种子舰进 extra。
                set_target(&mut r, EntityKind::City, city);
                extra(&mut r, EventRole::Third, EntityKind::Body, body);
                extra(&mut r, EventRole::Third, EntityKind::Ship, ship);
                r.data = json!({"faction": faction, "body": body, "ship": ship, "city": city});
            }
            GameEvent::Revolt { city, faction, loyalty } => {
                set_target(&mut r, EntityKind::City, city);
                extra(&mut r, EventRole::Victim, EntityKind::Faction, faction);
                r.data = json!({"city": city, "faction": faction, "loyalty": loyalty});
            }
            GameEvent::CityDefected { city, from, to, loyalty } => {
                set_target(&mut r, EntityKind::City, city);
                extra(&mut r, EventRole::Victim, EntityKind::Faction, from);
                extra(&mut r, EventRole::Beneficiary, EntityKind::Faction, to);
                r.data = json!({"city": city, "from": from, "to": to, "loyalty": loyalty});
            }
            GameEvent::CityOverrun { city, from, to } => {
                set_target(&mut r, EntityKind::City, city);
                extra(&mut r, EventRole::Victim, EntityKind::Faction, from);
                extra(&mut r, EventRole::Beneficiary, EntityKind::Faction, to);
                r.data = json!({"city": city, "from": from, "to": to});
            }
            GameEvent::CoalitionFormed { hegemon, members } => {
                // 联盟是**冲着**霸权结成的：霸权是被针对的目标，不是发起者。
                set_target(&mut r, EntityKind::Faction, hegemon);
                for m in members {
                    extra(&mut r, EventRole::Actor, EntityKind::Faction, m);
                }
                r.data = json!({"hegemon": hegemon, "members": members});
            }
            GameEvent::CoalitionEnded { hegemon, members } => {
                set_target(&mut r, EntityKind::Faction, hegemon);
                for m in members {
                    extra(&mut r, EventRole::Actor, EntityKind::Faction, m);
                }
                r.data = json!({"hegemon": hegemon, "members": members});
            }
            GameEvent::CapitalRelocated { faction, from, to, reason } => {
                set_actor(&mut r, EntityKind::Faction, faction);
                set_target(&mut r, EntityKind::Body, to);
                extra(&mut r, EventRole::Victim, EntityKind::Body, from);
                r.data = json!({"faction": faction, "from": from, "to": to, "reason": reason});
            }
        }
        r
    }

    /// 事件的类型名，与 serde 的 `type` 判别式（`rename_all = "snake_case"`）逐字一致。
    /// 守卫测试 `kind_matches_serde_tag` 钉住两者不漂移。
    pub fn kind(&self) -> &'static str {
        match self {
            GameEvent::Attack { .. } => "attack",
            GameEvent::ShipDestroyed { .. } => "ship_destroyed",
            GameEvent::Siege { .. } => "siege",
            GameEvent::CityRazed { .. } => "city_razed",
            GameEvent::ShipSpawned { .. } => "ship_spawned",
            GameEvent::ColonyFounded { .. } => "colony_founded",
            GameEvent::StaleOrder { .. } => "stale_order",
            GameEvent::Withdraw { .. } => "withdraw",
            GameEvent::WarStarted { .. } => "war_started",
            GameEvent::WarEnded { .. } => "war_ended",
            GameEvent::Story { .. } => "story",
            GameEvent::Resurgence { .. } => "resurgence",
            GameEvent::Revolt { .. } => "revolt",
            GameEvent::CityDefected { .. } => "city_defected",
            GameEvent::CityOverrun { .. } => "city_overrun",
            GameEvent::CoalitionFormed { .. } => "coalition_formed",
            GameEvent::CoalitionEnded { .. } => "coalition_ended",
            GameEvent::CapitalRelocated { .. } => "capital_relocated",
        }
    }

    /// 显著性分级：决定它是否进「长存账本」。
    pub fn salience(&self) -> Salience {
        match self {
            // 改变身份/归属/存亡的里程碑。
            GameEvent::CityRazed { .. }
            | GameEvent::CityDefected { .. }
            | GameEvent::CityOverrun { .. }
            | GameEvent::ColonyFounded { .. }
            | GameEvent::Revolt { .. }
            | GameEvent::Resurgence { .. }
            | GameEvent::ShipDestroyed { .. }
            | GameEvent::ShipSpawned { .. }
            | GameEvent::WarStarted { .. }
            | GameEvent::WarEnded { .. }
            | GameEvent::CoalitionFormed { .. }
            | GameEvent::CoalitionEnded { .. }
            | GameEvent::CapitalRelocated { .. }
            | GameEvent::Story { .. } => Salience::Milestone,
            // 值得注意但不改变归属。
            GameEvent::Withdraw { .. } | GameEvent::StaleOrder { .. } => Salience::Notable,
            // 逐次高频流水。
            GameEvent::Attack { .. } | GameEvent::Siege { .. } => Salience::Detail,
        }
    }

    /// 全部参与方（平铺，含 actor/target 两个主槽位），供投影层建长表索引。
    /// 与 [`GameEvent::history_row`] 同源，避免两处各写一份映射。
    pub fn participants(&self) -> Vec<Participant> {
        let r = self.history_row();
        let mut out = Vec::new();
        if let (Some(k), Some(id)) = (r.actor_kind, r.actor_id) {
            out.push(Participant { role: EventRole::Actor, kind: entity_kind_from_str(k), id });
        }
        if let (Some(k), Some(id)) = (r.target_kind, r.target_id) {
            out.push(Participant { role: EventRole::Target, kind: entity_kind_from_str(k), id });
        }
        out.extend(r.extra);
        out
    }
}

fn entity_kind_from_str(s: &str) -> EntityKind {
    match s {
        "city" => EntityKind::City,
        "ship" => EntityKind::Ship,
        "faction" => EntityKind::Faction,
        "body" => EntityKind::Body,
        _ => EntityKind::Settlement,
    }
}

/// 一条剧情编年史记录：回合里发生的一次「叙事事件」，带标题、正文与参与方。
///
/// 这是「剧情丰富」的可读载体——一条 `Story` 剧情事件在本回合触发时，除了记入
/// [`State::events`]（本回合流水），还把这个完整条目追加进 [`State::chronicle`]，
/// 供 agent 随时查询整段已展开的故事弧。
#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct ChronicleEntry {
    /// 触发时的回合号（时间戳）。
    pub round: u32,
    /// 事件模板 id（config/game.ron 的 `story` 表键）。
    pub id: String,
    /// 标题（如「外来的回响」）。
    pub title: String,
    /// 正文（一段叙事）。
    pub body: String,
    /// 参与方可读名（如「中国」「行星X崇拜教」），便于 agent 直读。
    pub participants: Vec<String>,
}
/// 一条剧情事件何时触发。数据驱动，全部可确定复现；一次事件默认只触发一次。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum StoryTrigger {
    /// 到达或超过某回合时触发（时间线上的「节拍」）。
    RoundAt { round: u32 },
    /// 世界上第一次出现任何交战时触发。
    FirstWar,
    /// 第一次有城市被夷平（razed）时触发。
    FirstRaze,
    /// 第一次建立/再殖民城市触发的殖民事件时触发。
    FirstColony,
    /// 指定两势力第一次进入交战时触发。
    WarBetween { a: FactionId, b: FactionId },
    /// 指定势力第一次与任何势力交战时触发。
    FactionAtWar { faction: FactionId },
    /// a 对 b 的关系跌破 `value` 时触发（如某势力失和、阵营反目）。
    RelationBelow { a: FactionId, b: FactionId, value: f64 },
}
/// 剧情事件的机械后果（可选；刻意保持小幅、确定性，避免扰动经济/军事平衡太久）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum StoryEffect {
    /// 调整 a↔b 的关系（双向）。
    Relations { a: FactionId, b: FactionId, delta: f64 },
    /// 给某势力注入一定量资源（key 为 config 原始资源 key）。
    GrantResources { faction: FactionId, resource: String, amount: f64 },
    /// 给某势力在指定天体附近「出厂」一艘舰（给剧情以真实的机械分量——如
    /// 一艘新锐旗舰从天体附近下水）。舰 id 由模拟按当前最大 id 连续分配，确定性。
    GrantShip { faction: FactionId, class: String, body: BodyId },
}
/// 一条剧情事件模板，来自 config/game.ron 的 `story` 表。
///
/// `trigger` 决定何时火（见 [`StoryTrigger`]）；`effects` 是可选的小幅机械后果；
/// `participants` 是可读参与方名。整张表数据驱动，模拟只按它触发即可复现地展开剧情。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StoryEvent {
    pub id: String,
    pub title: String,
    pub body: String,
    pub trigger: StoryTrigger,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub effects: Vec<StoryEffect>,
}
