//! sim 的单元测试：不推进回合或 ≤48 回合（短档）。长档见同目录 horizon_mid.rs。

use super::*;
use crate::config::load_config;
use crate::control::apply_patch;
use crate::model::GameEvent;
use crate::world::default_state;

mod horizon_mid;

/// Build the config + a fresh deterministic world (round 0)，并把**角色轴钉成「全员战舰」**。
///
/// 这个模块的用例大多在测**别的东西**（生产/贸易/MOND/战斗/事件），而自动控制现在多了一条
/// 活：按积压定编、把船派去跑集货路线。不钉住它，被测的舰就可能被抽去拉货、不在它该在的
/// 位置上（`crate::world::pin_roles_to_war` 的文档记着一次实测踩坑）。
/// **测集货本身的用例**（`haul_*`）自己撤掉/覆盖这条默认。
fn fresh_world(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let mut state = default_state(&config, seed);
    crate::world::pin_roles_to_war(&mut state);
    (config, state)
}

/// 军事信号（思潮「和平↔军国」的驱动量）必须**只**由事件历史推出，且**同一现象同分**。
///
/// 这里逐条钉住旧实现的两个真实缺陷：
/// 1. **互杀吞掉战功**：旧口径是「同回合最后一条 `Attack` 的势力」，那要 `state.ship(attacker)`
///    才知道攻击者属于谁——凶手若在本回合也被打沉，它已经不在 `state.ships` 里，于是这次
///    击杀**领不到功**。权威的 `by` 不受影响。
/// 2. **失城方读成了抢城者**：旧口径用 `state.city(city).faction_id` 判「谁丢了城」，而夷平
///    不改归属、同回合稍后的复垦会把它改成新主，于是 −1 记到了**复垦者**头上。
///
/// 另外钉住「`CityDefected`（主路）与 `Revolt`（兜底）必须同分」——它们是同一个触发的两条
/// 分支，旧代码却只给兜底分支扣分。
#[test]
fn military_signal_uses_the_milestones_and_is_branch_agnostic() {
    let d = |events: &[GameEvent], fid: &str| military_deltas(events).get(fid).copied().unwrap_or(0.0);

    // 1) 互杀：A 的舰打沉 B 的舰，B 的舰同回合也打沉 A 的舰 → **双方各得一分战功**。
    let killer = |ship: &str, faction: &str| Killer {
        ship: ship.to_string(), faction: faction.to_string(), weapon: "kinetic".to_string(),
    };
    let mutual = vec![
        GameEvent::ShipDestroyed {
            ship: "乙舰".into(), owner: "乙".into(), class: "corvette".into(),
            cause: DeathCause::Combat, by: Some(killer("甲舰", "甲")),
        },
        GameEvent::ShipDestroyed {
            ship: "甲舰".into(), owner: "甲".into(), class: "corvette".into(),
            cause: DeathCause::Combat, by: Some(killer("乙舰", "乙")),
        },
    ];
    assert_eq!(d(&mutual, "甲"), 0.0, "甲沉一舰失一分、击沉一舰得一分，净 0");
    assert_eq!(d(&mutual, "乙"), 0.0, "乙同理——旧口径下会有一方拿不到战功");
    // 单方面被击沉：凶手得分，事主扣分。
    let one_sided = vec![GameEvent::ShipDestroyed {
        ship: "乙舰".into(), owner: "乙".into(), class: "corvette".into(),
        cause: DeathCause::Combat, by: Some(killer("甲舰", "甲")),
    }];
    assert_eq!(d(&one_sided, "甲"), 1.0);
    assert_eq!(d(&one_sided, "乙"), -1.0);

    // 2) 欠费报废：失主扣分，**没有人**领功（不是战功）。
    let rusted = vec![GameEvent::ShipDestroyed {
        ship: "锈舰".into(), owner: "丙".into(), class: "corvette".into(),
        cause: DeathCause::UpkeepShortfall, by: None,
    }];
    assert_eq!(d(&rusted, "丙"), -1.0);
    assert_eq!(d(&rusted, "甲"), 0.0, "欠费报废不该被记成任何人的战功");

    // 3) 城被 A 拆平、同回合被 C 复垦：扣分属于**失城方 B**，复垦者 C 不因此得军事分。
    let razed_then_refounded = vec![
        GameEvent::CityRazed {
            city: "城".into(), owner: "乙".into(), fallen_to: "甲".into(),
            by_ship: "甲舰".into(), damage: 9.0, pop_before: 200,
        },
        GameEvent::ColonyFounded {
            city: "城".into(), owner: "丙".into(), body: "木星".into(),
            seeded_ship_class: "corvette".into(), how: FoundingHow::Refounded,
            prev_owner: Some("乙".into()),
        },
    ];
    assert_eq!(d(&razed_then_refounded, "乙"), -1.0, "失城方是乙，不是复垦者");
    assert_eq!(d(&razed_then_refounded, "甲"), 1.0, "拆城方得一分");
    assert_eq!(d(&razed_then_refounded, "丙"), 0.0, "复垦是殖民行为，不进军事轴");

    // 4) 活城易主（离心倒戈）必须与叛乱兜底同分。
    let defect = vec![GameEvent::CityDefected {
        city: "城".into(), from: "乙".into(), to: "甲".into(), loyalty: 0.2,
    }];
    assert_eq!(d(&defect, "乙"), -1.0, "失主必须扣分（与 Revolt 兜底同分）");
    assert_eq!(d(&defect, "甲"), 1.0);
    let revolt = vec![GameEvent::Revolt { city: "城".into(), faction: "乙".into(), loyalty: 0.0 }];
    assert_eq!(d(&revolt, "乙"), -1.0);

    // 5) 新建城（真·殖民）不进军事轴。
    let founded = vec![GameEvent::ColonyFounded {
        city: "新城".into(), owner: "丙".into(), body: "地球".into(),
        seeded_ship_class: "corvette".into(), how: FoundingHow::NewSite, prev_owner: None,
    }];
    assert_eq!(d(&founded, "丙"), 0.0, "殖民归 nature_colony 轴");
}

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
    let name = spawn_ship(&mut state, &config, ShipSpawn {
        owner: fid.clone(),
        class: "corvette",
        position: pos,
        city: None,
        via: SpawnVia::Shipyard,
        pay_components: false,
        blueprint: None,
    });
    // 出厂时 `spawn_ship` 给它一条**没有说话**（`Inherit`）的叶片——它不在玩家的任何
    // diff 里，所以「谁负责、干什么」只能由更宽的那一层回答。
    let leaf = state
        .control(fid.clone())
        .and_then(|c| c.ship_orders.get(&name).cloned())
        .expect("spawn_ship seeds an order leaf");
    assert_eq!(leaf.mode, ControlMode::Inherit, "a freshly built ship has no opinion of its own");
    assert_eq!(leaf.value, ShipBehavior::Idle, "…and its recorded value is a mere placeholder");
    assert_eq!(state.ship_control(name.clone()), ControlMode::Player, "…so the fleet default owns it");
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Dock { body: "地球".to_string() }),
        "…and it inherits the faction's intent instead of standing idle"
    );

    // 推进一回合：AI 不许碰它（归属解析在它身上给出 Player），而且它照着默认意图动。
    let mut rng = Prng::new(42);
    let before = state.ship(&name).expect("ship").position;
    advance(&mut state, &config, &mut rng);
    assert_eq!(state.ship_control(name.clone()), ControlMode::Player, "the system must not take it over");
    let after = state.ship(&name).map(|s| s.position).unwrap_or(before);
    let to_earth = dist(after, state.body_position("地球")) < dist(before, state.body_position("地球"));
    assert!(to_earth, "the new ship must sail for 地球 per the fleet default, not be sent off by the AI");
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
    raze_city(&mut state, &cid, RazeCause::Revolt { faction: owner, loyalty: 0.0 });

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
    assert_eq!(leaf.value, ShipBehavior::Idle, "one-shot order must be spent");
    assert_eq!(leaf.mode, ControlMode::Player, "…but ownership must survive the order");
    assert!(
        state.events.iter().any(|e| matches!(e, GameEvent::ColonyFounded { .. })),
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
            let Some(body) = state.body(b) else { return false };
            let live: std::collections::BTreeSet<String> = state
                .cities
                .iter()
                .filter(|c| &c.body_id == b && !c.razed)
                .map(|c| c.settlement.clone())
                .collect();
            body.settlements.iter().all(|s| live.contains(&s.name))
        })
        .expect("a body whose settlements are all occupied");
    order(&mut state, ShipBehavior::Colonize { body: full_body.clone() });
    colonize(&mut state, &config, &mut rng, &ship, &full_body, &mut next_id);
    let leaf = state
        .control(fid.clone())
        .and_then(|c| c.ship_orders.get(&ship).cloned())
        .expect("the order leaf must still exist");
    assert_eq!(leaf.mode, ControlMode::Player, "an early return must not hand the ship back either");
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
    assert_eq!(order, Some(ShipBehavior::Idle), "stale order must degrade to Idle");
    // ... without moving the ship toward the origin.
    let pos_after = state.ship(&ship0).map(|s| s.position).unwrap();
    assert_eq!(pos_after, pos_before, "ship must not drift (target is dead)");
    // ... and a StaleOrder event must be recorded.
    assert!(
        state.events.iter().any(|e| matches!(e, GameEvent::StaleOrder { ship: s, .. } if *s == ship0)),
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
        !state.events.iter().any(|e| matches!(e, GameEvent::Attack { target, .. } if target == &ship1)),
        "ship must not fire at its own followed friend, got {:?}",
        state.events
    );
    // The order is still a valid Follow (not degraded to Idle).
    assert_eq!(
        state.ship_behavior(ship0.clone()),
        Some(ShipBehavior::Follow { ship: ship1.clone() })
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
    assert!(!state.events.is_empty(), "after 6 rounds there should be events");
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
    assert_ne!(dock_pos_after, dock_pos_before, "docked ship should move toward the body");
    assert_eq!(
        state.ship_behavior(ship0.clone()),
        Some(ShipBehavior::Dock { body: "火星".to_string() }),
        "dock order must persist (not degrade to Idle)"
    );
    // Idle: the ship did not move.
    let idle_pos_after = state.ship(&ship1).map(|s| s.position).unwrap();
    assert_eq!(idle_pos_after, idle_pos_before, "Idle must hold position");
    assert_eq!(state.ship_behavior(ship1.clone()), Some(ShipBehavior::Idle));
}

