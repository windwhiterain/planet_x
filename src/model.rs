//! Core data model for the Planet X sandbox.
//!
//! The whole world is one [`State`]. Every field is serializable to and from
//! RON so that a start state can be supplied by the user (`--start`) and so
//! that round snapshots can be dumped as a trajectory.
//!
//! Everything that should be tunable is **data-driven**: resources, building
//! kinds and ship classes are string keys resolved against the config tables
//! in `config/game.ron`, and a resource bundle is a plain dictionary
//! (key → value). No entity or resource is hard-coded as an enum here.
//!
//! Buildings are **not atomic**: a building is a continuous `area` allocation
//! within a settlement's bounded total area. The kinds are open-ended (any
//! combination of housing / mining / shipyard), but the total area is finite,
//! so the numbers above all stay continuous.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identifiers are plain integers. References between entities use these ids so
/// that snapshots stay compact and easy to diff.
pub type FactionId = u32;
pub type BodyId = u32;
pub type CityId = u32;
pub type ShipId = u32;
pub type BuildingId = u32;

/// A resource bundle: resource key -> amount. Resource keys are configurable
/// and resolved against [`GameConfig::resources`].
pub type ResourceMap = BTreeMap<String, f64>;

/// A resource type definition (display metadata for a resource key).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceDef {
    pub name: String,
}

/// A deposit of a single resource on a settlement. `area` bounds how much
/// mining may be carved out of this deposit.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceDeposit {
    pub resource: String,
    pub area: f64,
}

/// 2D orbit around the central sun. The focus (sun) sits at the origin.
///
/// The aphelion direction is a unit vector pointing from the sun toward the
/// farthest point of the orbit; perihelion is the opposite direction.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Orbit {
    /// 近日点距离 (perihelion distance), in AU.
    pub perihelion_distance: f32,
    /// 远日点距离 (aphelion distance), in AU.
    pub aphelion_distance: f32,
    /// 远日点方向 (unit vector toward aphelion).
    pub aphelion_direction: [f32; 2],
    /// 公转周期 in months.
    pub period: f32,
}

impl Orbit {
    pub fn semi_major_axis(&self) -> f64 {
        (self.perihelion_distance as f64 + self.aphelion_distance as f64) / 2.0
    }

    pub fn eccentricity(&self) -> f64 {
        let a = self.semi_major_axis();
        let peri = self.perihelion_distance as f64;
        let aphe = self.aphelion_distance as f64;
        if peri <= 0.0 || a == 0.0 {
            return 0.0;
        }
        (aphe - peri) / (aphe + peri)
    }

    /// World position (in AU) of the body after `months` elapsed since the
    /// epoch. At `months == 0` the body sits at perihelion.
    pub fn position(&self, months: f32) -> [f64; 2] {
        let m = self.semi_major_axis();
        let e = self.eccentricity();
        let period = self.period as f64;
        let mean_motion = std::f64::consts::TAU / period;
        // Mean anomaly at the given time (radians), starting at perihelion.
        let mut mean_anomaly = (mean_motion * months as f64) % std::f64::consts::TAU;
        if mean_anomaly < 0.0 {
            mean_anomaly += std::f64::consts::TAU;
        }
        // Solve Kepler's equation  M = E - e * sin(E)  for the eccentric anomaly.
        let mut ecc_anomaly = mean_anomaly;
        for _ in 0..24 {
            let f = ecc_anomaly - e * ecc_anomaly.sin() - mean_anomaly;
            let fp = 1.0 - e * ecc_anomaly.cos();
            let delta = f / fp;
            ecc_anomaly -= delta;
            if delta.abs() < 1e-9 {
                break;
            }
        }
        // True anomaly from perihelion.
        let half = ecc_anomaly / 2.0;
        let true_anomaly = 2.0 * ((1.0 + e).sqrt() * half.sin()).atan2((1.0 - e).sqrt() * half.cos());
        let r = m * (1.0 - e * ecc_anomaly.cos());

        // Perihelion direction is the opposite of the aphelion direction.
        let aphe = normalize2(self.aphelion_direction);
        let peri = [-aphe[0], -aphe[1]];
        let perp = [-peri[1], peri[0]];

        let cx = peri[0] * true_anomaly.cos() + perp[0] * true_anomaly.sin();
        let cy = peri[1] * true_anomaly.cos() + perp[1] * true_anomaly.sin();
        [cx * r, cy * r]
    }
}

