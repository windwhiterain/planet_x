//! 集货运输：产地货栈、货舱容量、`haul_split` 的 max-min 公平、货权守恒、承包分账、腿别交替、指令路线入首都池。
//!
//! ## 2026-10：`cargo_capacity` 的**数据级**那一半搬去了 `play/tests/g2_mid.py`
//!
//! `ships` 表加了 `载货`（在舱货物）与 `cargo_capacity`（有效舱容，派生列）两列 ⇒ 原来那条
//! `cargo_capacity_is_class_capacity_times_hull_fraction` 里的
//!
//! * 「有效舱容 = 舰级舱容 × 战损折算 `船体/船体上限`」与「舰级舱容是设计裁决」，现在在
//!   **3 seed × 400 回合的每一行舰**上成立（g2 的 `cargo_checks`；实测 **16,504 个舰·回合**、
//!   其中 **1,522 行**受过伤、**4,164 行**舱里有货）；
//! * 剩下两个**手工边界**（壳打光 ⇒ 舱容 0、`hull_max ≤ 0` 的旧档 ⇒ 满舱）在真实投影里
//!   **不发生**（活舰 `船体 > 0`、所有档都有 `船体上限`）⇒ 现在走 `planet_x --call cargo_capacity`
//!   （g1 的 `call_functions`，调的还是引擎同一份实现）。
//!
//! ## 2026-10：`depots` 表落地，`off_capital_production_...` 的**读面**那一半也搬去了 g2
//!
//! `depots`（逐势力 × 天体 × 资源）现在是 `--index` 的一张派生表 ⇒ g2 的 `depot_checks`
//! 在 **3 seed × 400 回合**上钉住：**首都天体上不许凭空出现货栈**（迁都留下的 `stale` /
//! 首都本回合才搬来的 `same_round` 两类**逐处解释**，§12 相位错位）、`cities.depot_value`
//! ≡ Σ 货栈 × 资源价值（**118,779 行货栈 / 5,032 个积压城·回合**全对）。
//! 原来那条用例里**只有 `step_production` 隔离**（把运输那一步拿掉）才看得见的
//! 「**池子不因离岸产出增长**」留在下面——真投影里 haul 就在同一回合后面跑，池子会被运回来的货填上。

use super::*;

/// **产地货栈的「池子不动」那一半**：`step_production` **隔离**（跑这一步、不跑运输）。
///
/// 「离岸产出落在货栈」那一半现在能在**跑出来的数据**上成立（g2 的 `depot_checks`：`depots`
/// 表 ↔ `cities.depot_value`，3 seed × 400 回合 118,779 行货栈 / 5,032 个积压城·回合全对）；
/// 但「池子**不**因此增长」必须把运输那一步**拿掉**才看得见——真投影里 haul 就在同一回合后面跑，
/// 池子会被运回来的货填上。所以这条留在纯 step 侧。
#[test]
fn off_capital_production_does_not_touch_the_pool() {
    let (config, mut state) = fresh_world(42);
    assert_eq!(state.capital_body("中国"), "地球", "用例前提：中国首都在地球");
    assert_eq!(
        state.capital_body("无国界科学组织"),
        "木星",
        "用例前提：科学组织首都在木星"
    );

    let mut flow = RoundSink::default();
    let cn_silicon = |s: &State| {
        s.faction("中国")
            .unwrap()
            .resources
            .get("硅")
            .copied()
            .unwrap_or(0.0)
    };
    let cn_carbon = |s: &State| {
        s.faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0)
    };
    let sci_value = |s: &State| {
        let value_of = |rt: &str| config.resources.get(rt).map(|r| r.value).unwrap_or(1.0);
        s.faction("无国界科学组织")
            .unwrap()
            .resources
            .iter()
            .map(|(rt, amt)| amt * value_of(rt))
            .sum::<f64>()
    };

    let (si0, c0, sci0) = (cn_silicon(&state), cn_carbon(&state), sci_value(&state));
    step_production(&mut state, &config, &mut flow);

    // 中国：首都（地球）的硅直接进池；碳只从金星（非首都）出，所以池子里的碳一格都不该动。
    assert!(cn_silicon(&state) > si0, "地球（首都）上的硅应直接进池");
    assert!(
        (cn_carbon(&state) - c0).abs() < 1e-9,
        "碳只从金星（非首都）出，所以池子里的碳一格都不该动——运输才是货栈的上游"
    );

    // 科学组织：矿全在非首都 → 整批积压，池子不动。
    assert!(
        (sci_value(&state) - sci0).abs() < 1e-9,
        "一个「矿全在非首都天体」的势力，产出会整批积压在产地（= 等船来运）"
    );

    // 再跑一回合（仍然没有运输步）：货栈继续涨、池子仍不因它增长（库存冻结）。
    let carbon_in_depot = state
        .depot("中国", "金星")
        .and_then(|m| m.get("碳").copied())
        .unwrap_or(0.0);
    step_production(&mut state, &config, &mut flow);
    assert!(
        state
            .depot("中国", "金星")
            .and_then(|m| m.get("碳").copied())
            .unwrap_or(0.0)
            > carbon_in_depot,
        "没有船来运 → 货栈继续涨"
    );
    assert!(
        (cn_carbon(&state) - c0).abs() < 1e-9,
        "两回合过去，金星的碳一格都没进池——这就是「等船来运」"
    );
}

