//! 设计图（`--apply` 的 blueprint 叶）与船坞下水：图压舰队默认、`order_source`、悬空指针停产、买不起就不下水。夹具 `attach_blueprint`/`spawn_at`/`stock` 在 `super`。

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
        None,
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

/// **改图不碰已经下水的舰**（§7.1-2，`ship-blueprint.md` §3.1 的可检查形式）。
#[test]
fn editing_a_blueprint_does_not_touch_existing_ships() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    stock(&mut state, &config, &fid, 400.0);
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "护卫甲",
        "corvette",
        &["kinetic", "ion_drive"],
        None,
        ControlMode::Player,
    );
    let name = spawn_at(&mut state, &config, &fid, &class, "地球", Some("护卫甲"));
    let before = state.ship(&name).cloned().expect("the new ship");

    // 改图：换选装（写值即接管，仍然是 Player）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "护卫甲", "components": ["plasma"], "mode": "Player"}
    ]}]});
    let report = apply_patch(&mut state, &config, &diff).expect("editing a blueprint applies");
    assert!(report.is_clean(), "改图本身必须干净：{:?}", report.skipped);

    let after = state
        .ship(&name)
        .cloned()
        .expect("the old ship is still there");
    assert_eq!(
        after.components, before.components,
        "**已下水的舰的选装是快照**，改图不许动它"
    );
    assert_eq!(after.hull_max, before.hull_max);
    assert_eq!(after.shield_max, before.shield_max);
    assert_eq!(after.component_hp, before.component_hp);

    // 再下水一艘 ⇒ 带**新**选装。
    let name2 = spawn_at(&mut state, &config, &fid, &class, "地球", Some("护卫甲"));
    assert_eq!(
        state.ship(&name2).unwrap().components,
        vec!["plasma".to_string()],
        "之后下水的舰按**新**图装配"
    );
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
        None,
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
        None,
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
        None,
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

/// **图的意图轴默认沉默**（Q1(c)）＋ 图上真写了 `order` 时它压过舰队默认（Q1(c) 的链）。
#[test]
fn blueprint_default_order_governs_new_ships() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "护卫-守家",
        "corvette",
        &["kinetic", "ion_drive"],
        Some(ShipBehavior::Dock {
            body: "地球".to_string(),
        }),
        ControlMode::Player,
    );
    // 舰队默认**同时**是 Player 且冲突 ⇒ **更具体的图赢**（链：叶 → 图 → 舰队默认 → …）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_ship_order": {"behavior": {"type": "dock", "body": "火星"}}
    }]});
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");

    let name = spawn_at(&mut state, &config, &fid, &class, "水星", Some("护卫-守家"));
    assert_eq!(
        state.ship_control(name.clone()),
        ControlMode::Player,
        "图上的意图归玩家 ⇒ AI 不许接管这艘舰"
    );
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        }),
        "图比舰队默认更具体 ⇒ 图赢（Q1(c)）"
    );

    let mut rng = Prng::new(7);
    let before = state.ship(&name).unwrap().position;
    advance(&mut state, &config, &mut rng);
    let after = state.ship(&name).map(|s| s.position).unwrap_or(before);
    assert!(
        dist(after, state.body_position("地球")) < dist(before, state.body_position("地球")),
        "新舰按图的默认意图驶向地球（而不是被 AI 派去别处）"
    );
}

/// 回归守卫：**没有图 / 图没有写 `order`** 的舰仍由舰队默认作答（新层不许把旧行为吃掉）。
#[test]
fn fleet_default_still_covers_blueprintless_ships() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    // 一张**只钉选装**（没写 order）的图。
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "只钉选装",
        "corvette",
        &["kinetic", "ion_drive"],
        None,
        ControlMode::Player,
    );
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_ship_order": {"behavior": {"type": "dock", "body": "火星"}}
    }]});
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");

    let with_bp = spawn_at(&mut state, &config, &fid, &class, "水星", Some("只钉选装"));
    let without_bp = spawn_at(&mut state, &config, &fid, &class, "水星", None);
    for (ship, what) in [(&with_bp, "挂了图但图没写 order"), (&without_bp, "没有图")] {
        assert_eq!(
            state.ship_behavior(ship.clone()),
            Some(ShipBehavior::Dock {
                body: "火星".to_string()
            }),
            "{what} 的舰仍由舰队默认作答"
        );
        assert_eq!(
            state.ship_control(ship.clone()),
            ControlMode::Player,
            "{what} 的舰归属仍由舰队默认叶决定（图的意图轴沉默 ⇒ 建图 ≠ 表态）"
        );
    }
}

