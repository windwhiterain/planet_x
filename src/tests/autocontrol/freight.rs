//! 集货派单的单元测试。
//!
//! ## 2026-10：**抽签**那一半搬去了 `play/tests/g2_mid.py`
//!
//! `route_lottery_is_proportional_to_the_backlog` 原来在合成世界上掷 4000 次、看 3:1 的落点
//! 分布。现在 `round_inputs.rolls` 的每条 `route` 记录都带着**那一刻那艘舰**看到的候选腿池
//! （`pool`，权重 = 货量）+ 掷出的 `value` + 选中的 `picked` ⇒ g2 的 `dispatch_checks` 可以
//! **逐条精确复算**：`value × pool_total` 按池子顺序切段，落点必须 == `picked`（比 4000 次抽样
//! 更强：它验证的是同一段代码，而不是统计近似）。
//!
//! ⚠ **为什么不建 `haul_lanes` 表**（施工图 §5 第 4 批的建议）：`route_for` 在 `step_military`
//! 里**逐舰**调用，前面的舰已经把货搬走/池子改了（`haul_step` 就在同一循环里）⇒ 每艘舰看到的
//! 腿都不同。一张「每回合每势力一条」的 `lanes()` 表既不忠实（§12 同回合相位错位）、也会和
//! 抽签记录打架。**决策时刻的腿 = 抽签记录里的 `pool`**，而且是逐舰的 ⇒ 零新增序列化就够了。
//!
//! **留在这里的**：`crew_size_...` / `the_headcount_...` / `ideology_decides_...`（定编配额，
//! 要手工摆世界或直接调 `needed_haulers`）、`the_ai_writes_the_role_leaf_...` /
//! `deleting_the_role_leaf_...` / `the_effective_role_...`（控制叶取值链）、
//! `the_hub_supplies_the_yard_...`（保留量/双向腿，要 `site_reserve`/`site_deficit`）、
//! `freight_tonnage_...`，以及承包市场那 6 条。

use super::*;
use crate::config::load_config;
use crate::world::default_state;

fn fresh(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let state = default_state(&config, seed);
    (config, state)
}

/// 某势力此刻的**运输舰名单**（按舰名序）——用例里到处要看它。
fn roster(state: &State, fid: &str) -> Vec<String> {
    let mut v: Vec<String> = state
        .ships
        .iter()
        .filter(|s| {
            s.faction_id == fid
                && s.hull > 0.0
                && state.ship_role(s.name.clone()) == ShipRole::Freight
        })
        .map(|s| s.name.clone())
        .collect();
    v.sort();
    v
}

/// 钉住一个势力的**思潮两轴**（只有这两轴进集货倾向，见 `LEAN_MILITARY`/`LEAN_COLONY`）。
fn set_ideology(state: &mut State, fid: &str, military: f64, colony: f64) {
    let f = state
        .factions
        .iter_mut()
        .find(|f| f.name == fid)
        .unwrap_or_else(|| panic!("没有势力 {fid}"));
    f.ideology.peace_military = military;
    f.ideology.nature_colony = colony;
}

/// **把用例钉在「只有出口腿」的世界里**（新语义下做机制用例的前置）。
///
/// 改成两条腿之后，一个世界**同时**有两类腿，做单元用例时必须把另一类隔开：
/// * **清空首都池** ⇒ `lanes` 里进口腿一条都不成立（首都拿不出货 ⇒ 「有货才派」）；
/// * 货放在**本势力没有城的天体**上 ⇒ 那里没有建设需求、保留量为 0 ⇒ 全部是可运走的净剩余。
///
/// 于是「一处货栈 = 一条出口腿」，用例可以照旧数腿；进口腿与保留量的守卫各写各的
/// （`the_hub_supplies_the_yard_and_the_site_never_ships_back_what_it_needs`）。
fn export_only(state: &mut State, fid: &str, depots: &[&str], amount: f64) {
    for f in state.factions.iter_mut() {
        f.resources.clear();
    }
    state.depots.clear();
    for b in depots {
        state.depot_add(fid, b, "碳", amount);
    }
}

/// 把某势力的舰队克隆 `times` 倍（名字加后缀）——用例需要一支**够大的**舰队，
/// 否则「按比例投几条腿」会被舰队规模顶住，看不出思潮的差别。
fn grow_fleet(state: &mut State, fid: &str, times: usize) {
    let base: Vec<Ship> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid)
        .cloned()
        .collect();
    for k in 1..=times {
        for s in &base {
            let mut c = s.clone();
            c.name = format!("{}-{k}", s.name);
            state.ships.push(c);
        }
    }
}

/// 连跑 `rounds` 个回合的**角色定编**（每回合先推进 `round` 再定编，与 `sim` 同步：
/// 骰子是 `(势力, 舰名, 回合, "role")` 派生的，**换回合才换骰子**）。
/// 返回每回合的运输舰名单。
///
/// ⚠ **这份用例集测的是集货定编**，而角色轴上现在还有**优先级更高**的第三态（观测舰，
/// 用户裁决「观测 > 运输 > 战斗」）——不把它从棋盘上拿开的话，运输舰的名额会被观测抢走
/// （实测：本文件里四条用例当场翻红，平均头数从 1.46 掉到 1.00），测出来的就不是集货的定编了。
/// 拿开的方式用的是**真实存在的一种状态**：把本势力标成**已经学满 MOND**
/// （`mond_control = 1.0` ⇒ 棘轮之下没有东西可学 ⇒ `observer_quota = 0` ⇒ 没人去观测），
/// 而不是把观测那条机制关掉或改常数。
/// 观测自己的定编/选靶/优先级由 `src/tests/autocontrol/knowledge.rs` 专门覆盖。
fn run_roles(state: &mut State, config: &GameConfig, fid: &str, rounds: u32) -> Vec<Vec<String>> {
    if let Some(f) = state.factions.iter_mut().find(|f| f.name == fid) {
        f.mond_control = 1.0;
    }
    let mut hist = Vec::new();
    for _ in 0..rounds {
        state.round += 1;
        assign_roles(state, config, &mut crate::model::RoundInputs::default());
        hist.push(roster(state, fid));
    }
    hist
}

