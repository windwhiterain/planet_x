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
///
/// `value` is the resource's market price per unit (abstract credits): rare
/// minerals (氦-3/钍/铀/金/铂) are worth several times the common industrial
/// ones (铁/碳/硅/氢/甲烷/水冰). The automatic market trades surpluses for the
/// minerals a faction is short of, priced on these values.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceDef {
    pub name: String,
    pub value: f64,
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
    /// Per-round maintenance (in market value / credits) per ship — a continuous
    /// sink that caps fleet growth and makes big fleets expensive to sustain.
    pub upkeep: f64,
    /// 组件槽位数：本舰级可同时装配多少个「定制组件」（武器/防御/推进/辅助）。
    /// 0 = 不可定制（裸舰，仅按 class 基础面板）。这是「飞船定制化」的能力上限——
    /// 更大的舰级能承载更多组件，把资源优势转化为战斗力。
    #[serde(default)]
    pub slots: u32,
}

/// A ship component (舰船定制组件) keyed by id in the config. Fitting a component
/// onto a ship (during production) **changes what the ship can actually do in
/// combat**, not just a stat delta:
///
/// * weapon systems (`category="weapon"`) introduce a weapon with its own damage
///   **type** (kinetic/plasma/missile), engagement `range`, `tracking` (how well it
///   leads a fast target), and shield/hull multipliers. Choosing *which* weapons
///   you fit decides how you fight — so a 铀-rich faction fields anti-armor railguns,
///   a 金/铂-rich one fields shields + plasma, a 氦-3/氢-rich one fields fast
///   missile skirmishers. This is the resource→military link, and it is qualitative.
/// * defense (`category="defense"`) adds an energy **shield pool** (soaks damage
///   preferentially, regenerates), extra **hull** armor, or **point-defense**
///   interceptors that shoot down incoming missiles.
/// * thrust / utility (`category="thrust"|"utility"`) add speed, regen, etc.
///
/// The offensive baseline (a ship's class gun battery) is always present; the
/// fitted components are layered on top. All values are deterministic (no RNG in
/// combat; evasion is a deterministic function of target speed vs weapon tracking).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ComponentSpec {
    pub label: String,
    /// 组件类别：`"weapon"` / `"defense"` / `"thrust"` / `"utility"`——决定它在
    /// 战斗里扮演的角色（类别不直接进数值，数值由下列字段决定）。
    pub category: String,
    /// 武器伤害（非武器为 0）。对舰/对城都按「伤害类型 × 防御」结算。
    pub damage: f64,
    /// 武器伤害类型：`"kinetic"` / `"plasma"` / `"missile"`；非武器为 `""`。
    pub damage_type: String,
    /// 武器交战距离（AU）：只有目标进入此距离该武器才参与齐射——所以「射程」是
    /// 真实的战位选择，而非一个加分项。
    pub range: f64,
    /// 武器追踪能力（AU/月）：越高的武器越能咬住高速目标（跑得快的船对低追踪
    /// 武器规避强）。deterministic 命中 = 1 - evasion(target_speed, tracking)。
    pub tracking: f64,
    /// 武器对能量护盾的伤害倍率（护盾先吸收；kinetic×低、plasma×高）。
    pub shield_mult: f64,
    /// 武器对船体（装甲）的伤害倍率。
    pub hull_mult: f64,
    /// 能量护盾池（防御组件加）：0 = 无护盾。护盾池每回合再生。
    pub shield: f64,
    /// 护盾再生（占 shield_max / 月）。
    pub shield_regen: f64,
    /// 额外船体护甲（防御组件加）。
    pub hull: f64,
    /// 点防御拦截强度（防御组件加）：拦下来袭导弹（确定性按强度折算导弹伤害）。
    pub intercept: f64,
    /// 额外速度（AU/月）。
    pub speed: f64,
    /// 额外船体再生（占 hull_max / 月）。
    pub hull_regen: f64,
    /// 额外每回合维护费（市场价值/舰）。组件越强，舰队越难养——抑制无脑堆强组件。
    pub upkeep: f64,
    /// 一次性装配成本（在造舰出厂时从势力库存扣除）。绑定资源优势的落点。
    pub cost: ResourceMap,
}

/// A concrete weapon a ship fields (its class battery plus any fitted weapon
/// components). `Copy` so the hot combat path iterates it cheaply.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct Weapon {
    /// 单发齐射伤害。
    pub damage: f64,
    /// 交战距离（AU）。
    pub range: f64,
    /// 追踪能力（AU/月）——越高越难被高速目标规避。
    pub tracking: f64,
    /// 对护盾伤害倍率。
    pub shield_mult: f64,
    /// 对船体伤害倍率。
    pub hull_mult: f64,
    /// 伤害类型（0=kinetic, 1=plasma, 2=missile）。
    pub kind: u8,
}

