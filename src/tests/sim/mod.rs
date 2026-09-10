//! sim 的单元测试：不推进回合或 ≤48 回合（短档）。长档见同目录 horizon_mid.rs。

use super::*;
use crate::config::load_config;
use crate::control::apply_patch;
use crate::model::GameEvent;
use crate::world::default_state;

mod blueprints;
mod capital;
mod combat;
mod fleet;
mod governance;
mod haul;
mod ideology;
mod knowledge;
mod mond;
mod spending;
mod story;

mod horizon_mid;

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

/// 定居点 ↔ 城市 一一对应: 每个城市占据其天体上一个合法定居点；同一座城不会
/// 让一个定居点被两座城占用；地球恰好 5 个定居点各坐一座 spec 都市，矿藏按
/// 定居点隔离（巴黎只产 铀/铂，不再共享整个地球的矿藏池）。
#[test]
fn settlements_and_cities_are_one_to_one() {
    let (_config, state) = fresh_world(42);
    for b in &state.bodies {
        let cities: Vec<&City> = state.cities.iter().filter(|c| c.body_id == b.name).collect();
        assert!(
            cities.len() <= b.settlements.len(),
            "body {}: {} cities must not exceed {} settlements",
            b.name,
            cities.len(),
            b.settlements.len()
        );
        for c in cities {
            assert!(
                b.settlements.iter().any(|s| s.name == c.settlement),
                "city {} (body {}) points at an unknown settlement {}",
                c.name,
                b.name,
                c.settlement
            );
        }
    }

    let earth = &state.bodies[2];
    assert_eq!(earth.settlements.len(), 5, "Earth has five spec metropolises");
    let earth_cities = state.cities.iter().filter(|c| c.body_id == "地球").count();
    assert_eq!(earth_cities, 5, "five cities on five Earth settlements (1:1)");
    // 巴黎 (settlement named 巴黎) hosts only 铀/铂 — its own region's ores.
    let paris = earth.settlements[3].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
    assert_eq!(paris, vec!["铀", "铂"], "Paris settlement mines only its own ores");
    assert_eq!(
        state.cities.iter().find(|c| c.name == "巴黎").map(|c| c.settlement.as_str()),
        Some("巴黎"),
        "巴黎 occupies the settlement named 巴黎"
    );
    // 长三角/珠三角 are distinct settlements, so both may mine 铁 independently.
    let cn = earth.settlements[0].resources.iter().map(|d| d.resource.as_str()).collect::<Vec<_>>();
    assert!(cn.contains(&"铁"), "长三角 settlement has 铁");
    assert!(cn.contains(&"硅") && cn.contains(&"水冰"), "长三角 has 硅/水冰");
}

// ---- 舰船设计图（blueprint）：出厂快照 / 归属 / 意图链 -------------------------
//
// 设计图的语义见 `.agents/notes/ship-blueprint-spec.md`（§8.0 的十条裁决）。这一组用例
// 钉住的是**别人最容易改坏**的几条：快照 vs 活层、图的意图轴默认沉默、买不起不下水、
// 悬空指针停产。

/// 在势力 `fid` 的**第一座有建造区的城**上挂一张图，并把该舰级的进度池准备好。
///
/// 返回 `(城名, 建筑下标, 舰级)`。`progress` 给 `build_points` 就下一回合必下水。
fn attach_blueprint(
    state: &mut State,
    config: &GameConfig,
    fid: &str,
    bp_name: &str,
    class: &str,
    components: &[&str],
    order: Option<ShipBehavior>,
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
                    order,
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
    spawn_ship(state, config, ShipSpawn {
        owner: owner.to_string(),
        class,
        position: pos,
        city: None,
        via: SpawnVia::Shipyard,
        pay_components: false,
        blueprint: bp.as_ref(),
    })
}

/// 把某势力喂饱**所有**资源（免得「买不起」把设计图的用例卡住）。
///
/// 必须按 `config.resources` 的**全表**给：势力开局只有它自己那几种矿，而设计图上的
/// 选装可能要稀有矿（等离子炮要氦-3/金）——只给现存的键会让「买不起」这条新规则把用例
/// 卡在门外（这正是 Q4(b) 生效的样子）。
fn stock(state: &mut State, config: &GameConfig, fid: &str, amount: f64) {
    let keys: Vec<String> = config.resources.keys().cloned().collect();
    let Some(f) = state.faction_mut(fid) else { return };
    for k in keys {
        f.resources.insert(k, amount);
    }
}
