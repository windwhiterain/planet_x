//! 舰队行为与默认：舰队默认管新舰、殖民归属、陈旧 follow 退化成 idle、跟随友舰时自动开火只打敌对者、回合事件日志、dock/idle 的位姿。

use super::*;

/// 舰队默认指令要真的管住**新造出来的舰**：它出厂时没有任何指令叶片（不点名 = 不在
/// 任何 diff 里），但不能因此默认归系统、被 AI 拿去远征或停在 Idle —— 它应当直接执行
/// 势力的默认意图。
///
/// 这条是 note `agent-control-long-game.md` §5 的端到端守卫（控制面单测在
/// `control::tests::fleet_default_order_covers_new_ships`）。
#[test]
fn fleet_default_governs_newly_built_ships() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国".to_string();
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国",
            "default_ship_order": {"behavior": {"type": "dock", "body": "地球"}}
        }]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("fleet default applies");

    // 与船坞出厂同一条漏斗造一艘新舰（不带指令叶片）。
    let pos = state.body_position("水星");
    let name = spawn_ship(
        &mut state,
        &config,
        ShipSpawn {
            owner: fid.clone(),
            class: "corvette",
            position: pos,
            city: None,
            via: SpawnVia::Shipyard,
            pay_components: false,
            blueprint: None,
        },
    );
    // 出厂时 `spawn_ship` 给它一条**没有说话**（`Inherit`）的叶片——它不在玩家的任何
    // diff 里，所以「谁负责、干什么」只能由更宽的那一层回答。
    let leaf = state
        .control(fid.clone())
        .and_then(|c| c.ship_orders.get(&name).cloned())
        .expect("spawn_ship seeds an order leaf");
    assert_eq!(
        leaf.mode,
        ControlMode::Inherit,
        "a freshly built ship has no opinion of its own"
    );
    assert_eq!(
        leaf.value,
        ShipBehavior::Idle,
        "…and its recorded value is a mere placeholder"
    );
    assert_eq!(
        state.ship_control(name.clone()),
        ControlMode::Player,
        "…so the fleet default owns it"
    );
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Dock {
            body: "地球".to_string()
        }),
        "…and it inherits the faction's intent instead of standing idle"
    );

    // 推进一回合：AI 不许碰它（归属解析在它身上给出 Player），而且它照着默认意图动。
    let mut rng = Prng::new(42);
    let before = state.ship(&name).expect("ship").position;
    advance(&mut state, &config, &mut rng);
    assert_eq!(
        state.ship_control(name.clone()),
        ControlMode::Player,
        "the system must not take it over"
    );
    let after = state.ship(&name).map(|s| s.position).unwrap_or(before);
    let to_earth =
        dist(after, state.body_position("地球")) < dist(before, state.body_position("地球"));
    assert!(
        to_earth,
        "the new ship must sail for 地球 per the fleet default, not be sent off by the AI"
    );
}

