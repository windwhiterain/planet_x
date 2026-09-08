//! Procedural generation of a default start state (`--start` not given).
//!
//! This builds a recognizable Solar-system sandbox: the bodies from the
//! design doc (Earth with its city clusters, the ice giants, the Kuiper-belt
//! dwarf planets), one city per faction, and a small starting navy. The seed
//! jitters starting positions/relations so different seeds lead to visibly
//! different trajectories.
//!
//! Buildings are continuous-area allocations inside a settlement's finite area
//! (see the model). The default world just seeds a reasonable starting
//! footprint; the simulation expands it over time. All kinds/resources are
//! string keys resolved against the config.

use crate::model::*;
use crate::prng::Prng;

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
        period: (a.powf(1.5) * 12.0) as f32,
    };
    Body {
        id,
        name: name.to_string(),
        orbit,
        settlement,
    }
}

fn deposit(rt: &str, area: f64) -> ResourceDeposit {
    ResourceDeposit {
        resource: rt.to_string(),
        area,
    }
}

fn settlement(total_area: f64, ecocap: f64, speed: f64, res_mod: f64, resources: Vec<ResourceDeposit>) -> Settlement {
    Settlement {
        total_area,
        ecological_capacity: ecocap,
        construction_speed_mod: speed,
        construction_resource_mod: res_mod,
        resources,
    }
}

/// Seed a city's initial building footprint from its settlement's deposits,
/// sized so population is housed and a bit of mining/industry is up and running.
fn seed_buildings(s: &Settlement, population: u32) -> Vec<Building> {
    let mut buildings = Vec::new();

    let resid = (population as f64 / s.ecological_capacity).max(4.0);
    buildings.push(Building {
        kind: "residential".to_string(),
        resource: None,
        area: resid,
        deployed: resid,
    });

    let construction = (s.total_area * 0.15).clamp(3.0, 10.0);
    let mut budget = (s.total_area - resid - construction).max(0.0);
    for d in &s.resources {
        if budget <= 0.0 {
            break;
        }
        let area = d.area.min(budget);
        if area > 0.0 {
            buildings.push(Building {
                kind: "mining".to_string(),
                resource: Some(d.resource.clone()),
                area,
                deployed: area,
            });
            budget -= area;
        }
    }
    buildings.push(Building {
        kind: "construction".to_string(),
        resource: None,
        area: construction,
        deployed: construction,
    });
    buildings
}

fn city(
    id: CityId,
    name: &str,
    body_id: BodyId,
    faction: FactionId,
    population: u32,
    settlement: &Settlement,
    ship_class: &str,
) -> City {
    City {
        id,
        name: name.to_string(),
        body_id,
        faction_id: faction,
        population,
        buildings: seed_buildings(settlement, population),
        ship_build: ShipBuild {
            progress: 0.0,
            target_class: ship_class.to_string(),
        },
        defense: 40.0,
    }
}