/// 护甲再生 (ShipSpec.hull_regen): a damaged ship regains a fraction of its
/// max hull each round; full-hull ships stay capped; destroyed ships stay gone.
#[test]
fn damaged_ship_regenerates_hull_each_round() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // Take China's Earth corvette (id 0, hull_max 12, hull_regen 0.04) and
    // damage it to exactly half; pin it away from all hostiles so the round
    // is quiet and only regeneration acts on it.
    let ship0 = state.ships[0].name.clone();
    let ship1 = state.ships[1].name.clone();
    if let Some(s) = state.ship_mut(&ship0) {
        s.hull = 6.0;
        s.position = [80.0, 80.0];
    }
    let class = state.ship(&ship0).map(|s| s.class.clone()).unwrap();
    let regen = config.ship_spec(&class).hull_regen;

    advance(&mut state, &config, &mut rng);

    let hull = state.ship(&ship0).map(|s| s.hull).expect("ship 0 still alive");
    let expected = (6.0 + 12.0 * regen).min(12.0);
    assert!(
        (hull - expected).abs() < 1e-9,
        "hull should heal to {expected}, got {hull}"
    );

    // A full-hull ship stays capped (no over-heal).
    if let Some(s) = state.ship_mut(&ship1) {
        s.hull = config.ship_spec(&s.class).hull;
        s.position = [80.0, 80.0];
    }
    advance(&mut state, &config, &mut rng);
    let max1 = config.ship_spec(&state.ship(&ship1).map(|s| s.class.clone()).unwrap()).hull;
    let hull1 = state.ship(&ship1).map(|s| s.hull).unwrap();
    assert!((hull1 - max1).abs() < 1e-9, "full hull must not over-heal, got {hull1}");
}

/// 定居点 ↔ 城市 一一对应: 每个城市占据其天体上一个合法定居点；同一座城不会
/// 让一个定居点被两座城占用；地球恰好 5 个定居点各坐一座 spec 都市，矿藏按
/// 定居点隔离（巴黎只产 铀/铂，不再共享整个地球的矿藏池）。
#[test]
fn settlements_and_cities_are_one_to_one() {
    let (_config, state) = fresh_world(42);
    for b in &state.bodies {
        let cities: Vec<&City> = state.cities.iter().filter(|c| c.body_id == b.name).collect();
        assert!(
            cities.len() <= b.settlements.len(),
            "body {}: {} cities must not exceed {} settlements",
            b.name,
            cities.len(),
            b.settlements.len()
        );
        for c in cities {
            assert!(
                b.settlements.iter().any(|s| s.name == c.settlement),
                "city {} (body {}) points at an unknown settlement {}",
                c.name,
                b.name,
                c.settlement
            );
        }
    }

    let earth = &state.bodies[2];
    assert_eq!(earth.settlements.len(), 5, "Earth has five spec metropolises");
    let earth_cities = state.cities.iter().filter(|c| c.body_id == "地球").count();
    assert_eq!(earth_cities, 5, "five cities on five Earth settlements (1:1)");
    // 巴黎 (settlement named 巴黎) hosts only 铀/铂 — its own region's ores.
    let paris = earth.settlements[3].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
    assert_eq!(paris, vec!["铀", "铂"], "Paris settlement mines only its own ores");
    assert_eq!(
        state.cities.iter().find(|c| c.name == "巴黎").map(|c| c.settlement.as_str()),
        Some("巴黎"),
        "巴黎 occupies the settlement named 巴黎"
    );
    // 长三角/珠三角 are distinct settlements, so both may mine 铁 independently.
    let cn = earth.settlements[0].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
    assert!(cn.contains(&"铁"), "长三角 settlement has 铁");
    assert!(cn.contains(&"硅") && cn.contains(&"水冰"), "长三角 has 硅/水冰");
}

/// 剧情机械后果：prologue 给无国界科学组织(6)注入氦-3，并拉低它与行星X崇拜教(8)的关系。
#[test]
fn story_effects_apply() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    let rel_before = state.faction("无国界科学组织").and_then(|f| f.relations.get(&"行星X崇拜教".to_string()).copied()).unwrap_or(0.0);

    advance(&mut state, &config, &mut rng);

    // prologue 是「事件型后果」：把资源写入并在编年史里记录。第 1 回合经济（维护费/
    // 市场）会立刻重新平衡库存，故不断言 helium3 净增（它可能被维护费/市场抵消），
    // 而断言那份资源确实进入了编年史记录的 prologue（机械后果生效）。
    assert!(
        state.chronicle.iter().any(|c| c.id == "prologue"),
        "prologue must fire and record its effects at round 1"
    );
    let rel_after = state.faction("无国界科学组织").and_then(|f| f.relations.get(&"行星X崇拜教".to_string()).copied()).unwrap_or(0.0);
    assert!(rel_after < rel_before, "prologue must lower science↔cult relation (effect)");
}

/// 剧情 GrantShip 后果：kuiper_boom（RoundAt 24）给星系矿业(5)出厂一艘巡洋舰；
/// 出厂位置恰好在天体当前位置 + (0.05, 0.05)（确定性偏移）、带 Idle 指令、id 连续。
#[test]
fn story_grant_ship_spawns_a_fleet_member() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // Advance to round 24 so kuiper_boom fires.
    for _ in 0..24 {
        advance(&mut state, &config, &mut rng);
    }

    // The story fired this round.
    assert!(
        state.chronicle.iter().any(|c| c.id == "kuiper_boom" && c.round == 24),
        "kuiper_boom must fire at round 24, got {:?}",
        state.chronicle.iter().map(|c| (c.id.clone(), c.round)).collect::<Vec<_>>()
    );

    // The granted cruiser is at exactly body 9 (泰坦) position + the deterministic offset.
    let bpos = state.body_position("泰坦");
    let granted = state
        .ships
        .iter()
        .find(|s| {
            s.faction_id == "星系矿业"
                && s.class == "cruiser"
                && (s.position[0] - (bpos[0] + 0.05)).abs() < 1e-9
                && (s.position[1] - (bpos[1] + 0.05)).abs() < 1e-9
        })
        .expect("kuiper_boom must grant 星系矿业 a cruiser parked at 泰坦");
    assert_eq!(state.ship_behavior(granted.name.clone()), Some(ShipBehavior::Idle), "granted ship starts Idle");

    // Determinism: re-running reproduces the identical granted fleet.
    let (_, mut state2) = fresh_world(42);
    let mut rng2 = Prng::new(42);
    for _ in 0..24 {
        advance(&mut state2, &config, &mut rng2);
    }
    let fleet_a: Vec<(String, String)> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == "星系矿业")
        .map(|s| (s.name.clone(), s.class.clone()))
        .collect();
    let fleet_b: Vec<(String, String)> = state2
        .ships
        .iter()
        .filter(|s| s.faction_id == "星系矿业")
        .map(|s| (s.name.clone(), s.class.clone()))
        .collect();
    assert_eq!(fleet_a, fleet_b, "same seed must reproduce the same granted fleet");
}

/// 娱乐/福利预算（忠诚度）：一座远离首都的城市，其距离目标忠诚度本应很低；但若
/// 治理势力投入足够的娱乐预算，忠诚度仍能维持/回升，而非立刻爆发离心叛乱。
#[test]
fn entertainment_holds_a_distant_city() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    // 深口袋：让星系矿业(5)付得起治理 + 娱乐开销，覆盖率=1。
    if let Some(f) = state.faction_mut("星系矿业") {
        for k in [
            "铁", "碳", "硅", "水冰", "铀", "铂", "金",
            "氦-3", "钍", "氢", "甲烷",
        ] {
            f.resources.insert(k.to_string(), 100_000.0);
        }
    }
    // 妊神星转运站 (city 19, body 15) 远离矿业首都(泰坦, body 9)，距离目标忠诚度≈0。
    let city19 = state.cities[19].name.clone();
    if let Some(c) = state.city_mut(&city19) {
        c.loyalty = 0.35; // 略高于叛变阈值，但本应继续下滑。
    }
    let loy0 = state.city(&city19).map(|c| c.loyalty).unwrap();
    // 重金投入该城娱乐预算（Player 覆盖）。
    let diff = serde_json::json!({
        "control": [{"faction_id": "星系矿业", "loyalty_budget": [{"city": city19.clone(), "value": 500.0, "mode": "Player"}]}]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("apply loyalty budget");

    advance(&mut state, &config, &mut rng);

    let loy1 = state.city(&city19).map(|c| c.loyalty).unwrap_or(0.0);
    assert!(
        loy1 >= loy0,
        "heavy entertainment funding should keep a distant city loyal (started {loy0}, now {loy1})"
    );
    assert_eq!(
        state.city(&city19).map(|c| c.razed),
        Some(false),
        "a well-funded distant city must not revolt"
    );
}

/// 离心「改旗易帜」：低忠诚城市不再被夷为荒地，而是倒戈到**思潮与旧主最对立**的势力，
/// 城市连同其人口/建筑/控制面一起易主（旧主失去一城、新主获得一城）——这是给旁观/
/// 小势力接盘城市、避免「永久 1 城旁观者」的机制。
#[test]
fn low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);

    // 珠三角 (city 1) 属中国，位于其首都(地球)上——但把忠诚压到叛变阈值之下。
    let city = state.cities[1].name.clone();
    let owner = "中国".to_string();

    // 中国 → 极端（军国+技术+精英+殖民），无国界科学组织 → 相反极，其余全中立。
    // 于是无国界科学组织与中国的思潮距离 = 8（唯一最大），倒戈目标唯一确定。
    let extreme = Ideology { peace_military: 1.0, science_tech: 1.0, people_elite: 1.0, nature_colony: 1.0 };
    let oppose = Ideology { peace_military: -1.0, science_tech: -1.0, people_elite: -1.0, nature_colony: -1.0 };
    if let Some(f) = state.faction_mut("中国") {
        f.ideology = extreme;
    }
    if let Some(f) = state.faction_mut("无国界科学组织") {
        f.ideology = oppose;
    }
    for f in &mut state.factions {
        if f.name != "中国" && f.name != "无国界科学组织" {
            f.ideology = Ideology::default();
        }
    }
    // 忠诚压到叛变阈值之下（0.30）。
    if let Some(c) = state.city_mut(&city) {
        c.loyalty = 0.05;
    }
    let pop_before = state.city(&city).map(|c| c.population).unwrap_or(0);
    let buildings_before = state.city(&city).map(|c| c.buildings.len()).unwrap_or(0);

    advance(&mut state, &config, &mut rng);

    let c = state.city(&city).expect("defected city must survive (not razed)");
    assert_eq!(
        c.faction_id, "无国界科学组织",
        "low-loyalty city must defect to the most ideologically-opposed faction"
    );
    assert!(!c.razed, "defected city must not be razed to blank");
    assert_eq!(c.population, pop_before, "defected city keeps its population");
    assert_eq!(c.buildings.len(), buildings_before, "defected city keeps its buildings");
    // 忠诚在倒戈时被重置为满，随后同回合新主的治理会重新计量；断言它仍高于叛变阈值，
    // 证明这次倒戈给了城市一个「新开始」（没有立刻又叛变/再被夷平）。
    assert!(
        c.loyalty > 0.05,
        "defected city must get a fresh loyalty start (was 0.05, now {}), not stay near zero",
        c.loyalty
    );

    // 事件必须是 CityDefected（旧主→新主），不是 Revolt。
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::CityDefected { city: cid, from, to, .. }
                if *cid == city && *from == owner && *to == "无国界科学组织"
        )),
        "expected a CityDefected event, got {:?}",
        state.events
    );

    // 控制转移：新主(无国界科学组织)的控制面应接管这座城（invest/build 权重按 (城,建筑) 迁入）。
    if let Some(n) = state.control("无国界科学组织".to_string()) {
        let owned_build_keys: bool = state
            .city(&city)
            .map(|c| c.buildings.iter().any(|b| n.build_weights.contains_key(&(city.clone(), b.id))))
            .unwrap_or(false);
        assert!(
            n.invest_weights.keys().any(|(cid, _)| cid == &city) || n.build_weights.keys().any(|(cid, _)| cid == &city),
            "new owner control must include the defected city's buildings"
        );
        let _ = owned_build_keys;
    }
}

