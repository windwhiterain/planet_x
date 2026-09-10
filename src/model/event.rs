use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{BodyId, CityId, FactionId, ResourceMap, ShipId};
use serde_json::json;

/// 给「纯标签 enum」（只有单元变体）生成 **字符串化** 的 `From`/`TryFrom<String>`，配合
/// `#[serde(into = "String", try_from = "String")]` 使用。
///
/// # 为什么不能按 serde 默认表示
///
/// serde 默认把单元变体序列化成**裸标识符**（RON 里是 `cause:combat`）。这样的 enum 一旦
/// 嵌在**内部标签 enum**（`#[serde(tag = "...")]`，本文件里就是 [`GameEvent`]）内部，
/// **RON 就再也读不回来**：deserialize 侧要把整个结构先缓冲成自描述内容，而 RON 的
/// `deserialize_any` 无法把裸标识符还原成内容，于是报
/// `Expected string or map but found a unit value instead`。
///
/// 后果不是「少一个字段」，而是**整份 `State` 反序列化失败**——`--save` 写出的 checkpoint
/// 用 `--start` 读不回来（`load_initial` 还会把这个错误咽掉、退回当裸 `State` 解析，最终
/// 报成一句莫名其妙的「missing field `round` in `State`」，指向文件末尾）。实测：
/// `(type:"a",ship:"x",cause:combat)` 读不回来，而 `(type:"a",ship:"x",cause:"combat")` 可以。
///
/// 写成字符串是一石二鸟：RON 与 JSON 都能原样读回（`source:""` 之类），而 **JSON 侧的形状
/// 完全不变**——`serde_json` 本来就把单元变体写成 `"combat"` 这样的字符串，所以 agent 视图、
/// 投影、WebUI 一个字节都不动。
macro_rules! stringly_unit_enum {
    ($ty:ident { $($name:literal => $variant:ident),+ $(,)? }) => {
        impl From<$ty> for String {
            fn from(v: $ty) -> String { v.as_str().to_string() }
        }
        impl TryFrom<String> for $ty {
            type Error = String;
            /// 认不出的字符串**明确报错**（而不是退回某个默认变体）——checkpoint 里出现
            /// 未知标签说明档与新二进制对不上，必须响亮地失败。
            fn try_from(s: String) -> Result<Self, String> {
                match s.as_str() {
                    $($name => Ok($ty::$variant),)+
                    other => Err(format!("unknown {}: {other:?}", stringify!($ty))),
                }
            }
        }
    };
}

/// 舰被击毁的**原因**。
///
/// 此前「战死」与「维护费欠缴锈蚀报废」复用同一个 [`GameEvent::ShipDestroyed`]，agent 无法
/// 判断一艘舰到底是打没的还是锈没的。把原因显式化，「谁被谁打沉」与「哪支舰队被经济拖垮」
/// 才能分开统计。
///
/// **序列化成字符串**（见 [`stringly_unit_enum`] 的说明）：单元变体若按 serde 默认写成
/// **裸标识符**，嵌在内部标签 enum（[`GameEvent`]）里时 RON 读不回来。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(into = "String", try_from = "String")]
pub enum DeathCause {
    /// 被敌方火力打沉（凶手见 [`Killer`]）。
    Combat,
    /// 维护费欠缴、舰队「锈蚀」到 0 被拆解（不是战损）。
    UpkeepShortfall,
    /// 其它拆解（Scrapped）。
    Scrapped,
}

impl DeathCause {
    pub fn as_str(self) -> &'static str {
        match self {
            DeathCause::Combat => "combat",
            DeathCause::UpkeepShortfall => "upkeep_shortfall",
            DeathCause::Scrapped => "scrapped",
        }
    }
}

