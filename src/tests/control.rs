//! control 的单元测试（读写面 = 控制树，不推进回合）。

use super::*;

/// The tagged agent-state `order` form must be accepted and rewritten into
/// the default enum form that the rest of the pipeline expects.
#[test]
fn normalize_behavior_accepts_tagged_form() {
    let cases = [
        (serde_json::json!({"type":"idle"}), serde_json::json!("Idle")),
        (
            serde_json::json!({"type":"follow","ship":"华盛顿"}),
            serde_json::json!({"Follow":{"ship":"华盛顿"}}),
        ),
        (
            serde_json::json!({"type":"dock_city","city":"长三角"}),
            serde_json::json!({"DockCity":{"city":"长三角"}}),
        ),
        (
            serde_json::json!({"type":"move","position":[-0.5,0.3]}),
            serde_json::json!({"Move":{"position":[-0.5,0.3]}}),
        ),
        (
            serde_json::json!({"type":"colonize","body":"地球"}),
            serde_json::json!({"Colonize":{"body":"地球"}}),
        ),
    ];
    for (tagged, expected) in cases {
        let mut v = tagged.clone();
        normalize_behavior(&mut v, "test").expect("legal tag normalizes");
        assert_eq!(v, expected, "tagged input {tagged:?} must normalize to {expected:?}");
    }
}

/// An unknown tag must be **rejected with a self-correcting message**: the
/// removed `target_ship` behavior is the single most likely thing an agent
/// writes (it is what the old manual taught), and the old code let it fall
/// through to serde's `invalid value: map, expected map with a single key`
/// — which names neither the field nor the alternatives.
#[test]
fn unknown_behavior_tag_is_rejected_with_the_legal_tags_and_a_hint() {
    let mut v = serde_json::json!({"type":"target_ship","ship":"华盛顿","attack":true});
    let err = normalize_behavior(&mut v, "中国.ship_orders[0]").expect_err("removed behavior must be rejected");
    for needle in ["target_ship", "合法", "follow", "自动开火"] {
        assert!(err.contains(needle), "error must mention {needle:?}, got: {err}");
    }
    // 未知但也不像旧行为的标签同样被拒（不再静默流过）。
    let mut v = serde_json::json!({"type":"teleport"});
    assert!(normalize_behavior(&mut v, "x").is_err());
}

/// The default form must pass through unchanged.
#[test]
fn normalize_behavior_keeps_default_form() {
    let mut v = serde_json::json!({"Follow":{"ship":"华盛顿"}});
    normalize_behavior(&mut v, "test").expect("default form is untouched");
    assert_eq!(v, serde_json::json!({"Follow":{"ship":"华盛顿"}}));
}

/// Applying a tagged-form diff to a real world must produce the same
/// controllable behavior as the equivalent default-form diff.
#[test]
fn apply_patch_accepts_tagged_ship_order() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    // 长城 = 中国 (faction 3) 的起始护卫舰；华盛顿 = 美国的一艘舰。舰名即唯一 key。
    let tagged = serde_json::json!({
        "control": [{
            "faction_id": "中国",
            "ship_orders": [{"ship": "长城", "behavior": {"type": "follow", "ship": "华盛顿"}, "mode": "Player"}]
        }]
    });
    apply_patch(&mut state, &config, &tagged).expect("tagged diff applies");
    let b = state.ship_behavior("长城".to_string()).expect("长城 has an order");
    assert_eq!(b, ShipBehavior::Follow { ship: "华盛顿".to_string() });
}

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
        "control": [{"faction_id": "中国",
            "ship_doctrine": [
                {"ship": "长城", "temper": -1.0, "lone_wolf": 3.0},
                {"ship": "华盛顿", "temper": 1.0}
            ],
            "ship_kiting": [
                {"ship": "长城", "kiting": -0.5},
                {"ship": "华盛顿", "kiting": 1.0}
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
    assert_eq!(state.ship_doctrine_control("长城".to_string()), ControlMode::Player);
    assert_eq!(state.ship_kiting_control("长城".to_string()), ControlMode::Player);
    // 华盛顿 belongs to 美国, not 中国 → the 中国 patch must be a no-op.
    assert_eq!(
        state.ship_doctrine("华盛顿".to_string()).temper,
        0.0,
        "other-faction ship must be untouched"
    );
    assert_eq!(state.ship_kiting("华盛顿".to_string()), 0.0, "other-faction ship must be untouched");
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
        "control": [{"faction_id": "中国", "default_kiting": {"kiting": -1.0}}]
    });
    apply_patch(&mut state, &config, &diff).expect("default style applies");
    assert_eq!(state.ship_kiting(ship.clone()), -1.0, "没有叶片的舰必须跟随舰队默认");
    assert_eq!(
        state.ship_kiting_control(ship.clone()),
        ControlMode::Player,
        "只写值不写 mode ⇒ 接管（与其它叶同一条规则）"
    );

    // 2) 单舰特例（更具体的层）优先。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "ship_kiting": [{"ship": ship, "kiting": 1.0, "mode": "Player"}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("per-ship override applies");
    assert_eq!(state.ship_kiting(ship.clone()), 1.0, "单舰叶片比舰队默认更具体");

    // 3) 把叶片交回上层（Inherit）⇒ 又回到舰队默认的值。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "ship_kiting": [{"ship": ship, "mode": "Inherit"}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("release applies");
    assert_eq!(state.ship_kiting(ship.clone()), -1.0, "叶 Inherit + 舰队默认 Player ⇒ 取默认值");
}