fn normalize2(v: [f32; 2]) -> [f64; 2] {
    let len = ((v[0] as f64).powi(2) + (v[1] as f64).powi(2)).sqrt();
    if len <= 1e-12 {
        [1.0, 0.0]
    } else {
        [v[0] as f64 / len, v[1] as f64 / len]
    }
}

/// A habitable place (定居点) on a body. 定居点与城市一一对应：一个定居点至多
/// 容纳一座城市（见 [`City::settlement`]）。它的面积有限——坐落在其上的城市的
/// 建筑必须装得下；它的资源矿藏限定本定居点上采矿的上限。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settlement {
    /// 定居点名（地球的五大城市群各占一个定居点；气态巨行星的定居点为轨道空间站）。
    pub name: String,
    /// 总面积 (total buildable area).
    pub total_area: f64,
    /// 生态容量 (population per unit area).
    pub ecological_capacity: f64,
    /// 建设速度修正 (area built per unit time).
    pub construction_speed_mod: f64,
    /// 建设资源修正 (resources consumed per unit area built).
    pub construction_resource_mod: f64,
    /// 资源 deposits (resource key + area).
    pub resources: Vec<ResourceDeposit>,
}

/// A celestial body hosting zero or more 定居点 (settlement sites), each of which
/// hosts **at most one** city (settlement ↔ city 1:1). A body's settlements are
/// indexed; a city on this body points at its site via [`City::settlement`].
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Body {
    pub id: BodyId,
    pub name: String,
    pub orbit: Orbit,
    /// 当前位置 (current position in AU, recomputed each round from `orbit`).
    pub position: [f64; 2],
    pub settlements: Vec<Settlement>,
}

impl Body {
    pub fn settlement(&self, idx: usize) -> Option<&Settlement> {
        self.settlements.get(idx)
    }
}

/// A single continuous-area building allocation on a city. Not an atom:
/// `area` is the planned extent and `deployed` is how much is actually built.
///
/// `kind` is a config key (open-ended): `"residential"` (居住区),
/// `"mining"` (开采区) or `"construction"` (建造区). The district's
/// characteristic attributes live here next to the kind:
///   * a mining building (`kind == "mining"`) carries the mined resource key
///     in `resource`;
///   * a shipyard building (`kind == "construction"`) carries the ship class it
///     produces in `ship_type` (a command-controlled attribute);
///   * every building carries a `structure` ("concrete" | "steel"), the
///     building's own attribute controlling its hardness/armor and cost.
///
/// Buildings are identified by a stable [`BuildingId`] so a city can hold
/// several shipyards (one per `ship_type`). The command-controlled investment
/// weights live in [`ControllableState`] (keyed by [`InvestKey`]/[`BuildKey`]).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Building {
    pub id: BuildingId,
    pub kind: String,
    /// Mined resource key, for a mining (开采区) building.
    pub resource: Option<String>,
    /// Ship class produced, for a shipyard (建造区) building.
    pub ship_type: Option<String>,
    /// Building's own structure attribute: "concrete" 混凝土 | "steel" 钢结构.
    pub structure: String,
    pub area: f64,
    pub deployed: f64,
    /// Current hardness (armor). Max approaches `deployed × armor_per_area`.
    pub armor: f64,
}

impl Building {
    pub fn under_construction(&self) -> bool {
        self.deployed < self.area - 1e-9
    }

    /// Is this building a shipyard (建造区)?
    pub fn is_shipyard(&self) -> bool {
        self.kind == "construction"
    }

    /// The maximum hardness for the currently deployed area, using the config's
    /// per-structure armor-per-area table.
    pub fn armor_max(&self, config: &GameConfig) -> f64 {
        self.deployed * config.structure_spec(&self.structure).armor_per_area
    }
}

