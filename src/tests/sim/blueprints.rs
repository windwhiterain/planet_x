//! 设计图（`--apply` 的 blueprint 叶）与船坞下水：出厂快照、图上的**倾向**、`order_source`、悬空指针停产、买不起就不下水。夹具 `attach_blueprint`/`spawn_at`/`stock` 在 `super`。

//! ## 2026-10：能只看数据的那几条搬去了 `play/tests/g2_mid.py`
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `editing_a_blueprint_does_not_touch_existing_ships` / `retuning_a_design_never_touches_ships_already_in_space` | g2「**出厂快照不随时间变**」（一条舰的选装一生恒定，400 条舰零漂移） | `ships.components` 与 `blueprints.components` 都在读面上 |
//! | `ship_spawned_event_carries_the_blueprint_only_when_there_is_one` | g2「造舰事件的图归因与舰表一致」（374 条事件） | 事件层 `ship_spawned.data.blueprint` ↔ 舰表 `blueprint` |
//! | `designs_are_deduped_by_class_and_signature` | g2「图按 `(舰级, 选装)` 去重」（18,691 个签名） | 图库表逐回合可查 |
//! | `a_class_drift_between_the_yard_and_its_design_is_reconciled` | g2「建造区挂了图就必须挂到存在的图上」+「舰级不符只是**滞后**、会自己收敛」（16,709 个建造区·回合；实测 3 行不符、最长滞后 1 回合） | `cities.buildings[].{ship_type,blueprint}` + 图库表 |
//!
//! **留在这里的**（这一族住在 `src/tests/autocontrol/blueprints.rs`，本文件是船坞下水那一侧）：
//! `the_ai_creates_a_design_for_every_yard_it_owns`——它要「每个区的**有效**归属」（读面只有
//! 逐个区自己的 `blueprint` 指针，判不出「AI 该不该给它建图」）。另外三条（玩家钉住的图 /
//! 悬空指针 / 回收只碰自己造的）已随第 6 批搬去 g2 的**合成场景 · 拨控制叶**（施工图 §5.6）。

use super::*;

/// **出厂快照**（§7.1-1）：船坞挂了 `Player` 图 ⇒ 下水那艘舰的选装就是图上的选装。
#[test]
fn spawn_uses_the_yard_blueprint() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    stock(&mut state, &config, &fid, 400.0);
    let (cid, bid, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "重甲护卫",
        "corvette",
        &["kinetic", "ion_drive"],
        (None, None, None),
        ControlMode::Player,
    );
    let mut rng = Prng::new(42);
    advance(&mut state, &config, &mut rng);

    let ship = state
        .ships
        .iter()
        .find(|s| s.blueprint.as_deref() == Some("重甲护卫"))
        .expect("挂了图的那个建造区必须产出一艘舰")
        .clone();
    assert_eq!(
        ship.components,
        vec!["kinetic".to_string(), "ion_drive".to_string()],
        "选装 = 图上的选装（顺序也照图）"
    );
    assert_eq!(ship.class, class);
    assert_eq!(
        ship.hull_max,
        ship_panel(&config, &ship).hull_max,
        "面板是快照，且与 config 现算的面板一致"
    );
    assert_eq!(
        ship.spawned_round,
        Some(state.round),
        "下水回合要记下来（Q9）"
    );
    // 组件成本**真的**从库存里扣了：在同一份状态上再走一次「出厂 + 付款」，逐资源比对
    // （比「跟 400 比」可靠——那一回合里还有开采/市场/别处的建造在同一条库存上进账）。
    let mut probe = state.clone();
    let before = state.faction(&fid).unwrap().resources.clone();
    spawn_ship(
        &mut probe,
        &config,
        ShipSpawn {
            owner: fid.clone(),
            class: "corvette",
            position: state.body_position("地球"),
            city: None,
            via: SpawnVia::Shipyard,
            pay_components: true,
            blueprint: Some(&"重甲护卫".to_string()),
        },
    );
    let after = probe.faction(&fid).unwrap().resources.clone();
    let mut need: ResourceMap = ResourceMap::new();
    for comp in ["kinetic", "ion_drive"] {
        for (rt, cost) in &config.components[comp].cost {
            *need.entry(rt.clone()).or_insert(0.0) += *cost;
        }
    }
    for (rt, cost) in &need {
        let paid = before.get(rt).copied().unwrap_or(0.0) - after.get(rt).copied().unwrap_or(0.0);
        assert!(
            (paid - cost).abs() < 1e-9,
            "选装里 {rt} 的成本必须从库存里扣掉：应付 {cost}，实扣 {paid}"
        );
    }
    let _ = (cid, bid);
}

