//! 设计图叶片：引用它的船坞、意图轴、`apply_blueprint`。

use super::*;

/// 挂了这张图的建造区（城名、建筑下标、该区当前的 `ship_type`）。
pub fn referencing_yards(
    state: &State,
    fid: &str,
    bp: &BlueprintId,
) -> Vec<(CityId, BuildingId, String)> {
    state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        .flat_map(|c| {
            c.buildings
                .iter()
                .filter(|b| b.blueprint.as_deref() == Some(bp.as_str()))
                .map(|b| {
                    (
                        c.name.clone(),
                        b.id,
                        b.ship_type.clone().unwrap_or_default(),
                    )
                })
        })
        .collect()
}

/// 本份 diff 打算把**哪些建造区**改成**哪个舰级**（键 = `(城, 建筑下标)`）。
///
/// 为什么需要它：口径 A 要求「图的 `class` == 建造区的 `ship_type`」，而这个约束的校验
/// 发生在两处（改图 / 改区）。若两处各自只看**当前**状态，「两处一起写」（这是 spec §4.5
/// 教的正解）就会被先落地的那一半拒掉——正确的判据是**这份 diff 之后的意图**。
pub fn yard_ship_type_intent(fac: &FactionControlPatch) -> BTreeMap<(CityId, BuildingId), String> {
    let mut m = BTreeMap::new();
    for b in &fac.buildings {
        if let (Some(city), Some(bid), Some(st)) =
            (b.city.as_ref(), b.building, b.ship_type.as_ref())
        {
            m.insert((city.clone(), bid), st.clone());
        }
    }
    m
}