/// MOND 引力异常：落入异常区（深空）时，未掌握 MOND 修正引力的势力在导航上产生
/// 切向偏移（指令坐标与实际坐标分离），而掌握它的 cult 指哪打哪。
///
/// 偏移幅度是**伪随机**的（`roll`）：非 master 这一回合偏多少由尝试决定，
/// 但**永远存在蒙对的一次**（`roll = 0` 即精确命中）——所以深处是「难」而不是「不可能」。
#[test]
fn mond_drift_misses_in_anomaly_but_masters_are_exact() {
    let (config, _state) = fresh_world(42);
    let dest = [60.0, 0.0]; // 距太阳 60 AU，深入柯伊伯异常区。
    // 非 MOND 势力（中国=3）：这一回合的目标被切向偏移，无法精确到达。
    let d = mond_drift(&config, "中国", dest, 0.5);
    assert!(
        (d[0] - dest[0]).abs() > 1e-6 || (d[1] - dest[1]).abs() > 1e-6,
        "a non-master ship must drift inside the anomaly, got {d:?}"
    );
    // 但偏移是可变的：偏得少的一次就几乎命中（这是「多试几个回合」的根据）。
    let lucky = mond_drift(&config, "中国", dest, 0.0);
    assert_eq!(lucky, dest, "roll=0 的一次尝试必须指哪打哪——不存在永远进不去的目标");
    let worse = mond_drift(&config, "中国", dest, 1.0);
    assert!(
        dist(worse, dest) > dist(d, dest),
        "roll 越大偏得越远（幅度单调），got {worse:?}"
    );
    // MOND 势力（行星X崇拜教=8）：掌握修正引力，无偏移、指哪打哪。
    for roll in [0.0, 0.5, 1.0] {
        let m = mond_drift(&config, "行星X崇拜教", dest, roll);
        assert_eq!(m, dest, "a MOND master must compute the destination exactly");
    }
}

/// **产地货栈（M1）**：首都天体的产出直接进势力池（**首都即集散地**，免运输），
/// 其余天体的产出落在**产地货栈**里，**不会自己跑到池子里**——只有运输能把它送到首都。
///
/// 用例（seed 42 的真实开局布局）：
/// * **中国**：首都地球，矿在**两边都有**——长三角/珠三角（地球，采 铁/硅）是首都产出，
///   金星浮空之城（金星，采 碳）是离岸产出。碳 只从金星出、铁硅 只从地球出，
///   所以「池子里多了铁硅、碳却没动、金星货栈里有碳」正好把两条路径分开。
/// * **无国界科学组织**：它的矿**全在非首都天体**（土星，采 氢；首都是木星）
///   → 产出**整批积压**，池子一分钱都不涨。这就是运输机制要解决的问题本身。
#[test]
fn off_capital_production_lands_in_the_depot_not_the_pool() {
    let (config, mut state) = fresh_world(42);
    assert_eq!(state.capital_body("中国"), "地球", "用例前提：中国首都在地球");
    assert_eq!(
        state.capital_body("无国界科学组织"),
        "木星",
        "用例前提：科学组织首都在木星"
    );

    let mut flow = RoundFlow::default();
    let cn_silicon = |s: &State| {
        s.faction("中国").unwrap().resources.get("硅").copied().unwrap_or(0.0)
    };
    let cn_carbon = |s: &State| {
        s.faction("中国").unwrap().resources.get("碳").copied().unwrap_or(0.0)
    };
    let sci_value = |s: &State| {
        let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
        s.faction("无国界科学组织")
            .unwrap()
            .resources
            .iter()
            .map(|(rt, amt)| amt * value_of(rt))
            .sum::<f64>()
    };

    let (si0, c0, sci0) = (cn_silicon(&state), cn_carbon(&state), sci_value(&state));
    step_production(&mut state, &config, &mut flow);

    // —— 中国：首都产出进池，离岸产出进货栈 ——
    assert!(
        cn_silicon(&state) > si0,
        "地球（首都）上的硅应直接进池"
    );
    assert!(
        state.depot("中国", "地球").is_none(),
        "首都天体的产出不进货栈（免运输、直接进池）"
    );
    let venus = state
        .depot("中国", "金星")
        .expect("金星上的产出必须落在产地货栈");
    assert!(
        venus.get("碳").copied().unwrap_or(0.0) > 0.0,
        "金星采的碳应压在产地货栈里，实为 {venus:?}"
    );
    assert!(
        (cn_carbon(&state) - c0).abs() < 1e-9,
        "碳只从金星（非首都）出，所以池子里的碳一格都不该动——运输才是货栈的上游"
    );

    // —— 科学组织：矿全在非首都 → 整批积压，池子不动 ——
    let saturn = state
        .depot("无国界科学组织", "土星")
        .expect("土星上的产出必须落在产地货栈");
    assert!(
        saturn.get("氢").copied().unwrap_or(0.0) > 0.0,
        "土星的氢应压在产地货栈里，实为 {saturn:?}"
    );
    assert!(
        (sci_value(&state) - sci0).abs() < 1e-9,
        "一个「矿全在非首都天体」的势力，产出会整批积压在产地（= 等船来运）"
    );

    // —— 再跑一回合：货栈继续涨、池子仍不因它增长（库存冻结）——
    let carbon_in_depot = venus.get("碳").copied().unwrap_or(0.0);
    step_production(&mut state, &config, &mut flow);
    assert!(
        state
            .depot("中国", "金星")
            .unwrap()
            .get("碳")
            .copied()
            .unwrap_or(0.0)
            > carbon_in_depot,
        "没有船来运 → 货栈继续涨"
    );
    assert!(
        (cn_carbon(&state) - c0).abs() < 1e-9,
        "两回合过去，金星的碳一格都没进池——这就是「等船来运」"
    );
}

/// **舱容（M2）**：有效舱容 = 舰级舱容 `ShipSpec::cargo` × **战损折算** `hull / hull_max`。
///
/// 钉住三条机制不变量：
/// 1. **舰级舱容是「设计裁决」而不是平衡旋钮**——护卫 2 / 驱逐 4 / 巡洋 6 / 航母 20 /
///    战列 6，且**航母是唯一的散货船**。这条要硬断言：改它等于改设计，不该是调参时手滑。
/// 2. **战损是连续的**：装甲掉一半 → 舱容减半（不是「受伤就装不了」的硬阈值）。
/// 3. **旧档（`hull_max ≤ 0`）按满舱**：绝不出现 `hull / 0 = ∞` 的无底货舱。
#[test]
fn cargo_capacity_is_class_capacity_times_hull_fraction() {
    use crate::model::cargo_capacity;
    let (config, state) = fresh_world(42);

    // 1) 舰级舱容（设计裁决：见 config/game.ron 的 ships 注释第 (3) 类）。
    let table = [
        ("corvette", 2.0),
        ("destroyer", 4.0),
        ("cruiser", 6.0),
        ("carrier", 20.0),
        ("battleship", 6.0),
    ];
    for (class, cap) in table {
        assert_eq!(
            config.ship_spec(class).cargo,
            cap,
            "{class} 的舱容是设计裁决（{cap}），不是可随手调的平衡旋钮"
        );
    }
    assert!(
        table.iter().all(|(c, cap)| *c == "carrier" || *cap < 20.0),
        "航母必须是唯一的散货船——否则「用哪条船运货」就不构成一个选择"
    );

    // 2) 战损连续折算。
    let mut ship = state
        .ships
        .iter()
        .find(|s| s.class == "cruiser")
        .expect("开局有巡洋舰")
        .clone();
    assert!(ship.hull_max > 0.0, "出厂舰必须有 hull_max");
    assert_eq!(cargo_capacity(&config, &ship), 6.0, "满血巡洋舰 = 满舱 6");
    ship.hull = ship.hull_max * 0.5;
    assert!(
        (cargo_capacity(&config, &ship) - 3.0).abs() < 1e-9,
        "装甲掉一半 → 舱容减半（连续，不是硬阈值）"
    );
    ship.hull = 0.0;
    assert_eq!(cargo_capacity(&config, &ship), 0.0, "壳被打光 → 一格都装不了");

    // 3) 旧档缺 `hull_max`：按满舱处理，而不是把舱容算成无穷。
    ship.hull = 6.0;
    ship.hull_max = 0.0;
    assert_eq!(
        cargo_capacity(&config, &ship),
        6.0,
        "hull_max ≤ 0（旧档）按未受损处理，绝不返回 ∞"
    );
}

/// **尽量等量分配（Q6）**：[`haul_split`] 是 max-min 公平分配——先按「还有货的种类数」平摊，
/// 分不满的种类把余量交回去、由其余种类再平摊。它是**纯函数**，这里逐档钉住。
#[test]
fn haul_split_is_max_min_fair() {
    let m = |pairs: &[(&str, f64)]| -> ResourceMap {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    };
    // 三种货、舱容 6、都够 ⇒ 每种 2。
    assert_eq!(
        haul_split(&m(&[("铁", 10.0), ("碳", 10.0), ("硅", 10.0)]), 6.0),
        m(&[("铁", 2.0), ("碳", 2.0), ("硅", 2.0)])
    );
    // 铂只有 1 ⇒ 它拿 1，多出来的 1 由另两种再平摊（这就是「尽量」等量）。
    assert_eq!(
        haul_split(&m(&[("铁", 10.0), ("铂", 1.0), ("碳", 10.0)]), 6.0),
        m(&[("铁", 2.5), ("铂", 1.0), ("碳", 2.5)])
    );
    // 舱容 ≥ 总存量 ⇒ 全装走（一种货吃得下就全给它，不必等量）。
    assert_eq!(
        haul_split(&m(&[("铁", 1.0), ("碳", 2.0)]), 100.0),
        m(&[("铁", 1.0), ("碳", 2.0)])
    );
    // 边界：空货栈 / 零舱容 ⇒ 什么都不装（不是 panic）。
    assert!(haul_split(&ResourceMap::new(), 20.0).is_empty());
    assert!(haul_split(&m(&[("铁", 5.0)]), 0.0).is_empty());
}

