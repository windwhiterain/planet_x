//! 战术决策的单元测试。

use super::*;
use crate::config::load_config;
use crate::model::GameEvent;
use crate::prng::Prng;
use crate::sim::{advance, dist};
use crate::world::default_state;

/// Build the config + a fresh deterministic world (round 0)，并把**角色轴钉成「全员战舰」**。
///
/// 这里 8 个用例测的都是**战术**（接战/撤退/护航/轰炸），不是集货。自动控制现在多了一条
/// 活：按积压定编、派船去跑运输路线——那会让被测的舰不在它该在的位置上（实测踩过：
/// 中国的驱逐舰成了运输舰，在金星原地装 0.75 件碳，于是「打残了该撤」的用例不再撤）。
/// 见 [`crate::world::pin_roles_to_war`]：它顺带也演示了「玩家意图能压住 AI 定编」。
fn fresh_world(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let mut state = default_state(&config, seed);
    crate::world::pin_roles_to_war(&mut state);
    (config, state)
}

/// 护航目标：旗舰 = 本势力最小的航母（高价值舰种），用于让闲着的舰护卫它。
#[test]
fn fleet_flag_is_the_factions_first_carrier() {
    let (_config, mut state) = fresh_world(42);
    // China (3) ship 2 设为航母 → 成为旗舰。
    let ship2 = state.ships[2].name.clone();
    if let Some(s) = state.ship_mut(&ship2) {
        s.class = "carrier".to_string();
    }
    assert_eq!(fleet_flag(&state, "中国"), Some(ship2));
    // 没有航母 → 无旗舰（无护航）。
}

