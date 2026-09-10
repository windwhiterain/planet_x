//! 钱去哪了：B2 批——把「批了为什么没花 / 我的船为什么在掉血 / 造舰慢是缺钱还是缺产能」
//! 从**算完就扔**变成读面。
//!
//! 清单与批次见 `.agents/notes/step-intermediates.md` §6（B2）。规矩同 B1（`pre-post-unify.md`）：
//! 观测与过程**同处一行**、纯追加（行为中性 ⇒ digest 逐字不变）、「这一步没跑」用中性缺省**显式**
//! 表达。
//!
//! 这一批的关键约定有一条与 B1 不同，专门钉在下面的用例里：**「批了多少」不进读面**——限额是
//! 控制面的持久叶（`control` 的 `investment_budget`/`construction_budget`），`RoundView` 只记
//! 「真花掉的」，两者相减才是「批了却没花掉的那部分」（同一个数不存两处）。

use super::*;

/// **维护欠费会锈船，而锈的比例是读面唯一能拿到的东西**（锈到 0 才发事件）。
///
/// 构造：把某势力的库存精确地改到「只够付一半维护费」⇒ 欠费 = 一半 ⇒ 每艘舰掉
/// `hull_max × frac` 的船体。这里钉住三件事：
/// 1. `upkeep_unpaid` = 维护费 − 库存价值（它就是生锈的分子）；
/// 2. `fleet_rust` 就是**实际应用**的那个比例（含引擎的可见性下限，欠得少时反而更大）；
/// 3. 每艘舰真的掉了这么多——读面这两个数说的是真事，不是「大概如此」。
#[test]
fn upkeep_shortfall_records_the_unpaid_part_and_the_rust_it_causes() {
    let (config, mut state) = fresh_world(42);
    let fid = state.factions[0].name.clone();

    // 满血 + 库存砍到半价：欠费 = 维护费的一半（远高于 0.2 的可见性下限，所以读到的就是它）。
    let mut upkeep_total = 0.0;
    let mut hull_max: Vec<(String, f64)> = Vec::new();
    for s in state.ships.iter_mut() {
        if s.faction_id != fid || s.hull <= 0.0 {
            continue;
        }
        let p = ship_panel(&config, s);
        upkeep_total += p.upkeep;
        s.hull = p.hull_max;
        hull_max.push((s.name.clone(), p.hull_max));
    }
    assert!(upkeep_total > 0.0, "这条守卫要求 {fid} 开局有舰队");
    let rt = state
        .faction(&fid)
        .and_then(|f| f.resources.keys().next().cloned())
        .expect("势力总有库存键");
    let value = config.resources.get(&rt).map(|r| r.value).unwrap_or(1.0);
    let half = upkeep_total / 2.0;
    if let Some(f) = state.faction_mut(&fid) {
        f.resources.clear();
        f.resources.insert(rt.clone(), half / value);
    }

    let mut sink = RoundSink::default();
    step_upkeep(&mut state, &config, &mut sink);

    let up = sink.upkeep.get(&fid).expect("step_upkeep 必须记这一格");
    assert!(
        (up.total - upkeep_total).abs() < 1e-9,
        "{fid} 的维护费 {upkeep_total} 与实际记的 {} 对不上",
        up.total
    );
    assert!(
        (up.unpaid - half).abs() < 1e-6,
        "{fid} 欠费应为库存的一半 {half}，实际 {}",
        up.unpaid
    );
    let frac = up.unpaid / up.total;
    assert!(frac > 0.2, "用例前提：欠费比例 {frac} 要高于可见性下限 0.2");
    assert!(
        (up.rust - frac).abs() < 1e-12,
        "{fid} 生锈比例应为 {frac}（欠费/维护费），实际 {}",
        up.rust
    );

    // 每艘舰掉的船体 = hull_max × rust。**这就是「我的船为什么在掉血」的读法**。
    for (name, hmax) in &hull_max {
        let s = state
            .ships
            .iter()
            .find(|s| &s.name == name)
            .expect("舰还在（只锈了一半）");
        let expected = hmax * up.rust;
        assert!(
            (s.hull - (hmax - expected)).abs() < 1e-6,
            "{name}: 船体应为 {hmax} − {expected}，实际 {}",
            s.hull
        );
    }

    // 折进视图以后是同一份数（「同一个量只有一个位置」）。
    let view = observe(&state, &config, &sink);
    let row = view.factions.get(&fid).expect("势力行");
    assert_eq!(row.upkeep_unpaid, up.unpaid);
    assert_eq!(row.fleet_rust, up.rust);
    assert!(
        row.fleet_rust > 0.0,
        "掉血必须在读面上看得见——锈到 0 之前没有任何事件"
    );
}

