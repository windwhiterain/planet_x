//! 建筑叶片（含城市建筑校验）的写入。

use super::*;

/// 校验一对 `(city, building)`：两个名字/下标都必须在**同一座城**里对得上。/// `building` 是 u32 下标（在它所属城内部唯一，见 `.agents/notes/name-as-unique-key.md`
/// 的裁决），所以换一座城
/// 就得换下标——这是 agent 手写权重时最容易错的地方。
pub fn check_city_building(
    state: &State,
    fid: &str,
    city: &str,
    building: BuildingId,
    path: &str,
    report: &mut ApplyReport,
) -> bool {
    let Some(c) = state.city(city) else {
        report.skip(
            format!("{path}.city"),
            city,
            "no_such_city",
            format!("没有名为「{city}」的城（城被夷平后名字会从活城列表里消失）。"),
        );
        return false;
    };
    if c.faction_id != fid {
        report.skip(
            format!("{path}.city"),
            city,
            "not_your_city",
            format!("「{city}」属于 {}，不是 {fid} 的城。", c.faction_id),
        );
        return false;
    }
    if !c.buildings.iter().any(|b| b.id == building) {
        let ids: Vec<String> = c.buildings.iter().map(|b| b.id.to_string()).collect();
        report.skip(
            format!("{path}.building"),
            building.to_string(),
            "no_such_building",
            format!("「{city}」里没有 building={building}；它的建筑下标是 [{}]（下标只在城内部唯一，换城要换下标）。", ids.join(", ")),
        );
        return false;
    }
    true
}

