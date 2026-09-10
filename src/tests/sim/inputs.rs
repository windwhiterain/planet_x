//! B5：**输入面**（`pre`）的守卫 —— 掷出的随机数与判定输入必须在**回合中段**被接住。
//!
//! 用户裁决把这一面定成「凡是可能未来与随机/输入有关的东西都放 `pre`」（不要求当前的实现有关），
//! 并顺手砍掉了它原来装的那份**零信息量的观测副本**（同一份 state、同一个 `observe`、空 sink
//! 只把过程量抹成中性值 ⇒ 与上一回合的 `post` 逐字段相同）。
//!
//! 用例钉四件事：① 解算顺序是**不重不漏的名单**（覆盖所有幸存者，多出来的都是本回合战沉）；
//! ② 关系噪声落在配置的 `±noise` 里且**每对都有**；③ `pre` 里**没有**观测字段（B5 砍掉的那份
//! 重复）；④ 记录本身**不改世界**（同 seed 两次跑出同一份输入面）。

use super::*;
use crate::sim::advance_round;

/// **C7 · 解算顺序是一份不重不漏的名单**：每艘舰最多出现一次，且**回合末还活着的舰一个不少**。
///
/// ⚠ 顺序里是**洗牌那一刻**的 `state.ships`：这一回合稍后被打沉/除名的舰**在名单里、已不在
/// 世界**（死亡清扫在本步进收尾），这一回合稍后才下水的舰**在名单外**。所以判据是
/// 「无重复 + 覆盖所有幸存者 + 多出来的都是本回合的战沉」，**不是**「等于回合末的舰集」。
#[test]
fn the_resolution_order_is_a_permutation_of_every_ship() {
    let (config, mut state) = fresh_world(7);
    let mut rng = Prng::new(7);
    let mut inputs = RoundInputs::default();
    for _ in 0..5 {
        advance_round(&mut state, &config, &mut rng, &mut inputs);
        let round = state.round;
        assert!(!inputs.order.is_empty(), "回合 {round}：解算顺序是空的");

        // ① 无重复。
        let mut sorted: Vec<ShipId> = inputs.order.clone();
        sorted.sort();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "回合 {round}：解算顺序里有重复的舰");

        // ② 覆盖所有幸存者。
        let alive: std::collections::BTreeSet<ShipId> = state
            .ships
            .iter()
            .filter(|s| s.hull > 0.0)
            .map(|s| s.name.clone())
            .collect();
        let listed: std::collections::BTreeSet<ShipId> = inputs.order.iter().cloned().collect();
        for s in &alive {
            assert!(listed.contains(s), "回合 {round}：幸存者 {s} 不在解算顺序里");
        }

        // ③ 多出来的每一个都必须是**本回合战沉/报废**的舰（清扫把它们从世界里拿走了）。
        let dead: std::collections::BTreeSet<ShipId> = state
            .events
            .iter()
            .filter_map(|e| match e {
                GameEvent::ShipDestroyed { ship, .. } => Some(ship.clone()),
                _ => None,
            })
            .collect();
        for s in listed.difference(&alive) {
            assert!(
                dead.contains(s),
                "回合 {round}：{s} 不在世界末态里，也不是本回合沉掉的——顺序在编舰？"
            );
        }

        // ④ 顺序必须**真的被打乱过**（否则记的是「世界里的舰序」——那说明洗牌被绕过了）。
        if inputs.order.len() > 3 {
            let world_order: Vec<ShipId> = state.ships.iter().map(|s| s.name.clone()).collect();
            let head: Vec<ShipId> = world_order
                .iter()
                .filter(|n| inputs.order.contains(n))
                .cloned()
                .collect();
            assert_ne!(
                inputs.order, head,
                "回合 {round}：解算顺序与「世界里的舰序」逐字相同——洗牌没生效？"
            );
        }
    }
}

/// **C13 · 关系噪声**：每对势力都有、且落在配置的 `±noise` 区间里。
///
/// `noise = 0`（配置关掉了扰动）时这一节为空——那也是对的（「没掷」就是这个意思）。
#[test]
fn relation_noise_covers_every_pair_and_stays_in_range() {
    let (config, mut state) = fresh_world(7);
    let mut rng = Prng::new(7);
    let mut inputs = RoundInputs::default();
    let noise = config.diplomacy.noise;
    advance_round(&mut state, &config, &mut rng, &mut inputs);

    let fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    let expected_pairs = fids.len() * (fids.len() - 1) / 2;
    let n_fids = fids.len();
    let mut seen = 0usize;
    for (a, row) in &inputs.relation_noise {
        for (b, v) in row {
            seen += 1;
            assert!(fids.contains(a) && fids.contains(b), "噪声里出现了不存在的势力：{a}/{b}");
            assert_ne!(a, b, "自己跟自己的噪声？");
            assert!(
                v.abs() <= noise + 1e-9,
                "{a}→{b} 的噪声 {v} 超出配置区间 ±{noise}"
            );
        }
    }
    if noise > 0.0 {
        assert_eq!(
            seen, expected_pairs,
            "噪声只记了 {seen} 对，{n_fids} 个势力应有 {expected_pairs} 对"
        );
    } else {
        assert_eq!(seen, 0, "配置里 noise = 0 ⇒ 什么都没掷，这一节该是空的");
    }
}

