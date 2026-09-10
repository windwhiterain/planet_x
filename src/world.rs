//! Procedural generation of a default start state (`--start` not given).
//!
//! This builds the Solar-system sandbox fixed by the design spec's 天体表: every
//! one of the 18 listed bodies hosts at least one 定居点 (settlement — on the
//! four gas giants it is an **orbital space station**) whose deposits are the
//! spec's 资源丰富 list, and 定居点 ↔ 城市 are **one-to-one**: each site holds at
//! most one city. Earth therefore holds five named settlements (长三角/珠三角/
//! 亚特兰大/巴黎/莫斯科) each with its own region and ores, one city per site;
//! every other body holds one settlement hosting its labelled 势力's home city.
//! The seed only jitters relations and starting positions, so different seeds
//! still lead to visibly different trajectories.
//!
//! Buildings are continuous-area allocations inside a settlement's finite area
//! (see the model). The default world just seeds a reasonable starting
//! footprint; the simulation expands it over time. All kinds/resources are
//! string keys resolved against the config.

use crate::model::*;
use crate::prng::Prng;
use std::collections::BTreeMap;

// Faction identity = the faction's unique **name** (per the schema). These constants
// are the names themselves so the world is easy to read and stable across seeds.
pub const F_UN: &str = "联合国";
pub const F_US: &str = "美国";
pub const F_EU: &str = "欧盟";
pub const F_CN: &str = "中国";
pub const F_RU: &str = "俄罗斯";
pub const F_MINING: &str = "星系矿业";
pub const F_SCIENCE: &str = "无国界科学组织";
pub const F_TRANSPORT: &str = "深空运输联盟";
pub const F_CULT: &str = "行星X崇拜教";

fn body(
    name: &str,
    semi_major: f64,
    e: f64,
    dir_angle_deg: f64,
    settlements: Vec<Settlement>,
) -> Body {
    build_body(name, semi_major, e, dir_angle_deg, None, settlements)
}

/// A satellite (moon) body: its orbit is centered on `parent` (a planet), and its
/// `semi_major` is measured **from the parent**, not the sun. Its world position
/// is resolved later by [`resolve_positions`] (parent + local offset).
fn satellite_body(
    name: &str,
    semi_major: f64,
    e: f64,
    dir_angle_deg: f64,
    parent: &str,
    settlements: Vec<Settlement>,
) -> Body {
    build_body(name, semi_major, e, dir_angle_deg, Some(parent), settlements)
}

fn build_body(
    name: &str,
    semi_major: f64,
    e: f64,
    dir_angle_deg: f64,
    parent: Option<&str>,
    settlements: Vec<Settlement>,
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
        parent: parent.map(|s| s.to_string()),
    };
    // 占位局部位置；建好全部天体后由 resolve_positions 覆写为世界（日心）坐标。
    let initial_position = orbit.position(0.0);
    Body {
        name: name.to_string(),
        orbit,
        position: initial_position,
        // 类型/星环在 bodies 建好后按名字统一赋值（见 body_kind_for / body_ring_for）。
        kind: "rocky".to_string(),
        ring: false,
        settlements,
    }
}

/// 天体 → `config/game.ron` `body_kinds` 表里的类型 key。state 每个 body 只存这个引用，
/// 类型/视觉数据由 config 提供（见 [`crate::model::Body::kind`] 与 `BodyKindSpec`）。
fn body_kind_for(name: &str) -> &'static str {
    match name {
        "水星" | "灶神星" => "rocky",
        "金星" => "venusian",
        "地球" => "terran",
        "月球" => "lunar",
        "火星" => "martian",
        "木星" | "土星" => "gas_giant",
        "欧罗巴" => "ice_world",
        "泰坦" => "titan",
        "天王星" | "海王星" => "ice_giant",
        "冥王星" | "卡戎" | "伊克西翁" | "妊神星" | "创神星" | "阋神星" => "dwarf",
        _ => "rocky",
    }
}

/// 是否渲染星环（intrinsic body 特征）：土星显著、天王星细环。其余无环。
fn body_ring_for(name: &str) -> bool {
    matches!(name, "土星" | "天王星")
}

fn deposit(rt: &str, area: f64) -> ResourceDeposit {
    ResourceDeposit {
        resource: rt.to_string(),
        area,
    }
}

fn settlement(name: &str, total_area: f64, ecocap: f64, speed: f64, resources: Vec<ResourceDeposit>) -> Settlement {
    Settlement {
        name: name.to_string(),
        total_area,
        ecological_capacity: ecocap,
        construction_speed_mod: speed,
        construction_resource_mod: 1.0,
        resources,
    }
}

