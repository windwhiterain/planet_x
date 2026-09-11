//! 设计图叶片：skip 码要逐条报出、写值即接管、模板整面回传、悬空指针**响亮报错**、舰级与船坞要一起改、静默指令轴 ≠ 删图、归属沿 scope 链。

use super::*;

/// 设计图写面的**每一个丢弃码**都点名到叶（§7.3-9 / §4.6）。
#[test]
fn blueprint_patch_reports_every_skip_code() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let (cid, bid, _) = some_building(&state, "中国", true);
    let (rcid, rbid, _) = some_building(&state, "中国", false);
    let skip = |rep: &ApplyReport, code: &str| {
        let hit = rep
            .skipped
            .iter()
            .find(|s| s.code == code)
            .unwrap_or_else(|| panic!("要报 {code}，实际 {:?}", rep.skipped));
        assert!(
            hit.path.contains("设计图"),
            "{code} 要点名到叶，got {}",
            hit.path
        );
        assert!(!hit.reason.is_empty(), "{code} 要有一句人读的理由");
    };

    // ① 建造区指向一张**不存在**的图（悬空指针）。
    let diff = serde_json::json!({"control": [{"势力": "中国", "建筑": [
        {"城": cid, "建筑": bid, "设计图": "没有这张图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_blueprint");
    assert_eq!(
        state
            .city(&cid)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .unwrap()
            .blueprint,
        None,
        "被丢弃的指针不许落地"
    );

    // ② 图名不存在 + 只写 mode ⇒ 同样 `no_such_blueprint`（不许凭空造图）。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "没有这张图", "归属": "Auto"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_blueprint");
    assert!(
        !state.control["中国"].blueprints.contains_key("没有这张图"),
        "错别字不许造出一张谁都不认识的图（防幽灵图）"
    );

    // ③ 舰级对不上：图是 cruiser，建造区是 corvette。
    let (cid2, bid2, st2) = some_building(&state, "中国", true);
    let diff = serde_json::json!({"control": [{"势力": "中国",
        "设计图库": [{"图名": "巡洋图", "舰级": "cruiser", "选装": [], "归属": "Player"}],
        "建筑": [{"城": cid2, "建筑": bid2, "设计图": "巡洋图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    // 建图本身落地了（`applied`），只有指针被丢。
    skip(&rep, "blueprint_class_mismatch");
    assert!(st2.is_empty() || state.control["中国"].blueprints.contains_key("巡洋图"));
    assert_eq!(
        state
            .city(&cid2)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid2)
            .unwrap()
            .blueprint,
        None,
        "对不上的指针不许落地（口径 A）"
    );

    // ④ 不存在的组件。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "坏图", "舰级": "corvette", "选装": ["没有这个组件"], "归属": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_component");
    assert!(
        !state.control["中国"].blueprints.contains_key("坏图"),
        "被拒的图不许污染库"
    );

    // ⑤ 组件重复。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "双炮图", "舰级": "corvette", "选装": ["kinetic", "kinetic"], "归属": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "duplicate_component");
    assert!(!state.control["中国"].blueprints.contains_key("双炮图"));

    // ⑥ 超过槽位（corvette 只有 2 个槽）。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "超载图", "舰级": "corvette",
         "选装": ["kinetic", "ion_drive", "shield"], "归属": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "too_many_components");
    assert!(!state.control["中国"].blueprints.contains_key("超载图"));

    // ⑦ 建图没给舰级 / 舰级不存在。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "无级图", "选装": ["kinetic"], "归属": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "missing_class");
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "怪级图", "舰级": "无畏舰", "选装": [], "归属": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_class");

    // ⑧ 把图挂到**非建造区**上。
    let diff = serde_json::json!({"control": [{"势力": "中国",
        "设计图库": [{"图名": "民用图", "舰级": "corvette", "选装": [], "归属": "Player"}],
        "建筑": [{"城": rcid, "建筑": rbid, "设计图": "民用图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "not_a_shipyard");
}

/// **写值即接管**：只写 `components` ⇒ 图叶变 `Player` 并记一笔（§7.3-11）。
#[test]
fn writing_a_blueprint_value_without_mode_takes_over() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "我的图", "舰级": "corvette", "选装": ["kinetic", "ion_drive"]}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    let leaf = state.control["中国"].blueprints["我的图"].clone();
    assert_eq!(
        leaf.mode,
        ControlMode::Player,
        "只写值 ⇒ 这一层接管（免得「我写了图却没生效」）"
    );
    assert!(
        rep.took_over.iter().any(|p| p.contains("设计图库[0]")),
        "{:?}",
        rep.took_over
    );
    assert_eq!(
        state.blueprint_control(&"中国".to_string(), &"我的图".to_string()),
        ControlMode::Player
    );

    // 只写 `mode` 合法（值不动）——交回系统重估。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "我的图", "归属": "Auto"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    let leaf = state.control["中国"].blueprints["我的图"].clone();
    assert_eq!(leaf.mode, ControlMode::Auto);
    assert_eq!(
        leaf.value.components,
        vec!["kinetic".to_string(), "ion_drive".to_string()],
        "值不动"
    );
}

/// **读面即写面**：图库非空时，整面模板回传仍然合法，`ship_count` 这种只读列不许炸写面。
#[test]
fn the_blueprint_template_round_trips_back_through_apply() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    pin_blueprint(
        &mut state,
        &config,
        "护卫-守家",
        "corvette",
        &serde_json::json!(["kinetic", "ion_drive"]),
        "Player",
    );
    // 给它造一艘舰，让 `ship_count` 有个非零值（读面附加列）。
    let pos = state.body_position("地球");
    let bp = "护卫-守家".to_string();
    crate::sim::spawn_ship(
        &mut state,
        &config,
        crate::sim::ShipSpawn {
            owner: "中国".to_string(),
            class: "corvette",
            position: pos,
            city: None,
            via: SpawnVia::Shipyard,
            pay_components: false,
            blueprint: Some(&bp),
        },
    );

    let mut surface = control_surface(&state, &config);
    let row = surface["control"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["势力"] == "中国")
        .and_then(|f| f["设计图库"].as_array())
        .and_then(|b| b.first())
        .cloned()
        .expect("读面必须给出蓝图片");
    assert_eq!(
        row["选装"],
        serde_json::json!(["kinetic", "ion_drive"]),
        "选装要**全量**输出（少输出 = 回传时清空）"
    );
    assert_eq!(
        row["ship_count"],
        serde_json::json!(1),
        "读面附加：本图造了多少艘"
    );
    assert_eq!(
        row["launch_waiting"],
        serde_json::json!(false),
        "读面附加：这张图此刻没人在等钱"
    );
    assert!(
        row["order"].is_null(),
        "本图对意图没有说话 ⇒ null（不是缺字段）"
    );

    for fac in surface["control"].as_array_mut().unwrap() {
        fac.as_object_mut()
            .unwrap()
            .insert("建筑".to_string(), serde_json::json!([]));
    }
    let rep = apply_patch(&mut state, &config, &surface).expect("模板回传必须合法");
    assert!(rep.is_clean(), "模板回传不许丢叶：{:?}", rep.skipped);
    assert!(
        rep.applied >= 40,
        "整面模板要触碰很多叶，got {}",
        rep.applied
    );
    let leaf = state.control["中国"].blueprints["护卫-守家"].clone();
    assert_eq!(leaf.mode, ControlMode::Player);
    assert_eq!(leaf.value.components.len(), 2, "回传不改变选装");
}

/// **悬空指针**（图被删掉之后）：apply 报 `no_such_blueprint`，读面**原样输出**指针（Q10(a)）。
#[test]
fn a_dangling_blueprint_pointer_is_reported_not_silently_ignored() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let (cid, bid) = pin_blueprint(
        &mut state,
        &config,
        "会被删的图",
        "corvette",
        &serde_json::json!(["kinetic", "ion_drive"]),
        "Player",
    );
    // 删掉整张图（挂它的建造区**不会**被自动改指针：那是玩家的话，引擎不替他猜）。
    let diff = serde_json::json!({"control": [{"势力": "中国",
        "设计图库": [{"图名": "会被删的图", "删叶": true}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    assert!(
        rep.removed.iter().any(|p| p.contains("设计图库")),
        "{:?}",
        rep.removed
    );
    assert!(!state.control["中国"].blueprints.contains_key("会被删的图"));

    // 读面（web/--control 的 `buildings` 是结构补丁面，指针在 state 里读）**原样**输出。
    let b = state
        .city(&cid)
        .unwrap()
        .buildings
        .iter()
        .find(|b| b.id == bid)
        .unwrap();
    assert_eq!(
        b.blueprint.as_deref(),
        Some("会被删的图"),
        "指针原样保留（读面据此看出「这个区指着不存在的图」）"
    );

    // 再有人写这个指针 ⇒ 响亮报出来。
    let diff = serde_json::json!({"control": [{"势力": "中国", "建筑": [
        {"城": cid, "建筑": bid, "设计图": "会被删的图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(
        rep.skipped[0].code, "no_such_blueprint",
        "{:?}",
        rep.skipped
    );

    // 拆指针是**另一件事**（回到 `ship_type` + 生成器）：`null` 与缺席必须分得开。
    let diff = serde_json::json!({"control": [{"势力": "中国", "建筑": [
        {"城": cid, "建筑": bid, "设计图": null}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    assert_eq!(
        state
            .city(&cid)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .unwrap()
            .blueprint,
        None,
        "`\"blueprint\": null` = 拆掉指针（缺席才是「不动」）"
    );
    assert_ne!(rep.applied, 0, "拆指针是一次落地");
}

/// 口径 A 的**双向守卫**：图与建造区的舰级要一起写；只写一处必须**响亮**被拒（§9.3）。
#[test]
fn blueprint_and_yard_class_must_be_changed_together() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let (cid, bid) = pin_blueprint(
        &mut state,
        &config,
        "护卫图",
        "corvette",
        &serde_json::json!(["kinetic", "ion_drive"]),
        "Player",
    );

    // ① 只改图 ⇒ 拒绝（否则这张图对不上它自己的建造区）。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "护卫图", "舰级": "cruiser"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(
        rep.skipped[0].code, "blueprint_class_mismatch",
        "{:?}",
        rep.skipped
    );
    assert_eq!(
        state.control["中国"].blueprints["护卫图"].value.class, "corvette",
        "被拒 ⇒ 状态不动"
    );

    // ② 只改建造区 ⇒ 同样拒绝（另一条路，堵一条没用）。
    let diff = serde_json::json!({"control": [{"势力": "中国", "建筑": [
        {"城": cid, "建筑": bid, "建造舰级": "cruiser"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(
        rep.skipped[0].code, "blueprint_class_mismatch",
        "{:?}",
        rep.skipped
    );
    assert_eq!(
        state
            .city(&cid)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .unwrap()
            .ship_type
            .as_deref(),
        Some("corvette"),
        "被拒 ⇒ 建造区不动"
    );

    // ③ **两处一起写** ⇒ 一次成功（正解）。
    let diff = serde_json::json!({"control": [{"势力": "中国",
        "设计图库": [{"图名": "护卫图", "舰级": "cruiser", "选装": []}],
        "建筑": [{"城": cid, "建筑": bid, "建造舰级": "cruiser"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "两处一起写必须一次成功：{:?}", rep.skipped);
    assert_eq!(
        state.control["中国"].blueprints["护卫图"].value.class,
        "cruiser"
    );
    assert_eq!(
        state
            .city(&cid)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .unwrap()
            .ship_type
            .as_deref(),
        Some("cruiser")
    );
}

/// 「让图的**某条倾向轴**沉默」（`role: null`）与「删掉这张图」（`remove: true`）是两件事。
#[test]
fn silencing_a_stance_axis_is_not_deleting_the_blueprint() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let (cid, bid) = pin_blueprint(
        &mut state,
        &config,
        "护卫-守家",
        "corvette",
        &serde_json::json!(["kinetic", "ion_drive"]),
        "Player",
    );
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "护卫-守家", "角色": "Freight"}]}]});
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(
        state.control["中国"].blueprints["护卫-守家"].value.role,
        Some(ShipRole::Freight),
        "图上的倾向轴要写得进去（这里是角色：这张图造的舰一出来就跑运输）"
    );

    // 清空这条轴：图还在、指针还在，只是这一层不再说话。
    let diff = serde_json::json!({"control": [{"势力": "中国", "设计图库": [
        {"图名": "护卫-守家", "角色": null}]}]});
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(
        state.control["中国"].blueprints["护卫-守家"].value.role, None,
        "这条轴沉默了"
    );
    assert!(
        state.control["中国"].blueprints.contains_key("护卫-守家"),
        "图还在"
    );
    assert_eq!(
        state
            .city(&cid)
            .unwrap()
            .buildings
            .iter()
            .find(|b| b.id == bid)
            .unwrap()
            .blueprint
            .as_deref(),
        Some("护卫-守家"),
        "指针还在（= 选装仍按图装配，只有倾向那一层交还给下层）"
    );
}

/// 图的**有效归属**沿 scope 链上溯（图叶 → 势力 → 全局）：势力设成 `Player` 也能接管。
#[test]
fn blueprint_ownership_follows_the_scope_chain() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    state
        .control
        .entry("中国".to_string())
        .or_default()
        .blueprints
        .insert(
            "种子图".to_string(),
            Control::inherit(Blueprint {
                class: "corvette".to_string(),
                components: vec![],
                doctrine: None,
                kiting: None,
                role: None,
            }),
        );
    let fid = "中国".to_string();
    assert_eq!(
        state.blueprint_control(&fid, &"种子图".to_string()),
        ControlMode::Auto,
        "全链继承 ⇒ Auto"
    );
    let diff = serde_json::json!({"scope": {"factions": [["中国", "Player"]]}});
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(
        state.blueprint_control(&fid, &"种子图".to_string()),
        ControlMode::Player,
        "势力的 scope 表态也要能接管设计图（与其它叶同一条链的语义）"
    );
}
