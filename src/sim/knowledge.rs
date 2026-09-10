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

/// 在场强度对应的**目标掌握度**（0..1，指数饱和、无断崖；**够得着**的顶见
/// [`KnowledgeConfig::mastery_presence`]）。
pub fn mond_target(config: &GameConfig, presence: f64) -> f64 {
    let k = &config.mond.knowledge;
    if k.mastery_presence > 0.0 && presence >= k.mastery_presence {
        return 1.0;
    }
    if k.presence_ref <= 0.0 {
        return if presence > 0.0 { 1.0 } else { 0.0 };
    }
    1.0 - (-(presence.max(0.0)) / k.presence_ref).exp()
}

/// 逐回合推进 MOND 掌握度：**朝本回合的在场目标靠拢**（确定性、无 RNG、无主 `Prng` 消费）。
///
/// 掌握度是**活知识**：它是「人还在地里」这件事的函数，不是一个一次性的解锁。
/// 但只要**学到顶（1.0）就是永久的**——用户裁决：「一旦达到 1.0 就不会下降」。
/// 于是这条轴是一个**棘轮**：1.0 之下会锈，1.0 之上不回退 ⇒ MOND 是**先到先得、
/// 拿到就永久独占**的东西（这也是为什么「把它的好处做大」值得谨慎：好处越大，
/// 这个永久垄断越重）。
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
        let Some(target) = targets.get(&f.name) else {
            continue;
        };
        // **棘轮**：到顶就永久持有，不再参与松弛（用户裁决）。
        if f.mond_control >= 1.0 {
            continue;
        }
        // **学满 = 持续够格**：在场强度 ≥ `mastery_presence` 的回合按固定步长往上爬
        // （`mastery_rounds` 个够格的回合正好爬满 ⇒ 1.0 ⇒ 棘轮锁住）。
        // 第一版是「够格就**直接**到顶」，实测立刻出事：seed 7 里俄罗斯与深空运输联盟各自
        // 只在 6–7% 的回合有舰在带内，却都靠**某一回合恰好**凑够而永久当上 master
        // （r240 双双 1.0）。一回合的巧合不该换来永久垄断。
        if *target >= 1.0 {
            let step = 1.0 / (config.mond.knowledge.mastery_rounds.max(1) as f64);
            f.mond_control = (f.mond_control + step).min(1.0);
            continue;
        }
        f.mond_control = (f.mond_control + rate * (target - f.mond_control)).clamp(0.0, 1.0);
    }
}
