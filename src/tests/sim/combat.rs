//! 战斗拟真：杀伤、护盾、规避、防空屏护，以及本土修船。
//!
//! ## 2026-10（第 7 批）：`combat_respects_shields_and_speed_evasion` 搬走
//!
//! * **回避**那半早在 g1 的 `--call hit_factor` 里压着（目标越快命中折减越低）。
//! * **护盾先吸、船体吃溢出**那半**长局读面证不了**：seed 42 / 400 回合的 **249 发**里
//!   `absorbed > 0` 的有 **0 发**（没人装护盾组件）⇒ g2 **造**一仗：攻方装 `railgun`、
//!   守方装 `shield`（护盾打满 12、船体 24）、两家关系压到 `-35`、摆在远离首都处
//!   （本土倍率 = 1）。实测那一发 `damage 12.6 / absorbed 8.82 / hull_pen 8.19`，
//!   守方护盾 12.00→4.73、船体 24.00→18.21 且**没被打死**。
//!
//! ## 2026-10（第 7 批）：`fire_degrades_components_under_damage` 搬去了 g2 `combat_report`
//!
//! 长局两半：**没挨打 ⇒ 组件耐久一点不掉**（3 seed 共 **22,644** 个「没挨打」的舰·回合零反例）
//! + **真打进船体 ⇒ 有组件掉了**（238 次真伤里 7 次观察到下降）。⚠ 正向只能写成**存在性**
//! （原件也是 `any(|(a,b)| a < b)`）：实测有 3 次真伤下组件耐久一位没动——`组件耐久` 过 `r2`，
//! 浅伤折到组件上的量小到看不见。
//!
//! ## 2026-10（第 7 批）：`damaged_components_repair_in_friendly_territory` 搬去了 g2
//!
//! 长局读面**证不了**这条：实测 seed 42 / 400 回合里「未挨打却修了」的组件·回合**一共只有 1 个**，
//! 而且它在**外地**——「本土修得更快」那半根本没有样本。所以 g2 用 `h.scenario(patch=…)` **造**一个
//! 受损组件（`edit()` 要「带身份键的行表」，**舰有身份键** ⇒ 这处能捏）：两臂只差坐标，
//! 本土 **+1.8/回合** vs 外海 **+0.72/回合**（距首都 0.5 / 38.4 AU，半径 6.0）。
//!
//! ⚠ 判据量的是「到**首都**的距离」（本土规则认的就是它），不是「到投放的那个天体」——第一版量错了。
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