/// **花掉的钱不超过批的额度**——这是 B2 那条「限额在控制面、已花在读面」的**可检查形式**。
///
/// `view.factions[].investment_spent` 是「真花掉的」，`control` 的 `investment_budget` 是「批了
/// 多少」；两者的差就是文档里承诺的「批了却没花掉的」。顺带钉住：这份差**不小于 0**（引擎不会
/// 超批），以及这一局里真的发生过花钱（否则守卫退化成空转）。
///
/// ⚠ 必须跑几回合**再**看：开局那一回合既没有在建的建筑、也没有攒到启封的造舰进度，所以谁都
/// 没花钱——拿回合 0 当样本会让这条守卫变成「空表比空表」。
#[test]
fn spent_never_exceeds_the_batch_and_the_gap_is_the_unspent_part() {
    let (config, mut state) = fresh_world(42);
    let mut rng = Prng::new(42);
    let mut view = view_from_state(&state, &config);
    for _ in 0..8 {
        view = advance(&mut state, &config, &mut rng);
    }

    let mut flowed = 0usize;
    let mut unspent_seen = 0usize;
    for f in &state.factions {
        let fid = f.name.clone();
        let row = view.factions.get(&fid).expect("势力行");
        let ctl = state.control.get(&fid).expect("势力总有控制面");

        for (kind, spent, limits) in [
            (
                "investment_budget",
                &row.investment_spent,
                &ctl.investment_budget,
            ),
            (
                "construction_budget",
                &row.construction_spent,
                &ctl.construction_budget,
            ),
        ] {
            for (rt, amt) in spent {
                let limit = limits.get(rt).map(|l| l.value).unwrap_or(0.0);
                assert!(
                    *amt <= limit + 1e-9,
                    "{fid} 的 {kind} 在 {rt} 上花掉了 {amt}，超过了批的 {limit}"
                );
                if *amt > 0.0 {
                    flowed += 1;
                }
            }
            // 「没花掉的」= 限额 − 已花：文档就是这么承诺的，所以这里顺便要求它真的 ≥ 0
            // 且至少在一个资源上为正（全花光说明预算不是约束，用例就没在检查东西）。
            for (rt, leaf) in limits.iter() {
                let amt = spent.get(rt).copied().unwrap_or(0.0);
                assert!(
                    leaf.value - amt >= -1e-9,
                    "{fid} 的 {kind}.{rt} 出现了负余额"
                );
                if leaf.value - amt > 1e-9 {
                    unspent_seen += 1;
                }
            }
        }
    }
    assert!(flowed >= 1, "8 回合里没有任何势力真的花过钱——守卫太空");
    assert!(
        unspent_seen >= 1,
        "所有额度都被花光了——「批了没花掉」这条读法没被检查到"
    );
}

