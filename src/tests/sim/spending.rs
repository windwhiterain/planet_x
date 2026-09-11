//! 钱去哪了：B2 批——把「批了为什么没花 / 我的船为什么在掉血 / 造舰慢是缺钱还是缺产能」
//!
//! ## 2026-10（第 7 批）：`build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling`
//! 搬去了 g2 **合成场景 · 三条线**（5 条判据）
//!
//! 读面本来就有那两格：`city_process.build[舰级] = {rate, increment}`（schema 原话：
//! **"`increment < rate` ⇒ 钱是瓶颈；`increment ≈ rate` ⇒ 产能封顶"**）；**建造预算**是控制面
//! Player 叶、**库存**是势力 `资源` ⇒ 三条臂全用现成入口造：两头都足 ⇒ `increment = rate`
//! （11.92）；钱批 0 ⇒ 0；库存 0 ⇒ ≈0（**第三条线**）。
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
//! | `build_lines_separate_the_money_bottleneck_from_the_capacity_ceiling` 的**读面那一半** | g2 **合成场景 · 拨预算**（施工图 §5.6 第 6 批）：「批满 ⇒ 顶到产能上限」「批 0 ⇒ 建造行还在、`rate > 0`、而 `increment = 0`」「`rate` 与钱无关」 | 两边都先把国库垫到维护 reserve 之上（P1-5 的 `con_scale`，见下），再拨 `construction_budget`/`investment_budget` 走引擎自己的 `--apply`（`h.scenario_apply`），断言只读 `city_process.build`；`increment ≤ rate` 那一半已在 g3 |
//!
//! **留在这里的**：`upkeep_shortfall_records_the_unpaid_part_and_the_rust_it_causes`
//! （要「库存恰好只够付一半」的精确构造，而且要和 `RoundSink` 对账）与
//! `build_lines_…` 的**内部那半**：`poor.spend`（`RoundSink` 的支出账，读面看不到——§4）、
//! 以及「两次跑必须从**同一份** state 出发」的单步语义。
//!
//! ⚠ **别把「批满」当成「钱管够」**：P1-5 之后 Player 写的 `construction_budget` 还要再乘一个
//! `con_scale = clamp((库存价值 − 维护 reserve) / 建舰上限, 0, 1)`（`autocontrol/budget.rs`）
//! ——**库存不到 reserve 时，写 1e6 也是 0**。这条用例原来只设预算不垫库存，靠的是「种子 42
//! 开局库存刚好够」；P1-5 一落地**连第一回合都掉到 0**，所以两边现在都显式垫库存（那是**
//! 隔离变量**，不是作弊）。g2 那边有一份一模一样的注释。

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