fn stockpile(items: &[(&str, f64)]) -> ResourceMap {
    items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn faction(id: FactionId, name: &str, color: char, resources: ResourceMap) -> Faction {
    Faction {
        id,
        name: name.to_string(),
        color,
        resources,
        relations: std::collections::BTreeMap::new(),
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
            Some(settlement(
                120.0,
                25.0,
                2.4,
                1.0,
                vec![
                    deposit("water_ice", 40.0),
                    deposit("carbon", 36.0),
                    deposit("silicon", 32.0),
                    deposit("iron", 30.0),
                    deposit("uranium", 10.0),
                ],
            )),
        ),
        body(
            3,
            "火星",
            1.52,
            0.093,
            140.0,
            Some(settlement(
                60.0,
                16.0,
                1.8,
                1.0,
                vec![
                    deposit("iron", 40.0),
                    deposit("silicon", 30.0),
                    deposit("carbon", 20.0),
                    deposit("water_ice", 14.0),
                ],
            )),
        ),
        body(
            4,
            "谷神星",
            2.77,
            0.078,
            220.0,
            Some(settlement(
                40.0,
                11.0,
                1.2,
                1.0,
                vec![
                    deposit("water_ice", 40.0),
                    deposit("carbon", 32.0),
                    deposit("iron", 24.0),
                    deposit("gold", 5.0),
                ],
            )),
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
            Some(settlement(
                24.0,
                7.0,
                1.0,
                1.0,
                vec![
                    deposit("methane", 40.0),
                    deposit("hydrogen", 28.0),
                    deposit("water_ice", 22.0),
                ],
            )),
        ),
        body(10, "卡戎", 39.48, 0.12, 55.0, None),
        body(
            11,
            "阋神星",
            67.78,
            0.44,
            160.0,
            Some(settlement(
                18.0,
                5.0,
                0.8,
                1.0,
                vec![
                    deposit("helium3", 26.0),
                    deposit("methane", 22.0),
                    deposit("hydrogen", 18.0),
                    deposit("water_ice", 14.0),
                ],
            )),
        ),
        body(
            12,
            "妊神星",
            43.22,
            0.19,
            250.0,
            Some(settlement(
                20.0,
                5.0,
                0.9,
                1.0,
                vec![
                    deposit("water_ice", 36.0),
                    deposit("methane", 24.0),
                    deposit("hydrogen", 19.0),
                    deposit("helium3", 12.0),
                ],
            )),
        ),
        body(13, "创神星", 45.43, 0.16, 300.0, None),
        body(
            14,
            "伊克西翁",
            39.70,
            0.24,
            330.0,
            Some(settlement(
                16.0,
                4.0,
                0.8,
                1.0,
                vec![
                    deposit("uranium", 20.0),
                    deposit("thorium", 15.0),
                    deposit("platinum", 8.0),
                    deposit("methane", 12.0),
                ],
            )),
        ),
    ];

    // --- Factions -----------------------------------------------------------
    let mut factions = vec![
        faction(F_UN, "联合国", 'U', stockpile(&[("iron", 4.0), ("carbon", 4.0)])),
        faction(
            F_US,
            "美国",
            'A',
            stockpile(&[("iron", 6.0), ("carbon", 5.0), ("uranium", 1.0)]),
        ),
        faction(F_EU, "欧盟", 'E', stockpile(&[("iron", 5.0), ("carbon", 5.0)])),
        faction(
            F_CN,
            "中国",
            'C',
            stockpile(&[("iron", 7.0), ("carbon", 6.0), ("silicon", 2.0)]),
        ),
        faction(
            F_RU,
            "俄罗斯",
            'R',
            stockpile(&[("iron", 5.0), ("carbon", 4.0), ("uranium", 2.0)]),
        ),
        faction(
            F_MINING,
            "星系矿业",
            'M',
            stockpile(&[("iron", 6.0), ("gold", 2.0), ("platinum", 1.0)]),
        ),
        faction(
            F_SCIENCE,
            "无国界科学组织",
            'S',
            stockpile(&[("silicon", 4.0), ("helium3", 3.0), ("carbon", 2.0)]),
        ),
        faction(
            F_TRANSPORT,
            "深空运输联盟",
            'T',
            stockpile(&[("carbon", 6.0), ("hydrogen", 3.0), ("iron", 2.0)]),
        ),
        faction(
            F_CULT,
            "行星X崇拜教",
            'X',
            stockpile(&[("uranium", 3.0), ("thorium", 2.0), ("gold", 1.0)]),
        ),
    ];

    // --- Diplomacy: set up a couple of live wars and a hostile cult --------
    let set_rel = |x: &mut [Faction], a: u32, b: u32, v: f64| {
        x[a as usize].relations.insert(b, v);
        x[b as usize].relations.insert(a, v);
    };
    for f in 0..9 {
        if f != F_CULT {
            set_rel(&mut factions, F_CULT, f, -55.0 + rng.range_f64(-15.0, 15.0));
        }
    }
    set_rel(&mut factions, F_US, F_CN, -38.0 + rng.range_f64(-8.0, 8.0));
    set_rel(&mut factions, F_US, F_RU, -26.0 + rng.range_f64(-8.0, 8.0));
    set_rel(&mut factions, F_US, F_EU, 6.0 + rng.range_f64(-3.0, 3.0));
    set_rel(&mut factions, F_CN, F_RU, -12.0 + rng.range_f64(-6.0, 6.0));
    set_rel(&mut factions, F_CN, F_EU, 8.0 + rng.range_f64(-3.0, 3.0));
    set_rel(&mut factions, F_MINING, F_TRANSPORT, -14.0 + rng.range_f64(-6.0, 6.0));

    // --- Cities -------------------------------------------------------------
    let earth = bodies[2].settlement.as_ref().unwrap();
    let mars = bodies[3].settlement.as_ref().unwrap();
    let ceres = bodies[4].settlement.as_ref().unwrap();
    let pluto = bodies[9].settlement.as_ref().unwrap();
    let eris = bodies[11].settlement.as_ref().unwrap();
    let haumea = bodies[12].settlement.as_ref().unwrap();
    let ixion = bodies[14].settlement.as_ref().unwrap();

    let cities = vec![
        city(0, "长三角城市群", 2, F_CN, 1400, earth, "corvette"),
        city(1, "珠三角城市群", 2, F_CN, 1100, earth, "cruiser"),
        city(2, "奥林匹斯港", 3, F_US, 1000, mars, "cruiser"),
        city(3, "盖亚站", 3, F_UN, 600, mars, "corvette"),
        city(4, "谷神星采矿站", 4, F_MINING, 500, ceres, "transport"),
        city(5, "冥王星前哨", 9, F_RU, 360, pluto, "corvette"),
        city(6, "埃里斯前哨", 11, F_SCIENCE, 240, eris, "corvette"),
        city(7, "妊神星转运站", 12, F_TRANSPORT, 280, haumea, "transport"),
        city(8, "伊克西翁圣所", 14, F_CULT, 200, ixion, "cruiser"),
    ];

    // --- Starting navy ------------------------------------------------------
    let mut ships = Vec::new();
    let mut next_ship = 0u32;
    let mut add_ship = |ships: &mut Vec<Ship>, faction: FactionId, body_id: BodyId, class: &str| {
        let pos = bodies[body_id as usize].orbit.position(0.0);
        let spec = config.ship_spec(class);
        ships.push(Ship {
            id: next_ship,
            name: format!("{}-{}", spec.label, faction),
            class: class.to_string(),
            faction_id: faction,
            position: [pos[0] + rng.range_f64(-0.05, 0.05), pos[1] + rng.range_f64(-0.05, 0.05)],
            hull: spec.hull,
            target: None,
        });
        next_ship += 1;
    };

    add_ship(&mut ships, F_CN, 2, "corvette");
    add_ship(&mut ships, F_CN, 2, "corvette");
    add_ship(&mut ships, F_US, 3, "corvette");
    add_ship(&mut ships, F_US, 3, "cruiser");
    add_ship(&mut ships, F_EU, 2, "corvette");
    add_ship(&mut ships, F_RU, 9, "corvette");
    add_ship(&mut ships, F_UN, 3, "corvette");
    add_ship(&mut ships, F_MINING, 4, "corvette");
    add_ship(&mut ships, F_MINING, 4, "transport");
    add_ship(&mut ships, F_SCIENCE, 11, "corvette");
    add_ship(&mut ships, F_TRANSPORT, 12, "transport");
    add_ship(&mut ships, F_CULT, 14, "cruiser");
    add_ship(&mut ships, F_CULT, 14, "cruiser");
    add_ship(&mut ships, F_CULT, 14, "corvette");

    State {
        round: 0,
        time_month: 0.0,
        bodies,
        cities,
        factions,
        ships,
    }
}