/// 舰队默认指令（`default_ship_order`）：**新舰出生就有意图**，而且**一个叶片改全舰队**。
///
/// 这是 note `agent-control-long-game.md` §5 的正解：以前新下水的舰不在任何 diff 里
/// → 默认归系统 → 玩家每段都要重新枚举活舰名（而舰名会换代）。现在归属与意图都在
/// 更宽的那一层有答案，点名单舰只剩「例外」一种用途。
#[test]
fn fleet_default_order_covers_new_ships() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();

    // 造一艘锚点：舰队默认是**势力级**的，所以先只对「没有任何叶片的舰」验证语义。
    // 拿一艘中国的舰、**删掉它的叶片**来模拟「刚下水、还没人点名」。
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("a chinese ship");
    state.control_mut(fid.clone()).expect("control").ship_orders.remove(&ship);
    assert_eq!(state.ship_control(ship.clone()), ControlMode::Auto, "no leaf, no default → system");
    assert_eq!(state.ship_behavior(ship.clone()), None, "no leaf, no default → no order at all");

    // 写一个舰队默认（不带 mode → 写值即接管 = Player）。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "default_ship_order": {"behavior": {"type": "dock", "body": "地球"}}
        }]
    });
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");

    assert_eq!(
        state.ship_control(ship.clone()),
        ControlMode::Player,
        "a ship with no leaf inherits the faction default's ownership"
    );
    assert_eq!(
        state.ship_behavior(ship.clone()),
        Some(ShipBehavior::Dock { body: "地球".to_string() }),
        "…and its intent (this is the whole point: the new ship has orders without being named)"
    );

    // 单舰特例仍然压过舰队默认（更具体的层优先）——而且这是「改主意」的批量手段：
    // 改**一个**势力级叶片 = 全舰队改主意（B 不需要了）。
    let batch = serde_json::json!({
        "control": [{"faction_id": "中国",
            "default_ship_order": {"behavior": {"type": "idle"}, "mode": "Player"}
        }]
    });
    apply_patch(&mut state, &config, &batch).expect("fleet default retarget applies");
    assert_eq!(state.ship_behavior(ship.clone()), Some(ShipBehavior::Idle), "one leaf, whole fleet");

    let exception = serde_json::json!({
        "control": [{"faction_id": "中国",
            "ship_orders": [{"ship": ship.clone(), "behavior": {"type": "colonize", "body": "火星"}, "mode": "Player"}]
        }]
    });
    apply_patch(&mut state, &config, &exception).expect("per-ship exception applies");
    assert_eq!(
        state.ship_behavior(ship.clone()),
        Some(ShipBehavior::Colonize { body: "火星".to_string() }),
        "a named ship overrides the fleet default"
    );

    // 舰队默认读面即写面：`--control` 里看得见它，且值能原样回传。
    let view = control_view(&state, &config, fid.clone(), state.control(fid.clone()).expect("control"));
    let d = view.default_ship_order.expect("the fleet default is part of the read surface");
    assert_eq!(d.mode, Some(ControlMode::Player));
    assert_eq!(d.behavior, Some(ShipBehavior::Idle));
}

