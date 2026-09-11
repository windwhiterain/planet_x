//! 写面与报告：写值即接管、scope 压过独立势力、旧拼写仍可载入、报告要 `is_clean` 且逐叶计数、消失的舰/别人的舰/拼错的势力或字段都要**响亮报错**而不是静默丢、删叶把值还给来源。

use super::*;

/// 「写值即接管」：只写值、不写 mode 的 diff 必须真的生效（而不是被系统下一回合
/// 按自己的逻辑覆盖掉），并在回执里有一条 `took_over` 记录。
#[test]
fn writing_a_value_without_mode_takes_over() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);

    // 不带 mode 写一条舰指令 + 一条预算。
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "指令": [{"舰": "长城", "行为": {"type": "dock", "body": "地球"}}],
            "建造预算": [{"资源": "铁", "值": 3.5}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff applies");

    assert_eq!(
        state.ship_control("长城".to_string()),
        ControlMode::Player,
        "a written value is an order"
    );
    assert_eq!(
        state.ship_behavior("长城".to_string()),
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        })
    );
    assert_eq!(
        state.construction_budget_control("中国".to_string(), "铁"),
        ControlMode::Player,
        "same rule for budgets: the value would otherwise be recomputed away"
    );
    assert_eq!(
        report.took_over.len(),
        2,
        "the receipt must name every implicitly taken-over leaf: {:?}",
        report.took_over
    );
    assert!(
        report
            .took_over
            .iter()
            .any(|p| p.contains("指令[0].行为")),
        "{:?}",
        report.took_over
    );
    assert!(
        report
            .took_over
            .iter()
            .any(|p| p.contains("建造预算[0].值")),
        "{:?}",
        report.took_over
    );

    // 显式写 `Inherit` 仍然能把叶片交还给作用域链（这不是接管，是撤销表态）。
    let give_back = serde_json::json!({
        "control": [{"势力": "中国",
            "指令": [{"舰": "长城", "归属": "Inherit"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &give_back).expect("diff applies");
    assert!(
        report.took_over.is_empty(),
        "an explicit mode is not a takeover: {:?}",
        report.took_over
    );
    assert_eq!(
        state.ship_control("长城".to_string()),
        ControlMode::Auto,
        "Inherit hands it back to the system"
    );
}

/// Setting a faction's scope to Player must actually take over its leaves
/// (which are inert `inherit` now), even though the AI has "written" values.
#[test]
fn scope_player_takes_over_independent_faction() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    // Default scope (all Inherit) → everything resolves to Auto.
    assert_eq!(
        state.ship_control("长城".to_string()),
        ControlMode::Auto,
        "default scope is Auto"
    );

    // Take over faction 中国 via a scope-only diff.
    let scope_diff = serde_json::json!({"scope": {"factions": [["中国", "Player"]]}});
    apply_patch(&mut state, &config, &scope_diff).expect("scope diff applies");
    assert_eq!(
        state.ship_control("长城".to_string()),
        ControlMode::Player,
        "ship of a Player faction is player-owned"
    );
    assert_eq!(
        state.investment_budget_control("中国".to_string(), "铁"),
        ControlMode::Player,
        "budget leaf follows scope"
    );
    assert_eq!(
        state.construction_budget_control("中国".to_string(), "铁"),
        ControlMode::Player
    );
    // Other factions are untouched (still Auto): 华盛顿 is a US ship (美国).
    assert_eq!(
        state.ship_control("华盛顿".to_string()),
        ControlMode::Auto,
        "untouched faction stays Auto"
    );

    // An explicit leaf mode still overrides scope in the opposite direction:
    // hand 长城 back to the system inside a Player faction.
    let leaf_diff = serde_json::json!({
        "control": [{"势力": "中国", "指令": [{"舰": "长城", "归属": "Auto"}]}]
    });
    apply_patch(&mut state, &config, &leaf_diff).expect("leaf diff applies");
    assert_eq!(
        state.ship_control("长城".to_string()),
        ControlMode::Auto,
        "explicit Auto leaf beats Player scope"
    );

    // …and an explicit `Inherit` leaf un-does that override, falling back to scope.
    let back = serde_json::json!({
        "control": [{"势力": "中国", "指令": [{"舰": "长城", "归属": "Inherit"}]}]
    });
    apply_patch(&mut state, &config, &back).expect("inherit leaf applies");
    assert_eq!(
        state.ship_control("长城".to_string()),
        ControlMode::Player,
        "Inherit leaf falls back to the faction scope"
    );
}

/// 三态的**旧档拼写**必须继续能读进来（`.ron` 存档里存的是旧的
/// `Option<ControlMode>`：`None` = 没有说话、`Some(Ai)` / `Some(Player)` = 显式指定）。
/// 这是「改名不丢档」的守卫：映射是双射，所以加载时就能完成迁移。
#[test]
fn legacy_mode_spellings_still_load() {
    use crate::model::ControlMode;

    // JSON 读面：null / 旧名 "Ai" / 三个新名。
    let from_json = |s: &str| -> ControlMode { serde_json::from_str(s).expect("mode must load") };
    assert_eq!(from_json("null"), ControlMode::Inherit, "null = 没有说话");
    assert_eq!(
        from_json("\"Ai\""),
        ControlMode::Auto,
        "Ai is the pre-rename spelling of Auto"
    );
    assert_eq!(from_json("\"Auto\""), ControlMode::Auto);
    assert_eq!(from_json("\"Inherit\""), ControlMode::Inherit);
    assert_eq!(from_json("\"Player\""), ControlMode::Player);
    assert!(
        serde_json::from_str::<ControlMode>("\"玩家\"").is_err(),
        "unknown spellings must fail loudly"
    );

    // RON 存档里叶子写的是 `mode: None` / `Some(Ai)` / `Some(Player)`。
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Leaf {
        #[serde(default)]
        mode: ControlMode,
    }
    let load = |s: &str| -> ControlMode {
        ron::from_str::<Leaf>(s)
            .expect("legacy leaf must load")
            .mode
    };
    assert_eq!(
        load("(mode: None)"),
        ControlMode::Inherit,
        "legacy `None` = Inherit"
    );
    assert_eq!(
        load("(mode: Some(Ai))"),
        ControlMode::Auto,
        "legacy `Some(Ai)` = Auto"
    );
    assert_eq!(
        load("(mode: Some(Player))"),
        ControlMode::Player,
        "legacy `Some(Player)` = Player"
    );
    assert_eq!(
        load("(mode: Some(Inherit))"),
        ControlMode::Inherit,
        "new spelling, same `Some(..)` shell"
    );
    assert_eq!(
        load("()"),
        ControlMode::Inherit,
        "missing field defaults to Inherit"
    );
    // 手写 `.ron` 也必须带 `Some(..)` 外壳（RON 里 `deserialize_option` 不认裸标识符）；
    // 这是**有意的**：JSON 是写面，`.ron` 只由 `--save` 写。写错了会当场报 ExpectedOption，
    // 而不是静默当成 Inherit。
    assert!(
        ron::from_str::<Leaf>("(mode: Player)").is_err(),
        "a bare RON identifier must fail loudly"
    );

    // 作用域树同理：整棵树（含旧的 `global: None` 与 `Some(x)` 键值）必须能读。
    let scope: crate::model::ControlScope = ron::from_str(
        r#"(global: None, factions: {"中国": Some(Player), "美国": None}, bodies: {}, cities: {})"#,
    )
    .expect("legacy scope tree must load");
    assert_eq!(scope.global, ControlMode::Inherit);
    assert_eq!(
        scope.factions.get("中国").copied(),
        Some(ControlMode::Player)
    );
    assert_eq!(
        scope.factions.get("美国").copied(),
        Some(ControlMode::Inherit)
    );

    // 写面（线格式）：JSON 是干净的字符串三态，RON 是与旧档同形的 `Some(标识符)`
    // —— 而且**自己写的必须读得回来**（RON 的裸标识符 vs 引号字符串在这里是坑）。
    for m in [ControlMode::Inherit, ControlMode::Auto, ControlMode::Player] {
        let json = serde_json::to_string(&Leaf { mode: m }).expect("json");
        assert_eq!(
            json,
            format!("{{\"mode\":\"{}\"}}", m.name()),
            "JSON read面 must be a bare three-state string"
        );
        assert_eq!(
            serde_json::from_str::<Leaf>(&json).expect("json back").mode,
            m
        );

        let text = ron::to_string(&Leaf { mode: m }).expect("ron");
        assert_eq!(
            text,
            format!("(mode:Some({}))", m.name()),
            "RON must stay `Some(<ident>)`, same shape as legacy"
        );
        assert_eq!(
            ron::from_str::<Leaf>(&text).expect("ron back").mode,
            m,
            "own checkpoint must load back: {text}"
        );
    }
}

/// 娱乐/福利预算：一座城的忠诚度投入是一个可控叶子。按 Player 覆盖后，治理模型
/// 会读取它；省略 value 时保留当前值、省略 mode 时保留当前模式。
#[test]
fn apply_loyalty_budget_patch() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"势力": "中国", "城市福利预算": [{"城": "长三角", "值": 40.0, "归属": "Player"}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("loyalty budget diff applies");
    assert_eq!(
        state.loyalty_budget_control("中国".to_string(), "长三角".to_string()),
        ControlMode::Player
    );
    let v = state
        .control("中国".to_string())
        .and_then(|c| c.loyalty_budget.get("长三角"))
        .map(|c| c.value)
        .unwrap_or(f64::NAN);
    assert!(
        (v - 40.0).abs() < 1e-7,
        "loyalty budget value should be 40.0, got {v}"
    );
}

/// 一条全合法的 diff 必须报 `applied > 0` 且**没有**任何丢弃——否则
/// `WARN_APPLY_SKIPPED` 会变成噪声，agent 学会无视它，等于没做。
#[test]
fn a_valid_diff_reports_clean_and_counts_every_leaf() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "指令": [{"舰": "长城", "行为": {"type": "dock", "body": "地球"}, "归属": "Player"}],
            "城市福利预算": [{"城": "长三角", "值": 2.0, "归属": "Player"}],
            "建造预算": [{"资源": "铁", "值": 1.0, "归属": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("valid diff applies");
    assert!(
        report.is_clean(),
        "a valid diff must report nothing skipped: {:?}",
        report.skipped
    );
    assert_eq!(
        report.applied, 3,
        "one leaf per order / budget / loyalty entry"
    );
    assert_eq!(
        state.ship_behavior("长城".to_string()),
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        })
    );
}

/// **已战沉 / 已改名**的舰：从前是静默 no-op（退出码 0，看不出来）。现在必须
/// 报出来，并点名到叶、给出原因码——这是 agent 发现「命令没下达」的唯一渠道。
#[test]
fn a_vanished_ship_is_reported_not_dropped_silently() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "指令": [{"舰": "长城2", "行为": {"type": "idle"}, "归属": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(
        report.skipped.len(),
        1,
        "the dropped order must be reported: {report:?}"
    );
    let s = &report.skipped[0];
    assert_eq!(s.code, "no_such_ship");
    assert_eq!(s.value, "长城2");
    assert!(
        s.path.contains("指令[0]"),
        "path must point at the leaf: {}",
        s.path
    );
}

/// 别人的舰：与「查无此舰」必须分开报（一个要改名、一个是写错势力）。
#[test]
fn another_factions_ship_is_reported_with_its_own_code() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "指令": [{"舰": "华盛顿", "行为": {"type": "idle"}, "归属": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped[0].code, "not_your_ship");
    assert!(
        report.skipped[0].reason.contains("美国"),
        "reason should name the real owner"
    );
}

/// **幽灵势力**：一个错别字从前会在 `state.control` 里凭空造出一个势力，
/// 它随后出现在 `--control` 模板与 web 控制面里，看起来像一个真的势力。
/// 现在整条补丁被丢弃并报出来，世界不受污染。
#[test]
fn a_typo_faction_id_does_not_invent_a_phantom_faction() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let before = state.control.len();
    let diff = serde_json::json!({
        "control": [{"势力": "中国洋", "建造预算": [{"资源": "铁", "值": 9.9, "归属": "Player"}]}]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped[0].code, "no_such_faction");
    assert_eq!(
        state.control.len(),
        before,
        "a typo must not create a control entry"
    );
    assert!(state.control.get("中国洋").is_none(), "no phantom faction");
    // 而且它绝不能出现在读面（模板）里——那正是它从前最有害的地方。
    let surface = control_surface(&state, &config);
    let ids: Vec<&str> = surface["control"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["势力"].as_str())
        .collect();
    assert!(
        !ids.contains(&"中国洋"),
        "phantom faction leaked into the template: {ids:?}"
    );
}

/// `building` 是 u32 下标、只在城内部唯一，所以「另一座城的合法下标」在本城
/// 是错的。这条从前静默 no-op，现在报出**该城真实的下标列表**——agent 手写
/// 权重时最常踩的一脚。
#[test]
fn a_building_index_from_another_city_is_reported_with_the_real_indices() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "建造权重": [{"城": "长三角", "建筑": 21, "值": 0.5, "归属": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped[0].code, "no_such_building");
    assert!(
        report.skipped[0].reason.contains('0'),
        "reason should list the real indices: {}",
        report.skipped[0].reason
    );
}

/// 未知**字段名**（`ship_order` 少个 s）从前被 serde 静默忽略 → 整条意图蒸发、
/// 退出码 0。现在当场报错并列出合法字段。
#[test]
fn a_misspelled_control_field_is_rejected_rather_than_ignored() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"势力": "中国",
            "ship_order": [{"舰": "长城", "行为": {"type": "idle"}, "归属": "Player"}]
        }]
    });
    let err = apply_patch(&mut state, &config, &diff).expect_err("a typo'd field must be an error");
    assert!(
        err.contains("ship_order"),
        "error must name the bad field: {err}"
    );
    assert!(
        err.contains("指令"),
        "error must list the legal fields: {err}"
    );
}