/// A ship class keyed by name in the config. The mechanics reference this key
/// only through the config tables.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShipSpec {
    pub label: String,
    /// 护甲 (hull points).
    pub hull: f64,
    /// 护甲再生 (%/时间): fraction of the max hull restored per round.
    pub hull_regen: f64,
    pub attack: f64,
    /// Distance covered per round, in AU.
    pub speed: f64,
    /// Engagement distance, in AU.
    pub attack_range: f64,
    /// Build points required to finish this class at a shipyard.
    pub build_points: f64,
    /// Resource cost to fully build a ship of this class.
    pub build_cost: ResourceMap,
}

/// A ship's controllable behavior — the instruction a faction issues to one
/// of its ships. This is command-controlled state (see [`ControllableState`]),
/// not an event: the simulation merely reads this to decide where to move and
/// what to fire/bombard.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ShipBehavior {
    /// 目标地点：移动到指定位置。
    Move { position: [f64; 2] },
    /// 目标飞船：`attack: true` 表示追袭并开火；`attack: false` 表示**守卫**——
    /// 靠近并保护这艘友方舰（对接近范围内的敌方舰开火拦截），而非攻击它。
    TargetShip { ship: ShipId, attack: bool },
    /// 目标定居点上的城市（bombard 表示是否轰炸/围攻）。
    TargetSettlement { city: CityId, bombard: bool },
    /// 停泊轨道：跟随某个天体——持续向该天体当前位置移动，随其轨道巡航/停靠。
    Dock { body: BodyId },
    /// 殖民：前往定居点天体并（再）建立一座城市。
    Colonize { body: BodyId },
    /// 待命（无指令，原地保持当前坐标——由 AI/玩家写入的默认值）。
    Idle,
}

/// 一回合内发生的、值得 agent 知道的事件。每回合开始时被清空、回合演化中被
/// 追加；agent 无需反推状态差即可得知「谁开火/谁被毁/哪城被夷平/谁殖民」。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
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
}

/// A spaceship. Always owned by a faction.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ship {
    pub id: ShipId,
    pub name: String,
    pub class: String,
    pub faction_id: FactionId,
    /// Position in AU (same plane as the orbits).
    pub position: [f64; 2],
    pub hull: f64,
}

/// A city occupying one 定居点 (settlement) on a body, controlled by a faction.
/// 定居点 ↔ 城市一一对应: `settlement` is the index (into `Body::settlements`)
/// of the site this city sits on, and a settlement hosts at most one city — a
/// razed city stays on its site as a blank, re-colonizable footprint until it
/// is re-seeded. The city's area is split among a set of continuous-area
/// [`Building`]s, bounded by its settlement's `total_area`.
///
/// Ship production (`ship_progress`) is **per city**, keyed by the ship class
/// (舰型). Each 建造区 (shipyard building) contributes to its class's rate; the
/// rates of every shipyard in the city keep contributing into that city pool.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct City {
    pub id: CityId,
    pub name: String,
    pub body_id: BodyId,
    /// Index of this city's 定居点 within `body.settlements` (1:1 occupancy).
    pub settlement: usize,
    pub faction_id: FactionId,
    /// 人口, limits production efficiency.
    pub population: u32,
    pub buildings: Vec<Building>,
    /// 建造进度 (以城市为单位): ship class -> progress.
    pub ship_progress: BTreeMap<String, f64>,
    /// 被夷平为空白 (razed): no buildings / population; colonizable again.
    pub razed: bool,
}

/// A faction (势力) with diplomatic stances toward every other faction.
///
/// The command-controlled budget lives in [`ControllableState::budget`], not
/// here.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Faction {
    pub id: FactionId,
    pub name: String,
    /// Marker glyph on the CLI ASCII map (e.g. 'U').
    pub symbol: char,
    /// Display colour (CSS hex string) shown in the web UI, defined directly
    /// on the faction itself.
    pub color: String,
    /// Stockpiled resources (key -> amount).
    pub resources: ResourceMap,
    /// Relation of this faction toward another faction. Negative means hostile.
    pub relations: BTreeMap<FactionId, f64>,
}