/// 「写值即接管」：只写值、不写 mode 的 diff 必须真的生效（而不是被系统下一回合
/// 按自己的逻辑覆盖掉），并在回执里有一条 `took_over` 记录。
#[test]
fn writing_a_value_without_mode_takes_over() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);

    // 不带 mode 写一条舰指令 + 一条预算。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "ship_orders": [{"ship": "长城", "behavior": {"type": "dock", "body": "地球"}}],
            "construction_budget": [{"resource": "铁", "value": 3.5}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff applies");

    assert_eq!(state.ship_control("长城".to_string()), ControlMode::Player, "a written value is an order");
    assert_eq!(
        state.ship_behavior("长城".to_string()),
        Some(ShipBehavior::Dock { body: "地球".to_string() })
    );
    assert_eq!(
        state.construction_budget_control("中国".to_string(), "铁"),
        ControlMode::Player,
        "same rule for budgets: the value would otherwise be recomputed away"
    );
    assert_eq!(report.took_over.len(), 2, "the receipt must name every implicitly taken-over leaf: {:?}", report.took_over);
    assert!(report.took_over.iter().any(|p| p.contains("ship_orders[0].behavior")), "{:?}", report.took_over);
    assert!(report.took_over.iter().any(|p| p.contains("construction_budget[0].value")), "{:?}", report.took_over);

    // 显式写 `Inherit` 仍然能把叶片交还给作用域链（这不是接管，是撤销表态）。
    let give_back = serde_json::json!({
        "control": [{"faction_id": "中国",
            "ship_orders": [{"ship": "长城", "mode": "Inherit"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &give_back).expect("diff applies");
    assert!(report.took_over.is_empty(), "an explicit mode is not a takeover: {:?}", report.took_over);
    assert_eq!(state.ship_control("长城".to_string()), ControlMode::Auto, "Inherit hands it back to the system");
}

/// Setting a faction's scope to Player must actually take over its leaves
/// (which are inert `inherit` now), even though the AI has "written" values.
#[test]
fn scope_player_takes_over_independent_faction() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    // Default scope (all Inherit) → everything resolves to Auto.
    assert_eq!(state.ship_control("长城".to_string()), ControlMode::Auto, "default scope is Auto");

    // Take over faction 中国 via a scope-only diff.
    let scope_diff = serde_json::json!({"scope": {"factions": [["中国", "Player"]]}});
    apply_patch(&mut state, &config, &scope_diff).expect("scope diff applies");
    assert_eq!(state.ship_control("长城".to_string()), ControlMode::Player, "ship of a Player faction is player-owned");
    assert_eq!(state.investment_budget_control("中国".to_string(), "铁"), ControlMode::Player, "budget leaf follows scope");
    assert_eq!(state.construction_budget_control("中国".to_string(), "铁"), ControlMode::Player);
    // Other factions are untouched (still Auto): 华盛顿 is a US ship (美国).
    assert_eq!(state.ship_control("华盛顿".to_string()), ControlMode::Auto, "untouched faction stays Auto");

    // An explicit leaf mode still overrides scope in the opposite direction:
    // hand 长城 back to the system inside a Player faction.
    let leaf_diff = serde_json::json!({
        "control": [{"faction_id": "中国", "ship_orders": [{"ship": "长城", "mode": "Auto"}]}]
    });
    apply_patch(&mut state, &config, &leaf_diff).expect("leaf diff applies");
    assert_eq!(state.ship_control("长城".to_string()), ControlMode::Auto, "explicit Auto leaf beats Player scope");

    // …and an explicit `Inherit` leaf un-does that override, falling back to scope.
    let back = serde_json::json!({
        "control": [{"faction_id": "中国", "ship_orders": [{"ship": "长城", "mode": "Inherit"}]}]
    });
    apply_patch(&mut state, &config, &back).expect("inherit leaf applies");
    assert_eq!(state.ship_control("长城".to_string()), ControlMode::Player, "Inherit leaf falls back to the faction scope");
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
    assert_eq!(from_json("\"Ai\""), ControlMode::Auto, "Ai is the pre-rename spelling of Auto");
    assert_eq!(from_json("\"Auto\""), ControlMode::Auto);
    assert_eq!(from_json("\"Inherit\""), ControlMode::Inherit);
    assert_eq!(from_json("\"Player\""), ControlMode::Player);
    assert!(serde_json::from_str::<ControlMode>("\"玩家\"").is_err(), "unknown spellings must fail loudly");

    // RON 存档里叶子写的是 `mode: None` / `Some(Ai)` / `Some(Player)`。
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Leaf {
        #[serde(default)]
        mode: ControlMode,
    }
    let load = |s: &str| -> ControlMode { ron::from_str::<Leaf>(s).expect("legacy leaf must load").mode };
    assert_eq!(load("(mode: None)"), ControlMode::Inherit, "legacy `None` = Inherit");
    assert_eq!(load("(mode: Some(Ai))"), ControlMode::Auto, "legacy `Some(Ai)` = Auto");
    assert_eq!(load("(mode: Some(Player))"), ControlMode::Player, "legacy `Some(Player)` = Player");
    assert_eq!(load("(mode: Some(Inherit))"), ControlMode::Inherit, "new spelling, same `Some(..)` shell");
    assert_eq!(load("()"), ControlMode::Inherit, "missing field defaults to Inherit");
    // 手写 `.ron` 也必须带 `Some(..)` 外壳（RON 里 `deserialize_option` 不认裸标识符）；
    // 这是**有意的**：JSON 是写面，`.ron` 只由 `--save` 写。写错了会当场报 ExpectedOption，
    // 而不是静默当成 Inherit。
    assert!(ron::from_str::<Leaf>("(mode: Player)").is_err(), "a bare RON identifier must fail loudly");

    // 作用域树同理：整棵树（含旧的 `global: None` 与 `Some(x)` 键值）必须能读。
    let scope: crate::model::ControlScope = ron::from_str(
        r#"(global: None, factions: {"中国": Some(Player), "美国": None}, bodies: {}, cities: {})"#,
    )
    .expect("legacy scope tree must load");
    assert_eq!(scope.global, ControlMode::Inherit);
    assert_eq!(scope.factions.get("中国").copied(), Some(ControlMode::Player));
    assert_eq!(scope.factions.get("美国").copied(), Some(ControlMode::Inherit));

    // 写面（线格式）：JSON 是干净的字符串三态，RON 是与旧档同形的 `Some(标识符)`
    // —— 而且**自己写的必须读得回来**（RON 的裸标识符 vs 引号字符串在这里是坑）。
    for m in [ControlMode::Inherit, ControlMode::Auto, ControlMode::Player] {
        let json = serde_json::to_string(&Leaf { mode: m }).expect("json");
        assert_eq!(json, format!("{{\"mode\":\"{}\"}}", m.name()), "JSON read面 must be a bare three-state string");
        assert_eq!(serde_json::from_str::<Leaf>(&json).expect("json back").mode, m);

        let text = ron::to_string(&Leaf { mode: m }).expect("ron");
        assert_eq!(text, format!("(mode:Some({}))", m.name()), "RON must stay `Some(<ident>)`, same shape as legacy");
        assert_eq!(ron::from_str::<Leaf>(&text).expect("ron back").mode, m, "own checkpoint must load back: {text}");
    }
}

/// 娱乐/福利预算：一座城的忠诚度投入是一个可控叶子。按 Player 覆盖后，治理模型
/// 会读取它；省略 value 时保留当前值、省略 mode 时保留当前模式。
#[test]
fn apply_loyalty_budget_patch() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "loyalty_budget": [{"city": "长三角", "value": 40.0, "mode": "Player"}]}]
    });
    apply_patch(&mut state, &config, &diff).expect("loyalty budget diff applies");
    assert_eq!(state.loyalty_budget_control("中国".to_string(), "长三角".to_string()), ControlMode::Player);
    let v = state
        .control("中国".to_string())
        .and_then(|c| c.loyalty_budget.get("长三角"))
        .map(|c| c.value)
        .unwrap_or(f64::NAN);
    assert!((v - 40.0).abs() < 1e-7, "loyalty budget value should be 40.0, got {v}");
}