stringly_unit_enum!(DeathCause { "combat" => Combat, "upkeep_shortfall" => UpkeepShortfall, "scrapped" => Scrapped });

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
///
/// 与 [`DeathCause`] 一样**序列化成字符串**（见 [`stringly_unit_enum`]）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(into = "String", try_from = "String")]
pub enum SpawnVia {
    /// 本势力某城的建造区出厂（[`GameEvent::ShipSpawned`] 的 `city` 是出厂城）。
    Shipyard,
    /// 剧情效果白送（[`StoryEffect::GrantShip`]）——此前**完全不发事件**，舰凭空出现。
    Story,
}

impl SpawnVia {
    pub fn as_str(self) -> &'static str {
        match self {
            SpawnVia::Shipyard => "shipyard",
            SpawnVia::Story => "story",
        }
    }
}

stringly_unit_enum!(SpawnVia { "shipyard" => Shipyard, "story" => Story });

/// 新殖民 / 复垦的**方式**。
///
/// 空白城（razed）保留最后主人的 `faction_id`（diaspora claim），所以复垦时**旧主是可读的
/// 历史**——`prev_owner` 记下它，「谁失去了这座城市」不再丢失。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, JsonSchema)]
#[serde(into = "String", try_from = "String")]
pub enum FoundingHow {
    /// 在**从未被占据**的定居点上新建一座城。
    NewSite,
    /// 复垦一座**被夷平的空白城**（`prev_owner` = 它倒下时的主人）。
    Refounded,
}

impl FoundingHow {
    pub fn as_str(self) -> &'static str {
        match self {
            FoundingHow::NewSite => "new_site",
            FoundingHow::Refounded => "refounded",
        }
    }
}

stringly_unit_enum!(FoundingHow { "new_site" => NewSite, "refounded" => Refounded });

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
    ///
    /// `owner` 是**失去这座城市的那一方**——这是**只有在此刻才知道**的事实：夷平只把城变成
    /// 空白（`razed = true`），空白城**保留**最后主人的 `faction_id` 作为 diaspora claim，而
    /// 同一回合后来的复垦/重建会把 `faction_id` 改写成新主。因此「谁丢了这座城」若不在
    /// 这一刻记下来，回看时读到的就是**新主**（错的人）。
    CityRazed {
        city: CityId,
        owner: FactionId,
        fallen_to: FactionId,
        by_ship: ShipId,
        damage: f64,
        pop_before: u32,
    },
    /// 新舰从某城出厂（`via` 区分船坞建造 / 剧情赠舰）。`city` 只在船坞出厂时给出
    /// （剧情赠舰在天体附近下水，没有出厂城）。
    ///
    /// `blueprint` = 出厂所用的**设计图名**（`None` = 无图：旧档、开局预置舰队、剧情赠舰）。
    /// 它是**归因**：事后能回答「这艘舰是哪张图印出来的」（投影 `events.data.blueprint`）。
    /// `#[serde(default)]` + 只在 `Some` 时进 headline ⇒ 无图那一路的读面**一个字节不变**。
    ShipSpawned {
        ship: ShipId,
        owner: FactionId,
        class: String,
        city: Option<CityId>,
        via: SpawnVia,
        #[serde(default)]
        blueprint: Option<crate::model::BlueprintId>,
    },
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
    /// **装货**：一艘运输舰在某天体的**产地货栈**里装走一批货（`cargo` = 这次装了什么、各多少）。
    /// 这是「离岸产出 → 首都池」那条链的**上半段**，下半段是 [`GameEvent::CargoDelivered`]。
    /// 有了这两条，「池子里的铁是哪来的」可以一路追到产地与那艘船。
    CargoLoaded { ship: ShipId, faction: FactionId, body: BodyId, cargo: ResourceMap },
    /// **卸货**：一艘运输舰把在舱货物卸进某天体。`into_pool = true` 表示**直接进了势力池**
    /// （即该天体就是本势力首都，货从此可用）——那是集货腿的终点；`false` 表示卸进了该天体
    /// 的货栈（中转，还得再运一程）。`cargo` 是这一批货。
    ///
    /// 注意：**货随舰沉没**——满载的运输舰被击沉时，舱里的货跟着没了（没有对应的
    /// `CargoLost` 事件：货的消失就是那艘舰的 `ShipDestroyed` 的一部分）。
    CargoDelivered {
        ship: ShipId,
        faction: FactionId,
        body: BodyId,
        cargo: ResourceMap,
        into_pool: bool,
    },
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

