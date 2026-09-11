//! **站点自给**：池子里的货到不了异地城市、首都天体的城花的就是池子、站点保留量约束装货。
//!
//! ## 2026-10（第 7 批）：两条搬去了 g2 **合成场景 · 谁能给城出钱**
//!
//! 开局 `depots` 本来就是空的 ⇒ 原件的 `gut_all_stock`（清池 + 清货栈）在回合 0 是**空操作**，
//! 所以直接造：把城改成「还差一半没建」（`建筑[].已建成面积 = 面积 × 0.5`，`护甲` 跟着改），
//! 再只拨**池子**。实测：首都城池满 ⇒ **r1 就长**（60→69.12）；池空 ⇒ r1 不长；
//! 非首都城池满 5000 ⇒ **r1 不变**（池子到不了别人家门口）；自然跑 40 回合 ⇒ 20.89→41.77
//! （本地货栈就是它的钱包）。
//!
//! ⚠ 判据只敢看**第 1 回合**：城自己会产出，「池空」臂到 r3 也长起来（60→69.12），
//! 「非首都·池满」臂到 r4 也开始长 ⇒ 拿多回合当判据就是假绿。
//!
//! ## `an_export_haul_never_loads_the_site_reserve` **没搬**（试过两版都红）
//!
//! 想用「装完货栈里剩下的 ≥ 本地保留量」当读面判据，两版都被打回：
//! * 「残余 ≥ 保留量」：r11 金星见底时存量 0 < 保留量 6——**引擎的规则是「只能装走超出保留量的
//!   那部分」**，本来就见底时装到 0 是合规的；
//! * 「要么留够、要么见底」：r25 水星.碳 存量 6.075 < 保留量 6.080——**城自己也在从货栈花钱**，
//!   残余本来就可能低于保留量。
//! 真正的判据要**装货前那一刻**的存量与保留量，读面上没有 ⇒ 留这里（§4）。
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