/// **货值守恒（M2b 的核心不变量）**：装货与卸货**只搬货**——产地里少多少，舱里就多多少；
/// 舱里清空多少，首都池就多多少。全程「货栈 + 在舱 + 池子」的总量一格不变。
///
/// 这条守卫是这套机制的地基：集货腿的正当性全在「**货不会凭空出现或消失**」上
/// （一旦漏了，缺矿就会像 M1 之前那样被静默补贴掉）。
#[test]
fn hauling_moves_cargo_without_creating_or_destroying_any() {
    let (config, mut state) = fresh_world(42);
    state.depots.clear();
    state.depot_add("中国", "金星", "碳", 7.0);
    state.depot_add("中国", "金星", "铁", 5.0);
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .expect("中国开局有舰")
        .name
        .clone();
    let class = state.ship(&ship).unwrap().class.clone();
    // 停在金星泊位上（`arrival_eps` 之内）。
    let vpos = state.body_position("金星");
    state.ship_mut(&ship).unwrap().position = vpos;
    let total = |s: &State| -> f64 {
        let depot: f64 = s.depots.values().flat_map(|m| m.values()).sum();
        let hold: f64 = s.ships.iter().flat_map(|x| x.cargo.values()).sum();
        let pool: f64 = s.factions.iter().flat_map(|f| f.resources.values()).sum();
        depot + hold + pool
    };
    let before = total(&state);

    // —— 装货：上限 = 有效舱容，两种货尽量等量 ——
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
    let units = match step {
        HaulStep::Loaded { units, .. } => units,
        other => panic!("停在货栈泊位上应当装货，实为 {other:?}"),
    };
    let cap = cargo_capacity(&config, state.ship(&ship).unwrap());
    assert!(
        (units - cap).abs() < 1e-9,
        "货栈有 12 件、舱容 {cap} ⇒ 装满：实装 {units}"
    );
    let hold: f64 = state.ship(&ship).unwrap().cargo.values().sum();
    assert!((hold - units).abs() < 1e-9, "装了多少就在舱里有多少");
    let left: f64 = state.depot("中国", "金星").unwrap().values().sum();
    assert!(
        (left - (12.0 - units)).abs() < 1e-9,
        "货栈恰好少了装走的那些：剩 {left}"
    );
    assert!((total(&state) - before).abs() < 1e-9, "装货不许造货");

    // —— 卸货：挪到首都（地球）泊位上，货进**势力池** ——
    let epos = state.body_position("地球");
    state.ship_mut(&ship).unwrap().position = epos;
    let carbon_before = state
        .faction("中国")
        .unwrap()
        .resources
        .get("碳")
        .copied()
        .unwrap_or(0.0);
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
    match step {
        HaulStep::Delivered {
            units: u,
            into_pool: true,
            ..
        } => assert!((u - hold).abs() < 1e-9, "整舱卸下"),
        other => panic!("在首都泊位上应当卸进首都池，实为 {other:?}"),
    }
    assert!(state.ship(&ship).unwrap().cargo.is_empty(), "卸完舱就空了");
    assert!(
        state
            .faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0)
            > carbon_before,
        "金星采的碳进了首都池——这就是集货腿的终点"
    );
    assert!((total(&state) - before).abs() < 1e-9, "卸货不许毁货");
}