// --- the apply report: "did my diff actually land?" ---------------------
//
// 这组守卫钉住的是**可观测性**，不是模拟：一个 diff 的叶片被丢掉时，退出码
// 与 stdout 与成功落地完全一样，所以「有没有报出来」是 agent 唯一的信号。

/// 一条全合法的 diff 必须报 `applied > 0` 且**没有**任何丢弃——否则
/// `WARN_APPLY_SKIPPED` 会变成噪声，agent 学会无视它，等于没做。
#[test]
fn a_valid_diff_reports_clean_and_counts_every_leaf() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "ship_orders": [{"ship": "长城", "behavior": {"type": "dock", "body": "地球"}, "mode": "Player"}],
            "loyalty_budget": [{"city": "长三角", "value": 2.0, "mode": "Player"}],
            "construction_budget": [{"resource": "铁", "value": 1.0, "mode": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("valid diff applies");
    assert!(report.is_clean(), "a valid diff must report nothing skipped: {:?}", report.skipped);
    assert_eq!(report.applied, 3, "one leaf per order / budget / loyalty entry");
    assert_eq!(
        state.ship_behavior("长城".to_string()),
        Some(ShipBehavior::Dock { body: "地球".to_string() })
    );
}

/// **已战沉 / 已改名**的舰：从前是静默 no-op（退出码 0，看不出来）。现在必须
/// 报出来，并点名到叶、给出原因码——这是 agent 发现「命令没下达」的唯一渠道。
#[test]
fn a_vanished_ship_is_reported_not_dropped_silently() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "ship_orders": [{"ship": "长城2", "behavior": {"type": "idle"}, "mode": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped.len(), 1, "the dropped order must be reported: {report:?}");
    let s = &report.skipped[0];
    assert_eq!(s.code, "no_such_ship");
    assert_eq!(s.value, "长城2");
    assert!(s.path.contains("ship_orders[0]"), "path must point at the leaf: {}", s.path);
}

/// 别人的舰：与「查无此舰」必须分开报（一个要改名、一个是写错势力）。
#[test]
fn another_factions_ship_is_reported_with_its_own_code() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "ship_orders": [{"ship": "华盛顿", "behavior": {"type": "idle"}, "mode": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped[0].code, "not_your_ship");
    assert!(report.skipped[0].reason.contains("美国"), "reason should name the real owner");
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
        "control": [{"faction_id": "中国洋", "construction_budget": [{"resource": "铁", "value": 9.9, "mode": "Player"}]}]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped[0].code, "no_such_faction");
    assert_eq!(state.control.len(), before, "a typo must not create a control entry");
    assert!(state.control.get("中国洋").is_none(), "no phantom faction");
    // 而且它绝不能出现在读面（模板）里——那正是它从前最有害的地方。
    let surface = control_surface(&state, &config);
    let ids: Vec<&str> = surface["control"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["faction_id"].as_str())
        .collect();
    assert!(!ids.contains(&"中国洋"), "phantom faction leaked into the template: {ids:?}");
}

/// `building` 是 u32 下标、只在城内部唯一，所以「另一座城的合法下标」在本城
/// 是错的。这条从前静默 no-op，现在报出**该城真实的下标列表**——agent 手写
/// 权重时最常踩的一脚。
#[test]
fn a_building_index_from_another_city_is_reported_with_the_real_indices() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "build_weights": [{"city": "长三角", "building": 21, "value": 0.5, "mode": "Player"}]
        }]
    });
    let report = apply_patch(&mut state, &config, &diff).expect("diff itself is valid");
    assert_eq!(report.applied, 0);
    assert_eq!(report.skipped[0].code, "no_such_building");
    assert!(report.skipped[0].reason.contains('0'), "reason should list the real indices: {}", report.skipped[0].reason);
}