/// **图的选装是出厂规格**（本轮改掉的旧语义）：`components` 非空 ⇒ 出厂就按它装配，
/// **与图的归属无关**（归属只管"谁能改这张图"）。旧版只在图归 `Player` 时用它 ⇒ `Auto`
/// 图的选装被静默忽略，而 AI 建图那一层（`autocontrol::blueprints`）就永远造不出东西。
#[test]
fn the_yard_launches_from_any_designs_components() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    stock(&mut state, &config, &fid, 300.0);
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "auto:custom",
        "corvette",
        &["plasma", "ion_drive"],
        (None, None, None),
        ControlMode::Auto,
    );
    let site = state.stock_at(&fid, "地球").cloned().unwrap_or_default();
    assert_eq!(
        crate::autocontrol::resolve_loadout(
            &state,
            &config,
            fid.clone(),
            &class,
            Some(&"auto:custom".to_string()),
            &site
        ),
        vec!["plasma".to_string(), "ion_drive".to_string()],
        "`Auto` 图的选装必须真的生效（否则 AI 建图只是空头支票）"
    );
    let name = spawn_at(
        &mut state,
        &config,
        &fid,
        &class,
        "地球",
        Some("auto:custom"),
    );
    assert_eq!(
        state.ship(&name).unwrap().components,
        vec!["plasma".to_string(), "ion_drive".to_string()],
        "出厂用的是**图上的**选装（不再是现场生成的另一套）"
    );
    // 对照：空选装的图仍然走生成器（那是"交给生成器"的正式语义）。
    attach_blueprint(
        &mut state,
        &config,
        &fid,
        "auto:empty",
        "corvette",
        &[],
        (None, None, None),
        ControlMode::Auto,
    );
    let site = state.stock_at(&fid, "地球").cloned().unwrap_or_default();
    assert_eq!(
        crate::autocontrol::resolve_loadout(
            &state,
            &config,
            fid.clone(),
            &class,
            Some(&"auto:empty".to_string()),
            &site
        ),
        crate::autocontrol::choose_loadout(&state, &config, fid.clone(), &class),
        "空选装 = 交给生成器（与归属无关）"
    );
}