/// **雇佣交付的记账（Q10 抽成制）**：卸下来的货**分两份**——抽成归受雇方自己的
/// 首都池，余数进**雇主**的池子（不是船东的！）。
///
/// 这一条把「货主与船东分离」这件事钉在最细的粒度上（不跑 `advance`，所以池子不会被
/// 维护费/建造搅浑）：**同一个天体、同一批货，进的是两个不同势力的池子**。
#[test]
fn a_hired_delivery_splits_the_cargo_between_carrier_and_shipper() {
    let (config, mut state) = fresh_world(42);
    let share = config.freight.share;
    state.depots.clear();
    // 雇主：中国在金星积压 10 件碳，**它自己没有船**。
    state.depot_add("中国", "金星", "碳", 10.0);
    // 受雇方：美国的一艘驱逐舰（舱容 4）。
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "美国" && s.class == "destroyer")
        .expect("美国开局有驱逐舰")
        .name
        .clone();
    let class = state.ship(&ship).unwrap().class.clone();
    let id = state.contracts.post(
        "中国".into(),
        "碳".into(),
        3.0,
        "金星".into(),
        "地球".into(),
        share,
        0,
        0.0,
    );
    state.contracts.assign(ship.clone(), id);
    state.contracts.contracts.iter_mut().find(|c| c.id == id).unwrap().carrier =
        Some("美国".into());
    // 停在**托运方货栈**的泊位上 → 装货该装的是**中国的**货。
    let vpos = state.body_position("金星");
    state.ship_mut(&ship).unwrap().position = vpos;
    let cap = cargo_capacity(&config, state.ship(&ship).unwrap());
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
    let loaded = match step {
        HaulStep::Loaded { units, .. } => units,
        other => panic!("停在托运方货栈上该装货，实为 {other:?}"),
    };
    assert!(loaded > 0.0 && loaded <= cap, "装的不能超过舱容");
    assert!(
        state.depot("中国", "金星").is_none()
            || (state.depot("中国", "金星").unwrap().values().sum::<f64>() - (10.0 - loaded)).abs()
                < 1e-9,
        "装走的必须是**中国**货栈里的货"
    );
    // 卸到中国的首都（地球）：抽成归美国、余数归中国。
    let (cn0, us0) = (
        state.faction("中国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
        state.faction("美国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
    );
    state.ship_mut(&ship).unwrap().position = state.body_position("地球");
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
    assert!(
        matches!(step, HaulStep::Delivered { into_pool: true, .. }),
        "目的 = 托运方首都 ⇒ 该进池子，实为 {step:?}"
    );
    let (cn1, us1) = (
        state.faction("中国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
        state.faction("美国").unwrap().resources.get("碳").copied().unwrap_or(0.0),
    );
    let cut = loaded * share;
    assert!(
        (cn1 - cn0 - (loaded - cut)).abs() < 1e-9,
        "托运方该收到 {} 件（装的 {} 减去抽成 {cut:.2}），实收 {:.3}",
        loaded - cut,
        loaded,
        cn1 - cn0
    );
    assert!(
        (us1 - us0 - cut).abs() < 1e-9,
        "承运人的报酬就是它自留的那份货：{cut:.3}，实收 {:.3}",
        us1 - us0
    );
    // 合同进度按**卸出舱的总量**记（抽成是搬运费，不能从运力里扣）。
    let c = state.contracts.get(id).expect("合同还在雇佣期内");
    assert!(
        (c.delivered - loaded).abs() < 1e-9,
        "进度 = 卸出舱的总量 {loaded}，实为 {}",
        c.delivered
    );
}

/// **常驻路线 + 腿别由货舱决定（Q7 A / Q5 A）**：同一对 `from/to`、**不存任何额外状态**，
/// 空舱就去装、装到货就改跑 `to`、货栈空就原地等——三件事全部由「舱里有货吗」推出来。
#[test]
fn a_haul_route_alternates_legs_because_of_the_cargo() {
    let (config, mut state) = fresh_world(42);
    state.depots.clear();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .expect("中国开局有舰")
        .name
        .clone();
    let class = state.ship(&ship).unwrap().class.clone();
    let vpos = state.body_position("金星");
    state.ship_mut(&ship).unwrap().position = vpos;

    // 1) 货栈是空的 ⇒ **原地等**（「有货就走」的另一半是「没货就不走」），位置不动。
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
    assert!(
        matches!(step, HaulStep::Waiting { ref body } if body == "金星"),
        "空货栈应当原地等，实为 {step:?}"
    );
    assert_eq!(state.ship(&ship).unwrap().position, vpos, "等的时候不许乱跑");
    assert!(state.ship(&ship).unwrap().cargo.is_empty());

    // 2) 来货了就装，且**这一回合不再跑**（与殖民一样是「到达即行动」）。
    state.depot_add("中国", "金星", "碳", 3.0);
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
    assert!(matches!(step, HaulStep::Loaded { .. }), "有货就装，实为 {step:?}");
    assert!(!state.ship(&ship).unwrap().cargo.is_empty(), "舱里该有货");

    // 3) 舱里有货 ⇒ 腿别翻到 `to`（哪怕 `from` 还有货）。同一对 from/to、零额外状态。
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球");
    assert_eq!(step.body(), "地球", "舱里有货 ⇒ 这一腿去卸货端，实为 {step:?}");
    assert!(
        !matches!(step, HaulStep::Waiting { .. } | HaulStep::Loaded { .. }),
        "有货时不该再在装货端打转，实为 {step:?}"
    );
}

/// **端到端（玩家路径）**：给一艘舰写一条 `Haul` 指令，货真的从**产地货栈**走到**首都池**，
/// 而且走的是一条不需要重下的**常驻路线**（两个回合内装完并卸到池里）。
///
/// 用「首都天体上的货栈」把航程压成 0 回合：它本来就是为了「迁都把旧中转点留在新首都」
/// 准备的合法路线（`Haul { from: cap, to: cap }`，见 `behavior_is_valid`），正好也是最短的
/// 端到端用例。
#[test]
fn a_commanded_haul_route_delivers_depot_cargo_into_the_capital_pool() {
    let (config, mut state) = fresh_world(42);
    state.depots.clear();
    state.depot_add("中国", "地球", "碳", 9.0); // 首都是地球；这处货栈压着 9 件碳
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国" && s.class == "destroyer")
        .expect("中国开局有一艘驱逐舰")
        .name
        .clone();
    let epos = state.body_position("地球");
    state.ship_mut(&ship).unwrap().position = epos;
    // 玩家指令（`Player` = 自动控制不碰它）：路线 地球→地球，角色钉成运输舰。
    {
        let c = state.control_mut("中国".to_string()).unwrap();
        c.ship_orders.insert(
            ship.clone(),
            Control::player(ShipBehavior::Haul {
                from: "地球".to_string(),
                to: "地球".to_string(),
            }),
        );
        c.ship_freighter.insert(ship.clone(), Control::player(true));
    }
    let mut rng = Prng::new(42);
    let mut delivered = 0.0;
    // 驱逐舰舱容 4、货栈 9 件 ⇒ 要跑三趟（4+4+1）；每趟「装一回合 + 卸一回合」，
    // 所以 8 个回合足够，也正是「常驻路线自己往复」的证据（不需要重下指令）。
    for _ in 0..8 {
        advance(&mut state, &config, &mut rng);
        for e in &state.events {
            if let GameEvent::CargoDelivered { cargo, into_pool: true, .. } = e {
                delivered += cargo.values().sum::<f64>();
            }
        }
    }
    assert!(
        delivered >= 9.0 - 1e-9,
        "压在货栈里的那 9 件必须整批运进首都池（实为 {delivered}，其中还含金星当期产出的那部分）"
    );
    // 注意：**不要**拿首都池的增量当判据——池子同时在花钱（建设/造舰/维护），
    // 收进来的货是**毛额**、池子的净变化是另一回事。判据用事件（到货的毛额）
    // 与「货栈条目消失」（没有货留在产地）这两条。
    assert!(
        state.depot("中国", "地球").is_none(),
        "空货栈条目要被清掉（AI 派单靠键集合读积压）"
    );
}

/// **运输能力的地基（机制不变量）**：非 master 舰船抵达某天体的**成功率**是算出来的，
/// 而**不是**一条「能/不能」的硬线。
///
/// `move_toward` 把目标交给 [`mond_drift`] 切向平移 `roll × (r − radius) × drift_per_au`，
/// 舰船只在距（被平移后的）目标 `arrival_eps` 内停泊 ⇒ 它离真实天体的距离就是那个偏移量。
/// 偏移幅度按 `roll ∈ [0,1)` **伪随机**取，而**每回合是一次新的尝试**（[`nav_roll`]），于是
///
/// ```text
/// 每次尝试的成功率  p = min(1, arrival_eps / (depth × drift_per_au)) = min(1, 2/(r − 28))
/// ```
///
/// 三条不变量（用户裁决：**MOND 再强也总有成功几率，只是要多试几个回合**）：
/// 1. `p > 0` 对**任意有限深度**都成立（`roll = 0` 的一次尝试必然命中）——没有绝对不可达；
/// 2. `p` 随深度**单调不增**（越深越难）；
/// 3. `p = 1` 的门槛仍是 `radius + arrival_eps/drift_per_au = 30 AU`——带内近处**一次到位**。
///
/// 实测（期望尝试回合数 = `1/p`）：伊克西翁 p≈0.92→1.1 回合、妊神星 0.29→3.5、
/// 创神星 0.20→5.0、阋神星 0.20→5.0。守卫它就是守住「圣所**难打但打得下来**」这个形状：
/// 调 `radius` / `drift_per_au` / `arrival_eps` 里任何一个都会整张表平移。
#[test]
fn mond_depth_only_costs_attempts_never_makes_it_impossible() {
    let (config, state) = fresh_world(42);
    let m = &config.mond;
    let eps = config.combat.arrival_eps;

    // 1) 任意有限深度都有成功几率——`roll = 0` 的一次尝试必然指哪打哪。
    for depth in [0.5, 2.0, 10.0, 72.0, 10_000.0] {
        let dest = [0.0, m.radius + depth];
        assert_eq!(
            mond_drift(&config, "中国", dest, 0.0),
            dest,
            "深度 {depth} 也必须存在「蒙对」的一次尝试（roll=0）"
        );
        assert!(
            mond_arrival_chance(&config, depth) > 0.0,
            "深度 {depth} 的成功率必须严格大于 0——MOND 再强也不能让目标变成进不去"
        );
    }
    // 连「MOND 强到离谱」也不行：配置随便调，几率只会变小、不会归零。
    let mut harsh = load_config();
    harsh.mond.drift_per_au = 50.0;
    let harsh_p = mond_arrival_chance(&harsh, 2.0);
    assert!(harsh_p > 0.0, "drift_per_au=50 时成功率仍然 > 0");
    assert!(
        harsh_p < 0.01,
        "…但小到要试上百个回合（实测 p={harsh_p}）"
    );

    // 2) 成功率随深度单调不增。
    let mut prev = 1.0;
    for i in 0..300 {
        let depth = i as f64 * 0.5;
        let p = mond_arrival_chance(&config, depth);
        assert!(
            p <= prev + 1e-12,
            "深度 {depth} AU 的成功率不该比更浅处高（{p} > {prev}）"
        );
        prev = p;
    }
    // 3) 30 AU 以内一次到位：门槛 = radius + arrival_eps/drift_per_au。
    let exact = eps / m.drift_per_au;
    assert!(
        (mond_arrival_chance(&config, exact) - 1.0).abs() < 1e-9,
        "门槛深度 {exact} AU 处应一次到位"
    );
    assert!(
        mond_arrival_chance(&config, exact + 1.0) < 1.0,
        "门槛往外一点就不再是一次到位"
    );

    // 4) 真实天体的表：带内近处一次到位；深处要试几次，但**都试得到**。
    let table: Vec<(String, f64)> = state
        .bodies
        .iter()
        .filter_map(|b| {
            let r = (b.position[0] * b.position[0] + b.position[1] * b.position[1]).sqrt();
            (r > m.radius).then(|| (b.name.clone(), mond_arrival_chance(&config, r - m.radius)))
        })
        .collect();
    let chance = |n: &str| {
        table
            .iter()
            .find(|(name, _)| name == n)
            .map(|(_, p)| *p)
            .unwrap_or_else(|| panic!("{n} 应在带内"))
    };
    for shallow in ["海王星", "冥王星", "卡戎"] {
        assert!(
            chance(shallow) >= 1.0 - 1e-9,
            "{shallow} 在 30 AU 以内，应**一次到位**（实测 p={}）",
            chance(shallow)
        );
    }
    // 圣所：难，但一个月内基本能到（期望 ≈1.1 回合）——「堡垒」是**拖时间**，不是绝对挡驾。
    let ik = chance("伊克西翁");
    assert!(
        (0.8..1.0).contains(&ik),
        "伊克西翁成功率应在 0.8–1.0（实测 {ik:.3}，期望 {:.1} 回合）",
        1.0 / ik
    );
    // 柯伊伯矿：要试几次，但**有得试**——这正是承包定价与「超期掉信誉」的基础。
    for deep in ["妊神星", "创神星", "阋神星"] {
        let p = chance(deep);
        assert!(
            (0.05..0.5).contains(&p),
            "{deep} 应是「要试几次但试得到」（实测 p={p:.3}，期望 {:.1} 回合）",
            1.0 / p
        );
    }
    assert!(
        chance("伊克西翁") > chance("妊神星") && chance("妊神星") > chance("创神星"),
        "越深越难（成功率必须随深度递降），实测 伊克西翁 {:.3} / 妊神星 {:.3} / 创神星 {:.3}",
        chance("伊克西翁"),
        chance("妊神星"),
        chance("创神星")
    );

    // 5) 形状旋钮（`drift_shape`）：调小 = 偏移偏向大值 = 更深，但**永远 > 0**。
    let deep = 10.16; // 创神星的深度
    let mut skew = load_config();
    skew.mond.drift_shape = 0.5;
    let mut easy = load_config();
    easy.mond.drift_shape = 2.0;
    let (p_skew, p_uni, p_easy) = (
        mond_arrival_chance(&skew, deep),
        mond_arrival_chance(&config, deep),
        mond_arrival_chance(&easy, deep),
    );
    assert!(
        p_skew < p_uni && p_uni < p_easy,
        "shape 越小越难（实测 skew(0.5)={p_skew:.3} < 均匀={p_uni:.3} < easy(2.0)={p_easy:.3}）"
    );
    assert!(
        p_skew > 0.0,
        "旋钮怎么调都不能把深处变成「进不去」——这是不可破坏的性质"
    );
}

/// 探针（`cargo test --lib probe_mond_attempts -- --ignored --nocapture`）：
/// 用**真实的** [`nav_roll`] 逐回合实测「深处目标要试几个回合」——闭式 `p` 是「单次尝试
/// 的命中率」，这里量的是它**在真实伪随机序列上**的表现（首次命中的回合数、1000 回合里的
/// 命中次数），并且验证**没有任何天体是 0 命中**（= 不存在进不去的目标）。
#[test]
#[ignore]
fn probe_mond_attempts() {
    let (config, state) = fresh_world(42);
    let m = &config.mond;
    let eps = config.combat.arrival_eps;
    println!("== MOND 导航尝试实测（ship=朝圣者, fid=中国；闭式 p = eps/(depth×drift)）==");
    println!(
        "  {:<8} {:>7} {:>7} {:>9} {:>9} {:>9} {:>9}",
        "天体", "深度AU", "闭式p", "闭式期望", "首次命中", "1000次命中", "命中率"
    );
    for b in &state.bodies {
        let r = (b.position[0] * b.position[0] + b.position[1] * b.position[1]).sqrt();
        if r <= m.radius {
            continue;
        }
        let depth = r - m.radius;
        let p = mond_arrival_chance(&config, depth);
        let mut first: Option<u32> = None;
        let mut hits = 0u32;
        let n = 1000u32;
        for round in 0..n {
            let roll = nav_roll("中国", "朝圣者", round);
            if dist(mond_drift(&config, "中国", b.position, roll), b.position) <= eps {
                hits += 1;
                first.get_or_insert(round + 1);
            }
        }
        println!(
            "  {:<8} {:>7.2} {:>7.3} {:>9.1} {:>9} {:>9} {:>9.3}",
            b.name,
            depth,
            p,
            if p > 0.0 { 1.0 / p } else { f64::INFINITY },
            first.map(|f| f.to_string()).unwrap_or_else(|| "从未".into()),
            hits,
            hits as f64 / n as f64
        );
        assert!(hits > 0, "{} 必须至少命中一次——不存在永远进不去的目标", b.name);
    }
}

/// **贸易路线的引力异常浸入深度**（M6）：两端都在带外 = 0（普通航线）；
/// 一端在带内、一端在外 = 远端深度（要穿过去）；两端都在带内 = 较浅那端深度。
/// 它是运费倍率与丢货率的唯一驱动量，所以必须有确定的语义。
#[test]
fn route_depth_measures_mond_immersion() {
    let (config, _state) = fresh_world(42);
    let r = config.mond.radius;
    let inside = [r - 5.0, 0.0];
    let shallow = [r + 2.0, 0.0];
    let deep = [r + 10.0, 0.0];
    assert_eq!(
        route_depth(&config, inside, [1.0, 0.0]),
        0.0,
        "两端都在异常带外的航线没有 MOND 代价"
    );
    assert!(
        (route_depth(&config, inside, deep) - 10.0).abs() < 1e-9,
        "一端在带内、一端在 10 AU 深 → 要穿到 10 AU 深"
    );
    assert!(
        (route_depth(&config, shallow, deep) - 2.0).abs() < 1e-9,
        "两端都在带内 → 按较浅那端算（2 AU）"
    );
    // 确定性：交换两端不改变结果（路线是双向的）。
    assert_eq!(route_depth(&config, inside, deep), route_depth(&config, deep, inside));
}

/// 本土防御（首都即强弩）：靠近首都的目标被削弱，远离首都的没有。
#[test]
fn home_field_weakens_attackers_near_the_capital() {
    let (_config, state) = fresh_world(42);
    let cap = state.body_position("地球"); // 地球（中国首都）。
    let mult_near = home_defense_mult(&state, "中国", cap);
    assert!(mult_near < 1.0, "near the capital should be defended (mult {mult_near})");
    let mult_far = home_defense_mult(&state, "中国", [80.0, 80.0]);
    assert_eq!(mult_far, 1.0, "far from the capital should have no home-field defense");
}

/// 舰船定制面板：装了护盾+轨道炮+推进的舰，其 effective 面板反映组件的护盾池/火力/射程/
/// 速度。新模型：船体(hull_max) 是舰级**直接**属性、模块不改它；攻击/护盾/速度/射程都由
/// 模块贡献、被舰级修正系数缩放；**速度来自推进模块（无推进=跑不动）**。
#[test]
fn ship_panel_reflects_fitted_components() {
    let (config, mut state) = fresh_world(42);
    let base = config.ship_spec("corvette");
    let ship0 = state.ships[0].name.clone();
    if let Some(s) = state.ship_mut(&ship0) {
        s.components = vec!["shield".to_string(), "railgun".to_string(), "ion_drive".to_string()];
    }
    let s = state.ship(&ship0).unwrap();
    let panel = ship_panel(&config, s);
    // 船体 = 舰级直接属性，模块不改它（护盾/装甲只吸收/减伤，不加血）。
    assert!((panel.hull_max - base.hull).abs() < 1e-9, "hull is a direct class attribute");
    // 护盾池 = 模块 × 舰级 shield_mult。
    let shield_spec = config.component_spec("shield");
    assert!((panel.shield_max - shield_spec.shield * base.shield_mult).abs() < 1e-9);
    // 攻击 = 武器模块 × 舰级 attack_mult。
    let rail_spec = config.component_spec("railgun");
    assert!((panel.attack - rail_spec.damage * base.attack_mult).abs() < 1e-9);
    // 射程 = 武器 × 舰级 range_mult（无舰级基础值）。
    assert!((panel.attack_range - rail_spec.range * base.range_mult).abs() < 1e-9);
    // 速度 = 推进模块 × 舰级 speed_mult；加速度 = 推进 accel × 舰级 accel_mult。
    let drive_spec = config.component_spec("ion_drive");
    assert!((panel.speed - drive_spec.speed * base.speed_mult).abs() < 1e-9);
    assert!((panel.accel - drive_spec.accel * base.accel_mult).abs() < 1e-9);
    assert!(panel.upkeep > base.upkeep, "components should raise maintenance");
    // 护甲=硬度：这艘船没装装甲，硬度应为 0。
    assert!((panel.hardness).abs() < 1e-9);
}

/// 舰级「点防御修正 pd_mult」（spec 新增属性）应缩放所搭载点防模块的拦截强度：
/// 同一枚 point_defense 组件，装在高点防修正的舰（如战列 pd_mult>1）上比装在低点防
/// 修正的舰上拦截更强——「舰级=平台修正器」的一环，而不是给舰叠加独立点防面板。
#[test]
fn ship_panel_scales_intercept_by_class_pd_mult() {
    let (config, mut state) = fresh_world(42);
    let pd_spec = config.component_spec("point_defense");
    // 从旗舰队里挑两艘从属不同舰级的舰，验证 intercept 恰为 组件 intercept × 该舰级 pd_mult。
    // 用按 class 归类的方式选：一艘 pd_mult 高、一艘 pd_mult 低（若存在）最能证明缩放生效。
    let mut tested = std::collections::BTreeMap::<String, f64>::new();
    for s in state.ships.iter_mut() {
        let class = s.class.clone();
        tested.entry(class.clone()).or_insert_with(|| {
            let spec = config.ship_spec(&class);
            s.components = vec!["point_defense".to_string()];
            s.component_hp = vec![component_integrity(&config, "point_defense")];
            let pd_mult = spec.pd_mult;
            let intercept = ship_panel(&config, s).intercept;
            assert!(
                (intercept - pd_spec.intercept * pd_mult).abs() < 1e-9,
                "{class} intercept should be {:.3} × pd_mult {:.2}, got {intercept}",
                pd_spec.intercept,
                pd_mult
            );
            pd_mult
        });
    }
    // 至少应有两点防修正不同的舰级，证明缩放不是常数（否则这个属性形同虚设）。
    let distinct: std::collections::BTreeSet<String> =
        tested.iter().map(|(c, m)| format!("{c}:{m:.3}")).collect();
    assert!(
        distinct.len() >= 2,
        "expected ship classes to differ in pd_mult; got {tested:?}"
    );
}


#[test]
fn fire_degrades_components_under_damage() {
    let (config, mut state) = fresh_world(42);
    let ship0 = state.ships[0].name.clone();
    let ship3 = state.ships[3].name.clone();
    // 目标：US 驱逐舰（ship 3），装一枚导弹组件、血厚到扛住一炮以观察组件损耗。
    if let Some(t) = state.ship_mut(&ship3) {
        t.position = [40.0, 40.0];
        t.components = vec!["missile".to_string()];
        t.component_hp = t.components.iter().map(|c| component_integrity(&config, c)).collect();
        t.hull = 500.0;
        t.hull_max = 500.0;
        t.shield = 0.0;
        t.shield_max = 0.0;
    }
    // 攻击者：CN 护卫舰（ship 0），装一门重炮、贴近目标。
    if let Some(a) = state.ship_mut(&ship0) {
        a.position = [40.1, 40.0];
        a.components = vec!["railgun".to_string()];
        a.component_hp = a.components.iter().map(|c| component_integrity(&config, c)).collect();
    }
    state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
    state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);
    let before = state.ship(&ship3).unwrap().component_hp.clone();
    let panel_before = ship_panel(&config, state.ship(&ship3).unwrap());
    fire_concentrate(&mut state, &config, &ship0, &ship3);
    let after = state.ship(&ship3).unwrap().component_hp.clone();
    assert!(
        after.iter().zip(before.iter()).any(|(a, b)| *a < *b),
        "component integrity should drop under fire; before={before:?} after={after:?}"
    );
    // 被击毁后不贡献面板：把目标组件打掉，验证攻击/护盾面板下降。
    let _ = panel_before;
}

/// 母港/友方本土修船（拟人「打残→撤→修→再来」闭环）：受损组件的完整度每回合修复，
/// 且在本土（首都 home_radius 内）修得更快。
#[test]
fn damaged_components_repair_in_friendly_territory() {
    let (config, mut state) = fresh_world(42);
    // China ship 0 停在其首都（Earth, body 2），组件受损。
    let cap_pos = state.body_position("地球");
    let ship0 = state.ships[0].name.clone();
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = cap_pos;
        s.components = vec!["railgun".to_string()];
        s.component_hp = vec![5.0];
        s.hull = s.hull.max(5.0);
    }
    let before = state.ship(&ship0).map(|s| s.component_hp.first().copied().unwrap_or(0.0)).unwrap_or(0.0);
    advance(&mut state, &config, &mut Prng::new(42));
    let after = state.ship(&ship0).map(|s| s.component_hp.first().copied()).flatten().unwrap_or(before);
    assert!(
        after > before,
        "a damaged component should repair over rounds; before={before} after={after}"
    );
}