/// 未知**字段名**（`ship_order` 少个 s）从前被 serde 静默忽略 → 整条意图蒸发、
/// 退出码 0。现在当场报错并列出合法字段。
#[test]
fn a_misspelled_control_field_is_rejected_rather_than_ignored() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "ship_order": [{"ship": "长城", "behavior": {"type": "idle"}, "mode": "Player"}]
        }]
    });
    let err = apply_patch(&mut state, &config, &diff).expect_err("a typo'd field must be an error");
    assert!(err.contains("ship_order"), "error must name the bad field: {err}");
    assert!(err.contains("ship_orders"), "error must list the legal fields: {err}");
}

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
        s.doctrine = ShipDoctrine { temper: 0.71, lone_wolf: -0.2 };
    }
    let record = state.ship(&ship).unwrap().doctrine;

    // ① 写一片逐舰风格叶：有效值 = 叶里的值，这片叶**钉住**了它。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "temper": -1.0, "lone_wolf": 0.5}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(state.ship_doctrine(ship.clone()).temper, -1.0);

    // ② 「恢复继承」（只写 mode）撤不掉那个数：叶还在，取值优先用叶里的值。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "mode": "Inherit"}]}]
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
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(r.removed.len(), 1, "真的删掉了要留一条 NOTE_APPLY_REMOVED 回执：{:?}", r.removed);
    assert!(r.removed[0].contains("ship_doctrine"), "{:?}", r.removed);
    assert_eq!(state.ship_doctrine(ship.clone()), record, "删叶之后有效风格必须回到出厂快照");
    assert!(
        state.control.get(&fid).and_then(|c| c.ship_doctrine.get(&ship)).is_none(),
        "叶必须真的没了"
    );

    // ④ 幂等：再删一次不报错、不算丢弃、也不进 `removed`（目标状态已经达成）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert!(r.removed.is_empty(), "删一片本来就不存在的叶不进回执：{:?}", r.removed);
    assert_eq!(r.applied, 1, "但它是**成功的**（目标状态达成），不是被丢弃");

    // ⑤ `remove` 与值同时出现 ⇒ **拒绝**（任何一种静默优先级都会让人误判另一件事发生了）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true, "temper": 0.5}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert_eq!(r.skipped.len(), 1, "{:?}", r);
    assert_eq!(r.skipped[0].code, "remove_conflicts_with_value");
    assert_eq!(state.ship_doctrine(ship.clone()), record, "被拒绝的补丁一个字节都不许动");
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
        "control": [{"faction_id": fid, "default_doctrine": {"temper": 0.4, "lone_wolf": -0.6, "mode": "Player"}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!((state.ship_doctrine(ship.clone()).temper, state.ship_doctrine(ship.clone()).lone_wolf), (0.4, -0.6));

    // 删掉这片默认叶 ⇒ 这一层不再供值（回落到舰上记录值 / 作用域链）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "default_doctrine": {"remove": true}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
    assert!(state.control.get(&fid).and_then(|c| c.default_doctrine.as_ref()).is_none());
    let rec = state.ship(&ship).unwrap().doctrine;
    assert_eq!(state.ship_doctrine(ship.clone()), rec, "默认叶没了 ⇒ 回落到舰上记录值");

    // 舰已不在（战沉/换代）：它的陈叶仍然能被删掉——删的是**控制面**里的叶，不要求实体还在。
    state.ships.retain(|s| s.name != ship);
    state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_doctrine
        .insert(ship.clone(), Control::inherit(ShipDoctrine { temper: 0.9, lone_wolf: 0.9 }));
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "remove": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean(), "删陈叶不该因为舰没了而被丢弃：{:?}", r.skipped);
    assert_eq!(r.removed.len(), 1, "陈叶也是真的被删掉了：{:?}", r.removed);

    // 预算叶与迁都叶：同一套 `remove` 语义（这里只钉"删得掉"，值语义由各自的取值规则决定）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid,
            "investment_budget": [{"resource": "铁", "value": 3.0}],
            "capital": {"value": "地球", "mode": "Player"}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let diff = serde_json::json!({
        "control": [{"faction_id": fid,
            "investment_budget": [{"resource": "铁", "remove": true}],
            "capital": {"remove": true}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(r.removed.len(), 2, "{:?}", r.removed);
    let c = state.control.get(&fid).expect("control");
    assert!(c.investment_budget.get("铁").is_none() && c.capital.is_none());
}

/// **第三条风格轴（角色）也守同一套删叶规矩**——并且它有一条另两条轴没有的含义：
/// 删叶 = **交回自动定编**（`Inherit` 之下 AI 下回合可以立刻又写下结论），而不是
/// 「从此保持某个值」（要后者得写 `Player`）。
#[test]
fn the_role_axis_obeys_the_same_delete_rules() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let fid = "中国".to_string();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("中国至少有一艘舰");
    // 出厂记录值给成 `true`：否则「删叶回到记录值」与「钉在 false」分不出来。
    for s in state.ships.iter_mut().filter(|s| s.faction_id == fid) {
        s.freighter = true;
    }

    // ① 逐舰角色叶（玩家钉「打仗」）：有效值 = 叶里的值，归属 = Player（AI 从此不许碰）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "freighter": false}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert!(!state.ship_freighter(ship.clone()));
    assert_eq!(state.ship_freighter_control(ship.clone()), ControlMode::Player);

    // ② 删叶 ⇒ 回到出厂记录值，叶真的没了，并且**交回自动定编**（归属不再是 Player）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean(), "{:?}", r.skipped);
    assert_eq!(r.removed.len(), 1, "{:?}", r.removed);
    assert!(r.removed[0].contains("ship_freighter"), "{:?}", r.removed);
    assert!(state.ship_freighter(ship.clone()), "删叶之后回落到出厂记录值 true");
    assert!(
        state.control.get(&fid).and_then(|c| c.ship_freighter.get(&ship)).is_none(),
        "叶必须真的没了"
    );
    assert_ne!(
        state.ship_freighter_control(ship.clone()),
        ControlMode::Player,
        "删叶 = 交回自动定编：AI 下回合作出的结论可以再写进这片叶"
    );

    // ③ 幂等：再删一次仍然**成功**（目标状态已达成），但不进回执、不算丢弃。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean() && r.removed.is_empty() && r.applied == 1, "{:?}", r);

    // ④ `remove` 带值 / 带归属 ⇒ 拒绝（删与写是两件事）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true, "freighter": false}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert_eq!(r.skipped.len(), 1, "{:?}", r);
    assert_eq!(r.skipped[0].code, "remove_conflicts_with_value");
    assert!(state.ship_freighter(ship.clone()), "被拒绝的补丁一个字节都不许动");

    // ⑤ 势力级默认角色叶：`Player` 时它的值压过叶片值（AI 定编的闸门）；删掉它 ⇒ 不再供值。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "default_freighter": {"freighter": false, "mode": "Player"}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert!(!state.ship_freighter(ship.clone()), "舰队默认是 Player ⇒ 它的值说了算");
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "default_freighter": {"remove": true}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
    assert!(r.removed[0].contains("default_freighter"), "{:?}", r.removed);
    assert!(state.ship_freighter(ship.clone()), "默认叶没了 ⇒ 回落到舰上记录值 true");

    // ⑥ 陈叶（舰已不在）照删不误：与另两条轴同一条规矩。
    state.ships.retain(|s| s.name != ship);
    state
        .control
        .entry(fid.clone())
        .or_default()
        .ship_freighter
        .insert(ship.clone(), Control::inherit(true));
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_freighter": [{"ship": ship, "remove": true}]}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean(), "删陈叶不该因为舰没了而被丢弃：{:?}", r.skipped);
    assert_eq!(r.removed.len(), 1, "{:?}", r.removed);
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
        "control": [{"faction_id": fid, "default_doctrine": {"temper": 0.4}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(r.skipped.len(), 1, "{:?}", r);
    assert_eq!(r.skipped[0].code, "partial_doctrine_leaf");
    assert!(r.skipped[0].reason.contains("两条轴"), "拒绝理由要给改法：{}", r.skipped[0].reason);
    assert!(
        state.control.get(&fid).and_then(|c| c.default_doctrine.as_ref()).is_none(),
        "被拒绝的补丁不许留下半片叶"
    );

    // ② 只写 `mode`（先表态归属）合法 —— 值那两条轴暂时都是 0.0（引擎的 `ShipDoctrine::default()`）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "default_doctrine": {"mode": "Player"}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean() && r.took_over.is_empty(), "只写 mode 不是接管：{:?}", r.took_over);
    let leaf = state.control[&fid].default_doctrine.clone().expect("叶建出来了");
    assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.0, 0.0));

    // ③ 叶已存在 ⇒ 单轴写合法，缺省轴保留现值。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "default_doctrine": {"lone_wolf": -0.5}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(r.is_clean(), "{:?}", r.skipped);
    let leaf = state.control[&fid].default_doctrine.clone().expect("叶还在");
    assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.0, -0.5));

    // ④ 删掉之后"叶不存在"这条状态又回来了 ⇒ 再单轴写还是被拒（守卫看的是存在性，不是次数）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "default_doctrine": {"remove": true}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().removed.len() == 1);
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "default_doctrine": {"lone_wolf": -0.5}}]
    });
    let r = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(r.skipped.len(), 1, "{:?}", r);
    assert_eq!(r.skipped[0].code, "partial_doctrine_leaf");
}

