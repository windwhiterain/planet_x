//! 建造：投资与建造权重 → 开工结算、造舰、设计图下水门控。

use super::*;

pub fn step_construction(
    state: &mut State,
    config: &GameConfig,
    rng: &mut Prng,
    flow: &mut RoundSink,
) {
    let faction_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let mut next_building_id = state
        .cities
        .iter()
        .flat_map(|c| c.buildings.iter().map(|b| b.id))
        .max()
        .map_or(0, |m| m + 1);

    for fid in faction_ids {
        let (investment, inv_modes) = autocontrol::read_budget(
            state,
            config,
            fid.clone(),
            autocontrol::BudgetKind::Investment,
        );
        let (construction, con_modes) = autocontrol::read_budget(
            state,
            config,
            fid.clone(),
            autocontrol::BudgetKind::Construction,
        );
        autocontrol::write_budget(
            state,
            fid.clone(),
            autocontrol::BudgetKind::Investment,
            &investment,
            &inv_modes,
        );
        autocontrol::write_budget(
            state,
            fid.clone(),
            autocontrol::BudgetKind::Construction,
            &construction,
            &con_modes,
        );

        let mut inv_spent: ResourceMap = ResourceMap::new();
        let mut con_spent: ResourceMap = ResourceMap::new();

        let city_ids: Vec<CityId> = state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid)
            .map(|c| c.name.clone())
            .collect();
        for cid in city_ids {
            build_city(
                state,
                config,
                cid,
                fid.clone(),
                &investment,
                &construction,
                &mut inv_spent,
                &mut con_spent,
                &mut next_building_id,
                rng,
                flow,
            );
        }
        // 记录本回合**实际花掉的**投资/建造预算（B2 的中间量，写完即弃的局部变量）。
        // 批了多少不在这里——限额是控制面的持久叶（`investment_budget`/`construction_budget`，
        // 上面 `write_budget` 刚写回当回合用的额度），两者相减 = 「批了却没花掉的那部分」。
        flow.spend.insert(
            fid.clone(),
            crate::model::SpendFlow {
                investment: inv_spent,
                construction: con_spent,
            },
        );
    }
    // 威胁响应（整支舰队随威胁重构）：战时把过度生产的「轻舰」船坞按战况重定向到更重/更
    // 需要的舰型，让威胁响应不只作用于新建舰厂。确定性（seeded RNG）。
    let retool_ids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    for fid in retool_ids {
        // **两条动机、两条通道**（用户裁决：造货船的动机与造战争船的动机解耦）：
        // 战时重构先跑（行为与旧版逐字节相同），集货侧重构只认**它剩下的**船坞
        //（`claimed`）——于是「有威胁」不会把「缺运力」淹没掉。
        let before = flow.decisions.retools.len();
        autocontrol::retool_shipyards(state, config, &fid, rng, &mut flow.decisions.retools);
        let claimed: std::collections::BTreeSet<(CityId, BuildingId)> = flow.decisions.retools
            [before..]
            .iter()
            .map(|r| (r.city.clone(), r.building))
            .collect();
        autocontrol::retool_haulers(state, config, &fid, &claimed, &mut flow.decisions.retools);
    }
    // **设计图的执行者**（`Auto` 图那一层的真写入者）：AI 自己建图 / 重估选装 / 去重复用 /
    // 回收没人指向的自建图。放在 `retool_shipyards` **之后**：舰级重估刚刚落定，这一趟就能把
    // 「图与建造区对得上」顺手收敛（retool 改了舰级 ⇒ 图的名字与舰级跟着换）。
    autocontrol::design_fleets(state, config, &mut flow.decisions.blueprints);
}