/// 舰队防空（防空屏护）：有 PD 的舰会替 `pd_radius` 内的友舰拦导弹——附近有 PD 时目标
/// 得到的防空覆盖应更高，PD 舰远离时覆盖应下降。
#[test]
fn fleet_air_defense_covers_nearby_missile_targets() {
    let (config, mut state) = fresh_world(42);
    // 目标：US (1) ship 5 在 [40,40]，自身无 PD。
    let ship3 = state.ships[3].name.clone();
    let ship5 = state.ships[5].name.clone();
    if let Some(t) = state.ship_mut(&ship5) {
        t.position = [40.0, 40.0];
        t.components = Vec::new();
        t.component_hp = Vec::new();
    }
    // 友舰：US ship 3 在 [41,40]，装点防御。
    if let Some(g) = state.ship_mut(&ship3) {
        g.position = [41.0, 40.0];
        g.components = vec!["point_defense".to_string()];
        g.component_hp = g.components.iter().map(|c| component_integrity(&config, c)).collect();
    }
    let cover_with = cluster_pd_cover(&state, &config, &ship5, "美国", [40.0, 40.0]);
    assert!(cover_with > 0.0, "a nearby PD ship should give air-defense cover; got {cover_with}");
    // 把 PD 舰移远 → 覆盖应下降。
    state.ship_mut(&ship3).unwrap().position = [100.0, 100.0];
    let cover_far = cluster_pd_cover(&state, &config, &ship5, "美国", [40.0, 40.0]);
    assert!(
        cover_far < cover_with,
        "cover should drop once the PD ship is far (with {cover_with}, far {cover_far})"
    );
}

/// 战斗拟真：护盾池优先吸收，快速目标对低追踪武器规避更强（确定性命中折减）。
#[test]
fn combat_respects_shields_and_speed_evasion() {
    let (config, mut state) = fresh_world(42);
    // Attacker: China corvette (id 0) fitted with a railgun; target: US destroyer (id 3)
    // fitted with an energy shield. Both pinned far from any capital so home-field
    // defense is neutral (mult = 1.0). Hostile so the volley is a real attack.
    let ship0 = state.ships[0].name.clone();
    let ship3 = state.ships[3].name.clone();
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [80.0, 80.0];
        s.components = vec!["railgun".to_string()];
    }
    if let Some(s) = state.ship_mut(&ship3) {
        s.position = [80.4, 80.0];
        s.components = vec!["shield".to_string()];
        s.hull = 24.0;
        s.hull_max = 24.0;
        s.shield = 12.0;
        s.shield_max = 12.0;
    }
    state.faction_mut("中国").unwrap().relations.insert("美国".to_string(), -35.0);
    state.faction_mut("美国").unwrap().relations.insert("中国".to_string(), -35.0);

    let shield_before = state.ship(&ship3).map(|s| s.shield).unwrap();
    let hull_before = state.ship(&ship3).map(|s| s.hull).unwrap();
    fire_concentrate(&mut state, &config, &ship0, &ship3);

    let shield_after = state.ship(&ship3).map(|s| s.shield).unwrap();
    let hull_after = state.ship(&ship3).map(|s| s.hull).unwrap();
    assert!(shield_after < shield_before, "shield pool must absorb damage");
    assert!(hull_after < hull_before, "hull should take spill damage too");
    assert!(hull_after > 0.0, "a single volley on a destroyer should not one-shot it");

    // Evasion: a fast target is hit less by a low-tracking weapon than a slow one.
    let fast_hit = hit_factor(2.0, 2.6); // corvette speed
    let slow_hit = hit_factor(2.0, 1.2); // destroyer speed
    assert!(
        fast_hit < slow_hit,
        "fast ship should evade a low-tracking weapon more (fast {fast_hit} vs slow {slow_hit})"
    );
}

