//! 思潮与忠诚：军事信号（只由事件推出）、战争/经济如何推思潮、外交亲和方向、低忠诚倒戈、娱乐设施拉住远城。
//!
//! ## 2026-10（第 7 批）：`military_signal_uses_the_milestones_and_is_branch_agnostic` 搬去了 g1
//!
//! 新挂 `--call military_deltas {events}`（纯函数，只吃事件表；事件形状与状态里 `events` 一致，
//! 内部标记 `type`）。原件的六个用例逐条复现：**互杀双方各得一分**（各 0）、单方面 +1/−1、
//! 欠费报废只扣失主、拆城 ±1 而**复垦者不计分**、**活城易主与叛乱兜底同分**、新建城不计分。
//!
//! `ideology_military_win_drives_toward_militarism` 也走了，但走的是**另一条路**：
//! 不去注入事件，而是 g2 **造一场真仗**（把最偏和平端那家的舰摆成「一发即沉」、两家关系压到
//! `-35`）⇒ 那一回合真的产生一次**我方击杀**。实测打仗臂 `和平↔军国` **−0.60 → −0.54**
//! （Δ=0.06），对照臂（只差关系、无击杀）**−0.60 → −0.57**（Δ=0.03）——正是
//! `0.05 × (0.5 − (−0.6))` 与 `0.05 × (0 − (−0.6))` 的预期倍差。
//!
//! ⚠ **逐回合法条没搬**：`和平↔军国` 列过 `r2`，而每回合位移 ≤ `drift_rate`(0.05)
//! ⇒ 法条判据会退化成「怎么都过」的假绿；**方向性窗口统计**也试过（20 回合窗口累计信号 vs 轴位移），
//! 一致率只有 **0.784**——因为信号为 0 的回合会把轴往 0 拉，淹没窗口里那点净信号。
//!
//! **留在这里的两条**：要**往世界里注入事件**再跑 `step_ideology`
//! （`--call` 是**纯函数**契约，加「可变调用」是设计改动）⇒ 归 §4 内部契约/手工世界。
//!
//! ## 2026-10（第 7 批）：`low_loyalty_city_defects_to_most_opposing_ideology_instead_of_razing`
//! 搬去了 g2 **合成场景 · 低忠诚改旗易帜**（4 条判据）
//!
//! 捏三样（都有身份键 ⇒ 直接改档）：旧主思潮推到极端、一个对照势力推到相反极、其余中立，
//! 再把一座城的忠诚压到阈值下。**「倒向谁」不写死**：拿 `--call ideology_similarity` 把
//! 「旧主 × 每个势力」的相似度都算一遍，判据要求倒戈目标就是**相似度最低**的那一个
//! （实测 `0.0` vs 其余 `0.5`）。另加「没被夷平 + 人口/建筑都在」。
//!
//! ⚠ 相似度必须从**场景自己的**投影读（补丁落地后那份）——第一版读了对照局，判据只是
//! **碰巧**还是那一家。
//!
//! ## 2026-10（第 7 批）：`entertainment_holds_a_distant_city` 搬去了 g2 **合成场景 · 重金娱乐拉住远城**（4 条判据）
//!
//! 两臂**只差有没有那份福利预算**（同一座城 = 该势力 `gov_distance` 最大的那座、同样起点
//! 忠诚度、同样满仓国库）：重金臂 `[0.35, 0.39, 0.43, 0.46, 0.49, 0.52]` 逐回合不降，
//! 对照臂 `[0.35, 0.33, 0.32, 0.31, …]` 真的往下走（防空转）。
//! 国库/忠诚度走 `h.scenario(patch=…)`，福利两片叶走 `--apply`——与原件那份 diff 同形。
//!
//! ## 2026-10（第 7 批）：`ideology_similarity_ranges_and_is_monotonic` 搬去了 g1
//!
//! 新挂了 `--call ideology_similarity`（键名与读面 `factions.思潮` 一致）⇒ 判据比原件**更强**：
//! 同 = 1、全对极 = 0、恒在 `[0,1]`，外加**对称**与沿轴**单调**（原件只比了「自己 ≥ 别人」）。

use super::*;

/// 思潮：经济转负把「人民↔精英」推向人民端；且所有轴恒可有界、有限。
#[test]
fn ideology_economy_bad_drives_toward_populism_and_stays_bounded() {
    let (config, mut state) = fresh_world(42);
    let fname = state.factions[1].name.clone();
    let start = state.faction(&fname).unwrap().ideology.people_elite;

    // 经济转负：净流 = 产出(0) − 维护(100) − 治理(0) < 0 → 人民（民粹反弹）。
    let mut flow = RoundSink::default();
    flow.upkeep.entry(fname.clone()).or_default().total = 100.0;
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
            assert!(
                v.is_finite() && (-1.0..=1.0).contains(&v),
                "{k} out of bounds: {v}"
            );
        }
    }
}
