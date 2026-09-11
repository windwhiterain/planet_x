//! MOND 掌握度：**在场强度→掌握度**那条唯一的渠道。
//!
//! ## 2026-10（第 7 批）：三条搬走了，投影侧补了两列
//!
//! `factions` 新增 **`mond_presence`**（在场强度 `Σ(1 + 深度 × depth_weight)`，只数带外的活舰）
//! 与 **`mond_target`**（它的指数饱和目标）；另挂 `--call mond_presence` / `--call mond_target`。
//!
//! | 原用例 | 现在住 |
//! | --- | --- |
//! | `presence_comes_only_from_ships_in_the_band` | g2 **驻泊深度**合成场景：带内 ⇒ 强度 0；强度 == 舰数 × `(1 + 深度 × 权重)`（3 艘 × 1.5 = **4.5**）；同一批舰停深 40 AU ⇒ **33.04** |
//! | `control_climbs_toward_the_presence_target_and_stops_there` | g2 浅驻泊臂：目标严格在 `(0,1)`（**0.89**）、掌握度朝它爬、不超过它、差距从 0.890 缩到 0.350 |
//! | `a_real_deep_presence_reaches_the_top` | g2 深驻泊臂：目标正好 1.0，掌握度**第 48 回合**到顶（= `mastery_rounds`）；`--call mond_target` 在门槛两侧 `0.9974` / `1.0` |
//! | （另加，g3 长局） | 「在场强度 ⇔ 带内舰数」同生同灭（63,000 个势力·回合）+「上涨必须由**强度**解释」（1,323 次） |
//!
//! ⚠ **逐回合定律（`m' = m + rate×(target−m)`）没有搬**：两列都是**回合末**的值，而
//! `step_knowledge` 用的是**走那一刻**的在场强度 ⇒ 目标在动时对不上（实测最大偏差 0.0267，
//! 即便只看「目标稳定」的回合也还有 77 处、最大 0.0208——船是回合中途进出的）。收敛那半
//! 因此在 g2 划成「朝目标爬 / 不超目标 / 差距缩小」，而不是照抄法条。
//!
//! **留在这里的**：`control_rusts_back_when_the_fleet_leaves` —— 读面给的是**带内舰数**，
//! 目标是**在场强度**（逐舰深度不同），实测 1,946 次回落里 9 次带内有舰。
//!
//! ## 2026-10（第 7 批）：两条搬去了数据级
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `initial_mastery_comes_from_config_and_frontier_reads_it` | g1「MOND 开局打点从配置读」+「前沿海拔 = `--call mond_frontier`」 | 开局打点是 `meta.mond.initial` 的**配置事实**；前沿海拔是纯函数，挂了 `--call` 后逐值扫描（0→30 / 0.5→32 / 1.0→∞） |
//! | `mastery_at_one_is_a_ratchet_and_never_rusts` | g3「**1.0 是棘轮**——到过顶就永不回落」 | 7 seed × 1000 回合里 **7652 个「势力·回合」**在顶上，一个都没掉下来；顺带 `[0,1]` 与「涨 ⇒ 带内有舰」 |
//!
//! **留在这里的**：`a_real_deep_presence_reaches_the_top`（要造一个够深的在场）、
//! `control_climbs_toward_the_presence_target_and_stops_there` /
//! `control_rusts_back_when_the_fleet_leaves`（读面给的是**带内舰数**，目标是**在场强度**
//! `1 + 深度 × depth_weight`，逐舰不同——实测 123 次回落里 9 次带内有舰）。
//! 撤回来它就锈。判据与公式见 `sim::knowledge` 与 `.agents/notes/tech-system.md`。

use super::*;

/// 把某势力所有活舰搬到日心距 `r` 处（正 x 轴）——「在场/不在场」的最短写法。
fn park(state: &mut State, fid: &str, r: f64) {
    let names: Vec<String> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| s.name.clone())
        .collect();
    for n in names {
        if let Some(s) = state.ship_mut(&n) {
            s.position = [r, 0.0];
        }
    }
}

/// 锈：把舰队撤出异常区之后，掌握度按同一个速率回落——**知识是活量，不是一次性解锁**。
#[test]
fn control_rusts_back_when_the_fleet_leaves() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    park(&mut state, fid, config.mond.radius + 2.0);
    for _ in 0..400 {
        step_knowledge(&mut state, &config);
    }
    let learned = state.faction(fid).unwrap().mond_control;
    assert!(
        learned > 0.5,
        "先得学到东西，用例才有意义（实得 {learned}）"
    );

    park(&mut state, fid, 1.0); // 撤回来
    for _ in 0..100 {
        step_knowledge(&mut state, &config);
    }
    let rusted = state.faction(fid).unwrap().mond_control;
    assert!(rusted < learned, "不在场就必须锈（{learned} → {rusted}）");
    for _ in 0..2000 {
        step_knowledge(&mut state, &config);
    }
    assert!(
        state.faction(fid).unwrap().mond_control < 0.01,
        "长期不在场 ⇒ 回到凡人（实测 {}）",
        state.faction(fid).unwrap().mond_control
    );
}