/// **货值守恒（M2b 的核心不变量）**：装货与卸货**只搬货**——产地里少多少，舱里就多多少；
/// 舱里清空多少，首都池就多多少。全程「货栈 + 在舱 + 池子」的总量一格不变。
///
/// 这条守卫是这套机制的地基：集货腿的正当性全在「**货不会凭空出现或消失**」上
/// （一旦漏了，缺矿就会像 M1 之前那样被静默补贴掉）。
#[test]
fn hauling_moves_cargo_without_creating_or_destroying_any() {
    let (config, mut state) = fresh_world(42);
    state.depots.clear();
    state.depot_add("中国", "月球", "碳", 7.0);
    state.depot_add("中国", "月球", "铁", 5.0);
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .expect("中国开局有舰")
        .name
        .clone();
    let class = state.ship(&ship).unwrap().class.clone();
    // 停在月球泊位上（`arrival_eps` 之内）。
    let vpos = state.body_position("月球");
    state.ship_mut(&ship).unwrap().position = vpos;
    let total = |s: &State| -> f64 {
        let depot: f64 = s.depots.values().flat_map(|m| m.values()).sum();
        let hold: f64 = s.ships.iter().flat_map(|x| x.cargo.values()).sum();
        let pool: f64 = s.factions.iter().flat_map(|f| f.resources.values()).sum();
        depot + hold + pool
    };
    let before = total(&state);

    // —— 装货：上限 = 有效舱容，两种货尽量等量 ——
    let step = haul_step(&mut state, &config, &ship, &class, "月球", "地球", &mut crate::model::RoundInputs::default());
    let units = match step {
        HaulStep::Loaded { units, .. } => units,
        other => panic!("停在货栈泊位上应当装货，实为 {other:?}"),
    };
    let cap = cargo_capacity(&config, state.ship(&ship).unwrap());
    assert!(
        (units - cap).abs() < 1e-9,
        "货栈有 12 件、舱容 {cap} ⇒ 装满：实装 {units}"
    );
    let hold: f64 = state.ship(&ship).unwrap().cargo.values().sum();
    assert!((hold - units).abs() < 1e-9, "装了多少就在舱里有多少");
    let left: f64 = state.depot("中国", "月球").unwrap().values().sum();
    assert!(
        (left - (12.0 - units)).abs() < 1e-9,
        "货栈恰好少了装走的那些：剩 {left}"
    );
    assert!((total(&state) - before).abs() < 1e-9, "装货不许造货");

    // —— 卸货：挪到首都（地球）泊位上，货进**势力池** ——
    let epos = state.body_position("地球");
    state.ship_mut(&ship).unwrap().position = epos;
    let carbon_before = state
        .faction("中国")
        .unwrap()
        .resources
        .get("碳")
        .copied()
        .unwrap_or(0.0);
    let step = haul_step(&mut state, &config, &ship, &class, "月球", "地球", &mut crate::model::RoundInputs::default());
    match step {
        HaulStep::Delivered {
            units: u,
            into_pool: true,
            ..
        } => assert!((u - hold).abs() < 1e-9, "整舱卸下"),
        other => panic!("在首都泊位上应当卸进首都池，实为 {other:?}"),
    }
    assert!(state.ship(&ship).unwrap().cargo.is_empty(), "卸完舱就空了");
    assert!(
        state
            .faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0)
            > carbon_before,
        "月球采的碳进了首都池——这就是集货腿的终点"
    );
    assert!((total(&state) - before).abs() < 1e-9, "卸货不许毁货");
}