/// 事件的**分级**。判据是**后续计算需要访问哪一段历史**——不是「重要性」。
///
/// 这三条是**设计约束，不是描述**：它决定一条事件必须被存进哪一层，所以新增 variant 时
/// 要问的是「后面的逻辑要回看它吗、要回看多久」，而不是「它听起来重不重要」。反过来，
/// 每一条定级都应当能指到**具体的读者**——见 [`GameEvent::salience`] 里的读者盘点。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Salience {
    /// 后续计算需要访问**无限的过去历史**。→ 必须长存进 [`State::milestones`]，随 checkpoint
    /// 存活，不设时间窗口。
    Milestone,
    /// 后续计算需要访问**一定的事件窗口**（最近 W 个回合 / 最近 W 条）。→ 只需一个有界窗口
    /// 在 state 里，不需要无限长存。
    ///
    /// **范例：[`GameEvent::WarStarted`]。** 两国开战之后，相当一段时间里它们会**互相记恨**
    /// ——关系被一个会衰减的旧仇压着，所以后面的外交/思潮计算要回头看「最近 W 个回合里我们
    /// 打过仗吗」。W 之外的那场战争不再影响任何计算，因此**不需要**无限长存；反之，今天的关系
    /// 模型（`relations` 标量 + `war_fatigue` 回拉）**没有任何记忆**，于是关系在阈值上来回穿
    /// 越——这正是战争事件反复 `war_started`/`war_ended` 闪烁的原因。
    Notable,
    /// 后续计算**只需要前一帧**（本回合流水），或者根本不需要访问——只是记录下来供 agent
    /// 事后分析。→ 存档里只活一个回合；要跨段查询走磁盘投影 `idx/events.jsonl`。
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
    /// 一句话标题（见 [`GameEvent::headline`]）。**同一渲染器的唯一产物**：CLI 的
    /// `--milestones`/`--digest` 与投影表里的这一列说的是同一句话，不会各写一份文案。
    /// 它是**人读**的便利列，机器查询仍应走 `actor_*`/`target_*`/`data` 这些结构化列。
    pub headline: String,
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
            headline: self.headline(),
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
            GameEvent::CityRazed { city, owner, fallen_to, by_ship, damage, pop_before } => {
                set_actor(&mut r, EntityKind::Faction, fallen_to);
                set_target(&mut r, EntityKind::City, city);
                // 拆城的那艘舰是**次要发起方**：城际易主的「谁做的」在 actor（势力），
                // 「具体哪艘舰」在同一行的 extra 里，一次查询都拿得到。
                extra(&mut r, EventRole::Actor, EntityKind::Ship, by_ship);
                // 失去这座城的一方是**受损方**——与 `city_defected`/`city_overrun` 的
                // `from` 同槽位，于是「一座城的历史」用同一个查询形状就能读全。
                extra(&mut r, EventRole::Victim, EntityKind::Faction, owner);
                r.magnitude = *damage;
                r.data = json!({"city": city, "owner": owner, "fallen_to": fallen_to,
                                "by_ship": by_ship, "damage": damage, "pop_before": pop_before});
            }
            GameEvent::ShipSpawned { ship, owner, class, city, via, blueprint } => {
                set_actor(&mut r, EntityKind::Faction, owner);
                set_target(&mut r, EntityKind::Ship, ship);
                if let Some(c) = city {
                    extra(&mut r, EventRole::Third, EntityKind::City, c);
                }
                r.data = json!({"ship": ship, "owner": owner, "class": class, "city": city,
                                "via": via, "blueprint": blueprint});
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
            GameEvent::CargoLoaded { ship, faction, body, cargo } => {
                set_actor(&mut r, EntityKind::Ship, ship);
                set_target(&mut r, EntityKind::Body, body);
                extra(&mut r, EventRole::Third, EntityKind::Faction, faction);
                r.data = json!({"ship": ship, "faction": faction, "body": body, "cargo": cargo});
            }
            GameEvent::CargoDelivered { ship, faction, body, cargo, into_pool } => {
                set_actor(&mut r, EntityKind::Ship, ship);
                set_target(&mut r, EntityKind::Body, body);
                extra(&mut r, EventRole::Third, EntityKind::Faction, faction);
                r.data = json!({"ship": ship, "faction": faction, "body": body,
                                "cargo": cargo, "into_pool": into_pool});
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
            GameEvent::Revolt { .. } => "revolt",
            GameEvent::CityDefected { .. } => "city_defected",
            GameEvent::CoalitionFormed { .. } => "coalition_formed",
            GameEvent::CoalitionEnded { .. } => "coalition_ended",
            GameEvent::CapitalRelocated { .. } => "capital_relocated",
            GameEvent::CargoLoaded { .. } => "cargo_loaded",
            GameEvent::CargoDelivered { .. } => "cargo_delivered",
        }
    }

    /// 分级：按**后续计算的访问需求**定级（判据见 [`Salience`]），决定它被存进哪一层。
    ///
    /// **定级的依据 = 生产代码的读者盘点**（非测试的 `state.events` / `state.notables` /
    /// `state.milestones` 读者）：
    ///
    /// | 读者 | 访问的历史 |
    /// |---|---|
    /// | `step_diplomacy` → `war_scar_floor`（记恨地板） | **最近 `war_scar_rounds` 回合**的战争 |
    /// | `step_diplomacy` 的「本回合谁和谁交火」（由 `Attack`/`Siege` 反推） | 本回合 |
    /// | `step_ideology` → `military_deltas(&state.events)` | 本回合的军事得失 |
    /// | `kill_ship` 的同回合去重 | 本回合 |
    /// | `step_story` 的载荷提取 + `StoryTrigger::*` | 本回合 / [`State::chronicle`] |
    /// | `autocontrol::tactics` 的 `Withdraw` 判定 | 本回合 |
    /// | `agent::state_json`、`projection` 的渲染 | 本回合 |
    ///
    /// **结论：没有 variant 有「无限过去」读者，所以 `Milestone` 组当前为空。** 这是判据的
    /// 正确结果，不是遗漏——仅有的无限过去需求（`first_war`/`first_raze`/`first_colony` 的
    /// 「史上第一次」）由 [`State::chronicle`] 的 `id` 去重承担，历史层不必为它们长存。
    /// 将来哪个机制真的需要无限回看，再把对应 variant **提升**进 `Milestone` 组。
    ///
    /// 兜底分支是有意的：按本判据 **`Detail` 是缺省**（「不被任何计算读取」是安全假设），
    /// 而「提升」是一个需要指明读者的**主动**动作。新增 variant 时编译器不再强迫你定级
    /// ——这正是判据从「重要性」换成「读者需求」的代价，由本段文档承担提醒。
    pub fn salience(&self) -> Salience {
        match self {
            // 唯一的窗口读者：记恨地板（`sim::war_scar_floor`）要回看「最近打过仗吗」。
            GameEvent::WarStarted { .. } | GameEvent::WarEnded { .. } => Salience::Notable,
            // 其余全部只需要前一帧，或根本不被计算读取（只供 agent 事后分析）。
            _ => Salience::Detail,
        }
    }

    /// 一句话标题：**人读**的那一行（`第 47 回合：中国夷平火星-殖民城…`）。
    ///
    /// 三条设计约束：
    /// 1. **自足**——只读事件自身的字段，不接受 `&State`。于是它对**已经归档的历史**
    ///    （长存里程碑里跨 checkpoint 的老事件、投影表里的行）同样成立：那些实体可能早已
    ///    不存在/已改名，回查 state 只会得到**今天的**答案而不是当时的答案。
    /// 2. **单行**——不含换行，可直接进 JSONL / 表格 / 终端一行。
    /// 3. **点到名**——[`GameEvent::participants`] 列出的每一个 id 都**逐字出现**在标题里
    ///    （守卫测试 `headline_names_every_participant` 钉住）。这让「索引指向谁」与
    ///    「人读到的句子说的是谁」不可能分叉。
    ///
    /// 这里是**穷尽 match**：新增 variant 时编译器强迫你写它的那一句话，与
    /// [`GameEvent::history_row`]/[`GameEvent::salience`] 同样的「漏不掉」纪律。
    pub fn headline(&self) -> String {
        match self {
            GameEvent::Attack { attacker, target, damage } => {
                format!("{attacker} 对 {target} 开火（{} 伤害）", num(*damage))
            }
            GameEvent::ShipDestroyed { ship, owner, class, cause, by } => match (cause, by) {
                (DeathCause::Combat, Some(k)) => format!(
                    "{} 的 {}「{ship}」被 {} 的 {killer} 击毁（{weapon}）",
                    owner,
                    class,
                    k.faction,
                    killer = k.ship,
                    weapon = k.weapon,
                ),
                (DeathCause::Combat, None) => {
                    format!("{owner} 的 {class}「{ship}」战沉")
                }
                (DeathCause::UpkeepShortfall, _) => {
                    format!("{owner} 的 {class}「{ship}」因维护费欠缴锈蚀报废")
                }
                (DeathCause::Scrapped, _) => format!("{owner} 的 {class}「{ship}」被拆解"),
            },
            GameEvent::Siege { attacker, city, damage } => {
                format!("{attacker} 轰击城 {city}（{} 伤害）", num(*damage))
            }
            GameEvent::CityRazed { city, owner, fallen_to, by_ship, damage, pop_before } => format!(
                "{fallen_to} 的 {by_ship} 夷平 {owner} 的 {city}（人口 {pop_before} → 0，{} 伤害）",
                num(*damage)
            ),
            GameEvent::ShipSpawned { ship, owner, class, city, via, blueprint } => {
                let base = match (via, city) {
                    (SpawnVia::Shipyard, Some(c)) => {
                        format!("{owner} 的 {c} 出厂一艘 {class}「{ship}」")
                    }
                    (SpawnVia::Shipyard, None) => format!("{owner} 出厂一艘 {class}「{ship}」"),
                    (SpawnVia::Story, _) => {
                        format!("{owner} 因剧情得到一艘 {class}「{ship}」")
                    }
                };
                // 只有真挂了设计图才多说一句——无图那一路的句子**逐字不变**（digest 拿它
                // 当故事板，凭空加字会改变既有基线的输出）。
                match blueprint {
                    Some(bp) => format!("{base}（设计图：{bp}）"),
                    None => base,
                }
            }
            GameEvent::ColonyFounded { city, owner, body, how, prev_owner, .. } => match (how, prev_owner) {
                (FoundingHow::NewSite, _) => format!("{owner} 在 {body} 新建城市 {city}"),
                (FoundingHow::Refounded, Some(p)) => {
                    format!("{owner} 在 {body} 复垦 {p} 留下的废墟 {city}")
                }
                (FoundingHow::Refounded, None) => format!("{owner} 在 {body} 复垦 {city}"),
            },
            GameEvent::StaleOrder { ship, reason } => format!("{ship} 的指令失效（{reason}）"),
            GameEvent::Withdraw { ship, to_body } => format!("{ship} 撤往 {to_body} 修整"),
            GameEvent::WarStarted { a, b } => format!("{a} 与 {b} 开战"),
            GameEvent::WarEnded { a, b } => format!("{a} 与 {b} 停战"),
            GameEvent::Story { title, participants, .. } => {
                if participants.is_empty() {
                    title.clone()
                } else {
                    format!("{title}（{}）", participants.join("、"))
                }
            }
            GameEvent::Revolt { city, faction, loyalty } => format!(
                "{faction} 的 {city} 叛乱，城市化为废墟（忠诚 {}）",
                num(*loyalty)
            ),
            GameEvent::CityDefected { city, from, to, loyalty } => format!(
                "{from} 的 {city} 倒戈至 {to}（忠诚 {}）",
                num(*loyalty)
            ),
            GameEvent::CoalitionFormed { hegemon, members } => format!(
                "{} 结成联盟对抗霸权 {hegemon}",
                members.join("、")
            ),
            GameEvent::CoalitionEnded { hegemon, members } => format!(
                "反 {hegemon} 联盟解体（原成员 {}）",
                members.join("、")
            ),
            GameEvent::CapitalRelocated { faction, from, to, reason } => format!(
                "{faction} 迁都 {from} → {to}（{}）",
                capital_reason(reason)
            ),
            // 标题必须点到名：`faction` 也是本事件的参与方（`extra` 里的第三角色），
            // 所以它必须逐字出现在标题里（守卫 `headline_names_every_participant` 钉住这条）。
            GameEvent::CargoLoaded { ship, faction, body, cargo } => {
                format!("{faction} 的 {ship} 在 {body} 装 {}", cargo_summary(cargo))
            }
            GameEvent::CargoDelivered { ship, faction, body, cargo, into_pool } => format!(
                "{faction} 的 {ship} 在 {body} 卸 {}（{}）",
                cargo_summary(cargo),
                if *into_pool { "入首都池" } else { "入中转货栈" }
            ),
        }
    }

    /// **展示排序权重**：一个窗口内事件多于展示上限时，按此权重取最重要的若干条。
    ///
    /// **量纲是一个 0–9 的序数阶梯，不是 0–100 的分数**（实测投影里出现的值只有
    /// `0 / 2 / 5 / 7 / 8 / 9`）：`9` = 世界格局（开战/停战/结盟/迁都）、`8` = 城市易主或毁灭、
    /// `7` = 势力重建与剧情节拍、`5` = 舰的存亡、`2` = 撤退/指令降级、`0` = 逐发流水（开火/围城）。
    /// 所以「值得一读」的门槛是 **≥ 8**（格局 + 地图要重画的事），不是 60 之类的分数阈值
    /// ——踩过这个坑：`q.storyboard()` 一开始把门槛写成 60，于是**静默返回空表**。
    ///
    /// 这是**展示**用的排序键，不是模拟数值——它决定「故事板里先看到哪句话」，不影响任何
    /// 游戏机制，因此按本仓库「数值进 config」的惯例在此**不必**外置成配置表（外置反而会让
    /// 「新增 variant 必须声明权重」这条编译期纪律失效）。若将来真要按剧本调节叙事重点，
    /// 再把它改成读 `config` 的穷尽 match 即可。
    ///
    /// 与 [`GameEvent::salience`] 的分工：`salience` 回答「**谁要回看它**」（分层/存储），
    /// `weight` 回答「**人读起来重不重要**」（排序）。两者刻意分开——混用会让「挑值得读的
    /// 事件」在里程碑层清空后静默变空。
    pub fn weight(&self) -> u8 {
        match self {
            // 世界格局级：开战/停战、结盟/解体、迁都。
            GameEvent::WarStarted { .. }
            | GameEvent::WarEnded { .. }
            | GameEvent::CoalitionFormed { .. }
            | GameEvent::CoalitionEnded { .. }
            | GameEvent::CapitalRelocated { .. } => 9,
            // 城市易主/毁灭：地图要重画的事件。
            GameEvent::CityRazed { .. }
            | GameEvent::CityDefected { .. }
            | GameEvent::ColonyFounded { .. }
            | GameEvent::Revolt { .. } => 8,
            // 剧情节拍。
            GameEvent::Story { .. } => 7,
            // 舰的存亡。
            GameEvent::ShipDestroyed { .. } | GameEvent::ShipSpawned { .. } => 5,
            // 值得注意但不改变归属。
            GameEvent::Withdraw { .. }
            | GameEvent::StaleOrder { .. }
            | GameEvent::CargoLoaded { .. }
            | GameEvent::CargoDelivered { .. } => 2,
            // 逐发流水：按判据连读者都没有（只为 agent 分析而记录），也不该出现在故事板里。
            GameEvent::Attack { .. } | GameEvent::Siege { .. } => 0,
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

/// 标题里的人数：整数不带小数点，否则保留一位（标题给人读，精确值在 `data` 里）。
fn num(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

/// 一批货的可读摘要（`铁 6、碳 4`）。判据同 [`num`]：标题给人读，精确值在 `data` 里。
/// [`ResourceMap`] 是 `BTreeMap` ⇒ 名字序，同批货的标题不会因为遍历顺序而抖。
fn cargo_summary(cargo: &ResourceMap) -> String {
    if cargo.is_empty() {
        return "空舱".to_string();
    }
    cargo
        .iter()
        .map(|(rt, amt)| format!("{rt} {}", num(*amt)))
        .collect::<Vec<_>>()
        .join("、")
}

/// 迁都原因码 → 中文。机器码本身在投影 `data.reason` 里保持原样（查询用码、人读用这句）。
fn capital_reason(reason: &str) -> &str {
    match reason {
        "destroyed" => "首都沦陷，自动改立",
        "ai_review" => "周期性重估后更优",
        other => other,
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

/// 历史层里的一条记录：**第几回合**发生了什么。`GameEvent` 本身不带回合号（它只活在
/// 「本回合」的流水里），跨回合的存档必须自己带上时间戳——这是本类型存在的唯一理由。
///
/// [`Milestones`] 与 [`Notables`] 共用它：两层的差别只在**保留多久**，不在记录形状。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HistoryEntry {
    pub round: u32,
    pub event: GameEvent,
}

/// **长存里程碑层**（`State::milestones`）：后续计算需要访问**无限过去**的事件，按发生顺序追加。
///
/// **当前为空，而且这是判据的正确结果，不是遗漏。** 定级判据是「后续计算需要访问哪一段历史」
/// （见 [`Salience`]），而读者盘点（见 [`GameEvent::salience`]）表明：**没有任何生产逻辑读无限
/// 过去**。仅有的无限过去需求——`first_war`/`first_raze`/`first_colony` 的「史上第一次」——由
/// [`State::chronicle`] 的 `id` 去重承担，历史层不必为它们长存。
///
/// 所以本层**刻意不预填**：将来哪个机制真的需要无限回看，再把对应 variant 提升进来。它保留
/// 字段是为了给那条判据一个现成的家，而不是为了装「听起来重要」的事件。（曾经这里按「重要性」
/// 预填了 14 个 variant，结果吃掉存档 67%——见 `.agents/notes/sparse-history.md`。）
///
/// 与 [`Notables`] 的分工是**保留期**，不是重要性：本层无限，[`Notables`] 只有窗口。
/// 写入点是 [`crate::sim::ev`]（发事件的唯一漏斗），因此「该记而没记」在结构上不可能。
///
/// **容量**：默认不设上限（`history.max_milestones: 0` = 无损）；一旦截断，丢弃量与丢到哪一
/// 回合都记在 [`Milestones::dropped`]/[`Milestones::dropped_through_round`] 里——截断可见。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Milestones {
    /// 里程碑记录，最旧 → 最新。
    pub entries: Vec<HistoryEntry>,
    /// 因超出上限而被丢弃的记录数（`0` = 无损）。
    #[serde(default)]
    pub dropped: u64,
    /// 已被丢弃的记录所覆盖到的最后一个回合（`dropped > 0` 时，
    /// `entries` 从这之后开始，之前的历史只剩「丢了 `dropped` 条」这一条事实）。
    #[serde(default)]
    pub dropped_through_round: u32,
}

impl Milestones {
    /// 追加一条。**非 [`Salience::Milestone`] 直接忽略**——分层只在 [`GameEvent::salience`]
    /// 一处声明。当前没有任何 variant 定级为 `Milestone`，所以这是个空操作。
    pub fn push(&mut self, round: u32, event: GameEvent) {
        if event.salience() != Salience::Milestone {
            return;
        }
        self.entries.push(HistoryEntry { round, event });
    }

    /// 按上限裁剪（`cap == 0` = 不设上限，无损）。丢弃**最旧**的记录，并把丢弃量与丢弃
    /// 到的回合数记进本层自身——下游据此知道「这不是全部历史」。
    pub fn trim(&mut self, cap: usize) {
        if cap == 0 || self.entries.len() <= cap {
            return;
        }
        let excess = self.entries.len() - cap;
        self.dropped_through_round = self.entries[excess - 1].round;
        self.dropped += excess as u64;
        self.entries.drain(..excess);
    }

    /// 里程碑层是否完整（没有因上限而丢过记录）。
    pub fn is_complete(&self) -> bool {
        self.dropped == 0
    }

    /// 某一实体参与过的全部里程碑（`kind`/`id` 与投影的索引口径一致：**名字即 id**）。
    /// Rust 侧的等价物，供 CLI/测试使用；Python 侧走 `q.history()` 查投影。
    pub fn history_of(&self, kind: EntityKind, id: &str) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.event.participants().iter().any(|p| p.kind == kind && p.id == id))
            .collect()
    }
}

/// **窗口层**（`State::notables`）：后续计算需要访问**一定事件窗口**的事件，只保留最近
/// `history.notable_window` 个回合。
///
/// 与 [`Milestones`] 的差别只有**保留期**：本层会被裁剪，且裁剪是**预期行为而非损失**——
/// 出了窗口的历史按判据就不该再影响任何计算。因此这里**不记 `dropped`**：过期不是丢弃。
///
/// 本层当前的**唯一生产者与消费者**是战争：[`GameEvent::WarStarted`] /
/// [`GameEvent::WarEnded`]（判据范例见 [`Salience::Notable`]），读者是
/// [`crate::sim::war_scar_floor`]（记恨地板）——它回头看「最近打过仗吗」。
///
/// `window == 0` = **不裁剪**（与 `max_milestones: 0` 同义：无损）。注意此时本层退化成无限
/// 长存，那说明这条判据被绕过了——真要无限长存，该提升进 [`Milestones`]。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Notables {
    /// 窗口内的记录，最旧 → 最新。
    pub entries: Vec<HistoryEntry>,
}