/// **贯穿性要求 3**：`order_source` 必须把「叶不存在」与「叶写着 `Inherit`」分开报。
#[test]
fn order_source_separates_a_missing_leaf_from_a_silent_one() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    let (_, _, class) = attach_blueprint(
        &mut state,
        &config,
        &fid,
        "护卫-守家",
        "corvette",
        &["kinetic", "ion_drive"],
        Some(ShipBehavior::Dock {
            body: "地球".to_string(),
        }),
        ControlMode::Player,
    );
    let name = spawn_at(&mut state, &config, &fid, &class, "水星", Some("护卫-守家"));
    // ① 刚下水的舰：叶**存在**且写着 `Inherit`，但更具体的图层供值 ⇒ 出处是它。
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        Some(OrderSource::Blueprint("护卫-守家".to_string()))
    );
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        }),
        "叶没表态 ⇒ 图上的意图生效"
    );

    // ② 删掉那片叶（"叶不存在"）：出处**不变**（值仍然来自图）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "ship_orders": [{"ship": name, "remove": true}]
    }]});
    apply_patch(&mut state, &config, &diff).expect("remove applies");
    assert!(
        !state
            .control(fid.clone())
            .unwrap()
            .ship_orders
            .contains_key(&name),
        "叶真的被删了"
    );
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        Some(OrderSource::Blueprint("护卫-守家".to_string()))
    );
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        })
    );

    // ③ 把图的意图轴清空（`order: null`）：叶**不存在** + 没人供值 ⇒ **没有出处**
    //    （调用方按 Idle 兜底）。这就是「叶不存在」那一侧。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "护卫-守家", "order": null}
    ]}]});
    apply_patch(&mut state, &config, &diff).expect("clearing the order axis applies");
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        None,
        "没有任何一层说话"
    );
    assert_eq!(state.ship_behavior(name.clone()), None);

    // ④ **叶存在但写着 `Inherit`**（不是删掉它）：出处必须诚实报 `leaf`——值真的来自
    //    那片叶（`leaf.map(|l| l.value).unwrap_or(..)`），与「没有叶」**不等价**。
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

    // ⑤ 舰队默认供值时出处是它；叶有意见时叶赢。
    state
        .control_mut(fid.clone())
        .unwrap()
        .ship_orders
        .remove(&name);
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_ship_order": {"behavior": {"type": "dock", "body": "火星"}}
    }]});
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        Some(OrderSource::FleetDefault)
    );
    state
        .control_mut(fid.clone())
        .unwrap()
        .ship_orders
        .insert(name.clone(), Control::player(ShipBehavior::Idle));
    assert_eq!(
        state.ship_behavior_source(name.clone()),
        Some(OrderSource::Leaf),
        "叶有意见 ⇒ 叶赢"
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
        None,
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
        None,
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

/// 出厂事件带上**归因**（哪张图造的），而无图那一路的句子**逐字不变**（digest 拿它当故事板）。
#[test]
fn ship_spawned_event_carries_the_blueprint_only_when_there_is_one() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    attach_blueprint(
        &mut state,
        &config,
        &fid,
        "有图",
        "corvette",
        &["kinetic", "ion_drive"],
        None,
        ControlMode::Player,
    );
    let with_bp = spawn_at(&mut state, &config, &fid, "corvette", "地球", Some("有图"));
    let plain = spawn_at(&mut state, &config, &fid, "corvette", "地球", None);
    let headline = |ship: &str| {
        state
            .events
            .iter()
            .find_map(|e| match e {
                GameEvent::ShipSpawned {
                    ship: s, blueprint, ..
                } if s == ship => Some((e.headline(), blueprint.clone())),
                _ => None,
            })
            .expect("spawn_ship must emit an event")
    };
    let (h_bp, bp) = headline(&with_bp);
    assert_eq!(bp.as_deref(), Some("有图"));
    assert!(
        h_bp.contains("设计图：有图"),
        "挂了图的事件句子带归因：{h_bp}"
    );
    let (h_plain, bp2) = headline(&plain);
    assert_eq!(bp2, None);
    assert!(
        !h_plain.contains("设计图"),
        "无图那一路的句子不许变（digest 的故事板拿它比对）：{h_plain}"
    );
}