/// **定编 = 有货要动的腿按活量折算成的头数**（出口 + 进口），且它随货搬完自动归零。
///
/// 世界开局的基准不是 0：每个**建造区**都要常备一套最低可用选装（`site_standing`），
/// 所以「进口腿」从第一回合起就有——这是**故意**的（船坞手里该有装得出一艘能动的船的料）。
#[test]
fn crew_size_is_one_ship_per_stocked_depot() {
    let (config, mut state) = fresh(42);
    state.depots.clear();
    let baseline = needed_haulers(&state, &config, "中国");
    assert!(
        baseline > 0.0,
        "每个建造区常备一套选装 ⇒ 开局就有进口腿（实为 {baseline:.2}）"
    );
    // **首都没有货 ⇒ 进口腿一条都不成立**（「有货才派」，与出口侧「有净剩余才派」同一条纪律）。
    let mut dry = state.clone();
    for f in dry.factions.iter_mut() {
        f.resources.clear();
    }
    assert_eq!(
        needed_haulers(&dry, &config, "中国"),
        0.0,
        "首都拿不出货 ⇒ 不派船"
    );
    // 出口腿：往**没有城的天体**放货（那里没有建设需求 ⇒ 全是净剩余）。
    export_only(&mut state, "中国", &["冥王星", "卡戎"], 100.0);
    assert_eq!(
        needed_haulers(&state, &config, "中国"),
        2.0,
        "两处净剩余 = 两条腿 = 两艘船"
    );
    // 一处搬空 ⇒ 那条腿自然消失（「积压清空那艘船就改回战舰」的机制落点）。
    state.depots.clear();
    state.depot_add("中国", "冥王星", "碳", 100.0);
    assert_eq!(needed_haulers(&state, &config, "中国"), 1.0);
    // **涓流腿只分到几分之一艘船**：1 件货 ≠ 一整艘（见 `needed_haulers` 的注释）。
    state.depots.clear();
    state.depot_add("中国", "冥王星", "碳", 1.0);
    let trickle = needed_haulers(&state, &config, "中国");
    assert!(
        trickle > 0.0 && trickle < 1.0,
        "1 件货只该分到几分之一艘（实为 {trickle:.3}）"
    );
}

/// **角色是控制属性、AI 会写它、玩家能压住它**（用户裁决：像风格一样）。
///
/// 三件事一起钉：有积压时 AI 会把运输舰定出来（结论确实落在叶子上、模式是 `Inherit`）；
/// **运力最好的船优先**（旧的硬排序现在是**软**的，但偏好仍要看得出来）；**玩家把叶设成
/// `Player` 之后自动控制再也不碰它**（哪怕积压清空——否则「我明明钉了角色却没生效」）。
#[test]
fn the_ai_writes_the_role_leaf_but_never_over_a_player() {
    let (config, mut state) = fresh(42);
    state.depots.clear();
    state.depot_add("中国", "金星", "碳", 100.0);
    let hist = run_roles(&mut state, &config, "中国", 60);
    let with_hauler = hist.iter().filter(|r| !r.is_empty()).count();
    assert!(
        with_hauler > 35,
        "有积压就该有人跑运输（60 回合里只有 {with_hauler} 回合有）"
    );
    // 运力最好的船优先：开局是「护卫 ×2 + 驱逐 ×1」，驱逐的运力最高
    //（4×1.3÷2.5 = 2.08 vs 2×1.0÷1.5 = 1.33），它被选中的回合数该多于任何一艘护卫。
    let destroyer = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国" && s.class == "destroyer")
        .expect("中国开局有驱逐舰")
        .name
        .clone();
    let d = hist.iter().filter(|r| r.contains(&destroyer)).count();
    let c_max = hist
        .iter()
        .map(|r| r.iter().filter(|n| **n != destroyer).count())
        .max()
        .unwrap_or(0);
    assert!(
        d > c_max,
        "运力高的船该被优先选中：驱逐 {d} 回合 vs 单艘护卫最多 {c_max} 回合"
    );
    // 结论落在叶子上，且模式是 `Inherit`（玩家把**舰队默认**设成 Player 时能压过 AI）。
    let (hauler, leaf) = state
        .control("中国".to_string())
        .and_then(|c| {
            c.ship_role
                .iter()
                .find(|(_, l)| l.value == ShipRole::Freight)
                .map(|(n, l)| (n.clone(), l.clone()))
        })
        .expect("AI 该在某个回合写过一片 true 的叶");
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
        .ship_role
        .insert(hauler.clone(), Control::player(ShipRole::Freight));
    state.depots.clear();
    assign_roles(&mut state, &config, &mut crate::model::RoundInputs::default());
    assert!(
        state.ship_role(hauler.clone()) == ShipRole::Freight,
        "玩家钉的角色：AI 不得改写（哪怕没有积压）"
    );
    assert_eq!(
        state
            .control("中国".to_string())
            .unwrap()
            .ship_role
            .get(&hauler)
            .unwrap()
            .mode,
        ControlMode::Player,
        "那片叶仍然归玩家"
    );
}