/// The effective combat panel of a ship after its fitted components are applied.
/// Aggregated so movement / upkeep / regen read one struct; per-weapon resolution
/// (which weapon fires, at what damage, vs shield vs hull) is handled separately by
/// [`ship_weapons`].
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
pub struct ShipPanel {
    /// 舰级炮台+组件的总攻击（用于对城轰炸的简化结算与报告）。
    pub attack: f64,
    pub attack_range: f64,
    pub speed: f64,
    pub hull_max: f64,
    pub hull_regen: f64,
    pub shield_max: f64,
    pub shield_regen: f64,
    pub intercept: f64,
    pub upkeep: f64,
}

/// Damage type tags used in weapon resolution.
pub const WEAPON_KINETIC: u8 = 0;
pub const WEAPON_PLASMA: u8 = 1;
pub const WEAPON_MISSILE: u8 = 2;

fn weapon_kind(t: &str) -> u8 {
    match t {
        "plasma" => WEAPON_PLASMA,
        "missile" => WEAPON_MISSILE,
        _ => WEAPON_KINETIC,
    }
}

/// Compute a ship's effective combat panel from its class spec plus its fitted
/// components. Pure & deterministic (no RNG); cheap enough for the hot loop.
/// A component's 完整度 (integrity) when freshly installed: weapons/shields/armor each
/// add to how long it survives battle damage, with a floor so trivial parts are not
/// one-shot.
pub fn component_integrity(config: &GameConfig, id: &str) -> f64 {
    let cs = config.component_spec(id);
    (cs.shield + cs.hull + cs.damage * 0.4).max(18.0)
}

/// A ship's component at index `i`'s effectiveness (0..1): its current integrity
/// fraction. A damaged module contributes less (a shot-out weapon fires weaker) —
/// the mechanical basis of gradual combat degradation. A missing `component_hp`
/// (legacy / bare ships) is fully effective; an emptied one is dead (0). 
pub fn component_effectiveness(config: &GameConfig, ship: &Ship, i: usize) -> f64 {
    let Some(hp) = ship.component_hp.get(i) else {
        return 1.0; // 无完整度记录（裸舰/旧数据）：视为完好。
    };
    if ship.components.get(i).map(|c| component_integrity(config, c)).unwrap_or(1.0) <= 1e-9 {
        return 1.0;
    }
    (hp / component_integrity(config, &ship.components[i])).clamp(0.0, 1.0)
}

pub fn ship_panel(config: &GameConfig, ship: &Ship) -> ShipPanel {
    let base = config.ship_spec(&ship.class);
    let mut p = ShipPanel {
        attack: base.attack,
        attack_range: base.attack_range,
        speed: base.speed,
        hull_max: base.hull,
        hull_regen: base.hull_regen,
        shield_max: 0.0,
        shield_regen: 0.0,
        intercept: 0.0,
        upkeep: base.upkeep,
    };
    for (i, c) in ship.components.iter().enumerate() {
        let eff = component_effectiveness(config, ship, i);
        if eff <= 1e-9 {
            continue; // 彻底被击毁的组件不再贡献面板。
        }
        if let Some(cs) = config.components.get(c) {
            p.attack += cs.damage * eff;
            p.speed += cs.speed * eff;
            p.hull_max += cs.hull * eff;
            p.hull_regen += cs.hull_regen * eff;
            p.shield_max += cs.shield * eff;
            p.shield_regen += cs.shield_regen * eff;
            if cs.range > p.attack_range {
                p.attack_range = cs.range;
            }
            p.intercept += cs.intercept * eff;
            p.upkeep += cs.upkeep * eff;
        }
    }
    p
}