/// Build a single building allocation with the district attributes it carries
/// (kind, mined resource, shipyard ship_type, structure) and initial armor.
fn new_building(
    id: BuildingId,
    kind: &str,
    resource: Option<String>,
    ship_type: Option<String>,
    structure: &str,
    area: f64,
    deployed: f64,
    config: &GameConfig,
) -> Building {
    let armor = deployed * config.structure_spec(structure).armor_per_area;
    Building {
        id,
        kind: kind.to_string(),
        resource,
        ship_type,
        structure: structure.to_string(),
        area,
        deployed,
        armor,
    }
}

/// Seed a city's initial building footprint from its settlement's deposits,
/// sized so population is housed and a bit of mining/industry is up and running.
fn seed_buildings(s: &Settlement, population: u32, ship_class: &str, config: &GameConfig, next_id: &mut BuildingId) -> Vec<Building> {
    let mut buildings = Vec::new();
    let mut alloc = |kind: &str, resource: Option<String>, ship_type: Option<String>, area: f64, deployed: f64| -> Building {
        let b = new_building(*next_id, kind, resource, ship_type, "concrete", area, deployed, config);
        *next_id += 1;
        b
    };

    let resid = (population as f64 / s.ecological_capacity).max(4.0);
    buildings.push(alloc("residential", None, None, resid, resid));

    let construction = (s.total_area * 0.15).clamp(3.0, 10.0);
    let mut budget = (s.total_area - resid - construction).max(0.0);
    for d in &s.resources {
        if budget <= 0.0 {
            break;
        }
        let area = d.area.min(budget);
        if area > 0.0 {
            buildings.push(alloc("mining", Some(d.resource.clone()), None, area, area));
            budget -= area;
        }
    }
    buildings.push(alloc("construction", None, Some(ship_class.to_string()), construction, construction));
    buildings
}

/// A city occupying settlement site `site` (1:1) of the body named `body_name`.
fn city(
    name: &str,
    body_name: &str,
    faction: &str,
    population: u32,
    settlement: &Settlement,
    ship_class: &str,
    config: &GameConfig,
    next_id: &mut BuildingId,
) -> City {
    let buildings = seed_buildings(settlement, population, ship_class, config, next_id);
    let mut ship_progress = BTreeMap::new();
    ship_progress.insert(ship_class.to_string(), 0.0);
    City {
        name: name.to_string(),
        body_id: body_name.to_string(),
        settlement: settlement.name.clone(),
        faction_id: faction.to_string(),
        population,
        buildings,
        ship_progress,
        razed: false,
        // 地面/空间站由 default_state 里的天体类型循环覆写（此处先默认地面）。
        space_station: false,
        loyalty: 1.0,
    }
}

fn stockpile(items: &[(&str, f64)]) -> ResourceMap {
    items.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn faction(
    name: &str,
    symbol: char,
    color: &str,
    resources: ResourceMap,
    alignment: f64,
    aggression: f64,
    ideology: Ideology,
) -> Faction {
    let (home_radius, home_attack_mult, home_regen_bonus) = home_for(name);
    Faction {
        name: name.to_string(),
        symbol,
        color: color.to_string(),
        resources,
        relations: std::collections::BTreeMap::new(),
        alignment,
        aggression,
        home_radius,
        home_attack_mult,
        home_regen_bonus,
        ideology,
    }
}

/// 各势力的**初始思潮**（可变化意识形态的起点；之后由 `sim::step_ideology` 按
/// 「变化因素」驱动）。按身份拟定：教团=圣战+人民+反殖民；科学组织=和平+科学+精英+自然；
/// 矿业=军+技术+精英+殖民（采掘）；大国相应偏向。见 `Ideology` 的轴定义。
fn ideology(name: &str) -> Ideology {
    match name {
        F_UN => Ideology { peace_military: -0.4, science_tech: -0.5, people_elite: 0.5, nature_colony: -0.2 },
        F_US => Ideology { peace_military: 0.5, science_tech: 0.5, people_elite: 0.3, nature_colony: 0.6 },
        F_EU => Ideology { peace_military: -0.2, science_tech: 0.1, people_elite: 0.6, nature_colony: -0.3 },
        F_CN => Ideology { peace_military: 0.4, science_tech: 0.7, people_elite: -0.3, nature_colony: 0.5 },
        F_RU => Ideology { peace_military: 0.6, science_tech: 0.4, people_elite: 0.2, nature_colony: 0.4 },
        F_MINING => Ideology { peace_military: 0.3, science_tech: 0.8, people_elite: 0.5, nature_colony: 0.8 },
        F_SCIENCE => Ideology { peace_military: -0.6, science_tech: -0.8, people_elite: 0.2, nature_colony: -0.4 },
        F_TRANSPORT => Ideology { peace_military: -0.2, science_tech: 0.5, people_elite: 0.4, nature_colony: 0.3 },
        F_CULT => Ideology { peace_military: 0.7, science_tech: -0.1, people_elite: -0.4, nature_colony: -0.6 },
        _ => Ideology::default(),
    }
}

/// 各势力的**初始首都**天体（唯一事实来源 = 命令控制 `ControllableState::capital`，
/// 开局在此播种；之后由迁都/迁都亡城逻辑维护）。`Faction` 不存首都可以下——单源无
/// shadow 双状态。
fn initial_capital(name: &str) -> &'static str {
    match name {
        F_UN => "月球",
        F_US => "火星",
        F_EU => "地球",
        F_CN => "地球",
        F_RU => "地球",
        F_MINING => "泰坦",
        F_SCIENCE => "木星",
        F_TRANSPORT => "灶神星",
        F_CULT => "伊克西翁",
        _ => "地球",
    }
}