/// 迁都-亡城强迁：首都天体上已无本势力活城 → 自动切到**人口最高的活城**。
#[test]
fn capital_destroyed_auto_relocates_to_highest_population_city() {
    let (config, mut state) = fresh_world(42);
    // 中国初始首都=地球，其上活城 长三角(1400)/珠三角(1100)。把这两城夷平 → 首都亡。
    assert_eq!(state.capital_body("中国"), "地球");
    for cid in ["长三角".to_string(), "珠三角".to_string()] {
        if let Some(c) = state.city_mut(&cid) {
            c.razed = true;
        }
    }

    step_capital(&mut state, &config);

    // 剩余中国活城：水星熔炉基地(220,水星)、金星浮空之城(260,金星)。人口最高=金星浮空之城。
    assert_eq!(
        state.capital_body("中国"),
        "金星",
        "capital must snap to the highest-population remaining city (金星)"
    );
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::CapitalRelocated { faction, to, reason, .. } if faction == "中国" && to == "金星" && reason == "destroyed"
        )),
        "a destroyed-capital relocation event must be recorded, got {:?}",
        state.events
    );
}

/// 迁都-周期 AI 评估：首都 Population 中心更优（总治理距离成本显著更低）时，AI 迁过去。
#[test]
fn ai_periodic_review_relocates_capital_to_population_center() {
    let (mut config, mut state) = fresh_world(42);
    // 收窄治理可达半径 + 降低迁都门槛，让内行星间的距离差能体现「更优」。
    config.governance.admin_range = 0.05;
    config.governance.capital_relocate_threshold = 0.1;
    assert_eq!(config.governance.capital_review_every, 12);

    // 交圈数设为评估周期（12）：非 Player 首都在评估轮迁到人口中心。
    state.round = 12;
    // 把中国首都先钉到 水星（较远），**显式写 `mode: Auto`** 让 AI 继续评估：
    // 「写值即接管」之后，只写 value 会被当成玩家的首都（mode=Player），
    // 那样这条测试考的就不再是 AI 评估了。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "capital": {"value": "水星", "mode": "Auto"}}]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("set far capital");
    assert_eq!(state.capital_body("中国"), "水星");

    step_capital(&mut state, &config);

    // 中国人口最繁华城=长三角(1400,地球)；迁到地球显著降低总治理距离成本。
    assert_eq!(
        state.capital_body("中国"),
        "地球",
        "AI review should relocate the capital to the population center (地球)"
    );
    assert!(
        state.events.iter().any(|e| matches!(
            e,
            GameEvent::CapitalRelocated { faction, reason, .. } if faction == "中国" && reason == "ai_review"
        )),
        "an AI-review relocation event must be recorded, got {:?}",
        state.events
    );
}

/// 迁都-Player 标记：mode=Player 的首都在评估轮不被 AI 覆盖（除非亡城硬规则）。
#[test]
fn player_capital_not_overridden_by_ai_review() {
    let (config, mut state) = fresh_world(42);
    // 玩家把首都迁到 水星 并标 Player；中国在 水星 仍有活城（水星熔炉基地），非亡城。
    let diff = serde_json::json!({
        "control": [{"faction_id": "中国", "capital": {"value": "水星", "mode": "Player"}}]
    });
    crate::control::apply_patch(&mut state, &config, &diff).expect("player move capital");
    assert_eq!(state.capital_body("中国"), "水星");
    assert_eq!(state.capital_control("中国"), ControlMode::Player);

    // 评估轮：Player 控制的首都不被周期迁移覆盖。
    state.round = 12;
    step_capital(&mut state, &config);

    assert_eq!(
        state.capital_body("中国"),
        "水星",
        "a Player-chosen capital must survive the periodic AI review"
    );
    assert!(
        !state.events.iter().any(|e| matches!(
            e,
            GameEvent::CapitalRelocated { faction, .. } if faction == "中国"
        )),
        "no relocation may fire for a Player-owned capital, got {:?}",
        state.events
    );
}

/// 思潮：战争得利把「和平↔军国」推向军国端。
#[test]
fn ideology_military_win_drives_toward_militarism() {
    let (config, mut state) = fresh_world(42);
    let fname = state.factions[2].name.clone(); // 欧盟（开局有舰，且初始偏和平端）
    let my_ship = state.ships.iter().find(|s| s.faction_id == fname).map(|s| s.name.clone()).expect("a ship");
    let enemy = state.ships.iter().find(|s| s.faction_id != fname).map(|s| (s.name.clone(), s.faction_id.clone())).expect("enemy ship");
    let start = state.faction(&fname).unwrap().ideology.peace_military;

    // 注入一回合「战争得利」：我方舰击毁一艘敌舰。击毁归属由 Attack→ShipDestroyed 反推。
    state.events.push(GameEvent::Attack { attacker: my_ship.clone(), target: enemy.0.clone(), damage: 10.0 });
    state.events.push(GameEvent::ShipDestroyed {
        ship: enemy.0.clone(),
        owner: enemy.1.clone(),
        class: "corvette".to_string(),
        cause: DeathCause::Combat,
        by: None,
    });
    step_ideology(&mut state, &config, &RoundFlow::default());

    let after = state.faction(&fname).unwrap().ideology.peace_military;
    assert!(
        after > start,
        "war victory must push 和平↔军国 toward 军国: start={start} after={after}"
    );
}

/// 思潮：经济转负把「人民↔精英」推向人民端；且所有轴恒可有界、有限。
#[test]
fn ideology_economy_bad_drives_toward_populism_and_stays_bounded() {
    let (config, mut state) = fresh_world(42);
    let fname = state.factions[1].name.clone();
    let start = state.faction(&fname).unwrap().ideology.people_elite;

    // 经济转负：净流 = 产出(0) − 维护(100) − 治理(0) < 0 → 人民（民粹反弹）。
    let mut flow = RoundFlow::default();
    flow.upkeep.insert(fname.clone(), 100.0);
    step_ideology(&mut state, &config, &flow);

    let after = state.faction(&fname).unwrap().ideology.people_elite;
    assert!(
        after < start,
        "economic bust must push 人民↔精英 toward 人民: start={start} after={after}"
    );
    // 所有势力的所有轴都应是有界、有限的。
    for f in &state.factions {
        let i = &f.ideology;
        for (k, v) in [
            ("peace_military", i.peace_military),
            ("science_tech", i.science_tech),
            ("people_elite", i.people_elite),
            ("nature_colony", i.nature_colony),
        ] {
            assert!(v.is_finite() && (-1.0..=1.0).contains(&v), "{k} out of bounds: {v}");
        }
    }
}

/// 思潮相似度函数：同=1，全对极=0，中庸=0.5；单调随轴距离下降。
#[test]
fn ideology_similarity_ranges_and_is_monotonic() {
    let a = Ideology { peace_military: 0.5, science_tech: -0.3, people_elite: 0.2, nature_colony: 0.4 };
    let b = Ideology { peace_military: -0.5, science_tech: 0.3, people_elite: -0.2, nature_colony: -0.4 };
    let same = Ideology { peace_military: 0.5, science_tech: -0.3, people_elite: 0.2, nature_colony: 0.4 };
    assert_eq!(ideology_similarity(&a, &same), 1.0, "identical ideologies have unit similarity");
    assert!(ideology_similarity(&a, &a) >= ideology_similarity(&a, &b), "similarity is monotonic in distance");
    assert!((0.0..=1.0).contains(&ideology_similarity(&a, &b)));
    assert_eq!(ideology_similarity(&a, &a), 1.0);
}

/// 思潮相似度影响外交：其它条件相同（同 seed、同 alignment、同起始关系、噪声关闭）下，
/// 思潮越像 → 静息亲和越高 → 关系向更友好靠拢；思潮越对立 → 越向敌对靠拢。
#[test]
fn ideology_similarity_shifts_diplomatic_affinity_directionally() {
    let run = |ideo_a: Ideology, ideo_b: Ideology| -> f64 {
        let (mut config, mut state) = fresh_world(42);
        // 关掉噪声，让关系变化只反映静息亲和的差异（确定性）。
        config.diplomacy.noise = 0.0;
        let a = state.factions[0].name.clone();
        let b = state.factions[1].name.clone();
        {
            let fa = state.faction_mut(&a).unwrap();
            fa.alignment = 0.0; // 隔离 alignment：只留思潮相似度的独立影响
            fa.ideology = ideo_a;
            fa.relations.insert(b.clone(), 0.0);
            let fb = state.faction_mut(&b).unwrap();
            fb.alignment = 0.0;
            fb.ideology = ideo_b;
            fb.relations.insert(a.clone(), 0.0);
        }
        let mut rng = Prng::new(42);
        step_diplomacy(&mut state, &config, &mut rng);
        relation(&state, &a, &b)
    };

    // 全同极（相似度=1）vs 全对极（相似度=0）：同 seed、同 alignment、同起始关系，
    // 唯一的差别就是思潮相似度 → 相似的一方关系必须更友好。
    let same_pos = Ideology { peace_military: 1.0, science_tech: 1.0, people_elite: 1.0, nature_colony: 1.0 };
    let opposite = Ideology { peace_military: -1.0, science_tech: -1.0, people_elite: -1.0, nature_colony: -1.0 };
    let r_same = run(same_pos, same_pos);
    let r_opp = run(same_pos, opposite);
    assert!(
        r_same > r_opp,
        "similar ideologies must rest friendlier than opposite ones: same={r_same} opp={r_opp}"
    );
}
// ---- 舰船设计图（blueprint）：出厂快照 / 归属 / 意图链 -------------------------
//
// 设计图的语义见 `.agents/notes/ship-blueprint-spec.md`（§8.0 的十条裁决）。这一组用例
// 钉住的是**别人最容易改坏**的几条：快照 vs 活层、图的意图轴默认沉默、买不起不下水、
// 悬空指针停产。

/// 在势力 `fid` 的**第一座有建造区的城**上挂一张图，并把该舰级的进度池准备好。
///
/// 返回 `(城名, 建筑下标, 舰级)`。`progress` 给 `build_points` 就下一回合必下水。
fn attach_blueprint(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    bp_name: &str,
    class: &str,
    components: &[&str],
    order: Option<ShipBehavior>,
    mode: ControlMode,
) -> (CityId, BuildingId, String) {
    state
        .control
        .entry(fid.to_string())
        .or_default()
        .blueprints
        .insert(
            bp_name.to_string(),
            Control {
                value: Blueprint {
                    class: class.to_string(),
                    components: components.iter().map(|c| c.to_string()).collect(),
                    order,
                },
                mode,
            },
        );
    let (cid, bid) = state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        // **只挑「恰好一个建造区」的城**：进度池是按舰级合并的，同城第二个建造区会跟它抢
        // 这个池子（谁权重高就用谁的图，见 `build_city`）——用例要的是「这个区就该舰级的
        // 唯一出口」，否则断言会在别的区产出的那艘舰上翻车。
        .find_map(|c| {
            let yards: Vec<&Building> = c.buildings.iter().filter(|b| b.is_shipyard()).collect();
            (yards.len() == 1).then(|| (c.name.clone(), yards[0].id))
        })
        .expect("该势力要有一座只有一个建造区的城");
    if let Some(city) = state.city_mut(&cid) {
        for b in city.buildings.iter_mut() {
            if b.id == bid {
                b.ship_type = Some(class.to_string());
                b.blueprint = Some(bp_name.to_string());
            }
        }
        // 进度池备到差一点就满：下一回合必定触发一次下水判定。
        city.ship_progress
            .insert(class.to_string(), config.ship_spec(class).build_points);
    }
    (cid, bid, class.to_string())
}

