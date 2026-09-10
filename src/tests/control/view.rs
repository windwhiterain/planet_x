//! 读面：控制模板整面回传（含「不四舍五入」）、`ship_orders` 每舰一行且是不动点、`behavior` 为 null 的行不建叶。

use super::*;

/// 读面回传必须仍然合法：web 的 `POST /api/command` 把**整面**
/// `FactionControlView` 发回来，所以 `deny_unknown_fields` 不能把模板自己的
/// 键判成非法。（读面键集 ⊆ 写面键集。）
///
/// 这里刻意**逐字模仿前端**：`web/static/app.js` 拿到 `world.control` 后
/// `structuredClone` 一份并给每个势力补 `buildings = c.buildings || []`，
/// 发回来的就是「读面 + buildings」。少了这一步，守卫就测不到真实载荷。
#[test]
fn the_control_template_round_trips_back_through_apply() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let mut surface = control_surface(&state, &config);
    for fac in surface["control"].as_array_mut().expect("control is an array") {
        fac.as_object_mut().expect("faction is an object").insert("buildings".to_string(), serde_json::json!([]));
    }
    // 整面回传：应当被接受，且没有任何叶片被丢。
    let report = apply_patch(&mut state, &config, &surface).expect("the web payload must round-trip");
    assert!(
        report.is_clean(),
        "the editable template must be a valid diff ({{}}): {:?}",
        report.skipped
    );
    assert!(report.applied >= 40, "the whole template should touch many leaves, got {}", report.applied);
}

