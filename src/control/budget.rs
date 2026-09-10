//! 预算与权重叶（budget / invest / build / loyalty）的写入。

use super::*;

/// 资源预算补丁（投资 / 建造共用）：`value` 替换预算额、`mode` 指定由谁决定、`remove` 删叶。
pub fn apply_budget(
    state: &mut State,
    config: &GameConfig,
    fid: &FactionId,
    which: BudgetKind,
    bp: &BudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let name = which.name();
    let path = format!("{fid}.{name}[{i}].resource");
    let mut present = Vec::new();
    if bp.value.is_some() {
        present.push("value");
    }
    if bp.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(bp.remove, &present, &path, report) {
        return;
    }
    // 删叶先于「资源是否存在」的检查：config 里已经没有的 key 也能删（清理陈叶）。
    if bp.remove {
        let c = state.control.entry(fid.clone()).or_default();
        let map = match which {
            BudgetKind::Investment => &mut c.investment_budget,
            BudgetKind::Construction => &mut c.construction_budget,
        };
        let existed = map.remove(&bp.resource).is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if !config.resources.contains_key(&bp.resource) {
        report.skip(
            path,
            &bp.resource,
            "no_such_resource",
            format!("没有资源 key「{}」（WYSIWYG：状态里的 key 就是 diff 里的 key，用 --meta 的 resources 看全表）。", bp.resource),
        );
        return;
    }
    let c = state.control.entry(fid.clone()).or_default();
    let map = match which {
        BudgetKind::Investment => &mut c.investment_budget,
        BudgetKind::Construction => &mut c.construction_budget,
    };
    let ctrl = map.entry(bp.resource.clone()).or_insert_with(|| Control {
        value: bp.value.unwrap_or(0.0),
        mode: bp.mode.unwrap_or_default(),
    });
    write_value_leaf(
        ctrl,
        bp.value,
        bp.mode,
        format!("{fid}.{name}[{i}].value"),
        report,
    );
    report.applied += 1;
}

/// `apply_budget` 的两条路（投资 / 建造）——同一套代码，只有 map 与名字不同。
#[derive(Clone, Copy)]
pub enum BudgetKind {
    Investment,
    Construction,
}

impl BudgetKind {
    pub fn name(self) -> &'static str {
        match self {
            BudgetKind::Investment => "investment_budget",
            BudgetKind::Construction => "construction_budget",
        }
    }
}

/// 某城某建筑的两类权重补丁（建设投资权重 / 建造投资权重）：`value` 替换权重、`remove` 删叶。
pub fn apply_weight(
    state: &mut State,
    fid: &FactionId,
    which: WeightKind,
    city: &CityId,
    building: &BuildingId,
    value: Option<f64>,
    mode: Option<ControlMode>,
    remove: bool,
    i: usize,
    report: &mut ApplyReport,
) {
    let name = which.name();
    let path = format!("{fid}.{name}[{i}]");
    let mut present = Vec::new();
    if value.is_some() {
        present.push("value");
    }
    if mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(remove, &present, &path, report) {
        return;
    }
    let key = (city.clone(), *building);
    // 删叶先于「城/建筑是否存在」的检查：建筑没了、城易主了也能删（清理陈叶）。
    if remove {
        let c = state.control.entry(fid.clone()).or_default();
        let map = match which {
            WeightKind::Invest => &mut c.invest_weights,
            WeightKind::Build => &mut c.build_weights,
        };
        let existed = map.remove(&key).is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if !check_city_building(state, fid, city, *building, &path, report) {
        return;
    }
    let c = state.control.entry(fid.clone()).or_default();
    let map = match which {
        WeightKind::Invest => &mut c.invest_weights,
        WeightKind::Build => &mut c.build_weights,
    };
    let ctrl = map.entry(key).or_insert_with(|| Control {
        value: value.unwrap_or(0.0),
        mode: mode.unwrap_or_default(),
    });
    write_value_leaf(
        ctrl,
        value,
        mode,
        format!("{fid}.{name}[{i}].value"),
        report,
    );
    report.applied += 1;
}

/// `apply_weight` 的两条路（建设投资权重 / 建造投资权重）。
#[derive(Clone, Copy)]
pub enum WeightKind {
    Invest,
    Build,
}

impl WeightKind {
    pub fn name(self) -> &'static str {
        match self {
            WeightKind::Invest => "invest_weights",
            WeightKind::Build => "build_weights",
        }
    }
}

/// 某城娱乐/福利预算补丁：`value` 替换预算额、`remove` 删叶。
pub fn apply_loyalty_budget(
    state: &mut State,
    fid: &FactionId,
    lp: &LoyaltyBudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.loyalty_budget[{i}]");
    let mut present = Vec::new();
    if lp.value.is_some() {
        present.push("value");
    }
    if lp.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(lp.remove, &present, &path, report) {
        return;
    }
    if lp.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .loyalty_budget
            .remove(&lp.city)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    match state.city(&lp.city) {
        None => {
            report.skip(
                format!("{path}.city"),
                &lp.city,
                "no_such_city",
                format!(
                    "没有名为「{}」的城（城被夷平后名字会从活城列表里消失）。",
                    lp.city
                ),
            );
            return;
        }
        Some(city) if city.faction_id != *fid => {
            report.skip(
                format!("{path}.city"),
                &lp.city,
                "not_your_city",
                format!("「{}」属于 {}，不是 {fid} 的城。", lp.city, city.faction_id),
            );
            return;
        }
        Some(_) => {}
    }
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .loyalty_budget
        .entry(lp.city.clone())
        .or_insert_with(|| Control {
            value: lp.value.unwrap_or(0.0),
            mode: lp.mode.unwrap_or_default(),
        });
    write_value_leaf(
        ctrl,
        lp.value,
        lp.mode,
        format!("{fid}.loyalty_budget[{i}].value"),
        report,
    );
    report.applied += 1;
}
