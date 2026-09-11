//! 逐舰叶与舰队默认叶：doctrine 补丁、舰队默认覆盖无叶的舰、角色轴的 Auto 放手、两轴默认叶必须一起建、单轴叶用「在用的值」补另一轴。

use super::*;

/// 应用一条舰风格补丁：只写给定轴、钳制到 [-1,1]、只作用于本势力自己的舰。
///
/// 而且它写的是**叶片**：舰上的 `Ship.doctrine`/`Ship.kiting` 是**记录值**（出厂快照 +
/// AI 流水），补丁不该动它——有效值走 `State::ship_doctrine`/`State::ship_kiting`。
#[test]
fn apply_ship_doctrine_patch() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let record_before = state.ship("长城").expect("长城 exists").doctrine;
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "风格": [
                {"舰": "长城", "temper": -1.0, "lone_wolf": 3.0},
                {"舰": "华盛顿", "temper": 1.0}
            ],
            "姿态": [
                {"舰": "长城", "姿态": -0.5},
                {"舰": "华盛顿", "姿态": 1.0}
            ]
        }]
    });
    apply_patch(&mut state, &config, &diff).expect("doctrine patch applies");
    let d = state.ship_doctrine("长城".to_string());
    assert_eq!(d.temper, -1.0);
    assert_eq!(d.lone_wolf, 1.0, "axis must be clamped to [-1,1]");
    assert_eq!(state.ship_kiting("长城".to_string()), -0.5);
    assert_eq!(
        state.ship("长城").unwrap().doctrine,
        record_before,
        "补丁写的是叶片；舰上的 doctrine 是记录值，不该被改"
    );
    // 写值即接管：这片叶从此归玩家。
    assert_eq!(
        state.ship_doctrine_control("长城".to_string()),
        ControlMode::Player
    );
    assert_eq!(
        state.ship_kiting_control("长城".to_string()),
        ControlMode::Player
    );
    // 华盛顿 belongs to 美国, not 中国 → the 中国 patch must be a no-op.
    assert_eq!(
        state.ship_doctrine("华盛顿".to_string()).temper,
        0.0,
        "other-faction ship must be untouched"
    );
    assert_eq!(
        state.ship_kiting("华盛顿".to_string()),
        0.0,
        "other-faction ship must be untouched"
    );
}