#[allow(clippy::too_many_arguments)]
pub fn build_city(
    state: &mut State,
    config: &GameConfig,
    cid: CityId,
    fid: FactionId,
    invest_limit: &ResourceMap,
    con_limit: &ResourceMap,
    inv_spent: &mut ResourceMap,
    con_spent: &mut ResourceMap,
    next_building_id: &mut BuildingId,
    _rng: &mut Prng,
    flow: &mut RoundSink,
) {
    if state.city(&cid).map(|c| c.razed).unwrap_or(true) {
        return;
    }
    let (ecocap, total_area, speed_mod, res_mod, deposits) = {
        // 城市的定居点 = 它自己占据的那一个（1:1），面积/矿藏以该定居点为准。
        let s = state.city_settlement(&cid);
        match s {
            Some(s) => (
                s.ecological_capacity,
                s.total_area,
                s.construction_speed_mod,
                s.construction_resource_mod,
                s.resources
                    .iter()
                    .map(|d| (d.resource.clone(), d.area))
                    .collect::<Vec<_>>(),
            ),
            None => return,
        }
    };

    let mut buildings: Vec<Building> = state.city(&cid).expect("city gone").buildings.clone();
    let population = state.city(&cid).map(|c| c.population as f64).unwrap_or(0.0);
    let labor = labor_ratio(state, config, &cid);
    // 这座城市**所在的天体**：本回合一切实物消耗（建楼 / 造舰 / 装模块）都只从
    // [`State::stock_at`] 提货——首都天体是池子，其余天体是本地货栈。
    let body_id = state
        .city(&cid)
        .map(|c| c.body_id.clone())
        .unwrap_or_default();

    pub fn new_b(
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
            // 测试夹具不挂设计图：走 `ship_type` + `choose_loadout`（与无图的正式路径同形）。
            blueprint: None,
            structure: structure.to_string(),
            area,
            deployed,
            armor,
        }
    }

    pub fn find_area(buildings: &[Building], kind: &str, resource: Option<&str>) -> f64 {
        buildings
            .iter()
            .filter(|b| b.kind == kind)
            .find(|b| match (resource, b.resource.as_deref()) {
                (Some(r), Some(br)) => r == br,
                (None, None) => true,
                _ => false,
            })
            .map(|b| b.area)
            .unwrap_or(0.0)
    }

    pub fn raise_area(
        buildings: &mut Vec<Building>,
        config: &GameConfig,
        next_id: &mut BuildingId,
        kind: &str,
        resource: Option<&str>,
        add: f64,
    ) {
        if add <= 1e-9 {
            return;
        }
        let idx = buildings.iter().position(|b| {
            b.kind == kind
                && match (resource, b.resource.as_deref()) {
                    (Some(r), Some(br)) => r == br,
                    (None, None) => true,
                    _ => false,
                }
        });
        match idx {
            Some(i) => buildings[i].area += add,
            None => {
                buildings.push(new_b(
                    *next_id,
                    kind,
                    resource.map(str::to_string),
                    None,
                    "concrete",
                    add,
                    0.0,
                    config,
                ));
                *next_id += 1;
            }
        }
    }

    // 1) Plan new area (raising targets), bounded by total_area.
    let mut planning_remaining = total_area - buildings.iter().map(|b| b.area).sum::<f64>();

    let desired_res = (population / ecocap.max(1e-6)) * config.economy.housing_buffer;
    let res_area = find_area(&buildings, "residential", None);
    if res_area < desired_res - 1e-9 && planning_remaining > 0.0 {
        let add = (desired_res - res_area).min(planning_remaining);
        raise_area(
            &mut buildings,
            config,
            next_building_id,
            "residential",
            None,
            add,
        );
        planning_remaining -= add;
    }

    // Grow a 建造区 (shipyard) toward its target. If a city holds several
    // shipyards, grow the one with the largest current footprint.
    let con_target = (total_area * 0.15).clamp(4.0, 12.0);
    let shipyard_index = buildings
        .iter()
        .enumerate()
        .filter(|(_, b)| b.is_shipyard())
        .max_by(|(_, a), (_, b)| a.deployed.total_cmp(&b.deployed))
        .map(|(i, _)| i);
    let con_area = shipyard_index.map(|i| buildings[i].area).unwrap_or(0.0);
    if con_area < con_target - 1e-9 && planning_remaining > 0.0 {
        let add = (con_target - con_area).min(planning_remaining);
        let idx = shipyard_index;
        match idx {
            Some(i) => buildings[i].area += add,
            None => buildings.push(new_b(
                *next_building_id,
                "construction",
                None,
                None,
                "concrete",
                add,
                0.0,
                config,
            )),
        }
        if idx.is_none() {
            *next_building_id += 1;
        }
        planning_remaining -= add;
    }

    for (rt, darea) in &deposits {
        if planning_remaining <= 0.0 {
            break;
        }
        let cur = find_area(&buildings, "mining", Some(rt));
        if cur < *darea - 1e-9 {
            let add = (*darea - cur).min(planning_remaining);
            raise_area(
                &mut buildings,
                config,
                next_building_id,
                "mining",
                Some(rt),
                add,
            );
            planning_remaining -= add;
        }
    }

    // 2) Build: grow deployed toward the planned area, spending the investment
    // budget. Higher invest weight builds first.
    buildings.sort_by(|a, b| {
        invest_weight(state, config, &fid, &cid, b)
            .total_cmp(&invest_weight(state, config, &fid, &cid, a))
    });
    for b in buildings.iter_mut() {
        if !b.under_construction() {
            continue;
        }
        let spec = config.building_spec(&b.kind);
        let per_area = per_area_cost(config, spec, res_mod, b);
        let speed = spec.construction_speed * speed_mod * spec.productivity * labor;
        let desired = (b.area - b.deployed).min(speed);
        // **两个上限缺一不可**：预算是「这个月愿意花多少」（势力级速率），
        // 站点库存是「这座城市手上有没有这些货」（**首都 ⇒ 池子、其余 ⇒ 产地货栈**）。
        // 后者就是「完全禁止瞬移」的落点：非首都天体上没有的货，只能靠船运过去。
        let inc = max_affordable_inc(&per_area, invest_limit, inv_spent, desired);
        let inc = site_affordable(state, &fid, &body_id, &per_area, inc);
        if inc <= 1e-6 {
            continue;
        }
        let cost: Vec<(String, f64)> = per_area
            .iter()
            .map(|(rt, c)| (rt.clone(), *c * inc))
            .collect();
        commit_spend(state, &fid, &body_id, inv_spent, &cost);
        b.deployed += inc;
    }

    // Regrow armor: freshly-built area is intact; otherwise repair toward max at
    // the configured regen rate.
    for b in buildings.iter_mut() {
        let amax = b.armor_max(config);
        if b.under_construction() {
            b.armor = amax;
        } else {
            b.armor = (b.armor + (amax - b.armor) * config.combat.armor_regen)
                .min(amax)
                .max(0.0);
        }
    }

    // 3) Ship building: each 建造区 contributes to its class's rate; progress is
    // per city. The construction budget funds completed ships; shipyards compete
    // for it by build weight, so a higher-weight shipyard pays for its ship first.
    let mut shipyards: Vec<(usize, String, f64, f64)> = Vec::new(); // (index, class, weight, area)
    for (i, b) in buildings.iter().enumerate() {
        if b.is_shipyard() {
            if let Some(cls) = b.ship_type.clone() {
                let area = b.deployed;
                if area > 1e-9 {
                    // **悬空图指针 ⇒ 这个建造区停产**（用户裁决 Q10(a)）：指针指向的图不在
                    // 本势力的设计图库里（玩家改名/删图了）时，这个区**不再贡献产出速率**
                    // ⇒ 进度不再增加。它不会被静默换成「按 ship_type 生成」（那是
                    // 「失败看起来像成功」），也不会被删掉——读面照旧把指针原样输出，谁能
                    // 一眼看出「这个区指着一张不存在的图」。
                    if let Some(bp) = b.blueprint.as_ref() {
                        let known = state
                            .control(fid.clone())
                            .map(|c| c.blueprints.contains_key(bp))
                            .unwrap_or(false);
                        if !known {
                            continue;
                        }
                    }
                    shipyards.push((i, cls, build_weight(state, config, &fid, &cid, b), area));
                }
            }
        }
    }
    shipyards.sort_by(|a, b| b.2.total_cmp(&a.2));

    let city_progress: BTreeMap<String, f64> = state
        .city(&cid)
        .map(|c| c.ship_progress.clone())
        .unwrap_or_default();

    // Aggregate per-class production rate (all 建造区 of a class add up toward the
    // city pool) and per-class build priority (max of its shipyards' weights).
    let mut class_rate: BTreeMap<String, f64> = BTreeMap::new();
    let mut class_weight: BTreeMap<String, f64> = BTreeMap::new();
    for (_, cls, w, area) in &shipyards {
        *class_rate.entry(cls.clone()).or_insert(0.0) +=
            area * config.building_spec("construction").productivity * labor;
        let e = class_weight.entry(cls.clone()).or_insert(0.0);
        *e = e.max(*w);
    }
    let mut classes: Vec<(String, f64, f64)> = class_rate
        .iter()
        .map(|(c, r)| (c.clone(), *r, class_weight.get(c).copied().unwrap_or(0.0)))
        .collect();
    classes.sort_by(|a, b| b.2.total_cmp(&a.2));

    // 每个舰级的**出厂图**：取该舰级在本城里**排序最靠前**的那个建造区的图（排序见上：
    // 建造权重高的在前、同权重按建筑下标）。进度池是按**舰级**合并的（`City.ship_progress`），
    // 所以「同城两张同舰级的设计图共用进度池」是既有语义（口径 A，spec §2.6/§9）；
    // 「谁先攒够谁先下水」在这里落成一句明确的话：**按建造区顺序，谁权重高就用谁的图**。
    // 没有这张表时（无图/旧档）下面一切照旧。
    let mut class_blueprint: BTreeMap<String, Option<BlueprintId>> = BTreeMap::new();
    for (i, cls, _, _) in &shipyards {
        class_blueprint
            .entry(cls.clone())
            .or_insert_with(|| buildings.get(*i).and_then(|b| b.blueprint.clone()));
    }

    // The construction budget is a per-round rate: it funds ship progress
    // incrementally (cost-per-progress × increment). A class completes a ship
    // once it has accrued `build_points`, at the city level.
    let body_pos = state.body_position(&body_id);
    let mut to_write_progress = city_progress;
    for (cls, rate, _) in &classes {
        let spec = config.ship_spec(cls);
        let bp = spec.build_points;
        let launch_blueprint = class_blueprint.get(cls).cloned().flatten();
        let per_progress: Vec<(String, f64)> = spec
            .build_cost
            .iter()
            .map(|(rt, c)| (rt.clone(), c / bp))
            .collect();
        // 造舰与建楼同一条纪律：预算速率 × **本站点库存**（船坞在哪颗星，钢材就得在哪颗星）。
        let increment = max_affordable_inc(&per_progress, con_limit, con_spent, *rate).max(0.0);
        let increment = site_affordable(state, &fid, &body_id, &per_progress, increment);
        // 记本回合这一舰级的**产能速率上限**与**实得进度**（B2 的中间量）——「造舰慢是缺钱还是缺
        // 产能」的唯一入口。⚠ **`increment = 0` 也要占位**（本城有建造区、速率摆在那儿、却一分钱
        // 没批到 = 预算被别人吃光了）；只有「本城根本没这个舰级的建造区」才没有行。
        //
        // ⚠ **记的是「最终实得」**：`site_affordable` 又按**本站点库存**卡了一道（「完全禁止瞬移」
        // 之后船坞缺钢材是常见的第三种「造不动」），所以这一笔必须放在它**之后**——否则读面上
        // 写着「进度涨了」而实体没有（`flow` 的纪律：中间量是**观测**，得与实体对得上）。
        flow.city_flow.entry(cid.clone()).or_default().build.insert(
            cls.clone(),
            crate::model::BuildLine {
                rate: *rate,
                increment,
            },
        );
        if increment <= 1e-9 {
            continue;
        }
        let cost: Vec<(String, f64)> = per_progress
            .iter()
            .map(|(rt, c)| (rt.clone(), *c * increment))
            .collect();
        commit_spend(state, &fid, &body_id, con_spent, &cost);
        *to_write_progress.entry(cls.clone()).or_insert(0.0) += increment;
        // Spawn ships as their build points fill (the cost was paid as progress).
        // On launch, the ship is fitted with a deterministic component loadout chosen
        // from the faction's resource advantage (see `choose_loadout`); the component
        // cost is paid out of the stockpile and the effective panel (hull_max, etc.)
        // is computed from class + components. All of that (naming, fitting, paying,
        // recording `ShipSpawned`) lives in the `spawn_ship` funnel.
        //
        // **设计图**改的只有「装什么」：建造区挂了图且那张图归属解析为 `Player` 时按图装配
        // （否则 `resolve_loadout` 走生成器，与无图逐字节相同）。
        while to_write_progress.get(cls).copied().unwrap_or(0.0) >= bp - 1e-9 {
            // **买不起就不下水**（用户裁决 Q4(b)，**只对 `Player` 归属的图生效**）：
            // 今天 `commit_spend` 把库存钳到 0、绝不拒绝 ⇒ 玩家能凭空造出超预算舰队。
            // 这里改成「进度继续攒、下回合再试」（确定、诚实），并留下**可见标记**
            // （投影蓝图表 `launch_waiting` 列：这座城市这个舰级的进度已经满了却没下水）。
            // `Auto` 图与无图**保持旧行为**（生成器自己保证买得起），所以长局基线不必重标。
            if let Some(id) = launch_blueprint.as_ref() {
                if blueprint_launch_blocked(state, config, &fid, id, cls, &body_id) {
                    break;
                }
            }
            spawn_ship(
                state,
                config,
                ShipSpawn {
                    owner: fid.clone(),
                    class: cls.as_str(),
                    position: [body_pos[0] + 0.05, body_pos[1] + 0.05],
                    city: Some(cid.clone()),
                    via: SpawnVia::Shipyard,
                    pay_components: true,
                    blueprint: launch_blueprint.as_ref(),
                },
            );
            *to_write_progress.entry(cls.clone()).or_insert(0.0) -= bp;
        }
    }
    if let Some(city) = state.city_mut(&cid) {
        city.ship_progress = to_write_progress;
    }

    // 4) Write back, and ensure every building has invest/build-weight entries.
    if let Some(c) = state.control_mut(fid.clone()) {
        for b in &buildings {
            let key = (cid.clone(), b.id);
            c.invest_weights.entry(key.clone()).or_insert_with(|| {
                Control::inherit(config.building_spec(&b.kind).default_invest_weight)
            });
            if b.is_shipyard() {
                c.build_weights.entry(key).or_insert_with(|| {
                    Control::inherit(config.building_spec(&b.kind).default_build_weight)
                });
            }
        }
    }
    if let Some(city) = state.city_mut(&cid) {
        city.buildings = buildings;
    }
}

