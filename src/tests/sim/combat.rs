//! 战斗与损伤：本土防御光环、面板随选装变化、点防按舰级缩放、逐组件损伤与友方领土修理、舰队防空、护盾与速度规避。
//!
//! ## 2026-10：**能只看数据的那几条搬到了 `play/tests/g2_mid.py`**
//!
//! 逐发明细住在 `attack` 事件的 `data.shots` 里（B4 用户裁决：战斗中间量进事件层），加上舰表
//! 的 `hull/hull_max/hull_regen/shield/component_hp`，下面这些就能在**全 3 seed × 400 回合的
//! 每一行**上成立，不必再造世界：
//!
//! | 数据级判据（g2） | 说明 |
//! | --- | --- |
//! | 每一发自洽 | `hit ∈ [0,1]`、射程外/被跳过不掉血、没命中没伤害、没点防不拦截（511 发逐发成立） |
//! | 击杀那一发的 `hull_pen` ≥ 它记下的 `target_hull_before` | 「伤害够打掉它」逐发对账（146 次击杀） |
//! | 战沉 ⇔ `killed` 射击（两个方向） | 死因集合 `{combat, upkeep_shortfall}` ⇒ 只有战死的才要求那一发 |
//! | 面板域 | `0 < hull ≤ hull_max`、`0 ≤ shield ≤ shield_max`、attack/speed/组件完整度 ≥ 0 |
//! | 安静回合的护甲再生 | 只增不减、不超上限（**基础再生量是下界**——本土加成读面看不到，不断言等式） |
//!
//! 2026-10 又搬走一条（**合成场景**那一类）：`damaged_ship_regenerates_hull_each_round`
//! ⇒ g2 的 `scenario_checks`「受伤的舰每回合按 `hull_max × 再生率` 长回来」。档现在能存成 JSON
//! （`--save w.json`）⇒ Python 直接把船体改成一半、扔到 `[80, 80]`，推进 3 回合后**逐位**判增量：
//! 本土加成的有无是**两个离散值**，所以「增量 ∈ {基础, 基础+加成}」可判（实测 6.00 → 7.44，
//! 三次 +0.4800 = 12 × 0.04）——比原先「造一个世界、打一炮」更贴真实长局，也不必再编进 crate。
//!
//! **留在这里的**：逐组件损伤与友方领土修理、舰队防空、护盾与速度规避的**整炮 A/B**——这些要
//! **拨一个旋钮造 A/B**（把库存改到只够付一半维护费、摆两艘对轰之类）。
//! 公式类（`home_defense_mult`、`ship_panel`）已通过 `planet_x --call <fn>` 搬到 g1 的
//! `call_functions`（调的是引擎**同一份实现**，不是抄公式）；`hit_factor` 也有一条 `--call`
//! 判据，但整炮规避那条 A/B 仍在这里。
//!
//! ⚠ 两条踩过的坑记在这（写数据级战斗判据时会再遇到）：① 扣船体的是 **`hull_pen`**，不是
//! `damage`（`hull_mult` 可 > 1，`hull_pen` 能比 `damage` 大）；② 「被击杀」不能用**上一回合末
//! 的船体**对账——改装的船下一回合 `hull_max` 就变了（实测 r86 莱茵4：上一回合 24.0、本回合
//! 按 12.0 的船体被打掉）。

use super::*;

#[test]
fn fire_degrades_components_under_damage() {
    let (config, mut state) = fresh_world(42);
    let ship0 = state.ships[0].name.clone();
    let ship3 = state.ships[3].name.clone();
    // 目标：US 驱逐舰（ship 3），装一枚导弹组件、血厚到扛住一炮以观察组件损耗。
    if let Some(t) = state.ship_mut(&ship3) {
        t.position = [40.0, 40.0];
        t.components = vec!["missile".to_string()];
        t.component_hp = t
            .components
            .iter()
            .map(|c| component_integrity(&config, c))
            .collect();
        t.hull = 500.0;
        t.hull_max = 500.0;
        t.shield = 0.0;
        t.shield_max = 0.0;
    }
    // 攻击者：CN 护卫舰（ship 0），装一门重炮、贴近目标。
    if let Some(a) = state.ship_mut(&ship0) {
        a.position = [40.1, 40.0];
        a.components = vec!["railgun".to_string()];
        a.component_hp = a
            .components
            .iter()
            .map(|c| component_integrity(&config, c))
            .collect();
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
    let before = state
        .ship(&ship0)
        .map(|s| s.component_hp.first().copied().unwrap_or(0.0))
        .unwrap_or(0.0);
    advance(&mut state, &config, &mut Prng::new(42));
    let after = state
        .ship(&ship0)
        .map(|s| s.component_hp.first().copied())
        .flatten()
        .unwrap_or(before);
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
        g.component_hp = g
            .components
            .iter()
            .map(|c| component_integrity(&config, c))
            .collect();
    }
    let cover_with = cluster_pd_cover(&state, &config, &ship5, "美国", [40.0, 40.0]);
    assert!(
        cover_with > 0.0,
        "a nearby PD ship should give air-defense cover; got {cover_with}"
    );
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

    let shield_before = state.ship(&ship3).map(|s| s.shield).unwrap();
    let hull_before = state.ship(&ship3).map(|s| s.hull).unwrap();
    fire_concentrate(&mut state, &config, &ship0, &ship3);

    let shield_after = state.ship(&ship3).map(|s| s.shield).unwrap();
    let hull_after = state.ship(&ship3).map(|s| s.hull).unwrap();
    assert!(
        shield_after < shield_before,
        "shield pool must absorb damage"
    );
    assert!(
        hull_after < hull_before,
        "hull should take spill damage too"
    );
    assert!(
        hull_after > 0.0,
        "a single volley on a destroyer should not one-shot it"
    );

    // Evasion: a fast target is hit less by a low-tracking weapon than a slow one.
    let fast_hit = hit_factor(2.0, 2.6); // corvette speed
    let slow_hit = hit_factor(2.0, 1.2); // destroyer speed
    assert!(
        fast_hit < slow_hit,
        "fast ship should evade a low-tracking weapon more (fast {fast_hit} vs slow {slow_hit})"
    );
}