/// 舰队默认**风格**（`default_doctrine` / `default_kiting`）：一片叶改全舰队、
/// **新舰（还没有任何叶片）也自动跟随**、单舰特例仍然优先。
///
/// 这就是「按舰级默认」在控制面上的正解形态：不需要引擎加一层 `BTreeMap<舰级, …>`，
/// 「全舰队风筝、战列舰贴脸」= 一片默认叶 + 几片特例叶。
#[test]
fn fleet_default_style_covers_ships_without_leaves() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .map(|s| s.name.clone())
        .expect("中国 has a starting ship");

    // 1) 只写势力级默认：没有叶片的舰全部取它（写值即接管 ⇒ mode = Player）。
    let diff = serde_json::json!({
        "control": [{"势力": "中国", "舰队默认姿态": {"姿态": -1.0}}]
    });
    apply_patch(&mut state, &config, &diff).expect("default style applies");
    assert_eq!(
        state.ship_kiting(ship.clone()),
        -1.0,
        "没有叶片的舰必须跟随舰队默认"
    );
    assert_eq!(
        state.ship_kiting_control(ship.clone()),
        ControlMode::Player,
        "只写值不写 mode ⇒ 接管（与其它叶同一条规则）"
    );

    // 2) 单舰特例（更具体的层）优先。
    let diff = serde_json::json!({
        "control": [{"势力": "中国", "姿态": [{"舰": ship, "姿态": 1.0, "归属": "Player"}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("per-ship override applies");
    assert_eq!(
        state.ship_kiting(ship.clone()),
        1.0,
        "单舰叶片比舰队默认更具体"
    );

    // 3) 把叶片交回上层（Inherit）⇒ 又回到舰队默认的值。
    let diff = serde_json::json!({
        "control": [{"势力": "中国", "姿态": [{"舰": ship, "归属": "Inherit"}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("release applies");
    assert_eq!(
        state.ship_kiting(ship.clone()),
        -1.0,
        "叶 Inherit + 舰队默认 Player ⇒ 取默认值"
    );
}

/// **指令只剩逐舰叶**（2026-10 裁决）：没有叶 = 没有任何一层说话（调用方按 `Idle` 兜底），
/// 而**陈旧叶**就是有效值（不再有"舰队默认"或"图上意图"能把它盖掉）。
///
/// 这是 `default_ship_order` 与 `Blueprint::order` 两片叶删除之后的行为基线：
/// * 归属链：叶 → 势力 → 全局；
/// * 取值：叶里的值就是有效值；叶不存在 ⇒ `None`。
#[test]
fn orders_come_only_from_the_per_ship_leaf() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();

    // 拿一艘中国的舰、**删掉它的叶片**来模拟「刚下水、还没人点名」。
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("a chinese ship");
    state
        .control_mut(fid.clone())
        .expect("control")
        .ship_orders
        .remove(&ship);
    assert_eq!(
        state.ship_control(ship.clone()),
        ControlMode::Auto,
        "没有叶 ⇒ 归属沿作用域链走到 Auto（自动控制下一回合会给它写一条）"
    );
    assert_eq!(
        state.ship_behavior(ship.clone()),
        None,
        "没有叶 ⇒ 没有任何一层说话（调用方按 Idle 兜底）"
    );
    assert_eq!(state.ship_behavior_source(ship.clone()), None);

    // 逐舰写一条（写值即接管 = Player）：这就是**唯一**的下指令方式。
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "指令": [{"舰": ship.clone(), "行为": {"type": "dock", "body": "地球"}}]
        }]
    });
    apply_patch(&mut state, &config, &diff).expect("per-ship order applies");
    assert_eq!(
        state.ship_control(ship.clone()),
        ControlMode::Player,
        "写值即接管"
    );
    assert_eq!(
        state.ship_behavior(ship.clone()),
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        })
    );
    assert_eq!(
        state.ship_behavior_source(ship.clone()),
        Some(OrderSource::Leaf),
        "出处就是那片叶"
    );

    // 读面即写面：`--control` 里看得见它，且值能原样回传。
    let view = control_view(
        &state,
        &config,
        fid.clone(),
        state.control(fid.clone()).expect("control"),
    );
    let row = view
        .ship_orders
        .iter()
        .find(|o| o.ship == ship)
        .expect("逐舰指令在读面里");
    assert_eq!(row.mode, ControlMode::Player);
    assert_eq!(
        row.behavior,
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        })
    );
}

/// **舰队级那一片"默认指令"已经不存在**：写它必须**响亮**失败（`deny_unknown_fields`），
/// 而不是被静默吞掉（"失败不能看起来像成功"）。
#[test]
fn the_fleet_default_order_leaf_is_gone() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "default_ship_order": {"行为": {"type": "idle"}}
        }]
    });
    assert!(
        apply_patch(&mut state, &config, &diff).is_err(),
        "未知字段必须报错（旧的舰队默认指令驱动脚本会立刻发现自己过时了）"
    );
}

/// **角色轴**与另两条风格轴共用控制叶写值/归属契约；旧的 `"删叶"` 键在 API 边界统一拒绝。
#[test]
fn the_role_axis_uses_the_control_leaf_contract_without_deletion() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");
    for s in state.ships.iter_mut().filter(|s| s.faction_id == fid) {
        s.role = ShipRole::Freight;
    }

    // 写逐舰角色叶 = 玩家定活（写值即接管）。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "角色": [{"舰": ship, "角色": "War"}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(state.ship_role(ship.clone()), ShipRole::War);
    assert_eq!(state.ship_role_control(ship.clone()), ControlMode::Player);

    // 交回自动定编：写 `归属: Auto`（不是删叶）；AI 下回合可以再写值。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "角色": [{"舰": ship, "归属": "Auto"}]}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(state.ship_role_control(ship.clone()), ControlMode::Auto);

    // 旧删叶键：响亮失败，且不改变上面那片叶。
    let before = state.ship_role_control(ship.clone());
    let diff = serde_json::json!({
        "control": [{"势力": fid, "角色": [{"舰": ship, "删叶": true}]}]
    });
    let err = apply_patch(&mut state, &config, &diff).unwrap_err();
    assert!(err.contains("删叶") && err.contains("已删除"), "{err}");
    assert_eq!(state.ship_role_control(ship.clone()), before);
}

