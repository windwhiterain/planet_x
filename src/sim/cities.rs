//! 城市生命周期：立城 / 复垦 / 夷平 / 易主接线 + 殖民地建筑种子。

use super::*;

/// 一座城被**夷平**的方式（决定记哪条事件）。
///
/// `pub(crate)`：投影守卫要用它构造一个确定性的「先夷平、同回合再被别家复垦」样本。
pub enum RazeCause {
    /// 被舰炮拆平 → `CityRazed`（带拆城的舰/势力、伤害、夷平前人口）。
    Bombardment { by_ship: ShipId, by_faction: FactionId, damage: f64 },
    /// 离心叛乱：市民自己散伙，无外部攻击者 → `Revolt`。
    Revolt { faction: FactionId, loyalty: f64 },
}

/// 把一座城夷平为空白（漏斗）：清人口 / 建筑 / 造舰进度、`razed = true`，并记事件。
///
/// 两条路径（舰炮拆平 / 离心叛乱）统一走这里。**刻意保留两条路径各自对忠诚度的效果**
/// （炮击不动 `loyalty`、叛乱清零）：治理步进跳过 razed 城，故该值对模拟是惰性的，但它是
/// 投影 `cities.loyalty` 列的一部分——改它会改变已发布的轨迹。
///
/// `pub(crate)`：投影守卫需要一个**确定性**的「先夷平、同回合再被别家复垦」样本，
/// 否则只能靠长局恰好撞上（而唯一大量产生这种巧合的 `step_resurgence` 已删除）。
pub fn raze_city(state: &mut State, cid: &CityId, cause: RazeCause) {
    let pop_before = state.city(cid).map(|c| c.population).unwrap_or(0);
    // 「谁失去了这座城市」**只有在此刻才知道**：夷平不改 `faction_id`（空白城保留最后主人的
    // diaspora claim），但同一回合后来的 `reseed_city`/`found_city` 会把它改写成新主。
    // 事后再读就只会读到新主（错的人），所以在这里就把它钉进事件。
    let owner = state.city(cid).map(|c| c.faction_id.clone()).unwrap_or_default();
    if let Some(c) = state.city_mut(cid) {
        c.razed = true;
        c.population = 0;
        c.buildings.clear();
        c.ship_progress.clear();
        if matches!(&cause, RazeCause::Revolt { .. }) {
            c.loyalty = 0.0;
        }
    }
    match cause {
        RazeCause::Bombardment { by_ship, by_faction, damage } => ev(state, GameEvent::CityRazed {
            city: cid.clone(),
            owner,
            fallen_to: by_faction,
            by_ship,
            damage,
            pop_before,
        }),
        RazeCause::Revolt { faction, loyalty } => ev(state, GameEvent::Revolt {
            city: cid.clone(),
            faction,
            loyalty,
        }),
    }
}

/// 给一座城的新建筑补上「投资 / 建造权重」控制叶子。
///
/// `Control::inherit` 的 `mode = Inherit` → 控制解析沿作用域链上溯，与「叶子不存在」等价，
/// 因此这一步**不改变任何决策**（只是让控制面里那座城的建筑是可枚举的）。
pub fn wire_city_control(state: &mut State, config: &GameConfig, cid: &CityId, to: &FactionId) {
    let buildings = state.city(cid).map(|c| c.buildings.clone()).unwrap_or_default();
    let ctrl = state.control.entry(to.clone()).or_default();
    for b in &buildings {
        let ikey = (cid.clone(), b.id);
        ctrl.invest_weights.entry(ikey).or_insert_with(|| {
            Control::inherit(config.building_spec(&b.kind).default_invest_weight)
        });
        if b.is_shipyard() {
            let bkey = (cid.clone(), b.id);
            ctrl.build_weights.entry(bkey).or_insert_with(|| {
                Control::inherit(config.building_spec(&b.kind).default_build_weight)
            });
        }
    }
}

/// 复垦一座**空白城**（razed → 活城）并交给 `to`（漏斗）：重新播种建筑 / 人口 / 造舰进度、
/// 忠诚重置为 1.0，并记 `ColonyFounded { how: Refounded, prev_owner }`。
///
/// `prev_owner` 由漏斗自己读（改归属**之前**的持有者 = 空白城保留的 diaspora claim），
/// 所以「谁失去了这座城市」不可能被调用方漏掉。
///
/// 由**殖民舰抵达**触发（见 `colonize`）——这是唯一能让一座空白城重新立起来的路径：
/// 重建必须**有船跑到那里**，不再是凭空变城。
/// 返回 `false` = 该城没有可用的定居点（调用方自行处理）。
pub fn reseed_city(
    state: &mut State,
    config: &GameConfig,
    cid: &CityId,
    to: &FactionId,
    seeded_ship_class: &str,
    next_building_id: &mut BuildingId,
) -> bool {
    let Some(settlement) = state.city_settlement(cid).cloned() else { return false };
    let Some(body) = state.city(cid).map(|c| c.body_id.clone()) else { return false };
    let prev_owner = state.city(cid).map(|c| c.faction_id.clone());
    let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
    let buildings = seed_colony_buildings(&settlement, pop, seeded_ship_class, config, next_building_id);
    if let Some(c) = state.city_mut(cid) {
        c.razed = false;
        c.faction_id = to.clone();
        c.population = pop;
        c.buildings = buildings;
        c.ship_progress.clear();
        c.ship_progress.insert(seeded_ship_class.to_string(), 0.0);
        c.loyalty = 1.0;
    }
    wire_city_control(state, config, cid, to);
    ev(state, GameEvent::ColonyFounded {
        city: cid.clone(),
        owner: to.clone(),
        body,
        seeded_ship_class: seeded_ship_class.to_string(),
        how: FoundingHow::Refounded,
        prev_owner,
    });
    true
}