/// **`pre` 不再装观测**（B5 砍掉的那一份重复）：拿同一回合的 `pre` 与**上一回合的 `post`** 比，
/// 观测那一半必须**不再**由 `pre` 提供——`pre` 里压根没有那些字段。
///
/// 这条挡的是「有人图省事又把 `view_from_state` 塞回 `pre`」：那会让 `pre` 重新变成上一回合
/// `post` 的副本（零信息量），而真正该在那里的是**掷了什么**。
#[test]
fn pre_is_the_input_face_not_an_observation_copy() {
    let (config, mut state) = fresh_world(7);
    let mut rng = Prng::new(7);
    let mut inputs = RoundInputs::default();
    advance_round(&mut state, &config, &mut rng, &mut inputs);

    let v = serde_json::to_value(&inputs).unwrap();
    let obj = v.as_object().expect("RoundInputs 是个对象");
    let keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
    for banned in ["factions", "cities", "wars", "power_share", "decisions", "fleet_value"] {
        assert!(
            !obj.contains_key(banned),
            "输入面里出现了观测字段 `{banned}`——它属于 `post`（`{keys:?}`）"
        );
    }
    assert!(
        obj.contains_key("order") && obj.contains_key("relation_noise"),
        "输入面必须有 `order` / `relation_noise`（B5a 接上的两处主 Prng）：{keys:?}"
    );
}

/// **同一 seed 两次跑出同一份输入面**（记录不改世界，也不吃第二遍骰子）。
///
/// 主 `Prng` 的两处（洗牌、噪声）都是**读一次用一次**：记录它们**不额外消费**随机流，
/// 所以同 seed 的输入面必须逐字节一致——这条与 digest 逐字不变是同一件事的两面。
#[test]
fn the_input_face_reproduces_byte_for_byte() {
    let run = |seed: u64| {
        let (config, mut state) = fresh_world(seed);
        let mut rng = Prng::new(seed);
        let mut inputs = RoundInputs::default();
        for _ in 0..8 {
            advance_round(&mut state, &config, &mut rng, &mut inputs);
        }
        serde_json::to_value(&inputs).unwrap()
    };
    assert_eq!(run(7), run(7), "同 seed 的输入面不一致 ⇒ 记录改变了随机流");
}

/// **`rolls` 的形状与内容**（B5b）：每条要么是**闸门**（`threshold` + 走的那一支），要么是
/// **加权抽签**（`pool_total` + 选中谁）；`value ∈ [0,1)`；势力与对象都不空。
///
/// 防空转：真世界里这几个用途**必须真的出现过**——某一族没接上时这里会红，
/// 而不是安静地留一张空表（本仓库对「空表骗过守卫」有前科）。
#[test]
fn roll_records_are_well_formed_and_actually_happen() {
    // ⚠ `fresh_world` 把角色轴**钉成「全员战舰」**（那条默认让定编那两族骰子根本不掷），
    // 所以要测它们就得用**没钉**的世界——否则 `role` / `observe_role` 永不出现，
    // 「防空转」这条就变成了在断言一个假象。
    let config = load_config();
    let mut state = default_state(&config, 7);
    let mut rng = Prng::new(7);
    let mut inputs = RoundInputs::default();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for _ in 0..30 {
        advance_round(&mut state, &config, &mut rng, &mut inputs);
        for r in &inputs.rolls {
            assert!(
                (0.0..1.0).contains(&r.value),
                "掷出的值必须在 [0,1) 里：{:?}",
                r
            );
            assert!(
                !r.faction.is_empty() && !r.subject.is_empty(),
                "抽签记录要能指回「谁、对什么」：{r:?}"
            );
            match (r.threshold, r.pool_total) {
                (Some(p), None) => {
                    assert!(
                        (0.0..=1.0).contains(&p),
                        "闸门的机会值必须是个概率（{p}）：{r:?}"
                    );
                    assert!(r.picked.is_some(), "闸门要记下走了哪一支：{r:?}");
                }
                (None, Some(total)) => {
                    assert!(total > 0.0, "抽签池的总权重必须为正：{r:?}");
                    assert!(r.picked.is_some(), "抽签要记下选中谁：{r:?}");
                }
                other => panic!("既不闸门也不抽签的形状：{other:?}（{r:?}）"),
            }
            *seen.entry(r.purpose.clone()).or_default() += 1;
        }
    }
    for want in ["gate", "accept", "assign", "role", "review", "renew"] {
        assert!(
            seen.contains_key(want),
            "30 回合里一次 `{want}` 抽签都没记到（已记到：{seen:?}）——那一族没接上？"
        );
    }
}