/// Per-faction 本土防御 (home-field) parameters. The cult's are the MOND anomaly
/// (huge radius + strong damage reduction + strong regen), so the pariah can
/// survive being besieged at its isolated Kuiper-belt sanctuary — it *mastered the
/// correct Newtonian-corrected gravity* (MOND). Everyone else gets a modest core
/// stronghold so conquering near someone's capital costs extra.
fn home_for(name: &str) -> (f64, f64, f64) {
    match name {
        "行星X崇拜教" => (30.0, 0.35, 0.12),
        _ => (6.0, 0.85, 0.02),
    }
}

/// Build the default start state (round 0) for a given seed. The body table
/// (18 bodies, each with one or more named 定居点, spec deposits and a labelled
/// home faction) is fixed by the spec's 天体/城市 configuration; only balance
/// values come from `config`. 定居点 ↔ 城市 一一对应: 22 个定居点、22 座城.
pub fn default_state(config: &GameConfig, seed: u64) -> State {
    let mut rng = Prng::new(seed);

    // --- Bodies -------------------------------------------------------------
    // Ids are in orbital-radius order so the list reads like a real system.
    // 水星/金星/地球/月球/火星/灶神星/木星/欧罗巴/土星/泰坦/天王星/海王星/
    // 冥王星/卡戎/伊克西翁/妊神星/创神星/阋神星  (18 bodies).
    let mut bodies = vec![
        // 水星（中国）资源丰富：铁，铂
        body(
            "水星",
            0.39,
            0.206,
            20.0,
            vec![settlement("水星熔炉基地", 34.0, 6.0, 1.2, vec![deposit("铁", 32.0), deposit("铂", 8.0)])],
        ),
        // 金星（中国）资源丰富：碳
        body(
            "金星",
            0.72,
            0.007,
            95.0,
            vec![settlement("金星浮空之城", 40.0, 8.0, 1.2, vec![deposit("碳", 40.0)])],
        ),
        // 地球/城市 — 五大城市群各自占一个定居点（1:1），矿藏按 spec 各自列出。
        body(
            "地球",
            1.00,
            0.017,
            10.0,
            vec![
                // 长三角（中国）资源丰富：铁，硅，水冰
                settlement(
                    "长三角",
                    120.0,
                    25.0,
                    2.4,
                    vec![deposit("铁", 40.0), deposit("硅", 32.0), deposit("水冰", 36.0)],
                ),
                // 珠三角（中国）资源丰富：铁，硅，水冰
                settlement(
                    "珠三角",
                    120.0,
                    25.0,
                    2.4,
                    vec![deposit("铁", 40.0), deposit("硅", 32.0), deposit("水冰", 36.0)],
                ),
                // 亚特兰大（美国）资源丰富：水冰，碳，金
                settlement(
                    "亚特兰大",
                    100.0,
                    25.0,
                    2.4,
                    vec![deposit("水冰", 40.0), deposit("碳", 30.0), deposit("金", 10.0)],
                ),
                // 巴黎（欧盟）资源丰富：铀，铂
                settlement(
                    "巴黎",
                    90.0,
                    25.0,
                    2.4,
                    vec![deposit("铀", 20.0), deposit("铂", 14.0)],
                ),
                // 莫斯科（俄罗斯）资源丰富：碳，钍
                settlement(
                    "莫斯科",
                    100.0,
                    25.0,
                    2.4,
                    vec![deposit("碳", 40.0), deposit("钍", 16.0)],
                ),
            ],
        ),
        // 月球（联合国）资源丰富：铁，氦-3 —— 地球的卫星，绕地球 0.35 AU。
        satellite_body(
            "月球",
            0.35,
            0.05,
            240.0,
            "地球",
            vec![settlement("宁静海基地", 44.0, 10.0, 1.4, vec![deposit("铁", 28.0), deposit("氦-3", 22.0)])],
        ),
        // 火星（美国）资源丰富：硅，铁，水冰
        body(
            "火星",
            1.52,
            0.093,
            140.0,
            vec![settlement(
                "奥林匹斯港",
                70.0,
                16.0,
                1.8,
                vec![deposit("硅", 30.0), deposit("铁", 30.0), deposit("水冰", 20.0)],
            )],
        ),
        // 灶神星（深空运输联盟）资源丰富：硅，钍
        body(
            "灶神星",
            2.36,
            0.089,
            220.0,
            vec![settlement("灶神星转运港", 30.0, 9.0, 1.3, vec![deposit("硅", 30.0), deposit("钍", 12.0)])],
        ),
        // 木星（无国界科学组织）资源丰富：氢 —— 轨道空间站
        body(
            "木星",
            5.20,
            0.049,
            30.0,
            vec![settlement("木星轨道空间站", 48.0, 12.0, 1.6, vec![deposit("氢", 46.0)])],
        ),
        // 欧罗巴（美国）资源丰富：水冰 —— 木星的卫星，绕木星 0.60 AU。
        satellite_body(
            "欧罗巴",
            0.60,
            0.03,
            140.0,
            "木星",
            vec![settlement("欧罗巴冰下港", 34.0, 9.0, 1.2, vec![deposit("水冰", 40.0)])],
        ),
        // 土星（无国界科学组织）资源丰富：氢，铂，金，水冰（包括星环）—— 轨道空间站
        body(
            "土星",
            9.58,
            0.057,
            70.0,
            vec![settlement(
                "土星轨道空间站",
                56.0,
                12.0,
                1.6,
                vec![
                    deposit("氢", 40.0),
                    deposit("铂", 6.0),
                    deposit("金", 6.0),
                    deposit("水冰", 20.0),
                ],
            )],
        ),
        // 泰坦（星系矿业）资源丰富：硅，铁，铀 —— 土星的卫星，绕土星 0.75 AU。
        satellite_body(
            "泰坦",
            0.75,
            0.03,
            40.0,
            "土星",
            vec![settlement(
                "泰坦采矿城",
                46.0,
                11.0,
                1.4,
                vec![deposit("硅", 28.0), deposit("铁", 24.0), deposit("铀", 14.0)],
            )],
        ),
        // 天王星（欧盟）资源丰富：甲烷，氢 —— 轨道空间站
        body(
            "天王星",
            19.2,
            0.046,
            120.0,
            vec![settlement("天王星轨道站", 40.0, 10.0, 1.5, vec![deposit("甲烷", 32.0), deposit("氢", 20.0)])],
        ),
        // 海王星（欧盟）资源丰富：水冰，甲烷 —— 轨道空间站
        body(
            "海王星",
            30.05,
            0.009,
            200.0,
            vec![settlement("海王星轨道站", 36.0, 9.0, 1.5, vec![deposit("水冰", 30.0), deposit("甲烷", 22.0)])],
        ),
        // 冥王星（俄罗斯）资源丰富：水冰，硅，碳
        body(
            "冥王星",
            39.48,
            0.249,
            40.0,
            vec![settlement(
                "冥王星前哨",
                26.0,
                7.0,
                1.0,
                vec![deposit("水冰", 26.0), deposit("硅", 18.0), deposit("碳", 14.0)],
            )],
        ),
        // 卡戎（俄罗斯）资源丰富：铁，金 —— 冥王星的卫星，绕冥王星 1.20 AU。
        satellite_body(
            "卡戎",
            1.20,
            0.06,
            300.0,
            "冥王星",
            vec![settlement("卡戎深空港", 24.0, 7.0, 1.0, vec![deposit("铁", 24.0), deposit("金", 10.0)])],
        ),
        // 伊克西翁（行星X崇拜教）资源丰富：碳
        body(
            "伊克西翁",
            39.70,
            0.24,
            330.0,
            vec![settlement("伊克西翁圣所", 20.0, 5.0, 0.9, vec![deposit("碳", 24.0)])],
        ),
        // 妊神星（星系矿业）资源丰富：水冰，硅，铂
        body(
            "妊神星",
            43.22,
            0.19,
            250.0,
            vec![settlement(
                "妊神星转运站",
                22.0,
                6.0,
                1.0,
                vec![deposit("水冰", 22.0), deposit("硅", 12.0), deposit("铂", 8.0)],
            )],
        ),
        // 创神星（星系矿业）资源丰富：铁，铂
        body(
            "创神星",
            45.43,
            0.16,
            300.0,
            vec![settlement("创神星采矿站", 22.0, 6.0, 1.0, vec![deposit("铁", 20.0), deposit("铂", 9.0)])],
        ),
        // 阋神星（星系矿业）资源丰富：硅，铀
        body(
            "阋神星",
            67.78,
            0.44,
            160.0,
            vec![settlement("阋神星前哨", 24.0, 5.0, 0.8, vec![deposit("硅", 16.0), deposit("铀", 12.0)])],
        ),
    ];

    // 按名字给每个天体分配类型 key 与星环（视觉/类型引用，见 Body::kind / Body::ring）。
    for b in &mut bodies {
        b.kind = body_kind_for(&b.name).to_string();
        b.ring = body_ring_for(&b.name);
    }

    // 解析每颗天体的**世界（日心）坐标**：卫星（有 parent）的位置 = 母天体世界位置 + 本
    // 轨道局部偏移。父先子后，故单次正序遍历即得正确结果。
    crate::model::resolve_positions(&mut bodies, 0.0);

    // --- Factions -----------------------------------------------------------
    // Idéologie (alignment) & 好战度 (aggression) drive the dynamic international
    // relations: alignment-distance sets each pair's resting affinity (bloc
    // formation), aggression accelerates hostility with ideologically-distant
    // factions. The cult sits far outside the political band so it rests hostile
    // to everyone (a pariah that every conventional power eventually turns on).
    let mut factions = vec![
        faction(F_UN, 'U', "#3b82f6", stockpile(&[("铁", 4.0), ("碳", 4.0), ("氦-3", 2.0)]), 0.4, 0.10, ideology(F_UN)),
        faction(
            F_US,
            'A',
            "#06b6d4",
            stockpile(&[("铁", 6.0), ("碳", 5.0), ("铀", 1.0)]),
            1.0,
            0.60,
            ideology(F_US),
        ),
        faction(F_EU, 'E', "#8b5cf6", stockpile(&[("铁", 5.0), ("碳", 5.0), ("铀", 1.0)]), 0.9, 0.30, ideology(F_EU)),
        faction(
            F_CN,
            'C',
            "#ef4444",
            stockpile(&[("铁", 7.0), ("碳", 6.0), ("硅", 2.0)]),
            -1.0,
            0.50,
            ideology(F_CN),
        ),
        faction(
            F_RU,
            'R',
            "#ec4899",
            stockpile(&[("铁", 5.0), ("碳", 4.0), ("铀", 2.0)]),
            -0.9,
            0.45,
            ideology(F_RU),
        ),
        faction(
            F_MINING,
            'M',
            "#eab308",
            stockpile(&[("铁", 6.0), ("金", 2.0), ("铂", 1.0)]),
            0.0,
            0.20,
            ideology(F_MINING),
        ),
        faction(
            F_SCIENCE,
            'S',
            "#22c55e",
            stockpile(&[("硅", 4.0), ("氦-3", 3.0), ("碳", 2.0)]),
            0.2,
            0.05,
            ideology(F_SCIENCE),
        ),
        faction(
            F_TRANSPORT,
            'T',
            "#f8fafc",
            stockpile(&[("碳", 6.0), ("氢", 3.0), ("铁", 2.0)]),
            -0.1,
            0.15,
            ideology(F_TRANSPORT),
        ),
        faction(
            F_CULT,
            'X',
            "#d946ef",
            stockpile(&[("铀", 3.0), ("钍", 2.0), ("金", 1.0)]),
            -3.0,
            0.90,
            ideology(F_CULT),
        ),
    ];

    // --- Diplomacy: peaceful opening -----------------------------------------
    // No pair starts at war (everything sits above the war threshold); each
    // relation is seeded partway (30%) toward the pair's resting affinity with
    // a little jitter. The dynamic model in sim::step_diplomacy then lets blocs
    // coalesce and rivalries escalate on their own, giving a build-up phase
    // before the first war and letting wars later wind down.
    let set_rel = |x: &mut [Faction], a: usize, b: usize, v: f64| {
        let na = x[a].name.clone();
        let nb = x[b].name.clone();
        x[a].relations.insert(nb, v);
        x[b].relations.insert(na, v);
    };
    let affinity = |align_a: f64, align_b: f64| -> f64 {
        let band = 2.0;
        let d = (align_a - align_b).abs().min(band);
        config.diplomacy.affinity_floor + config.diplomacy.affinity_span * (1.0 - d / band)
    };
    let mut seed_rel = |align_a: f64, align_b: f64| -> f64 {
        let base = affinity(align_a, align_b) * 0.30;
        let jitter = rng.range_f64(-3.0, 3.0);
        (base + jitter).clamp(-15.0, 10.0)
    };
    for i in 0..factions.len() {
        for j in (i + 1)..factions.len() {
            let va = seed_rel(factions[i].alignment, factions[j].alignment);
            set_rel(&mut factions, i, j, va);
        }
    }

    // --- Cities -------------------------------------------------------------
    // 定居点 ↔ 城市 一一对应: 每个城占据其天体上的一个定居点 (settlement 索引)。
    // 地球有 5 个定居点（对应 spec 五大城市群），其余天体各 1 个。
    let earth = &bodies[2];
    let mut next_building_id: BuildingId = 0;
    let mut cities = vec![
        // 地球/城市（spec 命名；长三角/珠三角=中国、亚特兰大=美国、巴黎=欧盟、莫斯科=俄罗斯）
        city("长三角", "地球", F_CN, 1400, &earth.settlements[0], "corvette", config, &mut next_building_id),
        city("珠三角", "地球", F_CN, 1100, &earth.settlements[1], "destroyer", config, &mut next_building_id),
        city("亚特兰大", "地球", F_US, 900, &earth.settlements[2], "destroyer", config, &mut next_building_id),
        city("巴黎", "地球", F_EU, 700, &earth.settlements[3], "cruiser", config, &mut next_building_id),
        city("莫斯科", "地球", F_RU, 800, &earth.settlements[4], "battleship", config, &mut next_building_id),
        // 各族主星（spec 天体归属，定居点 ↔ 城 1:1）
        city("水星熔炉基地", "水星", F_CN, 220, &bodies[0].settlements[0], "corvette", config, &mut next_building_id),
        city("金星浮空之城", "金星", F_CN, 260, &bodies[1].settlements[0], "corvette", config, &mut next_building_id),
        city("宁静海基地", "月球", F_UN, 420, &bodies[3].settlements[0], "corvette", config, &mut next_building_id),
        city("奥林匹斯港", "火星", F_US, 1000, &bodies[4].settlements[0], "cruiser", config, &mut next_building_id),
        city("灶神星转运港", "灶神星", F_TRANSPORT, 280, &bodies[5].settlements[0], "corvette", config, &mut next_building_id),
        city("大红斑科学站", "木星", F_SCIENCE, 520, &bodies[6].settlements[0], "carrier", config, &mut next_building_id),
        city("欧罗巴冰下港", "欧罗巴", F_US, 300, &bodies[7].settlements[0], "corvette", config, &mut next_building_id),
        city("土星环科学站", "土星", F_SCIENCE, 480, &bodies[8].settlements[0], "corvette", config, &mut next_building_id),
        city("泰坦采矿城", "泰坦", F_MINING, 460, &bodies[9].settlements[0], "cruiser", config, &mut next_building_id),
        city("天王星轨道站", "天王星", F_EU, 380, &bodies[10].settlements[0], "cruiser", config, &mut next_building_id),
        city("海王星轨道站", "海王星", F_EU, 340, &bodies[11].settlements[0], "corvette", config, &mut next_building_id),
        city("冥王星前哨", "冥王星", F_RU, 360, &bodies[12].settlements[0], "cruiser", config, &mut next_building_id),
        city("卡戎深空港", "卡戎", F_RU, 260, &bodies[13].settlements[0], "corvette", config, &mut next_building_id),
        city("伊克西翁圣所", "伊克西翁", F_CULT, 200, &bodies[14].settlements[0], "cruiser", config, &mut next_building_id),
        city("妊神星转运站", "妊神星", F_MINING, 260, &bodies[15].settlements[0], "corvette", config, &mut next_building_id),
        city("创神星采矿站", "创神星", F_MINING, 240, &bodies[16].settlements[0], "corvette", config, &mut next_building_id),
        city("阋神星前哨", "阋神星", F_MINING, 220, &bodies[17].settlements[0], "corvette", config, &mut next_building_id),
    ];

    // 城市形态 = 空间站 还是 地面：建在气态/冰巨行星（体积上没有固体表面）上的定居点
    // 是轨道空间站；其余（固体天体表面）是地面城市。供前端按行星相对坐标放模型。
    for c in &mut cities {
        let kind = bodies
            .iter()
            .find(|b| b.name == c.body_id)
            .map(|b| b.kind.as_str())
            .unwrap_or("rocky");
        c.space_station = matches!(kind, "gas_giant" | "ice_giant");
    }

    // --- Starting navy ------------------------------------------------------
    // One ship per entry, id assigned in order; jittered around its home body.
    // Each faction's ships are named deterministically from its 名字库 (name pool) so
    // the ship *name* is a unique, meaningful key (per the design philosophy).
    let mut ships = Vec::new();
    let mut ship_name_seq: BTreeMap<FactionId, u64> = BTreeMap::new();
    let mut add_ship = |ships: &mut Vec<Ship>, faction: &str, body_name: &str, class: &str| {
        // 用已解析的**世界（日心）坐标**作母港位置（卫星的已是「母天体+局部」），开头舰
        // 小扰动即可落在其本天体的真实位置上。
        let pos = bodies
            .iter()
            .find(|b| b.name == body_name)
            .map(|b| b.position)
            .unwrap_or([0.0, 0.0]);
        let spec = config.ship_spec(class);
        let seq = *ship_name_seq.entry(faction.to_string()).or_insert(0);
        ship_name_seq.insert(faction.to_string(), seq + 1);
        let fname = factions
            .iter()
            .find(|f| f.name == faction)
            .map(|f| f.name.clone())
            .unwrap_or_default();
        let name = ship_display_name(config.ship_pool(&fname), seq);
        ships.push(Ship {
            name,
            class: class.to_string(),
            faction_id: faction.to_string(),
            position: [pos[0] + rng.range_f64(-0.05, 0.05), pos[1] + rng.range_f64(-0.05, 0.05)],
            hull: spec.hull,
            hull_max: spec.hull,
            shield: 0.0,
            shield_max: 0.0,
            components: Vec::new(),
            component_hp: Vec::new(),
            velocity: 0.0,
            doctrine: spec.default_doctrine,
            kiting: spec.default_kiting,
            attack_hist: BTreeMap::new(),
        });
    };

    // 中国：长三角/珠三角 + 水星（护卫×2、驱逐×1）
    add_ship(&mut ships, F_CN, "地球", "corvette");
    add_ship(&mut ships, F_CN, "地球", "corvette");
    add_ship(&mut ships, F_CN, "水星", "destroyer");
    // 美国：火星（驱逐×2、巡洋×1）
    add_ship(&mut ships, F_US, "火星", "destroyer");
    add_ship(&mut ships, F_US, "火星", "destroyer");
    add_ship(&mut ships, F_US, "火星", "cruiser");
    // 欧盟：地球/巴黎 + 天王星（巡洋×1、护卫×1）
    add_ship(&mut ships, F_EU, "地球", "cruiser");
    add_ship(&mut ships, F_EU, "天王星", "corvette");
    // 俄罗斯：冥王星（巡洋×1、战列×1）
    add_ship(&mut ships, F_RU, "冥王星", "cruiser");
    add_ship(&mut ships, F_RU, "冥王星", "battleship");
    // 联合国：月球（护卫×2）
    add_ship(&mut ships, F_UN, "月球", "corvette");
    add_ship(&mut ships, F_UN, "月球", "corvette");
    // 星系矿业：泰坦 + 创神星（护卫×1、巡洋×1）
    add_ship(&mut ships, F_MINING, "泰坦", "corvette");
    add_ship(&mut ships, F_MINING, "泰坦", "cruiser");
    // 无国界科学组织：木星（航空母舰×1、护卫×1）
    add_ship(&mut ships, F_SCIENCE, "木星", "carrier");
    add_ship(&mut ships, F_SCIENCE, "木星", "corvette");
    // 深空运输联盟：灶神星（护卫×2）
    add_ship(&mut ships, F_TRANSPORT, "灶神星", "corvette");
    add_ship(&mut ships, F_TRANSPORT, "灶神星", "corvette");
    // 行星X崇拜教：伊克西翁（巡洋×2、护卫×1）
    add_ship(&mut ships, F_CULT, "伊克西翁", "cruiser");
    add_ship(&mut ships, F_CULT, "伊克西翁", "cruiser");
    add_ship(&mut ships, F_CULT, "伊克西翁", "corvette");
    // Release the closure's borrows (factions / ship_name_seq / rng) so the control
    // setup and the State literal can use them.
    drop(add_ship);

    // --- 可控状态 (command-controlled state) --------------------------------
    // Populate each faction's controllable state from the config: per-round
    // investment (建设) and construction (建造) budgets, default per-building
    // invest/build weights, and an idle behavior for every starting ship.
    let mut control: BTreeMap<FactionId, ControllableState> = BTreeMap::new();
    for f in &factions {
        let mut c = ControllableState::default();
        c.investment_budget = f
            .resources
            .iter()
            .map(|(k, v)| (k.clone(), Control::inherit(*v * config.economy.invest_fraction)))
            .collect();
        // 首都（唯一事实来源 = 命令控制 `ControllableState::capital`）：开局按势力
        // 播种其初始首都天体，mode=Inherit（没有说话，沿作用域链上溯，全链无人表态则
        // 落到 Auto）。之后由 sim 的迁都步骤维护，`Faction` 不再存首都可以下
        //（单源无 shadow 双状态）。
        c.capital = Some(Control::inherit(initial_capital(&f.name).to_string()));
        // 建造预算默认与投资预算相等，作为造舰的资金池。
        c.construction_budget = f
            .resources
            .iter()
            .map(|(k, v)| (k.clone(), Control::inherit(*v * config.economy.invest_fraction)))
            .collect();
        control.insert(f.name.clone(), c);
    }
    for city in &cities {
        let c = control.entry(city.faction_id.clone()).or_default();
        for b in &city.buildings {
            let ikey = (city.name.clone(), b.id);
            c.invest_weights
                .insert(ikey, Control::inherit(config.building_spec(&b.kind).default_invest_weight));
            if b.is_shipyard() {
                let bkey = (city.name.clone(), b.id);
                c.build_weights
                    .insert(bkey, Control::inherit(config.building_spec(&b.kind).default_build_weight));
            }
        }
    }
    for s in &ships {
        if let Some(c) = control.get_mut(&s.faction_id) {
            c.ship_orders.insert(s.name.clone(), Control::inherit(ShipBehavior::Idle));
        }
    }

    // Default control scopes: everything AI until the player flips fields.
    let scope = ControlScope::default();

    let mut state = State {
        schema_version: SCHEMA_VERSION,
        round: 0,
        time_month: 0.0,
        bodies,
        cities,
        factions,
        ships,
        control,
        scope,
        events: Vec::new(),
        milestones: Milestones::default(),
        notables: Notables::default(),
        chronicle: Vec::new(),
        ship_name_seq,
        // 市场从空开始：挂单/价格在第一个回合由各势力的当期富余重新挂出（见
        // `sim::step_market`）——世界生成不预置市场，正如开局不预置成交历史。
        market: MarketState::default(),
        // 产地货栈从空开始：开局各城库存都在首都池里（世界生成不预置离岸积压）。
        depots: BTreeMap::new(),
    };

    // --- 开局舰队装配（消灭裸舰）---------------------------------------------
    // 舰级现在是「平台修正器」，攻击力完全来自所装配的武器模块。因此开局预置舰
    // （初始造舰时是建在 State 组装前的裸舰）必须在世界生成后按资源优势补装配组件，
    // 否则它们没有火力。预置舰的组件视为开局已内置（**不扣**库存——否则会掏空第
    // 一回合的经济，而「预置即已装备」在概念上更合理）。
    let fleet: Vec<(String, String, FactionId)> = state
        .ships
        .iter()
        .filter(|s| s.components.is_empty())
        .map(|s| (s.name.clone(), s.class.clone(), s.faction_id.clone()))
        .collect();
    for (sname, class, fid) in fleet {
        let comps = crate::autocontrol::choose_loadout(&state, &config, fid, &class);
        if let Some(s) = state.ships.iter_mut().find(|s| s.name == sname) {
            s.components = comps;
            s.component_hp = s.components.iter().map(|c| component_integrity(&config, c)).collect();
            // 组件可能会加护盾池/硬度/速度，重算并钳制当前值到新上限。
            let panel = ship_panel(&config, s);
            s.hull_max = panel.hull_max;
            s.hull = s.hull.min(panel.hull_max);
            s.shield_max = panel.shield_max;
            s.shield = panel.shield_max;
        }
    }

    state
}
