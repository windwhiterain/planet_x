//! Procedural generation of a default start state (`--start` not given).
//!
//! This builds a recognizable Solar-system sandbox: the bodies from the
//! design doc (Earth with its city clusters, the ice giants, the Kuiper-belt
//! dwarf planets), one city per faction, and a small starting navy. The seed
//! jitters starting positions/relations so different seeds lead to visibly
//! different trajectories.

use crate::model::*;
use crate::prng::Prng;
use std::collections::BTreeMap;

// Named faction ids so the world is easy to read and stable across seeds.
pub const F_UN: u32 = 0;
pub const F_US: u32 = 1;
pub const F_EU: u32 = 2;
pub const F_CN: u32 = 3;
pub const F_RU: u32 = 4;
pub const F_MINING: u32 = 5;
pub const F_SCIENCE: u32 = 6;
pub const F_TRANSPORT: u32 = 7;
pub const F_CULT: u32 = 8;

/// Build a body on a Keplerian orbit. `dir_angle_deg` rotates the aphelion
/// direction so distinct bodies on similar orbits do not overlap.
fn body(
    id: BodyId,
    name: &str,
    semi_major: f64,
    e: f64,
    dir_angle_deg: f64,
    settlement: Option<Settlement>,
) -> Body {
    let a = semi_major;
    let peri = a * (1.0 - e);
    let aphe = a * (1.0 + e);
    let theta = dir_angle_deg.to_radians();
    let orbit = Orbit {
        perihelion_distance: peri as f32,
        aphelion_distance: aphe as f32,
        aphelion_direction: [theta.cos() as f32, theta.sin() as f32],
        // Kepler's third law: period (years) = a^1.5; convert to months.
        period: (a.powf(1.5) * 12.0) as f32,
    };
    Body {
        id,
        name: name.to_string(),
        orbit,
        settlement,
    }
}

fn deposit(rt: ResourceType, area: f32) -> ResourceDeposit {
    ResourceDeposit {
        resource_type: rt,
        area,
    }
}

fn mine(rt: ResourceType, area: f32) -> MiningPoint {
    MiningPoint {
        resource_type: rt,
        area,
    }
}

/// Building a standard city on a body, owned by `faction`, with a given
/// population and construction setup.
fn city(
    id: CityId,
    name: &str,
    body_id: BodyId,
    faction: FactionId,
    population: u32,
    construction: Option<ShipClass>,
    mining_points: Vec<MiningPoint>,
) -> City {
    City {
        id,
        name: name.to_string(),
        body_id,
        faction_id: faction,
        population,
        construction: construction.map(|target| ConstructionPoint {
            progress: 0.0,
            target,
        }),
        mining_points,
        defense: 40.0,
    }
}

fn stockpile(items: &[(ResourceType, f64)]) -> BTreeMap<ResourceType, f64> {
    items.iter().cloned().collect()
}

fn faction(id: FactionId, name: &str, color: char, resources: BTreeMap<ResourceType, f64>) -> Faction {
    Faction {
        id,
        name: name.to_string(),
        color,
        resources,
        relations: BTreeMap::new(),
    }
}

