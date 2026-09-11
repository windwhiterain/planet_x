//! 写面入口：`apply_diff` / `apply_patch`，以及叶子写入原语（模式叶 / 值叶）。

use super::*;

/// `删除: true` 同时带了别的字段 ⇒ 记一条拒绝。返回 `true` = 这条补丁到此为止。
pub fn remove_conflicts(
    remove: bool,
    present: &[&str],
    path: &str,
    report: &mut ApplyReport,
) -> bool {
    if !remove || present.is_empty() {
        return false;
    }
    report.skip(
        path,
        "",
        "remove_conflicts_with_value",
        format!(
            "`删除: true` 不能再带 {}：删掉这张图与给它写值/写归属是两件事（要什么值请删完再单独发一条）。",
            present.join(" / ")
        ),
    );
    true
}

/// 记一次删除的结果：**真的**删掉了才进 `removed`；本来就没有这张图是幂等成功。
pub fn leaf_removed(report: &mut ApplyReport, path: String, existed: bool) {
    if existed {
        report.removed(path);
    }
    report.applied += 1;
}

/// 把一条 `ship` 引用解析成「本势力确实拥有的那艘舰」，或一条**丢弃记录**。
/// 舰名是唯一 key，但**会变**（战沉后重建的护卫舰叫 `长城2`），所以「查无此舰」
/// 与「这舰不是你的」必须分开报——前者要 agent 重新查名字，后者是写错了势力。
pub fn resolve_own_ship(
    state: &State,
    fid: &str,
    ship: &str,
    path: &str,
    report: &mut ApplyReport,
) -> Option<String> {
    match state.ships.iter().find(|s| s.name == ship) {
        None => {
            report.skip(
                path,
                ship,
                "no_such_ship",
                format!("世界里没有名为「{ship}」的舰。舰名会换代（战沉后重建的是「{ship}2」），用 --index + planet_xq 的 q.fleet(r, \"{fid}\") 查现名。"),
            );
            None
        }
        Some(s) if s.faction_id != fid => {
            report.skip(
                path,
                ship,
                "not_your_ship",
                format!(
                    "「{ship}」属于 {}，不是 {fid} 的舰——指令只对本势力的舰生效。",
                    s.faction_id
                ),
            );
            None
        }
        Some(s) => Some(s.name.clone()),
    }
}

/// 落一条「值 + 三态」的叶片补丁（预算 / 权重 / 忠诚预算共用）：**写值即接管**——
/// 只写了值、没写 `mode`，就意味着这是玩家的指令（该叶变成 `Player`），并在回执里记一笔。
///
/// 为什么：值写进去、而 mode 仍解析成 `Auto` 时，系统下一回合就会按自己的逻辑覆盖它。
/// stdout 与退出码一切正常，agent 却会带着「命令已下达」的错觉玩下去——这正是本项目
/// 反复吃过的「失败看起来像成功」。
/// 「写值即接管」的三态落点（叶片共用）：显式 `mode` 优先；只写了值没写 `mode` ⇒ `Player`
/// 并在回执里记一笔；什么都没写 ⇒ 保留现模式。
pub fn write_mode_leaf(
    ctrl: &mut ControlMode,
    mode: Option<ControlMode>,
    wrote_value: bool,
    path: String,
    report: &mut ApplyReport,
) {
    match mode {
        Some(m) => *ctrl = m,
        None if wrote_value => {
            *ctrl = ControlMode::Player;
            report.took_over(path);
        }
        None => {}
    }
}

pub fn write_value_leaf<T>(
    ctrl: &mut Control<T>,
    value: Option<T>,
    mode: Option<ControlMode>,
    path: String,
    report: &mut ApplyReport,
) {
    let wrote_value = value.is_some();
    if let Some(v) = value {
        ctrl.value = v;
    }
    match mode {
        Some(m) => ctrl.mode = m,
        None if wrote_value => {
            ctrl.mode = ControlMode::Player;
            report.took_over(path);
        }
        None => {}
    }
}