/// 逐舰**两轴叶**的单轴写：缺的那条轴种的是**这艘舰当时在用的那一条**——没有舰队默认时
/// 正是出厂记录值（`0.71`），有玩家默认时是默认值（界面上显示的就是它）。**绝不是一个
/// 凭空来的 `0.0`**（那正是 §3.1 那个坑的形态）。
#[test]
fn a_single_axis_ship_leaf_seeds_the_other_axis_from_what_is_in_use() {
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
        s.doctrine = ShipDoctrine { temper: 0.71, lone_wolf: -0.2 };
    }

    // 没有舰队默认 ⇒ 缺省轴 = 出厂记录值。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "temper": 0.5}]}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let leaf = state.control[&fid].ship_doctrine[&ship].clone();
    assert_eq!((leaf.value.temper, leaf.value.lone_wolf), (0.5, -0.2), "缺省轴要种出厂记录值，不是 0.0");

    // 舰队默认是玩家表态 ⇒ 缺省轴 = **当时在用的那个数**（界面上显示的就是它）。
    let diff = serde_json::json!({
        "control": [{"faction_id": fid,
            "ship_doctrine": [{"ship": ship, "remove": true}],
            "default_doctrine": {"temper": 0.1, "lone_wolf": 0.9, "mode": "Player"}}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let diff = serde_json::json!({
        "control": [{"faction_id": fid, "ship_doctrine": [{"ship": ship, "temper": 0.5}]}]
    });
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    let leaf = state.control[&fid].ship_doctrine[&ship].clone();
    assert_eq!(
        (leaf.value.temper, leaf.value.lone_wolf),
        (0.5, 0.9),
        "船正在跟随舰队默认 ⇒ 另一条轴种的是默认值（UI 上显示的数）"
    );
}

// ---- 舰船设计图（blueprint）的写面/读面契约 -------------------------------

/// 某势力第一座城的某个建造区：`(城名, 建筑下标, 舰级, 是不是建造区)`。
fn some_building(state: &State, fid: &str, shipyard: bool) -> (CityId, BuildingId, String) {
    for c in state.cities.iter().filter(|c| c.faction_id == fid) {
        for b in &c.buildings {
            if b.is_shipyard() == shipyard {
                return (c.name.clone(), b.id, b.kind.clone());
            }
        }
    }
    panic!("{fid} 没有 {} 的建筑", if shipyard { "建造区" } else { "非建造区" });
}

/// 建一张图 + 把它挂到某个建造区上（两个写面动作合并成一份 diff：这是正解用法）。
fn pin_blueprint(
    state: &mut State,
    config: &GameConfig,
    name: &str,
    class: &str,
    components: &serde_json::Value,
    mode: &str,
) -> (CityId, BuildingId) {
    let (cid, bid, _) = some_building(state, "中国", true);
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "blueprints": [{"name": name, "class": class, "components": components, "mode": mode}],
        "buildings": [{"city": cid, "building": bid, "ship_type": class, "blueprint": name}],
    }]});
    let rep = apply_patch(state, config, &diff).expect("建图 + 挂图必须一次成功");
    assert!(rep.is_clean(), "正解用法不该被丢弃：{:?}", rep.skipped);
    (cid, bid)
}

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
        assert!(hit.path.contains("blueprint"), "{code} 要点名到叶，got {}", hit.path);
        assert!(!hit.reason.is_empty(), "{code} 要有一句人读的理由");
    };

    // ① 建造区指向一张**不存在**的图（悬空指针）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
        {"city": cid, "building": bid, "blueprint": "没有这张图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_blueprint");
    assert_eq!(
        state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().blueprint,
        None,
        "被丢弃的指针不许落地"
    );

    // ② 图名不存在 + 只写 mode ⇒ 同样 `no_such_blueprint`（不许凭空造图）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "没有这张图", "mode": "Auto"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_blueprint");
    assert!(
        !state.control["中国"].blueprints.contains_key("没有这张图"),
        "错别字不许造出一张谁都不认识的图（防幽灵图）"
    );

    // ③ 舰级对不上：图是 cruiser，建造区是 corvette。
    let (cid2, bid2, st2) = some_building(&state, "中国", true);
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "blueprints": [{"name": "巡洋图", "class": "cruiser", "components": [], "mode": "Player"}],
        "buildings": [{"city": cid2, "building": bid2, "blueprint": "巡洋图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    // 建图本身落地了（`applied`），只有指针被丢。
    skip(&rep, "blueprint_class_mismatch");
    assert!(st2.is_empty() || state.control["中国"].blueprints.contains_key("巡洋图"));
    assert_eq!(
        state.city(&cid2).unwrap().buildings.iter().find(|b| b.id == bid2).unwrap().blueprint,
        None,
        "对不上的指针不许落地（口径 A）"
    );

    // ④ 不存在的组件。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "坏图", "class": "corvette", "components": ["没有这个组件"], "mode": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_component");
    assert!(!state.control["中国"].blueprints.contains_key("坏图"), "被拒的图不许污染库");

    // ⑤ 组件重复。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "双炮图", "class": "corvette", "components": ["kinetic", "kinetic"], "mode": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "duplicate_component");
    assert!(!state.control["中国"].blueprints.contains_key("双炮图"));

    // ⑥ 超过槽位（corvette 只有 2 个槽）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "超载图", "class": "corvette",
         "components": ["kinetic", "ion_drive", "shield"], "mode": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "too_many_components");
    assert!(!state.control["中国"].blueprints.contains_key("超载图"));

    // ⑦ 建图没给舰级 / 舰级不存在。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "无级图", "components": ["kinetic"], "mode": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "missing_class");
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "怪级图", "class": "无畏舰", "components": [], "mode": "Player"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "no_such_class");

    // ⑧ 把图挂到**非建造区**上。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "blueprints": [{"name": "民用图", "class": "corvette", "components": [], "mode": "Player"}],
        "buildings": [{"city": rcid, "building": rbid, "blueprint": "民用图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    skip(&rep, "not_a_shipyard");
}

/// **写值即接管**：只写 `components` ⇒ 图叶变 `Player` 并记一笔（§7.3-11）。
#[test]
fn writing_a_blueprint_value_without_mode_takes_over() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "我的图", "class": "corvette", "components": ["kinetic", "ion_drive"]}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    let leaf = state.control["中国"].blueprints["我的图"].clone();
    assert_eq!(leaf.mode, ControlMode::Player, "只写值 ⇒ 这一层接管（免得「我写了图却没生效」）");
    assert!(rep.took_over.iter().any(|p| p.contains("blueprints[0]")), "{:?}", rep.took_over);
    assert_eq!(state.blueprint_control(&"中国".to_string(), &"我的图".to_string()), ControlMode::Player);

    // 只写 `mode` 合法（值不动）——交回系统重估。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "我的图", "mode": "Auto"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    let leaf = state.control["中国"].blueprints["我的图"].clone();
    assert_eq!(leaf.mode, ControlMode::Auto);
    assert_eq!(leaf.value.components, vec!["kinetic".to_string(), "ion_drive".to_string()], "值不动");
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
    crate::sim::spawn_ship(&mut state, &config, crate::sim::ShipSpawn {
        owner: "中国".to_string(),
        class: "corvette",
        position: pos,
        city: None,
        via: SpawnVia::Shipyard,
        pay_components: false,
        blueprint: Some(&bp),
    });

    let mut surface = control_surface(&state, &config);
    let row = surface["control"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["faction_id"] == "中国")
        .and_then(|f| f["blueprints"].as_array())
        .and_then(|b| b.first())
        .cloned()
        .expect("读面必须给出蓝图片");
    assert_eq!(row["components"], serde_json::json!(["kinetic", "ion_drive"]), "选装要**全量**输出（少输出 = 回传时清空）");
    assert_eq!(row["ship_count"], serde_json::json!(1), "读面附加：本图造了多少艘");
    assert_eq!(row["launch_waiting"], serde_json::json!(false), "读面附加：这张图此刻没人在等钱");
    assert!(row["order"].is_null(), "本图对意图没有说话 ⇒ null（不是缺字段）");

    for fac in surface["control"].as_array_mut().unwrap() {
        fac.as_object_mut().unwrap().insert("buildings".to_string(), serde_json::json!([]));
    }
    let rep = apply_patch(&mut state, &config, &surface).expect("模板回传必须合法");
    assert!(rep.is_clean(), "模板回传不许丢叶：{:?}", rep.skipped);
    assert!(rep.applied >= 40, "整面模板要触碰很多叶，got {}", rep.applied);
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
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "blueprints": [{"name": "会被删的图", "remove": true}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    assert!(rep.removed.iter().any(|p| p.contains("blueprints")), "{:?}", rep.removed);
    assert!(!state.control["中国"].blueprints.contains_key("会被删的图"));

    // 读面（web/--control 的 `buildings` 是结构补丁面，指针在 state 里读）**原样**输出。
    let b = state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap();
    assert_eq!(b.blueprint.as_deref(), Some("会被删的图"), "指针原样保留（读面据此看出「这个区指着不存在的图」）");

    // 再有人写这个指针 ⇒ 响亮报出来。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
        {"city": cid, "building": bid, "blueprint": "会被删的图"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(rep.skipped[0].code, "no_such_blueprint", "{:?}", rep.skipped);

    // 拆指针是**另一件事**（回到 `ship_type` + 生成器）：`null` 与缺席必须分得开。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
        {"city": cid, "building": bid, "blueprint": null}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "{:?}", rep.skipped);
    assert_eq!(
        state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().blueprint,
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
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "护卫图", "class": "cruiser"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(rep.skipped[0].code, "blueprint_class_mismatch", "{:?}", rep.skipped);
    assert_eq!(state.control["中国"].blueprints["护卫图"].value.class, "corvette", "被拒 ⇒ 状态不动");

    // ② 只改建造区 ⇒ 同样拒绝（另一条路，堵一条没用）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "buildings": [
        {"city": cid, "building": bid, "ship_type": "cruiser"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert_eq!(rep.skipped[0].code, "blueprint_class_mismatch", "{:?}", rep.skipped);
    assert_eq!(
        state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().ship_type.as_deref(),
        Some("corvette"),
        "被拒 ⇒ 建造区不动"
    );

    // ③ **两处一起写** ⇒ 一次成功（正解）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "blueprints": [{"name": "护卫图", "class": "cruiser", "components": []}],
        "buildings": [{"city": cid, "building": bid, "ship_type": "cruiser"}]}]});
    let rep = apply_patch(&mut state, &config, &diff).unwrap();
    assert!(rep.is_clean(), "两处一起写必须一次成功：{:?}", rep.skipped);
    assert_eq!(state.control["中国"].blueprints["护卫图"].value.class, "cruiser");
    assert_eq!(
        state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().ship_type.as_deref(),
        Some("cruiser")
    );
}