/// 拟人的「不追远敌」（驻守而非过度延伸）：超出追击半径的敌对舰不应被选为追击目标。
#[test]
fn ships_do_not_chase_enemies_beyond_pursuit_range() {
    let (config, mut state) = fresh_world(42);
    // 中国 ship 0 在 [40,40]；把 US 的 ship 5 放到远处（远超 pursuit_range 12）。
    let ship0 = state.ships[0].name.clone();
    let ship5 = state.ships[5].name.clone();
    if let Some(s) = state.ship_mut(&ship5) {
        s.position = [140.0, 40.0];
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let picked = pick_target(
        &state,
        &config,
        "中国",
        [40.0, 40.0],
        &mut Prng::new(1),
        None,
        &ship0,
    );
    // 远处那艘敌舰不应被选中（超出追击半径）；可能选中更近的目标或城市/空。
    let chased = matches!(picked, Some(ShipBehavior::Follow { ship: ref s }) if *s == ship5);
    assert!(
        !chased,
        "a hostile beyond pursuit_range should not be chased; got {picked:?}"
    );
}

/// 风筝<->贴脸（kiting）软移动：附近有敌舰时，`kiting<0` 的舰被推到「最远武器射程」处
/// （敌近则拉开），`kiting>0` 的舰压近到目标；`kiting=0`（基线）不调整（None）。
#[test]
fn kiting_repositions_relative_to_nearby_enemy() {
    let (config, mut state) = fresh_world(42);
    let ship0 = state.ships[0].name.clone();
    let ship3 = state.ships[3].name.clone();
    let ship4 = state.ships[4].name.clone();
    let ship5 = state.ships[5].name.clone();
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [0.0, 0.0];
        s.kiting = -1.0; // 风筝
    }
    if let Some(e) = state.ship_mut(&ship3) {
        e.position = [0.3, 0.0];
    }
    // 其余美国舰挪远，确保最近敌舰就是 ship3。
    if let Some(s) = state.ship_mut(&ship4) {
        s.position = [50.0, 50.0];
    }
    if let Some(s) = state.ship_mut(&ship5) {
        s.position = [50.0, 50.0];
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let range = crate::model::ship_panel(&config, state.ship(&ship0).unwrap()).attack_range;
    let en = state.ship(&ship3).unwrap().position;
    // 风筝：目的地的敌我距离应拉到「最远武器射程」处（敌更近则被推开）。
    let kite = kiting_dest(&state, &config, &ship0).expect("kite ship has a dest");
    assert!(
        dist(kite, en) >= range - 1e-9,
        "kite ship should hold at weapon range; dest={kite:?} enemy={en:?} range={range}"
    );
    // 贴脸：压近到目标。
    if let Some(s) = state.ship_mut(&ship0) {
        s.kiting = 1.0;
    }
    let close = kiting_dest(&state, &config, &ship0).expect("face-hug ship has a dest");
    let d_now = dist([0.0, 0.0], en);
    let d_close = dist(close, en);
    assert!(
        d_close <= d_now,
        "face-hug ship should close in; dest={close:?} d_now={d_now} d_close={d_close}"
    );
    // 基线：kiting=0 不调整。
    if let Some(s) = state.ship_mut(&ship0) {
        s.kiting = 0.0;
    }
    assert!(
        kiting_dest(&state, &config, &ship0).is_none(),
        "baseline kiting must be None"
    );
}

/// 火力分配（雨露均沾）：一件 `fire_spread>0`、`fire_rate>1` 的武器，会把本回合的多发
/// 摊给**多个**目标——按攻击历史新鲜度，刚打过的目标在下一次选择时权重被降低。
#[test]
fn spread_weapon_distributes_fire_across_targets() {
    let (mut config, mut state) = fresh_world(42);
    // 让导弹变成「雨露均沾 + 两连发」，攻击舰装它、站在 [40,40]。
    if let Some(c) = config.components.get_mut("missile") {
        c.fire_rate = 2.0;
        c.fire_spread = 1.0;
    }
    let ship0 = state.ships[0].name.clone();
    let ship3 = state.ships[3].name.clone();
    let ship5 = state.ships[5].name.clone();
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [40.0, 40.0];
        s.components = vec!["missile".to_string()];
    }
    if let Some(s) = state.ship_mut(&ship3) {
        s.position = [40.1, 40.0];
    }
    if let Some(s) = state.ship_mut(&ship5) {
        s.position = [40.2, 40.0];
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let plan = build_fire_plan(&state, &config, &ship0);
    assert_eq!(
        plan.len(),
        2,
        "a fire_rate=2 weapon should fire 2 shots, got {plan:?}"
    );
    let first = plan[0].1.clone();
    let second = plan[1].1.clone();
    assert!(
        first != second,
        "a 雨露均沾 (fire_spread>0) weapon should spread its 2 shots across 2 targets, got {plan:?}"
    );
}

/// 理智<->热血（威慑对比）：一个热血(temper>0)的舰应倾向攻击**威慑高于自己**的目标
/// （飞蛾扑火），而理智(temper<0)应倾向攻击威慑低于自己的目标（欺软怕硬）。
#[test]
fn temper_biases_toward_weaker_or_stronger_deterrence() {
    let (mut config, mut state) = fresh_world(42);
    // 让导弹射程极大（能同时看到两处**分离**的敌群），并把两群目标放到不同簇（相隔
    // 远超 deterrence_radius），使它们的「舰队威慑」真正不同。
    if let Some(c) = config.components.get_mut("missile") {
        c.range = 200.0;
    }
    let ship0 = state.ships[0].name.clone();
    let ship1 = state.ships[1].name.clone();
    let ship3 = state.ships[3].name.clone();
    let ship4 = state.ships[4].name.clone();
    let ship5 = state.ships[5].name.clone();
    // 攻击舰 ship0 装导弹、站在 [40,40]；把中国队其它舰移远以免污染攻击方威慑。
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [40.0, 40.0];
        s.components = vec!["missile".to_string()];
    }
    if let Some(s) = state.ship_mut(&ship1) {
        s.position = [600.0, 600.0];
    }
    // 弱目标 ship3：孤立、空载（威慑低）。强目标 ship5：重装（威慑高）。
    // 两簇相隔 ~120 AU（远超 deterrence_radius 8），舰队的威慑互不叠加。
    if let Some(s) = state.ship_mut(&ship3) {
        s.position = [40.0, 40.0];
        s.components = Vec::new();
    }
    if let Some(s) = state.ship_mut(&ship4) {
        s.position = [600.0, 600.0];
    }
    if let Some(s) = state.ship_mut(&ship5) {
        s.position = [60.0, 40.0];
        s.components = vec!["railgun".to_string()];
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    // 理智(tempter<0)：欺软怕硬 → 挑威慑低的 ship3。
    if let Some(s) = state.ship_mut(&ship0) {
        s.doctrine.temper = -1.0;
    }
    let rational = nearest_enemy_ship(&state, &config, "中国", [40.0, 40.0], 200.0, None, &ship0);
    assert_eq!(
        rational,
        Some(ship3),
        "理智 should pick the weaker 威慑 target, got {rational:?}"
    );
    // 热血(temper>0)：飞蛾扑火 → 挑威慑高的 ship5。
    if let Some(s) = state.ship_mut(&ship0) {
        s.doctrine.temper = 1.0;
    }
    let hot = nearest_enemy_ship(&state, &config, "中国", [40.0, 40.0], 200.0, None, &ship0);
    assert_eq!(
        hot,
        Some(ship5),
        "热血 should pick the stronger 威慑 target, got {hot:?}"
    );
}

/// 武器克制选目标（拟人「别浪费导弹打点防重镇」）：一舰有导弹时，应优先攻击**没有**
/// 点防御、导弹不会被拦截的目标，而不是把导弹打在被点防全面阻挡的目标上。
#[test]
fn target_selection_respects_weapon_advantage() {
    let (config, mut state) = fresh_world(42);
    // China (3) fields a missile-armed attacker (ship 0). Two hostile US (1)
    // targets sit in range: ship 3 has point-defense (intercepts missiles),
    // ship 5 has none — the missile attacker should prefer ship 5.
    let ship0 = state.ships[0].name.clone();
    let ship3 = state.ships[3].name.clone();
    let ship5 = state.ships[5].name.clone();
    if let Some(s) = state.ship_mut(&ship0) {
        s.position = [40.0, 40.0];
        s.components = vec!["missile".to_string()];
    }
    if let Some(s) = state.ship_mut(&ship3) {
        s.position = [40.2, 40.0];
        s.components = vec!["point_defense".to_string()];
    }
    if let Some(s) = state.ship_mut(&ship5) {
        s.position = [40.3, 40.0];
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);
    let target = nearest_enemy_ship(&state, &config, "中国", [40.0, 40.0], 0.4, None, &ship0);
    assert_eq!(
        target,
        Some(ship5),
        "a missile attacker should shun the point-defense ship (5), got {target:?}"
    );
}

/// 自保撤退（拟人「别送死」）：一舰离首都较远、已被打残、且敌在射程内时，应后撤
/// 修整充能（发出 Withdraw、朝首都移动、不送死），而不是死战到被击毁。
#[test]
fn damaged_far_ai_ship_withdraws_to_heal() {
    let (config, mut state) = fresh_world(42);
    let home = state.body_position("地球"); // 地球（中国首都）。
    let ship2 = state.ships[2].name.clone();
    let ship3 = state.ships[3].name.clone();
    // China (3) destroyer id 2: badly wounded (hull 5/24) and far from its capital.
    if let Some(s) = state.ship_mut(&ship2) {
        s.position = [40.0, 40.0];
        s.hull = 5.0;
        s.hull_max = 24.0;
    }
    // A hostile US (1) ship within the destroyer's attack range.
    if let Some(s) = state.ship_mut(&ship3) {
        s.position = [40.3, 40.0];
    }
    state
        .faction_mut("中国")
        .unwrap()
        .relations
        .insert("美国".to_string(), -35.0);
    state
        .faction_mut("美国")
        .unwrap()
        .relations
        .insert("中国".to_string(), -35.0);

    let d_before = dist([40.0, 40.0], home);
    let mut rng = Prng::new(42);
    let derived = advance(&mut state, &config, &mut rng);

    assert!(
        state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::Withdraw { ship: s, .. } if *s == ship2)),
        "a damaged far-from-home ship must withdraw, events={:?}",
        state.events
    );
    let s = state.ship(&ship2).expect("withdrawing ship must survive");
    let d_after = dist(s.position, home);
    assert!(
        d_after < d_before,
        "withdrawing ship should head for home ({d_before:.2} -> {d_after:.2})"
    );

    // …而且**判定本身**要被记下来（不发事件的那一半）：从「它撤了」到「为什么撤」，
    // 只有 `decisions` 能回答——当时的血量比与撤退阈值就是判据。
    let rec = derived
        .decisions
        .ships
        .iter()
        .find(|d| d.ship == ship2 && d.verdict == ShipVerdict::Withdraw)
        .unwrap_or_else(|| {
            panic!(
                "撤退判定必须被记下来，decisions={:?}",
                derived.decisions.ships
            )
        });
    assert_eq!(rec.target.as_deref(), Some("地球"), "撤退目标是首都天体");
    assert!(
        (rec.retreat_hull - effective_retreat_hull(&config, rec.kiting)).abs() < 1e-12,
        "记下的撤退阈值必须等于当时的有效阈值（风格变了它也变）：{rec:?}"
    );
    assert!(
        rec.hull_ratio < rec.retreat_hull && rec.hull_ratio > 0.0,
        "血量比必须真的低于阈值，否则这条判定不成立：{rec:?}"
    );
    assert!(rec.enemy_in_range, "撤退的前提是敌在射程内：{rec:?}");
    assert!(
        matches!(rec.order, Some(ShipBehavior::Move { .. })),
        "判定里要带上实际写回指令叶的行为：{rec:?}"
    );
}