/// 这个建造区的图**此刻**是否让下水停摆（用户裁决 Q4(b)：玩家写的图买不起就不下水、
/// 进度继续攒）。
///
/// **只对 `Player` 归属的图生效**：`Auto` 图保持旧行为（生成器的选装本来就是按**站点**
/// 库存算的，见 `choose_loadout`），所以长局基线不必重标。
///
/// `components` 为空 = 「交给生成器」⇒ 同样按旧行为。
/// **判据读的是 `body` 那处的库存**（[`State::stock_at`]）：首都天体是池子、其余是本地货栈
/// ——船坞在哪儿，模块的钱就在哪儿付。
pub fn blueprint_launch_blocked(
    state: &State,
    config: &GameConfig,
    fid: &str,
    id: &BlueprintId,
    class: &str,
    body: &str,
) -> bool {
    if !state.blueprint_control(&fid.to_string(), id).is_player() {
        return false;
    }
    let Some(leaf) = state
        .control(fid.to_string())
        .and_then(|c| c.blueprints.get(id))
    else {
        return false;
    };
    // 口径 A：图的舰级必须与建造区一致（apply 已守卫，这里再判一次是防手写的 .ron）。
    if leaf.value.class != class {
        return false;
    }
    if leaf.value.components.is_empty() {
        return false;
    }
    let mut need: ResourceMap = ResourceMap::new();
    for c in &leaf.value.components {
        // 未知组件 ⇒ 这艘舰装不出来 ⇒ 别下水（响亮地卡住，而不是装成一艘裸舰）。
        let Some(cs) = config.components.get(c) else {
            return true;
        };
        for (r, amt) in &cs.cost {
            *need.entry(r.clone()).or_insert(0.0) += *amt;
        }
    }
    let stock = state.stock_at(fid, body);
    need.iter()
        .any(|(r, amt)| stock.and_then(|m| m.get(r)).copied().unwrap_or(0.0) < *amt - 1e-9)
}