/// **删叶 = 交回自动定编**：玩家给某艘舰钉过角色（`Player`）之后 AI 一个字都不写；
/// 把这片叶删掉，这艘舰立刻回到「AI 按积压 + 思潮定编」的自由状态——之后 AI 会把结论
/// 重新写进一片新叶。这正是这条轴与另两条风格轴的差别：**删叶不是"锁成某个值"，而是"放手"**。
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
    // 玩家钉死「它是运输舰」，而此刻没有任何积压 ⇒ AI 本来不会给它这个角色。
    state
        .control_mut("中国".to_string())
        .unwrap()
        .ship_role
        .insert(ship.clone(), Control::player(ShipRole::Freight));
    assign_roles(&mut state, &config, &mut crate::model::RoundInputs::default());
    assert_eq!(state.ship_role(ship.clone()), ShipRole::Freight);
    assert!(
        state
            .control("中国".to_string())
            .unwrap()
            .ship_role
            .contains_key(&ship),
        "玩家的叶 AI 不碰，所以它还在"
    );

    // 删叶：玩家放手 ⇒ 归属不再拦着 AI。
    let diff = serde_json::json!({
        "control": [{"势力": "中国", "角色": [{"舰": ship, "删叶": true}]}]
    });
    let r = crate::control::apply_patch(&mut state, &config, &diff).expect("diff applies");
    assert!(r.is_clean() && r.removed.len() == 1, "{:?}", r);
    assert!(
        !state
            .control("中国".to_string())
            .unwrap()
            .ship_role
            .contains_key(&ship),
        "叶必须真的没了"
    );

    // 有积压 ⇒ 定编重新生效（角色是掷骰定的，所以看的是「若干回合内有人被定上」）。
    state.depot_add("中国", "金星", "碳", 100.0);
    let hist = run_roles(&mut state, &config, "中国", 40);
    assert!(
        hist.iter().any(|r| !r.is_empty()),
        "删掉玩家的钉子之后 AI 重新定编：40 回合里一个运输舰都没定出来"
    );
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

    // 速度 0 的舰**一个回合都不会**被定成运输舰（物理，不是「排序靠后」）。
    let hist = run_roles(&mut state, &config, "中国", 200);
    assert!(
        hist.iter().all(|r| !r.contains(&stripped)),
        "速度 0 的舰物理上运不了货——一回合都不该被派去跑运输"
    );
    // 三处积压 ⇒ 目标头数 3（中庸），而只有 2 艘动得了 ⇒ 那两艘该基本都常在名单上。
    let mean = hist.iter().map(|r| r.len() as f64).sum::<f64>() / hist.len() as f64;
    assert!(
        mean > 1.85,
        "缺口大过候选数 ⇒ 两艘动得了的基本常驻名单（平均 {mean:.2}）"
    );
    // **软排序**：两艘里运力高的那艘被选中的回合数不少于低的那艘。
    let movable: Vec<(String, f64)> = state
        .ships
        .iter()
        .filter(|s| s.faction_id == "中国" && s.hull > 0.0 && s.name != stripped)
        .map(|s| (s.name.clone(), freight_tonnage(&config, s)))
        .collect();
    assert_eq!(movable.len(), 2, "用例前提：只剩两艘动得了的船");
    let hits = |n: &str| hist.iter().filter(|r| r.contains(&n.to_string())).count();
    let (a, ta) = &movable[0];
    let (b, tb) = &movable[1];
    let (hi, lo) = if ta > tb { (a, b) } else { (b, a) };
    assert!(
        hits(hi) >= hits(lo),
        "运力高的船该更容易被选中：{hi}（{:.2}）{hits_hi} 回合 vs {lo}（{:.2}）{hits_lo} 回合",
        ta.max(*tb),
        ta.min(*tb),
        hits_hi = hits(hi),
        hits_lo = hits(lo)
    );
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
    assert_eq!(
        state.ship_role(ship.clone()),
        ShipRole::War,
        "护卫舰出厂不是运输舰"
    );
    // 舰队默认（Player）⇒ 全舰队改口。
    state.control_mut("中国".to_string()).unwrap().default_role =
        Some(Control::player(ShipRole::Freight));
    assert!(
        state.ship_role(ship.clone()) == ShipRole::Freight,
        "叶没有说话（压根没有）时，Player 的舰队默认说了算"
    );
    // 逐舰的叶（Player）更具体 ⇒ 压过舰队默认。
    state
        .control_mut("中国".to_string())
        .unwrap()
        .ship_role
        .insert(ship.clone(), Control::player(ShipRole::War));
    assert!(
        state.ship_role(ship.clone()) != ShipRole::Freight,
        "更具体的叶（逐舰 Player）压过舰队默认"
    );
}

/// **思潮决定倾向**（用户裁决：「由国家思潮决定自动控制下舰船倾向于运输还是战斗」）。
///
/// 同一个世界、同一批骰子、同一份积压，**只改思潮两轴**：军国端少跑运输、殖民/和平端
/// 多跑运输；而**中庸端正好是 1.0 = 旧硬定编**（这条改动在世界的中位上是行为中性的）。
#[test]
fn ideology_decides_how_much_of_the_fleet_hauls() {
    let (config, base) = fresh(42);
    let stock_four = |st: &mut State| {
        // 四个**没有城**的天体 + 清空首都池 ⇒ 正好四条**出口腿**（进口腿与保留量都不掺进来）。
        export_only(st, "中国", &["冥王星", "卡戎", "土星", "泰坦"], 100.0);
    };
    let mean_headcount = |mil: f64, col: f64| -> f64 {
        let mut st = base.clone();
        set_ideology(&mut st, "中国", mil, col);
        grow_fleet(&mut st, "中国", 3); // 12 艘 ⇒ 舰队规模不顶住「按比例投几条腿」
        stock_four(&mut st);
        let hist = run_roles(&mut st, &config, "中国", 120);
        hist.iter().map(|r| r.len() as f64).sum::<f64>() / hist.len() as f64
    };
    // 中庸（尚武度 0）⇒ 倍数**恰好** 1.0：`2σ(0) = 1` ⇒ 目标头数 = 需求 = 4 条出口腿。
    let mut neutral_state = base.clone();
    set_ideology(&mut neutral_state, "中国", 0.0, 0.0);
    stock_four(&mut neutral_state);
    assert!(
        (freight_lean(&neutral_state, "中国") - 1.0).abs() < 1e-12,
        "中庸必须回到旧硬定编（倍数 1.0），实为 {}",
        freight_lean(&neutral_state, "中国")
    );
    assert!((freighter_quota(&neutral_state, &config, "中国") - 4.0).abs() < 1e-12);

    let militarist = mean_headcount(1.0, 0.0);
    let neutral = mean_headcount(0.0, 0.0);
    let pacifist = mean_headcount(-1.0, 0.0);
    let colonist = mean_headcount(0.0, 1.0);
    // 抽签的期望**正好**是配额（实测 200 回合：1.52 vs 1.46、3.97 vs 4.00、6.47 vs 6.54）。
    // 这条是「概率分布 = 想要的比例」那条纪律的守卫：机制走形（比如每人各掷一次身份）
    // 时它立刻会炸——实测过那种写法会在 0 与 12 之间两极震荡。
    for (mil, col, quota) in [(1.0, 0.0, 1.46), (0.0, 0.0, 4.0), (-1.0, 0.0, 6.54)] {
        let got = mean_headcount(mil, col);
        assert!(
            (got - quota).abs() < 0.35,
            "平均头数该贴着配额（思潮 {mil}×军事 + {col}×殖民 ⇒ 配额 {quota}）：实为 {got:.2}"
        );
    }
    assert!(
        militarist < neutral,
        "军国端该少跑运输：{militarist:.2} vs {neutral:.2}"
    );
    assert!(
        pacifist > neutral,
        "和平端该多跑运输：{pacifist:.2} vs {neutral:.2}"
    );
    assert!(
        colonist > neutral,
        "殖民端要给远方殖民地送补给 ⇒ 该多跑运输（所以它在「尚武度」上是负权重）：{colonist:.2} vs {neutral:.2}"
    );
    // 两轴**同权反号**：既军国又殖民 ⇒ 两股力量抵消（回到中庸附近）。
    let both = mean_headcount(1.0, 1.0);
    assert!(
        (both - neutral).abs() < 1.0,
        "军国 + 殖民该互相抵消：{both:.2} vs 中庸 {neutral:.2}"
    );
}