/// The complete world snapshot.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct State {
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
    #[serde(default)]
    pub events: Vec<GameEvent>,
}

impl State {
    pub fn body(&self, id: BodyId) -> Option<&Body> {
        self.bodies.iter().find(|b| b.id == id)
    }

    pub fn city(&self, id: CityId) -> Option<&City> {
        self.cities.iter().find(|c| c.id == id)
    }

    pub fn city_mut(&mut self, id: CityId) -> Option<&mut City> {
        self.cities.iter_mut().find(|c| c.id == id)
    }

    pub fn ship(&self, id: ShipId) -> Option<&Ship> {
        self.ships.iter().find(|s| s.id == id)
    }

    pub fn ship_mut(&mut self, id: ShipId) -> Option<&mut Ship> {
        self.ships.iter_mut().find(|s| s.id == id)
    }

    pub fn faction(&self, id: FactionId) -> Option<&Faction> {
        self.factions.iter().find(|f| f.id == id)
    }

    pub fn faction_mut(&mut self, id: FactionId) -> Option<&mut Faction> {
        self.factions.iter_mut().find(|f| f.id == id)
    }

    /// Resolve the current world position of a body. Uses the stored
    /// `position` field, which the simulation keeps current.
    pub fn body_position(&self, id: BodyId) -> [f64; 2] {
        self.body(id).map(|b| b.position).unwrap_or([0.0, 0.0])
    }

    /// The 定居点 (settlement) a city occupies — settlement ↔ city 1:1.
    pub fn city_settlement(&self, cid: CityId) -> Option<&Settlement> {
        let c = self.city(cid)?;
        self.body(c.body_id)?.settlement(c.settlement)
    }

    /// A body's settlement site at `idx`.
    pub fn body_settlement(&self, bid: BodyId, idx: usize) -> Option<&Settlement> {
        self.body(bid)?.settlement(idx)
    }

    /// Read one faction's controllable state.
    pub fn control(&self, fid: FactionId) -> Option<&ControllableState> {
        self.control.get(&fid)
    }

    /// Mutably borrow one faction's controllable state.
    pub fn control_mut(&mut self, fid: FactionId) -> Option<&mut ControllableState> {
        self.control.get_mut(&fid)
    }

    /// Current behavior (指令) of a ship, if any.
    pub fn ship_behavior(&self, ship_id: ShipId) -> Option<ShipBehavior> {
        let s = self.ship(ship_id)?;
        self.control(s.faction_id)?
            .ship_orders
            .get(&ship_id)
            .map(|c| c.value)
    }

    // --- 控制模式判定（沿作用域链上溯，最具体者优先） ----------------------

    /// 决定一艘舰的指令由谁控制：舰 → 势力 → 全局。
    pub fn ship_control(&self, ship_id: ShipId) -> ControlMode {
        let Some(s) = self.ship(ship_id) else {
            return ControlMode::Ai;
        };
        let fid = s.faction_id;
        let leaf = self.control(fid).and_then(|c| c.ship_orders.get(&ship_id)).and_then(|c| c.mode);
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某投资预算（建设用）由谁控制：资源 → 势力 → 全局。
    pub fn investment_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = self.control(fid).and_then(|c| c.investment_budget.get(resource)).and_then(|c| c.mode);
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某建造预算（造舰用）由谁控制：资源 → 势力 → 全局。
    pub fn construction_budget_control(&self, fid: FactionId, resource: &str) -> ControlMode {
        let leaf = self.control(fid).and_then(|c| c.construction_budget.get(resource)).and_then(|c| c.mode);
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, faction, self.scope.global])
    }

    /// 决定某建筑「建设投资权重」由谁控制：建筑 → 城市 → 天体 → 势力 → 全局。
    pub fn invest_control(&self, fid: FactionId, key: &InvestKey) -> ControlMode {
        let (cid, _) = key;
        let leaf = self.control(fid).and_then(|c| c.invest_weights.get(key)).and_then(|c| c.mode);
        let city = self.scope.cities.get(cid).copied().flatten();
        let body_id = self.city(*cid).map(|c| c.body_id);
        let body = body_id.and_then(|bid| self.scope.bodies.get(&bid).copied().flatten());
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }

    /// 决定某建造区「建造投资权重」由谁控制：建造区 → 城市 → 天体 → 势力 → 全局。
    pub fn build_control(&self, fid: FactionId, key: &BuildKey) -> ControlMode {
        let (cid, _) = key;
        let leaf = self.control(fid).and_then(|c| c.build_weights.get(key)).and_then(|c| c.mode);
        let city = self.scope.cities.get(cid).copied().flatten();
        let body_id = self.city(*cid).map(|c| c.body_id);
        let body = body_id.and_then(|bid| self.scope.bodies.get(&bid).copied().flatten());
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
    }
}