/// **指令读面：每舰一行 + 是一处不动点。**
///
/// 三条契约（`control-live-layers.md` §10.5 的读面缺口 + 本轮新增的一节）：
/// 1. 本势力**每一艘舰**都有一行——包括**从没被点名过**（连叶都没有）的舰；
/// 2. `behavior` 是**有效值**（链上没人说话时是 `null`），`mode` 是**叶自己的表态**
///    （没有叶 = `Inherit`）；
/// 3. 把整面模板**原样回传**再读一次，JSON **逐字节相同**（同一份状态、同一份模板）。
#[test]
fn the_order_read_face_lists_every_ship_and_is_a_fixed_point() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();

    // 三种情形，正好覆盖取值链的三个分支：
    //  ① 从没被点名过的舰（连叶都没有）——`remove: true` 之后就是这样；
    //  ② 有叶、叶说 `Inherit`（AI 每回合写的流水）——有效值来自叶里那个记录值；
    //  ③ 有叶、叶是 `Player`（玩家钉的）。
    let ours: Vec<ShipId> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .collect();
    assert!(ours.len() >= 3, "中国开局至少 3 艘舰（实际 {}）", ours.len());
    let (unnamed, inherit_leaf, player_leaf) =
        (ours[0].clone(), ours[1].clone(), ours[2].clone());
    let c = state.control_mut(fid.clone()).expect("control");
    c.ship_orders.remove(&unnamed);
    c.ship_orders.insert(
        inherit_leaf.clone(),
        Control {
            value: ShipBehavior::Move { position: [1.5, -2.0] },
            mode: ControlMode::Inherit,
        },
    );
    c.ship_orders.insert(
        player_leaf.clone(),
        Control { value: ShipBehavior::Dock { body: "地球".to_string() }, mode: ControlMode::Player },
    );

    let row = |s: &serde_json::Value, ship: &str| -> serde_json::Value {
        s["control"]
            .as_array()
            .expect("control 是数组")
            .iter()
            .find(|f| f["faction_id"] == serde_json::json!(fid))
            .expect("控制面里必须有这个势力")["ship_orders"]
            .as_array()
            .expect("ship_orders 是数组")
            .iter()
            .find(|r| r["ship"] == serde_json::json!(ship))
            .unwrap_or_else(|| panic!("读面里必须有「{ship}」这一行"))
            .clone()
    };

    let surface = control_surface(&state, &config);
    let rows = surface["control"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["faction_id"] == serde_json::json!(fid))
        .unwrap()["ship_orders"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(rows, ours.len(), "指令读面必须**每舰一行**（含没有叶的舰）");

    // ① 没有叶、链上没人说话 ⇒ `behavior: null` + `mode: Inherit`。
    assert_eq!(row(&surface, &unnamed), serde_json::json!({
        "ship": unnamed, "behavior": serde_json::Value::Null, "mode": "Inherit",
    }));
    // ② 叶在、叶说 Inherit ⇒ 值取叶里的记录值，表态照实报 Inherit。
    assert_eq!(row(&surface, &inherit_leaf), serde_json::json!({
        "ship": inherit_leaf, "behavior": {"Move": {"position": [1.5, -2.0]}}, "mode": "Inherit",
    }));
    // ③ 叶在、叶是 Player ⇒ 值取叶值、表态是 Player。
    assert_eq!(row(&surface, &player_leaf), serde_json::json!({
        "ship": player_leaf, "behavior": {"Dock": {"body": "地球"}}, "mode": "Player",
    }));

    // **不动点**：整面模板原样回传，再读一次必须逐字节相同。
    let before = surface.to_string();
    let report = apply_patch(&mut state, &config, &surface).expect("模板必须能原样回传");
    assert!(report.is_clean(), "模板回传不许丢叶子: {:?}", report.skipped);
    assert_eq!(
        control_surface(&state, &config).to_string(),
        before,
        "读面不是不动点：回传模板改变了它自己的形状"
    );
    // ① 那一行**没有**变出一片叶来（否则「链上没人说话」会被静默变成「叶里记着 Idle」，
    //    `behavior` 也就不再是 `null` 了——上面那条逐字节断言正是靠这条规则才成立）。
    assert!(
        !state.control(fid.clone()).unwrap().ship_orders.contains_key(&unnamed),
        "`behavior: null` 的行不许建叶"
    );

    // 舰队默认（势力级 Player）压过「叶说 Inherit」：② 的有效值换成默认值，
    // ①（没有叶）也一样；③ 仍然是自己钉的那个值。回传之后仍然是不动点。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid,
            "default_ship_order": {"behavior": {"type": "colonize", "body": "火星"}, "mode": "Player"}
        }]
    });
    apply_patch(&mut state, &config, &diff).expect("舰队默认落地");
    let surface = control_surface(&state, &config);
    assert_eq!(row(&surface, &unnamed)["behavior"], serde_json::json!({"Colonize": {"body": "火星"}}));
    assert_eq!(row(&surface, &inherit_leaf)["behavior"], serde_json::json!({"Colonize": {"body": "火星"}}));
    assert_eq!(row(&surface, &player_leaf)["behavior"], serde_json::json!({"Dock": {"body": "地球"}}));
    let before = surface.to_string();
    apply_patch(&mut state, &config, &surface).expect("模板必须能原样回传");
    assert_eq!(control_surface(&state, &config).to_string(), before, "有了舰队默认之后读面仍须是不动点");
}

