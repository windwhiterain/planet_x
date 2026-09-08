//! Core data model for the Planet X sandbox.
//!
//! The whole world is one [`State`]. Every field is serializable to and from
//! RON so that a start state can be supplied by the user (`--start`) and so
//! that round snapshots can be dumped as a trajectory.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identifiers are plain integers. References between entities use these ids so
/// that snapshots stay compact and easy to diff.
pub type FactionId = u32;
pub type BodyId = u32;
pub type CityId = u32;
pub type ShipId = u32;

/// A mineable resource type.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResourceType {
    WaterIce,
    Helium3,
    Uranium,
    Thorium,
    Gold,
    Platinum,
    Iron,
    Hydrogen,
    Methane,
    Carbon,
    Silicon,
}

impl ResourceType {
    /// Short human-readable name (used for the CLI and ASCII map keys).
    pub fn name(self) -> &'static str {
        match self {
            ResourceType::WaterIce => "水冰",
            ResourceType::Helium3 => "氦-3",
            ResourceType::Uranium => "铀",
            ResourceType::Thorium => "钍",
            ResourceType::Gold => "金",
            ResourceType::Platinum => "铂",
            ResourceType::Iron => "铁",
            ResourceType::Hydrogen => "氢",
            ResourceType::Methane => "甲烷",
            ResourceType::Carbon => "碳",
            ResourceType::Silicon => "硅",
        }
    }
}

/// A deposit of a single resource on a settlement. `area` bounds how much a
/// mining point may carve out from this deposit.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ResourceDeposit {
    pub resource_type: ResourceType,
    pub area: f32,
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

/// A habitable place on a body; can hold a city. Resources here bound how much
/// the cities built on this body may mine.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settlement {
    /// 人口容量 (how many people the settlement can support).
    pub population_capacity: u32,
    /// 建设速度 (construction speed multiplier for cities on this body).
    pub construction_speed: f64,
    /// 资源 deposits (type + area).
    pub resources: Vec<ResourceDeposit>,
}

/// A celestial body. Only some bodies host a [`Settlement`].
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Body {
    pub id: BodyId,
    pub name: String,
    pub orbit: Orbit,
    pub settlement: Option<Settlement>,
}

/// A mining point on a city: draws a given resource at a given area.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MiningPoint {
    pub resource_type: ResourceType,
    pub area: f32,
}

/// A construction point on a city, used to build spaceships. It accumulates
/// `progress` each round and spawns a ship once the target class is complete.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConstructionPoint {
    pub progress: f64,
    pub target: ShipClass,
}

/// A ship class with the combat/economy stats that define it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ShipClass {
    Corvette,
    Cruiser,
    Transport,
}

impl ShipClass {
    pub const ALL: [ShipClass; 3] = [ShipClass::Corvette, ShipClass::Cruiser, ShipClass::Transport];
}

/// Balance statistics for a ship class. Loaded from `config/game.ron`; nothing
/// game-balance related lives in code.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShipSpec {
    pub label: String,
    pub hull: f64,
    pub attack: f64,
    /// Distance covered per round, in AU.
    pub speed: f64,
    /// Engagement distance, in AU.
    pub attack_range: f64,
    /// Build points required to finish this class at a construction point.
    pub build_points: f64,
    /// Resource cost to fully build a ship of this class.
    pub build_cost: Vec<(ResourceType, f64)>,
}

/// Economy tuning (production and population).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EconomyConfig {
    /// Mined resource units per unit of mine area per round at full efficiency.
    pub production_rate: f64,
    /// Fractional population growth toward a settlement's capacity each round.
    pub pop_growth: f64,
    /// Floor on production efficiency.
    pub min_efficiency: f64,
}

/// Combat tuning.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CombatConfig {
    /// A relation at or below this value is treated as hostile (war).
    pub war_threshold: f64,
    /// Distance, in AU, within which a ship can besiege a city.
    pub siege_range: f64,
    /// Distance, in AU, at which a ship is considered to have arrived.
    pub arrival_eps: f64,
    /// Starting garrison/defense value of a freshly built city.
    pub defense_initial: f64,
    /// Defense value a city is reset to after being captured.
    pub defense_reset: f64,
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
    /// Ship statistics, keyed by class.
    pub ships: BTreeMap<ShipClass, ShipSpec>,
}

impl GameConfig {
    pub fn ship_spec(&self, class: ShipClass) -> &ShipSpec {
        self.ships
            .get(&class)
            .expect("game config is missing a ship class")
    }
}

/// Which direction a ship is currently heading.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum ShipTarget {
    Body(BodyId),
    Ship(ShipId),
    City(CityId),
    Position([f64; 2]),
}

/// A spaceship. Always owned by a faction.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Ship {
    pub id: ShipId,
    pub name: String,
    pub class: ShipClass,
    pub faction_id: FactionId,
    /// Position in AU (same plane as the orbits).
    pub position: [f64; 2],
    pub hull: f64,
    pub target: Option<ShipTarget>,
}

/// A city on a settlement, controlled by a faction.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct City {
    pub id: CityId,
    pub name: String,
    pub body_id: BodyId,
    pub faction_id: FactionId,
    /// 人口, limits production efficiency.
    pub population: u32,
    /// 建造点 (optional): can build ships.
    pub construction: Option<ConstructionPoint>,
    /// 开采点 (mining points).
    pub mining_points: Vec<MiningPoint>,
    /// Siege damage that has built up against this city.
    pub defense: f64,
}

/// A faction (势力) with diplomatic stances toward every other faction.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Faction {
    pub id: FactionId,
    pub name: String,
    pub color: char,
    /// Stockpiled resources.
    pub resources: BTreeMap<ResourceType, f64>,
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

    /// Resolve the current world position of a body (`months` is time already
    /// reflected by orbit.position), as a plain AU pair.
    pub fn body_position(&self, id: BodyId) -> [f64; 2] {
        self.body(id)
            .map(|b| b.orbit.position(self.time_month as f32))
            .unwrap_or([0.0, 0.0])
    }
}