impl Notables {
    /// 追加一条。**非 [`Salience::Notable`] 直接忽略**——分层只在 [`GameEvent::salience`]
    /// 一处声明。
    pub fn push(&mut self, round: u32, event: GameEvent) {
        if event.salience() != Salience::Notable {
            return;
        }
        self.entries.push(HistoryEntry { round, event });
    }

    /// 丢弃已滑出窗口的记录（`window == 0` = 不裁剪）。窗口是 `[round - window + 1, round]`：
    /// **本回合仍在窗口内**，所以 `window = 1` 等价于「只看本回合」。
    pub fn trim(&mut self, round: u32, window: usize) {
        if window == 0 {
            return;
        }
        let window = window as u32;
        let oldest = round.saturating_sub(window - 1);
        self.entries.retain(|e| e.round >= oldest);
    }

    /// 窗口内是否有这条事件（`kind`/`id` 口径与投影一致：**名字即 id**）。
    pub fn history_of(&self, kind: EntityKind, id: &str) -> Vec<&HistoryEntry> {
        self.entries
            .iter()
            .filter(|e| e.event.participants().iter().any(|p| p.kind == kind && p.id == id))
            .collect()
    }
}

/// 一条剧情编年史记录：回合里发生的一次「叙事事件」，带标题、正文与参与方。
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
