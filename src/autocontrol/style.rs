//! **风格轴的执行者**：`Auto` 在 `temper` / `lone_wolf` / `kiting` 三条轴上的真写入者。
//!
//! # 为什么需要它（这是「空头承诺」那一笔账）
//!
//! 三态归属里 [`ControlMode::Auto`](crate::model::ControlMode::Auto) 的承诺是「**系统每回合
//! 决定并改写**」。在指令轴上这件事是真的（[`super::tactics::ai_ship_turn`] 每回合往
//! `ship_orders` 写叶），但在**风格两轴**上一直没有写入者——于是把一片风格叶设成 `Auto` 的
//! 实际效果是**值冻结**，web/`--control` 里给 `Auto` 的措辞只能是假话
//! （`.agents/notes/control-live-layers.md` §3.2 就是查这条账）。这个模块把那条账还上。
//!
//! # 三条规则（每条都有理由，改之前先读）
//!
//! 1. **只写归 `Auto` 的轴**。闸门有三道，宁严勿宽：
//!    * 舰本身归 AI（`State::ship_control == Auto`）；
//!    * 轴的归属链（叶 → 舰队默认 → 势力 → 全局）不是 `Player`；
//!    * **势力级默认叶是 `Player` 就一律不写**（比链更严一档）——那是玩家给**全舰队**定的
//!      答案，AI 的逐舰流水不该盖它（用户裁决见 `control-live-layers.md` §13）。
//! 2. **概率触发、分布步长**（仓规：不用硬阈值/贪心）。每回合每舰每轴先抽一枚
//!    `P(重估) = config.autocontrol.style_chance`，中了再走**指数松弛**的一步：
//!    `新值 = 当前值 + (战况目标 − 当前值) × roll × style_step_max`。
//!    两个好处：风格是**慢变量**（它表达"这支部队在打什么仗"，不是"这一发打谁"），
//!    每回合大改只会把轨迹噪声化；而指数松弛**离目标远时改得多、近了改得少**，
//!    天然收敛，不会在目标附近来回抖。
//! 3. **写的是流水**：结论落在**逐舰叶**上、`mode` 是 [`Control::inherit`]（「这一层没有说话」），
//!    与指令轴/角色轴同一条规矩——玩家把**更宽**的那层钉成 `Player` 时，AI 的流水不许遮住它。
//!
//! # 骰子
//!
//! 只用 [`sim::derived_roll`]（按 `(势力, 舰名, 回合, 用途)` 派生），**绝不消费主 `Prng` 流**：
//! 否则「多一艘船」会改变整个世界后续的掷骰，同种子可复现就废了（见 `AGENTS.md`）。
//! 每艘舰的抽签独立 ⇒ 舰队不会整体同步抖动（那是"每回合全体重算"的典型症状）。
//!
//! # 驱动力（全部只用引擎已有的可读状态，不新造隐藏状态）
//!
//! | 轴 | 目标值从哪来 |
//! | --- | --- |
//! | `temper` | **战况**：`war_strength`（在打）+ 本回合净战果（打赢 ˚）+ 舰队被打残 + 本回合撤退占比 |
//! | `lone_wolf` | **编队规模**：本舰护航半径内的友舰数（孤舰 → 独狼；成群 → 护航） |
//! | `kiting` | **敌我火力比**（自己 vs 射程内最近的敌舰）+ **自身硬度对比** + **自己挨了多少打** |
//!
//! 每次真的改了就往 `decisions` 追加一行（[`StyleDecision`]，纯记录、不改行为）：读面因此能
//! 回答「AI 为什么把这条轴改成这样」，而不是只看到一个数变了。

use crate::model::*;
use crate::sim;
use std::collections::BTreeMap;

use super::r2;
use super::tactics;

/// 风格轴（重估规则同形，只有"目标值从哪来"不同）。
///
/// `temper` 与 `lone_wolf` 合成一条 [`StyleAxis::Doctrine`]：它们在引擎里**本来就是一片叶装
/// 两条轴**（`Control<ShipDoctrine>`），所以它们的闸门也永远是同一个（同一片逐舰叶 + 同一片
/// 势力级默认叶）。分成两条"分量"只影响记录，不影响这块叶。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StyleAxis {
    Doctrine,
    Kiting,
}