/// **AI 端到端（线路接通）**：有积压时自动控制会定出运输舰并给它排一条线；积压清空后
/// 那名额自然收回（船改回战舰）。
#[test]
fn the_ai_assigns_a_route_when_there_is_a_backlog_and_recalls_it_after() {
    let (config, mut state) = fresh(42);
    // ⚠ 先把观测那一支**从棋盘上拿开**（把本势力标成已学满 MOND ⇒ 观测主张 0）：角色轴上
    // 现在还有第三态，而它**优先级更高**（用户裁决 观测 > 运输 > 战斗）——不清场的话，
    // 「名额收回」之后那艘船会变成**观测舰**而不是战舰，测出来的就不是集货的收回；
    // 而且观测抽走的运力会让本势力把船**雇出去**（下面的承包承诺），又多一层干扰。
    // 拿开用的是真实存在的一种状态，不是把机制关掉（与 `run_roles` 同一处置）。
    state.faction_mut("中国").unwrap().mond_control = 1.0;
    // 出口腿（无城天体）——进口腿与保留量各有自己的守卫。
    export_only(&mut state, "中国", &["冥王星"], 100.0);
    // 角色是**掷骰**定的（有积压只是「有人去运」的概率高），所以这里跑几个回合而不是一个：
    // 这正是与旧版硬定编的行为差别，用例必须照新语义写，而不是照旧结论写。
    let mut rng = crate::prng::Prng::new(42);
    let mut found = None;
    for _ in 0..20 {
        sim::advance(&mut state, &config, &mut rng);
        // 只挑**没有承包承诺**的运输舰：在役的承包舰被硬承诺（`should_be_role` 第 1 条）钉在
        // 运输位上，配额清空也收不回去——那是正确行为，不是本用例要测的事。
        if let Some(n) = roster(&state, "中国")
            .into_iter()
            .find(|n| state.contracts.assignment_of(n).is_none())
        {
            found = Some(n);
            break;
        }
    }
    let hauler = found.expect("有积压 ⇒ 若干回合内该定出运输舰");
    assert!(
        matches!(
            state.ship_behavior(hauler.clone()),
            Some(ShipBehavior::Haul { .. })
        ),
        "运输舰该有一条路线，实为 {:?}",
        state.ship_behavior(hauler.clone())
    );
    // 积压清空 + 舱里也没货 ⇒ 名额收回。这里**每回合都清货栈与首都池**（模拟里城会当期
    // 产出新的货、货栈立刻又有货，而池子里有货就会开出进口腿——那都是正确行为，
    // 不是这个用例要测的事）。
    let mut recalled = false;
    for _ in 0..20 {
        state.depots.clear();
        for f in state.factions.iter_mut() {
            f.resources.clear();
        }
        for s in state.ships.iter_mut() {
            s.cargo.clear();
        }
        state.round += 1;
        assign_roles(&mut state, &config, &mut crate::model::RoundInputs::default());
        if state.ship_role(hauler.clone()) != ShipRole::Freight {
            recalled = true;
            break;
        }
    }
    assert!(
        recalled,
        "没有积压了 ⇒ 该把运输舰的名额收回去（船改回战舰）"
    );
    // 配额为 0 时**超额是确定的**（带上我就是超一条），所以收回是必然的、且很快。
}

/// **头数稳、人员流动**（用户裁决：「运输/战斗是**动态调整**的，而非固定」；同时也是
/// 笔记里那条「判据里不要出现被这个动作本身改变的量」的守卫——角色读的是上一回合的结论）。
///
/// 两件事一起钉：
/// 1. **条数**钉在配额上（±1 的呼吸），不会两极震荡；
/// 2. **谁去干**每回合都在换（轮换）——既不钉死，也不每回合翻烙饼。
#[test]
fn the_headcount_holds_at_the_quota_while_the_crew_rotates() {
    let (config, base) = fresh(42);
    let setup = |mil: f64, col: f64, depots: &[&str]| {
        let mut st = base.clone();
        set_ideology(&mut st, "中国", mil, col);
        grow_fleet(&mut st, "中国", 3);
        // 只留**出口腿**（无城天体 + 空首都池）⇒ 配额正好是「腿数 × 思潮倍数」。
        export_only(&mut st, "中国", depots, 100.0);
        st
    };
    // 1) 头数围着目标（4）站住，不会在两极之间摆。
    let mut st = setup(0.0, 0.0, &["冥王星", "卡戎", "土星", "泰坦"]);
    let hist = run_roles(&mut st, &config, "中国", 200);
    // 整数配额（需求 4 × 中庸 1.0）⇒ 头数该贴着 4（±1 的呼吸，而不是两极震荡）。
    let in_band = hist.iter().filter(|r| (3..=5).contains(&r.len())).count();
    assert!(
        in_band as f64 / hist.len() as f64 > 0.9,
        "头数该贴着目标：{in_band} / {} 回合落在 3..=5",
        hist.len()
    );
    // 2) 换岗是**慢**的：平均每回合进出的船数远小于 1。
    let churn: usize = hist
        .windows(2)
        .map(|w| w[1].iter().filter(|n| !w[0].contains(n)).count())
        .sum();
    let per_round = churn as f64 / (hist.len() - 1) as f64;
    // **动态但不抖**：轮换让岗位一直换手，而缺口项把换手量压在「岗位数」这个量级里
    //（实测 0.52 条/回合 ⇒ 每条岗位平均 8 个回合换人 ≈ 跑得完几趟来回）。
    assert!(
        (0.2..1.5).contains(&per_round),
        "换手该是「一直在动、但不成片翻烙饼」（实为 {per_round:.3} 条/回合）"
    );
    // 3) 没有积压 ⇒ 全员战舰；新积压一出现 ⇒ 几回合内补得上（不是「一旦改成战舰就回不去」）。
    let mut st = setup(0.0, 0.0, &[]);
    run_roles(&mut st, &config, "中国", 10);
    assert!(
        roster(&st, "中国").is_empty(),
        "没有货要动 ⇒ 谁都不该占着运输舰的名额"
    );
    st.depot_add("中国", "冥王星", "碳", 100.0);
    let mut waited = 0;
    for _ in 0..20 {
        st.round += 1;
        assign_roles(&mut st, &config, &mut crate::model::RoundInputs::default());
        waited += 1;
        if !roster(&st, "中国").is_empty() {
            break;
        }
    }
    assert!(
        waited <= 10,
        "新积压该在几回合内被顶上（实为 {waited} 回合）"
    );
}