/// Apply a single structural building patch: add / remove / modify a building.
///
/// 返回 `true` = 这条补丁确实改到了状态；`false` = 被丢弃（并在 `report` 里留了
/// 一笔）。`buildings` 是整份控制面里**静默失败点最密**的一处（未知 kind / 未知
/// structure / 不是本势力的城 / 下标对不上 / 对非建造区设 ship_type…），所以每个
/// `return` 都换成了带路径与原因的 `skip`。
pub fn apply_building_patch(
    state: &mut State,
    config: &GameConfig,
    fid: FactionId,
    patch: &BuildingPatch,
    path: &str,
    report: &mut ApplyReport,
) -> bool {
    let Some(cid) = patch.city.clone() else {
        report.skip(
            format!("{path}.city"),
            "",
            "missing_city",
            "建筑补丁必须指明 city（城名）。",
        );
        return false;
    };

    if patch.building.is_none() {
        // Add a new building.
        let kind = patch
            .kind
            .clone()
            .unwrap_or_else(|| "residential".to_string());
        let Some(spec) = config.buildings.get(&kind) else {
            let kinds: Vec<&str> = config.buildings.keys().map(String::as_str).collect();
            report.skip(
                format!("{path}.kind"),
                &kind,
                "no_such_kind",
                format!("没有建筑类型「{kind}」（可选：{}）。", kinds.join(" / ")),
            );
            return false;
        };
        // City identity = its unique name; the new building only lands in a city
        // owned by this faction.
        match state.city(&cid) {
            None => {
                report.skip(
                    format!("{path}.city"),
                    &cid,
                    "no_such_city",
                    format!("没有名为「{cid}」的城。"),
                );
                return false;
            }
            Some(c) if c.faction_id != fid => {
                report.skip(
                    format!("{path}.city"),
                    &cid,
                    "not_your_city",
                    format!(
                        "「{cid}」属于 {}，不是 {fid} 的城——新建建筑只能落在自己的城里。",
                        c.faction_id
                    ),
                );
                return false;
            }
            Some(_) => {}
        }
        let structure = patch
            .structure
            .clone()
            .unwrap_or_else(|| "concrete".to_string());
        if !config.structures.contains_key(&structure) {
            let all: Vec<&str> = config.structures.keys().map(String::as_str).collect();
            report.skip(
                format!("{path}.structure"),
                &structure,
                "no_such_structure",
                format!("没有结构「{structure}」（可选：{}）。", all.join(" / ")),
            );
            return false;
        }
        let area = patch.area.unwrap_or(4.0).max(0.0);
        let id = state
            .cities
            .iter()
            .flat_map(|c| c.buildings.iter().map(|b| b.id))
            .max()
            .map_or(0, |m| m + 1);
        let resource = if kind == "mining" {
            patch.resource.clone()
        } else {
            None
        };
        let ship_type = if kind == "construction" {
            patch
                .ship_type
                .clone()
                .or_else(|| Some("corvette".to_string()))
        } else {
            None
        };
        // **设计图指针**（新建建造区可以一步挂上图）：非建造区 ⇒ `not_a_shipyard`；
        // 库里没有 ⇒ `no_such_blueprint`；舰级对不上 ⇒ `blueprint_class_mismatch`。
        let mut blueprint: Option<BlueprintId> = None;
        if let Some(want) = &patch.blueprint {
            if let Some(name) = want {
                if kind != "construction" {
                    report.skip(
                        format!("{path}.blueprint"),
                        name,
                        "not_a_shipyard",
                        format!("新建的是「{kind}」而不是建造区（kind=construction），只有建造区能挂设计图。"),
                    );
                    return false;
                }
                let bp_class = state
                    .control(fid.clone())
                    .and_then(|c| c.blueprints.get(name))
                    .map(|l| l.value.class.clone());
                match bp_class {
                    None => {
                        report.skip(
                            format!("{path}.blueprint"),
                            name,
                            "no_such_blueprint",
                            format!("{fid} 的设计图库里没有「{name}」——新建造区要么不挂图（走 `ship_type` + 生成器），要么挂一张已经存在的图。"),
                        );
                        return false;
                    }
                    Some(bp_class) if ship_type.as_deref() != Some(bp_class.as_str()) => {
                        report.skip(
                            format!("{path}.blueprint"),
                            name,
                            "blueprint_class_mismatch",
                            format!(
                                "「{name}」是 {bp_class} 级的图，而这个新建造区产的是 {}：口径 A 下二者必须相等（同一份补丁里写上 `ship_type`）。",
                                ship_type.clone().unwrap_or_else(|| "（没有舰级）".to_string())
                            ),
                        );
                        return false;
                    }
                    Some(_) => blueprint = Some(name.clone()),
                }
            }
        }
        let b = Building {
            id,
            kind: kind.clone(),
            resource,
            ship_type,
            blueprint,
            structure: structure.clone(),
            area,
            deployed: 0.0,
            armor: 0.0,
        };
        if let Some(city) = state.city_mut(&cid) {
            city.buildings.push(b);
        }
        let ctrl = state.control.entry(fid.clone()).or_default();
        ctrl.invest_weights.insert(
            (cid.clone(), id),
            Control::player(spec.default_invest_weight),
        );
        if kind == "construction" {
            ctrl.build_weights.insert(
                (cid.clone(), id),
                Control::player(spec.default_build_weight),
            );
        }
        return true;
    }

    let bid = patch.building.unwrap_or(u32::MAX);
    if patch.remove {
        let owned = state.city(&cid).map(|c| c.faction_id.as_str()) == Some(fid.as_str());
        if !owned {
            report.skip(
                format!("{path}.city"),
                &cid,
                "not_your_city",
                format!("「{cid}」不是 {fid} 的城（或不存在）——不能删别家的建筑。"),
            );
            return false;
        }
        if state
            .city(&cid)
            .is_some_and(|c| !c.buildings.iter().any(|b| b.id == bid))
        {
            report.skip(
                format!("{path}.building"),
                bid.to_string(),
                "no_such_building",
                format!("「{cid}」里没有 building={bid}，没有可删的东西。"),
            );
            return false;
        }
        if let Some(city) = state.city_mut(&cid) {
            city.buildings.retain(|b| b.id != bid);
        }
        if let Some(c) = state.control_mut(fid.clone()) {
            c.invest_weights.remove(&(cid.clone(), bid));
            c.build_weights.remove(&(cid.clone(), bid));
        }
        return true;
    }

    // Modify an existing building's attributes (e.g. structure / ship_type).
    if state.city(&cid).map(|c| c.faction_id.as_str()) != Some(fid.as_str()) {
        report.skip(
            format!("{path}.city"),
            &cid,
            "not_your_city",
            format!("「{cid}」不是 {fid} 的城（或不存在）。"),
        );
        return false;
    }
    let Some(target) = state
        .city(&cid)
        .and_then(|c| c.buildings.iter().find(|b| b.id == bid))
    else {
        report.skip(
            format!("{path}.building"),
            bid.to_string(),
            "no_such_building",
            format!("「{cid}」里没有 building={bid}（下标只在城内部唯一，换城要换下标）。"),
        );
        return false;
    };
    let is_shipyard = target.is_shipyard();
    let target_kind = target.kind.clone();
    let cur_ship_type = target.ship_type.clone();
    let cur_blueprint = target.blueprint.clone();
    // 口径 A 的反向守卫可能会拒掉 `ship_type` 这一笔（见下）——那时**不许**写进去。
    let mut ship_type_blocked = false;
    // **设计图指针**的校验（写面三层：缺席 = 不动 / `null` = 拆掉指针 / 名字 = 指过去）。
    // 校验先于写入：借不到第二遍状态（下面的可变块已经把城借走了）。
    let mut next_blueprint: Option<Option<BlueprintId>> = None;
    if let Some(want) = &patch.blueprint {
        if !is_shipyard {
            report.skip(
                format!("{path}.blueprint"),
                want.clone().unwrap_or_default(),
                "not_a_shipyard",
                format!("「{cid}」的 building={bid} 不是建造区（kind={target_kind}），只有建造区能挂设计图（图决定这个区把「还不存在的舰」造成什么样）。"),
            );
        } else {
            match want {
                // `null` = 拆掉指针 ⇒ 回到 `ship_type` + `choose_loadout`（旧路径）。
                None => next_blueprint = Some(None),
                Some(name) => {
                    let bp_class = state
                        .control(fid.clone())
                        .and_then(|c| c.blueprints.get(name))
                        .map(|l| l.value.class.clone());
                    match bp_class {
                        None => report.skip(
                            format!("{path}.blueprint"),
                            name,
                            "no_such_blueprint",
                            format!("{fid} 的设计图库里没有「{name}」。**绝不静默回落生成器**：引用不存在的图会让这个建造区**停产**（进度不再增加）——想回到自动选装就写 `\"blueprint\": null`。"),
                        ),
                        // 口径 A：图的 class 必须与该区的 ship_type 相等（同补丁里写了
                        // `ship_type` 就按那个新值比，否则按现值）。
                        Some(bp_class) => {
                            let target_st = patch.ship_type.clone().or_else(|| cur_ship_type.clone());
                            if target_st.as_deref() != Some(bp_class.as_str()) {
                                report.skip(
                                    format!("{path}.blueprint"),
                                    name,
                                    "blueprint_class_mismatch",
                                    format!(
                                        "「{name}」是 {bp_class} 级的图，而这个建造区产的是 {}：口径 A 下二者必须相等（把 `ship_type` 也一起写，或换一张对得上舰级的图）。",
                                        target_st.unwrap_or_else(|| "（没有舰级）".to_string())
                                    ),
                                );
                            } else {
                                next_blueprint = Some(Some(name.clone()));
                            }
                        }
                    }
                }
            }
        }
    }
    // **反方向**的守卫（spec §9.3）：改 `ship_type` 时，若这个区挂着一张 class 对不上的图
    // 而图不在同一份 diff 里跟着改，那就是把玩家的图**间接作废**（图与区对不上 ⇒ 以后
    // 只会报 mismatch）。响亮报出来，并给两条出路。
    if let Some(st) = &patch.ship_type {
        if is_shipyard {
            if let Some(bp) = &cur_blueprint {
                if let Some(bp_class) = state
                    .control(fid.clone())
                    .and_then(|c| c.blueprints.get(bp))
                    .map(|l| l.value.class.clone())
                {
                    // 蓝图表在本份 diff 里已经先落地了：若那边也改了 `class`，这里读到的
                    // 就是**新** class，于是不会误报。
                    if bp_class != *st {
                        ship_type_blocked = true;
                        report.skip(
                            format!("{path}.ship_type"),
                            st,
                            "blueprint_class_mismatch",
                            format!("这个建造区挂着设计图「{bp}」（{bp_class} 级），而你要把它改成 {st} 级。要么同一份 diff 里把图的 `class` 也改成 {st}（`blueprints[].class`），要么先拆掉指针（`\"blueprint\": null`）——留着对不上的图等于把它作废。"),
                        );
                    }
                }
            }
        }
    }
    let mut touched = false;
    if let Some(city) = state.city_mut(&cid) {
        if let Some(b) = city.buildings.iter_mut().find(|b| b.id == bid) {
            if let Some(s) = &patch.structure {
                if config.structures.contains_key(s) {
                    b.structure = s.clone();
                    touched = true;
                } else {
                    let all: Vec<&str> = config.structures.keys().map(String::as_str).collect();
                    report.skip(
                        format!("{path}.structure"),
                        s,
                        "no_such_structure",
                        format!("没有结构「{s}」（可选：{}）。", all.join(" / ")),
                    );
                }
            }
            if let Some(s) = &patch.ship_type {
                if !is_shipyard {
                    report.skip(
                        format!("{path}.ship_type"),
                        s,
                        "not_a_shipyard",
                        format!("「{cid}」的 building={bid} 不是建造区（kind={target_kind}），只有建造区能定 ship_type（决定该区造哪一级舰）。"),
                    );
                } else if !ship_type_blocked {
                    b.ship_type = Some(s.clone());
                    touched = true;
                }
                // `ship_type_blocked` ⇒ 上面那条 `blueprint_class_mismatch` 已经报了，
                // **这一笔不许落地**（否则「响亮报错 + 悄悄改掉」= 最难查的一种）。
            }
            if let Some(k) = &patch.kind {
                if config.buildings.contains_key(k) {
                    b.kind = k.clone();
                    touched = true;
                } else {
                    let kinds: Vec<&str> = config.buildings.keys().map(String::as_str).collect();
                    report.skip(
                        format!("{path}.kind"),
                        k,
                        "no_such_kind",
                        format!("没有建筑类型「{k}」（可选：{}）。", kinds.join(" / ")),
                    );
                }
            }
            if let Some(r) = &patch.resource {
                if b.kind == "mining" {
                    b.resource = Some(r.clone());
                    touched = true;
                } else {
                    report.skip(
                        format!("{path}.resource"),
                        r,
                        "not_a_mining_building",
                        format!("「{cid}」的 building={bid} 不是开采区（kind={}），只有开采区能定开采哪种资源。", b.kind),
                    );
                }
            }
            if let Some(a) = patch.area {
                b.area = a.max(0.0);
                touched = true;
            }
            // 设计图指针（上面已经校验过：建造区 / 图存在 / 舰级对得上）。
            if let Some(next) = &next_blueprint {
                b.blueprint = next.clone();
                touched = true;
            }
        }
    }
    touched
}
