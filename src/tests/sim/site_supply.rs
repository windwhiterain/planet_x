//! **站点自给**：池子里的货到不了异地城市、首都天体的城花的就是池子、站点保留量约束装货。
//!
//! ## 2026-10：能只看数据的那两条搬走了（第 7 批）
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `the_reserve_grows_with_the_round_trip_time` | g1 `--call lane_rounds` / `site_reserve` | 新挂了 `--call`（`main.rs` 直接转发 `autocontrol::freight` 的**同一份实现**，不是抄公式）：两个真天体的往返回合数比出方向 |
//! | `a_site_never_exports_what_it_still_needs` | g2 `site_ledger_checks`（3 条） | 新挂了 `--call site_ledger`（**每个站点一行**的现货/保留/可出口/缺口）⇒ 在真实世界里各个回合扫，库存水平是**世界自己长出来的**，不用像原件那样手工摆三档 |
//!
//! ⚠ `site_ledger` 的站点集合必须包含「有城但还没货栈」的：只按 `state.depots` 收，
//! 回合 0 是一张空表（判据空转）——踩过。
//!
//! **留在这里的**：`only_the_local_depot_can_fund_an_offsite_city` /
//! `the_capital_body_spends_the_faction_pool` 要先把一座城的建筑改成「已建面积 < 总面积」
//! （原件用 `half_built` 直改 `deployed`），读面上没有可写的入口；
//! `an_export_haul_never_loads_the_site_reserve` 要 `haul_load` 的内部构造。
//! 首都天体照旧花势力池。守卫三件事：
//!
//! 1. **不许瞬移**：池子里有货、站点没有 ⇒ 那儿的楼**一寸也长不动**；本地有货 ⇒ 才动；
//! 2. **进出口互为镜像**：存货超过「本回合该留的量」才是出口、低于才是缺口——同一件货
//!    不可能同时出现在两条腿上（否则刚送到的货下一回合就被原路运回去）；
//! 3. **保留量随航程变**：一个往返越久的天体，该囤的料越多（lead-time 库存）。

use super::*;
use crate::autocontrol::freight;

/// 某势力赤手空拳地从零起一个测试城：把一座城的建筑**全部变成在建**（`deployed < area`），
/// 并清空一切库存（池子 + 所有货栈），这样「谁在付钱」是唯一变量。
fn gut_all_stock(state: &mut State) {
    for f in state.factions.iter_mut() {
        f.resources.clear();
    }
    state.depots.clear();
}

/// 把某座城的建筑改成「还差总面积的一半没建」。
fn half_built(state: &mut State, cid: &str) {
    if let Some(c) = state.city_mut(cid) {
        for b in c.buildings.iter_mut() {
            b.deployed = b.area * 0.5;
            b.armor = b.deployed;
        }
    }
}

fn built_area(state: &State, cid: &str) -> f64 {
    state
        .city(cid)
        .map(|c| c.buildings.iter().map(|b| b.deployed).sum())
        .unwrap_or(0.0)
}

/// **池子里的货到不了别人家门口**（「完全禁止瞬移」的核心）：同一座非首都城市，
/// 池子里堆满也只长不动；往**它所在的天体**放货，它立刻开始长。
#[test]
fn only_the_local_depot_can_fund_an_offsite_city() {
    let (config, mut state) = fresh_world(42);
    // 水星熔炉基地 = 中国的非首都城市（首都 = 地球）。
    let cid = "水星熔炉基地".to_string();
    assert_ne!(
        state.capital_body("中国"),
        "水星",
        "用例前提：水星不是中国的首都"
    );
    gut_all_stock(&mut state);
    half_built(&mut state, &cid);
    let before = built_area(&state, &cid);

    // ① 池子里堆满料（首都那一份），但**水星本地什么都没有** ⇒ 一寸也长不动。
    if let Some(f) = state.faction_mut("中国") {
        for rt in ["铁", "碳", "硅"] {
            f.resources.insert(rt.to_string(), 5000.0);
        }
    }
    let mut rng = Prng::new(42);
    advance(&mut state, &config, &mut rng);
    let after_pool_only = built_area(&state, &cid);
    assert!(
        (after_pool_only - before).abs() < 1e-6,
        "池子里有货不等于水星有货：那座城不该长出任何面积（{before:.3} → {after_pool_only:.3}）"
    );

    // ② 往**水星**放同样的料 ⇒ 它当场开始长（本地货栈就是它的钱包）。
    state.depot_add("中国", "水星", "铁", 5000.0);
    state.depot_add("中国", "水星", "碳", 5000.0);
    state.depot_add("中国", "水星", "硅", 5000.0);
    let mut rng = Prng::new(42);
    advance(&mut state, &config, &mut rng);
    let after_local = built_area(&state, &cid);
    assert!(
        after_local > after_pool_only + 1e-6,
        "本地有货就该长（{after_pool_only:.3} → {after_local:.3}）"
    );
}