/// **两轴叶的"新建"必须两条轴一起给**：`default_doctrine` 只给一条轴的话，另一条会静默
/// 变成 `0.0`（= 基线），而 `0.0` 是个正常取值——事后从读面完全看不出来全舰队的风格被改了。
/// 叶**已存在**时单轴写仍然合法（那时"缺省 = 保留现值"是真的）。
#[test]
fn a_two_axis_fleet_default_must_be_created_with_both_axes() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();

    // ① 叶还不存在 + 只给一条轴 ⇒ 拒绝，并**不许留下半片叶**。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "舰队默认风格": {"temper": 0.4}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(r.skipped.len(), 1, "{:?}", r);
    assert_eq!(r.skipped[0].code, "partial_doctrine_leaf");
    assert!(
        r.skipped[0].reason.contains("两条轴"),
        "拒绝理由要给改法：{}",
        r.skipped[0].reason
    );
    assert!(
        state
            .control
            .get(&fid)
            .and_then(|c| c.default_doctrine.as_ref())
            .is_none(),
        "被拒绝的补丁不许留下半片叶"
    );

    // ② 只写 `mode`（先表态归属）合法。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "舰队默认风格": {"归属": "Player"}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean() && r.took_over.is_empty(), "只写 mode 不是接管：{:?}", r.took_over);
    let leaf = state.control[&fid].default_doctrine.clone().expect("叶建出来了");
    assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.0, 0.0));

    // ③ 叶已存在 ⇒ 单轴写合法，缺省轴保留现值。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "舰队默认风格": {"lone_wolf": -0.5}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean(), "{:?}", r.skipped);
    let leaf = state.control[&fid].default_doctrine.clone().expect("叶还在");
    assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.0, -0.5));

    // ④ 旧删叶键拒绝；叶保持不动。
    let err = apply_patch(
        &mut state,
        &config,
        &serde_json::json!({
            "control": [{"势力": fid, "舰队默认风格": {"删叶": true}}]
        }),
    )
    .unwrap_err();
    assert!(err.contains("删叶") && err.contains("已删除"), "{err}");
    assert!(state.control[&fid].default_doctrine.is_some(), "叶必须还在");
}

/// 逐舰**两轴叶**的单轴写：缺的那条轴种的是**这艘舰当时在用的那一条**——没有舰队默认时
/// 正是舰上记录值，有玩家默认时是默认值（界面上显示的就是它）。**绝不是一个
/// 凭空来的 `0.0`**（那正是 §3.1 那个坑的形态）。
#[test]
fn a_single_axis_ship_leaf_seeds_the_other_axis_from_what_is_in_use() {
    let config = crate::config::load_config();
    let fid = "中国".to_string();

    // 场景 A：没有舰队默认 ⇒ 缺省轴 = 舰上记录值。
    let mut state = crate::world::default_state(&config, 42);
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");
    for s in state.ships.iter_mut().filter(|s| s.faction_id == fid) {
        s.doctrine = ShipDoctrine { temper: 0.71, lone_wolf: -0.2 };
    }
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "temper": 0.5}]}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let leaf = state.control[&fid].ship_doctrine[&ship].clone();
    assert_eq!(
        (leaf.value.temper, leaf.value.lone_wolf),
        (0.5, -0.2),
        "缺省轴要种舰上记录值，不是 0.0"
    );

    // 场景 B：叶不存在、舰队默认是玩家表态 ⇒ 缺省轴 = 当时在用的默认值。
    let mut state = crate::world::default_state(&config, 42);
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");
    let diff = serde_json::json!({
        "control": [{"势力": fid,
            "舰队默认风格": {"temper": 0.1, "lone_wolf": 0.9, "归属": "Player"}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "temper": 0.5}]}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let leaf = state.control[&fid].ship_doctrine[&ship].clone();
    assert_eq!(
        (leaf.value.temper, leaf.value.lone_wolf),
        (0.5, 0.9),
        "船正在跟随舰队默认 ⇒ 另一条轴种的是默认值（UI 上显示的数）"
    );
}