/// Enumerate the weapons a ship fields: always its class gun battery (a kinetic
/// array) plus each fitted weapon component. `kind` marks the damage type so the
/// combat resolver can apply shield/hull multipliers and missile interception.
pub fn ship_weapons(config: &GameConfig, ship: &Ship) -> Vec<Weapon> {
    let base = config.ship_spec(&ship.class);
    let mut ws = Vec::new();
    if base.attack > 0.0 {
        ws.push(Weapon {
            damage: base.attack,
            range: base.attack_range,
            tracking: 2.0, // 舰级炮台 = 中等追踪（快船能规避）
            shield_mult: 0.6,
            hull_mult: 1.0,
            kind: WEAPON_KINETIC,
        });
    }
    for (i, c) in ship.components.iter().enumerate() {
        let eff = component_effectiveness(config, ship, i);
        if eff <= 1e-9 {
            continue; // 彻底被击毁的武器组件不再开火。
        }
        if let Some(cs) = config.components.get(c) {
            if cs.category == "weapon" && cs.damage > 0.0 {
                ws.push(Weapon {
                    damage: cs.damage * eff,
                    range: cs.range,
                    tracking: cs.tracking,
                    shield_mult: cs.shield_mult,
                    hull_mult: cs.hull_mult,
                    kind: weapon_kind(&cs.damage_type),
                });
            }
        }
    }
    ws
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
    /// Current hull (armor) — must never exceed [`Self::hull_max`].
    pub hull: f64,
    /// 本舰最大护甲（含组件加成）。`hull` 是当前值；再生/损毁以 `hull_max` 为上限。
    #[serde(default = "default_hull_max")]
    pub hull_max: f64,
    /// 当前能量护盾值（护盾优先吸收、每回合再生，见 `shield_regen`）。
    #[serde(default)]
    pub shield: f64,
    /// 最大能量护盾（护盾组件的 `shield` 加总；无护盾组件为 0）。
    #[serde(default)]
    pub shield_max: f64,
    /// 本舰装配的组件 id（舰船定制）。空 = 裸舰（仅按 class 基础面板）。
    /// 由模拟在造舰出厂时确定性挑选并扣成本；agent 可直读以了解舰队构成。
    #[serde(default)]
    pub components: Vec<String>,
    /// 每个组件的完整度（与 `components` 同下标）。战斗中被击中会「溢出」损坏组件——
    /// 完整度 ≤0 即该组件被击毁，不贡献面板/武器（渐进丧失战力，而非满血抗到壳破）。
    /// 空 = 旧数据/裸舰（视为全部完好）。
    #[serde(default)]
    pub component_hp: Vec<f64>,
}

fn default_hull_max() -> f64 {
    0.0
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
    /// 忠诚度 (0..1)：城市对其统治势力的向心力。治理到位（能付清光速管理费）时
    /// 向「距离目标」恢复；欠费时下降；低于叛变阈值则爆发「离心叛乱」，城市被
    /// 夷平为空白。这让超大帝国难以维持遥远殖民地——光速治理延迟的体现。
    #[serde(default = "default_loyalty")]
    pub loyalty: f64,
}

fn default_loyalty() -> f64 {
    1.0
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
    /// 意识形态位置（约 -1..1；越负越「东方/教派」，越正越「西方/国际」）。
    /// 由它推算出两两之间的静息亲和（阵营亲缘），驱动外交漂移，令国际关系波动。
    pub alignment: f64,
    /// 好战度（0..1）：越高的势力越会加速与异己阵营走向敌对。
    pub aggression: f64,
    /// 首都天体（光速治理的锚点）：治理开销随城市距此天体的距离递增，忠诚度随
    /// 距离衰减——超大帝国难以远距离管辖其在远离首都的殖民地。
    #[serde(default = "default_capital_body")]
    pub capital_body: BodyId,
    /// 本土防御半径（AU）：本方城市/舰在此半径（距 `capital_body`）内获得本土防御。
    /// cult 的数值按 MOND 异常放大——它「掌握了正确的牛顿修正引力」，孤悬柯伊伯带，
    /// 被围攻时依靠此异常自保。
    #[serde(default = "default_home_radius")]
    pub home_radius: f64,
    /// 在本方本土区域内，敌方对其造成的伤害倍率（<1 = 削弱入侵者）。
    #[serde(default = "default_home_attack_mult")]
    pub home_attack_mult: f64,
    /// 在本方本土区域内，本方舰只的额外护甲再生（占最大护甲/回合）。
    #[serde(default = "default_home_regen_bonus")]
    pub home_regen_bonus: f64,
}

fn default_capital_body() -> BodyId {
    2 // 地球, the default anchor when a .ron state omits it.
}

fn default_home_radius() -> f64 {
    0.0
}

fn default_home_attack_mult() -> f64 {
    1.0
}

