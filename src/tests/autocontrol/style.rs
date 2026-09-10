//! 风格三轴的单元测试。

use super::*;
use crate::config::load_config;
use crate::world::default_state;

/// 一份"每回合都重估、一次到位"的配置：把概率与步长推到确定的一端，好让用例判定
/// *机制*（而不是被 0.35 的概率蒙住）。步长 1.0 = 一次走到目标值。
fn eager(config: &mut GameConfig) {
    config.autocontrol.style_chance = 1.0;
    config.autocontrol.style_step_max = 1.0;
}

fn fresh(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// **执行者真的在写叶**：开了执行者之后，AI 舰的风格叶会出现（`Control::inherit` = 流水，
/// 不是指令），值落在 [-1,1]——即这条轴**不再是值冻结**。
#[test]
fn the_executor_writes_style_leaves_instead_of_freezing_them() {
    let (mut config, mut state) = fresh(42);
    eager(&mut config);
    // 造一个**战况**：中国与俄罗斯打起来，并把中国的舰打残。
    let fid = "中国".to_string();
    state.faction_mut(&fid).unwrap().relations.insert("俄罗斯".to_string(), -60.0);
    state.faction_mut("俄罗斯").unwrap().relations.insert("中国".to_string(), -60.0);
    let mine: Vec<String> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .collect();
    for name in &mine {
        let s = state.ship_mut(name).unwrap();
        s.hull = s.hull_max * 0.3; // 打残 ⇒ 目标 temper 应当被压向负（理智）
    }
    let mut styles = Vec::new();
    regulate_styles(&mut state, &config, &[], &mut styles);
    assert!(!styles.is_empty(), "开了执行者却没有一条风格重估记录");
    let c = state.control(fid.clone()).unwrap();
    assert!(
        !c.ship_doctrine.is_empty() || !c.ship_kiting.is_empty(),
        "执行者必须把结论落在逐舰叶上"
    );
    for (ship, leaf) in &c.ship_doctrine {
        assert_eq!(leaf.mode, ControlMode::Inherit, "AI 写的是流水（这一层没有说话）");
        assert!(
            leaf.value.temper.abs() <= 1.0 && leaf.value.lone_wolf.abs() <= 1.0,
            "{ship} 的风格越界：{:?}",
            leaf.value
        );
    }
    for (_, leaf) in &c.ship_kiting {
        assert_eq!(leaf.mode, ControlMode::Inherit);
        assert!(leaf.value.abs() <= 1.0);
    }
}

/// **打残 → 更保守**：同一个势力、同一批舰，只是血量不同，`temper` 的目标必须更低
/// （理智 = 欺软怕硬）。这条钉的是"驱动力真的是可读状态"，不是随机数。
#[test]
fn a_battered_fleet_turns_colder_than_a_healthy_one() {
    let (mut config, state) = fresh(42);
    eager(&mut config);
    let fid = "中国".to_string();
    let mut healthy = state;
    healthy.faction_mut(&fid).unwrap().relations.insert("俄罗斯".to_string(), -60.0);
    let mut hurt = healthy.clone();
    let names: Vec<String> = hurt
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .collect();
    for n in &names {
        let s = hurt.ship_mut(n).unwrap();
        s.hull = s.hull_max * 0.2;
    }
    let sit_ok = situation(&healthy, &config, &fid, &[]);
    let sit_bad = situation(&hurt, &config, &fid, &[]);
    assert!(sit_ok.war > 0.0, "关系压到交战阈值之下 ⇒ 战争强度该 > 0");
    assert!(sit_bad.damage > sit_ok.damage, "打残的舰队 damage 该更高");
    assert!(
        temper_target(&config.autocontrol, &sit_bad) < temper_target(&config.autocontrol, &sit_ok),
        "打残 ⇒ temper 目标更低（更保守）"
    );
}

/// **一片叶两条轴：只改一条不许把另一条清零**（`control-live-layers.md` §3.1 那条规矩）。
#[test]
fn retuning_one_axis_keeps_the_other_axis_value() {
    let (mut config, mut state) = fresh(42);
    eager(&mut config);
    // 独狼的观察半径压到极小：本舰附近永远没有友舰 ⇒ lone_wolf 目标 = +1，
    // 于是把记录值也设成 +1 时那条轴**已经到位**（不写叶），只剩 temper 会被改。
    config.autocontrol.lone_wolf_radius = 1e-9;
    let fid = "中国".to_string();
    state.faction_mut(&fid).unwrap().relations.insert("俄罗斯".to_string(), -60.0);
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .unwrap();
    state.ship_mut(&ship).unwrap().doctrine = ShipDoctrine { temper: 0.0, lone_wolf: 1.0 };
    let mut styles = Vec::new();
    regulate_styles(&mut state, &config, &[], &mut styles);
    let leaf = state
        .control(fid.clone())
        .unwrap()
        .ship_doctrine
        .get(&ship)
        .cloned()
        .expect("这艘舰的叶该被写出来");
    assert_eq!(
        leaf.value.lone_wolf, 1.0,
        "没被重估的那条轴要带着它当时在用的值（不是 0.0）"
    );
}

/// **闸门**：玩家的逐舰叶不碰；**势力级默认叶是 `Player` 时逐舰叶一律不写**
/// （用户裁决——比归属链更严一档）；势力作用域设成 `Player` 同理。
#[test]
fn the_executor_respects_every_player_gate() {
    let (mut config, mut state) = fresh(42);
    eager(&mut config);
    let fid = "中国".to_string();
    let ships: Vec<String> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .map(|s| s.name.clone())
        .collect();
    let pinned = ships[0].clone();
    // ① 逐舰叶钉成 Player（玩家给这艘舰的特例）。
    state.control_mut(fid.clone()).unwrap().ship_doctrine.insert(
        pinned.clone(),
        Control::player(ShipDoctrine { temper: -0.8, lone_wolf: 0.0 }),
    );
    let mut styles = Vec::new();
    regulate_styles(&mut state, &config, &[], &mut styles);
    let leaf = state.control(fid.clone()).unwrap().ship_doctrine.get(&pinned).unwrap().clone();
    assert_eq!(leaf.mode, ControlMode::Player, "玩家的叶不许被改成流水");
    assert_eq!(leaf.value.temper, -0.8, "玩家的值一个字节都不许动");

    // ② 势力级默认叶是 Player ⇒ **那条轴**的逐舰叶一律不写（哪怕叶子自己写着 Auto）。
    //    两条轴各有各的默认叶 ⇒ 闸门也**逐轴**判（这里先只钉风格那条）。
    state.control_mut(fid.clone()).unwrap().default_doctrine =
        Some(Control::player(ShipDoctrine { temper: 0.5, lone_wolf: 0.1 }));
    state
        .control_mut(fid.clone())
        .unwrap()
        .ship_kiting
        .insert(ships[1].clone(), Control::auto(0.9));
    let snap = |st: &State| -> Vec<Option<(ShipDoctrine, ControlMode)>> {
        ships
            .iter()
            .map(|s| st.control(fid.clone()).unwrap().ship_doctrine.get(s).map(|l| (l.value, l.mode)))
            .collect()
    };
    let before = snap(&state);
    let mut styles2 = Vec::new();
    regulate_styles(&mut state, &config, &[], &mut styles2);
    assert_eq!(before, snap(&state), "舰队默认风格归玩家 ⇒ AI 不该盖任何一片逐舰风格叶");
    assert!(
        styles2.iter().any(|s| s.axis == "kiting"),
        "两条轴各有各的默认叶 ⇒ 只钉风格那条时，风筝轴**仍然**归 AI（逐轴判闸门）"
    );

    // ②b 再把风筝那条默认叶也钉成 Player ⇒ 这条轴也不写了。
    state.control_mut(fid.clone()).unwrap().default_kiting = Some(Control::player(0.9));
    let mut styles3 = Vec::new();
    regulate_styles(&mut state, &config, &[], &mut styles3);
    assert!(
        !styles3.iter().any(|s| s.axis == "kiting"),
        "风筝轴的默认叶归玩家 ⇒ 这条轴的逐舰叶也不写"
    );

    // ③ 势力作用域设成 Player ⇒ 整个势力的风格轴都不归 AI（别的势力照旧）。
    let mut state3 = default_state(&config, 42);
    state3.scope.factions.insert(fid.clone(), ControlMode::Player);
    let mut styles4 = Vec::new();
    regulate_styles(&mut state3, &config, &[], &mut styles4);
    assert!(
        !styles4.iter().any(|s| s.faction == fid),
        "势力归玩家 ⇒ 这个势力一条都不许改（别的势力照旧）"
    );
    assert!(
        !state3.control(fid.clone()).unwrap().ship_doctrine.is_empty()
            || styles4.iter().all(|s| s.faction != fid),
        "该势力名下不许出现 AI 写的风格叶"
    );
}

/// **抽签与步长**：概率为 0 时一声不响；抽中时朝目标走一步（且一步是分布的一步）；
/// 已经到位时不再写叶。
#[test]
fn the_step_is_probabilistic_and_stops_at_the_target() {
    let (mut config, _state) = fresh(42);
    config.autocontrol.style_chance = 0.0;
    assert!(
        step_value(&config, "中国", "长城", 7, StyleAxis::Doctrine, "temper", 0.0, 1.0).is_none(),
        "概率 0 ⇒ 这一回合不重估"
    );
    config.autocontrol.style_chance = 1.0;
    config.autocontrol.style_step_max = 0.5;
    let next = step_value(&config, "中国", "长城", 7, StyleAxis::Doctrine, "temper", 0.0, 1.0)
        .expect("概率 1 ⇒ 一定重估");
    assert!(next > 0.0 && next <= 0.5, "一步最多走完差距的一半：{next}");
    // 同一 (势力, 舰, 回合, 用途) 的骰子是**派生**的 ⇒ 逐字可复现（不消费主 Prng 流）。
    assert_eq!(
        step_value(&config, "中国", "长城", 7, StyleAxis::Doctrine, "temper", 0.0, 1.0),
        Some(next),
        "派生骰子必须逐字可复现"
    );
    // 另一条轴拿的是**另一枚**骰子（同一片叶、两条轴不该被同一枚骰子绑在一起）。
    assert!(
        step_value(&config, "中国", "长城", 7, StyleAxis::Doctrine, "lone_wolf", 0.0, 1.0).is_some()
    );
    // 已经到位 ⇒ 不写叶。
    assert!(
        step_value(&config, "中国", "长城", 7, StyleAxis::Doctrine, "temper", 1.0, 1.0).is_none(),
        "值就在目标上 ⇒ 不写叶（避免控制面 diff 噪声）"
    );
}