/// **造舰慢是缺钱还是缺产能**：`build.<舰级>.rate` 是产能上限，`increment` 是实得进度。
///
/// 用例把两个极端都造出来（同一把预算尺子，只改钱）：
/// * 批满 ⇒ `increment ≈ rate`（产能封顶）；
/// * 批 0 ⇒ 仍然有 `rate`（产能摆在那儿）而 `increment = 0`（**一分钱没批到**）。
///
/// 第二条正是这一列必须存在的理由：没有它，「这个船坞这个月为什么一艘没造」与「这个城根本没
/// 这个舰级的建造区」在读面上长得一模一样。
#[test]
fn build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling() {
    let (config, mut state) = fresh_world(42);
    let fid = state.factions[0].name.clone();
    // 找一个真有建造区的城，并把它的舰级钉死（免得自动控制中途改装把它换掉）。
    let (cid, bid, class) = state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid && !c.razed)
        .find_map(|c| {
            c.buildings.iter().find(|b| b.is_shipyard()).map(|b| {
                (
                    c.name.clone(),
                    b.id,
                    b.ship_type.clone().unwrap_or_else(|| "护卫舰".to_string()),
                )
            })
        })
        .expect("中国开局应有建造区");
    if let Some(city) = state.city_mut(&cid) {
        for b in city.buildings.iter_mut() {
            if b.id == bid {
                b.blueprint = None;
                b.ship_type = Some(class.clone());
            }
        }
    }

    // 全资源预算钉成一个值（玩家叶 ⇒ `read_budget` 原样取用）。
    let keys: Vec<String> = state
        .faction(&fid)
        .map(|f| f.resources.keys().cloned().collect())
        .unwrap_or_default();
    assert!(!keys.is_empty(), "势力总有库存键");
    let set_budget = |state: &mut State, v: f64| {
        let c = state.control.entry(fid.clone()).or_default();
        for rt in &keys {
            c.construction_budget.insert(rt.clone(), Control::player(v));
            c.investment_budget.insert(rt.clone(), Control::player(v));
        }
    };

    // —— 批满：进度应当顶到产能上限 ——
    // ⚠ 两次跑必须从**同一份** state 出发：`step_construction` 末尾的 `retool_shipyards` 会按战况
    // 改装舰级（把上面钉死的那个舰级换掉），拿跑过的 state 再跑一遍就找不到那条建造行了。
    let base = state.clone();
    set_budget(&mut state, 1e6);
    let mut rng = Prng::new(7);
    let mut rich = RoundSink::default();
    step_construction(&mut state, &config, &mut rng, &mut rich);
    let rich_line = rich
        .city_flow
        .get(&cid)
        .and_then(|f| f.build.get(&class))
        .unwrap_or_else(|| panic!("{cid} 应有 {class} 的建造行（它有建造区）"))
        .clone();
    assert!(rich_line.rate > 0.0, "{cid} 的 {class} 产能不该是 0");
    assert!(
        rich_line.increment > 0.0,
        "批了 1e6 却一点进度都没有——用例构造失败了"
    );
    assert!(
        (rich_line.increment - rich_line.rate).abs() < 1e-9,
        "钱管够时进度应当顶到产能上限（rate={} increment={}）",
        rich_line.rate,
        rich_line.increment
    );

    // —— 批 0：产能还在，进度归零 ——
    let mut state2 = base;
    set_budget(&mut state2, 0.0);
    let mut rng = Prng::new(7);
    let mut poor = RoundSink::default();
    step_construction(&mut state2, &config, &mut rng, &mut poor);
    let poor_line = poor
        .city_flow
        .get(&cid)
        .and_then(|f| f.build.get(&class))
        .unwrap_or_else(|| {
            panic!("批 0 时 {cid} 的建造行**仍然必须在**（那是「没钱」而不是「没船坞」）")
        })
        .clone();
    assert_eq!(poor_line.increment, 0.0, "批 0 就不该有进度");
    assert_eq!(
        poor_line.rate, rich_line.rate,
        "产能与钱无关：同一座城同一舰级的 rate 不该因为预算变了而变"
    );
    assert!(
        poor.spend
            .get(&fid)
            .map(|s| s.investment.is_empty())
            .unwrap_or(false)
            || poor
                .spend
                .get(&fid)
                .map(|s| s.construction.values().all(|v| *v <= 0.0))
                .unwrap_or(false),
        "批 0 时不该有花销（花销表要么空、要么全是 0）"
    );

    // 折进视图：稀疏 map 里的键就是「这个城有这个舰级的建造区」。
    let view = observe(&state2, &config, &poor);
    let row = view.cities.get(&cid).expect("活城");
    assert!(
        row.build.contains_key(&class),
        "{cid} 的 build 表里必须有 {class}——有键而 increment=0 才是「有产能没批到钱」"
    );
}

