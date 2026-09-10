//! **站点自给的物理**（用户裁决：「完全禁止瞬移」）：非首都天体上的建造只能花**那处货栈**的货，
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

/// **同一件货不可能同时出现在两条腿上**：一处站点的**出口净额**与**进口缺口**，
/// 逐资源**至少有一个是 0**（存货高于保留量 ⇒ 只有出口；低于 ⇒ 只有缺口）。
///
/// 这是防「往返乒乓」的那条口径（实测踩过：出口按全部现货算时，刚送到殖民地的建材
/// 下一回合就被集货签抽中运回首都）。
#[test]
fn a_site_never_exports_what_it_still_needs() {
    let (config, mut state) = fresh_world(42);
    let fid = "中国";
    let body = "水星"; // 非首都天体，且这里有中国的一座城
    for (label, amount) in [("空空如也", 0.0), ("刚好一点", 1.0), ("堆成山", 5000.0)] {
        gut_all_stock(&mut state);
        if amount > 0.0 {
            for rt in ["铁", "碳", "硅", "氢", "氦-3"] {
                state.depot_add(fid, body, rt, amount);
            }
        }
        let out = freight::exportable_at(&state, &config, fid, body);
        let inc = freight::site_deficit(&state, &config, fid, body);
        for rt in ["铁", "碳", "硅", "氢", "氦-3"] {
            let o = out.get(rt).copied().unwrap_or(0.0);
            let i = inc.get(rt).copied().unwrap_or(0.0);
            assert!(
                o <= 1e-9 || i <= 1e-9,
                "{label}：{rt} 同时出现在出口（{o:.2}）与进口（{i:.2}）两侧——货会乒乓"
            );
        }
    }
    // 堆成山 ⇒ 该有出口（否则货永远出不去，首都会饿死）。
    gut_all_stock(&mut state);
    state.depot_add(fid, body, "铁", 5000.0);
    assert!(
        freight::exportable_at(&state, &config, fid, body)
            .get("铁")
            .copied()
            .unwrap_or(0.0)
            > 0.0,
        "远超本地需求的存货必须有出口（否则首都没货可收）"
    );
}

/// **保留量随航程变**：同一个站点、同样的建设活，**离首都越远**该囤的料越多
/// （`site_reserve = min(计划, 每回合消耗速率 × 这条线一个往返的回合数)`）。
///
/// 用「两个真实天体各自与首都的往返回合数」比出方向，不写死数字。
#[test]
fn the_reserve_grows_with_the_round_trip_time() {
    let (config, state) = fresh_world(42);
    let fid = "中国";
    let cap = state.capital_body(fid);
    // 中国在**水星**（近）与**金星**（更远）各有一座城：同一份建设活，远的那处该囤更多。
    let near = freight::site_reserve(&state, &config, fid, "水星");
    let far = freight::site_reserve(&state, &config, fid, "金星");
    let r_near = crate::model::lane_rounds(&state, &config, "水星", &cap);
    let r_far = crate::model::lane_rounds(&state, &config, "金星", &cap);
    assert!(
        r_far > r_near,
        "用例前提：金星这条线的一个往返更久（{r_far} vs {r_near}）"
    );
    let u_near: f64 = near.values().sum();
    let u_far: f64 = far.values().sum();
    assert!(
        u_far > u_near,
        "更远的站点该囤更多（金星 {u_far:.2} 应 > 水星 {u_near:.2}）"
    );
}