/// **首都天体不受影响**（公理「首都即集散地」）：首都上的城花的就是池子。
#[test]
fn the_capital_body_spends_the_faction_pool() {
    let (config, mut state) = fresh_world(42);
    let cid = "长三角".to_string();
    assert_eq!(
        state.city(&cid).unwrap().body_id,
        state.capital_body("中国"),
        "用例前提"
    );
    gut_all_stock(&mut state);
    half_built(&mut state, &cid);
    if let Some(f) = state.faction_mut("中国") {
        for rt in ["铁", "碳", "硅"] {
            f.resources.insert(rt.to_string(), 5000.0);
        }
    }
    let before = built_area(&state, &cid);
    let mut rng = Prng::new(42);
    advance(&mut state, &config, &mut rng);
    assert!(
        built_area(&state, &cid) > before + 1e-6,
        "首都上的城花池子，池子里有货就该长"
    );
}

/// **实际装货也受出口保留量约束**：起点不是货主首都时，`haul_load` 只能装
/// `exportable_at` 允许的净剩余；站点自己留着建楼/造舰的 `site_reserve` 不能被装走。
///
/// 这是 P0-1 的守卫：仓里本来足够留出保留量，船却会把整仓装走的那条 bug。
#[test]
fn an_export_haul_never_loads_the_site_reserve() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    let body = "水星";
    let cid = "水星熔炉基地".to_string();
    assert_ne!(state.capital_body(fid), body, "用例前提：水星不是中国首都");
    assert!(state.city(&cid).is_some(), "用例前提：水星上有一处中国城市");

    // 只留这一处城市的建设需求，库存先清空，保证保留量全部来自该站点。
    gut_all_stock(&mut state);
    half_built(&mut state, &cid);
    let reserve = freight::site_reserve(&state, &config, fid, body);
    let (rt, need) = reserve
        .iter()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(r, v)| (r.clone(), *v))
        .expect("半建成城市必须有保留量");
    assert!(need > 1e-9, "用例前提：该站点对 {rt} 的保留量为正");

    // 库存 = 保留量 + 1：出口净剩余恰好 1，但旧代码会按「全量库存」装货。
    state.depot_add(fid, body, &rt, need + 1.0);
    let exportable = freight::exportable_at(&state, &config, fid, body)
        .get(&rt)
        .copied()
        .unwrap_or(0.0);
    assert!(
        (exportable - 1.0).abs() < 1e-9,
        "用例构造失败：{rt} 的出口净剩余应为 1，实为 {exportable}"
    );

    let ship = state
        .ships
        .iter()
        .filter(|s| s.faction_id == fid && s.hull > 0.0)
        .max_by(|a, b| cargo_capacity(&config, a).total_cmp(&cargo_capacity(&config, b)))
        .map(|s| s.name.clone())
        .expect("用例前提：中国至少有一艘舰");
    let room = cargo_capacity(&config, state.ship(&ship).unwrap());
    assert!(
        room > exportable + 1.0,
        "用例前提：有效舱容 {room} 要大于出口净剩余 {exportable}，否则旧 bug 会被舱容掩盖"
    );
    if let Some(s) = state.ship_mut(&ship) {
        s.cargo.clear();
    }

    let loaded = haul_load(&mut state, &config, fid, &ship, body, "地球");
    let left = state
        .stock_at(fid, body)
        .and_then(|m| m.get(&rt))
        .copied()
        .unwrap_or(0.0);
    assert!(loaded > 0.0, "这条腿真的有货可装——守卫不能空转");
    assert!(
        loaded <= exportable + 1e-9,
        "装货量 {loaded} 超过了出口净剩余 {exportable}（把保留量装走了）"
    );
    assert!(
        left + 1e-9 >= need,
        "装完后站点库存 {left} 低于保留量 {need}——本地建设料被运走了"
    );
}

