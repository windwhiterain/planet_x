//! **集货派单**：自动控制怎么给运输舰排路线、又该让几艘舰去跑运输。
//!
//! 三件事，都是**纯函数**（只读 `State`，不掷骰、不写状态）：
//!
//! 1. [`needed_freighters`]：本势力**该有多少艘运输舰**（一处有积压的货栈配一条船）；
//! 2. [`should_be_freighter`]：**这艘舰**是不是该跑运输（按上一个数定编，与遍历顺序无关）；
//! 3. [`route_for`]：这艘舰**该跑哪条线**——`from` 按**积压占比抽签**、`to` 永远是首都。
//!
//! # 为什么「选哪处积压」用抽签而不是贪心
//!
//! 用户裁决：**每艘船去哪个积压点的伪随机概率分布 = 积压占比**。这条规则一条话就说完，
//! 却自带三个好处：
//!
//! * **分配自动按积压成比例**：积压 100 的货栈期望分到 10 倍于积压 10 的那处的运力，
//!   不必维护「已派了几条船」的账（那本账还得考虑船在路上、船被击沉、船改行……）；
//! * **不需要协商**：每艘舰自己掷一次骰子就走，天然无中心、无顺序依赖；
//! * **仍然完全确定**：骰子是 `(势力, 舰名, 回合, "route")` 派生出来的（[`sim::derived_roll`]），
//!   同种子同回合逐字复现，且**不消费主 `Prng` 流**（否则「多派一艘船」会改变整个世界
//!   后续的掷骰，同种子可复现就退化成了「舰队数量一变后面全变」）。
//!
//! 抽签只在**需要选一条新线**时掷：`route_for` 优先续用现有路线（货栈还有货、或舱里载着货），
//! 所以船不会每回合在几处积压之间反复改道——「一票货永远运不回家」的抖动没有落点。
//!
//! # 与「角色」的分工
//!
//! 这一层**只给运输舰排线**。谁是运输舰是第三条风格轴
//! （[`State::ship_freighter`](crate::model::State::ship_freighter)）说了算，
//! 而「该有几艘」是这里的 [`needed_freighters`]——自动控制把结论写回那片叶
//! （`Player` 的叶不碰），于是玩家能覆写、AI 也不必每回合重新发明结论。

use crate::model::*;
use crate::sim;

/// 一批货栈存货的**总件数**（不折算价值：「把东西搬回来」与「值多少钱」是两件事，
/// 后者交给市场）。
fn units_of(map: &ResourceMap) -> f64 {
    map.values().sum()
}

/// 本势力**有积压的货栈**（天体名 + 积压件数），按天体名序（`BTreeMap` 遍历顺序 ⇒ 确定性）。
///
/// 含**首都天体上的货栈**（若存在）：那通常意味着迁都把一处旧中转点留在了首都——
/// 把它扫进池子也是一条合法路线（`Haul { from: cap, to: cap }`）。
pub fn stocked_depots(state: &State, fid: &str) -> Vec<(BodyId, f64)> {
    state
        .depots
        .iter()
        .filter(|((f, _), _)| f == fid)
        .map(|((_, b), m)| (b.clone(), units_of(m)))
        .filter(|(_, u)| *u > 1e-9)
        .collect()
}

/// 本势力**该有多少艘运输舰**：一处有积压的货栈配一条船。
///
/// 这是个**刻意粗糙**的定编（用户的指示是「先确定机制的正确性，不着急管平衡性」）：
/// 它不含任何阈值常数，且「积压清空一处 ⇒ 那艘船自然改回战舰」（见 [`should_be_freighter`]）。
/// 真要做细，该考虑的是「按积压量 + 航程折算需要几艘」，那是平衡层的事。
pub fn needed_freighters(state: &State, fid: &str) -> usize {
    stocked_depots(state, fid).len()
}