/// **续用现有路线**：这条腿还有活时不改道（常驻路线不抖动）；没活了才重掷。
#[test]
fn an_existing_route_is_kept_while_it_still_has_cargo() {
    let (config, mut state) = fresh(42);
    // 两条出口腿（无城天体 ⇒ 保留量为 0，存货全是净剩余）；只有出口腿，抽签结论才唯一。
    export_only(&mut state, "中国", &[], 0.0);
    state.depot_add("中国", "冥王星", "碳", 5.0);
    state.depot_add("中国", "卡戎", "铁", 500.0); // 积压大变（若重掷，几乎必去卡戎）
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .unwrap()
        .name
        .clone();
    // 先给这艘舰写一条去冥王星的路线。
    state
        .control_mut("中国".to_string())
        .unwrap()
        .ship_orders
        .insert(
            ship.clone(),
            Control::auto(ShipBehavior::Haul {
                from: "冥王星".to_string(),
                to: "地球".to_string(),
            }),
        );
    let picked = route_for(&state, &config, "中国", &ship, &mut crate::model::RoundInputs::default()).unwrap();
    assert_eq!(
        picked.0, "冥王星",
        "那条腿还有货 ⇒ 续用现有路线，不按货量重掷"
    );
    // 冥王星清空 ⇒ 才重掷（这次必然去卡戎，因为只剩它一处）。
    state
        .depots
        .remove(&("中国".to_string(), "冥王星".to_string()));
    let picked = route_for(&state, &config, "中国", &ship, &mut crate::model::RoundInputs::default()).unwrap();
    assert_eq!(picked.0, "卡戎", "原路线没货了 ⇒ 重新抽签");
}

// --- 雇佣挂单（雇主的缺口口径）---------------------------------------------

/// 挂的是**自己派不出船的运力缺口**：`要求运力 − 自有运力（落到这处的期望份额）− 已雇到的`。
///
/// 这里**不重算实现里的公式**，而是先量出「一条船都没有时挂多少」（= 这条线的要求运力），
/// 再用它预测「已经有受雇方顶掉一部分」时该挂多少。这样验证的是**形状**
/// （仿射、斜率 −1、下界处夹到 0），而不是把实现抄一遍——公式改了但形状错了，它照样报错。
#[test]
fn the_order_asks_for_the_capacity_the_employer_cannot_cover() {
    let (config, mut state) = fresh(42);
    state.ships.retain(|s| s.faction_id != "中国"); // 中国的船全没了 ⇒ 自有运力 0
    let gap_at = |state: &mut State, hired: f64| -> f64 {
        // 只留**出口腿**（清空首都池 ⇒ 不开进口腿），并只看**那一条腿**上的未接单——
        // 两条腿之后，簿子上会同时有出口与进口的单子，求和会把两个方向混在一起。
        export_only(state, "中国", &["冥王星"], 100.0);
        state.contracts.contracts.clear();
        if hired > 0.0 {
            // 一张**已接单**的合同，承诺了 `hired` 的运力（它就该顶掉缺口）。
            let id = state.contracts.post(
                "中国".into(),
                "碳".into(),
                hired,
                "冥王星".into(),
                "地球".into(),
                0.1,
                0,
                0.0,
            );
            let c = state.contracts.get_mut(id).unwrap();
            c.carrier = Some("美国".into());
            c.accepted_round = Some(0);
            c.expires_round = 99;
        }
        post_contracts(state, &config, &mut RoundSink::default());
        state
            .contracts
            .contracts
            .iter()
            .filter(|c| c.is_open() && c.from == "冥王星")
            .map(|c| c.capacity)
            .sum()
    };
    let need = gap_at(&mut state, 0.0);
    assert!(
        need > 0.0,
        "一条船都没有 ⇒ 该把整条线的要求运力挂出去（实为 {need:.3}）"
    );
    // 已经雇到四成 ⇒ 只该挂剩下的六成。
    let partial = gap_at(&mut state, need * 0.4);
    assert!(
        (partial - need * 0.6).abs() < 1e-9,
        "已雇到 40% ⇒ 该挂 60%（{:.3}），实为 {partial:.3}",
        need * 0.6
    );
    // 雇够了（甚至雇多了）⇒ 一件都不挂（`max(0, ·)` 的下界，不是阈值判断）。
    assert_eq!(gap_at(&mut state, need * 1.2), 0.0, "雇够了 ⇒ 不必再请人");
    // 自己有船 ⇒ 缺口变小（缺口可以小到 0：自己的船把这条线顶上了，那就不必请人）。
    let with_ships = {
        let (config, mut st) = fresh(42);
        // 只留**一艘**护卫（舱容 2、速度 1.0）：能顶掉一部分，但顶不满整条线的要求运力。
        let keep = st
            .ships
            .iter()
            .find(|s| s.faction_id == "中国" && s.class == "corvette")
            .expect("中国开局有护卫舰")
            .name
            .clone();
        st.ships
            .retain(|s| s.faction_id != "中国" || s.name == keep);
        // 这艘船必须**确实在跑运输**：角色现在是掷骰定的（思潮驱动），所以这里用一片
        // `Player` 的叶把它钉住——`should_be_role` 对归玩家的轴不掷骰。
        st.control_mut("中国".to_string())
            .unwrap()
            .ship_role
            .insert(keep.clone(), Control::player(ShipRole::Freight));
        st.depots.clear();
        st.contracts.contracts.clear();
        st.depot_add("中国", "冥王星", "碳", 100.0);
        post_contracts(&mut st, &config, &mut RoundSink::default());
        st.contracts
            .contracts
            .iter()
            .filter(|c| c.is_open() && c.from == "冥王星")
            .map(|c| c.capacity)
            .sum::<f64>()
    };
    assert!(
        with_ships > 0.0 && with_ships < need,
        "一条小船顶不满 ⇒ 缺口该在 (0, {need:.3}) 之间，实为 {with_ships:.3}"
    );
}