fn default_home_regen_bonus() -> f64 {
    0.0
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
    /// 剧情编年史：本局已发生的叙事事件（按发生先后追加）。这是「剧情丰富」的载体——
    /// agent 用 `story` 命令/查询即可读到整段已展开的故事弧；`#[serde(default)]` 让旧的
    /// `.ron` 状态缺字段也能正常加载。
    #[serde(default)]
    pub chronicle: Vec<ChronicleEntry>,
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

    /// 决定某城「娱乐/福利预算」由谁控制：城市 → 天体 → 势力 → 全局。
    pub fn loyalty_budget_control(&self, fid: FactionId, cid: CityId) -> ControlMode {
        let leaf = self.control(fid).and_then(|c| c.loyalty_budget.get(&cid)).and_then(|c| c.mode);
        let city = self.scope.cities.get(&cid).copied().flatten();
        let body_id = self.city(cid).map(|c| c.body_id);
        let body = body_id.and_then(|bid| self.scope.bodies.get(&bid).copied().flatten());
        let faction = self.scope.factions.get(&fid).copied().flatten();
        resolve_chain(&[leaf, city, body, faction, self.scope.global])
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
    /// 本方各城的「娱乐/福利预算」（每城一个 Control，市场价值/回合）：把资源投入
    /// 城市娱乐以提升忠诚度。这是「枪支与黄油」的现实权衡——花钱安抚居民，就少了
    /// 建设与造舰的预算；玩家/agent 用它来稳固对大/远城市的统治（见治理模型）。
    #[serde(default)]
    pub loyalty_budget: BTreeMap<CityId, Control<f64>>,
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
    /// 它（`TargetShip{attack:false}` 的守卫行为：贴近旗舰 + 拦截进入射程之敌），保护
    /// 高价值舰种。0 = 关闭。
    #[serde(default = "default_escort_range")]
    pub escort_range: f64,
    /// 追击半径（AU）：AI 舰只在**此半径内**才会去追敌对舰——避免跨越全图去追一艘远逃
    /// 的敌舰（过度延伸、漂移、被伏击）。超过此半径的敌舰不在追击范围（转而去轰炸城市/
    /// 殖民/护卫）。这是拟人的「不追远敌、就近平守」。0 = 不限（总是追）。
    #[serde(default = "default_pursuit_range")]
    pub pursuit_range: f64,
}

fn default_retreat_hull() -> f64 {
    0.28
}

fn default_retreat_min_dist() -> f64 {
    1.5
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
    /// Resting affinity at maximum ideological distance (opposite blocs).
    pub affinity_floor: f64,
    /// Extra affinity at full ideological closeness (same bloc allies).
    pub affinity_span: f64,
    /// Random fluctuation per round, so relations oscillate and can cross the
    /// war threshold.
    pub noise: f64,
    /// Lower clamp on any relation (bounded hostility).
    pub hostility_floor: f64,
    /// Upper clamp on any relation (bounded friendliness).
    pub friendship_ceiling: f64,
}

/// Market tuning. An automatic interstellar exchange that lets each faction buy
/// the minerals it is short of (so shipyards rarely stall on a single drought)
/// by selling its scarce-value surpluses. This gives the economy a **sink** for
/// surplus stockpiles and a **supply** that keeps a faction building even when
/// it cannot mine a keystone mineral (e.g. carbon).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MarketConfig {
    /// Per-round cap, in market value (credits), on how much a faction may
    /// auto-trade. `0.0` disables the market entirely.
    pub auto_trade_limit: f64,
    /// Each yard-critical resource is kept at least this many units in stock;
    /// the market tops it up when it falls short.
    pub working_buffer: f64,
    /// Market fee: a faction sells `(1+spread)`× worth to buy `1×` worth, a
    /// small friction that stops trades from being perfect conversions.
    pub spread: f64,
}

/// 一条剧情编年史记录：回合里发生的一次「叙事事件」，带标题、正文与参与方。
///
/// 这是「剧情丰富」的可读载体——一条 `Story` 剧情事件在本回合触发时，除了记入
/// [`State::events`]（本回合流水），还把这个完整条目追加进 [`State::chronicle`]，
/// 供 agent 随时查询整段已展开的故事弧。
#[derive(Serialize, Deserialize, Clone, Debug)]
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
    /// 非 master 势力在异常区内，每超出 [`Self::radius`] 1 AU 的导航偏移量（AU）。
    pub drift_per_au: f64,
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
    pub coalition_estrange: f64,
    /// 经济制裁：当一个反制联盟（≥ [`Self::min_members`]）成立并对霸权实施封锁时，
    /// 霸权保留的自动市场交易额度比例（0..1；1 = 不制裁）。这会给一家独大的经济体
    /// 造成资源封锁与失衡——它难以再靠市场兑换到短缺矿物（如铀/氦-3），产业受抑。
    pub sanction_trade_mult: f64,
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
            sanction_trade_mult: 1.0,
            sanction_cost_mult: 1.6,
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
    pub governance: GovernanceConfig,
    pub mond: MondConfig,
    /// 合纵连横 / 均势外交（弱者联盟对抗霸权）。`#[serde(default)]` 容忍旧配置无此节。
    #[serde(default)]
    pub balance: BalanceOfPowerConfig,
    /// Resource definitions (key -> display metadata). This is the source of
    /// truth for which resource keys exist.
    pub resources: BTreeMap<String, ResourceDef>,
    /// Building-structure definitions (混凝土 / 钢结构). Source of truth for
    /// which structure keys exist.
    pub structures: BTreeMap<String, StructureSpec>,
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