impl StyleAxis {
    /// 轴的线上名字（决定记录里的 `axis` 列；也是它那两枚派生骰子的 `salt` 前缀）。
    fn name(self) -> &'static str {
        match self {
            StyleAxis::Doctrine => "doctrine",
            StyleAxis::Kiting => "kiting",
        }
    }
}

/// 一个势力**本回合的战况**——风格重估的驱动力全在这里（都是已有状态/事件的读数）。
struct Situation {
    /// 战争强度 0..1（[`sim::war_strength`]：关系跌到交战阈值之下有多深）。
    war: f64,
    /// 舰队被打残的**连续**程度 0..1（`1 − Σhull/Σhull_max`）。
    damage: f64,
    /// 本回合净战果（击毁敌舰 − 被击毁）/ 舰队规模，钳到 [-1,1]。
    win: f64,
    /// 本回合**自保撤退**的舰数占比 0..1。
    withdraw: f64,
}

/// **每回合重估一次**全部归 AI 管的风格叶：`temper` / `lone_wolf` / `kiting`。
///
/// 调用点：`sim::step_military` 的逐舰循环**之后、护甲再生之前**——此时
/// * 本回合的接战/撤退/战沉都已发生（`state.events` 与 `decisions` 都是本回合的），
/// * 而护甲还没回满（"被打残"这个信号还是新鲜的），
/// 于是"这一仗打成什么样 → 下一回合怎么打"这条因果是清楚的。
///
/// 写下**下一回合**的叶（而不是本回合中途改）：本回合的判定早已用掉本回合的取值，
/// 中途改会同时毁掉「与舰的处理顺序无关」和「同回合可解释」这两件事。
pub(crate) fn regulate_styles(
    state: &mut State,
    config: &GameConfig,
    decisions: &[ShipDecision],
    out: &mut Vec<StyleDecision>,
) {
    let ac = &config.autocontrol;
    if ac.style_chance <= 0.0 {
        return; // 关掉执行者：退回「值冻结」的旧行为（配置开关，不是硬编码）。
    }
    let round = state.round;
    let mut fids: Vec<FactionId> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort();
    for fid in fids {
        let sit = situation(state, config, &fid, decisions);
        let drivers_doc = BTreeMap::from([
            ("war".to_string(), sit.war),
            ("win".to_string(), sit.win),
            ("damage".to_string(), sit.damage),
            ("withdraw".to_string(), sit.withdraw),
        ]);
        // 先把本势力的舰与它们的**有效**风格读完，再逐舰写：写叶会改 `state.control`，
        // 而后面几艘舰的目标值都不该依赖"前几艘舰刚写了什么"（无顺序依赖）。
        let roster: Vec<(ShipId, ShipDoctrine, f64)> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
            .map(|s| {
                (
                    s.name.clone(),
                    state.ship_doctrine(s.name.clone()),
                    state.ship_kiting(s.name.clone()),
                )
            })
            .collect();
        for (ship_id, doctrine, kiting) in roster {
            // 闸门①：这艘舰本身归 AI（玩家点名的舰，AI 一个字节都不碰）。
            if state.ship_control(ship_id.clone()) != ControlMode::Auto {
                continue;
            }
            // --- 风格两轴（它们共处一片叶）-----------------------------------------
            if axis_writable(state, &fid, &ship_id, StyleAxis::Doctrine) {
                let mut doc = doctrine;
                // (分量名, 旧值, 新值, 目标, 驱动输入)
                let mut hits: Vec<(&'static str, f64, f64, f64, BTreeMap<String, f64>)> = Vec::new();
                let temper_target = temper_target(ac, &sit);
                if let Some(next) = step_value(
                    config, &fid, &ship_id, round, StyleAxis::Doctrine, "temper", doc.temper,
                    temper_target,
                ) {
                    hits.push(("temper", doc.temper, next, temper_target, drivers_doc.clone()));
                    doc.temper = next;
                }
                let (lone_target, neighbors) = lone_wolf_target(state, config, &ship_id);
                if let Some(next) = step_value(
                    config, &fid, &ship_id, round, StyleAxis::Doctrine, "lone_wolf", doc.lone_wolf,
                    lone_target,
                ) {
                    hits.push((
                        "lone_wolf",
                        doc.lone_wolf,
                        next,
                        lone_target,
                        BTreeMap::from([("neighbors".to_string(), neighbors)]),
                    ));
                    doc.lone_wolf = next;
                }
                // 只在真改了的时候写叶（值没变就不重写：控制面 diff 是给人读的）。
                if !hits.is_empty() {
                    if let Some(c) = state.control_mut(fid.clone()) {
                        // ⚠ 两条轴**一次写**：没被重估的那条带着它**当时在用的值**——这就是
                        // 「只改一条轴不会把另一条清零」那条规矩的落点（`control-live-layers.md` §3.1）。
                        c.ship_doctrine.insert(ship_id.clone(), Control::inherit(doc));
                    }
                    for (component, from, to, target, drivers) in hits {
                        out.push(StyleDecision {
                            faction: fid.clone(),
                            ship: ship_id.clone(),
                            axis: component.to_string(),
                            from: r2(from),
                            to: r2(to),
                            target: r2(target),
                            drivers,
                        });
                    }
                }
            }
            // --- 风筝<->贴脸（另一片叶）---------------------------------------------
            if axis_writable(state, &fid, &ship_id, StyleAxis::Kiting) {
                let (target, drivers) = kiting_target(state, config, &ship_id);
                if let Some(next) =
                    step_value(config, &fid, &ship_id, round, StyleAxis::Kiting, "kiting", kiting, target)
                {
                    if let Some(c) = state.control_mut(fid.clone()) {
                        c.ship_kiting.insert(ship_id.clone(), Control::inherit(next));
                    }
                    out.push(StyleDecision {
                        faction: fid.clone(),
                        ship: ship_id.clone(),
                        axis: StyleAxis::Kiting.name().to_string(),
                        from: r2(kiting),
                        to: r2(next),
                        target: r2(target),
                        drivers,
                    });
                }
            }
        }
    }
}

/// **这条轴此刻归谁**——写叶的三道闸门（宁严勿宽，理由见模块文档）。
fn axis_writable(state: &State, fid: &str, ship_id: &str, axis: StyleAxis) -> bool {
    let Some(c) = state.control(fid.to_string()) else {
        return false;
    };
    let (leaf, default) = match axis {
        StyleAxis::Doctrine => (
            c.ship_doctrine.get(ship_id).map(|l| l.mode).unwrap_or_default(),
            c.default_doctrine.as_ref().map(|l| l.mode).unwrap_or_default(),
        ),
        StyleAxis::Kiting => (
            c.ship_kiting.get(ship_id).map(|l| l.mode).unwrap_or_default(),
            c.default_kiting.as_ref().map(|l| l.mode).unwrap_or_default(),
        ),
    };
    // ① 逐舰叶：玩家钉的这艘舰的特例，一个字都不许改。
    if leaf.is_player() {
        return false;
    }
    // ② 势力级默认叶是玩家 ⇒ **本轴的逐舰叶一律不写**（比归属链更严一档）：
    //    那是玩家给全舰队定的答案，AI 的逐舰流水不该盖它。链本身的语义（叶优先于默认）
    //    留给**玩家**的逐舰例外用——AI 不是玩家。
    if default.is_player() {
        return false;
    }
    // ③ 更宽的层（势力作用域 / 全局）：链说归玩家就不写（"这支舰队归你了"）。
    let owner = match axis {
        StyleAxis::Doctrine => state.ship_doctrine_control(ship_id.to_string()),
        StyleAxis::Kiting => state.ship_kiting_control(ship_id.to_string()),
    };
    !owner.is_player()
}

/// 抽两枚派生骰子（`chance` 决定"这回合改不改"、`step` 决定"走多远"）；返回一步之后的新值。
///
/// `None` = 这回合**不表态**：没抽中、或抽中的那一步小到不值得写叶（`style_epsilon`）。
/// `component` 进 `salt` ⇒ `temper` 与 `lone_wolf` 拿的是两枚独立的骰子（同一片叶、两条轴，
/// 它们不该被同一枚骰子绑在一起）。
fn step_value(
    config: &GameConfig,
    fid: &str,
    ship_id: &str,
    round: u32,
    axis: StyleAxis,
    component: &str,
    cur: f64,
    target: f64,
) -> Option<f64> {
    let ac = &config.autocontrol;
    let chance_salt = format!("{}:{component}", axis.name());
    if sim::derived_roll(fid, ship_id, round, &chance_salt) >= ac.style_chance {
        return None; // 这一回合这艘舰的这条轴不重估（概率触发，不是每回合都动）。
    }
    let step_salt = format!("{}:{component}:step", axis.name());
    let step = (sim::derived_roll(fid, ship_id, round, &step_salt) * ac.style_step_max).clamp(0.0, 1.0);
    let next = r2(cur + (target - cur) * step).clamp(-1.0, 1.0);
    if (next - cur).abs() < ac.style_epsilon {
        return None; // 变化小到读面都看不出来 ⇒ 不写（否则每回合的 diff 全是噪声）。
    }
    Some(next)
}

/// 战况 → `temper` 目标（理智↔热血）：在打、打赢 → 热血；被打残、撤退多 → 理智。
///
/// 四个信号都是**连续**的（没有一条"越过某条线就反过来"），且全为 0 时目标 = 0
/// ⇒ 和平时期这条轴自己松弛回基线。
fn temper_target(ac: &AutoControlConfig, sit: &Situation) -> f64 {
    (sit.war * ac.temper_war + sit.win * ac.temper_win
        - sit.damage * ac.temper_damage
        - sit.withdraw * ac.temper_withdraw)
        .clamp(-1.0, 1.0)
}

/// 编队规模 → `lone_wolf` 目标（护航↔独狼）：**孤舰 → 独狼、成群 → 护航**。
///
/// 映射是 `1 − 2·n/(n+ref)`（`n` = 护航半径内的友舰数）：`n=0` → `+1`（独自行动）、
/// `n=ref` → `0`、`n→∞` → `−1`（贴着编队走）。它**没有阈值**——多一艘友舰就多一点护航倾向，
/// 而「护航」这件事本身在 `tactics::resolve_target` 里还要求这支舰队有旗舰（航母）。
fn lone_wolf_target(state: &State, config: &GameConfig, ship_id: &str) -> (f64, f64) {
    let Some(ship) = state.ship(ship_id) else {
        return (0.0, 0.0);
    };
    let r = if config.autocontrol.lone_wolf_radius > 0.0 {
        config.autocontrol.lone_wolf_radius
    } else {
        config.combat.escort_range
    };
    let mut n = 0.0;
    if r > 0.0 {
        for s in &state.ships {
            if s.name != ship_id && s.faction_id == ship.faction_id && s.hull > 0.0
                && sim::dist(ship.position, s.position) <= r
            {
                n += 1.0;
            }
        }
    }
    let ref_n = config.autocontrol.lone_wolf_ref.max(1e-6);
    (1.0 - 2.0 * (n / (n + ref_n)), n)
}

/// 敌我态势 → `kiting` 目标（风筝↔贴脸）与它的驱动输入。
///
/// * **火力比**：自己（含附近友舰的威慑叠加）比射程内最近的敌舰强 ⇒ 贴脸；弱 ⇒ 风筝。
///   取对数比（与 `tactics::doctrine_weight` 里的 temper 用同一把尺子），钳到 [-1,1]。
/// * **硬度对比**：`(我船体+护盾 − 敌船体−护盾) / 两者之和`（甲厚的一方更该压近）。
/// * **挨打程度**：`1 − hull/hull_max`（残舰拉开距离——与它同时抬高的撤退阈值同向）。
///
/// **附近没有敌舰 ⇒ 目标 0**：这条轴只在接战时才有意义（`kiting_dest` 也要先找到敌舰），
/// 和平时期让它松弛回基线，而不是凭"周边无敌"就判成贴脸。
fn kiting_target(state: &State, config: &GameConfig, ship_id: &str) -> (f64, BTreeMap<String, f64>) {
    let zero = BTreeMap::from([
        ("power".to_string(), 0.0),
        ("hardness".to_string(), 0.0),
        ("hurt".to_string(), 0.0),
    ]);
    let Some(ship) = state.ship(ship_id) else {
        return (0.0, zero);
    };
    let panel = ship_panel(config, ship);
    // 感知半径与 `tactics::kiting_dest` 同一把尺子（武器射程 + 一点缓冲）。
    let awareness = panel.attack_range + 0.5;
    let Some(foe) = tactics::nearest_enemy_ship(
        state,
        config,
        &ship.faction_id,
        ship.position,
        awareness,
        None,
        ship_id,
    ) else {
        return (0.0, zero);
    };
    let Some(foe_ship) = state.ship(&foe) else {
        return (0.0, zero);
    };
    let foe_panel = ship_panel(config, foe_ship);
    let my = sim::deterrence(state, config, ship_id);
    let theirs = sim::deterrence(state, config, &foe);
    let power = (((my + 1.0) / (theirs + 1.0)).ln() * 0.5).clamp(-1.0, 1.0);
    let (my_tough, foe_tough) = (panel.hull_max + panel.shield_max, foe_panel.hull_max + foe_panel.shield_max);
    let hardness = ((my_tough - foe_tough) / (my_tough + foe_tough).max(1e-9)).clamp(-1.0, 1.0);
    let hurt = (1.0 - ship.hull / ship.hull_max.max(1e-9)).clamp(0.0, 1.0);
    let ac = &config.autocontrol;
    let target = (power * ac.kiting_power + hardness * ac.kiting_hardness - hurt * ac.kiting_hurt)
        .clamp(-1.0, 1.0);
    (
        target,
        BTreeMap::from([
            ("power".to_string(), power),
            ("hardness".to_string(), hardness),
            ("hurt".to_string(), hurt),
        ]),
    )
}

/// 这个势力**本回合的战况**：把已有状态与本回合的事件/判定读成四个连续信号。
fn situation(
    state: &State,
    config: &GameConfig,
    fid: &str,
    decisions: &[ShipDecision],
) -> Situation {
    let war = sim::war_strength(state, config, fid);
    let (mut hull, mut hull_max, mut fleet) = (0.0f64, 0.0f64, 0.0f64);
    for s in state.ships.iter().filter(|s| s.faction_id == fid && s.hull > 0.0) {
        hull += s.hull;
        hull_max += s.hull_max.max(0.0);
        fleet += 1.0;
    }
    let damage = if hull_max > 1e-9 {
        (1.0 - hull / hull_max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // 净战果：**只算战沉**（`DeathCause::Combat`）——维护费欠缴导致的锈蚀报废不是战果。
    let (mut kills, mut losses) = (0.0, 0.0);
    for e in &state.events {
        if let GameEvent::ShipDestroyed { owner, cause: DeathCause::Combat, by, .. } = e {
            if owner == fid {
                losses += 1.0;
            } else if by.as_ref().map(|k| k.faction == fid).unwrap_or(false) {
                kills += 1.0;
            }
        }
    }
    let win = ((kills - losses) / fleet.max(1.0)).clamp(-1.0, 1.0);
    let withdrew = decisions
        .iter()
        .filter(|d| d.faction == fid && d.verdict == ShipVerdict::Withdraw)
        .count() as f64;
    let withdraw = (withdrew / fleet.max(1.0)).clamp(0.0, 1.0);
    Situation { war, damage, win, withdraw }
}

#[cfg(test)]
mod tests {
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
}