/// **一处货栈只有一张未接单，而且它每回合跟着缺口走；已接单的冻结成承诺。**
///
/// 三个必须成立的行为：缺口变了**改的是同一张单**（不新开、不留旧数）；
/// 货栈被搬空 ⇒ **撤单**（受雇方不该照着不存在的需求派船过来）；
/// 有人接了 ⇒ 一个字都不再动。
#[test]
fn an_open_order_follows_the_gap_while_a_hired_one_is_frozen() {
    let (config, mut state) = fresh(42);
    state.ships.retain(|s| s.faction_id != "中国"); // 没有运力 ⇒ 挂单 = 整条线的要求运力
    state.depots.clear();
    state.contracts.contracts.clear();
    let post = |state: &mut State, stock: f64| {
        // 清空首都池 ⇒ 只会有**出口腿**（否则簿子上同时有进口单，`len` 就不是 1 了）。
        export_only(state, "中国", &[], 0.0);
        if stock > 0.0 {
            state.depot_add("中国", "冥王星", "碳", stock);
        }
        post_contracts(state, &config, &mut RoundSink::default());
    };
    post(&mut state, 1000.0);
    assert_eq!(state.contracts.contracts.len(), 1, "一条腿一张单");
    let id = state.contracts.contracts[0].id;
    let capacity = state.contracts.contracts[0].capacity;
    assert!(capacity > 0.0, "该挂出这条线的要求运力");
    assert_eq!(
        state.contracts.contracts[0].resource, "碳",
        "主货种 = 积压最多的那种"
    );

    // 积压变小/变大 ⇒ **同一张单**（运力要求与积压量无关，所以这里该一个字都不变）。
    post(&mut state, 400.0);
    assert_eq!(
        state.contracts.contracts.len(),
        1,
        "仍是同一张单，不是第二张"
    );
    assert_eq!(
        state.contracts.contracts[0].id, id,
        "单号不变（改数不是新单）"
    );

    // 有人接了 ⇒ 冻结：此后货栈怎么变都不再改这张单（它已经是**承诺**）。
    state.contracts.contracts[0].carrier = Some("美国".into());
    state.contracts.contracts[0].accepted_round = Some(0);
    state.contracts.contracts[0].expires_round = 99;
    post(&mut state, 100.0);
    assert!(
        (state.contracts.contracts[0].capacity - capacity).abs() < 1e-9,
        "已接单的合同冻结，实为 {}",
        state.contracts.contracts[0].capacity
    );
    assert_eq!(state.contracts.contracts.len(), 1, "有人接了就不再开第二张");

    // 没人接 + 货没了 ⇒ **撤单**（需求信号必须跟着现实走，哪怕现实是「没货了」）。
    state.contracts.contracts[0].carrier = None;
    state.contracts.contracts[0].accepted_round = None;
    post(&mut state, 0.0);
    assert!(
        state.contracts.contracts.is_empty(),
        "货栈空了 ⇒ 未接单的该撤回"
    );
}

/// **没人接 ⇒ 每过一个考核周期抬一档抽成**（用户裁决：价格做成**动态平衡**）。
///
/// 抬价的节拍就是**这条线的一个往返**（与受雇方的验货节拍同一把尺子），不是另设一个
/// 「多久没人接就加价」的数；抬到 `share_max` 就不再加。**已接单的冻结**。
#[test]
fn an_unaccepted_order_escalates_once_per_review_period() {
    let (config, mut state) = fresh(42);
    let open = state.contracts.post(
        "中国".into(),
        "碳".into(),
        3.0,
        "金星".into(),
        "地球".into(),
        config.freight.share,
        0,
        0.6,
    );
    let taken = state.contracts.post(
        "中国".into(),
        "铁".into(),
        3.0,
        "水星".into(),
        "地球".into(),
        config.freight.share,
        0,
        0.6,
    );
    {
        let c = state.contracts.get_mut(taken).unwrap();
        c.carrier = Some("美国".into());
        c.accepted_round = Some(0);
        c.expires_round = 99;
    }
    let interval = crate::model::hire_terms(&state, &config, "金星", "地球").interval;
    assert!(interval >= 1, "考核周期至少一回合");
    let share0 = state.contracts.get(open).unwrap().share;
    // 还没过一个周期 ⇒ 不加价（单子该有机会在开叫价上被接走）。
    for r in 0..interval {
        state.round = r;
        escalate_open_contracts(&mut state, &config);
    }
    assert_eq!(
        state.contracts.get(open).unwrap().share,
        share0,
        "一个考核周期之内不该加价"
    );
    // 满一个周期 ⇒ 抬一档，并把叫价起点挪到本回合。
    state.round = interval;
    escalate_open_contracts(&mut state, &config);
    let c = state
        .contracts
        .get(open)
        .expect("没人接的单**留在簿上**继续叫价");
    assert!(
        (c.share - share0 * config.freight.share_escalation).abs() < 1e-9,
        "满一个周期该抬一档：{share0:.3} → {:.3}",
        c.share
    );
    assert_eq!(
        c.posted_round, state.round,
        "抬价后重新起叫（下一档要再等一个完整周期）"
    );
    assert_eq!(
        state.contracts.get(taken).unwrap().share,
        share0,
        "已接单的合同抽成**冻结**（那是承诺）"
    );
    // 反复过期 ⇒ 抬到上限为止。
    for _ in 0..40 {
        state.round += 1000;
        escalate_open_contracts(&mut state, &config);
    }
    let c = state.contracts.get(open).unwrap();
    assert!(
        (c.share - config.freight.share_max).abs() < 1e-9,
        "抬价有上限（{:.2}）：实为 {:.3}",
        config.freight.share_max,
        c.share
    );
    assert!(
        state.contracts.get(taken).is_some(),
        "已接单的合同不会因为加价被动过"
    );
}

