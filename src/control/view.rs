//! 读面：`control_view` / `scope_view` / `control_surface` / `control_schema_value`。

use super::*;

/// `config` 只用于**派生读面**（`launch_waiting` 要按舰级的 `build_points` 判进度是否攒够），
/// 它不参与任何取值决策——写了什么就是什么，所以这条参数不改变控制面的语义。
pub fn control_view(
    state: &State,
    config: &GameConfig,
    fid: FactionId,
    c: &ControllableState,
) -> FactionControlView {
    // 指令：与下面三条风格轴**同形**——遍历 `state.ships`，**每舰一行**。
    // 值取**有效值**（叶 → 出厂图 → 舰队默认），`mode` 取叶片自己的表态（没有叶 = Inherit）。
    // 以前这里遍历的是 `c.ship_orders`（只有存在叶的舰），后果见 [`ShipOrderEntry`] 的说明。
    let ship_orders = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ShipOrderEntry {
            ship: s.name.clone(),
            behavior: state.ship_behavior(s.name.clone()),
            mode: c
                .ship_orders
                .get(&s.name)
                .map(|l| l.mode)
                .unwrap_or_default(),
        })
        .collect();
    // 读面：本势力每艘舰当前的行为风格。**值取有效值**（叶 → 舰队默认 → 舰上记录值），
    // **mode 取叶片自己的表态**（没有叶片 = Inherit）——于是"模板原样回传"安全：没被改过的
    // 行写回去仍然没有意见（有效值原样落进叶，而叶说 Inherit，取值回到原处）。
    let ship_doctrine = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| {
            let eff = state.ship_doctrine(s.name.clone());
            ShipDoctrineEntry {
                ship: s.name.clone(),
                temper: eff.temper,
                lone_wolf: eff.lone_wolf,
                mode: c
                    .ship_doctrine
                    .get(&s.name)
                    .map(|l| l.mode)
                    .unwrap_or_default(),
            }
        })
        .collect();
    let ship_kiting = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ShipKitingEntry {
            ship: s.name.clone(),
            kiting: state.ship_kiting(s.name.clone()),
            mode: c
                .ship_kiting
                .get(&s.name)
                .map(|l| l.mode)
                .unwrap_or_default(),
        })
        .collect();
    // 角色：**有效值**（自动控制可能刚写过它）+ 那片叶自己的表态（`Player` = 玩家钉的）。
    let ship_role = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| ShipRoleEntry {
            ship: s.name.clone(),
            role: state.ship_role(s.name.clone()),
            mode: c.ship_role.get(&s.name).map(|l| l.mode).unwrap_or_default(),
        })
        .collect();
    let investment_budget = c
        .investment_budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry {
            resource: rt.clone(),
            value: ctrl.value,
            mode: ctrl.mode,
        })
        .collect();
    let construction_budget = c
        .construction_budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry {
            resource: rt.clone(),
            value: ctrl.value,
            mode: ctrl.mode,
        })
        .collect();
    let welfare_budget = c
        .welfare_budget
        .iter()
        .map(|(rt, ctrl)| BudgetEntry {
            resource: rt.clone(),
            value: ctrl.value,
            mode: ctrl.mode,
        })
        .collect();
    let invest_weights = c
        .invest_weights
        .iter()
        .map(|((cid, bid), ctrl)| {
            let b = state
                .city(cid)
                .and_then(|cty| cty.buildings.iter().find(|b| b.id == *bid));
            InvestWeightEntry {
                city: cid.clone(),
                building: *bid,
                kind: b.map(|x| x.kind.clone()).unwrap_or_default(),
                resource: b.and_then(|x| x.resource.clone()),
                ship_type: b.and_then(|x| x.ship_type.clone()),
                structure: b.map(|x| x.structure.clone()).unwrap_or_default(),
                value: ctrl.value,
                mode: ctrl.mode,
            }
        })
        .collect();
    let build_weights = c
        .build_weights
        .iter()
        .map(|((cid, bid), ctrl)| {
            let b = state
                .city(cid)
                .and_then(|cty| cty.buildings.iter().find(|b| b.id == *bid));
            BuildWeightEntry {
                city: cid.clone(),
                building: *bid,
                ship_type: b.and_then(|x| x.ship_type.clone()),
                value: ctrl.value,
                mode: ctrl.mode,
            }
        })
        .collect();
    let loyalty_budget = c
        .loyalty_budget
        .iter()
        .map(|(cid, ctrl)| LoyaltyBudgetEntry {
            city: cid.clone(),
            value: ctrl.value,
            mode: ctrl.mode,
        })
        .collect();
    let development_money = c
        .development_money
        .iter()
        .map(|(cid, ctrl)| LoyaltyBudgetEntry {
            city: cid.clone(),
            value: ctrl.value,
            mode: ctrl.mode,
        })
        .collect();
    let construction_money = c
        .construction_money
        .iter()
        .map(|(cid, ctrl)| LoyaltyBudgetEntry {
            city: cid.clone(),
            value: ctrl.value,
            mode: ctrl.mode,
        })
        .collect();
    // 设计图库：**每张图一行**。`ship_count` 是**现算的派生量**（不落状态），`mode` 是图叶
    // 自己的表态；有效归属（图叶 → 势力 scope）走 `State::blueprint_control`，
    // 读面在投影的 `blueprints.effective_mode` 列里给（`--control` 是**写面模板**，
    // 多给派生列只会让模板与写面漂移）。
    //
    // ⚠ `ship_count` / `launch_waiting` 是这条规则的两个**例外**：它们确实是派生量，但
    // **必须**在写面模板里（读面即写面 ⇒ 写面得先收下它们；而且「这张图在等钱」是
    // 玩家做决定要看的东西，投影列在 CLI 侧够用、在 web 里够不着）。它们**只读**：
    // `BlueprintPatch` 收下但不写回状态。
    let blueprints = c
        .blueprints
        .iter()
        .map(|(name, ctrl)| BlueprintEntry {
            name: name.clone(),
            class: ctrl.value.class.clone(),
            components: ctrl.value.components.clone(),
            doctrine: ctrl.value.doctrine,
            kiting: ctrl.value.kiting,
            role: ctrl.value.role,
            mode: ctrl.mode,
            ship_count: state
                .ships
                .iter()
                .filter(|s| s.faction_id == fid && s.blueprint.as_deref() == Some(name.as_str()))
                .count(),
            launch_waiting: crate::sim::blueprint_launch_waiting(state, config, &fid, name),
        })
        .collect();
    FactionControlView {
        faction_id: fid,
        capital: c.capital.clone(),
        // 读面这几片是**值 + 表态**（"这一层说了什么"）：
        // 读面表达"没有这片叶"的方式就是 `None`/不给这一行（见 `scope_view` 同理）。
        default_doctrine: c.default_doctrine.as_ref().map(|d| DefaultDoctrine {
            temper: Some(d.value.temper),
            lone_wolf: Some(d.value.lone_wolf),
            mode: Some(d.mode),
        }),
        default_kiting: c.default_kiting.as_ref().map(|d| DefaultKiting {
            kiting: Some(d.value),
            mode: Some(d.mode),
        }),
        default_role: c.default_role.as_ref().map(|d| DefaultShipRole {
            role: Some(d.value),
            mode: Some(d.mode),
        }),
        blueprints,
        ship_orders,
        ship_doctrine,
        ship_kiting,
        ship_role,
        investment_budget,
        construction_budget,
        welfare_budget,
        invest_weights,
        build_weights,
        loyalty_budget,
        development_money,
        construction_money,
    }
}