/// `Auto` 图（`components: []`）⇒ 出厂那一刻**现场**调 `choose_loadout`（§7.1-4）。
///
/// ⚠ 本用例测的是**空选装**那条路（`components` 为空 = 交给生成器，与归属无关）。
/// 「`Auto` 图的选装被**静默忽略**」那条旧语义本轮已经改掉：图 = 出厂规格、`mode` = 谁能改图
/// ⇒ `components` 非空时**任何**归属都按图装配（见 `the_yard_launches_from_any_designs_components`）。
#[test]
fn auto_blueprint_uses_choose_loadout_at_launch() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    stock(&mut state, &config, &fid, 300.0);
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "auto:corvette",
        "corvette",
        &[],
        (None, None, None),
        ControlMode::Auto,
    );
    // 同一时点的生成器答案（`spawn_ship` 内部就是走它——不许另写一份）。
    let site = state.stock_at(&fid, "地球").cloned().unwrap_or_default();
    let expected = crate::autocontrol::resolve_loadout(
        &state,
        &config,
        fid.clone(),
        &class,
        Some(&"auto:corvette".to_string()),
        &site,
    );
    assert_eq!(
        expected,
        crate::autocontrol::choose_loadout(&state, &config, fid.clone(), &class),
        "`Auto` 图的选装必须**就是** `choose_loadout` 的答案（不是第二份生成逻辑）"
    );
    let name = spawn_at(
        &mut state,
        &config,
        &fid,
        &class,
        "地球",
        Some("auto:corvette"),
    );
    assert_eq!(
        state.ship(&name).unwrap().components,
        expected,
        "出厂用的是当场算出来的选装"
    );

    // **没有提前缓存**：把库存掏空之后再算，生成器给不出完整选装——只剩**平台兜底**的那件
    // 推进器（「至少一件推进」是硬保证：没有推进器的舰速度 0、永远不能当运输舰，那是
    // 「完全禁止瞬移」下最容易踩的死亡螺旋，见 `choose_loadout_prefs` 的注释）。
    // 若选装是在回合步进里预生成的，这里就会拿到上一回合那份完整的选装。
    if let Some(f) = state.faction_mut(&fid) {
        for v in f.resources.values_mut() {
            *v = 0.0;
        }
    }
    let site = state.stock_at(&fid, "地球").cloned().unwrap_or_default();
    let empty_stock = crate::autocontrol::resolve_loadout(
        &state,
        &config,
        fid.clone(),
        &class,
        Some(&"auto:corvette".to_string()),
        &site,
    );
    assert!(
        empty_stock.len() < expected.len() && empty_stock.len() <= 1,
        "库存掏空 ⇒ 只剩平台兜底（证明它是**出厂那一刻**算的）：{empty_stock:?} vs {expected:?}"
    );
    assert!(
        empty_stock
            .iter()
            .all(|c| config.component_spec(c).category == "thrust"),
        "兜底只给**平台**（推进器）：买不起的军备绝不白送，实为 {empty_stock:?}"
    );
}

/// **图能表态的是倾向**（2026-10 裁决）：图上写了 `role` ⇒ 新舰一造出来就是那个角色，
/// 而**图比舰队默认更具体**（链：叶 → 图 → 舰队默认 → 记录值）。
#[test]
fn blueprint_role_governs_new_ships() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "运输-地球线",
        "corvette",
        &["kinetic", "ion_drive"],
        (None, None, Some(ShipRole::Freight)),
        ControlMode::Player,
    );
    // 舰队默认**同时**是 Player 且冲突 ⇒ **更具体的图赢**（叶 → 图 → 舰队默认 → …）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_role": {"role": "War"}
    }]});
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");

    let name = spawn_at(&mut state, &config, &fid, &class, "水星", Some("运输-地球线"));
    assert_eq!(
        state.ship_role(name.clone()),
        ShipRole::Freight,
        "图比舰队默认更具体 ⇒ 图赢"
    );
    assert_eq!(
        state.ship_role_control(name.clone()),
        ControlMode::Player,
        "图上写了这条轴、且图归玩家 ⇒ 这条轴归玩家，自动控制不许改写它"
    );
    // 反例：**换一条轴**——图上没写风格，风格就仍由舰队默认作答（逐轴独立）。
    assert_eq!(
        state.ship_doctrine(name.clone()),
        ShipDoctrine::default(),
        "图上没写风格 ⇒ 这一轴跟着链往下走（这里没有舰队默认风格，落到记录值）"
    );
}