/// **AI 端到端**：一条船都没有 ⇒ 把整条线的**要求运力**挂到雇佣市场上，并发一条事件。
#[test]
fn the_ai_posts_an_order_for_the_capacity_it_cannot_cover() {
    let (config, mut state) = fresh(42);
    // 只留出口腿（清空首都池 ⇒ 不开进口腿），这样「中国的单」只有一张。
    export_only(&mut state, "中国", &["冥王星"], 100.0);
    state.ships.retain(|s| s.faction_id != "中国"); // 中国没有舰 ⇒ 自有运力 0
    let mut rng = crate::prng::Prng::new(42);
    sim::advance(&mut state, &config, &mut rng);
    // 挂出来的那张单**可能已经在本回合被接走**（撮合与派工都在 `step_contracts` 里）——
    // 所以要看的是「簿上那张属于中国、跑冥王星那条出口腿的单」，而不是「还没人接的单」。
    // 这一回合世界还在产货，所以池子里也会开出**进口腿**的单子（那是新语义下正确的事，
    // 见 `a_shipless_faction_hires_carriers_for_both_directions`），这里只认出口那条。
    let mine: Vec<&crate::model::Contract> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.shipper == "中国" && c.from == "冥王星")
        .collect();
    assert_eq!(
        mine.len(),
        1,
        "一条腿一张单，实为 {:?}",
        state.contracts.contracts
    );
    let need =
        crate::model::required_throughput(&state, &config, "冥王星", &state.capital_body("中国"));
    assert!(
        (mine[0].capacity - need).abs() < 1e-9,
        "没有运力 ⇒ 该挂整条线的要求运力（应挂 {need:.3}，实为 {:.3}）",
        mine[0].capacity
    );
    assert_eq!(mine[0].from, "冥王星", "起运 = 产地货栈");
    assert_eq!(
        mine[0].to,
        state.capital_body("中国"),
        "目的照公理 = 雇主首都"
    );
    assert!(
        (mine[0].share - config.freight.share).abs() < 1e-12,
        "抽成 = 配置里的费率"
    );
    assert!(mine[0].min_reputation > 0.0, "门槛要在挂单时算好并冻结");
    assert!(
        state
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::ContractPosted { .. })),
        "挂单要发事件（否则投影/故事板里这件事不存在）"
    );
}

/// **没船的势力靠承包商运货**（用户裁决：「不是有运输承包商吗」）——这是「完全禁止瞬移」
/// 之下**唯一**的应急通路，所以它必须是**两个方向**都通的：
///
/// * **集货**：产地货栈的净剩余 → 自己的首都（`from` = 产地）；
/// * **补给**：自己首都的货 → 自己缺料的站点（`from` = 首都）。
///
/// 前者本来就有，后者是本轮新增的（`freight::lanes` 的两个方向 + `open_mut` 按整条腿索引）。
/// 一个**一条船都没有**的势力自己顶不上任何运力 ⇒ 两条腿的缺口都是「整条线的要求运力」，
/// 于是别的势力（有船的那些）接单、派自己的船去跑。**没有第二条通路**：它自己的船是 0，
/// 库存又只能在本天体上花。
#[test]
fn a_shipless_faction_hires_carriers_for_both_directions() {
    let (config, mut state) = fresh(42);
    state.ships.retain(|s| s.faction_id != "中国"); // 中国一条船都没有
    // 造一处**没人搬得走**的净剩余（本势力没有城的天体 ⇒ 没有建设需求、保留量为 0）：
    // 自己没船 ⇒ 集货腿的缺口也是「整条线的要求运力」，只能请人。
    state.depot_add("中国", "冥王星", "碳", 100.0);
    let mut rng = crate::prng::Prng::new(42);
    // 跑十几回合：站点缺的模块料一直等不来（补给腿成立），而请来的人真的把货运过去了。
    for _ in 0..12 {
        sim::advance(&mut state, &config, &mut rng);
    }
    let cap = state.capital_body("中国");
    let mine: Vec<crate::model::Contract> = state
        .contracts
        .contracts
        .iter()
        .filter(|c| c.shipper == "中国")
        .cloned()
        .collect();
    // 两个方向都挂上了单，而且**都被人接了**（簿子上留着已接单的合同）。
    let outbound: Vec<&crate::model::Contract> = mine.iter().filter(|c| c.to == cap).collect();
    let inbound: Vec<&crate::model::Contract> = mine.iter().filter(|c| c.from == cap).collect();
    assert!(!outbound.is_empty(), "集货腿该挂单，实为 {mine:?}");
    assert!(
        !inbound.is_empty(),
        "补给腿也该挂单（首都 → 缺料的站点），实为 {mine:?}"
    );
    assert!(
        mine.iter().any(|c| c.is_hired()),
        "没船的势力请的人该有人接（承包商就是这条通路），实为 {mine:?}"
    );
    // 接单的确实是**别人**（自己没船），派来的是受雇方自己的船，而且**真的把货运到了**。
    // ⚠ 「每张已接单都派着船」**不是**不变量：受雇方可能这一回合刚接、或船被抽回去打仗
    //（`assign_hired_ships` 每回合按缺口重派）——所以看的是「这条通路通不通」。
    let mut delivered = 0.0;
    let mut staffed = 0usize;
    for c in mine.iter().filter(|c| c.is_hired()) {
        let carrier = c.carrier.clone().expect("已接单必有承运方");
        assert_ne!(carrier, "中国", "自己没船，不可能是自己接的");
        if !state.contracts.ships_of(c.id).is_empty() {
            staffed += 1;
        }
        delivered += c.delivered;
    }
    assert!(staffed > 0, "受雇方该真的派自己的船来跑（实为 0 条）");
    assert!(
        delivered > 0.0,
        "承包商的船该真的把货运到（「没船就雇人」这条通路得是**通**的），实为 {delivered:.2}"
    );
}