/// 沿作用域链（从具体到宽泛）取第一个显式设置的 `ControlMode`，全 `None` 则
/// 默认 [`ControlMode::Ai`]。
fn resolve_chain(chain: &[Option<ControlMode>]) -> ControlMode {
    chain.iter().find_map(|m| *m).unwrap_or(ControlMode::Ai)
}

impl ControlScope {
    /// 把 `other` 按节点叠加到 `self`：仅覆盖 `other` 中显式给定的节点；`global`
    /// 只有在 `other.global` 为 `Some` 时才被改写。节点值 `None` 表示「继承/清除
    /// 该层的显式指定」。
    pub fn overlay(&mut self, other: &ControlScope) {
        if other.global.is_some() {
            self.global = other.global;
        }
        self.factions.extend(other.factions.iter().map(|(k, v)| (*k, *v)));
        self.bodies.extend(other.bodies.iter().map(|(k, v)| (*k, *v)));
        self.cities.extend(other.cities.iter().map(|(k, v)| (*k, *v)));
    }
}

// --- 可控状态 (controllable / command-controlled state) --------------------

/// 建筑「建设投资权重」定位键：(城市, 建筑)。同一座城的每栋建筑一个值。
pub type InvestKey = (CityId, BuildingId);
/// 建造区「建造投资权重」定位键：(城市, 建造区建筑)。每座城的每个建造区一个值。
pub type BuildKey = (CityId, BuildingId);

/// 一个可控字段由「谁决定」：AI（系统每回合自动决策/改写）还是
/// Player（玩家指令，系统只读不改写）。
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub enum ControlMode {
    /// 系统自动决策（现有行为）。
    #[default]
    Ai,
    /// 玩家下达指令，系统只用该值。
    Player,
}

/// 一个可控叶子：值 + 由谁决定。
///
/// `mode = None` 表示未在此层显式指定，沿作用域链上溯继承
/// （舰 → 资源 → 城市 → 天体 → 势力 → 全局），全链 `None` 时默认 [`ControlMode::Ai`]。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Control<T> {
    pub value: T,
    pub mode: Option<ControlMode>,
}

impl<T> Control<T> {
    /// 一个由系统自动决策的可控值。
    pub fn ai(value: T) -> Self {
        Self { value, mode: Some(ControlMode::Ai) }
    }

    /// 一个由玩家指令决定的可控值。
    pub fn player(value: T) -> Self {
        Self { value, mode: Some(ControlMode::Player) }
    }

    /// 一个继承上层作用域的可控值。
    pub fn inherit(value: T) -> Self {
        Self { value, mode: None }
    }
}

/// 城市/天体/势力/全局 的控制作用域。这些不是 [`ControllableState`] 的字段，
/// 单独建一棵作用域树；判定可控叶子的 AI/玩家边界时沿链上溯。
///
/// `None` 表示该作用域没有显式指定，继承下一层（更宽泛）作用域。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ControlScope {
    pub global: Option<ControlMode>,
    pub factions: BTreeMap<FactionId, Option<ControlMode>>,
    pub bodies: BTreeMap<BodyId, Option<ControlMode>>,
    pub cities: BTreeMap<CityId, Option<ControlMode>>,
}