/// 玩家点名的殖民舰建完城之后必须**仍然是玩家的**。
///
/// 殖民是「命令 → 执行 → 指令失效」的一次性动作：收尾只该把**值**复位成 `Idle`，
/// 不许把**归属**一起清掉——以前无条件写 `Control::inherit(..)`，于是玩家刚下达的处置
/// 在城建好的那一刻被静默交还给系统（AI 下一回合就把它征去别处）。
#[test]
fn colonize_keeps_player_ownership() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    let fid = "中国".to_string();

    // 空出一座城（复垦路径：空白城仍占着它的定居点），再让中国的一艘舰去殖民。
    let victim = state
        .cities
        .iter()
        .find(|c| !c.razed && c.faction_id != fid)
        .map(|c| (c.name.clone(), c.body_id.clone(), c.faction_id.clone()))
        .expect("a foreign city to raze");
    let (cid, body, owner) = victim;
    raze_city(
        &mut state,
        &cid,
        RazeCause::Revolt {
            faction: owner,
            loyalty: 0.0,
        },
    );

    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .expect("a chinese ship");
    let order = |state: &mut State, v: ShipBehavior| {
        state
            .control_mut(fid.clone())
            .expect("control")
            .ship_orders
            .insert(ship.clone(), Control::player(v));
    };
    order(&mut state, ShipBehavior::Colonize { body: body.clone() });

    let mut next_id = 100_000;
    colonize(&mut state, &config, &mut rng, &ship, &body, &mut next_id);

    let leaf = state
        .control(fid.clone())
        .and_then(|c| c.ship_orders.get(&ship).cloned())
        .expect("the order leaf must still exist");
    assert_eq!(
        leaf.value,
        ShipBehavior::Idle,
        "one-shot order must be spent"
    );
    assert_eq!(
        leaf.mode,
        ControlMode::Player,
        "…but ownership must survive the order"
    );
    assert!(
        state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::ColonyFounded { .. })),
        "the city must actually have been refounded, got {:?}",
        state.events
    );

    // 早退路径（无处可殖民）同样不许动归属：找一个所有定居点都被活的城占满的天体。
    let full_body = state
        .cities
        .iter()
        .filter(|c| !c.razed)
        .map(|c| c.body_id.clone())
        .find(|b| {
            let Some(body) = state.body(b) else {
                return false;
            };
            let live: std::collections::BTreeSet<String> = state
                .cities
                .iter()
                .filter(|c| &c.body_id == b && !c.razed)
                .map(|c| c.settlement.clone())
                .collect();
            body.settlements.iter().all(|s| live.contains(&s.name))
        })
        .expect("a body whose settlements are all occupied");
    order(
        &mut state,
        ShipBehavior::Colonize {
            body: full_body.clone(),
        },
    );
    colonize(
        &mut state,
        &config,
        &mut rng,
        &ship,
        &full_body,
        &mut next_id,
    );
    let leaf = state
        .control(fid.clone())
        .and_then(|c| c.ship_orders.get(&ship).cloned())
        .expect("the order leaf must still exist");
    assert_eq!(
        leaf.mode,
        ControlMode::Player,
        "an early return must not hand the ship back either"
    );
}

/// A player-facing regression guard for the "stale follow" bug: a player
/// ship ordered to Follow an already-destroyed ship must degrade to Idle,
/// never drift toward the origin ([0,0]).
#[test]
fn player_stale_follow_degrades_to_idle_and_does_not_drift() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // China (3) corvette id=0 is Player-ordered to Follow US (1) destroyer id=3.
    let ship0 = state.ships[0].name.clone();
    let ship3 = state.ships[3].name.clone();
    let diff = serde_json::json!({
        "control": [{
            "faction_id": "中国",
            "ship_orders": [{"ship": ship0.clone(), "behavior": {"Follow": {"ship": ship3.clone()}}, "mode": "Player"}]
        }]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("apply order");

    // Simulate the target being destroyed before the round advances. 走 `kill_ship` 漏斗——
    // 它现在是**唯一**合法的「让一艘舰死」的方式（绕过它会被 `sweep_dead_ships` 的兜底
    // `debug_assert` 当场抓住，这正是这条测试以前直接 `t.hull = 0.0` 会炸的原因）。
    assert!(
        kill_ship(&mut state, &ship3, DeathCause::Combat, None),
        "target must get a recorded death event"
    );
    let pos_before = state.ship(&ship0).map(|s| s.position).unwrap();

    advance(&mut state, &config, &mut rng);

    // The order must have degraded to Idle ...
    let order = state.ship_behavior(ship0.clone());
    assert_eq!(
        order,
        Some(ShipBehavior::Idle),
        "stale order must degrade to Idle"
    );
    // ... without moving the ship toward the origin.
    let pos_after = state.ship(&ship0).map(|s| s.position).unwrap();
    assert_eq!(
        pos_after, pos_before,
        "ship must not drift (target is dead)"
    );
    // ... and a StaleOrder event must be recorded.
    assert!(
        state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::StaleOrder { ship: s, .. } if *s == ship0)),
        "expected a StaleOrder event for ship 0, got {:?}",
        state.events
    );
}

