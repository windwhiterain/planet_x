//! 逐舰叶片 + 舰队默认叶（order / doctrine / kiting / freighter）的写入。

use super::*;

/// 舰队默认指令（势力级）：新舰出生与一次性指令收尾都回落到它。
pub fn apply_default_ship_order(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultShipOrder,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_ship_order");
    let mut present = Vec::new();
    if d.behavior.is_some() {
        present.push("behavior");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .default_ship_order
            .take()
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let c = state.control.entry(fid.clone()).or_default();
    let ctrl = c
        .default_ship_order
        .get_or_insert_with(|| Control::inherit(ShipBehavior::Idle));
    if let Some(v) = &d.behavior {
        ctrl.value = v.clone();
    }
    // 写值即接管（与叶子同一条规则）：只写默认行为、没写 mode，就是「这是我的默认」。
    match (d.mode, d.behavior.is_some()) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 舰队默认**行为风格**（势力级，两片之一）：与「写值即接管」同一条规则，外加一条
/// **两轴叶**的额外守卫（见 `partial_doctrine_leaf`）。
pub fn apply_default_doctrine(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultDoctrine,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_doctrine");
    let mut present = Vec::new();
    if d.temper.is_some() {
        present.push("temper");
    }
    if d.lone_wolf.is_some() {
        present.push("lone_wolf");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .default_doctrine
            .take()
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let wrote = d.temper.is_some() || d.lone_wolf.is_some();
    // **两轴叶的"新建"必须两条轴一起给**（`0.0` 是个正常取值：静默把它填进另一条轴，
    // 事后从读面完全看不出来——这正是这条守卫存在的理由）。叶已存在时单轴写仍然合法
    // （缺省轴保留现值）。
    let exists = state
        .control
        .get(fid)
        .and_then(|c| c.default_doctrine.as_ref())
        .is_some();
    if !exists && wrote && !(d.temper.is_some() && d.lone_wolf.is_some()) {
        report.skip(
            path,
            "",
            "partial_doctrine_leaf",
            "`default_doctrine` 是**两轴一片叶**（temper + lone_wolf），而这片叶还不存在：只给一条轴会把另一条静默设成 0.0（= 基线），而 0.0 是个正常取值，事后从读面看不出来。三条路任选：① 两条轴一起给；② 先只写 `mode`（先表态归属，值下次再给）；③ `remove: true` 删掉这片叶（回到「没有说话」）。",
        );
        return;
    }
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .default_doctrine
        .get_or_insert_with(|| Control::inherit(ShipDoctrine::default()));
    if let Some(v) = d.temper {
        ctrl.value.temper = v.clamp(-1.0, 1.0);
    }
    if let Some(v) = d.lone_wolf {
        ctrl.value.lone_wolf = v.clamp(-1.0, 1.0);
    }
    match (d.mode, wrote) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 舰队默认**风筝<->贴脸姿态**（势力级，两片之二）。
pub fn apply_default_kiting(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultKiting,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_kiting");
    let mut present = Vec::new();
    if d.kiting.is_some() {
        present.push("kiting");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .default_kiting
            .take()
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let wrote = d.kiting.is_some();
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .default_kiting
        .get_or_insert_with(|| Control::inherit(0.0));
    if let Some(v) = d.kiting {
        ctrl.value = v.clamp(-1.0, 1.0);
    }
    match (d.mode, wrote) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 舰队默认**角色**（势力级，第三条风格轴）：与 [`apply_default_kiting`] 同形，
/// 外加这片叶特有的用途——设成 `Player` 是「AI 定编别碰我的舰队」的闸门。
pub fn apply_default_role(
    state: &mut State,
    fid: &FactionId,
    d: &DefaultShipRole,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.default_role");
    let mut present = Vec::new();
    if d.role.is_some() {
        present.push("role");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    if d.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .default_role
            .take()
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    let wrote = d.role.is_some();
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .default_role
        .get_or_insert_with(|| Control::inherit(ShipRole::default()));
    if let Some(v) = d.role {
        ctrl.value = v;
    }
    match (d.mode, wrote) {
        (Some(m), _) => ctrl.mode = m,
        (None, true) => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        (None, false) => {}
    }
    report.applied += 1;
}

/// 逐舰指令补丁：`behavior` 替换该舰行为、`mode` 指定由谁决定、`remove` 删掉这片叶。
pub fn apply_ship_order(
    state: &mut State,
    fid: &FactionId,
    sp: &ShipOrderPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_orders[{i}].ship");
    let mut present = Vec::new();
    if sp.behavior.is_some() {
        present.push("behavior");
    }
    if sp.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(sp.remove, &present, &path, report) {
        return;
    }
    // 删叶**不要求舰还在**：删的是我们控制面里的那片叶（陈叶清理也是它的用途之一）。
    if sp.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_orders
            .remove(&sp.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    // Resolve the ship by its **name** (the unique key): an order only applies to a
    // ship that exists and that this faction actually owns, so ordering another
    // faction's ship (or a vanished one) is a no-op.
    let Some(ship_name) = resolve_own_ship(state, fid, &sp.ship, &path, report) else {
        return;
    };
    // 「写值即接管」：只写了 behavior 而没写 mode，就意味着这是**玩家的指令**
    // （否则值会被系统下一回合按自己的逻辑覆盖，而 agent 以为命令已下达——
    // 这正是 `agent-play-friction` 里那类「失败看起来像成功」）。
    let implied = match (sp.mode, sp.behavior.is_some()) {
        (Some(m), _) => m,
        (None, true) => {
            report.took_over(format!("{fid}.ship_orders[{i}].behavior"));
            ControlMode::Player
        }
        (None, false) => ControlMode::Inherit,
    };
    // **不建"空叶"**：读面每舰一行之后，模板里会出现
    // `{"ship": X, "behavior": null, "mode": "Inherit"}` —— 它说的正是「这一层没有说话」。
    // 若无条件 `or_insert_with`，回传模板会给每一艘**叶被删过**的舰重新建出一片
    // `value = Idle` 的叶，于是「链上没人说话」（有效值 `null`）静默变成「叶里记着 Idle」
    // （有效值 `Some(Idle)`）——模板回传就不再是不动点，而且这是一次**没人要求**的写操作。
    // 规则：既没写值、表态又是 `Inherit` ⇒ 幂等成功（目标状态"这一层没有说话"已经成立），
    // 与「删一片本来就不存在的叶」同一条语义（`control-live-layers.md` §11.1 规则 2）。
    let leaf_exists = state
        .control
        .get(fid)
        .map(|c| c.ship_orders.contains_key(&ship_name))
        .unwrap_or(false);
    if !leaf_exists && sp.behavior.is_none() && implied == ControlMode::Inherit {
        report.applied += 1;
        return;
    }
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_orders
        .entry(ship_name)
        .or_insert_with(|| Control {
            value: sp.behavior.clone().unwrap_or(ShipBehavior::Idle),
            mode: implied,
        });
    if let Some(v) = &sp.behavior {
        ctrl.value = v.clone();
    }
    if let Some(m) = sp.mode {
        ctrl.mode = m;
    } else if sp.behavior.is_some() {
        ctrl.mode = ControlMode::Player;
    }
    report.applied += 1;
}

/// 逐舰**行为风格**补丁：写的是**叶片**（值 + 三态），不是舰上那个记录值——`Ship.doctrine`
/// 只是出厂快照 / AI 流水。每条轴钳制到 [-1,1]；缺省轴保留「当前有效」的那条（所以只写一条轴
/// 不会把另一条清零，也**不会**把另一条变成 0）。写值即接管（与其它叶同一条规则）。
pub fn apply_ship_doctrine(
    state: &mut State,
    fid: &FactionId,
    d: &ShipDoctrinePatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_doctrine[{i}].ship");
    let mut present = Vec::new();
    if d.temper.is_some() {
        present.push("temper");
    }
    if d.lone_wolf.is_some() {
        present.push("lone_wolf");
    }
    if d.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(d.remove, &present, &path, report) {
        return;
    }
    // 删叶 ⇒ 有效风格回落到舰队默认 / **出厂快照**（叶不存在 = 这一层没有说话，且叶里的值
    // 不再参与取值）。舰已战沉也能删。
    if d.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_doctrine
            .remove(&d.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if resolve_own_ship(state, fid, &d.ship, &path, report).is_none() {
        return;
    }
    let base = state.ship_doctrine(d.ship.clone());
    let value = ShipDoctrine {
        temper: d.temper.map(|v| v.clamp(-1.0, 1.0)).unwrap_or(base.temper),
        lone_wolf: d
            .lone_wolf
            .map(|v| v.clamp(-1.0, 1.0))
            .unwrap_or(base.lone_wolf),
    };
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_doctrine
        .entry(d.ship.clone())
        .or_insert_with(|| Control::inherit(base));
    ctrl.value = value;
    write_mode_leaf(
        &mut ctrl.mode,
        d.mode,
        d.temper.is_some() || d.lone_wolf.is_some(),
        format!("{fid}.ship_doctrine[{i}]"),
        report,
    );
    report.applied += 1;
}

/// 逐舰**风筝<->贴脸姿态**补丁：与 [`apply_ship_doctrine`] 同一条路（叶片 + 写值即接管 + 删叶）。
pub fn apply_ship_kiting(
    state: &mut State,
    fid: &FactionId,
    k: &ShipKitingPatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_kiting[{i}].ship");
    let mut present = Vec::new();
    if k.kiting.is_some() {
        present.push("kiting");
    }
    if k.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(k.remove, &present, &path, report) {
        return;
    }
    if k.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_kiting
            .remove(&k.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if resolve_own_ship(state, fid, &k.ship, &path, report).is_none() {
        return;
    }
    let base = state.ship_kiting(k.ship.clone());
    let value = k.kiting.map(|v| v.clamp(-1.0, 1.0)).unwrap_or(base);
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_kiting
        .entry(k.ship.clone())
        .or_insert_with(|| Control::inherit(base));
    ctrl.value = value;
    write_mode_leaf(
        &mut ctrl.mode,
        k.mode,
        k.kiting.is_some(),
        format!("{fid}.ship_kiting[{i}]"),
        report,
    );
    report.applied += 1;
}

/// 逐舰**角色**补丁（第三条风格轴）：与 [`apply_ship_kiting`] 同一条路（叶片 + 写值即接管 +
/// 删叶）。唯一与另两条轴的差别：这片叶**自动控制也会写**（按积压定编），所以
/// 「删叶」在这条轴上的意思是**交回自动定编**（AI 可能下回合立刻又写下结论），
/// 而不是「从此保持某个值」——要后者就写 `Player`。
pub fn apply_ship_role(
    state: &mut State,
    fid: &FactionId,
    f: &ShipRolePatch,
    i: usize,
    report: &mut ApplyReport,
) {
    let path = format!("{fid}.ship_role[{i}].ship");
    let mut present = Vec::new();
    if f.role.is_some() {
        present.push("role");
    }
    if f.mode.is_some() {
        present.push("mode");
    }
    if remove_conflicts(f.remove, &present, &path, report) {
        return;
    }
    // 与另两条轴一样：删叶**不要求舰还在**（陈叶清理）。
    if f.remove {
        let existed = state
            .control
            .entry(fid.clone())
            .or_default()
            .ship_role
            .remove(&f.ship)
            .is_some();
        leaf_removed(report, path, existed);
        return;
    }
    if resolve_own_ship(state, fid, &f.ship, &path, report).is_none() {
        return;
    }
    let base = state.ship_role(f.ship.clone());
    let value = f.role.unwrap_or(base);
    let ctrl = state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_role
        .entry(f.ship.clone())
        .or_insert_with(|| Control::inherit(base));
    ctrl.value = value;
    write_mode_leaf(
        &mut ctrl.mode,
        f.mode,
        f.role.is_some(),
        format!("{fid}.ship_role[{i}]"),
        report,
    );
    report.applied += 1;
}