/// 读面的 `behavior: null` 与「叶不存在」是**同一件事**的两面，所以回传它**不许建叶**；
/// 而 `mode` 表态（`Auto`/`Player`）与写值照旧建叶——那才是「给这艘舰设归属」的动作。
///
/// 为什么值得单独立一条：建叶规则错一格的后果不是报错，而是**有效值静默改变**
/// （`null` → `Some(Idle)`），而两次 `--control` 的差异只有那一格。
#[test]
fn a_null_behavior_row_never_invents_a_leaf() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let has_leaf = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国有一艘舰");
    let no_leaf = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .nth(1)
        .expect("中国有第二艘舰");
    state.control_mut(fid.clone()).unwrap().ship_orders.remove(&no_leaf);

    // ① 读面那一行原样回传 ⇒ 幂等成功，但**不建叶**。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid,
            "ship_orders": [{"ship": no_leaf, "behavior": null, "mode": "Inherit"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("null 行回传必须被接受");
    assert!(report.is_clean(), "{:?}", report.skipped);
    assert_eq!(report.applied, 1, "「这一层没有说话」也是达成了目标状态（幂等成功）");
    assert!(
        !state.control(fid.clone()).unwrap().ship_orders.contains_key(&no_leaf),
        "读面那一行说的就是「没有叶」，回传它当然不许建叶"
    );
    assert_eq!(state.ship_behavior(no_leaf.clone()), None, "有效值仍然是「没人说话」");

    // ② 只写 `mode`（哪怕写的是 `Inherit`）而**叶已经存在** ⇒ 值不动。
    let before = state.ship_behavior(has_leaf.clone());
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_orders": [{"ship": has_leaf, "mode": "Inherit"}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("只写 mode 落地");
    assert_eq!(state.ship_behavior(has_leaf.clone()), before, "只写表态不许动值");

    // ③ `mode: Auto`（表态）与写值仍然建叶 —— 那正是「给这艘没有叶的舰设归属」。
    for (i, patch) in [serde_json::json!({"ship": no_leaf, "mode": "Auto"}),
                       serde_json::json!({"ship": no_leaf, "behavior": "Idle", "mode": "Inherit"})]
        .into_iter()
        .enumerate()
    {
        let mut s = state.clone();
        let diff = serde_json::json!({"control": [{"faction_id": fid, "ship_orders": [patch]}]});
        apply_patch(&mut s, &config, &diff).expect("建叶");
        assert!(
            s.control(fid.clone()).unwrap().ship_orders.contains_key(&no_leaf),
            "第 {i} 条补丁（表态/写值）必须建出那片叶 —— 否则「设归属」这条唯一的入口就断了"
        );
    }
}

/// 「读面即写面、模板原样回传安全」是一条**可检查**的承诺：读面里出现的数字必须**逐位**
/// 等于状态里存着的那个数 —— 否则"原样回传"就成了一次没人要求的写操作。
///
/// 历史：这里曾把所有数值四舍五入到 2 位小数（为了 token 干净），于是 `0.7131` 显示成
/// `0.71`、回传后**真的**变成 `0.71`。这条守卫就是那次教训的化身：三个"舍入会改变它"的值，
/// 落在三种不同的叶上（势力级默认风格 / 逐舰风格 / 资源预算）。
#[test]
fn the_control_template_never_rounds_a_leaf_value() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");
    let noisy = 0.7131_f64;
    let diff = serde_json::json!({
        "control": [{
            "faction_id": fid,
            // 两轴一片叶：这片叶还不存在，必须两条轴一起给（否则 `partial_doctrine_leaf` 拒绝）。
            "default_doctrine": {"temper": noisy, "lone_wolf": 0.0},
            "ship_kiting": [{"ship": ship, "kiting": noisy}],
            "investment_budget": [{"resource": "铁", "value": noisy}]
        }]
    });
    apply_patch(&mut state, &config, &diff).expect("diff applies");

    let surface = control_surface(&state, &config);
    let fac = surface["control"]
        .as_array()
        .expect("control 是数组")
        .iter()
        .find(|f| f["faction_id"] == serde_json::json!(fid))
        .expect("控制面里必须有这个势力")
        .clone();
    let exact = serde_json::json!(noisy);
    assert_eq!(fac["default_doctrine"]["temper"], exact, "势力级默认风格被舍入了");
    let kite = fac["ship_kiting"]
        .as_array()
        .expect("ship_kiting 是数组")
        .iter()
        .find(|k| k["ship"] == serde_json::json!(ship))
        .expect("刚写过的那艘舰必须在读面里");
    assert_eq!(kite["kiting"], exact, "逐舰风筝距离被舍入了");
    let budget = fac["investment_budget"]
        .as_array()
        .expect("investment_budget 是数组")
        .iter()
        .find(|b| b["resource"] == serde_json::json!("铁"))
        .expect("刚写过的资源预算必须在读面里");
    assert_eq!(budget["value"], exact, "投资预算被舍入了");
    // 而且它必须就是状态里真的存着的那个数（读面 = 真值，不是"看起来像"）。
    assert_eq!(state.ship_kiting(ship), noisy);
}