/// **设计图**补丁：新建 / 改值（舰级、选装、**倾向三轴**）/ 改归属 / 删图。
///
/// ⚠ 图能表态的是**长期倾向**（风格 / 姿态 / 角色），**不是指令**（2026-10 裁决：
/// 指令是即时操作，只写逐舰叶）——原来的 `order` 字段已删。
///
/// 校验与丢弃码见 [`BlueprintPatch`] 与 `.agents/notes/ship-blueprint-spec.md` §4.6。
/// 顺序与其它叶一致：**删叶（含冲突检查）→ 校验 → 写**。
pub fn apply_blueprint(
    state: &mut State,
    config: &GameConfig,
    fid: &FactionId,
    patch: &BlueprintPatch,
    i: usize,
    yard_intent: &BTreeMap<(CityId, BuildingId), String>,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.blueprints[{i}]");
    let mut present = Vec::new();
    if patch.class.is_some() {
        present.push("class");
    }
    if patch.components.is_some() {
        present.push("components");
    }
    if patch.doctrine.is_some() {
        present.push("doctrine");
    }
    if patch.kiting.is_some() {
        present.push("kiting");
    }
    if patch.role.is_some() {
        present.push("role");
    }
    if patch.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(patch.remove, &present, &path, report) {
        return;
    }
    // 删**整张图**：挂它的建造区随后是悬空指针 ⇒ 停产（Q10(a)）。图不存在时是幂等成功。
    if patch.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .blueprints
            .remove(&patch.name)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let current = state
        .control(fid.clone())
        .and_then(|c| c.blueprints.get(&patch.name))
        .cloned();
    let wrote_value = patch.class.is_some()
        || patch.components.is_some()
        || patch.doctrine.is_some()
        || patch.kiting.is_some()
        || patch.role.is_some();
    // 图名不存在 + 没有写任何值 ⇒ **绝不凭空造图**（同 `no_such_faction` 防幽灵势力的理由：
    // 一个错别字会造出一张谁都不认识的图，它随后出现在读面里，看起来像真的）。
    if current.is_none() && !wrote_value {
        report.skip(
            format!("{path}.name"),
            &patch.name,
            "no_such_blueprint",
            format!(
                "「{}」这张设计图不在 {fid} 的设计图库里（图名是唯一 key，会被改名/删除）。想建一张新图请把 `class` 一起写上（写值即接管）；只写 `mode` 不会凭空造图。",
                patch.name
            ),
        );
        return;
    }
    // 目标值：写了的用写的，没写的保留现值（新建时 `class` 必给）。
    let class = match (patch.class.as_ref(), current.as_ref()) {
        (Some(c), _) => c.clone(),
        (None, Some(v)) => v.value.class.clone(),
        (None, None) => {
            report.skip(
                format!("{path}.class"),
                "",
                "missing_class",
                "新建一张设计图必须给 `class`（舰级 = config.ships 的 key）：图是「还不存在的舰」的出厂规格，没有舰级的图印不出舰。",
            );
            return;
        }
    };
    if !config.ships.contains_key(&class) {
        let all: Vec<&str> = config.ships.keys().map(String::as_str).collect();
        report.skip(
            format!("{path}.class"),
            &class,
            "no_such_class",
            format!("没有舰级「{class}」（可选：{}）。", all.join(" / ")),
        );
        return;
    }
    let components = patch.components.clone().unwrap_or_else(|| {
        current
            .as_ref()
            .map(|v| v.value.components.clone())
            .unwrap_or_default()
    });
    for c in &components {
        if !config.components.contains_key(c) {
            let all: Vec<&str> = config.components.keys().map(String::as_str).collect();
            report.skip(
                format!("{path}.components"),
                c,
                "no_such_component",
                format!(
                    "没有组件「{c}」（可选：{}；也可用 --meta 看全表）。",
                    all.join(" / ")
                ),
            );
            return;
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &components {
        if !seen.insert(c.clone()) {
            report.skip(
                format!("{path}.components"),
                c,
                "duplicate_component",
                format!(
                    "组件「{c}」在同一张图里出现了两次。一件组件一个槽位——与 `choose_loadout` 的 `!chosen.contains(id)` 同一条规则；「双主炮」是新机制，要先重审槽位与平衡。"
                ),
            );
            return;
        }
    }
    let slots = config.ship_spec(&class).slots as usize;
    if components.len() > slots {
        report.skip(
            format!("{path}.components"),
            components.len().to_string(),
            "too_many_components",
            format!(
                "这张图装了 {} 件，而 {class} 只有 {slots} 个槽位（槽位上限必须仍然生效，否则设计图就是新的失衡入口）。",
                components.len()
            ),
        );
        return;
    }
    // 口径 A（Q3）：图的 `class` 必须与**挂它的每个建造区**的 `ship_type` 相等。
    // 「两处一起写」是正解（spec §4.5）⇒ 本份 diff 里那个区的目标舰级也算数。
    for (cid, bid, st) in referencing_yards(state, fid, &patch.name) {
        if st == class {
            continue;
        }
        if yard_intent.get(&(cid.clone(), bid)) == Some(&class) {
            continue;
        }
        report.skip(
            format!("{path}.class"),
            &class,
            "blueprint_class_mismatch",
            format!(
                "「{}」的建造区（{cid} / building={bid}）产的是「{st}」，而这张图的 class 要写成「{class}」：口径 A 下二者必须相等。要么同一份 diff 里把这个建造区的 `ship_type` 也改成同一级，要么别动图的舰级。",
                patch.name
            ),
        );
        return;
    }
    // 写：值 + 归属（写值即接管，与其它叶同一条规则）。
    let leaf = state
        .control
        .entry(fid.clone())
        .or_default()
        .blueprints
        .entry(patch.name.clone())
        .or_insert_with(|| {
            Control::inherit(Blueprint {
                class: class.clone(),
                components: Vec::new(),
                doctrine: None,
                kiting: None,
                role: None,
            })
        });
    leaf.value.class = class;
    leaf.value.components = components;
    // 倾向三轴：`Some(None)` = 本图对**这条轴**没有说话（清空这一层，链继续往下降到舰队默认）；
    // `Some(Some(v))` = 表态；缺席 = 不动。
    if let Some(doctrine) = patch.doctrine {
        leaf.value.doctrine = doctrine;
    }
    if let Some(kiting) = patch.kiting {
        leaf.value.kiting = kiting;
    }
    if let Some(role) = patch.role {
        leaf.value.role = role;
    }
    write_mode_leaf(&mut leaf.mode, patch.mode, wrote_value, path, report);
    report.applied += 1;
}
