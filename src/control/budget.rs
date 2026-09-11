//! 预算与权重叶（budget / invest / build / loyalty）的写入。

use super::*;

/// 资源预算补丁（投资 / 建造共用）：`value` 替换预算额、`mode` 指定由谁决定。
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
    let path = format!("{fid}.{name}[{i}].资源");
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
        BudgetKind::Welfare => &mut c.welfare_budget,
    };
    let ctrl = map.entry(bp.resource.clone()).or_insert_with(|| Control {
        value: bp.value.unwrap_or(0.0),
        mode: bp.mode.unwrap_or_default(),
    });
    write_value_leaf(
        ctrl,
        bp.value,
        bp.mode,
        format!("{fid}.{name}[{i}].值"),
        report,
    );
    report.applied += 1;
}

/// `apply_budget` 的两条路（投资 / 建造）——同一套代码，只有 map 与名字不同。
#[derive(Clone, Copy)]
pub enum BudgetKind {
    Investment,
    Construction,
    Welfare,
}

impl BudgetKind {
    pub fn name(self) -> &'static str {
        match self {
            BudgetKind::Investment => "投资预算",
            BudgetKind::Construction => "建造预算",
            BudgetKind::Welfare => "福利预算",
        }
    }
}

/// 某城某建筑的两类权重补丁（建设投资权重 / 建造投资权重）：`value` 替换权重、`mode` 指定由谁决定。
pub fn apply_weight(
    state: &mut State,
    fid: &FactionId,
    which: WeightKind,
    city: &CityId,
    building: &BuildingId,
    value: Option<f64>,
    mode: Option<ControlMode>,
    i: usize,
    report: &mut ApplyReport,
) {
    let name = which.name();
    let path = format!("{fid}.{name}[{i}]");
    let key = (city.clone(), *building);
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
        format!("{fid}.{name}[{i}].值"),
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
            WeightKind::Invest => "建设权重",
            WeightKind::Build => "建造权重",
        }
    }
}

/// 城市级数值叶（福利权重 / 逐城开发货币 / 逐城建造货币）的同一条写路径。
#[derive(Clone, Copy)]
pub enum CityLeafKind {
    WelfareWeight,
    DevelopmentMoney,
    ConstructionMoney,
}

impl CityLeafKind {
    pub fn name(self) -> &'static str {
        match self {
            CityLeafKind::WelfareWeight => "城市福利预算",
            CityLeafKind::DevelopmentMoney => "开发货币预算",
            CityLeafKind::ConstructionMoney => "建造货币预算",
        }
    }
}

/// 城市级数值叶补丁：`value` 替换值、`mode` 指定由谁决定。
pub fn apply_city_leaf(
    state: &mut State,
    fid: &FactionId,
    which: CityLeafKind,
    lp: &LoyaltyBudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let name = which.name();
    let path = format!("{fid}.{name}[{i}]");
    match state.city(&lp.city) {
        None => {
            report.skip(
                format!("{path}.城"),
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
                format!("{path}.城"),
                &lp.city,
                "not_your_city",
                format!("「{}」属于 {}，不是 {fid} 的城。", lp.city, city.faction_id),
            );
            return;
        }
        Some(_) => {}
    }
    let c = state.control.entry(fid.clone()).or_default();
    let map = match which {
        CityLeafKind::WelfareWeight => &mut c.loyalty_budget,
        CityLeafKind::DevelopmentMoney => &mut c.development_money,
        CityLeafKind::ConstructionMoney => &mut c.construction_money,
    };
    let ctrl = map.entry(lp.city.clone()).or_insert_with(|| Control {
        value: lp.value.unwrap_or(0.0),
        mode: lp.mode.unwrap_or_default(),
    });
    write_value_leaf(
        ctrl,
        lp.value,
        lp.mode,
        format!("{fid}.{name}[{i}].值"),
        report,
    );
    report.applied += 1;
}

/// 某城**福利权重**补丁（兼容旧入口名）。
pub fn apply_loyalty_budget(
    state: &mut State,
    fid: &FactionId,
    lp: &LoyaltyBudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    apply_city_leaf(state, fid, CityLeafKind::WelfareWeight, lp, i, report);
}

/// 某城**开发货币预算**补丁。
pub fn apply_development_money(
    state: &mut State,
    fid: &FactionId,
    lp: &LoyaltyBudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    apply_city_leaf(state, fid, CityLeafKind::DevelopmentMoney, lp, i, report);
}

/// 某城**建造货币预算**补丁。
pub fn apply_construction_money(
    state: &mut State,
    fid: &FactionId,
    lp: &LoyaltyBudgetPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    apply_city_leaf(state, fid, CityLeafKind::ConstructionMoney, lp, i, report);
}