/// **挂单是确定性的**：同一个世界跑两次，挂出来的单子逐字相同。
///
/// 这条是 `AGENTS.md` 那条纪律的守卫：新机制**绝不消费主 `Prng` 流**——挂单用的是纯
/// 公式（连派生骰子都没用），所以「多挂一张单」不会改变世界后续的掷骰。
///
/// 布景要**明确造出缺口**（把一个势力的船全撤走）：默认开局里各家舰队基本都能顶上自己
/// 那几处货栈，一回合下来往往一张单都不挂——那样这条守卫就是空转的。
#[test]
fn posting_the_same_world_twice_yields_the_same_orders() {
    let (config, state0) = fresh(42);
    let run = || {
        let mut state = state0.clone();
        state.ships.retain(|s| s.faction_id != "中国"); // 中国没有船 ⇒ 必然要请人
        let mut rng = crate::prng::Prng::new(42);
        sim::advance(&mut state, &config, &mut rng);
        state
            .contracts
            .contracts
            .iter()
            .map(|c| {
                (
                    c.id,
                    c.shipper.clone(),
                    c.from.clone(),
                    c.capacity,
                    c.share,
                    c.min_reputation,
                )
            })
            .collect::<Vec<_>>()
    };
    let a = run();
    let b = run();
    assert_eq!(a, b, "同种子同回合的挂单必须逐字相同");
    assert!(
        !a.is_empty(),
        "造了缺口就该有单子可测（否则这条守卫是空转的）"
    );
}
/// 【探针·思潮→角色】逐思潮打印：倾向倍数、目标头数、平均头数、换岗率、头数分布。
/// 跑法：`cargo test --lib probe_ideology_roles -- --ignored --nocapture`。
#[test]
#[ignore]
fn probe_ideology_roles() {
    let (config, base) = fresh(42);
    println!("--- 思潮 → 集货倾向（配额 4 处货栈、12 艘舰、200 回合）---");
    for (tag, mil, col) in [
        ("军国 +1", 1.0, 0.0),
        ("偏军国 +0.5", 0.5, 0.0),
        ("中庸  0", 0.0, 0.0),
        ("偏和平 -0.5", -0.5, 0.0),
        ("和平 -1", -1.0, 0.0),
        ("殖民 +1", 0.0, 1.0),
        ("军国+殖民", 1.0, 1.0),
    ] {
        let mut st = base.clone();
        set_ideology(&mut st, "中国", mil, col);
        grow_fleet(&mut st, "中国", 3);
        st.depots.clear();
        for b in ["金星", "水星", "火星", "木星"] {
            st.depot_add("中国", b, "碳", 100.0);
        }
        let lean = freight_lean(&st, "中国");
        let quota = freighter_quota(&st, &config, "中国");
        let hist = run_roles(&mut st, &config, "中国", 200);
        let mean = hist.iter().map(|r| r.len() as f64).sum::<f64>() / hist.len() as f64;
        let churn: usize = hist
            .windows(2)
            .map(|w| w[1].iter().filter(|n| !w[0].contains(n)).count())
            .sum();
        let mut dist = std::collections::BTreeMap::<usize, usize>::new();
        for r in &hist {
            *dist.entry(r.len()).or_insert(0) += 1;
        }
        println!(
            "{tag:>14}: lean={lean:.3} 配额={quota:.2} 平均头数={mean:.2} 换岗={:.3}/回合 分布={dist:?}",
            churn as f64 / (hist.len() - 1) as f64
        );
    }
    println!("--- 单处货栈（需求 1）：第一艘运输舰要等几回合 ---");
    for (tag, mil, col) in [
        ("军国 +1", 1.0, 0.0),
        ("中庸 0", 0.0, 0.0),
        ("和平 -1", -1.0, 0.0),
    ] {
        let mut st = base.clone();
        set_ideology(&mut st, "中国", mil, col);
        grow_fleet(&mut st, "中国", 3);
        st.depots.clear();
        st.depot_add("中国", "金星", "碳", 100.0);
        let mut waited = 0;
        for _ in 0..200 {
            st.round += 1;
            assign_roles(&mut st, &config, &mut crate::model::RoundInputs::default());
            waited += 1;
            if !roster(&st, "中国").is_empty() {
                break;
            }
        }
        println!("{tag:>14}: 第 {waited} 回合出现第一条运输舰");
    }
    println!("--- 单处货栈清空之后：名额收回要几回合 ---");
    for (tag, mil, col) in [
        ("军国 +1", 1.0, 0.0),
        ("中庸 0", 0.0, 0.0),
        ("和平 -1", -1.0, 0.0),
    ] {
        let mut st = base.clone();
        set_ideology(&mut st, "中国", mil, col);
        grow_fleet(&mut st, "中国", 3);
        st.depots.clear();
        st.depot_add("中国", "金星", "碳", 100.0);
        for _ in 0..40 {
            st.round += 1;
            assign_roles(&mut st, &config, &mut crate::model::RoundInputs::default());
        }
        let before = roster(&st, "中国").len();
        st.depots.clear();
        let mut waited = 0;
        for _ in 0..200 {
            st.round += 1;
            assign_roles(&mut st, &config, &mut crate::model::RoundInputs::default());
            waited += 1;
            if roster(&st, "中国").is_empty() {
                break;
            }
        }
        println!("{tag:>14}: 清空前 {before} 条 ⇒ 第 {waited} 回合清空");
    }
}
