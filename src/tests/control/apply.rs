//! 写面与报告：写值即接管、scope 压过独立势力、旧拼写仍可载入、报告要 `is_clean` 且逐叶计数、消失的舰/别人的舰/拼错的势力或字段都要**响亮报错**而不是静默丢；旧的 `删叶` 键在 API 边界被拒。

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

/// **删叶机制已删除**（2026-10 用户裁决：出厂默认只是初始值，不是可恢复的目标）：
/// 旧客户端发 `"删叶": true` 必须**响亮失败**，不能静默 no-op。
#[test]
fn removing_a_leaf_is_rejected_and_leaves_the_value_untouched() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");
    let before = state.ship_doctrine(ship.clone());

    // 先写一片逐舰风格叶（正常路径仍然可用）。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "temper": -1.0, "lone_wolf": 0.5}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("diff applies");
    let leaf_before = state.ship_doctrine(ship.clone());
    assert_ne!(leaf_before, before, "前置条件：叶真的写进去了");

    // 旧删叶键现在应该在 API 边界被拒——整份 diff 不落地。
    let diff = serde_json::json!({
        "control": [{"势力": fid, "风格": [{"舰": ship, "删叶": true}]}]
    });
    let err = apply_patch(&mut state, &config, &diff).unwrap_err();
    assert!(err.contains("删叶"), "{err}");
    assert!(
        err.contains("已删除"),
        "错误必须说明机制已删：{err}"
    );
    assert_eq!(
        state.ship_doctrine(ship.clone()),
        leaf_before,
        "被拒绝的补丁一个字节都不许动"
    );

    // 风格叶、势力级默认叶、预算、迁都：旧键在嵌套的任何一层都拒绝。
    for diff in [
        serde_json::json!({
            "control": [{"势力": fid, "舰队默认风格": {"删叶": true}}]
        }),
        serde_json::json!({
            "control": [{"势力": fid, "投资预算": [{"资源": "铁", "删叶": true}]}]
        }),
        serde_json::json!({
            "control": [{"势力": fid, "首都": {"删叶": true}}]
        }),
    ] {
        let err = apply_patch(&mut state, &config, &diff).unwrap_err();
        assert!(err.contains("删叶") && err.contains("已删除"), "{err}");
    }
}

/// 读面仍然可以原样回传；控制叶不再因为模板里有旧字段而被改写。
#[test]
fn the_control_template_round_trips_without_any_remove_mechanism() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let template = crate::control::control_surface(&state, &config);
    let fac = template["control"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["势力"] == fid)
        .unwrap()
        .clone();
    let diff = serde_json::json!({"control": [fac]});
    let r = apply_patch(&mut state, &config, &diff).expect("read face must round-trip");
    assert!(r.is_clean(), "{:?}", r.skipped);
}
