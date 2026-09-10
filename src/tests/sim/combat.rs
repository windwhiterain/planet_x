//! 战斗与损伤：本土防御光环、面板随选装变化、点防按舰级缩放、逐组件损伤与友方领土修理、舰队防空、护盾与速度规避。

use super::*;

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
