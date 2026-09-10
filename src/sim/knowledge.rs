//! **MOND 知识**：掌握度（科技体系的干线）随「飞船在异常区的在场强度」涨落。
//!
//! 用户裁决（`.agents/notes/tech-system.md` §8）：第一版**只做这一条渠道**——飞船在异常区
//! 观测。不开采、不建研究建筑、不做自然扩散，所以：
//!
//! * **不去就学不会**：没有舰在带内 ⇒ 在场强度 0 ⇒ 目标 0 ⇒ 掌握度慢慢锈回凡人；
//! * **深处更值钱**：一艘舰的价值 = `1 + 深度 × depth_weight`，外缘永远有理由派人去；
//! * **指数饱和**：`目标 = 1 − e^(−在场强度 / presence_ref)`——投入翻倍不等于进度翻倍，
//!   越接近满越难（这就是「领跑者刹车」的一半，另一半是思潮优势端的忠诚 debuff）。
//!
//! 与 `step_ideology` 完全同形：先算**本回合的目标值**（只读 state），再让可变状态向它靠拢。
//! 顺序上两者互不读对方，但都读「回合末的舰位」，所以放在一起。

use super::*;

/// 势力此刻的**在场强度**：自己的活舰中，位于异常区（日心距 > `mond.radius`）的那些，
/// 每艘按 `1 + 深度(AU) × depth_weight` 计。
///
/// 这是「飞船触发 MOND 现象」的**唯一**尺子：观察面、探针与守卫都读它，别再各算一份。
pub fn mond_presence(state: &State, config: &GameConfig, fid: &str) -> f64 {
    let r = config.mond.radius;
    let w = config.mond.knowledge.depth_weight;
    state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .map(|s| dist(s.position, [0.0, 0.0]) - r)
        .filter(|depth| *depth > 0.0)
        .map(|depth| 1.0 + depth * w)
        .sum()
}

/// 在场强度对应的**目标掌握度**（0..1，指数饱和、无断崖）。
pub fn mond_target(config: &GameConfig, presence: f64) -> f64 {
    let k = &config.mond.knowledge;
    if k.presence_ref <= 0.0 {
        return if presence > 0.0 { 1.0 } else { 0.0 };
    }
    1.0 - (-(presence.max(0.0)) / k.presence_ref).exp()
}

/// 逐回合推进 MOND 掌握度：**朝本回合的在场目标靠拢**（确定性、无 RNG、无主 `Prng` 消费）。
///
/// 掌握度是**活知识**：它是「人还在地里」这件事的函数，不是一个一次性的解锁。
/// 崇拜教开局 1.0（`config.mond.initial`），只要它的舰还在异常区就维持在接近 1；
/// 一旦把舰队撤回来，它也会慢慢锈掉——这正是让「谁掌握 MOND」成为**可争夺**的东西。
pub fn step_knowledge(state: &mut State, config: &GameConfig) {
    let rate = config.mond.knowledge.drift_rate.clamp(0.0, 1.0);
    // Pass 1（只读 state）：算每势力的在场强度与目标值。
    let mut targets: BTreeMap<FactionId, f64> = BTreeMap::new();
    for f in &state.factions {
        let presence = mond_presence(state, config, &f.name);
        targets.insert(f.name.clone(), mond_target(config, presence));
    }
    // Pass 2（可变 state）：向目标靠拢并钳到 [0,1]。
    for f in &mut state.factions {
        let Some(target) = targets.get(&f.name) else { continue };
        f.mond_control = (f.mond_control + rate * (target - f.mond_control)).clamp(0.0, 1.0);
    }
}