/// 回归守卫：**图对某条轴沉默**（`None`）时，那条轴仍由舰队默认作答
/// （新层不许把旧行为吃掉——"建图 ≠ 表态"）。
#[test]
fn fleet_default_still_covers_blueprintless_ships() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    // 一张**只钉选装**（三条倾向轴全沉默）的图。
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "只钉选装",
        "corvette",
        &["kinetic", "ion_drive"],
        (None, None, None),
        ControlMode::Player,
    );
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_role": {"role": "Freight"}
    }]});
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");

    let with_bp = spawn_at(&mut state, &config, &fid, &class, "水星", Some("只钉选装"));
    let without_bp = spawn_at(&mut state, &config, &fid, &class, "水星", None);
    for (ship, what) in [(&with_bp, "挂了图但图没表态"), (&without_bp, "没有图")] {
        assert_eq!(
            state.ship_role(ship.clone()),
            ShipRole::Freight,
            "{what} 的舰仍由舰队默认作答"
        );
        assert_eq!(
            state.ship_role_control(ship.clone()),
            ControlMode::Player,
            "{what} 的归属仍由舰队默认叶决定（图沉默 ⇒ 建图 ≠ 表态）"
        );
    }
}

/// **指令只剩逐舰叶**（2026-10 裁决）：图与舰队默认**都不再**指挥指令；
/// `order_source` 仍要把「叶不存在」与「叶写着 `Inherit`」分开报。
#[test]
fn order_source_separates_a_missing_leaf_from_a_silent_one() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    // 一张**写得满满的**图：三轴全表态、归属 Player——它**依然**不能供指令。
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "全能图",
        "corvette",
        &["kinetic", "ion_drive"],
        (
            Some(ShipDoctrine {
                temper: 0.5,
                lone_wolf: -0.5,
            }),
            Some(0.7),
            Some(ShipRole::War),
        ),
        ControlMode::Player,
    );
    let name = spawn_at(&mut state, &config, &fid, &class, "水星", Some("全能图"));

    // ① 刚下水的舰：船坞给它写一片**沉默的**（`Inherit`）Idle 叶 ⇒ 出处是那片叶、值就是叶里的值。
    //    **图写得再满也不能指挥指令**（图只带倾向）。
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        Some(OrderSource::Leaf),
        "逐舰叶是唯一的供值者（下水时船坞写的那片 Idle 叶）"
    );
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Idle),
        "图不再供指令 ⇒ 值只来自那片叶"
    );
    // 而**倾向**确实是图给的（图这条链是活的）。
    assert_eq!(
        state.ship_doctrine(name.clone()),
        ShipDoctrine {
            temper: 0.5,
            lone_wolf: -0.5
        }
    );

    // ② 舰队默认指令这片叶**已经不存在**：写它会被 apply 当成未知字段拒掉（响亮）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_ship_order": {"behavior": {"type": "dock", "body": "火星"}}
    }]});
    assert!(
        apply_patch(&mut state, &config, &diff).is_err(),
        "`default_ship_order` 已删 ⇒ 写入必须报错，不能静默吞掉"
    );

    // ③ 换一片叶里的值（mode 仍是 `Inherit`）：出处照旧是 `leaf`——值真的来自那片叶。
    state.control_mut(fid.clone()).unwrap().ship_orders.insert(
        name.clone(),
        Control::inherit(ShipBehavior::Move {
            position: [1.0, 2.0],
        }),
    );
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        Some(OrderSource::Leaf)
    );
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Move {
            position: [1.0, 2.0]
        }),
        "叶存在就用叶里的值（与它的 mode 无关）——这正是「叶 Inherit ≠ 没有叶」"
    );

    // ④ 删掉那片叶 ⇒ 出处回到 None（"没有人说话"），**不回落到图**。
    state
        .control_mut(fid.clone())
        .unwrap()
        .ship_orders
        .remove(&name);
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        None,
        "没有叶、图也不供指令 ⇒ 没有任何一层说话"
    );
    assert_eq!(state.ship_behavior(name.clone()), None);
    // 而倾向**不受影响**（图那条链还在）。
    assert_eq!(state.ship_kiting(name.clone()), 0.7);
    assert_eq!(state.ship_role(name.clone()), ShipRole::War);

    // ⑤ 把图上的 `role` 清空（`role: null`）⇒ 这条轴回到链的**下一层**：舰队默认。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_role": {"role": "Freight", "mode": "Player"},
        "blueprints": [{"name": "全能图", "role": null}]
    }]});
    apply_patch(&mut state, &config, &diff).expect("clearing the role axis applies");
    assert_eq!(
        state.ship_role(name.clone()),
        ShipRole::Freight,
        "图对这条轴沉默了 ⇒ 舰队默认接手（这就是链的意义：图在前、舰队默认在后）"
    );
    assert_eq!(
        state.ship_role_control(name.clone()),
        ControlMode::Player,
        "现在由舰队默认叶（Player）表态"
    );
    assert_eq!(
        state.ship_kiting(name.clone()),
        0.7,
        "另外两条轴不受影响（逐轴独立）"
    );
}

