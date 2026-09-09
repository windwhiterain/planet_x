use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{BodyId, CityId, FactionId, ShipId};

/// 一回合内发生的、值得 agent 知道的事件。每回合开始时被清空、回合演化中被
/// 追加；agent 无需反推状态差即可得知「谁开火/谁被毁/哪城被夷平/谁殖民」。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameEvent {
    /// 开火：攻击者对目标舰造成 damage 伤害。
    Attack { attacker: ShipId, target: ShipId, damage: f64 },
    /// 舰被击毁（hull ≤ 0）。
    ShipDestroyed { ship: ShipId, owner: FactionId, class: String },
    /// 围城：攻击者对本回合城市建筑造成 damage 伤害。
    Siege { attacker: ShipId, city: CityId, damage: f64 },
    /// 城市被夷平（razed），可再殖民。
    CityRazed { city: CityId, fallen_to: FactionId },
    /// 新舰从某城出厂。
    ShipSpawned { ship: ShipId, owner: FactionId, class: String, city: CityId },
    /// 新殖民 / 再殖民城市建立。
    ColonyFounded { city: CityId, owner: FactionId, body: BodyId, seeded_ship_class: String },
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
    Resurgence { faction: FactionId, body: BodyId, ship: ShipId },
    /// 离心叛乱（光速治理的代价）：城市忠诚度跌破叛变阈值，居民脱离其统治势力，
    /// 城市被夷平为空白（可再殖民）。这是超大帝国管理廉价的远方殖民地失败的结果。
    Revolt { city: CityId, faction: FactionId },
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