/// Build the default start state (round 0) for a given seed. The world is
/// procedurally generated in code; only balance values come from `config`.
pub fn default_state(config: &GameConfig, seed: u64) -> State {
    let mut rng = Prng::new(seed);

    // --- Bodies -------------------------------------------------------------
    let bodies = vec![
        body(0, "水星", 0.39, 0.206, 20.0, None),
        body(1, "金星", 0.72, 0.007, 95.0, None),
        body(
            2,
            "地球",
            1.00,
            0.017,
            10.0,
            Some(Settlement {
                population_capacity: 9000,
                construction_speed: 2.0,
                resources: vec![
                    deposit(ResourceType::WaterIce, 300.0),
                    deposit(ResourceType::Carbon, 280.0),
                    deposit(ResourceType::Silicon, 260.0),
                    deposit(ResourceType::Iron, 220.0),
                    deposit(ResourceType::Uranium, 60.0),
                ],
            }),
        ),
        body(
            3,
            "火星",
            1.52,
            0.093,
            140.0,
            Some(Settlement {
                population_capacity: 4000,
                construction_speed: 1.6,
                resources: vec![
                    deposit(ResourceType::Iron, 300.0),
                    deposit(ResourceType::Silicon, 260.0),
                    deposit(ResourceType::Carbon, 180.0),
                    deposit(ResourceType::WaterIce, 120.0),
                ],
            }),
        ),
        body(
            4,
            "谷神星",
            2.77,
            0.078,
            220.0,
            Some(Settlement {
                population_capacity: 1500,
                construction_speed: 1.1,
                resources: vec![
                    deposit(ResourceType::WaterIce, 400.0),
                    deposit(ResourceType::Carbon, 300.0),
                    deposit(ResourceType::Iron, 200.0),
                    deposit(ResourceType::Gold, 40.0),
                ],
            }),
        ),
        body(5, "木星", 5.20, 0.049, 30.0, None),
        body(6, "土星", 9.58, 0.057, 70.0, None),
        body(7, "天王星", 19.2, 0.046, 120.0, None),
        body(8, "海王星", 30.05, 0.009, 200.0, None),
        body(
            9,
            "冥王星",
            39.48,
            0.249,
            40.0,
            Some(Settlement {
                population_capacity: 900,
                construction_speed: 0.9,
                resources: vec![
                    deposit(ResourceType::Methane, 300.0),
                    deposit(ResourceType::Hydrogen, 220.0),
                    deposit(ResourceType::WaterIce, 180.0),
                ],
            }),
        ),
        body(10, "卡戎", 39.48, 0.12, 55.0, None),
        body(
            11,
            "阋神星",
            67.78,
            0.44,
            160.0,
            Some(Settlement {
                population_capacity: 500,
                construction_speed: 0.7,
                resources: vec![
                    deposit(ResourceType::Helium3, 200.0),
                    deposit(ResourceType::Methane, 180.0),
                    deposit(ResourceType::Hydrogen, 150.0),
                    deposit(ResourceType::WaterIce, 120.0),
                ],
            }),
        ),
        body(
            12,
            "妊神星",
            43.22,
            0.19,
            250.0,
            Some(Settlement {
                population_capacity: 600,
                construction_speed: 0.8,
                resources: vec![
                    deposit(ResourceType::WaterIce, 320.0),
                    deposit(ResourceType::Methane, 200.0),
                    deposit(ResourceType::Hydrogen, 170.0),
                    deposit(ResourceType::Helium3, 110.0),
                ],
            }),
        ),
        body(13, "创神星", 45.43, 0.16, 300.0, None),
        body(
            14,
            "伊克西翁",
            39.70,
            0.24,
            330.0,
            Some(Settlement {
                population_capacity: 400,
                construction_speed: 0.8,
                resources: vec![
                    deposit(ResourceType::Uranium, 160.0),
                    deposit(ResourceType::Thorium, 120.0),
                    deposit(ResourceType::Platinum, 60.0),
                    deposit(ResourceType::Methane, 90.0),
                ],
            }),
        ),
    ];

    // --- Factions -----------------------------------------------------------
    let mut factions = vec![
        faction(
            F_UN,
            "联合国",
            'U',
            stockpile(&[(ResourceType::Iron, 4.0), (ResourceType::Carbon, 4.0)]),
        ),
        faction(
            F_US,
            "美国",
            'A',
            stockpile(&[
                (ResourceType::Iron, 6.0),
                (ResourceType::Carbon, 5.0),
                (ResourceType::Uranium, 1.0),
            ]),
        ),
        faction(
            F_EU,
            "欧盟",
            'E',
            stockpile(&[(ResourceType::Iron, 5.0), (ResourceType::Carbon, 5.0)]),
        ),
        faction(
            F_CN,
            "中国",
            'C',
            stockpile(&[
                (ResourceType::Iron, 7.0),
                (ResourceType::Carbon, 6.0),
                (ResourceType::Silicon, 2.0),
            ]),
        ),
        faction(
            F_RU,
            "俄罗斯",
            'R',
            stockpile(&[
                (ResourceType::Iron, 5.0),
                (ResourceType::Carbon, 4.0),
                (ResourceType::Uranium, 2.0),
            ]),
        ),
        faction(
            F_MINING,
            "星系矿业",
            'M',
            stockpile(&[
                (ResourceType::Iron, 6.0),
                (ResourceType::Gold, 2.0),
                (ResourceType::Platinum, 1.0),
            ]),
        ),
        faction(
            F_SCIENCE,
            "无国界科学组织",
            'S',
            stockpile(&[
                (ResourceType::Silicon, 4.0),
                (ResourceType::Helium3, 3.0),
                (ResourceType::Carbon, 2.0),
            ]),
        ),
        faction(
            F_TRANSPORT,
            "深空运输联盟",
            'T',
            stockpile(&[
                (ResourceType::Carbon, 6.0),
                (ResourceType::Hydrogen, 3.0),
                (ResourceType::Iron, 2.0),
            ]),
        ),
        faction(
            F_CULT,
            "行星X崇拜教",
            'X',
            stockpile(&[
                (ResourceType::Uranium, 3.0),
                (ResourceType::Thorium, 2.0),
                (ResourceType::Gold, 1.0),
            ]),
        ),
    ];

    // --- Diplomacy: set up a couple of live wars and a hostile cult --------
    // relation(faction, target, value): value <= -20 means hostilities.
    let set_rel = |x: &mut [Faction], a: u32, b: u32, v: f64| {
        x[a as usize].relations.insert(b, v);
        x[b as usize].relations.insert(a, v);
    };
    // Cult is hostile to everyone, and everyone is hostile to the cult.
    for f in 0..9 {
        if f != F_CULT {
            set_rel(&mut factions, F_CULT, f, -55.0 + rng.range_f64(-15.0, 15.0));
        }
    }
    // Cold-war / hot conflict between the great powers.
    set_rel(&mut factions, F_US, F_CN, -38.0 + rng.range_f64(-8.0, 8.0));
    set_rel(&mut factions, F_US, F_RU, -26.0 + rng.range_f64(-8.0, 8.0));
    set_rel(&mut factions, F_US, F_EU, 6.0 + rng.range_f64(-3.0, 3.0));
    set_rel(&mut factions, F_CN, F_RU, -12.0 + rng.range_f64(-6.0, 6.0));
    set_rel(&mut factions, F_CN, F_EU, 8.0 + rng.range_f64(-3.0, 3.0));
    // A mining/transport rivalry (not yet a war, but tensions).
    set_rel(&mut factions, F_MINING, F_TRANSPORT, -14.0 + rng.range_f64(-6.0, 6.0));

    // --- Cities -------------------------------------------------------------
    let cities = vec![
        city(
            0,
            "长三角城市群",
            2,
            F_CN,
            3200,
            Some(ShipClass::Corvette),
            vec![mine(ResourceType::Iron, 40.0), mine(ResourceType::Silicon, 30.0)],
        ),
        city(
            1,
            "珠三角城市群",
            2,
            F_CN,
            2400,
            Some(ShipClass::Cruiser),
            vec![mine(ResourceType::Carbon, 40.0), mine(ResourceType::Uranium, 8.0)],
        ),
        city(
            2,
            "奥林匹斯港",
            3,
            F_US,
            2200,
            Some(ShipClass::Cruiser),
            vec![mine(ResourceType::Iron, 50.0), mine(ResourceType::Silicon, 30.0)],
        ),
        city(
            3,
            "盖亚站",
            3,
            F_UN,
            1200,
            Some(ShipClass::Corvette),
            vec![mine(ResourceType::Carbon, 30.0), mine(ResourceType::WaterIce, 15.0)],
        ),
        city(
            4,
            "谷神星采矿站",
            4,
            F_MINING,
            1100,
            Some(ShipClass::Transport),
            vec![
                mine(ResourceType::WaterIce, 60.0),
                mine(ResourceType::Carbon, 40.0),
                mine(ResourceType::Gold, 6.0),
            ],
        ),
        city(
            5,
            "冥王星前哨",
            9,
            F_RU,
            700,
            Some(ShipClass::Corvette),
            vec![mine(ResourceType::Methane, 40.0), mine(ResourceType::Hydrogen, 25.0)],
        ),
        city(
            6,
            "埃里斯前哨",
            11,
            F_SCIENCE,
            500,
            Some(ShipClass::Corvette),
            vec![mine(ResourceType::Helium3, 30.0), mine(ResourceType::Methane, 20.0)],
        ),
        city(
            7,
            "妊神星转运站",
            12,
            F_TRANSPORT,
            600,
            Some(ShipClass::Transport),
            vec![mine(ResourceType::WaterIce, 40.0), mine(ResourceType::Methane, 25.0)],
        ),
        city(
            8,
            "伊克西翁圣所",
            14,
            F_CULT,
            400,
            Some(ShipClass::Cruiser),
            vec![
                mine(ResourceType::Uranium, 20.0),
                mine(ResourceType::Thorium, 14.0),
                mine(ResourceType::Platinum, 6.0),
            ],
        ),
    ];

    // --- Starting navy -------------------------------------------------------
    let mut ships = Vec::new();
    let mut next_ship = 0u32;
    let mut add_ship = |ships: &mut Vec<Ship>, faction: FactionId, body_id: BodyId, class: ShipClass| {
        let pos = bodies[body_id as usize].orbit.position(0.0);
        let spec = config.ship_spec(class);
        ships.push(Ship {
            id: next_ship,
            name: format!("{}-{}", spec.label, faction),
            class,
            faction_id: faction,
            position: [pos[0] + rng.range_f64(-0.05, 0.05), pos[1] + rng.range_f64(-0.05, 0.05)],
            hull: spec.hull,
            target: None,
        });
        next_ship += 1;
    };

    add_ship(&mut ships, F_CN, 2, ShipClass::Corvette);
    add_ship(&mut ships, F_CN, 2, ShipClass::Corvette);
    add_ship(&mut ships, F_US, 3, ShipClass::Corvette);
    add_ship(&mut ships, F_US, 3, ShipClass::Cruiser);
    add_ship(&mut ships, F_EU, 2, ShipClass::Corvette);
    add_ship(&mut ships, F_RU, 9, ShipClass::Corvette);
    add_ship(&mut ships, F_UN, 3, ShipClass::Corvette);
    add_ship(&mut ships, F_MINING, 4, ShipClass::Corvette);
    add_ship(&mut ships, F_MINING, 4, ShipClass::Transport);
    add_ship(&mut ships, F_SCIENCE, 11, ShipClass::Corvette);
    add_ship(&mut ships, F_TRANSPORT, 12, ShipClass::Transport);
    add_ship(&mut ships, F_CULT, 14, ShipClass::Cruiser);
    add_ship(&mut ships, F_CULT, 14, ShipClass::Cruiser);
    add_ship(&mut ships, F_CULT, 14, ShipClass::Corvette);

    State {
        round: 0,
        time_month: 0.0,
        bodies,
        cities,
        factions,
        ships,
    }
}