/// 这张图此刻是否**在等钱**：挂了它的建造区所在城里，这个舰级的进度已经攒够
/// `build_points`（`≥`）却**没有下水**。
///
/// 这是 Q4(b) 要求的**可见标记**，而且它是**状态的可观察后果**、不是新状态：出厂循环
/// 只要正常跑完，进度一定 `< build_points`（每下水一艘就减一次）；所以「回合末进度
/// 仍然 ≥ build_points」⟺ 这一回合有下水被卡住了。投影的蓝图表把它摊成 `launch_waiting`
/// 列，免得「进度满了却不出舰」被当成 bug。
pub fn blueprint_launch_waiting(
    state: &State,
    config: &GameConfig,
    fid: &str,
    id: &BlueprintId,
) -> bool {
    let Some(leaf) = state
        .control(fid.to_string())
        .and_then(|c| c.blueprints.get(id))
    else {
        return false;
    };
    let class = &leaf.value.class;
    let Some(spec) = config.ships.get(class) else {
        return false;
    };
    state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        .any(|city| {
            let carries = city
                .buildings
                .iter()
                .any(|b| b.is_shipyard() && b.blueprint.as_deref() == Some(id.as_str()));
            carries
                && city.ship_progress.get(class).copied().unwrap_or(0.0) >= spec.build_points - 1e-9
        })
}

// --- military ---------------------------------------------------------------