/// **删叶**（`remove: true`）：控制叶的"存在性"本身就是一种状态——「没有叶」= 这一层
/// 没有说话。而取值规则里「叶不存在」与「叶写着 `Inherit`」**不**等价
/// （`leaf.map(|l| l.value).unwrap_or(record)`：叶存在就用叶里的值，与 `mode` 无关），
/// 所以"碰过一次的风格叶"以前永远钉着那个数。这条测试把三段都钉住：
/// ① 写叶 ⇒ 钉住；② 「恢复继承」撤**不掉**它；③ **删叶**才真的回到出厂快照。
#[test]
fn removing_a_leaf_returns_the_value_to_its_source() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");
    // 出厂记录值给成非零：`config/*.ron` 从来没填过 `default_doctrine`，开局记录值是 {0,0}，
    // 那样子"回到出厂快照"与"钉在 0"分不出来。
    for s in state.ships.iter_mut().filter(|s| s.faction_id == fid) {
        s.doctrine = ShipDoctrine {
            temper: 0.71,
            lone_wolf: -0.2,
        };
    }
    let record = state.ship(&ship).unwrap().doctrine;

    // ① 写一片逐舰风格叶：有效值 = 叶里的值，这片叶**钉住**了它。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "temper": -1.0, "lone_wolf": 0.5}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(state.ship_doctrine(ship.clone()).temper, -1.0);

    // ② 「恢复继承」（只写 mode）撤不掉那个数：叶还在，取值优先用叶里的值。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "归属": "Inherit"}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(
        state.ship_doctrine(ship.clone()).temper,
        -1.0,
        "叶存在就用叶里的值（哪怕它写着 Inherit）——这正是「恢复继承」不够用的原因"
    );

    // ③ 删叶 ⇒ 有效值回到**出厂快照**，而且这片叶真的从控制面里消失。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "删叶": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(
        r.removed.len(),
        1,
        "真的删掉了要留一条 NOTE_APPLY_REMOVED 回执：{:?}",
        r.removed
    );
    assert!(r.removed[0].contains("风格"), "{:?}", r.removed);
    assert_eq!(
        state.ship_doctrine(ship.clone()),
        record,
        "删叶之后有效风格必须回到出厂快照"
    );
    assert!(
        state
            .control
            .get(&fid)
            .and_then(|c| c.ship_doctrine.get(&ship))
            .is_none(),
        "叶必须真的没了"
    );

    // ④ 幂等：再删一次不报错、不算丢弃、也不进 `removed`（目标状态已经达成）。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "删叶": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert!(
        r.removed.is_empty(),
        "删一片本来就不存在的叶不进回执：{:?}",
        r.removed
    );
    assert_eq!(r.applied, 1, "但它是**成功的**（目标状态达成），不是被丢弃");

    // ⑤ `remove` 与值同时出现 ⇒ **拒绝**（任何一种静默优先级都会让人误判另一件事发生了）。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "删叶": true, "temper": 0.5}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert_eq!(r.skipped.len(), 1, "{:?}", r);
    assert_eq!(r.skipped[0].code, "remove_conflicts_with_value");
    assert_eq!(
        state.ship_doctrine(ship.clone()),
        record,
        "被拒绝的补丁一个字节都不许动"
    );
}