/// 一艘舰的**集货运力**（定编的排序键）：`有效舱容 × 巡航速度 ÷ 维护费`。
///
/// * **舱容**只说明「一趟能装多少」；单位时间的运力还要乘**速度**——航程一定时，跑得快就是
///   跑得勤（来回时间 ≈ `2 × 航程 ÷ 巡航速度`）。只看舱容会把「装得多但慢」的船排错，
///   而在本作里速度**完全来自推进模块**：*没有推进模块的船速度是 0*，派它去运货等于派一尊
///   雕像——所以 0 速的舰**根本不该出现在运力名单里**（本函数返回 0，调用处据此剔除）。
/// * 两项都取**有效值**：舱容乘战损折算（[`cargo_capacity`]），速度取 [`ship_panel`] 的巡航速度
///   ——推进模块的**完整度**已经折在里面了。于是打残的船自动让位给完好的船。
/// * 除以**维护费**：运货的成本是养船，同样的钱能搬多少货才是舰级的运输效率；
///   这也顺手把「主力舰（战列）别去拉货」变成排序的自然结果，而不是一条特判。
pub fn freight_tonnage(config: &GameConfig, ship: &Ship) -> f64 {
    let panel = ship_panel(config, ship);
    if panel.speed <= 0.0 {
        return 0.0; // 动不了 ⇒ 运力是**零**，不是「很小」。
    }
    cargo_capacity(config, ship) * panel.speed / panel.upkeep.max(1e-6)
}

