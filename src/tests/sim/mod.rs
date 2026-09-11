//! sim 的单元测试：不推进回合或 ≤48 回合（短档）。
//!
//! ⚠ 「中档（49–480 回合）」在本仓库的 Rust 侧**现在是空的**：那几条按模拟时长才算得出的
//! 判据（同回合复垦、选装、编年史、战争最短回合）都搬到了 `play/tests/g2_mid.py`
//! （数据级、不重编）。见 `.agents/notes/test-decoupled-suite.md`。
//!
//! ## 2026-10：本文件那条也搬走了（第 7 批）
//!
//! | 原用例 | 现在住 | 为什么能搬 |
//! | --- | --- | --- |
//! | `settlements_and_cities_are_one_to_one` | g1「世界形状：每座城占的定居点都在它自己的天体上、不重号、不超过该天体的定居点数」+「矿藏按定居点隔离」 | `bodies.settlement_count` / `settlements.资源` / `cities.{天体名,定居点}` 都在读面上，回合 0 的静态形状 |
//!
//! 本文件现在**只剩夹具**（`fresh_world` / `attach_blueprint` / `spawn_at` / `stock`），
//! 一条用例都没有——这是对的，夹具在这儿、用例在各自的主题文件里。

use super::*;
use crate::config::load_config;
use crate::control::apply_patch;
use crate::model::GameEvent;
use crate::world::default_state;

mod blueprints;
mod domestic_market;
mod fleet;
mod governance;
mod haul;
mod ideology;
mod knowledge;
mod market;
mod mond;
mod shots;
mod site_supply;
mod spending;
mod trade;
mod war_scar;

/// Build the config + a fresh deterministic world (round 0)，并把**角色轴钉成「全员战舰」**。
///
/// 这个模块的用例大多在测**别的东西**（生产/贸易/MOND/战斗/事件），而自动控制现在多了一条
/// 活：按积压定编、把船派去跑集货路线。不钉住它，被测的舰就可能被抽去拉货、不在它该在的
/// 位置上（`crate::world::pin_roles_to_war` 的文档记着一次实测踩坑）。
/// **测集货本身的用例**（`haul_*`）自己撤掉/覆盖这条默认。
fn fresh_world(seed: u64) -> (GameConfig, State) {
    let config = load_config();
    let mut state = default_state(&config, seed);
    crate::world::pin_roles_to_war(&mut state);
    (config, state)
}

//! 战斗拟真（`combat.rs`）**已经全部搬走**（2026-10 第 7 批），这个文件不再存在。
//!
//! 全部落点：
//! * 逐发取值域与击伤守恒 → g2 `combat_report`（`hit/soak/armor_soak/def_mult/score_spread`）；
//! * 齐射一级对账（逐发之和 == 聚合伤害）→ g2 `combat_checks`；
//! * 组件损耗（没挨打不掉 / 打进船体就掉）→ g2 `combat_report`；
//! * 本土修船（本土 +1.8/回合 vs 外海 +0.72/回合）→ g2 合成场景（造一个受损组件）；
//! * 护盾先吸、船体吃溢出 → g2 合成场景（造一仗：攻方 railgun、守方 shield）；
//! * 舰队防空屏护 → g2 合成场景（两臂只差 PD 友舰的坐标）；
//! * 回避（目标越快命中折减越低）→ g1 `--call hit_factor`。

// ---- 舰船设计图（blueprint）：出厂快照 / 归属 / 意图链 -------------------------
//
// 设计图的语义见 `.agents/notes/ship-blueprint-spec.md`（§8.0 的十条裁决）。这一组用例
// 钉住的是**别人最容易改坏**的几条：快照 vs 活层、图的意图轴默认沉默、买不起不下水、
// 悬空指针停产。

/// 在势力 `fid` 的**第一座有建造区的城**上挂一张图，并把该舰级的进度池准备好。
///
/// 返回 `(城名, 建筑下标, 舰级)`。`progress` 给 `build_points` 就下一回合必下水。
///
/// `stance` = 图上的**倾向三轴**（`(doctrine, kiting, role)`，各自 `None` = 本图对该轴沉默）。
/// ⚠ 图**不再**携带指令（2026-10 裁决）：`ShipBehavior` 只走逐舰叶。
#[allow(clippy::too_many_arguments)]
fn attach_blueprint(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    bp_name: &str,
    class: &str,
    components: &[&str],
    stance: (Option<ShipDoctrine>, Option<f64>, Option<ShipRole>),
    mode: ControlMode,
) -> (CityId, BuildingId, String) {
    state
        .control
        .entry(fid.to_string())
        .or_default()
        .blueprints
        .insert(
            bp_name.to_string(),
            Control {
                value: Blueprint {
                    class: class.to_string(),
                    components: components.iter().map(|c| c.to_string()).collect(),
                    doctrine: stance.0,
                    kiting: stance.1,
                    role: stance.2,
                },
                mode,
            },
        );
    let (cid, bid) = state
        .cities
        .iter()
        .filter(|c| c.faction_id == fid)
        // **只挑「恰好一个建造区」的城**：进度池是按舰级合并的，同城第二个建造区会跟它抢
        // 这个池子（谁权重高就用谁的图，见 `build_city`）——用例要的是「这个区就该舰级的
        // 唯一出口」，否则断言会在别的区产出的那艘舰上翻车。
        .find_map(|c| {
            let yards: Vec<&Building> = c.buildings.iter().filter(|b| b.is_shipyard()).collect();
            (yards.len() == 1).then(|| (c.name.clone(), yards[0].id))
        })
        .expect("该势力要有一座只有一个建造区的城");
    if let Some(city) = state.city_mut(&cid) {
        for b in city.buildings.iter_mut() {
            if b.id == bid {
                b.ship_type = Some(class.to_string());
                b.blueprint = Some(bp_name.to_string());
            }
        }
        // 进度池备到差一点就满：下一回合必定触发一次下水判定。
        city.ship_progress
            .insert(class.to_string(), config.ship_spec(class).build_points);
    }
    (cid, bid, class.to_string())
}

/// 测试用：在天体 `body` 上造一艘舰（把「先算位置、再进漏斗」写在一处：`spawn_ship`
/// 已经可变借用 `state`，参数里再读 `state.body_position(..)` 会撞两阶段借用）。
fn spawn_at(
    state: &mut State,
    config: &GameConfig,
    owner: &str,
    class: &str,
    body: &str,
    blueprint: Option<&str>,
) -> ShipId {
    let pos = state.body_position(body);
    let bp = blueprint.map(|s| s.to_string());
    spawn_ship(
        state,
        config,
        ShipSpawn {
            owner: owner.to_string(),
            class,
            position: pos,
            city: None,
            via: SpawnVia::Shipyard,
            pay_components: false,
            blueprint: bp.as_ref(),
        },
    )
}

/// 把某势力喂饱**所有**资源（免得「买不起」把设计图的用例卡住）。
///
/// 必须按 `config.resources` 的**全表**给：势力开局只有它自己那几种矿，而设计图上的
/// 选装可能要稀有矿（等离子炮要氦-3/金）——只给现存的键会让「买不起」这条新规则把用例
/// 卡在门外（这正是 Q4(b) 生效的样子）。
fn stock(state: &mut State, config: &GameConfig, fid: &str, amount: f64) {
    let keys: Vec<String> = config.resources.keys().cloned().collect();
    let Some(f) = state.faction_mut(fid) else {
        return;
    };
    for k in keys {
        f.resources.insert(k, amount);
    }
}