/// 删叶的三个边角：**势力级默认叶**（删了 ⇒ 这一层不再供值）、**舰已不在**（陈叶清理）、
/// 以及**预算/迁都**这几片同形的叶（同一套规则，不是只给风格轴开的后门）。
#[test]
fn removing_works_for_fleet_defaults_stale_ships_and_budgets() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");

    // 势力级默认风格：建成"玩家表态"的叶 ⇒ 叶 Inherit 的舰取它的值。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "舰队默认风格": {"temper": 0.4, "lone_wolf": -0.6, "归属": "Player"}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(
        (
            state.ship_doctrine(ship.clone()).temper,
            state.ship_doctrine(ship.clone()).lone_wolf
        ),
        (0.4, -0.6)
    );

    // 删掉这片默认叶 ⇒ 这一层不再供值（回落到舰上记录值 / 作用域链）。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "舰队默认风格": {"删叶": true}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
    assert!(
        state
            .control
            .get(&fid)
            .and_then(|c| c.default_doctrine.as_ref())
            .is_none()
    );
    let rec = state.ship(&ship).unwrap().doctrine;
    assert_eq!(
        state.ship_doctrine(ship.clone()),
        rec,
        "默认叶没了 ⇒ 回落到舰上记录值"
    );

    // 舰已不在（战沉/换代）：它的陈叶仍然能被删掉——删的是**控制面**里的叶，不要求实体还在。
    state.ships.retain(|s| s.name != ship);
    state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_doctrine
        .insert(
            ship.clone(),
            Control::inherit(ShipDoctrine {
                temper: 0.9,
                lone_wolf: 0.9,
            }),
        );
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "删叶": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(
        r.is_clean(),
        "删陈叶不该因为舰没了而被丢弃：{:?}",
        r.skipped
    );
    assert_eq!(r.removed.len(), 1, "陈叶也是真的被删掉了：{:?}", r.removed);

    // 预算叶与迁都叶：同一套 `remove` 语义（这里只钉"删得掉"，值语义由各自的取值规则决定）。
    let diff = serde_json::json!({
        "control": [{"势力": fid,
            "投资预算": [{"资源": "铁", "值": 3.0}],
            "首都": {"值": "地球", "归属": "Player"}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let diff = serde_json::json!({
        "control": [{"势力": fid,
            "投资预算": [{"资源": "铁", "删叶": true}],
            "首都": {"删叶": true}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(r.removed.len(), 2, "{:?}", r.removed);
    let c = state.control.get(&fid).expect("control");
    assert!(c.investment_budget.get("铁").is_none() && c.capital.is_none());
}
