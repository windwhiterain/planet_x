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
//!
//! ## 2026-10：能只看数据的那几条搬去了 Python
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `spent_never_exceeds_the_batch_and_the_gap_is_the_unspent_part` | g1「花掉的 ≤ 批的额度」 | 两个读面各给一半：`factions[].investment_spent`（真花掉的）+ `--control` 的额度（批了多少） |
//! | `is_hub_matches_the_capital_body` | g3「一个回合里只有一个 hub 天体」 | `city_process.is_hub` 与 `capital_body` 都在读面上（且修掉了同回合易主/复垦的相位错位） |
//! | `labor_and_housing_capacity…` 的**用工系数那一半** | g2 **合成场景**「人口压到 1 ⇒ 用工系数掉到 `min_efficiency`」 | 档能存成 JSON（`--save w.json`）⇒ Python 把人口改成 1 再推进，断言读面（连同「回合 0 的中性值是 1.0」） |
//! | `upkeep_shortfall…` 的**读面那一半** | g3「欠费 ⇔ 生锈」「欠费 ≤ 账单」 | 全 7 seed × 1000 回合逐行 |
//! | `build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling` 的**活回合那一半** | g2 **合成场景 · 拨预算**（施工图 §5.6 第 6 批）：「批 0 ⇒ 建造行还在、`rate > 0`、而 `increment = 0`」「批满 ⇒ 顶到产能上限」「`rate` 与钱无关」 | 拨 `investment_budget`/`construction_budget` 走引擎自己的 `--apply`（`h.scenario_apply`），断言只读 `city_process.build`；`increment ≤ rate` 那一半已在 g3 |
//!
//! **留在这里的**：`upkeep_shortfall_records_the_unpaid_part_and_the_rust_it_causes`
//! （要「库存恰好只够付一半」的精确构造，而且「每艘舰真的掉了 `hull_max × rust`」得在**只有锈、
//! 没有再生**的一步里看——合成场景推的是整回合，再生同时发生）与
//! `build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling` 的**另一半**：
//! 同一组断言，但**不依赖开局库存**。
//!
//! ⚠ 为什么「`increment ≈ rate`」两边都留：它在活回合里**只在第一回合**成立——**活回合有第二个
//! 瓶颈（库存）**。g2 那份推一回合（= 开局库存，等于这条用例的一次 `step_construction`，
//! 忠实但脆）；这条用例**直接调** `step_construction` 且国库随便造，所以它管的是「不看库存」
//! 的那份。实测 `main@1ccbb2c`（P1-4 改市场定价之后）批满 1e6 的同一座城：回合 1 是
//! `10.0 == 10.0`，**回合 2 掉到 7.27、回合 3 干脆 0**——推长一点，两个极端就分不出来了。

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

/// **造舰慢是缺钱还是缺产能**：`build.<舰级>.rate` 是产能上限，`increment` 是实得进度。
///
/// 用例把两个极端都造出来（同一把预算尺子，只改钱）：
/// * 批满 ⇒ `increment ≈ rate`（产能封顶）；
/// * 批 0 ⇒ 仍然有 `rate`（产能摆在那儿）而 `increment = 0`（**一分钱没批到**）。
///
/// 第二条正是这一列必须存在的理由：没有它，「这个船坞这个月为什么一艘没造」与「这个城根本没
/// 这个舰级的建造区」在读面上长得一模一样。
///
/// ⚠ **这条用例留在这里，是为了「不看库存」那份**（见模块头）：g2 里那条合成场景版本的
/// `increment ≈ rate` 只在**第一回合**（= 开局库存）读得出来。这里直接调一次
/// `step_construction`、国库随便造 ⇒ 它管的是纯粹的「钱 vs 产能」。
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

    let keys: Vec<String> = config.resources.keys().cloned().collect();
    let set_budget = |state: &mut State, v: f64| {
        // P1-5 之后 Player 的 construction 值会先乘维护 reserve 的 `con_scale`；
        // 这条用例测的是「批满 vs 批 0」，所以要先把库存垫到 reserve 之上。
        if let Some(f) = state.faction_mut(&fid) {
            for rt in &keys {
                f.resources.insert(rt.clone(), 1e6);
            }
        }
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