/// 单个势力的可控状态：所有「指令控制」的量的集合。
///
/// 实体演化结果（资源量、耐久、人口、防御…）不在其中；本结构体是被指令
/// 直接改写的状态。指令 = 对它的修改（diff）。
///
/// 每个叶子用 [`Control`] 包裹：值 + 谁决定它。舰/资源/建筑的粒度在各自的
/// `mode`；城市/天体/势力/全局这些更粗的作用域在 [`State::scope`]。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ControllableState {
    /// 本方各飞船的当前指令（每艘舰一个 Control）。
    pub ship_orders: BTreeMap<ShipId, Control<ShipBehavior>>,
    /// 投资预算（资源/时间）：决定拿出多少资源用于「建设（建筑）」，按各建筑
    /// 建设投资权重竞争（每资源一个 Control）。
    pub investment_budget: BTreeMap<String, Control<f64>>,
    /// 建造预算（资源/时间）：决定拿出多少资源用于「造舰」，按各建造区建造
    /// 投资权重竞争（每资源一个 Control）。
    pub construction_budget: BTreeMap<String, Control<f64>>,
    /// 本方各建筑的「建设投资权重」（每建筑一个 Control）。
    pub invest_weights: BTreeMap<InvestKey, Control<f64>>,
    /// 本方各建造区的「建造投资权重」（每建造区一个 Control）。
    pub build_weights: BTreeMap<BuildKey, Control<f64>>,
}

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
    /// Fraction of a building's lost armor repaired each round when not under
    /// bombardment.
    pub armor_regen: f64,
    /// Fraction of the settlement area a freshly founded (colonized) city may
    /// claim, capped for the initial footprint.
    pub colony_footprint: f64,
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

/// Diplomacy tuning.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiplomacyConfig {
    /// Relation change caused by a hostile attack.
    pub attack_delta: f64,
    /// Relation change caused by capturing an enemy city.
    pub capture_delta: f64,
    /// Drift of non-war relations back toward neutral each round.
    pub relax_rate: f64,
}

/// The whole game configuration, loaded from `config/game.ron`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameConfig {
    pub economy: EconomyConfig,
    pub combat: CombatConfig,
    pub diplomacy: DiplomacyConfig,
    /// Resource definitions (key -> display metadata). This is the source of
    /// truth for which resource keys exist.
    pub resources: BTreeMap<String, ResourceDef>,
    /// Building-structure definitions (混凝土 / 钢结构). Source of truth for
    /// which structure keys exist.
    pub structures: BTreeMap<String, StructureSpec>,
    /// Ship statistics, keyed by class name.
    pub ships: BTreeMap<String, ShipSpec>,
    /// Building statistics, keyed by building kind name.
    pub buildings: BTreeMap<String, BuildingSpec>,
}

impl GameConfig {
    pub fn ship_spec(&self, class: &str) -> &ShipSpec {
        self.ships
            .get(class)
            .expect("game config is missing a ship class")
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

    pub fn resource_name(&self, key: &str) -> String {
        self.resources
            .get(key)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| key.to_string())
    }
}

/// Building statistics for a building kind. Loaded from `config/game.ron`.
///
/// `role` identifies the mechanical behaviour in the simulation and is one of
/// `"housing"` (provides population capacity), `"mining"` (extracts the
/// building's resource) or `"shipyard"` (builds ships).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BuildingSpec {
    pub label: String,
    pub role: String,
    /// 建设速度 (base area this kind can be built per round).
    pub construction_speed: f64,
    /// 建设各类资源 (resources consumed per unit area).
    pub build_cost: ResourceMap,
    /// 员工需求 (population required per unit area, 人口/面积).
    pub staff_per_area: f64,
    /// 幸福度/生产效率修正 (productivity multiplier).
    pub productivity: f64,
    /// Default 建设投资权重 for buildings of this kind.
    pub default_invest_weight: f64,
    /// Default 建造投资权重 for shipyard (建造区) buildings of this kind.
    pub default_build_weight: f64,
}