/// **这艘舰是不是该跑运输**。纯函数、与舰的遍历顺序无关（自动控制逐舰调用它，结论必须一致）。
///
/// 规则，按优先级：
/// 1. **舱里有货 ⇒ 一定是运输舰**。角色一改，就再没人执行那条路线，货会永远烂在舱里；
/// 2. 否则按**定编**取：本势力活舰按 [`freight_tonnage`]（舱容 × 速度 ÷ 维护费）大者优先、
///    同分按名字序，前 `needed_freighters`（扣掉已在运货的那些）名是运输舰。
///
/// # 为什么排序键长这样
///
/// 先试过**只看舱容**——它有两个实测出来的硬伤：
///
/// 1. **把主力舰从战线上抽走**：实测 seed 7 / r120，俄罗斯按舱容选出的 8 条运输舰里有
///    **5 条战列舰**（载火力 2.0 的一锤定音舰全去拉货了）。
/// 2. **不算速度**：一趟装多少只是每趟的量，**单位时间的运力 = 舱容 × 速度**。
///
/// 换成 `舱容 × 速度 ÷ 维护费`（速度取**实装推进模块**的有效值，不是舰级系数）后，
/// 排序天然对上舰级身份与招牌（航母=散货船、驱逐=远洋部署、战列=最不该拉货的那条）：
///
/// | 舰级 | 舱容 | speed_mult | 维护费 | 运力÷维护费（同型推进下） |
/// |---|---|---|---|---|
/// | 航母 | 20 | 1.0 | 7.5 | **2.67** |
/// | 驱逐 | 4 | **1.3** | 2.5 | 2.08（远洋部署 = 天生的护航/集货舰） |
/// | 护卫 | 2 | 1.0 | 1.5 | 1.33 |
/// | 巡洋 | 6 | 1.0 | 6.0 | 1.00 |
/// | 战列 | 6 | 0.9 | 6.5 | 0.83（**最不该去拉货的一条**） |
pub fn should_be_freighter(state: &State, config: &GameConfig, fid: &str, ship_id: &str) -> bool {
    // 1) 舱里有货：它必须把货送完（否则货烂在舱里）。这条**故意压过**运力排序——
    //    哪怕它刚被打残、运力掉到很低，也得把手上那票货交出去（或死在路上）。
    if state
        .ship(ship_id)
        .map(|s| !s.cargo.is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    let needed = needed_freighters(state, fid);
    if needed == 0 {
        return false;
    }
    // 2) 定编：先把「舱里有货」的舰排在最前（它们已经占掉名额），再按运力/维护费。
    //    **只算自动控制开的舰**（玩家开的舰不替玩家派活），**且必须动得了**（运力 > 0）。
    let mut cands: Vec<(String, bool, f64)> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .filter(|s| state.ship_control(s.name.clone()) == ControlMode::Auto)
        .map(|s| (s.name.clone(), !s.cargo.is_empty(), freight_tonnage(config, s)))
        .filter(|(_, holding, tonnage)| *holding || *tonnage > 0.0)
        .collect();
    cands.sort_by(|a, b| {
        b.1.cmp(&a.1) // 载着货的优先
            .then_with(|| b.2.total_cmp(&a.2)) // 运力/维护费大者优先
            .then_with(|| a.0.cmp(&b.0)) // 名字序兜底（确定性）
    });
    cands
        .iter()
        .take(needed)
        .any(|(name, _, _)| name == ship_id)
}

/// **本回合的定编**：把「谁是运输舰」一次性写进第三条风格轴
/// （[`State::ship_freighter`](crate::model::State::ship_freighter) 那片叶）。
///
/// 每回合跑一次，且**只看本回合开始时的状态**（在 `step_ships` 的逐舰循环**之前**调用）
/// ——逐舰现算会让结论依赖舰的处理顺序，而那个顺序是按 `rng` 打乱的。
///
/// 写叶有两道闸，缺一不可：
/// 1. **只写自动控制自己开的舰**（`ship_control == Auto`）：玩家开的舰一个字节都不碰；
/// 2. **归属链判定是 `Player` 就不写**：玩家在叶上或舰队默认上表过态 ⇒ 这条轴归玩家。
///
/// 值没变就不重写：控制面的 diff 是给人读的，把同一个值每回合重写一遍只会制造噪声。
pub(crate) fn assign_roles(state: &mut State, config: &GameConfig) {
    let mut fids: Vec<String> = state.factions.iter().map(|f| f.name.clone()).collect();
    fids.sort();
    for fid in fids {
        // 先算完整个势力的名单再写：同一回合内几个势力的结论互不影响（也更好推理）。
        let mut plan: Vec<(String, bool)> = Vec::new();
        for s in state
            .ships
            .iter()
            .filter(|s| s.faction_id == fid && s.hull > 0.0)
        {
            if state.ship_control(s.name.clone()) != ControlMode::Auto {
                continue;
            }
            if state.ship_freighter_control(s.name.clone()).is_player() {
                continue;
            }
            plan.push((s.name.clone(), should_be_freighter(state, config, &fid, &s.name)));
        }
        for (name, role) in plan {
            let unchanged = state
                .control(fid.clone())
                .and_then(|c| c.ship_freighter.get(&name))
                .map(|l| l.value == role && l.mode == ControlMode::Inherit)
                .unwrap_or(false);
            if unchanged {
                continue;
            }
            if let Some(c) = state.control_mut(fid.clone()) {
                c.ship_freighter.insert(name, Control::inherit(role));
            }
        }
    }
}

/// 给某舰挑一条集货路线：`from` 按**积压占比**抽签，`to` 永远是本势力首都
/// （公理：首都即集散地）。`None` = 没有货要运（或势力连首都都没有）。
///
/// **优先续用现有路线**（舱里有货、或那处货栈还有货）——常驻路线不该每回合重掷。
/// 抽签细节见本模块的文档。
pub fn route_for(state: &State, fid: &str, ship_id: &str) -> Option<(BodyId, BodyId)> {
    let to = state.capital_body(fid);
    if to.is_empty() || state.body(&to).is_none() {
        return None;
    }
    let holding = state
        .ship(ship_id)
        .map(|s| !s.cargo.is_empty())
        .unwrap_or(false);
    let cands = stocked_depots(state, fid);
    // 续用现有路线：只要那处还有货（或舱里载着货要送），就不改道。
    if let Some(ShipBehavior::Haul { from, .. }) = state.ship_behavior(ship_id.to_string()) {
        if state.body(&from).is_some() {
            if holding || cands.iter().any(|(b, _)| *b == from) {
                return Some((from, to));
            }
        }
    }
    // 舱里有货但**没有**路线（例如玩家把指令清掉了）：先把货送回家再说。
    // `from = to = 首都` 是合法的「只卸不装」路线——`haul_step` 只看 `to`（舱里有货时腿别就是 `to`）。
    if holding {
        return Some((to.clone(), to));
    }
    if cands.is_empty() {
        return None;
    }
    // 抽签：把 [0, 总积压) 按各处积压切成区间，落在哪段就去哪儿 ⇒ 概率 = 积压占比。
    let total: f64 = cands.iter().map(|(_, u)| *u).sum();
    if total <= 0.0 {
        return None;
    }
    let mut x = sim::derived_roll(fid, ship_id, state.round, "route") * total;
    let mut from = cands.last().map(|(b, _)| b.clone())?; // 浮点兜底：落到末尾之外就取最后一处
    for (b, u) in &cands {
        if x < *u {
            from = b.clone();
            break;
        }
        x -= u;
    }
    Some((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;
    use crate::world::default_state;

    fn fresh(seed: u64) -> (GameConfig, State) {
        let config = load_config();
        let state = default_state(&config, seed);
        (config, state)
    }

    /// **定编 = 有积压的货栈数**，且它随积压清空自动归零（船自然改回战舰）。
    #[test]
    fn crew_size_is_one_ship_per_stocked_depot() {
        let (_config, mut state) = fresh(42);
        state.depots.clear();
        assert_eq!(needed_freighters(&state, "中国"), 0, "没有积压就没有运输舰");
        state.depot_add("中国", "金星", "碳", 1.0);
        assert_eq!(needed_freighters(&state, "中国"), 1, "一处积压配一条船");
        state.depot_add("中国", "水星", "铁", 1.0);
        assert_eq!(needed_freighters(&state, "中国"), 2, "两处积压配两条船");
        // 清空积压 ⇒ 定编回 0（「积压清空那艘船就改回战舰」的机制落点）。
        state.depots.clear();
        assert_eq!(needed_freighters(&state, "中国"), 0);
    }

    /// **抽签分布 = 积压占比**（用户裁决）：两处货栈积压 3:1 时，多条舰抽出来的比例要贴近 3:1。
    ///
    /// 这里直接验证机制而不跑模拟：同一回合里换舰名掷骰子，看落点分布。
    #[test]
    fn route_lottery_is_proportional_to_the_backlog() {
        let (_config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 30.0);
        state.depot_add("中国", "水星", "铁", 10.0);
        let mut hits = std::collections::BTreeMap::<String, usize>::new();
        for i in 0..4000 {
            let ship = format!("抽签舰{i}");
            if let Some((from, _)) = route_for(&state, "中国", &ship) {
                *hits.entry(from).or_insert(0) += 1;
            }
        }
        let venus = hits.get("金星").copied().unwrap_or(0) as f64;
        let mercury = hits.get("水星").copied().unwrap_or(0) as f64;
        assert!(venus + mercury > 3900.0, "每艘舰都该抽到一处：{hits:?}");
        let ratio = venus / mercury;
        assert!(
            (2.6..3.4).contains(&ratio),
            "积压 3:1 ⇒ 抽中比例应贴近 3:1，实为 {ratio:.2}（{hits:?}）"
        );
    }

    /// **角色是控制属性、AI 会写它、玩家能压住它**（用户裁决：像风格一样）。
    ///
    /// 三件事一起钉：定编按舱容选船；结论确实落在叶子上；**玩家把叶设成 `Player` 之后
    /// 自动定编再也不碰它**（哪怕积压清空——否则「我明明钉了角色却没生效」）。
    #[test]
    fn the_ai_writes_the_role_leaf_but_never_over_a_player() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        // 中国开局：两艘护卫（舱容 2）+ 一艘驱逐（舱容 4）。一处积压 ⇒ 只定一艘，且该是驱逐。
        state.depot_add("中国", "金星", "碳", 100.0);
        assign_roles(&mut state, &config);
        let haulers: Vec<String> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == "中国" && state.ship_freighter(s.name.clone()))
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(haulers.len(), 1, "一处积压配一条船：{haulers:?}");
        let hauler = haulers[0].clone();
        assert_eq!(
            state.ship(&hauler).unwrap().class,
            "destroyer",
            "按运力（舱容 × 速度 ÷ 维护费）大者优先——同样的舰队规模，舱容与速度一起决定吞吐"
        );
        let leaf = state
            .control("中国".to_string())
            .and_then(|c| c.ship_freighter.get(&hauler))
            .expect("结论要落在叶子上（否则每回合都要重新发明）");
        assert!(leaf.value, "这艘是运输舰");
        assert_eq!(
            leaf.mode,
            ControlMode::Inherit,
            "AI 写的是 Inherit（「这一层没有说话」）——与舰指令同一条规矩：\
             于是玩家把**舰队默认**设成 Player 时，玩家的意图能压过 AI 的逐舰结论"
        );

        // 玩家钉死这艘舰的角色 ⇒ 自动定编一个字都不许写（哪怕积压已经清空）。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_freighter
            .insert(hauler.clone(), Control::player(true));
        state.depots.clear();
        assign_roles(&mut state, &config);
        assert!(
            state.ship_freighter(hauler.clone()),
            "玩家钉的角色：AI 不得改写（哪怕没有积压）"
        );
        assert_eq!(
            state
                .control("中国".to_string())
                .unwrap()
                .ship_freighter
                .get(&hauler)
                .unwrap()
                .mode,
            ControlMode::Player,
            "那片叶仍然归玩家"
        );
    }

    /// **删叶 = 交回自动定编**：玩家给某艘舰钉过角色（`Player`）之后 AI 一个字都不写；
    /// 把这片叶删掉，这艘舰立刻回到「AI 按积压定编」的自由状态——下回合 AI 会把结论重新写进
    /// 一片新叶。这正是这条轴与另两条风格轴的差别：**删叶不是"锁成某个值"，而是"放手"**。
    #[test]
    fn deleting_the_role_leaf_hands_the_ship_back_to_auto_planning() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .map(|s| s.name.clone())
            .expect("中国至少有一艘舰");
        // 玩家钉死「它是运输舰」，而此刻没有任何积压 ⇒ 定编本来会把它判成战舰。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_freighter
            .insert(ship.clone(), Control::player(true));
        assign_roles(&mut state, &config);
        assert!(state.ship_freighter(ship.clone()));
        assert!(
            state.control("中国".to_string()).unwrap().ship_freighter.contains_key(&ship),
            "玩家的叶 AI 不碰，所以它还在"
        );

        // 删叶：玩家放手 ⇒ 归属不再拦着 AI。
        let diff = serde_json::json!({
            "control": [{"faction_id": "中国", "ship_freighter": [{"ship": ship, "remove": true}]}]
        });
        let r = crate::control::apply_patch(&mut state, &config, &diff).expect("diff applies");
        assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
        assert!(
            !state.control("中国".to_string()).unwrap().ship_freighter.contains_key(&ship),
            "叶必须真的没了"
        );

        // 有积压 ⇒ 定编重新生效（谁去运由运力排序决定，但必须有人运）。
        state.depot_add("中国", "金星", "碳", 100.0);
        assign_roles(&mut state, &config);
        let haulers: Vec<String> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == "中国" && state.ship_freighter(s.name.clone()))
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(haulers.len(), 1, "删掉玩家的钉子之后 AI 重新定编：{haulers:?}");
    }

    /// **运力要算速度**（用户点破的那条）：一趟装多少只是**每趟**的量，单位时间的运力是
    /// `舱容 × 速度`（航程一定时，跑得快 = 跑得勤）。而且速度完全来自推进模块 ⇒
    /// **没有推进模块的船速度是 0，派它去运货等于派一尊雕像**：它必须被剔出运力名单。
    #[test]
    fn freight_tonnage_counts_speed_and_never_picks_a_ship_that_cannot_move() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 100.0);
        state.depot_add("中国", "火星", "铁", 100.0);
        // **三处**积压 ⇒ 定编 3，而中国只有 3 艘舰、其中 1 艘将被拆成裸舰 ⇒ 动得了的只有 2 艘。
        // 这样「0 速的舰要被剔出运力名单」才有判别力：没有剔除时它会被凑进定编（3 条）。
        state.depot_add("中国", "水星", "硅", 100.0);
        // 把一艘护卫拆成**裸舰**：没有推进模块 ⇒ 巡航速度 0。
        let stripped = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.class == "corvette")
            .expect("中国开局有护卫舰")
            .name
            .clone();
        {
            let s = state.ship_mut(&stripped).unwrap();
            s.components.clear();
            s.component_hp.clear();
        }
        assert_eq!(
            ship_panel(&config, state.ship(&stripped).unwrap()).speed,
            0.0,
            "用例前提：裸舰没有推进模块 ⇒ 速度 0"
        );
        assert_eq!(
            freight_tonnage(&config, state.ship(&stripped).unwrap()),
            0.0,
            "速度 0 ⇒ 运力为零（不是「很小」）"
        );
        assign_roles(&mut state, &config);
        assert!(
            !state.ship_freighter(stripped.clone()),
            "速度 0 的舰物理上运不了货——不该被派去跑运输"
        );
        // 选中的人必须**正好是按「舱容 × 速度 ÷ 维护费」排出来的前二**。
        let mut ranked: Vec<(String, f64)> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == "中国" && s.hull > 0.0)
            .map(|s| (s.name.clone(), freight_tonnage(&config, s)))
            .collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let chosen: Vec<String> = state
            .ships
            .iter()
            .filter(|s| s.faction_id == "中国" && state.ship_freighter(s.name.clone()))
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(
            chosen.len(),
            2,
            "定编 3（三处积压）但只有 2 艘动得了 ⇒ 只该定 2 条运输舰，实为 {chosen:?}"
        );
        assert!(
            !chosen.contains(&stripped),
            "速度 0 的舰不该混进定编：{chosen:?}"
        );
        // 公式本身：运力 = 舱容 × 速度 ÷ 维护费，其中舱容按战损**连续**折算
        //（把一艘完好的舰打到半血 ⇒ 运力减半）。
        let intact = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.name != stripped && s.hull > 0.0)
            .unwrap()
            .clone();
        let t = freight_tonnage(&config, &intact);
        assert!(t > 0.0, "完好的舰运力必须为正");
        let mut hurt = intact.clone();
        hurt.hull = hurt.hull_max * 0.5;
        assert!(
            (freight_tonnage(&config, &hurt) - t * 0.5).abs() < 1e-9,
            "装甲掉一半 ⇒ 舱容减半 ⇒ 运力减半"
        );
        let top2: Vec<&String> = ranked.iter().take(2).map(|(n, _)| n).collect();
        for name in &chosen {
            assert!(
                top2.contains(&name),
                "选中的必须是运力前二：选中 {chosen:?}，排名 {ranked:?}"
            );
        }
    }

    /// **有效角色的取值链**：叶 → 舰队默认 → 舰上记录值，与前两条风格轴同形。
    /// 舰队默认要是 `Player`，逐舰的叶就说了不算（这是「玩家意图压过 AI 定编」的机制落点）。
    #[test]
    fn the_effective_role_follows_the_leaf_then_the_fleet_default_then_the_record() {
        let (_config, mut state) = fresh(42);
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .unwrap()
            .name
            .clone();
        // 记录值（出厂快照）：护卫舰 = 战舰。
        assert!(!state.ship_freighter(ship.clone()), "护卫舰出厂不是运输舰");
        // 舰队默认（Player）⇒ 全舰队改口。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .default_freighter = Some(Control::player(true));
        assert!(
            state.ship_freighter(ship.clone()),
            "叶没有说话（压根没有）时，Player 的舰队默认说了算"
        );
        // 逐舰的叶（Player）更具体 ⇒ 压过舰队默认。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_freighter
            .insert(ship.clone(), Control::player(false));
        assert!(
            !state.ship_freighter(ship.clone()),
            "更具体的叶（逐舰 Player）压过舰队默认"
        );
    }

    /// **AI 端到端（线路接通）**：有积压时自动控制会定出运输舰并给它排一条线；积压清空后
    /// 那名额自然收回（船改回战舰）。
    #[test]
    fn the_ai_assigns_a_route_when_there_is_a_backlog_and_recalls_it_after() {
        let (config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 100.0);
        let mut rng = crate::prng::Prng::new(42);
        sim::advance(&mut state, &config, &mut rng);
        let hauler = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && state.ship_freighter(s.name.clone()))
            .map(|s| s.name.clone())
            .expect("有积压 ⇒ 自动控制该定一艘运输舰");
        assert!(
            matches!(state.ship_behavior(hauler.clone()), Some(ShipBehavior::Haul { .. })),
            "运输舰该有一条路线，实为 {:?}",
            state.ship_behavior(hauler.clone())
        );
        // 积压清空 + 舱里也没货 ⇒ 定编收回。这里**直接重跑定编**而不是再跑一回合模拟：
        // 模拟里金星会当期产出新的碳、货栈立刻又有货（那是正确行为，不是这个用例要测的事）。
        state.depots.clear();
        for s in state.ships.iter_mut() {
            s.cargo.clear();
        }
        assign_roles(&mut state, &config);
        assert!(
            !state.ship_freighter(hauler.clone()),
            "没有积压了 ⇒ 不该再占着运输舰的名额（船改回战舰）"
        );
    }

    /// **续用现有路线**：货栈还有货时不改道（常驻路线不抖动）；货栈空了才重掷。
    #[test]
    fn an_existing_route_is_kept_while_it_still_has_cargo() {
        let (_config, mut state) = fresh(42);
        state.depots.clear();
        state.depot_add("中国", "金星", "碳", 5.0);
        state.depot_add("中国", "水星", "铁", 500.0); // 积压大变（若重掷，几乎必去水星）
        let ship = state
            .ships
            .iter()
            .find(|s| s.faction_id == "中国")
            .unwrap()
            .name
            .clone();
        // 先给这艘舰写一条去金星的路线。
        state
            .control_mut("中国".to_string())
            .unwrap()
            .ship_orders
            .insert(
                ship.clone(),
                Control::auto(ShipBehavior::Haul {
                    from: "金星".to_string(),
                    to: "地球".to_string(),
                }),
            );
        let picked = route_for(&state, "中国", &ship).unwrap();
        assert_eq!(picked.0, "金星", "金星还有货 ⇒ 续用现有路线，不按积压重掷");
        // 金星清空 ⇒ 才重掷（这次必然去水星，因为只剩它一处）。
        state.depots.remove(&("中国".to_string(), "金星".to_string()));
        let picked = route_for(&state, "中国", &ship).unwrap();
        assert_eq!(picked.0, "水星", "原路线没货了 ⇒ 重新抽签");
    }
}