/// Apply a presence-aware control patch (a structural multi-level diff) to
/// `state`. Only the factions and leaves present in `req` are modified; for a
/// leaf that is present, an omitted `value`/`behavior` keeps the current value
/// and an omitted `mode` keeps the current mode. `scope` is overlaid when given.
///
/// 返回 [`ApplyReport`]：落地了几个叶片、**丢了哪些以及为什么**。丢弃不是错误
/// （「只触碰 diff 里出现的叶片」本来就允许引用已经不存在的实体），但它是 agent
/// 唯一能发现「命令其实没下达」的渠道，所以每一条 `continue`/`return` 都必须
/// 先记一笔——**新增分支时别忘了 `report.skip`**。
pub fn apply_diff(state: &mut State, config: &GameConfig, req: &CommandReq) -> ApplyReport {
    let mut report = ApplyReport::default();
    for (fi, fac) in req.control.iter().enumerate() {
        let fid = fac.faction_id.clone();
        // 势力必须真实存在：否则 `control.entry().or_default()` 会在可控状态里
        // **凭空造出一个幽灵势力**，它随后出现在 `--control` 模板与 web 控制面里，
        // 看起来像一个真的（只是永远不动）势力——一个错别字污染整个世界。
        if state.faction(&fid).is_none() {
            report.skip(
                format!("control[{fi}].势力"),
                &fid,
                "no_such_faction",
                format!("没有名为「{fid}」的势力（势力名是唯一 key；用 --index + planet_xq 的 q.facts 看现名）。"),
            );
            continue;
        }
        // 注意：这里**不能**提前 `let c = state.control.entry(..)`——那会把
        // `state.control` 借出去，后面所有需要 `state.city(..)` 的校验都借不动。
        // 每个写点各自取一次 entry（同名 `fid` 的 `Control` 是同一个）。
        //
        // 每片叶一个 `apply_*` 助手：它们各自处理「写值 / 写归属」两件事，
        // 顺序统一是 **实体校验 → 写**。抽出来的原因不是行数：写值即接管与
        // presence-aware 保留现值的语义要在**每一片**叶上完全一致。
        if let Some(d) = &fac.default_doctrine {
            apply_default_doctrine(state, &fid, d, &mut report);
        }
        if let Some(d) = &fac.default_kiting {
            apply_default_kiting(state, &fid, d, &mut report);
        }
        // 舰队默认**角色**（势力级，第三条风格轴）。写它 = 全舰队按这个角色走；
        // 设成 `Player` 之后自动控制的逐舰定编不再生效（那片叶归玩家）。
        if let Some(d) = &fac.default_role {
            apply_default_role(state, &fid, d, &mut report);
        }
        // **设计图库**（势力级）：**先于** `buildings` 应用——同一份 diff 里「建图 + 把某个
        // 建造区指过去」必须一次成功（否则 agent 得写两条命令，中间那条会报
        // `no_such_blueprint`）。校验用的是**本份 diff 之后**的意图（`yard_intent`），
        // 所以「图与区的舰级两处一起写」也一次成功。
        let yard_intent = yard_ship_type_intent(fac);
        for (i, bp) in fac.blueprints.iter().enumerate() {
            apply_blueprint(state, config, &fid, bp, i, &yard_intent, &mut report);
        }
        for (i, sp) in fac.ship_orders.iter().enumerate() {
            apply_ship_order(state, &fid, sp, i, &mut report);
        }
        for (i, d) in fac.ship_doctrine.iter().enumerate() {
            apply_ship_doctrine(state, &fid, d, i, &mut report);
        }
        for (i, k) in fac.ship_kiting.iter().enumerate() {
            apply_ship_kiting(state, &fid, k, i, &mut report);
        }
        // **角色**补丁（per-舰，第三条风格轴）：同一条路（叶片 + 写值即接管）。
        // 与另两条轴的差别：这片叶自动控制**也会写**，但**玩家写过（`Player`）之后 AI 不再碰**
        // ——所以「手动给某艘舰定活」是一次性的、且能一直压住自动定编。
        for (i, f) in fac.ship_role.iter().enumerate() {
            apply_ship_role(state, &fid, f, i, &mut report);
        }
        for (i, bp) in fac.investment_budget.iter().enumerate() {
            apply_budget(
                state,
                config,
                &fid,
                BudgetKind::Investment,
                bp,
                i,
                &mut report,
            );
        }
        for (i, bp) in fac.construction_budget.iter().enumerate() {
            apply_budget(
                state,
                config,
                &fid,
                BudgetKind::Construction,
                bp,
                i,
                &mut report,
            );
        }
        for (i, bp) in fac.welfare_budget.iter().enumerate() {
            apply_budget(
                state,
                config,
                &fid,
                BudgetKind::Welfare,
                bp,
                i,
                &mut report,
            );
        }
        for (i, ip) in fac.invest_weights.iter().enumerate() {
            apply_weight(
                state,
                &fid,
                WeightKind::Invest,
                &ip.city,
                &ip.building,
                ip.value,
                ip.mode,
                i,
                &mut report,
            );
        }
        for (i, bp) in fac.build_weights.iter().enumerate() {
            apply_weight(
                state,
                &fid,
                WeightKind::Build,
                &bp.city,
                &bp.building,
                bp.value,
                bp.mode,
                i,
                &mut report,
            );
        }
        for (i, lp) in fac.loyalty_budget.iter().enumerate() {
            apply_loyalty_budget(state, &fid, lp, i, &mut report);
        }
        for (i, lp) in fac.development_money.iter().enumerate() {
            apply_development_money(state, &fid, lp, i, &mut report);
        }
        for (i, lp) in fac.construction_money.iter().enumerate() {
            apply_construction_money(state, &fid, lp, i, &mut report);
        }
        for (i, bpatch) in fac.buildings.iter().enumerate() {
            let path = format!("{fid}.buildings[{i}]");
            if apply_building_patch(state, config, fid.clone(), bpatch, &path, &mut report) {
                report.applied += 1;
            }
        }
    }
    // 迁都：写「有效首都」的唯一事实来源（`ControllableState::capital`）。独立的循环
    // 以拿到干净的借用：先做只读（当前首都、天体存在性），再在独立作用域里写控制。
    // 只允许迁到真实存在的天体；若指向的天体上并无本势力活城，则由 sim 的亡城强迁
    // 规则兜底，避免把首都钉在虚天体上。`mode` 沿作用域链与其它叶子一致。
    for (fi, fac) in req.control.iter().enumerate() {
        if let Some(cap) = &fac.capital {
            if state.faction(&fac.faction_id).is_none() {
                continue; // 已在上面的循环里记过 no_such_faction。
            }
            let path = format!("control[{fi}].首都");
            let cur = state.capital_body(&fac.faction_id);
            let new_value = match cap.value.as_ref() {
                Some(v) if state.body(v).is_none() => {
                    report.skip(
                        format!("{path}.值"),
                        v,
                        "no_such_body",
                        format!("没有名为「{v}」的天体（天体名是唯一 key）。"),
                    );
                    None
                }
                other => other.cloned(),
            };
            {
                let ctrl = state.control.entry(fac.faction_id.clone()).or_default();
                let ctrl = ctrl.capital.get_or_insert_with(|| Control::inherit(cur));
                // 无效天体（上面刚跳过）不算「写了值」，否则会记一条假的隐含接管。
                let wrote = new_value.is_some();
                if let Some(v) = new_value {
                    ctrl.value = v;
                }
                match (cap.mode, wrote) {
                    (Some(m), _) => ctrl.mode = m,
                    (None, true) => {
                        ctrl.mode = ControlMode::Player;
                        report.took_over(format!("{path}.值"));
                    }
                    (None, false) => {}
                }
            }
            report.applied += 1;
        }
    }
    if let Some(sv) = &req.scope {
        // 作用域补丁：**每个被触碰的节点算一个叶片**（`applied` 是「落地了几个叶片」
        // 的口径，作用域节点与叶子同权）。
        let touched = usize::from(sv.global.is_some())
            + sv.factions.len()
            + sv.bodies.len()
            + sv.cities.len();
        state.scope.overlay(sv);
        report.applied += touched;
    }
    report
}

/// Parse a control diff file (JSON) and apply it to `state` as a structural
/// multi-level patch. Accepts the same shape as `POST /api/command`
/// (`{control:[...],scope:{...}}`). Ship behaviors may be written in either the
/// default enum form or the tagged agent-state form (see [`normalize_behavior`]).
///
/// Returns the [`ApplyReport`] (what landed / what was dropped and why) so the
/// caller can tell the agent. Callers that don't care (the web command route)
/// may ignore it.
pub fn apply_patch(
    state: &mut State,
    config: &GameConfig,
    value: &serde_json::Value,
) -> Result<ApplyReport, String> {
    let mut v = value.clone();
    normalize_control_diffs(&mut v)?;
    let req: CommandReq =
        serde_json::from_value(v).map_err(|e| format!("invalid control diff: {e}"))?;
    Ok(apply_diff(state, config, &req))
}