/// **用工系数与住房容量**是「这座城产量为什么低 / 人口为什么不涨」的两个答案，且都只由
/// **这一步用的** state 算出来（不是事后重算的近似值）。
///
/// 用工系数取的是人口增长**之前**的人口，所以用例从两边夹：把人口压到 1 ⇒ 必然掉到
/// `min_efficiency` 下限；把住房加满不缺人 ⇒ 必然是 1.0。住房容量则按引擎的定义
/// （住宅面积 × 生态容量）在测试里重算一遍对账。
#[test]
fn labor_and_housing_capacity_are_captured_from_this_steps_state() {
    let (config, mut state) = fresh_world(42);
    let fid = state.factions[0].name.clone();
    let cid = state
        .cities
        .iter()
        .find(|c| c.faction_id == fid && !c.razed)
        .map(|c| c.name.clone())
        .expect("中国开局应有城");

    // —— 人手极度不足：人口 1 ⇒ 用工系数掉到下限 ——
    if let Some(c) = state.city_mut(&cid) {
        c.population = 1;
    }
    let mut sink = RoundSink::default();
    step_production(&mut state, &config, &mut sink);
    let cf = sink
        .city_flow
        .get(&cid)
        .expect("step_production 必须记这一格");
    assert!(
        (cf.labor - config.economy.min_efficiency).abs() < 1e-12,
        "{cid}: 人口压到 1 之后用工系数应当是下限 {}，实际 {}",
        config.economy.min_efficiency,
        cf.labor
    );
    assert!(
        (0.0..=1.0).contains(&cf.labor),
        "{cid} 的用工系数越界：{}",
        cf.labor
    );

    // —— 住房容量 = 住宅面积 × 生态容量（照引擎的定义重算对账）——
    let (ecocap, want_housing) = {
        let ecocap = state
            .city_settlement(&cid)
            .map(|s| s.ecological_capacity)
            .unwrap_or(0.0);
        let area: f64 = state
            .city(&cid)
            .expect("刚查过")
            .buildings
            .iter()
            .filter(|b| config.building_spec(&b.kind).role == "housing")
            .map(|b| b.deployed * building_health(b, &config))
            .sum();
        (ecocap, area * ecocap)
    };
    assert!(
        cf.housing_capacity > 0.0,
        "{cid} 开局应有住宅（否则这条对账没意义）"
    );
    assert!(
        (cf.housing_capacity - want_housing).abs() < 1e-6,
        "{cid}: 住房容量 {} ≠ 住宅面积 × 生态容量 {want_housing}（生态容量 {ecocap}）",
        cf.housing_capacity
    );

    // 折进视图以后是同一个数；`pre` 面（没跑这一步）用工系数是**中性值 1.0**，不是 0。
    let view = observe(&state, &config, &sink);
    let row = view.cities.get(&cid).expect("活城");
    assert_eq!(row.labor, cf.labor);
    assert_eq!(row.housing_capacity, cf.housing_capacity);
    let pre = view_from_state(&state, &config);
    for (cid, c) in &pre.cities {
        assert_eq!(
            c.labor, 1.0,
            "{cid}: 「这一步还没跑」的用工系数中性值是 1.0（不缺人手），不是 0（全城没人上工）"
        );
        assert_eq!(c.housing_capacity, 0.0);
        assert!(!c.is_hub, "{cid}: pre 面里入库路径还没定 ⇒ 中性值 false");
        assert!(c.build.is_empty(), "{cid}: pre 面里还没有造舰进度");
    }
}

/// **集散地（`is_hub`）就是首都天体上的城**：产出直进势力池 vs 先落产地货栈，两条路的分岔点。
///
/// 「我挖出来的矿为什么用不了」= 那些矿躺在**非首都**城的货栈里等船——读面此前只给开采量，
/// 不分入库路径。这里钉住它与 `capital_body` 的对应关系（引擎的权威定义）。
#[test]
fn is_hub_matches_the_capital_body() {
    let (config, mut state) = fresh_world(42);
    let mut sink = RoundSink::default();
    step_production(&mut state, &config, &mut sink);
    let view = observe(&state, &config, &sink);

    let mut hubs = 0usize;
    let mut non_hubs = 0usize;
    for f in &state.factions {
        let fid = f.name.clone();
        let cap = state.capital_body(&fid);
        for c in state
            .cities
            .iter()
            .filter(|c| c.faction_id == fid && !c.razed)
        {
            let row = view.cities.get(&c.name).expect("活城");
            let want = c.body_id == cap;
            assert_eq!(
                row.is_hub, want,
                "{}: is_hub={} 但首都天体是 {cap}、它自己在天体 {}",
                c.name, row.is_hub, c.body_id
            );
            if want {
                hubs += 1;
            } else {
                non_hubs += 1;
            }
        }
    }
    assert!(hubs >= 1, "至少要有势力把首都放在自己有城的天体上");
    assert!(
        non_hubs >= 1,
        "至少要有一座非首都城——否则「产出先落产地货栈」这条路没被检查到"
    );
}