/// 在**从未被占据**的定居点上新建一座城（漏斗）并记 `ColonyFounded { how: NewSite }`。
/// 城名由调用方给出（殖民城 / 收容所两种命名不同）。返回 `false` = 同名城已存在（防御）。
pub fn found_city(
    state: &mut State,
    config: &GameConfig,
    name: &CityId,
    body: &BodyId,
    settlement: &Settlement,
    to: &FactionId,
    seeded_ship_class: &str,
    next_building_id: &mut BuildingId,
) -> bool {
    if state.city(name).is_some() {
        return false;
    }
    let pop = (settlement.ecological_capacity * 20.0).round().max(40.0) as u32;
    let buildings = seed_colony_buildings(settlement, pop, seeded_ship_class, config, next_building_id);
    let mut progress: BTreeMap<String, f64> = BTreeMap::new();
    progress.insert(seeded_ship_class.to_string(), 0.0);
    state.cities.push(City {
        name: name.clone(),
        body_id: body.clone(),
        settlement: settlement.name.clone(),
        faction_id: to.clone(),
        population: pop,
        buildings,
        ship_progress: progress,
        razed: false,
        space_station: false,
        loyalty: 1.0,
    });
    wire_city_control(state, config, name, to);
    ev(state, GameEvent::ColonyFounded {
        city: name.clone(),
        owner: to.clone(),
        body: body.clone(),
        seeded_ship_class: seeded_ship_class.to_string(),
        how: FoundingHow::NewSite,
        prev_owner: None,
    });
    true
}

/// A building's health ratio (armor / armor_max), clamped to [0, 1]. Intact
/// buildings are 1.0; damaged buildings produce/operate at a reduced ratio.
pub fn building_health(b: &Building, config: &GameConfig) -> f64 {
    let amax = b.armor_max(config);
    if amax <= 1e-9 {
        1.0
    } else {
        (b.armor / amax).clamp(0.0, 1.0)
    }
}

/// Does `body` have a colonizable 定居点 site? Yes iff it hosts a settlement
/// whose city is razed (blank footprint, re-seedable) or a settlement no city
/// occupies yet. Settlement ↔ city is 1:1, so a site with a live city never
/// counts as blank.
pub fn has_blank_site(state: &State, body: &str) -> bool {
    let Some(b) = state.body(body) else { return false };
    if b.settlements.is_empty() {
        return false;
    }
    let has_razed = state.cities.iter().any(|c| c.body_id == body && c.razed);
    if has_razed {
        return true;
    }
    let occupied: BTreeSet<String> = state
        .cities
        .iter()
        .filter(|c| c.body_id == body)
        .map(|c| c.settlement.clone())
        .collect();
    b.settlements.iter().any(|s| !occupied.contains(&s.name))
}


/// Seed buildings for a newly founded / razed-and-reseeded city.
pub fn seed_colony_buildings(
    s: &Settlement,
    population: u32,
    ship_class: &str,
    config: &GameConfig,
    next_id: &mut BuildingId,
) -> Vec<Building> {
    let mut buildings = Vec::new();
    let mut alloc = |kind: &str, resource: Option<String>, ship_type: Option<String>, area: f64, deployed: f64| -> Building {
        let id = *next_id;
        *next_id += 1;
        let armor = deployed * config.structure_spec("concrete").armor_per_area;
        Building {
            id,
            kind: kind.to_string(),
            resource,
            ship_type,
            blueprint: None,
            structure: "concrete".to_string(),
            area,
            deployed,
            armor,
        }
    };

    let footprint = s.total_area * config.combat.colony_footprint;
    let resid = (population as f64 / s.ecological_capacity.max(1e-6)).min(footprint * 0.5).max(4.0);
    buildings.push(alloc("residential", None, None, resid, resid));
    let mut budget = (footprint - resid).max(0.0);
    for d in &s.resources {
        if budget <= 0.0 {
            break;
        }
        let area = d.area.min(budget * 0.5);
        if area > 0.0 {
            buildings.push(alloc("mining", Some(d.resource.clone()), None, area, area));
            budget -= area;
        }
    }
    let construction = (footprint * 0.3).clamp(2.0, 8.0);
    buildings.push(alloc("construction", None, Some(ship_class.to_string()), construction, construction));
    buildings
}