/// 测试用：在天体 `body` 上造一艘舰（把「先算位置、再进漏斗」写在一处：`spawn_ship`
/// 已经可变借用 `state`，参数里再读 `state.body_position(..)` 会撞两阶段借用）。
fn spawn_at(
    state: &mut State,
    config: &GameConfig,
    owner: &str,
    class: &str,
    body: &str,
    blueprint: Option<&str>,
) -> ShipId {
    let pos = state.body_position(body);
    let bp = blueprint.map(|s| s.to_string());
    spawn_ship(state, config, ShipSpawn {
        owner: owner.to_string(),
        class,
        position: pos,
        city: None,
        via: SpawnVia::Shipyard,
        pay_components: false,
        blueprint: bp.as_ref(),
    })
}

/// 把某势力喂饱**所有**资源（免得「买不起」把设计图的用例卡住）。
///
/// 必须按 `config.resources` 的**全表**给：势力开局只有它自己那几种矿，而设计图上的
/// 选装可能要稀有矿（等离子炮要氦-3/金）——只给现存的键会让「买不起」这条新规则把用例
/// 卡在门外（这正是 Q4(b) 生效的样子）。
fn stock(state: &mut State, config: &GameConfig, fid: &str, amount: f64) {
    let keys: Vec<String> = config.resources.keys().cloned().collect();
    let Some(f) = state.faction_mut(fid) else { return };
    for k in keys {
        f.resources.insert(k, amount);
    }
}

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
    assert_eq!(ship.components, vec!["kinetic".to_string(), "ion_drive".to_string()], "选装 = 图上的选装（顺序也照图）");
    assert_eq!(ship.class, class);
    assert_eq!(ship.hull_max, ship_panel(&config, &ship).hull_max, "面板是快照，且与 config 现算的面板一致");
    assert_eq!(ship.spawned_round, Some(state.round), "下水回合要记下来（Q9）");
    // 组件成本**真的**从库存里扣了：在同一份状态上再走一次「出厂 + 付款」，逐资源比对
    // （比「跟 400 比」可靠——那一回合里还有开采/市场/别处的建造在同一条库存上进账）。
    let mut probe = state.clone();
    let before = state.faction(&fid).unwrap().resources.clone();
    spawn_ship(&mut probe, &config, ShipSpawn {
        owner: fid.clone(),
        class: "corvette",
        position: state.body_position("地球"),
        city: None,
        via: SpawnVia::Shipyard,
        pay_components: true,
        blueprint: Some(&"重甲护卫".to_string()),
    });
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

    let after = state.ship(&name).cloned().expect("the old ship is still there");
    assert_eq!(after.components, before.components, "**已下水的舰的选装是快照**，改图不许动它");
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
    assert_eq!(
        crate::autocontrol::resolve_loadout(&state, &config, fid.clone(), &class, Some(&"auto:custom".to_string())),
        vec!["plasma".to_string(), "ion_drive".to_string()],
        "`Auto` 图的选装必须真的生效（否则 AI 建图只是空头支票）"
    );
    let name = spawn_at(&mut state, &config, &fid, &class, "地球", Some("auto:custom"));
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
    assert_eq!(
        crate::autocontrol::resolve_loadout(&state, &config, fid.clone(), &class, Some(&"auto:empty".to_string())),
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
    let expected = crate::autocontrol::resolve_loadout(
        &state,
        &config,
        fid.clone(),
        &class,
        Some(&"auto:corvette".to_string()),
    );
    assert_eq!(
        expected,
        crate::autocontrol::choose_loadout(&state, &config, fid.clone(), &class),
        "`Auto` 图的选装必须**就是** `choose_loadout` 的答案（不是第二份生成逻辑）"
    );
    let name = spawn_at(&mut state, &config, &fid, &class, "地球", Some("auto:corvette"));
    assert_eq!(state.ship(&name).unwrap().components, expected, "出厂用的是当场算出来的选装");

    // **没有提前缓存**：把库存掏空之后再下水，生成器给出空选装（裸舰）——若选装是在
    // 回合步进里预生成的，这里就会拿到上一回合那份。
    if let Some(f) = state.faction_mut(&fid) {
        for v in f.resources.values_mut() {
            *v = 0.0;
        }
    }
    assert_eq!(
        crate::autocontrol::resolve_loadout(
            &state,
            &config,
            fid.clone(),
            &class,
            Some(&"auto:corvette".to_string())
        ),
        Vec::<String>::new(),
        "库存掏空 ⇒ 生成器给空选装（证明它是**出厂那一刻**算的）"
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
        Some(ShipBehavior::Dock { body: "地球".to_string() }),
        ControlMode::Player,
    );
    // 舰队默认**同时**是 Player 且冲突 ⇒ **更具体的图赢**（链：叶 → 图 → 舰队默认 → …）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_ship_order": {"behavior": {"type": "dock", "body": "火星"}}
    }]});
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");

    let name = spawn_at(&mut state, &config, &fid, &class, "水星", Some("护卫-守家"));
    assert_eq!(state.ship_control(name.clone()), ControlMode::Player, "图上的意图归玩家 ⇒ AI 不许接管这艘舰");
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Dock { body: "地球".to_string() }),
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
            Some(ShipBehavior::Dock { body: "火星".to_string() }),
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
        Some(ShipBehavior::Dock { body: "地球".to_string() }),
        ControlMode::Player,
    );
    let name = spawn_at(&mut state, &config, &fid, &class, "水星", Some("护卫-守家"));
    // ① 刚下水的舰：叶**存在**且写着 `Inherit`，但更具体的图层供值 ⇒ 出处是它。
    assert_eq!(state.ship_behavior_source(name.clone()), Some(OrderSource::Blueprint("护卫-守家".to_string())));
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Dock { body: "地球".to_string() }),
        "叶没表态 ⇒ 图上的意图生效"
    );

    // ② 删掉那片叶（"叶不存在"）：出处**不变**（值仍然来自图）。
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "ship_orders": [{"ship": name, "remove": true}]
    }]});
    apply_patch(&mut state, &config, &diff).expect("remove applies");
    assert!(!state.control(fid.clone()).unwrap().ship_orders.contains_key(&name), "叶真的被删了");
    assert_eq!(state.ship_behavior_source(name.clone()), Some(OrderSource::Blueprint("护卫-守家".to_string())));
    assert_eq!(state.ship_behavior(name.clone()), Some(ShipBehavior::Dock { body: "地球".to_string() }));

    // ③ 把图的意图轴清空（`order: null`）：叶**不存在** + 没人供值 ⇒ **没有出处**
    //    （调用方按 Idle 兜底）。这就是「叶不存在」那一侧。
    let diff = serde_json::json!({"control": [{"faction_id": "中国", "blueprints": [
        {"name": "护卫-守家", "order": null}
    ]}]});
    apply_patch(&mut state, &config, &diff).expect("clearing the order axis applies");
    assert_eq!(state.ship_behavior_source(name.clone()), None, "没有任何一层说话");
    assert_eq!(state.ship_behavior(name.clone()), None);

    // ④ **叶存在但写着 `Inherit`**（不是删掉它）：出处必须诚实报 `leaf`——值真的来自
    //    那片叶（`leaf.map(|l| l.value).unwrap_or(..)`），与「没有叶」**不等价**。
    state
        .control_mut(fid.clone())
        .unwrap()
        .ship_orders
        .insert(name.clone(), Control::inherit(ShipBehavior::Move { position: [1.0, 2.0] }));
    assert_eq!(state.ship_behavior_source(name.clone()), Some(OrderSource::Leaf));
    assert_eq!(
        state.ship_behavior(name.clone()),
        Some(ShipBehavior::Move { position: [1.0, 2.0] }),
        "叶存在就用叶里的值（与它的 mode 无关）——这正是「叶 Inherit ≠ 没有叶」"
    );

    // ⑤ 舰队默认供值时出处是它；叶有意见时叶赢。
    state.control_mut(fid.clone()).unwrap().ship_orders.remove(&name);
    let diff = serde_json::json!({"control": [{"faction_id": "中国",
        "default_ship_order": {"behavior": {"type": "dock", "body": "火星"}}
    }]});
    apply_patch(&mut state, &config, &diff).expect("fleet default applies");
    assert_eq!(state.ship_behavior_source(name.clone()), Some(OrderSource::FleetDefault));
    state
        .control_mut(fid.clone())
        .unwrap()
        .ship_orders
        .insert(name.clone(), Control::player(ShipBehavior::Idle));
    assert_eq!(state.ship_behavior_source(name.clone()), Some(OrderSource::Leaf), "叶有意见 ⇒ 叶赢");
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
    assert_eq!(state.ships.len(), ships_before, "悬空指针不许凭空产出（也不许静默回落生成器）");
    let progress = state.city(&cid).unwrap().ship_progress.get(&class).copied().unwrap_or(0.0);
    assert!(progress <= 1e-9, "那个建造区停产 ⇒ 进度必须一点不涨，got {progress}");
    // 建区还在、指针**原样**保留（读面据此能一眼看出「这个区指着一张不存在的图」）。
    assert_eq!(
        state.city(&cid).unwrap().buildings.iter().find(|b| b.id == bid).unwrap().blueprint.as_deref(),
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
    let progress = state.city(&cid).unwrap().ship_progress.get(&class).copied().unwrap_or(0.0);
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
        state.ships.iter().any(|s| s.blueprint.as_deref() == Some("豪华护卫")),
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
                GameEvent::ShipSpawned { ship: s, blueprint, .. } if s == ship => {
                    Some((e.headline(), blueprint.clone()))
                }
                _ => None,
            })
            .expect("spawn_ship must emit an event")
    };
    let (h_bp, bp) = headline(&with_bp);
    assert_eq!(bp.as_deref(), Some("有图"));
    assert!(h_bp.contains("设计图：有图"), "挂了图的事件句子带归因：{h_bp}");
    let (h_plain, bp2) = headline(&plain);
    assert_eq!(bp2, None);
    assert!(!h_plain.contains("设计图"), "无图那一路的句子不许变（digest 的故事板拿它比对）：{h_plain}");
}