/// 作用域树的**读面**：只列出**有意见**的节点/键（`Inherit` ≡ 没有说话，不必列出，
/// 与「这个键不存在」等价）。读面即写面，所以这份模板原样回传安全：没列出的层不会被
/// 意外清掉。
pub fn scope_view(s: &ControlScope) -> ControlScopePatch {
    ControlScopePatch {
        factions: explicit(&s.factions),
        bodies: explicit(&s.bodies),
        cities: explicit(&s.cities),
    }
}

/// 过滤掉「没有说话」（`Inherit`）的键。
pub fn explicit<K: Clone + Ord>(
    m: &std::collections::BTreeMap<K, ControlMode>,
) -> Vec<(K, ControlMode)> {
    m.iter()
        .filter(|(_, v)| **v != ControlMode::Inherit)
        .map(|(k, v)| (k.clone(), *v))
        .collect()
}

/// Render the current editable control surface (control + scope) as JSON —
/// the template an agent edits and posts back as a diff.
///
/// **这里刻意不做任何数值舍入**（曾经把所有数字四舍五入到 2 位小数以求 token 干净）。
/// 理由：这个函数是「**读面即写面、模板原样回传安全**」这句话的兑现处，
/// 而往模板里塞一个**有损**变换，等于把那句承诺变成假的 —— `0.125` 会被显示成 `0.13`，
/// 原样回传就真的把叶值改成了 `0.13`：一次静默的、没人要求的写操作。
///
/// 代价核算过（`engine-data-plane.md` §8.3）：实测一份跑到 120 回合的真实控制面里，
/// **470 个数值没有一个是 2 位小数舍入会改变的** —— 也就是说这点 token 噪声在当前世界里
/// 根本不存在，舍入**只带来风险、没带来收益**。想要好看的数字是客户端的事
/// （Python kit / LLM 自己 `round()`），引擎的输出是数据。
pub fn control_surface(state: &State, config: &GameConfig) -> serde_json::Value {
    let control = state
        .control
        .iter()
        .map(|(fid, c)| control_view(state, config, fid.clone(), c))
        .collect();
    let surface = ControlSurface {
        control,
        scope: scope_view(&state.scope),
    };
    serde_json::to_value(surface).expect("control surface is serializable")
}

/// The machine-readable JSON Schema for the **control/`--apply` diff**
/// ([`CommandReq`]). Handed to the agent so it can write a steering diff without
/// memorising the contract. Auto-derived from the same structs the diff is
/// deserialised into, so it can never drift from `apply_patch`'s shape.
///
/// ⚠ 除了 `schemars` 派生的**形状**，这里还并进 [`super::leaves::facts`] 那几段
/// **结构事实**（`leaves` / `actions` / `owner_field`）：
/// schemars 只知道「有这么个字段」，不知道「哪几个字段是身份键、哪几个是值、
/// 哪些是只读派生列」。写面（web 的控制行、kit 的 `set_*`）要的正是后者——
/// 见 [`super::leaves`] 的模块文档（一份事实、三端共用）。
/// 并进来的键名与 `schemars` 的不冲突（`$schema`/`title`/`type`/`properties`/`definitions`）。
pub fn control_schema_value() -> serde_json::Value {
    let mut out = crate::schema::of::<CommandReq>();
    let facts =
        serde_json::to_value(super::leaves::facts()).expect("control facts are serializable");
    if let (Some(top), Some(facts)) = (out.as_object_mut(), facts.as_object()) {
        for (k, v) in facts {
            top.insert(k.clone(), v.clone());
        }
    }
    out
}