/// 「让图的**意图轴**沉默」（`order: null`）与「删掉这张图」（`remove: true`）是两件事。
#[test]
fn silencing_the_order_axis_is_not_deleting_the_blueprint() {
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
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "护卫-守家", "order": {"type": "dock", "body": "地球"}}]}]});
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(
        state.control["中国"].blueprints["护卫-守家"].value.order,
        Some(ShipBehavior::Dock { body: "地球".to_string() }),
        "tagged 写法的意图要被认下来（与 default_ship_order.behavior 同一套）"
    );

    // 清空意图轴：图还在、指针还在，只是这一层不再说话。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "护卫-守家", "order": null}]}]});
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(state.control["中国"].blueprints["护卫-守家"].value.order, None, "意图轴沉默");
    assert!(state.control["中国"].blueprints.contains_key("护卫-守家"), "图还在");
    assert_eq!(
        state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().blueprint.as_deref(),
        Some("护卫-守家"),
        "指针还在（= 选装仍按图装配，只有意图那一层交还给下层）"
    );
}

/// 图的**有效归属**沿 scope 链上溯（图叶 → 势力 → 全局）：势力设成 `Player` 也能接管。
#[test]
fn blueprint_ownership_follows_the_scope_chain() {
    let config = crate::config::load_config();
    let mut state = crate::world::default_state(&config, 42);
    state.control.entry("中国".to_string()).or_default().blueprints.insert(
        "种子图".to_string(),
        Control::inherit(Blueprint { class: "corvette".to_string(), components: vec![], order: None }),
    );
    let fid = "中国".to_string();
    assert_eq!(state.blueprint_control(&fid, &"种子图".to_string()), ControlMode::Auto, "全链继承 ⇒ Auto");
    let diff = serde_json::json!({"scope": {"factions": [["中国", "Player"]]}});
    assert!(apply_patch(&mut state, &config, &diff).unwrap().is_clean());
    assert_eq!(
        state.blueprint_control(&fid, &"种子图".to_string()),
        ControlMode::Player,
        "势力的 scope 表态也要能接管设计图（与其它叶同一条链的语义）"
    );
}