/// Follow semantics: `Follow { ship }` escorts/drives the ship — it is a pure
/// movement behavior. Combat is now automatic: when any hostile is inside the
/// ship's own attack range it auto-fires (via the unified base-weight targeting),
/// so a Follow ship still defends itself but never fires at the followed friend.
#[test]
fn follow_ship_auto_attacks_hostile_but_not_the_followed_friend() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // China (3): ship 0 Follows its own friendly ship 1. Co-located at [0,0].
    let ship0 = state.ships[0].name.clone();
    let ship1 = state.ships[1].name.clone();
    let ship3 = state.ships[3].name.clone();
    let ship4 = state.ships[4].name.clone();
    let ship5 = state.ships[5].name.clone();
    let diff = serde_json::json!({
        "control": [{
            "faction_id": "中国",
            "ship_orders": [{"ship": ship0.clone(), "behavior": {"Follow": {"ship": ship1.clone()}}, "mode": "Player"}]
        }]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("apply follow order");

    // Pin positions: follower + followed friend at [0,0]; US (1) enemy
    // destroyer id=3 just inside the corvette attack range (0.4) so the
    // auto-attack can fire. Move the other US ships (4 destroyer, 5 cruiser)
    // far out so only ship 3 engages (its damage 6 won't one-shot the follower's
    // hull 12, letting it retaliate).
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [0.0, 0.0];
    }
    if let Some(s) = state.ship_mut(&ship1) {
        s.position = [0.0, 0.0];
    }
    if let Some(enemy) = state.ship_mut(&ship3) {
        enemy.position = [0.3, 0.0];
    }
    if let Some(s) = state.ship_mut(&ship4) {
        s.position = [50.0, 50.0];
    }
    if let Some(s) = state.ship_mut(&ship5) {
        s.position = [50.0, 50.0];
    }

    // The default world now opens peacefully, so make US (1) explicitly
    // hostile to China (3) for this scenario.
    if let Some(f) = state.faction_mut("中国") {
        f.relations.insert("美国".to_string(), -35.0);
    }
    if let Some(f) = state.faction_mut("美国") {
        f.relations.insert("中国".to_string(), -35.0);
    }

    advance(&mut state, &config, &mut rng);

    // The ship must auto-fire on the hostile, not on the friend.
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::Attack { attacker, target, .. } if attacker == &ship0 && target == &ship3
        )),
        "ship should auto-attack the hostile, got {:?}",
        state.events
    );
    // The followed friend must be unharmed (no attack targeting ship 1).
    assert!(
        !state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::Attack { target, .. } if target == &ship1)),
        "ship must not fire at its own followed friend, got {:?}",
        state.events
    );
    // The order is still a valid Follow (not degraded to Idle).
    assert_eq!(
        state.ship_behavior(ship0.clone()),
        Some(ShipBehavior::Follow {
            ship: ship1.clone()
        })
    );
}

/// Events must populate as the world advances (growth / spurious events are
/// fine; the round log must simply be populated and contain no panics).
#[test]
fn advance_populates_round_events() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    assert!(state.events.is_empty(), "round 0 has no events yet");
    for _ in 0..6 {
        advance(&mut state, &config, &mut rng);
    }
    // After a few rounds of a war-torn seed, an event log should exist.
    assert!(
        !state.events.is_empty(),
        "after 6 rounds there should be events"
    );
}

/// 停泊轨道 (Dock) follows a body's current position; 待命 (Idle) holds
/// position. Dock persists (never degrades), and Idle never moves the ship.
#[test]
fn dock_follows_body_and_idle_holds_position() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // China (3) corvette id=0 docks body 4 (火星); id=1 is ordered Idle.
    let ship0 = state.ships[0].name.clone();
    let ship1 = state.ships[1].name.clone();
    let diff = serde_json::json!({
        "control": [{
            "faction_id": "中国",
            "ship_orders": [
                {"ship": ship0.clone(), "behavior": {"Dock": {"body": "火星"}}, "mode": "Player"},
                {"ship": ship1.clone(), "behavior": "Idle", "mode": "Player"}
            ]
        }]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("apply dock/idle order");

    // Pin ship 0 away from the body so `Dock` must move it toward the body.
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [5.0, 5.0];
    }
    if let Some(s) = state.ship_mut(&ship1) {
        s.position = [3.0, 3.0];
    }
    let dock_pos_before = state.ship(&ship0).map(|s| s.position).unwrap();
    let idle_pos_before = state.ship(&ship1).map(|s| s.position).unwrap();

    advance(&mut state, &config, &mut rng);

    // Dock: the ship moved toward the body (not froze, not degraded).
    let dock_pos_after = state.ship(&ship0).map(|s| s.position).unwrap();
    assert_ne!(
        dock_pos_after, dock_pos_before,
        "docked ship should move toward the body"
    );
    assert_eq!(
        state.ship_behavior(ship0.clone()),
        Some(ShipBehavior::Dock {
            body: "火星".to_string()
        }),
        "dock order must persist (not degrade to Idle)"
    );
    // Idle: the ship did not move.
    let idle_pos_after = state.ship(&ship1).map(|s| s.position).unwrap();
    assert_eq!(idle_pos_after, idle_pos_before, "Idle must hold position");
    assert_eq!(state.ship_behavior(ship1.clone()), Some(ShipBehavior::Idle));
}
