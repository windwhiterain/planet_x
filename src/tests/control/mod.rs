//! control 的单元测试（读写面 = 控制树，不推进回合）。

use super::*;

mod apply;
mod blueprint;
mod normalize;
mod ship;
mod view;

// --- the apply report: "did my diff actually land?" ---------------------
//
// 这组守卫钉住的是**可观测性**，不是模拟：一个 diff 的叶片被丢掉时，退出码
// 与 stdout 与成功落地完全一样，所以「有没有报出来」是 agent 唯一的信号。

// ---- 舰船设计图（blueprint）的写面/读面契约 -------------------------------

/// 某势力第一座城的某个建造区：`(城名, 建筑下标, 舰级, 是不是建造区)`。
fn some_building(state: &State, fid: &str, shipyard: bool) -> (CityId, BuildingId, String) {
    for c in state.cities.iter().filter(|c| c.faction_id == fid) {
        for b in &c.buildings {
            if b.is_shipyard() == shipyard {
                return (c.name.clone(), b.id, b.kind.clone());
            }
        }
    }
    panic!(
        "{fid} 没有 {} 的建筑",
        if shipyard {
            "建造区"
        } else {
            "非建造区"
        }
    );
}

/// 建一张图 + 把它挂到某个建造区上（两个写面动作合并成一份 diff：这是正解用法）。
fn pin_blueprint(
    state: &mut State,
    config: &GameConfig,
    name: &str,
    class: &str,
    components: &serde_json::Value,
    mode: &str,
) -> (CityId, BuildingId) {
    let (cid, bid, _) = some_building(state, "中国", true);
    let diff = serde_json::json!({"control": [{"势力": "中国",
        "设计图库": [{"图名": name, "舰级": class, "选装": components, "归属": mode}],
        "建筑": [{"城": cid, "建筑": bid, "建造舰级": class, "设计图": name}],
    }]});
    let rep = apply_patch(state, config, &diff).expect("建图 + 挂图必须一次成功");
    assert!(rep.is_clean(), "正解用法不该被丢弃：{:?}", rep.skipped);
    (cid, bid)
}