/// **雇佣交付的记账（Q10 抽成制）**：卸下来的货**分两份**——抽成归受雇方自己的
/// 首都池，余数进**雇主**的池子（不是船东的！）。
///
/// 这一条把「货主与船东分离」这件事钉在最细的粒度上（不跑 `advance`，所以池子不会被
/// 维护费/建造搅浑）：**同一个天体、同一批货，进的是两个不同势力的池子**。
#[test]
fn a_hired_delivery_splits_the_cargo_between_carrier_and_shipper() {
    let (config, mut state) = fresh_world(42);
    let share = config.freight.share;
    state.depots.clear();
    // 雇主：中国在金星积压 10 件碳，**它自己没有船**。
    state.depot_add("中国", "金星", "碳", 10.0);
    // 受雇方：美国的一艘驱逐舰（舱容 4）。
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "美国" && s.class == "destroyer")
        .expect("美国开局有驱逐舰")
        .name
        .clone();
    let class = state.ship(&ship).unwrap().class.clone();
    let id = state.contracts.post(
        "中国".into(),
        "碳".into(),
        3.0,
        "金星".into(),
        "地球".into(),
        share,
        0,
        0.0,
    );
    state.contracts.assign(ship.clone(), id);
    state
        .contracts
        .contracts
        .iter_mut()
        .find(|c| c.id == id)
        .unwrap()
        .carrier = Some("美国".into());
    // 停在**托运方货栈**的泊位上 → 装货该装的是**中国的**货。
    let vpos = state.body_position("金星");
    state.ship_mut(&ship).unwrap().position = vpos;
    let cap = cargo_capacity(&config, state.ship(&ship).unwrap());
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球", &mut crate::model::RoundInputs::default());
    let loaded = match step {
        HaulStep::Loaded { units, .. } => units,
        other => panic!("停在托运方货栈上该装货，实为 {other:?}"),
    };
    assert!(loaded > 0.0 && loaded <= cap, "装的不能超过舱容");
    assert!(
        state.depot("中国", "金星").is_none()
            || (state.depot("中国", "金星").unwrap().values().sum::<f64>() - (10.0 - loaded)).abs()
                < 1e-9,
        "装走的必须是**中国**货栈里的货"
    );
    // 卸到中国的首都（地球）：抽成归美国、余数归中国。
    let (cn0, us0) = (
        state
            .faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
        state
            .faction("美国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
    );
    state.ship_mut(&ship).unwrap().position = state.body_position("地球");
    let step = haul_step(&mut state, &config, &ship, &class, "金星", "地球", &mut crate::model::RoundInputs::default());
    assert!(
        matches!(
            step,
            HaulStep::Delivered {
                into_pool: true,
                ..
            }
        ),
        "目的 = 托运方首都 ⇒ 该进池子，实为 {step:?}"
    );
    let (cn1, us1) = (
        state
            .faction("中国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
        state
            .faction("美国")
            .unwrap()
            .resources
            .get("碳")
            .copied()
            .unwrap_or(0.0),
    );
    let cut = loaded * share;
    assert!(
        (cn1 - cn0 - (loaded - cut)).abs() < 1e-9,
        "托运方该收到 {} 件（装的 {} 减去抽成 {cut:.2}），实收 {:.3}",
        loaded - cut,
        loaded,
        cn1 - cn0
    );
    assert!(
        (us1 - us0 - cut).abs() < 1e-9,
        "承运人的报酬就是它自留的那份货：{cut:.3}，实收 {:.3}",
        us1 - us0
    );
    // 合同进度按**卸出舱的总量**记（抽成是搬运费，不能从运力里扣）。
    let c = state.contracts.get(id).expect("合同还在雇佣期内");
    assert!(
        (c.delivered - loaded).abs() < 1e-9,
        "进度 = 卸出舱的总量 {loaded}，实为 {}",
        c.delivered
    );
}

/// **常驻路线 + 腿别由货舱决定（Q7 A / Q5 A）**：同一对 `from/to`、**不存任何额外状态**，
/// 空舱就去装、装到货就改跑 `to`、货栈空就原地等——三件事全部由「舱里有货吗」推出来。
#[test]
fn a_haul_route_alternates_legs_because_of_the_cargo() {
    let (config, mut state) = fresh_world(42);
    state.depots.clear();
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国")
        .expect("中国开局有舰")
        .name
        .clone();
    let class = state.ship(&ship).unwrap().class.clone();
    let vpos = state.body_position("月球");
    state.ship_mut(&ship).unwrap().position = vpos;

    // 1) 货栈是空的 ⇒ **原地等**（「有货就走」的另一半是「没货就不走」），位置不动。
    let step = haul_step(&mut state, &config, &ship, &class, "月球", "地球", &mut crate::model::RoundInputs::default());
    assert!(
        matches!(step, HaulStep::Waiting { ref body } if body == "月球"),
        "空货栈应当原地等，实为 {step:?}"
    );
    assert_eq!(
        state.ship(&ship).unwrap().position,
        vpos,
        "等的时候不许乱跑"
    );
    assert!(state.ship(&ship).unwrap().cargo.is_empty());

    // 2) 来货了就装，且**这一回合不再跑**（与殖民一样是「到达即行动」）。
    state.depot_add("中国", "月球", "碳", 3.0);
    let step = haul_step(&mut state, &config, &ship, &class, "月球", "地球", &mut crate::model::RoundInputs::default());
    assert!(
        matches!(step, HaulStep::Loaded { .. }),
        "有货就装，实为 {step:?}"
    );
    assert!(!state.ship(&ship).unwrap().cargo.is_empty(), "舱里该有货");

    // 3) 舱里有货 ⇒ 腿别翻到 `to`（哪怕 `from` 还有货）。同一对 from/to、零额外状态。
    let step = haul_step(&mut state, &config, &ship, &class, "月球", "地球", &mut crate::model::RoundInputs::default());
    assert_eq!(
        step.body(),
        "地球",
        "舱里有货 ⇒ 这一腿去卸货端，实为 {step:?}"
    );
    assert!(
        !matches!(step, HaulStep::Waiting { .. } | HaulStep::Loaded { .. }),
        "有货时不该再在装货端打转，实为 {step:?}"
    );
}

/// **端到端（玩家路径）**：给一艘舰写一条 `Haul` 指令，货真的从**产地货栈**走到**首都池**，
/// 而且走的是一条不需要重下的**常驻路线**（两个回合内装完并卸到池里）。
///
/// 用「首都天体上的货栈」把航程压成 0 回合：它本来就是为了「迁都把旧中转点留在新首都」
/// 准备的合法路线（`Haul { from: cap, to: cap }`，见 `behavior_is_valid`），正好也是最短的
/// 端到端用例。
#[test]
fn a_commanded_haul_route_delivers_depot_cargo_into_the_capital_pool() {
    let (config, mut state) = fresh_world(42);
    state.depots.clear();
    state.depot_add("中国", "地球", "碳", 9.0); // 首都是地球；这处货栈压着 9 件碳
    let ship = state
        .ships
        .iter()
        .find(|s| s.faction_id == "中国" && s.class == "destroyer")
        .expect("中国开局有一艘驱逐舰")
        .name
        .clone();
    let epos = state.body_position("地球");
    state.ship_mut(&ship).unwrap().position = epos;
    // 玩家指令（`Player` = 自动控制不碰它）：路线 地球→地球，角色钉成运输舰。
    {
        let c = state.control_mut("中国".to_string()).unwrap();
        c.ship_orders.insert(
            ship.clone(),
            Control::player(ShipBehavior::Haul {
                from: "地球".to_string(),
                to: "地球".to_string(),
            }),
        );
        c.ship_role
            .insert(ship.clone(), Control::player(ShipRole::Freight));
    }
    let mut rng = Prng::new(42);
    let mut delivered = 0.0;
    // 驱逐舰舱容 4、货栈 9 件 ⇒ 要跑三趟（4+4+1）；每趟「装一回合 + 卸一回合」，
    // 所以 8 个回合足够，也正是「常驻路线自己往复」的证据（不需要重下指令）。
    for _ in 0..8 {
        advance(&mut state, &config, &mut rng);
        for e in &state.events {
            if let GameEvent::CargoDelivered {
                cargo,
                into_pool: true,
                ..
            } = e
            {
                delivered += cargo.values().sum::<f64>();
            }
        }
    }
    assert!(
        delivered >= 9.0 - 1e-9,
        "压在货栈里的那 9 件必须整批运进首都池（实为 {delivered}，其中还含金星当期产出的那部分）"
    );
    // 注意：**不要**拿首都池的增量当判据——池子同时在花钱（建设/造舰/维护），
    // 收进来的货是**毛额**、池子的净变化是另一回事。判据用事件（到货的毛额）
    // 与「货栈条目消失」（没有货留在产地）这两条。
    assert!(
        state.depot("中国", "地球").is_none(),
        "空货栈条目要被清掉（AI 派单靠键集合读积压）"
    );
}