/// **悬空图指针 ⇒ 该建造区停产**（Q10(a)）：进度不再增加，也没有舰凭空冒出来。
#[test]
fn a_dangling_blueprint_pointer_stops_the_yard() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    stock(&mut state, &config, &fid, 400.0);
    let (cid, bid, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "会被删掉的图",
        "corvette",
        &["kinetic", "ion_drive"],
        (None, None, None),
        ControlMode::Player,
    );
    // 把进度清零，再把指针改成一张**不存在**的图（模拟玩家改名/删图之后的现场）。
    if let Some(city) = state.city_mut(&cid) {
        city.ship_progress.insert(class.clone(), 0.0);
        for b in city.buildings.iter_mut() {
            if b.id == bid {
                b.blueprint = Some("已经不存在的图".to_string());
            }
        }
    }
    let ships_before = state.ships.len();
    let mut rng = Prng::new(42);
    advance(&mut state, &config, &mut rng);
    assert_eq!(
        state.ships.len(),
        ships_before,
        "悬空指针不许凭空产出（也不许静默回落生成器）"
    );
    let progress = state
        .city(&cid)
        .unwrap()
        .ship_progress
        .get(&class)
        .copied()
        .unwrap_or(0.0);
    assert!(
        progress <= 1e-9,
        "那个建造区停产 ⇒ 进度必须一点不涨，got {progress}"
    );
    // 建区还在、指针**原样**保留（读面据此能一眼看出「这个区指着一张不存在的图」）。
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
        Some("已经不存在的图"),
        "指针原样输出，不被静默清掉"
    );
}

/// **玩家归属的图买不起 ⇒ 不下水、进度继续攒**（Q4(b)），并留下可见标记。
#[test]
fn a_player_blueprint_that_cannot_be_afforded_waits_for_money() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    // 一件**稀有到买不起**的选装（等离子炮要氦-3/金）。
    let (cid, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "豪华护卫",
        "corvette",
        &["plasma", "ion_drive"],
        (None, None, None),
        ControlMode::Player,
    );
    if let Some(f) = state.faction_mut(&fid) {
        for v in f.resources.values_mut() {
            *v = 0.0;
        }
    }
    let ships_before = state.ships.len();
    let mut rng = Prng::new(42);
    advance(&mut state, &config, &mut rng);
    assert_eq!(state.ships.len(), ships_before, "买不起就不下水");
    let progress = state
        .city(&cid)
        .unwrap()
        .ship_progress
        .get(&class)
        .copied()
        .unwrap_or(0.0);
    assert!(
        progress >= config.ship_spec(&class).build_points - 1e-9,
        "进度**继续攒**（下回合再试），got {progress}"
    );
    assert!(
        crate::sim::blueprint_launch_waiting(&state, &config, &fid, &"豪华护卫".to_string()),
        "要有**可见标记**：进度满了却没下水（投影蓝图表 launch_waiting 列就是它）"
    );

    // 给钱 → 下一回合就下水。
    stock(&mut state, &config, &fid, 400.0);
    advance(&mut state, &config, &mut rng);
    assert!(
        state
            .ships
            .iter()
            .any(|s| s.blueprint.as_deref() == Some("豪华护卫")),
        "攒够钱之后必须下水（进度没丢）"
    );
}