/// 捕获的**基本不变量**（不是"有数据就行"）：
///
/// * 一艘 AI 舰一回合**最多两条**判定，且是「先机动 → 移动后再判一次」这个固定序列
///   （`move` 在前、`after_move` 只有第二条才为真）——没有这条，读表的人会把同一艘舰的
///   两行当成"它同时做了两件事"；
/// * 每条判定的舰属于它自报的势力；
/// * `retreat_hull` 与 `kiting` 自洽（读表的人正是靠这一列解释"它为什么撤/为什么打"，
///   错了就会得出反向的结论）。
#[test]
fn decisions_are_consistent_and_at_most_two_per_ship() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(7);
    let derived = advance(&mut state, &config, &mut rng);
    let ds = &derived.decisions.ships;
    assert!(!ds.is_empty(), "一回合至少要留下一批判定");

    let mut per_ship: std::collections::BTreeMap<String, Vec<&ShipDecision>> =
        std::collections::BTreeMap::new();
    for d in ds {
        assert!(d.ship != "", "判定必须指明是哪艘舰");
        if let Some(s) = state.ship(&d.ship) {
            assert_eq!(
                s.faction_id, d.faction,
                "判定的势力必须与舰的归属一致：{d:?}"
            );
        }
        assert!(
            (d.retreat_hull - effective_retreat_hull(&config, d.kiting)).abs() < 1e-12,
            "retreat_hull 与 kiting 不自洽：{d:?}"
        );
        assert!(
            (0.0..=1.0).contains(&d.hull_ratio),
            "血量比超出 0..1：{d:?}"
        );
        // `Hold` 的语义是「这回合 AI 没派活」——那它也就不该写叶。
        if d.verdict == ShipVerdict::Hold {
            assert!(d.order.is_none(), "没派活的判定不该带写回的行为：{d:?}");
        }
        per_ship.entry(d.ship.clone()).or_default().push(d);
    }
    for (ship, rows) in &per_ship {
        assert!(
            rows.len() <= 2,
            "{ship} 一回合出现了 {} 条判定（上限是 2）：{rows:?}",
            rows.len()
        );
        if rows.len() == 2 {
            assert_eq!(
                rows[0].verdict,
                ShipVerdict::Move,
                "{ship} 的第一条判定应当是机动：{rows:?}"
            );
            assert!(
                !rows[0].after_move,
                "{ship} 的第一条判定不该标成「移动之后」：{rows:?}"
            );
            assert!(
                rows[1].after_move,
                "{ship} 的第二条判定必须标成「移动之后」：{rows:?}"
            );
        } else {
            assert!(
                !rows[0].after_move,
                "{ship} 只有一条判定时不该标成「移动之后」：{rows:?}"
            );
        }
    }
}
